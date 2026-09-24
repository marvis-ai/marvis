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

/** `keystore_status` return / `keystore:changed` payload. Masked only —
 * `keys.json` is plaintext on disk but never serialized that way. */
export interface KeystoreStatus {
  /** `[provider, masked | null]` pairs — never plaintext keys. */
  keys: [string, string | null][];
}

/** `app:state` event payload value. */
export type Gate = 'needs_permission' | 'main';

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

/** `model_get_selected` / `model_set_selected` return. `null` from
 * `model_get_selected` means no usable provider (all disabled or
 * unconfigured). */
export interface ModelSelection {
  provider: string;
  model: string;
}

/** `model_validate_key` return — validation failures are data, not errors.
 * Deepgram success includes an explicit no-live-probe message; normal LLM
 * keys are live-validated. */
export type ModelValidation =
  | { ok: true; message?: string }
  | { ok: false; error: string };

/** `[providers]` section — the failover chain + enable switches + the
 * per-provider model memory that replaced `[models] llm_*`. */
export interface ProviderPrefs {
  /** Failover priority — the user's drag order, catalog ids only. */
  order: string[];
  /** Switched-off ids — skipped at ask time, kept in `order`. */
  disabled: string[];
  /** `providers.models.<id>` — each provider's remembered model. */
  models: Record<string, string>;
}

/** `[models]` section of `config.toml` — STT only on the wire; the
 * legacy `llm_*` fields are read-but-never-serialized migration
 * carriers on the Rust side. */
export interface ModelPrefs {
  stt_provider: string;
  stt_model: string;
}

/** `[window]` section — `skip_serializing_if` means keys may be absent. */
export interface WindowPrefs {
  bar_x?: number;
  bar_y?: number;
}

/** `[app]` section — first-run state + the appearance/accent prefs. */
export interface AppPrefs {
  /** `false` until the wizard finishes (or is finished early). */
  onboarding_done: boolean;
  /** `'auto' | 'light' | 'dark'` — validated server-side. */
  appearance: string;
  /** `#rrggbb` — the one hue the UI derives (`--accent` and every
   * `color-mix` off it). `''` in a file reads as the spec default. */
  accent: string;
}

/** `[compat]` section — the OpenAI-compatible endpoint (DESIGN.md §6). */
export interface CompatPrefs {
  name: string;
  base_url: string;
}

/** `[vision]` section — the dedicated screen reader. `provider` is a
 * vision-capable provider id or `''` (off — frames attach to the
 * answering chat provider); `models.<id>` is its per-provider model
 * memory, same shape as `providers.models`. Keys and the compat
 * endpoint are shared with the provider rows above. */
export interface VisionPrefs {
  provider: string;
  models: Record<string, string>;
}

/** `config_get` / `config_set` return / `config:changed` payload. */
export interface Config {
  app: AppPrefs;
  models: ModelPrefs;
  providers: ProviderPrefs;
  hotkeys: Record<string, string>;
  window: WindowPrefs;
  compat: CompatPrefs;
  vision: VisionPrefs;
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

export interface Transcript {
  id: number;
  session_id: number;
  speaker: 'me' | 'them';
  text: string;
  ts: number;
}

export interface ListenSummary {
  id: number;
  session_id: number;
  tldr: string;
  bullets: string[];
  follow_ups: string[];
  topic: string | null;
  ts: number;
}

// ---------------------------------------------------------------------------
// keystore
// ---------------------------------------------------------------------------

export const keystoreStatus = () => invoke<KeystoreStatus>('keystore_status');

/** Stores the key after validation: normal LLM keys are live-probed;
 * Deepgram keys are trimmed and accepted after non-empty shape validation
 * without a live provider probe. */
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
  invoke<ModelSelection | null>('model_get_selected');

/** Writes `providers.models.<id>` — per-provider model memory, not the
 * failover order (that's `providersReorder`). Returns the EFFECTIVE
 * selection (chain head) — `null` when no provider is usable. */
export const modelSetSelected = (provider: string, model: string) =>
  invoke<ModelSelection | null>('model_set_selected', { provider, model });

/** Static per-provider list; Ollama resolves its daemon's `/api/tags`. */
export const modelListAvailable = (provider: string) =>
  invoke<string[]>('model_list_available', { provider });

/** Persist the drag order — must be a permutation of the catalog ids. */
export const providersReorder = (order: string[]) =>
  invoke<Config>('providers_reorder', { order });

/** Flip one provider's enabled switch (`providers.disabled`). */
export const providerSetEnabled = (provider: string, enabled: boolean) =>
  invoke<Config>('provider_set_enabled', { provider, enabled });

// ---------------------------------------------------------------------------
// ask
// ---------------------------------------------------------------------------

/** Fire-and-forget: returns after pre-flight; tokens stream as `ask:*`. */
export const askSend = (text: string) => invoke<void>('ask_send', { text });

/** The bar's camera affordance — same screen-only ask as `Cmd+Shift+S`. */
export const askSendScreenOnly = () => invoke<void>('ask_send_screen_only');

export const askClose = () => invoke<void>('ask_close');

/** `ask_current` return — the in-flight run's resync payload. */
export interface AskCurrent {
  state: 'idle' | 'loading' | 'streaming';
  question: string;
  response: string;
  /** Last `ask:error` payload or `null` — re-delivers a pre-flight
   * error that fired before this webview's `listen()` was up. */
  error: { message: string; needs_setup?: boolean } | null;
}

/** The live ask tail — a re-expanded chat resyncs from this. */
export const askCurrent = () => invoke<AskCurrent>('ask_current');

// ---------------------------------------------------------------------------
// listen
// ---------------------------------------------------------------------------

export interface ListenErrorPayload {
  message: string;
  needs_setup: boolean;
}

export interface ListenStatus {
  state: 'idle' | 'listening' | 'error';
  provider: string | null;
  session_id: number | null;
  turns: number;
  mic: boolean;
  error: ListenErrorPayload | null;
}

export interface VoiceModelCatalogEntry {
  id: 'tiny' | 'base' | 'small';
  filename: string;
  label: string;
  description: string;
  bytes: number;
  source: string;
}

export interface WhisperInstalledModel {
  id: string;
  filename: string;
  installed: boolean;
  bytes: number;
}

export interface WhisperDownload {
  model: string;
  received: number;
  total: number;
}

export type WhisperBinarySource = 'Bundled' | 'Path' | 'Homebrew' | 'User';

export interface WhisperBinaryStatus {
  available: boolean;
  source: WhisperBinarySource | null;
}

export interface WhisperStatus {
  binary: string | null;
  binary_status: WhisperBinaryStatus;
  models: WhisperInstalledModel[];
  download: WhisperDownload | null;
}

export const listenStart = () => invoke<ListenStatus>('listen_start');
export const listenStop = () => invoke<void>('listen_stop');
export const listenStatus = () => invoke<ListenStatus>('listen_status');
export const voiceModelsCatalog = () =>
  invoke<VoiceModelCatalogEntry[]>('voice_models_catalog');
export const whisperStatus = () => invoke<WhisperStatus>('whisper_status');
export const whisperDownload = (model: string) =>
  invoke<void>('whisper_download', { model });
export const whisperCancelDownload = () =>
  invoke<void>('whisper_cancel_download');
export const whisperRemoveModel = (model: string) =>
  invoke<WhisperStatus>('whisper_remove_model', { model });

// ---------------------------------------------------------------------------
// dictation
// ---------------------------------------------------------------------------

export interface DictationErrorPayload {
  message: string;
  needs_setup: boolean;
}

/** `dictation_start` / `dictation_status` return — same wire shape as the
 * `dictation:state` event. */
export interface DictationStatus {
  state: 'idle' | 'listening' | 'error';
  provider: string | null;
  error: DictationErrorPayload | null;
}

/** `dictation_stop` return / `dictation:draft` payload. `final` is
 * serde-renamed from `finality` — only the authoritative stop draft
 * arrives `final: true`. */
export interface DictationDraftPayload {
  text: string;
  final: boolean;
}

/** Mic-only dictation into the Ask input — mutually exclusive with
 * meeting Listen; the backend rejects a conflicting start either way. */
export const dictationStart = () => invoke<DictationStatus>('dictation_start');

/** Idempotent — resolves to the authoritative final draft (empty when
 * nothing was dictated). */
export const dictationStop = () =>
  invoke<DictationDraftPayload>('dictation_stop');

/** Live status for the bar's mount-time resync. */
export const dictationStatus = () =>
  invoke<DictationStatus>('dictation_status');

// ---------------------------------------------------------------------------
// windows
// ---------------------------------------------------------------------------

export const windowToggleAll = () => invoke<void>('window_toggle_all');

// ---------------------------------------------------------------------------
// alert toast
// ---------------------------------------------------------------------------

/** `alert_show` arg / `alert typography` payload — informational only. */
export interface AlertPayload {
  message: string;
}

/** Raise the toast — the bar's only error surface (the pill has no room). */
export const alertShow = (message: string) =>
  invoke<void>('alert_show', { message });

/** The live payload, or `null` — read on mount in case the emit raced. */
export const alertCurrent = () => invoke<AlertPayload | null>('alert_current');

export const alertDismiss = () => invoke<void>('alert_dismiss');

/** Same entry point as `Cmd+,` and the tray's Settings item — opens the
 * decorated `prefs` window in settings mode (any gate state). */
export const windowShowSettings = () => invoke<void>('window_show_settings');

/** The prefs window in onboarding mode — the sidebar's "Re-run setup". */
export const windowShowOnboarding = () =>
  invoke<void>('window_show_onboarding');

export const windowHidePrefs = () => invoke<void>('window_hide_prefs');

/** The mode `prefs` was last shown in — read on mount so a `prefs:mode`
 * emit that raced the loading webview still lands. */
export const prefsMode = () => invoke<string>('prefs_mode');

/** Bar edge picker — `'top'|'bottom'|'left'|'right'`; snaps the bar and
 * the resulting position persists via the Moved→debounce write. */
export const windowSnapEdge = (edge: string) =>
  invoke<void>('window_snap_edge', { edge });

/** Restore the default bar position — centered, 21 px under the primary
 * work area's top. Persists via the same Moved→debounce write. */
export const windowRecenter = () => invoke<void>('window_recenter');

/** Nearest work-area edge of the live bar — the picker's current value. */
export const windowBarEdge = () => invoke<string>('window_bar_edge');

/** Reports the whole card's desired TOTAL window height — expanded
 * mode only; the backend clamps [104, min(900, free space)]. */
export const windowAdjustHeight = (height: number) =>
  invoke<void>('window_adjust_height', { height });

/** Direct card open/close — the mic button's listen mode and the
 * permission-needed collapse use it (toggle would close an open card
 * when the user only wants to switch modes). */
export const windowSetChatOpen = (open: boolean) =>
  invoke<void>('window_set_chat_open', { open });

/** The pill⇄input morph resizes the window itself (the capsule IS the
 * window under liquid glass) — report `expanded` so Rust can animate
 * the idle 112 ⇄ expanded 480 width change. */
export const windowSetBarExpanded = (expanded: boolean) =>
  invoke<void>('window_set_bar_expanded', { expanded });

/** `'glass' | 'vibrancy' | 'none'` — whether a native material backs the
 * window; CSS strips its fake frost when one does. */
export const surfaceMaterial = () =>
  invoke<'glass' | 'vibrancy' | 'none'>('surface_material');

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

export const transcriptsFor = (id: number, limit?: number) =>
  invoke<Transcript[]>('transcripts_for', { id, limit });

export const summaryLatest = (id: number) =>
  invoke<ListenSummary | null>('summary_latest', { id });

export const sessionDelete = (id: number) =>
  invoke<void>('session_delete', { id });

/** End the active session of `kind` — ChatSection's "New chat". */
export const sessionEndActive = (kind: string) =>
  invoke<boolean>('session_end_active', { kind });

// ---------------------------------------------------------------------------
// config / app
// ---------------------------------------------------------------------------

export const configGet = () => invoke<Config>('config_get');

/**
 * Writable keys only: `hotkeys.<action>`, `window.bar_x`, `window.bar_y`
 * (number sets, null clears), `app.onboarding_done` (bool),
 * `app.appearance` (`'auto'|'light'|'dark'`), `app.accent` (`'#rrggbb'`,
 * `''` resets), `compat.name`, `compat.base_url` (http(s) URL, `''`
 * clears), `vision.provider` (`''` or a vision-capable provider id),
 * `vision.models.<id>` (string; `''` removes). Provider
 * order/switches/models go through `providersReorder`/
 * `providerSetEnabled`/`modelSetSelected`. Every successful write
 * broadcasts `config:changed` and resolves to the full updated config.
 */
export const configSet = (key: string, value: unknown) =>
  invoke<Config>('config_set', { key, value });

export const quitApplication = () => invoke<void>('quit_application');
