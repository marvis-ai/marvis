use crate::*;

/// `{"running": bool, "frames": ring.len(), "target": CaptureTarget|null}`
/// — read-only; no emit.
#[tauri::command]
pub(crate) fn capture_status(state: State<'_, AppState>) -> serde_json::Value {
    capture_snapshot(&state)
}

/// Idempotent capture start — the same boundary `enter_main` uses.
/// Emits `capture:state`, then resolves to `{"running", "frames", "target"}`.
#[tauri::command]
pub(crate) fn capture_start(app: AppHandle) -> serde_json::Value {
    let state = app.state::<AppState>();
    // Crafted-invoke guard: the shipped UI disables the toggle outside
    // `Main`, but a crafted invoke during onboarding would otherwise
    // light the recorder while capture doesn't exist yet. The command
    // still resolves the (unchanged) status — the toggle resyncs off
    // `running` either way.
    //
    // The check and the start share `gate_transition` with
    // `transition_gate`: a bare gate read could pass just before a
    // transition swaps the gate and `leave_main` tears capture down,
    // letting this start relight capture outside `Main`.
    let Some(_transition) = state.gate_transition.try_lock() else {
        return capture_snapshot(&state);
    };
    if *state.gate.lock() != Gate::Main {
        log::warn!("capture_start dropped while gate != Main");
        return capture_snapshot(&state);
    }
    match primary_display_source() {
        Ok((source, w, h)) => start_capture(&app, source, w, h, None),
        Err(e) => {
            log::warn!("capture: display source failed: {e}");
            capture_snapshot(&state)
        }
    }
}

/// Idempotent capture stop — the same boundary `leave_main` and app
/// teardown use. Deliberately ungated: stopping must always be safe.
/// Emits `capture:state`, then resolves to `{"running", "frames", "target"}`.
#[tauri::command]
pub(crate) fn capture_stop(app: AppHandle) -> serde_json::Value {
    stop_capture(&app)
}

/// Idle-state record button: the native macOS content-sharing picker
/// (window / display / application) — same UI Zoom shows. Marvis's own
/// bundle id is excluded so it can never offer itself. Cancel is a
/// silent no-op; `capture:state` reports the picked `target`.
/// Display picks are re-resolved to exclude Marvis windows; if that fails,
/// capture uses the picker's original filter without that exclusion.
/// A valid pick replaces a running capture only while the gate remains Main;
/// a pick arriving during a gate transition is discarded. Picker and dispatch
/// failures do not propagate to the caller.
///
/// The picker is a main-thread API (its config setters require it), so
/// the command hops via `run_on_main_thread`; `show` is non-blocking
/// and its `Send` callback fires later with the outcome. On other OSes
/// the in-app picker window (Windows) or the portal dialog (Linux)
/// already covers the flow, so this alias just forwards there.
#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub(crate) fn capture_pick_and_start(app: AppHandle) {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("capture_pick_and_start dropped while gate != Main");
        return;
    }
    let app2 = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        use screencapturekit::content_sharing_picker::*;
        let Some(mut cfg) = SCContentSharingPickerConfiguration::try_new() else {
            log::warn!("capture_pick_and_start: picker unavailable (macOS < 14)");
            return;
        };
        cfg.set_allowed_picker_modes(&[
            SCContentSharingPickerMode::SingleWindow,
            SCContentSharingPickerMode::SingleDisplay,
            SCContentSharingPickerMode::SingleApplication,
        ]);
        cfg.set_excluded_bundle_ids(&["com.getmarvis.marvis"]);
        SCContentSharingPicker::show(&cfg, move |outcome| {
            match outcome {
                SCPickerOutcome::Picked(result) => {
                    // The picker's own filter carries no self-exclusion,
                    // and Marvis's windows are no longer content-protected
                    // — a display pick would composite our bar into the
                    // model's recording. Re-resolve display picks through
                    // `resolve_candidate` (the `d:` path rebuilds the
                    // display filter excluding our application). Window/app
                    // picks can't be ours (`excluded_bundle_ids`), so
                    // their filters pass through untouched.
                    let resolver_id = match result.source() {
                        SCPickedSource::Display(id) => Some(format!("d:{id}")),
                        _ => None,
                    };
                    let (filter, w, h, target) = match result.source() {
                        SCPickedSource::Display(id) => {
                            match capture::resolve_candidate(&format!("d:{id}")) {
                                Ok(res) => (
                                    res.source,
                                    res.w,
                                    res.h,
                                    CaptureTarget {
                                        kind: res.kind,
                                        label: res.label,
                                    },
                                ),
                                // Preserve the current capture if we cannot build
                                // a display filter that excludes Marvis.
                                Err(e) => {
                                    log::warn!(
                                        "capture_pick_and_start: display {id} re-resolve failed: {e}"
                                    );
                                    return;
                                }
                            }
                        }
                        other => {
                            let (w, h) = result.pixel_size();
                            let target = match other {
                                SCPickedSource::Window(t) => CaptureTarget {
                                    kind: "window",
                                    label: t,
                                },
                                SCPickedSource::Application(n) => CaptureTarget {
                                    kind: "app",
                                    label: n,
                                },
                                _ => CaptureTarget {
                                    kind: "app",
                                    label: "Screen".into(),
                                },
                            };
                            (result.filter(), w, h, target)
                        }
                    };
                    // The pick can land after a leave-Main transition ran
                    // `stop_capture` — the invoke-time check can't see it.
                    // Re-check under `gate_transition`, the same critical
                    // section `capture_start` uses (lock order
                    // `gate_transition` → `gate` → `capture`). try_lock:
                    // this callback runs on the main thread, and a worker
                    // holding `gate_transition` blocks on it via
                    // `tray.set_menu` — parking here is an ABBA deadlock.
                    // Contention means a transition is in flight, so
                    // dropping the pick is correct.
                    let state2 = app2.state::<AppState>();
                    let Some(_transition) = state2.gate_transition.try_lock() else {
                        log::warn!(
                            "capture_pick_and_start: pick dropped — gate transition in flight"
                        );
                        return;
                    };
                    if *state2.gate.lock() != Gate::Main {
                        log::warn!("capture_pick_and_start: pick landed after gate left Main");
                        return;
                    }
                    // `start_capture` is idempotent while a capture lives —
                    // a pick made over a running session must retarget, so
                    // stop it first rather than silently keep the old scope.
                    if state2
                        .capture
                        .lock()
                        .as_ref()
                        .is_some_and(PlatformCapture::is_running)
                    {
                        stop_capture(&app2);
                    }
                    start_capture_with_resolver(&app2, filter, w, h, Some(target), resolver_id);
                }
                SCPickerOutcome::Error(e) => {
                    log::warn!("capture_pick_and_start: picker error: {e}");
                }
                SCPickerOutcome::Cancelled => {}
            }
        });
    }) {
        log::warn!("capture_pick_and_start: main-thread hop failed: {e}");
    }
}

/// Non-macOS alias: the record button's "pick and start" is the same
/// custom-picker flow (Windows) or portal dialog (Linux) — whichever
/// `capture_pick_begin` drives on this OS.
#[cfg(not(target_os = "macos"))]
#[tauri::command(async)]
pub(crate) fn capture_pick_and_start(app: AppHandle) {
    capture_pick_begin(app);
}

/// Linux pick flow, run on a worker thread spawned by
/// `capture_pick_begin`: the XDG screencast portal's own dialog IS the
/// picker (Linux exposes no source enumeration to apps), so the command
/// returns immediately and the pick lands here — same post-pick path
/// the macOS native picker callback takes: gate re-check under
/// `gate_transition`, stop a live capture, start the picked source.
/// The bar comes back in every outcome.
#[cfg(target_os = "linux")]
pub(crate) fn portal_pick_flow(app: AppHandle) {
    match capture::portal_pick_blocking() {
        Ok(Some((source, w, h, kind, label))) => {
            let state = app.state::<AppState>();
            let _transition = state.gate_transition.lock();
            if *state.gate.lock() != Gate::Main {
                log::warn!("capture_pick_begin: portal pick landed after gate left Main");
            } else {
                if state
                    .capture
                    .lock()
                    .as_ref()
                    .is_some_and(PlatformCapture::is_running)
                {
                    stop_capture(&app);
                }
                start_capture(&app, source, w, h, Some(CaptureTarget { kind, label }));
            }
        }
        Ok(None) => {} // user cancelled the portal dialog — silent no-op
        Err(e) => log::warn!("capture: portal pick failed: {e}"),
    }
    if let Some(bar) = app.state::<AppState>().pool.lock().bar() {
        let _ = bar.show();
    }
}

/// Idle record button: hide the bar and open the share-picker window.
/// Gate-guarded like `capture_start` — a crafted invoke outside Main
/// must not surface the picker (or a capture behind it). The bar is
/// re-shown by `capture_pick_select`/`capture_pick_cancel`. On Linux
/// the portal's native dialog replaces the picker window entirely.
#[tauri::command(async)]
pub(crate) fn capture_pick_begin(app: AppHandle) {
    let state = app.state::<AppState>();
    let Some(_transition) = state.gate_transition.try_lock() else {
        return;
    };
    if *state.gate.lock() != Gate::Main {
        log::warn!("capture_pick_begin dropped while gate != Main");
        return;
    }
    #[cfg(target_os = "linux")]
    {
        // Hide the bar under the portal dialog, then let a worker own
        // the (user-paced) pick — the command can't block on it.
        if let Some(bar) = state.pool.lock().bar() {
            let _ = bar.hide();
        }
        drop(_transition);
        let app2 = app.clone();
        std::thread::spawn(move || portal_pick_flow(app2));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut pool = state.pool.lock();
        if let Some(bar) = pool.bar() {
            let _ = bar.hide();
        }
        if !pool.show_picker(&app) {
            // A failed build can't be cancelled from a picker that never
            // opened — restore the bar so the UI isn't left hidden.
            if let Some(bar) = pool.bar() {
                let _ = bar.show();
            }
        }
    }
}

/// The picker's candidate list — meta only, returned fast; a detached
/// blocking task then thumbs each candidate and emits `picker:thumb`
/// to the picker window (SCK calls must not run on the async
/// executor; `capture_sample_buffer` is a sync Cocoa call).
/// App cards reuse their largest window's thumb — the map fills as
/// windows emit, so apps need no extra capture. Gate-guarded: a
/// crafted invoke outside Main could otherwise enumerate window
/// titles.
#[cfg(not(target_os = "linux"))]
#[tauri::command]
pub(crate) fn capture_pick_list(app: AppHandle) -> Result<Vec<PickCandidate>, String> {
    let state = app.state::<AppState>();
    let Some(_transition) = state.gate_transition.try_lock() else {
        return Err("capture transition in progress".into());
    };
    if *state.gate.lock() != Gate::Main {
        return Err("picker is only available in the main window".into());
    }
    let metas = capture::pick_candidates().map_err(|e| e.to_string())?;
    let app2 = app.clone();
    let list = metas.clone();
    tauri::async_runtime::spawn(async move {
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let mut thumbs: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for m in &list {
                let jpeg = if m.kind == "app" {
                    m.thumb_of.as_ref().and_then(|w| thumbs.get(w).cloned())
                } else {
                    let j = capture::thumb_for(&m.id);
                    if let Some(j) = &j {
                        thumbs.insert(m.id.clone(), j.clone());
                    }
                    j
                };
                if let Some(jpeg) = jpeg {
                    let _ = app2.emit_to(
                        windows::PICKER_LABEL,
                        "picker:thumb",
                        json!({ "id": m.id, "jpeg": jpeg }),
                    );
                }
            }
        })
        .await;
    });
    Ok(metas)
}

/// Picker card click: re-resolve the id against fresh content, stop a
/// live capture if one raced in, start scoped, restore the bar.
/// Errors on stale ids ("no longer available") — the picker shows it
/// and refetches.
#[cfg(not(target_os = "linux"))]
#[tauri::command]
pub(crate) fn capture_pick_select(app: AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let Some(_transition) = state.gate_transition.try_lock() else {
        return Err("capture transition in progress".into());
    };
    if *state.gate.lock() != Gate::Main {
        return Err("picker is only available in the main window".into());
    }
    let res = capture::resolve_candidate(&id).map_err(|e| e.to_string())?;
    if state
        .capture
        .lock()
        .as_ref()
        .is_some_and(PlatformCapture::is_running)
    {
        stop_capture(&app);
    }
    let target = CaptureTarget {
        kind: res.kind,
        label: res.label,
    };
    start_capture_with_resolver(&app, res.source, res.w, res.h, Some(target), Some(id));
    let pool = state.pool.lock();
    pool.hide_picker();
    if let Some(bar) = pool.bar() {
        let _ = bar.show();
    }
    Ok(())
}

/// Linux: no in-app candidate list exists — the portal's own dialog owns
/// selection, so the picker window is never shown and the frontend never
/// calls this. A defensive empty list keeps the command surface uniform.
#[cfg(target_os = "linux")]
#[tauri::command]
pub(crate) fn capture_pick_list(_app: AppHandle) -> Result<Vec<PickCandidate>, String> {
    Ok(vec![])
}

/// Linux: nothing to resolve — see `capture_pick_list`.
#[cfg(target_os = "linux")]
#[tauri::command]
pub(crate) fn capture_pick_select(_app: AppHandle, _id: String) -> Result<(), String> {
    Err("the portal dialog owns source selection on Linux".into())
}

/// Esc / Cancel: drop the picker, restore the bar. No gate check —
/// cancel must always be safe.
#[tauri::command]
pub(crate) fn capture_pick_cancel(app: AppHandle) {
    let state = app.state::<AppState>();
    let pool = state.pool.lock();
    pool.hide_picker();
    if let Some(bar) = pool.bar() {
        let _ = bar.show();
    }
}

// ---------------------------------------------------------------------------
// Commands — sessions
// ---------------------------------------------------------------------------

