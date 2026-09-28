//! ScreenCaptureKit-backed [`FrameSource`] for macOS.
//!
//! Pipeline (privacy: pixels live only in process memory, never on disk):
//!
//! ```text
//! SCStream ──dispatch queue──> extract_raw (lock, memcpy BGRA, unlock)
//!          ──mpsc──> worker: frame_hash → dedupe → resize ≤1600w → JPEG q80
//!          ──> on_frame(Frame)
//! ```
//!
//! The dispatch-queue callback does no hashing, resizing, or encoding so it
//! can never stall `ScreenCaptureKit`'s delivery queue; everything expensive
//! runs on our worker thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{GenericImageView, Rgba};
use parking_lot::Mutex;
use screencapturekit::cm::{CMSampleBuffer, CMSampleBufferExt, CMTime};
use screencapturekit::screenshot_manager::SCScreenshotManager;
use screencapturekit::shareable_content::SCShareableContent;
use screencapturekit::shareable_content::SCWindow;
use screencapturekit::stream::configuration::{PixelFormat, SCStreamConfiguration};
use screencapturekit::stream::content_filter::SCContentFilter;
use screencapturekit::stream::output_type::SCStreamOutputType;
use screencapturekit::stream::sc_stream::SCStream;

use super::{frame_hash_rows, Frame, FrameSource};

/// Longest-side cap for the encoded frame: width is capped at 1600 px,
/// height follows aspect. Screen reading needs legible text — the old
/// 384 px height cap made on-screen text illegible and the vision model
/// confabulated details. `Frame` widths below the cap pass through.
const TARGET_WIDTH: u32 = 1600;
/// fps → `minimum_frame_interval` seconds (8→0.125, 4→0.25, 2→0.5).
/// The stream is event-driven so this only bounds rate; `fps.max(1)`
/// guards a 0 write from dividing by zero.
pub(crate) fn frame_interval_secs(fps: u32) -> f64 {
    1.0 / f64::from(fps.max(1))
}
/// `frame_hash` samples every 4096th byte of the raw BGRA buffer.
const HASH_STRIDE: usize = 4096;
const JPEG_QUALITY: u8 = 80;
/// Worker polls the channel this often so `stop()` can interrupt a quiet
/// stream even while the sender is still alive.
const POLL: Duration = Duration::from_millis(200);
/// Big-endian FourCC for BGRA — checked per frame so a surprise format can
/// never silently garble colors.
const FOURCC_BGRA: u32 = u32::from_be_bytes(*b"BGRA");

/// Raw BGRA frame handed from the dispatch-queue callback to the worker.
/// `data` is the full locked pixel-buffer copy; `bytes_per_row` may exceed
/// `width * 4` (row alignment padding).
struct RawFrame {
    data: Vec<u8>,
    width: u32,
    height: u32,
    bytes_per_row: usize,
}

/// Zero-copy strided view over raw BGRA rows. `get_pixel` swizzles
/// B,G,R,A → R,G,B,A so `resize` and `JpegEncoder` see true RGBA ordering.
struct BgraView<'a> {
    data: &'a [u8],
    width: u32,
    height: u32,
    bytes_per_row: usize,
}

impl GenericImageView for BgraView<'_> {
    type Pixel = Rgba<u8>;

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn get_pixel(&self, x: u32, y: u32) -> Rgba<u8> {
        let i = y as usize * self.bytes_per_row + x as usize * 4;
        Rgba([
            self.data[i + 2],
            self.data[i + 1],
            self.data[i],
            self.data[i + 3],
        ])
    }
}

/// Screen capture of the primary display. Create with [`MacosCapture::new`]
/// (enumerates shareable content eagerly so permission failures surface at
/// construction, not mid-stream), then drive via [`FrameSource`].
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

/// Build the primary-display filter shared by screen and system-audio streams.
pub(crate) fn primary_display_filter() -> anyhow::Result<(SCContentFilter, u32, u32)> {
    let content = SCShareableContent::get()?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no shareable display"))?;
    let (width, height) = (display.width(), display.height());
    let own_pid = std::process::id() as i32;
    let windows = content.windows();
    let own: Vec<&SCWindow> = windows
        .iter()
        .filter(|window| {
            window
                .owning_application()
                .is_some_and(|app| app.process_id() == own_pid)
        })
        .collect();
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&own)
        .build();
    Ok((filter, width, height))
}

/// One-shot screenshot of the primary display via
/// `SCScreenshotManager` — the "read my screen" path when ambient
/// recording is off. Same filter and encode path as the stream, so a
/// single-shot `Frame` is indistinguishable from a ring frame.
/// `Ok(None)` means the call succeeded but delivered no pixels.
pub(crate) fn shot_fullscreen() -> anyhow::Result<Option<Frame>> {
    let (filter, width, height) = primary_display_filter()?;
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

impl MacosCapture {
    /// Build the filter (primary display, our own windows excluded) and
    /// stream configuration. Does not start capturing.
    ///
    /// # Errors
    ///
    /// Fails if screen-recording permission is missing or no display is
    /// shareable.
    pub fn new(fps: u32) -> anyhow::Result<Self> {
        let (filter, width, height) = primary_display_filter()?;
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

    /// Whether the stream + worker are live — false after a failed `start`
    /// (the error path never fills `running`) or after `stop`.
    pub fn is_running(&self) -> bool {
        self.state.lock().running.is_some()
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

/// BGRA → JPEG `Frame`: optional width-cap downscale, q80 encode, stamp.
/// Shared by the stream worker and the one-shot screenshot so both
/// produce identical `Frame`s. `hash` is the caller's dedupe hash (0
/// for single-shots).
fn encode_frame(raw: &RawFrame, hash: u64) -> Option<Frame> {
    let view = BgraView {
        data: &raw.data,
        width: raw.width,
        height: raw.height,
        bytes_per_row: raw.bytes_per_row,
    };
    let mut jpeg = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY);
    let (out_w, out_h, encoded) = if raw.width > TARGET_WIDTH {
        let out_h = (u64::from(raw.height) * u64::from(TARGET_WIDTH) / u64::from(raw.width)) as u32;
        let out_h = out_h.max(1);
        let resized = imageops::resize(&view, TARGET_WIDTH, out_h, FilterType::Triangle);
        (TARGET_WIDTH, out_h, encoder.encode_image(&resized))
    } else {
        (raw.width, raw.height, encoder.encode_image(&view))
    };
    match encoded {
        Ok(()) => Some(Frame {
            jpeg,
            width: out_w,
            height: out_h,
            ts: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            hash,
        }),
        Err(e) => {
            log::warn!("jpeg encode failed ({out_w}x{out_h}): {e}");
            None
        }
    }
}

/// Worker loop: dedupe by raw-BGRA hash, downscale, JPEG-encode, emit.
/// Exits when `stop` is set or the sender disconnects.
fn run_worker(
    rx: mpsc::Receiver<RawFrame>,
    stop: Arc<AtomicBool>,
    on_frame: Box<dyn Fn(Frame) + Send>,
) {
    let mut last_hash: Option<u64> = None;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let raw = match rx.recv_timeout(POLL) {
            Ok(raw) => raw,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        // Hash pixel bytes only — `bytes_per_row` padding is pool garbage
        // that would defeat dedupe (identical screens → different hashes).
        let hash = frame_hash_rows(
            &raw.data,
            raw.width,
            raw.height,
            raw.bytes_per_row,
            HASH_STRIDE,
        );
        if Some(hash) == last_hash {
            continue; // changed-frames-only: identical screen, drop entirely
        }

        // Leave last_hash untouched on encode failure so the next
        // identical frame retries (unchanged rule).
        if let Some(frame) = encode_frame(&raw, hash) {
            last_hash = Some(hash);
            on_frame(frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::FrameSource;
    use super::{MacosCapture, RawFrame};
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
        assert_eq!(super::frame_interval_secs(8), 0.125);
        assert_eq!(super::frame_interval_secs(4), 0.25);
        assert_eq!(super::frame_interval_secs(2), 0.5);
        assert_eq!(super::frame_interval_secs(0), 1.0); // defensive floor
    }

    #[test]
    fn encode_frame_caps_width_at_target() {
        // 3200x2000 solid-black BGRA (bytes_per_row == width * 4).
        let raw = RawFrame {
            data: vec![0u8; 3200 * 2000 * 4],
            width: 3200,
            height: 2000,
            bytes_per_row: 3200 * 4,
        };
        let frame = super::encode_frame(&raw, 0).expect("encode succeeds");
        assert_eq!(frame.width, 1600);
        assert_eq!(frame.height, 1000);
        assert!(!frame.jpeg.is_empty());
        assert_eq!(frame.hash, 0);
    }

    #[test]
    fn encode_frame_passes_small_sources_through() {
        let raw = RawFrame {
            data: vec![0u8; 800 * 600 * 4],
            width: 800,
            height: 600,
            bytes_per_row: 800 * 4,
        };
        let frame = super::encode_frame(&raw, 7).expect("encode succeeds");
        assert_eq!((frame.width, frame.height), (800, 600));
        assert_eq!(frame.hash, 7);
    }

    #[test]
    #[ignore = "requires screen-recording permission and a GUI session"]
    fn captures_frames_for_three_seconds() {
        let capture = MacosCapture::new(4).expect("shareable content (screen permission granted?)");
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
