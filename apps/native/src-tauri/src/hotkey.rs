//! Global hotkeys: every binding is config-driven (`config.hotkeys`) —
//! Settings → Hotkeys rebinds any of the four actions, and the bar's
//! movement/snap is pointer-driven so there are no fixed-position
//! shortcuts at all.
//!
//! Dispatch is decoupled: this module reports [`Action`]s through a
//! caller-supplied closure that routes each one onto the pool/ask
//! service (lib.rs `hotkey_dispatch`).

use std::collections::BTreeMap;
use std::sync::Arc;

use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::config;

/// A fired hotkey's intent. The dispatch closure maps each variant onto
/// the real `WindowPool`/`ask` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `toggle_visibility` — hide all panels / restore the remembered set.
    ToggleVisibility,
    /// `next_step` — send the current input (or screen-only ask).
    NextStep,
    /// `screen_only` — manual screenshot → screen-only ask (`Cmd+Shift+S`).
    ScreenOnly,
    /// `show_settings` — the prefs window (`Cmd+,`; also the tray item).
    ShowSettings,
}

/// Config action name (`[hotkeys]` table key) → `Action`.
/// `None` for names this module doesn't know — they're skipped with a
/// warning at registration so a stale config key can't kill the rest.
fn action_for(name: &str) -> Option<Action> {
    Some(match name {
        "toggle_visibility" => Action::ToggleVisibility,
        "next_step" => Action::NextStep,
        "screen_only" => Action::ScreenOnly,
        "show_settings" => Action::ShowSettings,
        _ => return None,
    })
}

/// Resolve a hotkey binding into a `Shortcut`.
///
/// `binding` is normally a config accelerator string like `"Cmd+/"`; for
/// convenience an action name (`"toggle_visibility"`) first resolves to
/// its spec-default accelerator. Key tokens are normalized into the
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

/// Every `(shortcut, action)` pair to register for `scope` — the whole
/// set is the four `[hotkeys]` config actions; nothing is hardcoded.
/// Unknown action names and accelerators that don't parse are warned and
/// skipped rather than failing the whole set.
fn bindings(binds: &BTreeMap<String, String>, scope: Scope) -> Vec<(Shortcut, Action)> {
    let mut out = Vec::new();
    for (name, accel) in binds {
        let Some(action) = action_for(name) else {
            log::warn!("hotkey: unknown action {name:?} (bound to {accel:?}); skipping");
            continue;
        };
        // While gated only the chrome shortcuts stay live — show/hide and
        // settings (the window where the user fixes the gated state).
        // `next_step`/`screen_only` touch LLM/capture, so they're Main-only.
        if scope == Scope::Limited
            && !matches!(action, Action::ToggleVisibility | Action::ShowSettings)
        {
            continue;
        }
        match accelerator_for(accel) {
            Some(shortcut) => out.push((shortcut, action)),
            None => {
                log::warn!("hotkey: accelerator {accel:?} for action {name:?} didn't parse");
                // Fall back to the spec default — a bad hand-edit must not
                // leave the action (e.g. `toggle_visibility`) unbound.
                if let Some(default_accel) = crate::config::default_hotkeys().get(name.as_str()) {
                    if let Some(shortcut) = accelerator_for(default_accel) {
                        out.push((shortcut, action));
                    }
                }
            }
        }
    }

    // Dedup by shortcut id — the plugin keys handlers by id, so a config
    // collision (e.g. `next_step = "Cmd+/"`) would silently overwrite the
    // first action's handler or fail OS registration. First wins.
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

/// Which set of bindings a `register_*` call installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Everything: all four config actions.
    All,
    /// Gated state: only `toggle_visibility` + `show_settings` — nothing
    /// that touches LLM/capture (no `next_step`, no `screen_only`).
    Limited,
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

/// Register every binding: all recognized `[hotkeys]` config actions
/// (`toggle_visibility`, `next_step`, `screen_only`, `show_settings`).
/// Called when the app reaches `main` state.
///
/// **Re-registration contract:** on any register error this call
/// unregisters only the shortcuts it registered during this call and
/// returns `Err` — a previously returned [`RegisteredHotkeys`] is never
/// touched, so the old set stays fully active. Callers swapping sets
/// should prefer [`swap_hotkey_set`], which keeps shared pairs live and
/// never re-registers them (macOS refuses duplicate registrations).
/// Signature kept for the startup path / callers that predate the swap.
#[allow(dead_code)]
pub fn register_all(
    app: &AppHandle,
    binds: &BTreeMap<String, String>,
    dispatch: impl Fn(Action) + Send + Sync + 'static,
) -> anyhow::Result<RegisteredHotkeys> {
    register(app, binds, Scope::All, dispatch)
}

/// Limited set for the gated state (startup): only `toggle_visibility`
/// plus `show_settings` — the chrome shortcuts that don't touch
/// LLM/capture. Same re-registration contract as [`register_all`].
pub fn register_limited(
    app: &AppHandle,
    binds: &BTreeMap<String, String>,
    dispatch: impl Fn(Action) + Send + Sync + 'static,
) -> anyhow::Result<RegisteredHotkeys> {
    register(app, binds, Scope::Limited, dispatch)
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

/// Delta-swap to a new binding set: pairs already live with the same
/// `(shortcut, action)` stay registered untouched (their handler still
/// dispatches correctly); pairs absent from the new set are unregistered;
/// new pairs are registered. This ordering is what makes the swap work
/// on macOS — Carbon's `RegisterEventHotKey` fails with
/// `eventHotKeyExistsErr` on a duplicate combo, so register-then-
/// unregister can never succeed when the sets overlap (limited ⊂ all).
///
/// On register failure, the newly-registered pairs are unwound AND the
/// dropped old pairs are re-registered (best-effort rollback), then a
/// [`SwapError`] carrying the surviving set is returned — the caller's
/// state is as close to the old set as the OS allows.
pub fn swap_hotkey_set(
    app: &AppHandle,
    binds: &BTreeMap<String, String>,
    limited: bool,
    dispatch: impl Fn(Action) + Send + Sync + 'static,
    prev: RegisteredHotkeys,
) -> Result<RegisteredHotkeys, SwapError> {
    let scope = if limited { Scope::Limited } else { Scope::All };
    let new_bindings = bindings(binds, scope);
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

/// Resolve `binds` for `scope` and register the whole set (the
/// `register_*` startup path — equivalent to `swap_hotkey_set` with an
/// empty prev, kept separate so those signatures stay stable).
fn register(
    app: &AppHandle,
    binds: &BTreeMap<String, String>,
    scope: Scope,
    dispatch: impl Fn(Action) + Send + Sync + 'static,
) -> anyhow::Result<RegisteredHotkeys> {
    let dispatch: Arc<dyn Fn(Action) + Send + Sync> = Arc::new(dispatch);
    let pairs = register_pairs(app, &bindings(binds, scope), &dispatch)?;
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
    fn accelerator_for_toggle_visibility_parses_cmd_slash() {
        // Spec default `Cmd+/` must parse — the grammar takes the literal
        // `/` (and our normalized `Slash`) as the key token.
        let s = accelerator_for("toggle_visibility").expect("Cmd+/ should parse");
        assert_eq!(s.mods, Modifiers::SUPER);
        assert_eq!(s.key, Code::Slash);
        assert_eq!(s, "Cmd+/".parse::<Shortcut>().unwrap());
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
        // `Cmd+[` / `Cmd+]` (spec: prev/next response, Phase 3) must map
        // to BracketLeft/BracketRight rather than erroring.
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
        assert_eq!(binds.len(), 4);
        for (action, accel) in &binds {
            let s = accelerator_for(accel)
                .unwrap_or_else(|| panic!("default {action} = {accel:?} must parse"));
            // Resolving by action name yields the same shortcut.
            assert_eq!(accelerator_for(action), Some(s), "action {action}");
        }
        // Spec spellings → expected codes.
        let expect = [
            ("Cmd+Enter", Code::Enter, Modifiers::SUPER),
            ("Cmd+,", Code::Comma, Modifiers::SUPER),
            (
                "Cmd+Shift+S",
                Code::KeyS,
                Modifiers::SUPER | Modifiers::SHIFT,
            ),
        ];
        for (accel, key, mods) in expect {
            let s = accelerator_for(accel).unwrap();
            assert_eq!((s.key, s.mods), (key, mods), "accel {accel}");
        }
    }

    #[test]
    fn action_for_maps_config_names() {
        assert_eq!(
            action_for("toggle_visibility"),
            Some(Action::ToggleVisibility)
        );
        assert_eq!(action_for("next_step"), Some(Action::NextStep));
        assert_eq!(action_for("screen_only"), Some(Action::ScreenOnly));
        assert_eq!(action_for("show_settings"), Some(Action::ShowSettings));
        // Stale names from the old nine-action set no longer bind.
        assert_eq!(action_for("move_up"), None);
        assert_eq!(action_for("scroll_down"), None);
        assert_eq!(action_for("toggle_click_through"), None);
        assert_eq!(action_for("bogus"), None);
    }

    #[test]
    fn bindings_limited_keeps_only_gated_set() {
        // While gated: toggle_visibility + show_settings only — nothing
        // that touches LLM/capture (no next_step, no screen_only).
        // Settings stays live because it's where the user fixes the
        // gated state — parity with the always-enabled tray item.
        let binds = config::default_hotkeys();
        let got = bindings(&binds, Scope::Limited);
        let actions: Vec<Action> = got.iter().map(|(_, a)| *a).collect();
        assert!(actions.contains(&Action::ToggleVisibility));
        assert!(actions.contains(&Action::ShowSettings));
        assert!(!actions.contains(&Action::ScreenOnly));
        assert!(!actions.contains(&Action::NextStep));
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn bindings_all_includes_everything() {
        let binds = config::default_hotkeys();
        let got = bindings(&binds, Scope::All);
        let actions: Vec<Action> = got.iter().map(|(_, a)| *a).collect();
        assert!(actions.contains(&Action::ToggleVisibility));
        assert!(actions.contains(&Action::NextStep));
        assert!(actions.contains(&Action::ScreenOnly));
        assert!(actions.contains(&Action::ShowSettings));
        assert_eq!(got.len(), 4);
    }

    #[test]
    fn settings_is_bound_to_cmd_comma_in_both_scopes() {
        let binds = config::default_hotkeys();
        for scope in [Scope::Limited, Scope::All] {
            let (shortcut, _) = bindings(&binds, scope)
                .into_iter()
                .find(|(_, a)| *a == Action::ShowSettings)
                .unwrap_or_else(|| panic!("ShowSettings must be bound in {scope:?}"));
            assert_eq!(shortcut.mods, Modifiers::SUPER);
            assert_eq!(shortcut.key, Code::Comma);
            assert_eq!(shortcut, accelerator_for("Cmd+,").unwrap());
        }
    }

    #[test]
    fn plan_swap_downgrade_drops_only_nongated_pairs() {
        // All → Limited (leave_main): every limited pair is already live
        // in the full set, so keep = the whole limited set — nothing may
        // be re-registered (macOS would error on the duplicate combo).
        let binds = config::default_hotkeys();
        let all = bindings(&binds, Scope::All);
        let limited = bindings(&binds, Scope::Limited);
        let (keep, drop, add) = plan_swap(&all, &limited);
        assert_eq!(keep, limited);
        assert!(add.is_empty());
        assert_eq!(drop.len(), all.len() - limited.len());
        // The dropped pairs are exactly the LLM/capture-touching ones.
        let dropped: Vec<Action> = drop.iter().map(|(_, a)| *a).collect();
        assert!(dropped.contains(&Action::ScreenOnly));
        assert!(dropped.contains(&Action::NextStep));
        assert!(!dropped.contains(&Action::ToggleVisibility));
        assert!(!dropped.contains(&Action::ShowSettings));
    }

    #[test]
    fn plan_swap_upgrade_adds_only_new_pairs() {
        // Limited → All (enter_main): shared pairs stay live; only the
        // newly allowed actions register.
        let binds = config::default_hotkeys();
        let all = bindings(&binds, Scope::All);
        let limited = bindings(&binds, Scope::Limited);
        let (keep, drop, add) = plan_swap(&limited, &all);
        assert_eq!(keep, limited);
        assert!(drop.is_empty());
        assert_eq!(add.len(), all.len() - limited.len());
        let added: Vec<Action> = add.iter().map(|(_, a)| *a).collect();
        assert!(added.contains(&Action::ScreenOnly));
        assert!(added.contains(&Action::NextStep));
    }

    #[test]
    fn plan_swap_identical_sets_is_noop() {
        let binds = config::default_hotkeys();
        let all = bindings(&binds, Scope::All);
        let (keep, drop, add) = plan_swap(&all, &all.clone());
        assert_eq!(keep, all);
        assert!(drop.is_empty());
        assert!(add.is_empty());
    }

    #[test]
    fn plan_swap_rebound_shortcut_is_drop_plus_add() {
        // `next_step = "Cmd+/"` collides with `toggle_visibility`'s
        // default — `bindings` dedup gives Cmd+/ to `next_step` (first in
        // map order). The live Cmd+/ pair changes action, so it must be
        // dropped and re-registered: keeping it would leave the old
        // handler firing the wrong action, and re-registering over it
        // would hit the OS duplicate error.
        let mut binds = config::default_hotkeys();
        let prev = bindings(&binds, Scope::All);
        binds.insert("next_step".to_string(), "Cmd+/".to_string());
        let new = bindings(&binds, Scope::All);
        let (keep, drop, add) = plan_swap(&prev, &new);
        let cmd_slash = accelerator_for("Cmd+/").unwrap();
        assert_eq!(add, vec![(cmd_slash, Action::NextStep)]);
        assert_eq!(drop.len(), 2);
        assert!(drop.contains(&(cmd_slash, Action::ToggleVisibility)));
        // The old next_step binding (Cmd+Enter) is gone entirely.
        let cmd_enter = accelerator_for("Cmd+Enter").unwrap();
        assert!(drop.contains(&(cmd_enter, Action::NextStep)));
        assert_eq!(keep.len() + drop.len(), prev.len());
    }
}
