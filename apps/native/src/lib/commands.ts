/**
 * Typed wrappers over the Rust command surface in `src-tauri/src/lib.rs`.
 * Names and arg keys must match `tauri::generate_handler!` exactly —
 * Tauri rejects invokes whose arg keys don't line up with the Rust
 * parameter names.
 */
import { invoke } from '@tauri-apps/api/core';

// ---------------------------------------------------------------------------
// Shared payload types (serialized shapes from lib.rs / storage.rs /
// config.rs / permissions.rs — serde field names are authoritative)
// ---------------------------------------------------------------------------

/** `keystore_status` return / `keystore:changed` payload. Masked only. */
export interface KeystoreStatus {
  state: 'Unset' | 'Locked' | 'Unlocked';
  /** `[provider, masked | null]` pairs — never plaintext keys. */
  keys: [string, string | null][];
}

/** `app:state` event payload value. */
export type Gate = 'needs_unlock' | 'needs_permission' | 'main';

export interface AppStatePayload {
  gate: Gate;
}

/** `PermissionState` serde camelCase from permissions.rs. */
export type PermissionState =
  | 'notDetermined'
  | 'restricted'
  | 'denied'
  | 'authorized';

/** `permissions_status` return. */
export interface PermissionsStatus {
  screen: boolean;
  mic: PermissionState;
}

/** `capture_status` return. */
export interface CaptureStatus {
  running: boolean;
  frames: number;
}

/** `model_get_selected` / `model_set_selected` return. */
export interface ModelSelection {
  provider: string;
  model: string;
}

/** `model_validate_key` return — validation failures are data, not errors. */
export type ModelValidation = { ok: true } | { ok: false; error: string };

/** `[models]` section of `config.toml`. */
export interface ModelPrefs {
  llm_provider: string;
  llm_model: string;
  stt_provider: string;
  stt_model: string;
}

/** `[window]` section — `skip_serializing_if` means keys may be absent. */
export interface WindowPrefs {
  bar_x?: number;
  bar_y?: number;
}

/** `config_get` / `config_set` return. */
export interface Config {
  models: ModelPrefs;
  hotkeys: Record<string, string>;
  window: WindowPrefs;
}

/** `session_list` row (storage.rs `Session`; `kind` is the `type` column). */
export interface Session {
  id: number;
  kind: string;
  title: string | null;
  started_at: number;
  ended_at: number | null;
  last_active_at: number;
}

/** `session_get` row (storage.rs `AiMessage`). */
export interface AiMessage {
  id: number;
  session_id: number;
  role: string;
  content: string;
  ts: number;
}

// ---------------------------------------------------------------------------
// keystore
// ---------------------------------------------------------------------------

export const keystoreStatus = () => invoke<KeystoreStatus>('keystore_status');

/**
 * First-run only — creates `keys.enc` and the Keychain DEK item. Silent
 * (no auth prompt — the next `keystoreUnlock` prompts). Errors once a
 * keystore exists.
 */
export const keystoreInit = () => invoke<KeystoreStatus>('keystore_init');

/** Triggers the system-auth prompt (Touch ID / password). */
export const keystoreUnlock = () => invoke<KeystoreStatus>('keystore_unlock');

/** Deletes `keys.enc` → `Unset` (recovery for obsolete/corrupt stores). */
export const keystoreReset = () => invoke<KeystoreStatus>('keystore_reset');

export const keystoreLock = () => invoke<KeystoreStatus>('keystore_lock');

/** Validates the key against the provider BEFORE storing it. */
export const keystoreSetKey = (provider: string, key: string) =>
  invoke<KeystoreStatus>('keystore_set_key', { provider, key });

export const keystoreRemoveKey = (provider: string) =>
  invoke<KeystoreStatus>('keystore_remove_key', { provider });

// ---------------------------------------------------------------------------
// models
// ---------------------------------------------------------------------------

/** Probe a candidate key without storing it. */
export const modelValidateKey = (provider: string, key: string) =>
  invoke<ModelValidation>('model_validate_key', { provider, key });

export const modelGetSelected = () =>
  invoke<ModelSelection>('model_get_selected');

export const modelSetSelected = (provider: string, model: string) =>
  invoke<ModelSelection>('model_set_selected', { provider, model });

/** Static per-provider list; Ollama resolves its daemon's `/api/tags`. */
export const modelListAvailable = (provider: string) =>
  invoke<string[]>('model_list_available', { provider });

// ---------------------------------------------------------------------------
// ask
// ---------------------------------------------------------------------------

/** Fire-and-forget: returns after pre-flight; tokens stream as `ask:*`. */
export const askSend = (text: string) => invoke<void>('ask_send', { text });

/** The bar's camera affordance — same screen-only ask as `Cmd+Shift+S`. */
export const askSendScreenOnly = () => invoke<void>('ask_send_screen_only');

export const askClose = () => invoke<void>('ask_close');

// ---------------------------------------------------------------------------
// windows
// ---------------------------------------------------------------------------

export const windowToggleAll = () => invoke<void>('window_toggle_all');

// ---------------------------------------------------------------------------
// alert toast
// ---------------------------------------------------------------------------

/**
 * `alert_show` arg / `alert:show` payload. `action` is the recovery
 * affordance the toast renders as a button — only `'reset'` exists
 * (`keystore_reset`); omit it for informational alerts, which
 * self-dismiss.
 */
export interface AlertPayload {
  message: string;
  action?: 'reset' | null;
}

/** Raise the toast — the bar's only error surface (the pill has no room). */
export const alertShow = (message: string, action?: 'reset') =>
  invoke<void>('alert_show', { message, action: action ?? null });

/** The live payload, or `null` — read on mount in case the emit raced. */
export const alertCurrent = () => invoke<AlertPayload | null>('alert_current');

export const alertDismiss = () => invoke<void>('alert_dismiss');

/** Same entry point as `Cmd+,` and the tray's Settings item. */
export const windowShowSettings = () => invoke<void>('window_show_settings');

export const windowHideSettings = () => invoke<void>('window_hide_settings');

/** Panels only — `name` is `'ask' | 'listen' | 'settings'`, never `'bar'`. */
export const windowAdjustHeight = (name: string, height: number) =>
  invoke<void>('window_adjust_height', { name, height });

// ---------------------------------------------------------------------------
// permissions / capture
// ---------------------------------------------------------------------------

export const permissionsStatus = () =>
  invoke<PermissionsStatus>('permissions_status');

/** May show the system prompt; backend re-evaluates the gate afterwards. */
export const permissionsRequestScreen = () =>
  invoke<boolean>('permissions_request_screen');

export const permissionsRequestMic = () =>
  invoke<boolean>('permissions_request_mic');

/** `section` is the full pane name, e.g. `'Privacy_ScreenCapture'`. */
export const permissionsOpenPrefs = (section: string) =>
  invoke<void>('permissions_open_prefs', { section });

export const captureStatus = () => invoke<CaptureStatus>('capture_status');

// ---------------------------------------------------------------------------
// sessions
// ---------------------------------------------------------------------------

export const sessionList = () => invoke<Session[]>('session_list');

export const sessionGet = (id: number) =>
  invoke<AiMessage[]>('session_get', { id });

export const sessionDelete = (id: number) =>
  invoke<void>('session_delete', { id });

// ---------------------------------------------------------------------------
// config / app
// ---------------------------------------------------------------------------

export const configGet = () => invoke<Config>('config_get');

/**
 * Writable keys only: `models.llm_provider`, `models.llm_model`,
 * `hotkeys.<action>`, `window.bar_x`, `window.bar_y` (number sets, null
 * clears).
 */
export const configSet = (key: string, value: unknown) =>
  invoke<Config>('config_set', { key, value });

export const quitApplication = () => invoke<void>('quit_application');
