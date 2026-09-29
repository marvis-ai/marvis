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
/** Ask stream protocol (ask.rs), emitted to the `bar` window only. */
// `loading` also carries `question` — the run's submitted text.
export const EV_ASK_STATE = 'ask:state'; // { state: 'loading'|'streaming'|'idle', question?: string }
export const EV_ASK_CHUNK = 'ask:chunk'; // { text: string }
export const EV_ASK_DONE = 'ask:done'; // { full, provider, model } — who answered
export const EV_ASK_ERROR = 'ask:error'; // { message: string, needs_setup?: bool }
/** Listen lifecycle, transcript, summary, and terminal error events. */
export const EV_LISTEN_STATE = 'listen:state';
export const EV_LISTEN_TURN = 'listen:turn';
export const EV_LISTEN_SUMMARY = 'listen:summary';
export const EV_LISTEN_ERROR = 'listen:error';
/** Dictation lifecycle, live draft, and terminal error events — emitted to
 * the `bar` window only (lib.rs `emit_dictation_*`). */
export const EV_DICTATION_STATE = 'dictation:state';
export const EV_DICTATION_DRAFT = 'dictation:draft';
export const EV_DICTATION_ERROR = 'dictation:error';
/** Whisper model download byte progress; contains no URL or local path. */
export const EV_WHISPER_DOWNLOAD_PROGRESS = 'whisper:download-progress';
/** Whisper model download failure; contains only the model and safe error text. */
export const EV_WHISPER_DOWNLOAD_ERROR = 'whisper:download-error';
export interface WhisperDownloadProgressPayload {
  model: string;
  received: number;
  total: number;
}
export interface WhisperDownloadErrorPayload {
  model: string;
  message: string;
}
export const EV_SHERPA_DOWNLOAD_PROGRESS = 'sherpa:download-progress';
export const EV_SHERPA_DOWNLOAD_ERROR = 'sherpa:download-error';
export type SherpaDownloadProgressPayload = WhisperDownloadProgressPayload;
export type SherpaDownloadErrorPayload = WhisperDownloadErrorPayload;
export interface ListenStatePayload {
  state: 'idle' | 'listening' | 'paused' | 'error';
  provider: string | null;
  session_id: number | null;
  mic: boolean;
  error: ListenErrorPayload | null;
  started_at: number | null;
  paused_secs: number;
  paused_since: number | null;
}
export interface ListenTurnPayload {
  speaker: 'me' | 'them';
  /** Diarized voice cluster within `speaker`'s channel — null when
   * diarization is off or the turn was unlabelable. */
  speaker_idx: number | null;
  text: string;
  ts: number;
  session_id: number;
  final: boolean;
}
export interface ListenSummaryPayload {
  tldr: string;
  bullets: string[];
  follow_ups: string[];
  topic: string | null;
}
export interface ListenErrorPayload {
  message: string;
  needs_setup: boolean;
}
/** `dictation:state` payload — the whole durable `DictationStatus`. */
export interface DictationStatePayload {
  state: 'idle' | 'listening' | 'error';
  provider: string | null;
  error: DictationErrorPayload | null;
}
/** `dictation:draft` payload — `final` only on the authoritative stop
 * draft; live snapshots are `final: false`. */
export interface DictationDraftPayload {
  text: string;
  final: boolean;
}
export interface DictationErrorPayload {
  message: string;
  needs_setup: boolean;
}
/** Emitted to the `alert` window only — the toast payload (lib.rs
 * `show_alert`); `{ message }`. */
export const EV_ALERT_SHOW = 'alert:show';
/** Emitted to the `bar` window only — the `toggle_input` global hotkey
 * (lib.rs `hotkey_dispatch`). The webview morphs capsule ⇄ input pill;
 * an open card counts as "shown" and collapses. */
export const EV_BAR_TOGGLE_INPUT = 'bar:toggle-input';
/** Emitted to the `bar` window only — the `start_listen` hotkey and the
 * shared menu's Start Listening item. The webview starts a Listen
 * session (a no-op while one is live). */
export const EV_BAR_START_LISTEN = 'bar:start-listen';
/** Emitted to the `bar` window only — the `show_history` hotkey and the
 * shared menu's History item. The webview pins the history section and
 * opens the card. */
export const EV_BAR_SHOW_HISTORY = 'bar:show-history';
/** Emitted to the bar when a frame exists but screen permission was
 * revoked mid-session (ask.rs) — it flips back to its permission card. */
export const EV_CAPTURE_PERMISSION_NEEDED = 'capture:permission-needed'; // { permission: 'screen' }
/** Broadcast to every window after each capture start/stop transition
 * (lib.rs `emit_capture_state` — fired by the `capture_start`/
 * `capture_stop`/`capture_pick_and_start` commands, gate
 * `enter_main`/`leave_main`, and app teardown); payload =
 * `CaptureStatePayload` `{ running, frames, target }`. */
export const EV_CAPTURE_STATE = 'capture:state';
export interface CaptureStatePayload {
  running: boolean;
  frames: number;
  /** The picker-selected capture scope — `null` on the auto
   *  primary-display path and after a stop. */
  target: { kind: 'display' | 'window' | 'app'; label: string } | null;
}
/** Emitted to the picker window by `show_picker` — the view refetches
 * `capture_pick_list` on it. The view ALSO fetches on mount: first
 * open can emit before this webview's listener exists. */
export const EV_PICKER_OPEN = 'picker:open';
/** Per-candidate thumbnail, emitted to the picker as each renders —
 * `{ id, jpeg }` where jpeg is base64 (pick-list background task). */
export const EV_PICKER_THUMB = 'picker:thumb';
export interface PickerThumbPayload {
  id: string;
  jpeg: string;
}
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
 *
 * `active` neutralizes a Tauri race (tauri-apps/tauri#15799): `unlisten`
 * throws when called before the registration eval lands, so the backend
 * listener LEAKS and keeps delivering — under StrictMode's
 * mount→cleanup→mount every event then fires twice (`ask:chunk` doubling
 * streamed text). The flag drops the zombie's deliveries; the catch
 * swallows the racy throw.
 */
export const useTauriEvent = <T>(name: string, cb: (payload: T) => void) => {
  const cbRef = useRef(cb);
  useEffect(() => {
    cbRef.current = cb;
  });

  useEffect(() => {
    let active = true;
    const unlisten = listen<T>(name, (event) => {
      if (active) cbRef.current(event.payload);
    });
    return () => {
      active = false;
      void unlisten.then((u) => u()).catch(() => {});
    };
  }, [name]);
};
