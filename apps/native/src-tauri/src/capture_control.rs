//! The capture start/stop/toggle boundaries shared by the gate's
//! `enter_main`/`leave_main`, the `capture_*` commands, the picker's
//! `Picked` callback, and app teardown — every caller funnels through
//! these so the ring/reader/restart bookkeeping and the `capture:state`
//! broadcast stay consistent.

use crate::*;

/// The picker-selected capture scope shown by `capture:state` —
/// `None` on the auto primary-display path.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureTarget {
    /// "display" | "window" | "app"
    pub kind: &'static str,
    pub label: String,
}

/// `{"running": bool, "frames": ring.len(), "target": CaptureTarget|null}`
/// — the capture status snapshot returned by the `capture_*` commands
/// and carried by `capture:state`. Frame bytes are never serialized.
pub(crate) type CaptureStatus = serde_json::Value;

/// Live `{"running", "frames", "target"}` snapshot — the single
/// serializer for the status contract.
pub(crate) fn capture_snapshot(state: &AppState) -> CaptureStatus {
    let running = state
        .capture
        .lock()
        .as_ref()
        .is_some_and(PlatformCapture::is_running);
    json!({
        "running": running,
        "frames": state.ring.lock().len(),
        "target": state.capture_target.lock().clone(),
    })
}

/// Broadcast `capture:state` to every window — fire-and-forget like
/// `app:state`; a dead webview must never stall a transition. Also the
/// capture label funnel for the tray menu's Start ⇄ Stop item.
pub(crate) fn emit_capture_state(app: &AppHandle, status: &CaptureStatus) {
    let _ = app.emit("capture:state", status);
    refresh_tray_menu(app);
}

#[derive(Clone)]
pub(crate) struct CaptureRestart {
    pub(crate) source: CaptureSource,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) target: Option<CaptureTarget>,
    pub(crate) resolver_id: Option<String>,
}


/// The one capture-start boundary — `enter_main`, the `capture_start`
/// command, `toggle_capture`, and the picker's start share it (callers
/// own source construction; `target` is the picker-selected scope or
/// `None` on the auto-display path). Idempotent: a live capture
/// returns the current status untouched. Otherwise a fresh
/// [`PlatformCapture`] is built and started with a callback that feeds
/// the screen reader ahead of the ring, then stored only when
/// `is_running()` — a silently-failed start must not block a later
/// retry. Every step warns and continues; `capture:state` carries the
/// result.
pub(crate) fn start_capture(
    app: &AppHandle,
    source: CaptureSource,
    width: u32,
    height: u32,
    target: Option<CaptureTarget>,
) -> CaptureStatus {
    start_capture_with_resolver(app, source, width, height, target, None)
}

pub(crate) fn start_capture_with_resolver(
    app: &AppHandle,
    source: CaptureSource,
    width: u32,
    height: u32,
    target: Option<CaptureTarget>,
    resolver_id: Option<String>,
) -> CaptureStatus {
    let state = app.state::<AppState>();
    // Config reads precede the capture lock (existing rule).
    let (fps, read_interval_secs) = {
        let cfg = state.config.lock();
        (cfg.recording.fps, cfg.recording.read_interval_secs)
    };
    {
        let mut slot = state.capture.lock();
        // A stored capture counts as running only while `is_running`
        // holds — a stopped leftover falls through to `Create` and is
        // replaced by the fresh capture below.
        let mut lifecycle = CaptureLifecycle::default();
        if slot.as_ref().is_some_and(PlatformCapture::is_running) {
            lifecycle.mark_running();
        }
        if lifecycle.start_decision() == StartDecision::Create {
            let restart = CaptureRestart {
                source: source.clone(),
                width,
                height,
                target: target.clone(),
                resolver_id,
            };
            match PlatformCapture::new(source, width, height, fps) {
                Ok(capture) => {
                    let ring = Arc::clone(&state.ring);
                    let reader = Arc::clone(&state.screen_reader);
                    capture.start(Box::new(move |frame| {
                        reader.note_frame(frame.clone());
                        ring.lock().push(frame);
                    }));
                    // Store only on success (`is_running` is false when
                    // SCStream rejected the handler or `start_capture`
                    // failed).
                    if capture.is_running() {
                        *slot = Some(capture);
                        *state.capture_target.lock() = target;
                        *state.capture_restart.lock() = Some(restart);
                        lifecycle.mark_running();
                    } else {
                        log::warn!("gate: screen capture failed to start");
                    }
                }
                Err(e) => log::warn!("gate: capture init failed: {e}"),
            }
        }
    }
    // Background reader: resolve [vision] per read so provider changes
    // take effect live; a missing vision config just skips reads.
    if state
        .capture
        .lock()
        .as_ref()
        .is_some_and(PlatformCapture::is_running)
    {
        let app2 = app.clone();
        let describe: screen_read::Describer = Arc::new(move |frame| {
            let app = app2.clone();
            Box::pin(async move {
                let state = app.state::<AppState>();
                let vision = {
                    let cfg = state.config.lock();
                    let ks = state.keystore.lock();
                    crate::vision_candidate(&cfg, &ks)
                };
                match vision {
                    Some(v) => crate::screen_read::describe_screen(
                        &*v.provider,
                        &frame,
                        &tokio_util::sync::CancellationToken::new(),
                    )
                    .await
                    .map(|o| o.map(|r| r.full)),
                    // No [vision] provider — skip the read quietly
                    // (`Ok(None)` keeps the cache and never warns).
                    None => Ok(None),
                }
            })
        });
        state
            .screen_reader
            .start(describe, Duration::from_secs(read_interval_secs.max(1)));
    }
    let status = capture_snapshot(&state);
    emit_capture_state(app, &status);
    status
}

/// The one capture-stop boundary — `leave_main`, the `capture_stop`
/// command, and app teardown share it. Idempotent: an empty slot is a
/// no-op. The capture is taken out of the mutex and the lock released
/// BEFORE `stop()` — it joins the capture worker, which must not hold
/// `state.capture` while `capture_status` waits on it — then
/// `capture:state` carries the result.
pub(crate) fn stop_capture(app: &AppHandle) -> CaptureStatus {
    let state = app.state::<AppState>();
    let capture = {
        let mut slot = state.capture.lock();
        // Anything stored is a live session to end; `stop()` itself
        // no-ops when the worker already isn't running.
        let mut lifecycle = CaptureLifecycle::default();
        if slot.is_some() {
            lifecycle.mark_running();
        }
        match lifecycle.stop_decision() {
            StopDecision::Stop => {
                let taken = slot.take();
                lifecycle.mark_stopped();
                taken
            }
            StopDecision::Noop => None,
        }
    };
    if let Some(capture) = capture {
        capture.stop();
    }
    state.screen_reader.stop();
    *state.capture_target.lock() = None;
    *state.capture_restart.lock() = None;
    let status = capture_snapshot(&state);
    emit_capture_state(app, &status);
    status
}

/// The `menu.capture` item + `toggle_capture` hotkey shared boundary:
/// stop when live, else the `capture_start` command's gate-checked
/// start — the check and `start_capture` share `gate_transition` so a
/// racing leave-transition can't interleave.
pub(crate) fn toggle_capture(app: &AppHandle) {
    let state = app.state::<AppState>();
    let running = state
        .capture
        .lock()
        .as_ref()
        .is_some_and(PlatformCapture::is_running);
    if running {
        stop_capture(app);
        return;
    }
    let _transition = state.gate_transition.lock();
    if *state.gate.lock() != Gate::Main {
        log::warn!("toggle_capture dropped while gate != Main");
        return;
    }
    match primary_display_source() {
        Ok((source, w, h)) => {
            start_capture(app, source, w, h, None);
        }
        Err(e) => log::warn!("capture: display source failed: {e}"),
    }
}
