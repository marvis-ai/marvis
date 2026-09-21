/**
 * Tauri event contract — names mirror the constants in
 * `src-tauri/src/{lib,ask}.rs`; change both sides together.
 */
import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';

/** Broadcast to every window: `{ gate: Gate }` (lib.rs `transition_gate`). */
export const EV_APP_STATE = 'app:state';
/** Broadcast after every keystore mutation; payload = `KeystoreStatus`. */
export const EV_KEYSTORE_CHANGED = 'keystore:changed';
/** Ask-window stream protocol (ask.rs), emitted to the `ask` window only. */
// `loading` also carries `question` — the run's submitted text.
export const EV_ASK_STATE = 'ask:state'; // { state: 'loading'|'streaming'|'idle', question?: string }
export const EV_ASK_CHUNK = 'ask:chunk'; // { text: string }
export const EV_ASK_DONE = 'ask:done'; // { full, provider, model } — who answered
export const EV_ASK_ERROR = 'ask:error'; // { message: string, needs_setup?: bool }
export const EV_ASK_SCROLL = 'ask:scroll'; // { dir: 'up'|'down' }
/** Emitted to the `alert` window only — the toast payload (lib.rs
 * `show_alert`); `{ message }`. */
export const EV_ALERT_SHOW = 'alert:show';
/** Broadcast when a frame exists but screen permission was revoked
 * mid-session (ask.rs) — the bar flips back to its permission card. */
export const EV_CAPTURE_PERMISSION_NEEDED = 'capture:permission-needed'; // { permission: 'screen' }
/** Broadcast after every successful `config_set` — payload is the full
 * `Config`, so windows re-render without a second `config_get`. */
export const EV_CONFIG_CHANGED = 'config:changed';
/** Emitted to the `prefs` window only — `{"mode": "settings"|"onboarding"}`.
 * `prefs_mode` is the mount-time read for shows that raced the load. */
export const EV_PREFS_MODE = 'prefs:mode';

/**
 * `listen<T>(name)` with cleanup. Subscribes once per `name`; the callback
 * is held in a ref so a new `cb` identity each render never triggers a
 * re-subscribe (and the ref always points at the latest render's closure).
 */
export function useTauriEvent<T>(name: string, cb: (payload: T) => void) {
  const cbRef = useRef(cb);
  useEffect(() => {
    cbRef.current = cb;
  });

  useEffect(() => {
    const unlisten = listen<T>(name, (event) => cbRef.current(event.payload));
    return () => {
      void unlisten.then((u) => u());
    };
  }, [name]);
}
