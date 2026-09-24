//! Global hotkeys: the only OS-level binding is config-driven
//! (`config.hotkeys.toggle_input`) — Settings → Hotkeys rebinds
//! it, and the bar's movement/snap is pointer-driven so there are no
//! fixed-position shortcuts at all. Every other key is fixed inside
//! the bar webview: `Cmd+,` opens settings while the bar is active,
//! and at the input `Enter` sends, `Shift+Enter` adds a line, and
//! `Cmd+Enter` sends with the current screen frame.
//!
//! Dispatch is decoupled: this module reports [`Action`]s through a
//! caller-supplied closure (lib.rs `hotkey_dispatch`).

use std::collections::BTreeMap;
use std::sync::Arc;

use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::config;

/// A fired hotkey's intent. The dispatch closure maps each variant onto
/// the real `WindowPool`/emit call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `toggle_input` — capsule ⇄ input pill (`Cmd+Alt+Space` by
    /// default). The webview owns the morph; dispatch emits
    /// `bar:toggle-input` to it.
    ToggleInput,
}

/// Config action name (`[hotkeys]` table key) → `Action`.
/// `None` for names this module doesn't know — including the retired
/// `toggle_visibility`/`next_step`/`screen_only`/`show_settings` set
/// (now fixed in-webview keys — see the module doc). Unknown names are
/// skipped with a warning at registration so a stale config key can't
/// kill the rest.
fn action_for(name: &str) -> Option<Action> {
    match name {
        "toggle_input" => Some(Action::ToggleInput),
        _ => None,
    }
}

/// Resolve a hotkey binding into a `Shortcut`.
///
/// `binding` is normally a config accelerator string like
/// `"Cmd+Alt+Space"`; for convenience an action name
/// (`"toggle_input"`) first resolves to its spec-default
/// accelerator. Key tokens are normalized into the
/// `global-hotkey` crate's accelerator grammar before parsing — the
/// crate already accepts every spec spelling verbatim (`/`, `[`, `]`,
/// `Enter`, `Up`, `M`, digits are all valid tokens), so normalization is
/// belt-and-suspenders, but it keeps the accepted surface explicit.
/// Returns `None` on any empty/unknown/mis-ordered token.
pub fn accelerator_for(binding: &str) -> Option<Shortcut> {
    let defaults = config::default_hotkeys();
    let accel = defaults.get(binding).map_or(binding, String::as_str);
    parse_accelerator(accel)
}

/// Normalize `accel` into the `global-hotkey` accelerator grammar and
/// parse it. Symbol tokens are rewritten to the Code names the grammar
/// documents (`/`→`Slash`, `[`→`BracketLeft`, …); named codes
/// (`Enter`, `ArrowUp`/`Up`, `KeyM`/`M`, `Digit1`/`1`) pass through to
/// the crate's own key table. `None` when the rebuilt string still
/// doesn't parse (empty token, unknown key, key-before-modifier order).
fn parse_accelerator(accel: &str) -> Option<Shortcut> {
    let mut tokens = Vec::new();
    for raw in accel.split('+') {
        let token = raw.trim();
        if token.is_empty() {
            return None;
        }
        tokens.push(match token.to_ascii_uppercase().as_str() {
            "COMMAND" | "CMD" | "SUPER" => "Cmd",
            "SHIFT" => "Shift",
            "CONTROL" | "CTRL" => "Ctrl",
            "OPTION" | "ALT" => "Alt",
            "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => "CmdOrCtrl",
            "/" => "Slash",
            "[" => "BracketLeft",
            "]" => "BracketRight",
            "\\" => "Backslash",
            "`" => "Backquote",
            "," => "Comma",
            "." => "Period",
            "-" => "Minus",
            "=" => "Equal",
            ";" => "Semicolon",
            "'" => "Quote",
            _ => token,
        });
    }
    tokens.join("+").parse().ok()
}

/// Every `(shortcut, action)` pair to register — the whole set is the
/// single `[hotkeys]` config action; nothing is hardcoded. Unknown
/// action names and accelerators that don't parse are warned and
/// skipped rather than failing the whole set.
fn bindings(binds: &BTreeMap<String, String>) -> Vec<(Shortcut, Action)> {
    let mut out = Vec::new();
    for (name, accel) in binds {
        let Some(action) = action_for(name) else {
            log::warn!("hotkey: unknown action {name:?} (bound to {accel:?}); skipping");
            continue;
        };
        match accelerator_for(accel) {
            Some(shortcut) => out.push((shortcut, action)),
            None => {
                log::warn!("hotkey: accelerator {accel:?} for action {name:?} didn't parse");
                // Fall back to the spec default — a bad hand-edit must not
                // leave the action (e.g. `toggle_input`) unbound.
                if let Some(default_accel) = crate::config::default_hotkeys().get(name.as_str()) {
                    if let Some(shortcut) = accelerator_for(default_accel) {
                        out.push((shortcut, action));
                    }
                }
            }
        }
    }

    // Dedup by shortcut id — the plugin keys handlers by id, so a config
    // collision would silently overwrite the first action's handler or
    // fail OS registration. First wins.
    let mut seen = std::collections::HashSet::new();
    out.retain(|(s, action)| {
        if seen.insert(s.id()) {
            true
        } else {
            log::warn!("hotkey: dropping duplicate binding {s:?} for {action:?}");
            false
        }
    });
    out
}

/// The `(shortcut, action)` pairs one registration pass installed; the
/// caller's handle for tearing them down or delta-swapping to a new set.
/// `Default` is the empty set — what's live before the first register.
#[derive(Debug, Default)]
pub struct RegisteredHotkeys {
    pairs: Vec<(Shortcut, Action)>,
}

impl RegisteredHotkeys {
    /// Unregister every shortcut in this set. Errors are logged and
    /// skipped — a shortcut the OS already dropped shouldn't block the
    /// rest. Kept for full teardown (e.g. shutdown); set transitions go
    /// through [`swap_hotkey_set`]'s delta path instead.
    #[allow(dead_code)]
    pub fn unregister_all(&self, app: &AppHandle) {
        for (shortcut, _) in &self.pairs {
            if let Err(e) = app.global_shortcut().unregister(*shortcut) {
                log::warn!("hotkey: unregister {shortcut} failed: {e}");
            }
        }
    }
}

/// Register every binding — the recognized `[hotkeys]` config actions
/// (just `toggle_input`). Called at startup; the set is
/// gate-independent, so gate transitions never re-register.
///
/// **Re-registration contract:** on any register error this call
/// unregisters only the shortcuts it registered during this call and
/// returns `Err` — a previously returned [`RegisteredHotkeys`] is never
/// touched, so the old set stays fully active. Callers swapping sets
/// should prefer [`swap_hotkey_set`], which keeps shared pairs live and
/// never re-registers them (macOS refuses duplicate registrations).
pub fn register_all(
    app: &AppHandle,
    binds: &BTreeMap<String, String>,
    dispatch: impl Fn(Action) + Send + Sync + 'static,
) -> anyhow::Result<RegisteredHotkeys> {
    register(app, binds, dispatch)
}

/// [`swap_hotkey_set`] failure: a new pair failed to register and the
/// swap rolled back. `restored` is the surviving live set (kept pairs
/// plus whatever dropped pairs could be re-registered) — the caller must
/// store it so its handle still reflects what's actually bound.
#[derive(Debug)]
pub struct SwapError {
    pub restored: RegisteredHotkeys,
    pub source: anyhow::Error,
}

impl std::fmt::Display for SwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl std::error::Error for SwapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Delta-swap to a new binding set: a pair already live with the same
/// `(shortcut, action)` stays registered untouched (its handler still
/// dispatches correctly); pairs absent from the new set are
/// unregistered; new pairs are registered. This ordering is what makes
/// the swap work on macOS — Carbon's `RegisterEventHotKey` fails with
/// `eventHotKeyExistsErr` on a duplicate combo, so register-then-
/// unregister can never succeed when the sets overlap.
///
/// On register failure, the newly-registered pairs are unwound AND the
/// dropped old pairs are re-registered (best-effort rollback), then a
/// [`SwapError`] carrying the surviving set is returned — the caller's
/// state is as close to the old set as the OS allows.
pub fn swap_hotkey_set(
    app: &AppHandle,
    binds: &BTreeMap<String, String>,
    dispatch: impl Fn(Action) + Send + Sync + 'static,
    prev: RegisteredHotkeys,
) -> Result<RegisteredHotkeys, SwapError> {
    let new_bindings = bindings(binds);
    let (keep, drop_old, add) = plan_swap(&prev.pairs, &new_bindings);

    for (shortcut, _) in &drop_old {
        if let Err(e) = app.global_shortcut().unregister(*shortcut) {
            log::warn!("hotkey: unregister {shortcut} failed: {e}");
        }
    }

    let dispatch: Arc<dyn Fn(Action) + Send + Sync> = Arc::new(dispatch);
    match register_pairs(app, &add, &dispatch) {
        Ok(added) => Ok(RegisteredHotkeys {
            pairs: keep.into_iter().chain(added).collect(),
        }),
        Err(err) => {
            // Best-effort rollback: the just-added pairs were already
            // unwound inside `register_pairs`; re-register the dropped
            // old pairs. Failures are logged — `restored` records only
            // what's actually live.
            let mut restored = keep;
            for &(shortcut, action) in &drop_old {
                match register_pairs(app, &[(shortcut, action)], &dispatch) {
                    Ok(pair) => restored.extend(pair),
                    Err(e) => {
                        log::warn!("hotkey: rollback re-register {shortcut} failed: {e}")
                    }
                }
            }
            Err(SwapError {
                restored: RegisteredHotkeys { pairs: restored },
                source: err,
            })
        }
    }
}

/// `(keep, drop, add)` pair sets from [`plan_swap`] — see its docs for
/// the semantics of each position.
type SwapPlan = (
    Vec<(Shortcut, Action)>,
    Vec<(Shortcut, Action)>,
    Vec<(Shortcut, Action)>,
);

/// Pure delta computation for [`swap_hotkey_set`], split out for tests:
/// `keep` = prev pairs still wanted (stay registered untouched);
/// `drop` = prev pairs absent from `new` (unregister); `add` = `new`
/// pairs not already live (register). Comparison is pair-level — the
/// same shortcut bound to a different action is drop + add, not keep.
fn plan_swap(prev: &[(Shortcut, Action)], new: &[(Shortcut, Action)]) -> SwapPlan {
    let keep: Vec<_> = prev.iter().copied().filter(|p| new.contains(p)).collect();
    let drop: Vec<_> = prev.iter().copied().filter(|p| !new.contains(p)).collect();
    let add: Vec<_> = new.iter().copied().filter(|p| !prev.contains(p)).collect();
    (keep, drop, add)
}

/// Resolve `binds` and register the whole set (the startup path —
/// equivalent to `swap_hotkey_set` with an empty prev, kept separate
/// so that signature stays stable).
fn register(
    app: &AppHandle,
    binds: &BTreeMap<String, String>,
    dispatch: impl Fn(Action) + Send + Sync + 'static,
) -> anyhow::Result<RegisteredHotkeys> {
    let dispatch: Arc<dyn Fn(Action) + Send + Sync> = Arc::new(dispatch);
    let pairs = register_pairs(app, &bindings(binds), &dispatch)?;
    Ok(RegisteredHotkeys { pairs })
}

/// Shared registration loop over concrete `(shortcut, action)` pairs.
/// Each shortcut gets its own `on_shortcut` handler (the crate supports
/// per-shortcut closures) that fires `dispatch(action)` on `Pressed`
/// only — `Released` is ignored. On any register error, shortcuts
/// already registered *by this call* are unregistered and the error
/// propagates.
fn register_pairs(
    app: &AppHandle,
    pairs: &[(Shortcut, Action)],
    dispatch: &Arc<dyn Fn(Action) + Send + Sync>,
) -> anyhow::Result<Vec<(Shortcut, Action)>> {
    let mut registered = Vec::new();
    for &(shortcut, action) in pairs {
        let dispatch = Arc::clone(dispatch);
        let result = app.global_shortcut().on_shortcut(
            shortcut,
            move |_app: &AppHandle, _shortcut: &Shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    dispatch(action);
                }
            },
        );
        if let Err(err) = result {
            for (s, _) in &registered {
                let _ = app.global_shortcut().unregister(*s);
            }
            return Err(err.into());
        }
        registered.push((shortcut, action));
    }
    Ok(registered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_global_shortcut::{Code, Modifiers};

    #[test]
    fn accelerator_for_toggle_input_parses_cmd_alt_space() {
        // Spec default `Cmd+Alt+Space` must parse — `Space` is a named
        // key in the grammar.
        let s = accelerator_for("toggle_input").expect("Cmd+Alt+Space should parse");
        assert_eq!(s.mods, Modifiers::SUPER | Modifiers::ALT);
        assert_eq!(s.key, Code::Space);
        assert_eq!(s, "Cmd+Alt+Space".parse::<Shortcut>().unwrap());
    }

    #[test]
    fn accelerator_for_unknown_returns_none() {
        assert_eq!(accelerator_for("no_such_action"), None);
        assert_eq!(accelerator_for("Cmd+NotAKey"), None);
        assert_eq!(accelerator_for(""), None);
        assert_eq!(accelerator_for("Cmd+"), None);
    }

    #[test]
    fn bracket_tokens_map_to_named_codes() {
        // `Cmd+[` / `Cmd+]` must map to BracketLeft/BracketRight rather
        // than erroring.
        let left = accelerator_for("Cmd+[").unwrap();
        assert_eq!(left.mods, Modifiers::SUPER);
        assert_eq!(left.key, Code::BracketLeft);
        let right = accelerator_for("Cmd+]").unwrap();
        assert_eq!(right.mods, Modifiers::SUPER);
        assert_eq!(right.key, Code::BracketRight);
    }

    #[test]
    fn every_config_default_parses() {
        // Regression net: a default that fails to parse silently loses a
        // core binding at registration time.
        let binds = config::default_hotkeys();
        assert_eq!(binds.len(), 1);
        for (action, accel) in &binds {
            let s = accelerator_for(accel)
                .unwrap_or_else(|| panic!("default {action} = {accel:?} must parse"));
            // Resolving by action name yields the same shortcut.
            assert_eq!(accelerator_for(action), Some(s), "action {action}");
        }
    }

    #[test]
    fn action_for_maps_config_names() {
        assert_eq!(action_for("toggle_input"), Some(Action::ToggleInput));
        // The retired actions (now fixed in-webview keys) and other
        // stale names no longer bind.
        assert_eq!(action_for("toggle_visibility"), None);
        assert_eq!(action_for("next_step"), None);
        assert_eq!(action_for("screen_only"), None);
        assert_eq!(action_for("show_settings"), None);
        assert_eq!(action_for("move_up"), None);
        assert_eq!(action_for("scroll_down"), None);
        assert_eq!(action_for("toggle_click_through"), None);
        assert_eq!(action_for("bogus"), None);
    }

    #[test]
    fn bindings_returns_the_single_toggle_pair() {
        let got = bindings(&config::default_hotkeys());
        assert_eq!(
            got,
            vec![(
                accelerator_for("Cmd+Alt+Space").unwrap(),
                Action::ToggleInput
            )]
        );
    }

    #[test]
    fn plan_swap_identical_sets_is_noop() {
        let binds = config::default_hotkeys();
        let all = bindings(&binds);
        let (keep, drop, add) = plan_swap(&all, &all.clone());
        assert_eq!(keep, all);
        assert!(drop.is_empty());
        assert!(add.is_empty());
    }

    #[test]
    fn plan_swap_rebound_shortcut_is_drop_plus_add() {
        // Rebinding `toggle_input` to a new chord: the old
        // (shortcut, action) pair drops and the new one registers —
        // re-registering over a live combo would hit the OS duplicate
        // error.
        let mut binds = config::default_hotkeys();
        let prev = bindings(&binds);
        binds.insert("toggle_input".to_string(), "Ctrl+Alt+J".to_string());
        let new = bindings(&binds);
        let (keep, drop, add) = plan_swap(&prev, &new);
        let old = accelerator_for("Cmd+Alt+Space").unwrap();
        let rebound = accelerator_for("Ctrl+Alt+J").unwrap();
        assert_eq!(add, vec![(rebound, Action::ToggleInput)]);
        assert_eq!(drop, vec![(old, Action::ToggleInput)]);
        assert!(keep.is_empty());
    }
}
