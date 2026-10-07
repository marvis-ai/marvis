//! `Windows.Graphics.Capture`-backed [`FrameSource`] for Windows.
//!
//! WGC delivers D3D11 frames on its own dispatcher thread (the crate's
//! `start_free_threaded`); the handler memcpy's the mapped BGRA into a
//! [`RawFrame`] and hands off to the shared `frame_pipe` worker — dedupe,
//! resize, JPEG — identical to the ScreenCaptureKit and PipeWire paths.
//!
//! Windows needs no pre-capture consent for displays or other apps'
//! windows (WGC itself is the consent boundary: apps may opt out), so
//! the custom picker enumerates monitors + windows directly — same
//! card contract the macOS picker produces.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose, Engine as _};
use parking_lot::Mutex;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame as WgcFrame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use super::frame_pipe::{encode_frame, jpeg_at, run_worker, RawFrame};
use super::{frame_interval_secs, Frame, FrameSource, PickCandidate, PickResolution};

/// Smallest window worth listing — drops palette/tooling slivers
/// (same rule as the macOS picker).
const PICK_MIN_W: i32 = 140;
const PICK_MIN_H: i32 = 100;
/// Picker thumbnail width — matches the macOS cards.
const THUMB_WIDTH: u32 = 480;
/// Frame channel depth — a stalled worker drops frames, never stalls WGC.
const FRAME_QUEUE: usize = 2;
/// One-shot captures give up after this — an occluded/minimized window
/// may simply never produce a frame.
const ONESHOT_TIMEOUT: Duration = Duration::from_secs(5);

/// Live capture sessions holding self-protection. WGC can't exclude
/// windows from a monitor grab, so Marvis's HWNDs go
/// `WDA_EXCLUDEFROMCAPTURE` while ANY session is live and back to
/// `WDA_NONE` when the last one ends — the same asymmetry
/// `SCContentFilter` gives macOS for free (visible to the user,
/// invisible to our own captures). Counted, not boolean: a one-shot
/// (`thumb_for`/`shot_fullscreen`) can overlap a running stream, and
/// the lock serializes the flip so a late unprotect can't outlive a
/// concurrent acquire.
static CAPTURE_DEPTH: Mutex<usize> = Mutex::new(0);

/// Whether a Marvis capture session is live — `build_window` reads
/// this so a window born mid-capture starts protected instead of
/// leaking into the stream until it ends.
pub(crate) fn protection_engaged() -> bool {
    *CAPTURE_DEPTH.lock() > 0
}

/// RAII guard: apply display-affinity self-exclusion for a session's
/// lifetime, then restore user-side capturability on the last drop.
struct SelfProtection;

impl SelfProtection {
    fn acquire() -> Self {
        let mut depth = CAPTURE_DEPTH.lock();
        if *depth == 0 {
            set_own_windows_protected(true);
        }
        *depth += 1;
        Self
    }
}

impl Drop for SelfProtection {
    fn drop(&mut self) {
        let mut depth = CAPTURE_DEPTH.lock();
        *depth -= 1;
        if *depth == 0 {
            set_own_windows_protected(false);
        }
    }
}

/// Flip display affinity on every Marvis-owned HWND. Enumeration is
/// fresh on each call — pool windows live for the app's lifetime, so
/// the set only changes when a lazy window (picker/prefs) is built.
fn set_own_windows_protected(protected: bool) {
    let Ok(windows) = Window::enumerate() else {
        return;
    };
    let own_pid = std::process::id();
    let affinity = if protected {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    for w in &windows {
        if w.process_id().map(|p| p == own_pid).unwrap_or(false) {
            unsafe {
                let _ = SetWindowDisplayAffinity(HWND(w.as_raw_hwnd()), affinity);
            }
        }
    }
}

/// What a capture session targets: a resolved monitor or window handle —
/// the WGC equivalent of an `SCContentFilter`.
pub enum Source {
    Monitor(Monitor),
    Window(Window),
}

impl TryFrom<Source> for GraphicsCaptureItemType {
    type Error = windows::core::Error;

    fn try_from(source: Source) -> Result<Self, Self::Error> {
        match source {
            Source::Monitor(m) => m.try_into(),
            Source::Window(w) => w.try_into(),
        }
    }
}

/// The primary display as a `Source` — the auto-capture path shared by
/// `enter_main`/`capture_start`/`toggle_capture`.
pub(crate) fn primary_display_source() -> Result<(Source, u32, u32)> {
    let monitor = Monitor::primary()?;
    let w = monitor.width()?;
    let h = monitor.height()?;
    Ok((Source::Monitor(monitor), w, h))
}

/// Per-session state handed to the capture thread via `Settings::flags`.
struct HandlerFlags {
    tx: SyncSender<RawFrame>,
    warned: Arc<AtomicBool>,
}

struct Handler {
    tx: SyncSender<RawFrame>,
    warned: Arc<AtomicBool>,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = HandlerFlags;
    type Error = anyhow::Error;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            tx: ctx.flags.tx,
            warned: ctx.flags.warned,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut WgcFrame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let buffer = frame.buffer()?;
        if buffer.color_format() != ColorFormat::Bgra8 {
            if !self.warned.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "capture: unexpected WGC format {:?} — dropping frames",
                    buffer.color_format()
                );
            }
            return Ok(());
        }
        let (width, height) = (buffer.width(), buffer.height());
        let mut scratch = Vec::new();
        let data = buffer.as_nopadding_buffer(&mut scratch).to_vec();
        let raw = RawFrame {
            data,
            width,
            height,
            bytes_per_row: width as usize * 4,
        };
        match self.tx.try_send(raw) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {}
        }
        Ok(())
    }

    /// The captured window/display went away — end the session rather
    /// than stream a stale last frame forever.
    fn on_closed(&mut self) -> Result<(), Self::Error> {
        Err(anyhow!("capture item closed"))
    }
}

struct Running {
    control: CaptureControl<Handler, anyhow::Error>,
    worker: JoinHandle<()>,
    stop: Arc<AtomicBool>,
    /// Held for the session's life — `stop`'s field drop restores
    /// user-side capturability once the last session ends.
    _protection: SelfProtection,
}

/// The production [`FrameSource`] on Windows.
pub struct WindowsCapture {
    source: Mutex<Option<Source>>,
    fps: u32,
    state: Mutex<Option<Running>>,
}

impl WindowsCapture {
    pub fn new(source: Source, _width: u32, _height: u32, fps: u32) -> Result<Self> {
        Ok(Self {
            source: Mutex::new(Some(source)),
            fps,
            state: Mutex::new(None),
        })
    }

    fn settings(source: Source, flags: HandlerFlags, fps: u32) -> Settings<HandlerFlags, Source> {
        Settings::new(
            source,
            CursorCaptureSettings::Default,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            // OS-side throttle matching SCK's `minimum_frame_interval`.
            MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(frame_interval_secs(
                fps,
            ))),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            flags,
        )
    }
}

impl FrameSource for WindowsCapture {
    fn start(&self, on_frame: Box<dyn Fn(Frame) + Send>) {
        let Some(source) = self.source.lock().take() else {
            log::warn!("capture: WindowsCapture already consumed");
            return;
        };
        let (tx, rx) = mpsc::sync_channel(FRAME_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || run_worker(rx, stop, on_frame))
        };
        let flags = HandlerFlags {
            tx,
            warned: Arc::new(AtomicBool::new(false)),
        };
        // Protect BEFORE the stream starts so the first delivered frame
        // already excludes our windows; on a failed start the guard
        // drops here and restores capturability.
        let protection = SelfProtection::acquire();
        match Handler::start_free_threaded(Self::settings(source, flags, self.fps)) {
            Ok(control) => {
                *self.state.lock() = Some(Running {
                    control,
                    worker,
                    stop,
                    _protection: protection,
                });
            }
            Err(e) => {
                log::warn!("capture: WGC start failed: {e}");
                stop.store(true, Ordering::Relaxed);
                let _ = worker.join();
            }
        }
    }

    fn stop(&self) {
        let Some(running) = self.state.lock().take() else {
            return;
        };
        if let Err(e) = running.control.stop() {
            log::warn!("capture: WGC stop failed: {e}");
        }
        running.stop.store(true, Ordering::Relaxed);
        let _ = running.worker.join();
    }

    fn is_running(&self) -> bool {
        let state = self.state.lock();
        state.as_ref().is_some_and(|r| !r.control.is_finished())
    }
}

impl Drop for WindowsCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One-shot capture of a resolved `Source` — picker thumbnails and the
/// "read my screen" path share it. `Ok(None)` means WGC produced no
/// frame within [`ONESHOT_TIMEOUT`] (occluded/minimized targets can
/// legitimately do that).
fn oneshot_capture(source: Source, fps: u32) -> Result<Option<RawFrame>> {
    let _protection = SelfProtection::acquire();
    let (tx, rx) = mpsc::sync_channel(FRAME_QUEUE);
    let flags = HandlerFlags {
        tx,
        warned: Arc::new(AtomicBool::new(false)),
    };
    let control = Handler::start_free_threaded(WindowsCapture::settings(source, flags, fps))
        .map_err(|e| anyhow!("WGC start failed: {e}"))?;
    let raw = rx.recv_timeout(ONESHOT_TIMEOUT).ok();
    if let Err(e) = control.stop() {
        log::warn!("capture: WGC oneshot stop failed: {e}");
    }
    Ok(raw)
}

/// One-shot screenshot of the primary display — the "read my screen"
/// path when ambient recording is off.
pub(crate) fn shot_fullscreen() -> Result<Option<Frame>> {
    let (source, _, _) = primary_display_source()?;
    Ok(oneshot_capture(source, 4)?
        .map(|raw| encode_frame(&raw, 0))
        .flatten())
}

// ---------------------------------------------------------------------------
// Picker — enumerate monitors + windows for the in-app picker window.
// ---------------------------------------------------------------------------

/// Candidates for the custom picker: displays first (matching macOS's
/// ordering), then windows. `id`s re-resolve against fresh enumeration
/// on pick — `d:`/`w:` keys carry raw HMONITOR/HWND values.
pub(crate) fn pick_candidates() -> Result<Vec<PickCandidate>> {
    let mut out = Vec::new();
    for (index, monitor) in Monitor::enumerate()?.into_iter().enumerate() {
        let (w, h) = (monitor.width().unwrap_or(0), monitor.height().unwrap_or(0));
        let name = monitor.name().unwrap_or_else(|_| format!("{}", index + 1));
        out.push(PickCandidate {
            id: format!("d:{}", monitor.as_raw_hmonitor() as usize),
            kind: "display",
            label: format!("Display {name}"),
            sub: None,
            w,
            h,
            thumb_of: None,
        });
    }
    let own_pid = std::process::id();
    for window in Window::enumerate()? {
        if !window.is_valid() {
            continue;
        }
        if window.process_id().map(|p| p == own_pid).unwrap_or(false) {
            continue; // never offer Marvis's own windows
        }
        let (w, h) = (window.width().unwrap_or(0), window.height().unwrap_or(0));
        if w < PICK_MIN_W || h < PICK_MIN_H {
            continue;
        }
        let title = window.title().unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        out.push(PickCandidate {
            id: format!("w:{}", window.as_raw_hwnd() as usize),
            kind: "window",
            label: title,
            sub: window.process_name().ok(),
            w: w as u32,
            h: h as u32,
            thumb_of: None,
        });
    }
    Ok(out)
}

/// Re-resolve a picker `id` against fresh enumeration — the source the
/// stream will actually capture plus the `capture:state` target fields.
pub(crate) fn resolve_candidate(id: &str) -> Result<PickResolution> {
    let (kind, raw) = id
        .split_once(':')
        .ok_or_else(|| anyhow!("malformed pick id"))?;
    let handle = raw
        .parse::<usize>()
        .map_err(|_| anyhow!("malformed pick id"))? as *mut std::ffi::c_void;
    match kind {
        "d" => {
            let monitor = Monitor::enumerate()?
                .into_iter()
                .find(|m| m.as_raw_hmonitor() == handle)
                .ok_or_else(|| anyhow!("display no longer available"))?;
            Ok(PickResolution {
                w: monitor.width()?,
                h: monitor.height()?,
                kind: "display",
                label: format!("Display {}", monitor.name().unwrap_or_else(|_| "?".into())),
                source: Source::Monitor(Monitor::from_raw_hmonitor(handle)),
            })
        }
        "w" => {
            let window = Window::enumerate()?
                .into_iter()
                .find(|w| w.as_raw_hwnd() == handle)
                .ok_or_else(|| anyhow!("window no longer available"))?;
            let title = window.title().unwrap_or_else(|_| "Window".into());
            Ok(PickResolution {
                w: window.width().unwrap_or(0) as u32,
                h: window.height().unwrap_or(0) as u32,
                kind: "window",
                label: title,
                source: Source::Window(Window::from_raw_hwnd(handle)),
            })
        }
        _ => Err(anyhow!("unknown pick kind")),
    }
}

/// One ~`THUMB_WIDTH`-wide JPEG for a candidate — the picker card's
/// preview image, base64'd for the `picker:thumb` event payload.
pub(crate) fn thumb_for(id: &str) -> Option<String> {
    let res = resolve_candidate(id).ok()?;
    let w = res.w;
    let raw = oneshot_capture(res.source, 1).ok()??;
    let (jpeg, _, _) = jpeg_at(&raw, THUMB_WIDTH.min(w.max(1)))?;
    Some(general_purpose::STANDARD.encode(jpeg))
}
