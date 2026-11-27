//! The app-gate transitions: `NeedsPermission` ⇄ `Main` recompute and
//! their side-effects (capture enter/leave). `transition_gate` runs after
//! every mutation that can change the gate and once at startup.

use crate::*;

/// `Main` needs both first-run halves: `app.onboarding_done` AND
/// `permissions::screen_status()`. While the wizard runs the gate stays
/// `NeedsPermission`, so `enter_main` (capture) can't
/// fire underneath it — finishing onboarding re-evaluates through
/// `config_set` → `transition_gate`.
pub(crate) fn app_gate(state: &AppState) -> Gate {
    let ready = state.config.lock().app.onboarding_done && permissions::screen_status();
    if ready {
        Gate::Main
    } else {
        Gate::NeedsPermission
    }
}

/// Hop window work onto the main thread — the pool's getters park the
/// caller on the main queue, so running them off a tokio worker is the
/// ABBA deadlock the window-event `try_lock`s exist to avoid. `what`
/// labels the warn; a dispatch failure only logs (fire-and-forget).
pub(crate) fn run_on_main(
    app: &AppHandle,
    what: &'static str,
    work: impl FnOnce(&AppHandle) + Send + 'static,
) {
    let app2 = app.clone();
    if let Err(e) = app.run_on_main_thread(move || work(&app2)) {
        log::warn!("{what}: main-thread hop failed: {e}");
    }
}

/// Recompute the gate, run the transition's side-effects, then ALWAYS
/// emit `app:state` `{"gate": ...}` — called after every mutation that can
/// change the gate and once at startup.
pub(crate) fn transition_gate(app: &AppHandle) {
    let state = app.state::<AppState>();
    // Serialize the whole transition: the gate read+swap, the Main
    // side-effects, and the `app:state` emit are one critical section.
    // `capture_start`, `toggle_capture`, and the picker's `Picked`
    // callback are the other `gate_transition` holders — all follow
    // the field's documented lock order, so this cannot deadlock.
    let _transition = state.gate_transition.lock();
    let new_gate = app_gate(&state);
    let old_gate = std::mem::replace(&mut *state.gate.lock(), new_gate);
    if old_gate != new_gate {
        if new_gate == Gate::Main {
            enter_main(app);
        } else if old_gate == Gate::Main {
            leave_main(app);
        }
    }
    let _ = app.emit("app:state", json!({ "gate": new_gate.name() }));
}

/// `Main` entry: start capture through the shared lifecycle boundary —
/// it warns and continues on failure, so a failed piece never wedges
/// the gate. Settings → Recording's `auto_screenshots` opts out of the
/// ambient start; the bar's record toggle stays a manual override.
pub(crate) fn enter_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.config.lock().recording.auto_screenshots {
        match primary_display_source() {
            Ok((source, w, h)) => {
                start_capture(app, source, w, h, None);
            }
            Err(e) => log::warn!("capture: display source failed: {e}"),
        }
    } else {
        // Still broadcast — listeners resync on every Main entry.
        let status = capture_snapshot(&state);
        emit_capture_state(app, &status);
    }
}

/// `Main` exit (onboarding reset / permission revoked): collapse the
/// card, stop dictation, stop + drop capture, hide the alert toast.
pub(crate) fn leave_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    // A gate exit is teardown, not a view leave — retire every live
    // run (each tagged `idle` still emits so mirrors and packet
    // filters settle), then collapse. Listen is independent of card
    // visibility; only `listen_stop` stops it.
    state.ask.retire_all(app);
    state.ask.close(app, &state.pool);
    // Dictation is bound to the ask input that leaving Main hides — an
    // invisible session must not keep the microphone. (Its epoch also
    // aborts any `dictation_start` still in flight.) Emit the state only
    // when one was live so bar views resync; `dictation_status` covers
    // the rest on mount.
    let dictation_was_live = state.dictation.status().is_listening();
    let _ = state.dictation.stop();
    if dictation_was_live {
        emit_dictation_state(app, &state.dictation.status());
    }
    // Capture teardown shares the command boundary: the capture leaves
    // the mutex before `stop()` joins the worker, then `capture:state`
    // broadcasts the result.
    stop_capture(app);
    // The picker is part of Main too — a gate leave mid-pick would
    // otherwise strand it visible over a hidden bar.
    let pool = state.pool.lock();
    pool.hide_alert();
    pool.hide_picker();
}
