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

/** `capture_status` return — `target` is the picked scope label,
 *  null on the auto primary-display path. */
export type CaptureStatus = {
  running: boolean;
  frames: number;
  target: { kind: 'display' | 'window' | 'app'; label: string } | null;
};

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
  /** `true` freezes the bar's position (pointer drags suppressed). */
  bar_locked?: boolean;
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
  /** `'en' | 'zh' | 'ja' | 'ko' | 'fr' | 'es'` — the default output
   * language for chat replies and summaries, and the STT hint. */
  main_language: string;
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

/** `[recording]` section — ambient screen capture + voice-summary prefs. */
export interface RecordingPrefs {
  /** `true` = capture starts on entering Main; `false` = manual only —
   * the bar's record toggle still works. */
  auto_screenshots: boolean;
  /** `8 | 4 | 2` — screen frame-rate cap; a write restarts a live capture. */
  fps: number;
  /** Summary focus instruction (template text or custom); `''` = Meeting. */
  summary_prompt: string;
}

/** `presets_list` row — built-ins (`b:` ids) then `prompts.custom`
 *  (`u:` ids). No kind field: a `text` containing `{input}` expands
 *  into the sent message (with `{lang}` → the param badge's language);
 *  anything else appends silently to that send's system prompt. */
export interface Preset {
  id: string;
  name: string;
  text: string;
}

/** `[prompts]` section — user presets only. */
export interface PromptPrefs {
  custom: Preset[];
}

/** `config_get` / `config_set` return / `config:changed` payload. */
export interface Config {
  app: AppPrefs;
  models: ModelPrefs;
  providers: ProviderPrefs;
  recording: RecordingPrefs;
  hotkeys: Record<string, string>;
  window: WindowPrefs;
  compat: CompatPrefs;
  vision: VisionPrefs;
  prompts: PromptPrefs;
}

/** `session_list` row (storage.rs `Session`; `kind` is the `type` column). */
export interface Session {
  id: number;
  kind: string;
  title: string | null;
  /** Retained recording path (`~/.marvis/audios/recording_*.wav`) — listen only. */
  audio_file: string | null;
  /** STT engine label that captured the session — listen only; null on
   *  ask rows and sessions written before the column existed. */
  stt: string | null;
  started_at: number;
  ended_at: number | null;
  last_active_at: number;
}

/** `session_get` row (storage.rs `Message`). */
export interface Message {
  id: number;
  session_id: number;
  role: string;
  content: string;
  /** Answering-provider metadata — assistant rows only; `null` on rows
   * written before the columns existed and on user rows. */
  provider: string | null;
  model: string | null;
  tokens_in: number | null;
  tokens_out: number | null;
  /** The armed `instruct` preset id — user rows only. */
  preset: string | null;
  ts: number;
}

export interface Transcript {
  id: number;
  session_id: number;
  speaker: 'me' | 'them';
  /** Diarized voice cluster within `speaker`'s channel — null when the
   * session ran without diarization or the turn was unlabelable. */
  speaker_idx: number | null;
  content: string;
  ts: number;
}

export interface ListenSummary {
  id: number;
  session_id: number;
  tldr: string;
  bullets: string[];
  follow_ups: string[];
  topic: string | null;
  created_at: number;
  updated_at: number;
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

/** Fire-and-forget: returns after pre-flight; tokens stream as `ask:*`.
 *  `withScreen` (the bar's Cmd/Ctrl+Enter) is the explicit attach flag —
 *  a screen read runs even when the text shows no intent. `listenId`
 *  binds the send to a listen doc — its own ask session (one chat per
 *  doc), its summary+transcript as the meeting context. `presetId`
 *  arms a preset for this send only; `presetLang` is the `{lang}`
 *  badge's edited value (`undefined` → the configured main language). */
export const askSend = (
  text: string,
  withScreen = false,
  listenId?: number,
  presetId?: string,
  presetLang?: string,
) =>
  invoke<void>('ask_send', {
    text,
    withScreen,
    listenId,
    presetId,
    presetLang,
  });

/** The bar's camera affordance — a screen-only ask (fixed prompt,
 *  frame required). */
export const askSendScreenOnly = () => invoke<void>('ask_send_screen_only');

/** Regenerate the last reply — re-asks the session's last user turn
 *  without persisting a duplicate user row. */
export const askRetry = () => invoke<void>('ask_retry');

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
// presets
// ---------------------------------------------------------------------------

/** The merged preset list — built-ins then customs. */
export const presetsList = () => invoke<Preset[]>('presets_list');

/** Open the preset palette — the small glass overlay left-aligned to
 *  `anchorX` (the composer caret's x in viewport px; omitted →
 *  pointer/center fallback). `query` seeds the filter (a `/token`'s
 *  name part); `focused` picks the mode — wand/right-click pass true
 *  (key-focused menu), a `/`-typed open passes false so the composer
 *  keeps typing (the filter then streams over `palette:query` and nav
 *  keys forward over `palette:key`). */
export const presetsPaletteOpen = (
  anchorX?: number,
  query?: string,
  focused?: boolean,
) => invoke<void>('presets_palette_open', { anchorX, query, focused });

/** A palette row pick — the backend closes the palette, refocuses the
 *  bar, and emits the chosen preset as `bar:preset-pick`. */
export const presetsPaletteSelect = (id: string) =>
  invoke<void>('presets_palette_select', { id });

/** The palette's keyed dismiss (Esc — click-away blur and the bar's
 *  own blur hide it without this call). */
export const presetsPaletteClose = () => invoke<void>('presets_palette_close');

/** Forward a composer key to the unfocused palette — `/`-mode nav
 *  (`ArrowUp`, `ArrowDown`, `Enter`, `Tab`, `Escape`, `Home`, `End`)
 *  rides `palette:key`. */
export const presetsPaletteKey = (key: string) =>
  invoke<void>('presets_palette_key', { key });

/** Push the composer's `/token` as the palette's live filter query. */
export const presetsPaletteQuery = (query: string) =>
  invoke<void>('presets_palette_query', { query });

/** The palette view's content-height report — the window hugs the
 *  list (auto-fit, capped + scrolling server-side). */
export const presetsPaletteHeight = (height: number) =>
  invoke<void>('presets_palette_height', { height });

// ---------------------------------------------------------------------------
// listen
// ---------------------------------------------------------------------------

export interface ListenErrorPayload {
  message: string;
  needs_setup: boolean;
}

export interface ListenStatus {
  state: 'idle' | 'listening' | 'paused' | 'error';
  provider: string | null;
  session_id: number | null;
  turns: number;
  mic: boolean;
  error: ListenErrorPayload | null;
  /** Session start epoch — elapsed = (paused_since ?? now) - started_at - paused_secs. */
  started_at: number | null;
  paused_secs: number;
  paused_since: number | null;
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
export const listenPause = () => invoke<void>('listen_pause');
export const listenResume = () => invoke<void>('listen_resume');
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

export interface SherpaInstalledModel {
  id: string;
  label: string;
  description: string;
  bytes: number;
  source: string;
  /** What the model is for — only `stt` entries may be selected as the
   * transcription model; `speaker-embedding` feeds diarization and
   * `punctuation` restores casing/punctuation in sherpa transcripts. */
  kind: 'stt' | 'speaker-embedding' | 'punctuation';
  installed: boolean;
}

export interface SherpaStatus {
  models: SherpaInstalledModel[];
  download: WhisperDownload | null;
}

export const sherpaStatus = () => invoke<SherpaStatus>('sherpa_status');
export const sherpaDownload = (model: string) =>
  invoke<void>('sherpa_download', { model });
export const sherpaCancelDownload = () =>
  invoke<void>('sherpa_cancel_download');
export const sherpaRemoveModel = (model: string) =>
  invoke<SherpaStatus>('sherpa_remove_model', { model });

// ---------------------------------------------------------------------------
// voice enrollment — the stored voiceprint pins mic speaker 0 to "You"
// ---------------------------------------------------------------------------

export interface VoiceprintStatus {
  enrolled: boolean;
  recording: boolean;
}

export interface VoiceEnrollResult {
  seconds: number;
}

export const voiceprintStatus = () =>
  invoke<VoiceprintStatus>('voiceprint_status');
export const voiceEnrollStart = () => invoke<void>('voice_enroll_start');
export const voiceEnrollStop = () =>
  invoke<VoiceEnrollResult>('voice_enroll_stop');
export const voiceEnrollCancel = () => invoke<void>('voice_enroll_cancel');
export const voiceprintRemove = () => invoke<void>('voiceprint_remove');

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

/** Fire-and-forget `alertShow` — every bar error goes to the alert
 *  window, and a failed toast must never become an unhandled rejection. */
export const raise = (message: string) =>
  void alertShow(message).catch(() => {});

/** The live payload, or `null` — read on mount in case the emit raced. */
export const alertCurrent = () => invoke<AlertPayload | null>('alert_current');

export const alertDismiss = () => invoke<void>('alert_dismiss');

/** Same entry point as `Cmd`/`Ctrl+,` and the tray's Settings item —
 * opens the decorated `prefs` window in settings mode (any gate state). */
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

/** Restore the default bar position — the middle of the primary work
 * area. Persists via the same Moved→debounce write. */
export const windowRecenter = () => invoke<void>('window_recenter');

/** Nearest work-area edge of the live bar — the picker's current value. */
export const windowBarEdge = () => invoke<string>('window_bar_edge');

/** The bar's idle-state right-click — pops the shared native menu
 * (the same items the tray icon shows) under the cursor. */
export const barContextMenu = () => invoke<void>('bar_context_menu');

/** Dev-only (`import.meta.env.DEV`): open THIS window's web inspector —
 * the right-click path for windows with no shared menu (prefs, alert).
 * The Rust side no-ops in release builds. */
export const openDevTools = () => invoke<void>('open_devtools');

/** Reports the whole card's desired TOTAL window height — expanded
 * mode only; the backend clamps [104, free space]. */
export const windowAdjustHeight = (height: number) =>
  invoke<void>('window_adjust_height', { height });

/** Direct card open/close — the mic button's listen mode and the
 * permission-needed collapse use it (toggle would close an open card
 * when the user only wants to switch modes). */
export const windowSetChatOpen = (open: boolean) =>
  invoke<void>('window_set_chat_open', { open });

/** Focus the bar window — the `toggle_input` hotkey's show path calls
 * it so a global-hotkey reveal lands typing in the field. */
export const windowFocusBar = () => invoke<void>('window_focus_bar');

/** The pill⇄input morph resizes the window itself (the capsule IS the
 * window under liquid glass) — report `expanded` so Rust can animate
 * the idle 172 ⇄ expanded 600 width change. */
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

export const captureStart = () => invoke<CaptureStatus>('capture_start');
export const captureStop = () => invoke<CaptureStatus>('capture_stop');
export const captureStatus = () => invoke<CaptureStatus>('capture_status');

/** The record button's start path — the native content-sharing picker
 *  (window / display / application). The Rust command returns `()`, so
 *  the invoke resolves null — the authoritative snapshot (with `target`)
 *  lands as the `capture:state` emit; cancel is a silent no-op. */
export const capturePickAndStart = () => invoke<void>('capture_pick_and_start');

/** One shareable target offered by the picker — meta only; thumbs
 *  arrive over `picker:thumb`. `id` is the opaque `"d:"/"w:"/"a:"`
 *  resolver key `capturePickSelect` echoes back. */
export interface PickCandidate {
  id: string;
  kind: 'display' | 'window' | 'app';
  label: string;
  sub: string | null;
  w: number;
  h: number;
  /** "app" only — the window id whose thumb this card reuses. */
  thumb_of: string | null;
}

/** Idle record button — hides the bar, opens the picker window. */
export const capturePickBegin = () => invoke<void>('capture_pick_begin');
/** Meta list for the picker grid; thumbs follow on `picker:thumb`. */
export const capturePickList = () =>
  invoke<PickCandidate[]>('capture_pick_list');
/** Card click — resolves void; rejects with a string error on
 *  stale ids. */
export const capturePickSelect = (id: string) =>
  invoke<void>('capture_pick_select', { id });
/** Esc / Cancel — drops the picker, restores the bar. */
export const capturePickCancel = () => invoke<void>('capture_pick_cancel');

// ---------------------------------------------------------------------------
// sessions
// ---------------------------------------------------------------------------

export const sessionList = () => invoke<Session[]>('session_list');

export const sessionGet = (id: number) =>
  invoke<Message[]>('session_get', { id });

export const transcriptsFor = (id: number, limit?: number) =>
  invoke<Transcript[]>('transcripts_for', { id, limit });

export const summaryLatest = (id: number) =>
  invoke<ListenSummary | null>('summary_latest', { id });

export const sessionDelete = (id: number) =>
  invoke<void>('session_delete', { id });

/** End the active session of `kind` — ChatSection's "New chat". */
export const sessionEndActive = (kind: string) =>
  invoke<boolean>('session_end_active', { kind });

/** Resume a past chat session — ends the open one, reopens `id`. */
export const sessionResume = (id: number) =>
  invoke<boolean>('session_resume', { id });

// ---------------------------------------------------------------------------
// config / app
// ---------------------------------------------------------------------------

export const configGet = () => invoke<Config>('config_get');

/**
 * Writable keys only: `hotkeys.toggle_input`, `window.bar_x`, `window.bar_y`
 * (number sets, null clears), `app.onboarding_done` (bool),
 * `app.appearance` (`'auto'|'light'|'dark'`), `app.accent` (`'#rrggbb'`,
 * `''` resets), `app.main_language` (`'en'|'zh'|'ja'|'ko'|'fr'|'es'`),
 * `compat.name`, `compat.base_url` (http(s) URL, `''`
 * clears), `vision.provider` (`''` or a vision-capable provider id),
 * `vision.models.<id>` (string; `''` removes),
 * `recording.auto_screenshots` (bool), `recording.fps` (`8|4|2`),
 * `recording.read_interval_secs` (number ≥1 — applies on next
 * capture start), `recording.summary_prompt` (string),
 * `prompts.custom` (array of `{id, name, text}` presets —
 * replaces the whole list). Provider
 * order/switches/models go through `providersReorder`/
 * `providerSetEnabled`/`modelSetSelected`. Every successful write
 * broadcasts `config:changed` and resolves to the full updated config.
 */
export const configSet = (key: string, value: unknown) =>
  invoke<Config>('config_set', { key, value });

export const quitApplication = () => invoke<void>('quit_application');
