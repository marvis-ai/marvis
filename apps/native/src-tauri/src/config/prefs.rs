use super::*;

/// App-level state that isn't a provider or a window rect: first-run
/// progress, the appearance override, and the accent hue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPrefs {
    /// `true` once the onboarding wizard has run through (or was finished
    /// early). `false` is also the correct read for a config file
    /// written before this field existed — those installs never saw the
    /// wizard, so onboarding re-runs once.
    pub onboarding_done: bool,
    /// `auto` | `light` | `dark`; validated by `config_set`.
    pub appearance: String,
    /// `en | zh | ja | ko | fr | es` — the user's main language: the
    /// chatbox/summary output language. STT always auto-detects the
    /// spoken language instead. Validated by `config_set`.
    pub main_language: String,
    /// `#rrggbb` accent — the single hue the whole UI derives from
    /// (`--accent` in index.css; `--primary`, the soft tint, the text
    /// variant, and the focus ring all `color-mix` off it). Validated by
    /// `config_set`; `""` in a file reads as the default.
    pub accent: String,
}

/// The spec's slate accent — `#3a7294` (DESIGN.md §2).
pub const DEFAULT_ACCENT: &str = "#3a7294";

pub(crate) fn validate_stt_provider(value: &str) -> Result<String, String> {
    let value = value.trim();
    if matches!(value, "deepgram" | "whisper" | "sherpa") {
        Ok(value.to_string())
    } else {
        Err(format!("unknown STT provider {value:?}"))
    }
}

pub(crate) fn validate_stt_model(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err("STT model must not be empty".to_string())
    } else {
        Ok(value.to_string())
    }
}

pub(crate) fn validate_whisper_model(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("STT model must not be empty".to_string());
    }
    entry_for_value(value)
        .map(|entry| entry.id.as_str().to_string())
        .ok_or_else(|| format!("unknown Whisper model {value:?}"))
}

pub(crate) fn validate_sherpa_model(value: &str) -> Result<String, String> {
    crate::sherpa_models::stt_entry_for_value(value.trim())
        .map(|entry| entry.id.as_str().to_string())
        .ok_or_else(|| format!("unknown Sherpa model {value:?}"))
}

pub(crate) fn validate_stt_model_for_provider(
    provider: &str,
    value: &str,
) -> Result<String, String> {
    if provider == "whisper" {
        validate_whisper_model(value)
    } else if provider == "sherpa" {
        validate_sherpa_model(value)
    } else {
        validate_stt_model(value)
    }
}

/// Apply the two STT config command keys. Returns `true` when `key` is an STT
/// key, allowing the command layer to keep its other writable keys separate.
pub(crate) fn apply_stt_config(
    models: &mut ModelPrefs,
    key: &str,
    value: &serde_json::Value,
) -> Result<bool, String> {
    match key {
        "models.stt_provider" => {
            let value = value
                .as_str()
                .ok_or("models.stt_provider must be a string")?;
            let provider = validate_stt_provider(value)?;
            // Provider and model are one transactional preference: never leave
            // a Whisper or Sherpa catalog name paired with another provider.
            if provider == "whisper" && entry_for_value(&models.stt_model).is_none() {
                models.stt_model = "tiny".to_string();
            } else if provider == "deepgram"
                && (entry_for_value(&models.stt_model).is_some()
                    || crate::sherpa_models::stt_entry_for_value(&models.stt_model).is_some())
            {
                models.stt_model = "nova-2".to_string();
            }
            if provider == "sherpa"
                && crate::sherpa_models::stt_entry_for_value(&models.stt_model).is_none()
            {
                models.stt_model = "sense-voice".to_string();
            }
            models.stt_provider = provider;
            Ok(true)
        }
        "models.stt_model" => {
            let value = value.as_str().ok_or("models.stt_model must be a string")?;
            models.stt_model = validate_stt_model_for_provider(&models.stt_provider, value)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// The six `app.main_language` codes (Settings → General).
pub(crate) fn validate_main_language(value: &str) -> Result<String, String> {
    let value = value.trim();
    if matches!(value, "en" | "zh" | "ja" | "ko" | "fr" | "es") {
        Ok(value.to_string())
    } else {
        Err(format!("unknown language {value:?}"))
    }
}

/// Apply the `recording.*` config command keys — same handled-shape as
/// [`apply_stt_config`].
pub(crate) fn apply_recording_config(
    recording: &mut RecordingPrefs,
    key: &str,
    value: &serde_json::Value,
) -> Result<bool, String> {
    match key {
        "recording.auto_screenshots" => {
            recording.auto_screenshots = value
                .as_bool()
                .ok_or("recording.auto_screenshots must be a bool")?;
            Ok(true)
        }
        "recording.fps" => {
            let fps = value.as_u64().ok_or("recording.fps must be a number")?;
            if !matches!(fps, 2 | 4 | 8) {
                return Err(format!("unknown fps {fps}"));
            }
            recording.fps = fps as u32;
            Ok(true)
        }
        "recording.read_interval_secs" => {
            let secs = value
                .as_u64()
                .ok_or("recording.read_interval_secs must be a number")?;
            if secs < 1 {
                return Err("recording.read_interval_secs must be >= 1".into());
            }
            recording.read_interval_secs = secs;
            Ok(true)
        }
        "recording.summary_prompt" => {
            recording.summary_prompt = value
                .as_str()
                .ok_or("recording.summary_prompt must be a string")?
                .trim()
                .to_string();
            Ok(true)
        }
        _ => Ok(false),
    }
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            onboarding_done: false,
            appearance: "auto".into(),
            accent: DEFAULT_ACCENT.into(),
            main_language: "en".into(),
        }
    }
}

/// `[recording]` — ambient screen capture + voice-summary prefs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingPrefs {
    /// `enter_main` auto-starts the ambient screen recorder while the
    /// gate is `Main`; `false` leaves capture manual-only — the bar's
    /// record toggle still starts a session.
    pub auto_screenshots: bool,
    /// Screen frame-rate cap: 8 | 4 | 2 fps.
    pub fps: u32,
    /// Minimum seconds between background screen reads (settle gate is
    /// fixed at ~1s); min 1.
    pub read_interval_secs: u64,
    /// The summary focus instruction appended to the summary system
    /// prompt (template text or custom); `""` reads as Meeting.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub summary_prompt: String,
}

impl Default for RecordingPrefs {
    fn default() -> Self {
        Self {
            auto_screenshots: true,
            fps: 4,
            read_interval_secs: 3,
            summary_prompt: String::new(),
        }
    }
}

/// The OpenAI-compatible endpoint (DESIGN.md §6 BYOK): a display name
/// plus the `https://…/v1` base the `compatible` adapter posts to. The
/// key, if the endpoint needs one, lives in `keys.json` under
/// `"compatible"` — never in this file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompatPrefs {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub base_url: String,
}

/// The dedicated screen reader (`[vision]`): when `provider` names one
/// of the vision-capable providers (`ProviderKind::is_vision`), an ask
/// that has a frame sends it to this model FIRST — the reply is a text
/// description the failover chain then answers over, so chat providers
/// never need image support. Keys and `compat.base_url` are shared with
/// the matching provider row; `providers.order`/`disabled` don't apply
/// here — this pick is independent of the chain. `""` = off: the frame
/// attaches to the answering provider as before.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VisionPrefs {
    /// `"openai"` | `"gemini"` | `"openrouter"` | `"compatible"` | `""`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub provider: String,
    /// Per-provider vision model memory (`vision.models.<id>`) — the
    /// same shape as `providers.models`, so switching the reader away
    /// and back restores its pick. Empty falls back to
    /// `llm::vision_default_model`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, String>,
}

impl VisionPrefs {
    /// The model the reader answers with: its remembered pick, else the
    /// provider's vision default (`""` for compatible — free-text only).
    pub fn model_for(&self, id: &str) -> String {
        if let Some(m) = self.models.get(id).filter(|m| !m.is_empty()) {
            return m.clone();
        }
        ProviderKind::from_str(id)
            .and_then(crate::llm::vision_default_model)
            .unwrap_or_default()
            .to_string()
    }
}

/// `[prompts]` — user-defined prompt presets; built-ins ship in
/// `presets.rs`, never in this file. Custom `id`s are webview-minted
/// `u:` strings; `prompts.custom` writes replace the whole list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptPrefs {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom: Vec<crate::presets::Preset>,
}

/// Apply the `prompts.*` config command keys — same handled-shape as
/// [`apply_stt_config`].
pub(crate) fn apply_prompts_config(
    prompts: &mut PromptPrefs,
    key: &str,
    value: &serde_json::Value,
) -> Result<bool, String> {
    match key {
        "prompts.custom" => {
            let raw: Vec<crate::presets::Preset> = serde_json::from_value(value.clone())
                .map_err(|e| format!("prompts.custom must be an array of presets: {e}"))?;
            *prompts = PromptPrefs {
                custom: crate::presets::validate_custom(raw)?,
            };
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Provider enable/order/model memory (`[providers]`). The ordered list
/// IS the failover chain: asks try providers front-to-back, skipping
/// `disabled` entries and any provider that isn't usable (no key where
/// one is required, no `compat.base_url` for `compatible`, no resolvable
/// model). The drag order in Settings → Providers writes `order`
/// verbatim; the switches write `disabled`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderPrefs {
    /// Provider ids in failover-priority order. Load-time normalization
    /// drops unknown ids, dedupes, and appends any provider the file is
    /// missing in catalog order — so a config written before a provider
    /// existed still picks it up at the end of the chain.
    pub order: Vec<String>,
    /// Ids switched off in settings — skipped at ask time but kept in
    /// `order` so re-enabling restores their priority slot.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub disabled: Vec<String>,
    /// Per-provider model memory (`providers.models.<id>`): the model
    /// each provider answers with when its turn comes. Replaces the
    /// single `[models] llm_*` pair, which `load_from` folds in.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, String>,
}

impl ProviderPrefs {
    /// The model `id` answers with: its remembered pick, else the
    /// provider's first static model (`""` for live-list providers —
    /// Ollama/compatible have no static catalog).
    pub fn model_for(&self, id: &str) -> String {
        if let Some(m) = self.models.get(id).filter(|m| !m.is_empty()) {
            return m.clone();
        }
        ProviderKind::from_str(id)
            .map(crate::llm::static_models)
            .and_then(|list| list.first().copied())
            .unwrap_or_default()
            .to_string()
    }

    /// True when `id` is enabled (absent from `disabled` = enabled).
    pub fn is_enabled(&self, id: &str) -> bool {
        !self.disabled.iter().any(|d| d == id)
    }
}

impl Default for ProviderPrefs {
    fn default() -> Self {
        Self {
            order: ProviderKind::ALL
                .iter()
                .map(|k| k.as_str().to_string())
                .collect(),
            disabled: Vec::new(),
            models: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelPrefs {
    /// Legacy single-provider selection (pre-`[providers]` configs).
    /// Still deserialized so `load_from` can fold it into
    /// `providers.models`; never written back — `skip_serializing`
    /// self-cleans it from `config.toml` on the next save.
    #[serde(skip_serializing)]
    pub llm_provider: String,
    #[serde(skip_serializing)]
    pub llm_model: String,
    pub stt_provider: String,
    pub stt_model: String,
}

impl Default for ModelPrefs {
    fn default() -> Self {
        Self {
            llm_provider: "openai".into(),
            llm_model: "gpt-4o".into(),
            stt_provider: "deepgram".into(),
            stt_model: "nova-2".into(),
        }
    }
}

/// Remembered bar position; `None` until the user moves the window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowPrefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar_y: Option<f64>,
    /// `true` freezes the bar's position — pointer drags are suppressed
    /// webview-side while programmatic moves (snap/re-center) still work.
    #[serde(skip_serializing_if = "is_false")]
    pub bar_locked: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// The configurable bindings (Settings → Hotkeys rebinds each).
/// `toggle_input` morphs the capsule ⇄ input pill; the rest map to the
/// shared menu's actions — the screen-recording toggle, meeting Listen,
/// the history card, and the position lock. Every other key is fixed
/// inside the bar webview: `Cmd+,` opens settings while the bar is
/// active, and at the input `Enter` sends / `Shift+Enter` adds a line /
/// `Cmd+Enter` sends with the current screen frame.
/// `CmdOrCtrl` resolves to Cmd on macOS and Ctrl elsewhere — plain
/// `Cmd` would bind the Win key on Windows/Linux.
pub fn default_hotkeys() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("toggle_input".into(), "CmdOrCtrl+Alt+Space".into()),
        ("toggle_capture".into(), "CmdOrCtrl+Alt+R".into()),
        ("start_listen".into(), "CmdOrCtrl+Alt+T".into()),
        ("show_history".into(), "CmdOrCtrl+Alt+H".into()),
        ("toggle_lock".into(), "CmdOrCtrl+Shift+L".into()),
    ])
}

/// A `[hotkeys]` table may list only user overrides; fill in the spec
/// defaults for every action it doesn't mention, and drop names this
/// build doesn't know (stale `move_up`/`next_step`/`screen_only`/
/// `show_settings` keys from older configs would otherwise sit in the
/// file forever — nothing binds them).
pub(super) fn merge_default_hotkeys<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut map = BTreeMap::<String, String>::deserialize(deserializer)?;
    let defaults = default_hotkeys();
    map.retain(|action, _| defaults.contains_key(action));
    for (action, accel) in defaults {
        map.entry(action).or_insert(accel);
    }
    Ok(map)
}

