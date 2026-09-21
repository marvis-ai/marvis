//! Global hotkeys: config-driven bindings (`config.hotkeys`) plus the
//! hardcoded Glass-parity shortcuts that must always exist.
//!
//! Dispatch is decoupled: this module reports [`Action`]s through a
//! caller-supplied closure — `ask` (Task 12) and `AppState` (Task 14)
//! don't exist yet, so nothing here names them. Task 14 passes a closure
//! that routes each `Action` onto the pool/ask service.

use std::collections::BTreeMap;
use std::sync::Arc;

use tauri::AppHandle;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::config;
use crate::windows::Dir;

/// A fired hotkey's intent. Task 14 maps each variant onto the real
/// `WindowPool`/`ask` calls inside the `dispatch` closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `toggle_visibility` — hide all panels / restore the remembered set.
    ToggleVisibility,
    /// `next_step` — send the current input (or screen-only ask).
    NextStep,
    /// `move_{up,down,left,right}` — step the bar 40 px.
    Move(Dir),
    /// `toggle_click_through` — flip `set_ignore_cursor_events`.
    ToggleClickThrough,
    /// `scroll_up` / `scroll_down` — scroll the ask panel.
    ScrollUp,
    ScrollDown,
    /// Hardcoded `Cmd+Shift+S` — manual screenshot → screen-only ask.
    ScreenOnly,
    /// Hardcoded `Cmd+,` — show the prefs window (also the tray item).
    ShowSettings,
    /// Hardcoded `Cmd+Shift+<n>` — move the bar to display `n` (1-based).
    MoveToDisplay(usize),
    /// Hardcoded `Cmd+Shift+Left/Right` — snap bar to a work-area edge.
    SnapEdge(Dir),
}

/// Config action name (`[hotkeys]` table key) → `Action`.
/// `None` for names this module doesn't know — they're skipped with a
/// warning at registration so a stale config key can't kill the rest.
fn action_for(name: &str) -> Option<Action> {
    Some(match name {
        "toggle_visibility" => Action::ToggleVisibility,
        "next_step" => Action::NextStep,
        "move_up" => Action::Move(Dir::Up),
        "move_down" => Action::Move(Dir::Down),
        "move_left" => Action::Move(Dir::Left),
        "move_right" => Action::Move(Dir::Right),
        "toggle_click_through" => Action::ToggleClickThrough,
        "scroll_up" => Action::ScrollUp,
        "scroll_down" => Action::ScrollDown,
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

/// `Digit1`–`Digit9` for the hardcoded `Cmd+Shift+<n>` display jumps.
const DIGIT_CODES: [Code; 9] = [
    Code::Digit1,
    Code::Digit2,
    Code::Digit3,
    Code::Digit4,
    Code::Digit5,
    Code::Digit6,
    Code::Digit7,
    Code::Digit8,
    Code::Digit9,
];

/// Every `(shortcut, action)` pair to register for `scope`: recognized
/// `[hotkeys]` config entries plus the hardcoded Glass-parity set.
/// Unknown action names and accelerators that don't parse are warned and
/// skipped rather than failing the whole set.
fn bindings(binds: &BTreeMap<String, String>, scope: Scope) -> Vec<(Shortcut, Action)> {
    let mut out = Vec::new();
    for (name, accel) in binds {
        let Some(action) = action_for(name) else {
            log::warn!("hotkey: unknown action {name:?} (bound to {accel:?}); skipping");
            continue;
        };
        if scope == Scope::Limited && action != Action::ToggleVisibility {
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

    // Hardcoded Glass-parity bindings — deliberately not configurable.
    // Edge snap and display jump are allowed while gated; `Cmd+Shift+S`
    // touches capture/ask so it's full-scope only. Built with
    // `Shortcut::new` (infallible) rather than re-parsing a literal.
    let cmd_shift = Modifiers::SUPER | Modifiers::SHIFT;
    for (code, action) in [
        (Code::ArrowLeft, Action::SnapEdge(Dir::Left)),
        (Code::ArrowRight, Action::SnapEdge(Dir::Right)),
    ] {
        out.push((Shortcut::new(Some(cmd_shift), code), action));
    }
    // `Cmd+,` is allowed while gated too (parity with the always-enabled
    // tray item): the dispatch explains it needs an unlock rather than
    // silently doing nothing.
    out.push((
        Shortcut::new(Some(Modifiers::SUPER), Code::Comma),
        Action::ShowSettings,
    ));
    for (i, code) in DIGIT_CODES.iter().enumerate() {
        out.push((
            Shortcut::new(Some(cmd_shift), *code),
            Action::MoveToDisplay(i + 1),
        ));
    }
    if scope == Scope::All {
        out.push((
            Shortcut::new(Some(cmd_shift), Code::KeyS),
            Action::ScreenOnly,
        ));
    }
    // Dedup by shortcut id — the plugin keys handlers by id, so a config
    // collision (e.g. `scroll_up = "Cmd+Shift+S"`) would silently overwrite
    // the first action's handler or fail OS registration. First wins.
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
    /// Everything: all config actions + all hardcoded shortcuts.
    All,
    /// Gated state (Glass `reregister-shortcuts` parity): only
    /// `toggle_visibility` + edge-snap + display-jump — nothing that
    /// touches LLM/capture (no `next_step`, no `Cmd+Shift+S`).
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

/// Register every binding: all recognized `[hotkeys]` config actions plus
/// the full hardcoded Glass-parity set (`Cmd+Shift+S`, `Cmd+Shift+<n>`,
/// `Cmd+Shift+Left/Right`). Called when the app reaches `main` state.
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

/// Limited set for the gated state (Task 14 startup): only
/// `toggle_visibility` plus the hardcoded edge-snap and display-jump
/// shortcuts. Same re-registration contract as [`register_all`].
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
        assert_eq!(binds.len(), 9);
        for (action, accel) in &binds {
            let s = accelerator_for(accel)
                .unwrap_or_else(|| panic!("default {action} = {accel:?} must parse"));
            // Resolving by action name yields the same shortcut.
            assert_eq!(accelerator_for(action), Some(s), "action {action}");
        }
        // Spec spellings → expected codes.
        let expect = [
            ("Cmd+Enter", Code::Enter, Modifiers::SUPER),
            ("Cmd+M", Code::KeyM, Modifiers::SUPER),
            ("Cmd+Up", Code::ArrowUp, Modifiers::SUPER),
            ("Cmd+Down", Code::ArrowDown, Modifiers::SUPER),
            (
                "Cmd+Shift+Down",
                Code::ArrowDown,
                Modifiers::SUPER | Modifiers::SHIFT,
            ),
        ];
        for (accel, key, mods) in expect {
            let s = accelerator_for(accel).unwrap();
            assert_eq!((s.key, s.mods), (key, mods), "accel {accel}");
        }
    }

    #[test]
    fn hardcoded_glass_bindings_parse() {
        for accel in ["Cmd+Shift+S", "Cmd+Shift+Left", "Cmd+Shift+Right"] {
            assert!(accelerator_for(accel).is_some(), "{accel}");
        }
        for n in 1..=9usize {
            let accel = format!("Cmd+Shift+{n}");
            let s = accelerator_for(&accel).unwrap_or_else(|| panic!("{accel}"));
            assert_eq!(s.mods, Modifiers::SUPER | Modifiers::SHIFT);
        }
    }

    #[test]
    fn action_for_maps_config_names() {
        assert_eq!(
            action_for("toggle_visibility"),
            Some(Action::ToggleVisibility)
        );
        assert_eq!(action_for("next_step"), Some(Action::NextStep));
        assert_eq!(action_for("move_up"), Some(Action::Move(Dir::Up)));
        assert_eq!(action_for("move_down"), Some(Action::Move(Dir::Down)));
        assert_eq!(action_for("move_left"), Some(Action::Move(Dir::Left)));
        assert_eq!(action_for("move_right"), Some(Action::Move(Dir::Right)));
        assert_eq!(
            action_for("toggle_click_through"),
            Some(Action::ToggleClickThrough)
        );
        assert_eq!(action_for("scroll_up"), Some(Action::ScrollUp));
        assert_eq!(action_for("scroll_down"), Some(Action::ScrollDown));
        assert_eq!(action_for("bogus"), None);
    }

    #[test]
    fn bindings_limited_keeps_only_gated_set() {
        // While gated: toggle_visibility + edge/display only — nothing
        // that touches LLM/capture (no next_step, no ScreenOnly).
        let binds = config::default_hotkeys();
        let got = bindings(&binds, Scope::Limited);
        let actions: Vec<Action> = got.iter().map(|(_, a)| *a).collect();
        assert!(actions.contains(&Action::ToggleVisibility));
        assert!(actions.contains(&Action::SnapEdge(Dir::Left)));
        assert!(actions.contains(&Action::SnapEdge(Dir::Right)));
        for n in 1..=9 {
            assert!(actions.contains(&Action::MoveToDisplay(n)));
        }
        assert!(!actions.contains(&Action::ScreenOnly));
        assert!(!actions.contains(&Action::NextStep));
        // Settings is reachable while gated (it explains the unlock) —
        // parity with the always-enabled tray item.
        assert!(actions.contains(&Action::ShowSettings));
        assert_eq!(got.len(), 1 + 2 + 9 + 1);
    }

    #[test]
    fn bindings_all_includes_everything() {
        let binds = config::default_hotkeys();
        let got = bindings(&binds, Scope::All);
        let actions: Vec<Action> = got.iter().map(|(_, a)| *a).collect();
        assert!(actions.contains(&Action::ScreenOnly));
        assert!(actions.contains(&Action::NextStep));
        assert!(actions.contains(&Action::ShowSettings));
        assert_eq!(got.len(), 9 + 2 + 9 + 1 + 1);
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
        assert!(dropped.contains(&Action::ToggleClickThrough));
        assert!(!dropped.contains(&Action::ToggleVisibility));
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
        // `scroll_up = "Cmd+M"` collides with `toggle_click_through`'s
        // default — `bindings` dedup gives Cmd+M to `scroll_up` (first in
        // map order). The live Cmd+M pair changes action, so it must be
        // dropped and re-registered: keeping it would leave the old
        // handler firing the wrong action, and re-registering over it
        // would hit the OS duplicate error.
        let mut binds = config::default_hotkeys();
        let prev = bindings(&binds, Scope::All);
        binds.insert("scroll_up".to_string(), "Cmd+M".to_string());
        let new = bindings(&binds, Scope::All);
        let (keep, drop, add) = plan_swap(&prev, &new);
        let cmd_m = accelerator_for("Cmd+M").unwrap();
        assert_eq!(add, vec![(cmd_m, Action::ScrollUp)]);
        assert_eq!(drop.len(), 2);
        assert!(drop.contains(&(cmd_m, Action::ToggleClickThrough)));
        // The old scroll_up binding (Cmd+Shift+Up) is gone entirely.
        let shift_up = accelerator_for("Cmd+Shift+Up").unwrap();
        assert!(drop.contains(&(shift_up, Action::ScrollUp)));
        assert_eq!(keep.len() + drop.len(), prev.len());
    }
}
