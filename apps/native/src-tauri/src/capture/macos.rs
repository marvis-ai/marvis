//! ScreenCaptureKit-backed [`FrameSource`] for macOS.
//!
//! The dispatch-queue callback is `extract_raw` only (lock, memcpy BGRA,
//! unlock) so it can never stall `ScreenCaptureKit`'s delivery queue;
//! dedupe/resize/JPEG all run in `frame_pipe::run_worker` shared with the
//! Windows and Linux backends.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use base64::{engine::general_purpose, Engine as _};
use parking_lot::Mutex;
use screencapturekit::cm::{CMSampleBuffer, CMSampleBufferExt, CMTime};
use screencapturekit::screenshot_manager::SCScreenshotManager;
use screencapturekit::shareable_content::{
    SCRunningApplication, SCShareableContent, SCShareableContentInfo, SCWindow,
};
use screencapturekit::stream::configuration::{PixelFormat, SCStreamConfiguration};
use screencapturekit::stream::content_filter::SCContentFilter;
use screencapturekit::stream::output_type::SCStreamOutputType;
use screencapturekit::stream::sc_stream::SCStream;

use super::frame_pipe::{encode_frame, jpeg_at, run_worker, RawFrame};
use super::{frame_interval_secs, Frame, FrameSource, PickCandidate, PickResolution};

/// Big-endian FourCC for BGRA — checked per frame so a surprise format can
/// never silently garble colors.
const FOURCC_BGRA: u32 = u32::from_be_bytes(*b"BGRA");

/// Screen capture over a caller-built filter. Create with
/// [`MacosCapture::for_display`] (primary display — enumerates shareable
/// content eagerly so permission failures surface at construction, not
/// mid-stream) or [`MacosCapture::new`], then drive via [`FrameSource`].
pub struct MacosCapture {
    state: Mutex<CaptureState>,
}

struct CaptureState {
    filter: SCContentFilter,
    config: SCStreamConfiguration,
    running: Option<Running>,
}

/// Everything that exists only while capture is live.
struct Running {
    stream: SCStream,
    worker: JoinHandle<()>,
    stop: Arc<AtomicBool>,
}

/// Build the primary-display filter shared by screen and system-audio
/// streams. The platform-neutral name is `primary_display_source` — on
/// macOS a `CaptureSource` IS an `SCContentFilter`.
pub(crate) fn primary_display_source() -> anyhow::Result<(SCContentFilter, u32, u32)> {
    let content = SCShareableContent::get()?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no shareable display"))?;
    let (width, height) = (display.width(), display.height());
    let own_pid = std::process::id() as i32;
    let applications = content.applications();
    let own = applications
        .iter()
        .find(|app| app.process_id() == own_pid)
        .ok_or_else(|| anyhow::anyhow!("Marvis application unavailable for exclusion"))?;
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_applications(&[own], &[])
        .build();
    Ok((filter, width, height))
}

/// One-shot screenshot of the primary display via
/// `SCScreenshotManager` — the "read my screen" path when ambient
/// recording is off. Same filter and encode path as the stream, so a
/// single-shot `Frame` is indistinguishable from a ring frame.
/// `Ok(None)` means the call succeeded but delivered no pixels.
pub(crate) fn shot_fullscreen() -> anyhow::Result<Option<Frame>> {
    let (filter, width, height) = primary_display_source()?;
    let config = SCStreamConfiguration::new()
        .with_width(width)
        .with_height(height)
        .with_pixel_format(PixelFormat::BGRA)
        .with_shows_cursor(false);
    let sample = SCScreenshotManager::capture_sample_buffer(&filter, &config)
        .map_err(|e| anyhow::anyhow!("screenshot failed: {e}"))?;
    let Some(raw) = extract_raw(&sample) else {
        return Ok(None);
    };
    Ok(encode_frame(&raw, 0))
}

/// Smallest window worth listing — drops palette/tooling slivers.
const PICK_MIN_W: f64 = 140.0;
const PICK_MIN_H: f64 = 100.0;
/// Picker thumbnail width — cards are ~360 CSS px; 480 stays crisp
/// on 2x displays without multi-hundred-KB payloads.
const THUMB_WIDTH: u32 = 480;

/// Window eligibility shared by `pick_candidates` passes: on-screen,
/// normal layer, reasonable size, not ours.
fn pick_window_ok(w: &SCWindow, own_pid: i32) -> Option<SCRunningApplication> {
    if !w.is_on_screen() || w.window_layer() != 0 {
        return None;
    }
    let f = w.frame();
    if f.size.width < PICK_MIN_W || f.size.height < PICK_MIN_H {
        return None;
    }
    let app = w.owning_application()?;
    (app.process_id() != own_pid).then_some(app)
}

/// Every shareable candidate: displays in order, eligible windows,
/// then one app entry per distinct window owner.
pub(crate) fn pick_candidates() -> anyhow::Result<Vec<PickCandidate>> {
    let content = SCShareableContent::get()?;
    let own_pid = std::process::id() as i32;
    let mut out = Vec::new();
    for (i, d) in content.displays().iter().enumerate() {
        out.push(PickCandidate {
            id: format!("d:{}", d.display_id()),
            kind: "display",
            label: format!("Screen {}", i + 1),
            sub: None,
            w: d.width(),
            h: d.height(),
            thumb_of: None,
        });
    }
    let windows = content.windows();
    // bundle_id → (app_name, largest window id, its area, w, h)
    let mut apps: HashMap<String, (String, u32, f64, u32, u32)> = HashMap::new();
    for w in &windows {
        let Some(app) = pick_window_ok(w, own_pid) else {
            continue;
        };
        let f = w.frame();
        // Untitled windows fall back to the app name as the label —
        // then the sub would repeat it, so only subtitle titled ones.
        let title = w.title().filter(|t| !t.trim().is_empty());
        let label = title.clone().unwrap_or_else(|| app.application_name());
        let sub = title.map(|_| app.application_name());
        out.push(PickCandidate {
            id: format!("w:{}", w.window_id()),
            kind: "window",
            label,
            sub,
            w: f.size.width as u32,
            h: f.size.height as u32,
            thumb_of: None,
        });
        let area = f.size.width * f.size.height;
        let entry = apps.entry(app.bundle_identifier()).or_insert_with(|| {
            (
                app.application_name(),
                w.window_id(),
                area,
                f.size.width as u32,
                f.size.height as u32,
            )
        });
        if area > entry.2 {
            *entry = (
                app.application_name(),
                w.window_id(),
                area,
                f.size.width as u32,
                f.size.height as u32,
            );
        }
    }
    // Stable order: sort apps by name so the section doesn't jitter.
    let mut apps: Vec<_> = apps.into_iter().collect();
    apps.sort_by(|a, b| a.1 .0.cmp(&b.1 .0));
    for (bundle, (name, win_id, _area, w, h)) in apps {
        out.push(PickCandidate {
            id: format!("a:{bundle}"),
            kind: "app",
            label: name,
            sub: None,
            w,
            h,
            thumb_of: Some(format!("w:{win_id}")),
        });
    }
    Ok(out)
}

/// Re-resolve a picker id against FRESH shareable content — windows
/// move/close between list and pick, so a stale id errors rather
/// than silently capturing the wrong thing.
pub(crate) fn resolve_candidate(id: &str) -> anyhow::Result<PickResolution> {
    let content = SCShareableContent::get()?;
    let own_pid = std::process::id() as i32;
    let windows = content.windows();
    let displays = content.displays();
    let (filter, kind, label) = if let Some(did) = id.strip_prefix("d:") {
        let did: u32 = did.parse().map_err(|_| anyhow::anyhow!("bad display id"))?;
        let idx = displays
            .iter()
            .position(|x| x.display_id() == did)
            .unwrap_or(0);
        let d = displays
            .iter()
            .find(|d| d.display_id() == did)
            .ok_or_else(|| anyhow::anyhow!("display no longer available"))?;
        // Excluding the application also covers windows created after
        // this filter, such as the lazily opened preferences window.
        let applications = content.applications();
        let own = applications
            .iter()
            .find(|app| app.process_id() == own_pid)
            .ok_or_else(|| anyhow::anyhow!("Marvis application unavailable for exclusion"))?;
        (
            SCContentFilter::create()
                .with_display(d)
                .with_excluding_applications(&[own], &[])
                .build(),
            "display",
            format!("Screen {}", idx + 1),
        )
    } else if let Some(wid) = id.strip_prefix("w:") {
        let wid: u32 = wid.parse().map_err(|_| anyhow::anyhow!("bad window id"))?;
        let win = windows
            .iter()
            .find(|w| w.window_id() == wid)
            .ok_or_else(|| anyhow::anyhow!("window no longer available"))?;
        let label = win
            .title()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| win.owning_application().map(|a| a.application_name()))
            .unwrap_or_else(|| "Window".into());
        (
            SCContentFilter::create().with_window(win).build(),
            "window",
            label,
        )
    } else if let Some(bundle) = id.strip_prefix("a:") {
        let app = content
            .applications()
            .into_iter()
            .find(|a| a.bundle_identifier() == bundle)
            .ok_or_else(|| anyhow::anyhow!("app no longer running"))?;
        // Display containing the app's largest eligible window's
        // center; first display as fallback.
        let mut best: Option<(f64, u32)> = None; // (area, display_id)
        for w in &windows {
            let Some(owner) = pick_window_ok(w, own_pid) else {
                continue;
            };
            if owner.bundle_identifier() != bundle {
                continue;
            }
            let f = w.frame();
            let (cx, cy) = (f.mid_x(), f.mid_y());
            let Some(d) = displays.iter().find(|d| {
                d.frame()
                    .contains_point(screencapturekit::cg::CGPoint { x: cx, y: cy })
            }) else {
                continue;
            };
            let area = f.size.width * f.size.height;
            if best.is_none_or(|(a, _)| area > a) {
                best = Some((area, d.display_id()));
            }
        }
        let did = best
            .map(|(_, d)| d)
            .unwrap_or_else(|| displays.first().map(|d| d.display_id()).unwrap_or(0));
        let d = displays
            .iter()
            .find(|d| d.display_id() == did)
            .ok_or_else(|| anyhow::anyhow!("no shareable display"))?;
        (
            SCContentFilter::create()
                .with_display(d)
                .with_including_applications(&[&app], &[])
                .build(),
            "app",
            app.application_name(),
        )
    } else {
        return Err(anyhow::anyhow!("unrecognized picker id"));
    };
    // Zero dims would build a broken MacosCapture — the info lookup
    // is required (it populated for the native picker path).
    let (w, h) = SCShareableContentInfo::for_filter(&filter)
        .map(|i| i.pixel_size())
        .ok_or_else(|| anyhow::anyhow!("filter info unavailable"))?;
    Ok(PickResolution {
        source: filter,
        w,
        h,
        kind,
        label,
    })
}

/// One ~`THUMB_WIDTH`-wide JPEG for a candidate — the picker card
/// image, base64 (the emit payload is a string).
pub(crate) fn thumb_for(id: &str) -> Option<String> {
    let res = resolve_candidate(id).ok()?;
    if res.w == 0 || res.h == 0 {
        return None;
    }
    let tw = THUMB_WIDTH.min(res.w);
    let th = ((u64::from(res.h) * u64::from(tw)) / u64::from(res.w)).max(1) as u32;
    let config = SCStreamConfiguration::new()
        .with_width(tw)
        .with_height(th)
        .with_pixel_format(PixelFormat::BGRA)
        .with_shows_cursor(false);
    let sample = SCScreenshotManager::capture_sample_buffer(&res.source, &config).ok()?;
    let raw = extract_raw(&sample)?;
    let (jpeg, _, _) = jpeg_at(&raw, tw)?;
    Some(general_purpose::STANDARD.encode(jpeg))
}

impl MacosCapture {
    /// Wrap a caller-built filter (primary display via
    /// [`primary_display_source`], or a picker result) with the shared
    /// stream configuration. Does not start capturing.
    pub fn new(filter: SCContentFilter, width: u32, height: u32, fps: u32) -> anyhow::Result<Self> {
        let config = SCStreamConfiguration::new()
            .with_width(width)
            .with_height(height)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(false)
            .with_minimum_frame_interval(&CMTime::from_seconds(frame_interval_secs(fps), 600))
            // Smallest allowed depth (3..=8): prefer dropping stale frames
            // over queueing them when the worker falls behind.
            .with_queue_depth(3);
        Ok(Self {
            state: Mutex::new(CaptureState {
                filter,
                config,
                running: None,
            }),
        })
    }

    /// The auto-start path: whole primary display, Marvis's own windows
    /// excluded (a picker can't appear without user interaction).
    ///
    /// # Errors
    ///
    /// Fails if screen-recording permission is missing or no display is
    /// shareable.
    #[allow(dead_code)] // callers build filters via `primary_display_source` + `new`; the manual test drives this
    pub fn for_display(fps: u32) -> anyhow::Result<Self> {
        let (filter, width, height) = primary_display_source()?;
        Self::new(filter, width, height, fps)
    }
}

impl FrameSource for MacosCapture {
    /// Start the stream and worker. A second `start` while already running
    /// is a no-op (documented semantics — it does not restart or rebind
    /// `on_frame`).
    fn start(&self, on_frame: Box<dyn Fn(Frame) + Send>) {
        let mut state = self.state.lock();
        if state.running.is_some() {
            return;
        }

        // Bounded queue: a slow worker drops fresh frames instead of
        // accumulating multi-MB raw buffers in memory.
        let (tx, rx) = mpsc::sync_channel::<RawFrame>(2);
        let mut stream = SCStream::new(&state.filter, &state.config);
        if stream
            .add_output_handler(
                move |sample: CMSampleBuffer, _of_type| {
                    if let Some(raw) = extract_raw(&sample) {
                        // Full queue or dead worker: drop the frame rather
                        // than blocking the dispatch queue.
                        let _ = tx.try_send(raw);
                    }
                },
                SCStreamOutputType::Screen,
            )
            .is_none()
        {
            log::error!("SCStream rejected the screen output handler");
            return;
        }
        if let Err(e) = stream.start_capture() {
            log::error!("SCStream start_capture failed: {e}");
            return;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || run_worker(rx, stop, on_frame))
        };
        state.running = Some(Running {
            stream,
            worker,
            stop,
        });
    }

    /// Stop the stream, close the frame channel, and join the worker.
    /// No-op when not running.
    fn stop(&self) {
        let Some(running) = self.state.lock().running.take() else {
            return;
        };
        running.stop.store(true, Ordering::Relaxed);
        if let Err(e) = running.stream.stop_capture() {
            log::warn!("SCStream stop_capture failed: {e}");
        }
        // Dropping the stream releases the output handler, which drops the
        // channel sender; the worker exits on `stop` or disconnect.
        drop(running.stream);
        let _ = running.worker.join();
    }

    /// Whether the stream + worker are live — false after a failed `start`
    /// (the error path never fills `running`) or after `stop`.
    fn is_running(&self) -> bool {
        self.state.lock().running.is_some()
    }
}

impl Drop for MacosCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Cheap extraction on the dispatch queue: lock the pixel buffer, memcpy the
/// BGRA rows, unlock. Hashing/resizing/encoding all happen on the worker.
fn extract_raw(sample: &CMSampleBuffer) -> Option<RawFrame> {
    let buffer = sample.pixel_buffer()?;
    if buffer.pixel_format() != FOURCC_BGRA {
        return None;
    }
    let guard = buffer.lock_read_only().ok()?;
    let (width, height, bytes_per_row) = (guard.width(), guard.height(), guard.bytes_per_row());
    if bytes_per_row < width.saturating_mul(4) {
        return None;
    }
    // SAFETY: the guard holds the read lock for this scope; `as_slice`
    // returns exactly `bytes_per_row * height` bytes of non-planar data, and
    // we copy them into an owned Vec before the guard (and its lock) drops.
    // No reference into the mapped range outlives this function.
    let data = unsafe { guard.as_slice() }?.to_vec();
    Some(RawFrame {
        data,
        width: width as u32,
        height: height as u32,
        bytes_per_row,
    })
}

#[cfg(test)]
mod tests {
    use super::super::FrameSource;
    use super::MacosCapture;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Manual verification — needs screen-recording permission and a real
    /// display, so it can't run in CI or this environment.
    ///
    /// Run once locally:
    /// `cargo test -- --ignored capture::macos::tests::captures_frames_for_three_seconds`
    /// then grant permission when prompted; prints per-frame sizes and the
    /// total count after ~3 s.
    #[test]
    fn frame_interval_maps_fps_to_seconds() {
        assert_eq!(super::super::frame_interval_secs(8), 0.125);
        assert_eq!(super::super::frame_interval_secs(4), 0.25);
        assert_eq!(super::super::frame_interval_secs(2), 0.5);
        assert_eq!(super::super::frame_interval_secs(0), 1.0); // defensive floor
    }

    /// The display filter's application exclusion is load-bearing:
    /// Marvis's windows are NOT `set_content_protected` on macOS (the
    /// user must be able to screenshot them), so
    /// `primary_display_source`'s `with_excluding_applications(&[own], &[])` is the
    /// only thing keeping Marvis out of its own captures.
    #[test]
    fn primary_display_source_excludes_own_windows() {
        let source = include_str!("macos.rs");
        let body = source
            .split("fn primary_display_source(")
            .nth(1)
            .and_then(|rest| rest.split("\npub(crate) fn ").next())
            .expect("primary_display_source body not found");
        for needle in [
            "process_id() == own_pid",
            "with_excluding_applications(&[own], &[])",
        ] {
            assert!(body.contains(needle), "display filter must keep {needle}");
        }
    }

    /// `pick_candidates` can never run in tests (needs real
    /// `SCShareableContent`), so guard the exclusion logic at source:
    /// `pick_window_ok` must drop own-pid windows AND filter by
    /// layer/screen/size.
    #[test]
    fn pick_window_ok_excludes_own_pid_and_nonstandard_windows() {
        let source = include_str!("macos.rs");
        let body = source
            .split("fn pick_window_ok(")
            .nth(1)
            .and_then(|rest| rest.split("\npub(crate) fn ").next())
            .expect("pick_window_ok body not found");
        for needle in [
            "is_on_screen()",
            "window_layer() != 0",
            "app.process_id() != own_pid",
        ] {
            assert!(body.contains(needle), "pick_window_ok must check {needle}");
        }
    }

    #[test]
    #[ignore = "requires screen-recording permission and a GUI session"]
    fn captures_frames_for_three_seconds() {
        let capture =
            MacosCapture::for_display(4).expect("shareable content (screen permission granted?)");
        let count = Arc::new(AtomicUsize::new(0));
        let reporter = Arc::clone(&count);
        capture.start(Box::new(move |frame| {
            let n = reporter.fetch_add(1, Ordering::SeqCst) + 1;
            eprintln!(
                "frame {n}: {}x{} jpeg={}B hash={:#x}",
                frame.width,
                frame.height,
                frame.jpeg.len(),
                frame.hash
            );
        }));
        std::thread::sleep(Duration::from_secs(3));
        capture.stop();
        eprintln!("captured {} frames in ~3s", count.load(Ordering::SeqCst));
    }
}
