use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::llm::ProviderKind;
use crate::paths;
use crate::voice_models::entry_for_value;

/// Non-secret preferences, persisted as `~/.marvis/config.toml` (0644).
///
/// Every section and field is `#[serde(default)]`: a partial or missing
/// section deserializes to defaults instead of erroring or resetting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub app: AppPrefs,
    pub models: ModelPrefs,
    /// Provider enable/order/model memory — see [`ProviderPrefs`].
    pub providers: ProviderPrefs,
    /// Ambient screen capture + voice-summary prefs — see [`RecordingPrefs`].
    pub recording: RecordingPrefs,
    #[serde(
        default = "default_hotkeys",
        deserialize_with = "merge_default_hotkeys"
    )]
    pub hotkeys: BTreeMap<String, String>,
    pub window: WindowPrefs,
    pub compat: CompatPrefs,
    /// The dedicated screen reader — see [`VisionPrefs`].
    pub vision: VisionPrefs,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app: AppPrefs::default(),
            models: ModelPrefs::default(),
            providers: ProviderPrefs::default(),
            recording: RecordingPrefs::default(),
            hotkeys: default_hotkeys(),
            window: WindowPrefs::default(),
            compat: CompatPrefs::default(),
            vision: VisionPrefs::default(),
        }
    }
}

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
pub fn default_hotkeys() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("toggle_input".into(), "Cmd+Alt+Space".into()),
        ("toggle_capture".into(), "Cmd+Alt+R".into()),
        ("start_listen".into(), "Cmd+Alt+T".into()),
        ("show_history".into(), "Cmd+Alt+H".into()),
        ("toggle_lock".into(), "Cmd+Shift+L".into()),
    ])
}

/// A `[hotkeys]` table may list only user overrides; fill in the spec
/// defaults for every action it doesn't mention, and drop names this
/// build doesn't know (stale `move_up`/`next_step`/`screen_only`/
/// `show_settings` keys from older configs would otherwise sit in the
/// file forever — nothing binds them).
fn merge_default_hotkeys<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
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

impl Config {
    /// Load from an explicit path. Missing file → defaults.
    pub fn load_from(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        let mut cfg: Self = toml::from_str(&text)?;
        cfg.normalize();
        Ok(cfg)
    }

    /// Post-load hygiene: `providers.order`/`disabled`/`models` drop
    /// unknown ids and fill gaps in catalog order, and a legacy
    /// `[models] llm_*` selection migrates into `providers.models`
    /// (only when that provider has no remembered model yet).
    fn normalize(&mut self) {
        let known = |id: &str| ProviderKind::from_str(id).is_some();

        let mut order: Vec<String> = Vec::new();
        for id in &self.providers.order {
            if known(id) && !order.contains(id) {
                order.push(id.clone());
            }
        }
        for kind in ProviderKind::ALL {
            let id = kind.as_str();
            if !order.iter().any(|o| o == id) {
                order.push(id.to_string());
            }
        }
        self.providers.order = order;

        self.providers.disabled.retain(|id| known(id));
        self.providers.disabled.dedup();
        self.providers
            .models
            .retain(|id, m| known(id) && !m.is_empty());

        if known(&self.models.llm_provider) && !self.models.llm_model.is_empty() {
            self.providers
                .models
                .entry(self.models.llm_provider.clone())
                .or_insert_with(|| self.models.llm_model.clone());
        }

        // The vision pick is stricter than the chain's: an id outside
        // `is_vision` (or junk) turns the reader off rather than letting
        // a hand-edited file name a provider the UI can't represent.
        let vision = |id: &str| ProviderKind::from_str(id).is_some_and(|k| k.is_vision());
        if !self.vision.provider.is_empty() && !vision(&self.vision.provider) {
            self.vision.provider.clear();
        }
        self.vision
            .models
            .retain(|id, m| vision(id) && !m.is_empty());

        if !matches!(
            self.app.main_language.as_str(),
            "en" | "zh" | "ja" | "ko" | "fr" | "es"
        ) {
            self.app.main_language = "en".into();
        }
        if !matches!(self.recording.fps, 2 | 4 | 8) {
            self.recording.fps = 4;
        }
        self.recording.read_interval_secs = self.recording.read_interval_secs.max(1);
        self.recording.summary_prompt = self.recording.summary_prompt.trim().to_string();
    }

    /// Atomic write: `config.toml.tmp` then rename — the same pattern
    /// `keys.json` uses.
    pub fn save_to(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut tmp = path.as_os_str().to_os_string();
        tmp.push(".tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Load `~/.marvis/config.toml`; falls back to defaults on any error.
pub fn load() -> Config {
    match Config::load_from(paths::config_file()) {
        Ok(cfg) => cfg,
        Err(err) => {
            log::warn!(
                "failed to load {}: {err}; using defaults",
                paths::config_file().display()
            );
            Config::default()
        }
    }
}

/// Save to `~/.marvis/config.toml` (atomic tmp + rename).
pub fn save(config: &Config) -> anyhow::Result<()> {
    config.save_to(paths::config_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn tempfile_dir() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "marvis-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn stt_command_keys_apply_and_validate_values() {
        let mut models = ModelPrefs::default();
        assert!(apply_stt_config(
            &mut models,
            "models.stt_provider",
            &serde_json::json!(" whisper "),
        )
        .unwrap());
        assert_eq!(models.stt_provider, "whisper");
        assert!(apply_stt_config(
            &mut models,
            "models.stt_model",
            &serde_json::json!("  base  "),
        )
        .unwrap());
        assert_eq!(models.stt_model, "base");
        assert_eq!(
            apply_stt_config(
                &mut models,
                "models.stt_provider",
                &serde_json::json!("unknown"),
            )
            .unwrap_err(),
            "unknown STT provider \"unknown\""
        );
        models.stt_provider = "whisper".into();
        assert_eq!(
            apply_stt_config(
                &mut models,
                "models.stt_model",
                &serde_json::json!("https://example.com/model.bin"),
            )
            .unwrap_err(),
            "unknown Whisper model \"https://example.com/model.bin\""
        );
        assert_eq!(
            apply_stt_config(
                &mut models,
                "models.stt_model",
                &serde_json::json!(" GGML-SMALL.BIN "),
            )
            .unwrap_err(),
            "unknown Whisper model \"GGML-SMALL.BIN\""
        );
        assert!(apply_stt_config(
            &mut models,
            "models.stt_model",
            &serde_json::json!(" small "),
        )
        .unwrap());
        assert_eq!(models.stt_model, "small");
        assert!(apply_stt_config(
            &mut models,
            "models.stt_model",
            &serde_json::json!(" ggml-small.bin "),
        )
        .is_ok());
        assert_eq!(models.stt_model, "small");
        models.stt_provider = "deepgram".into();
        assert!(apply_stt_config(
            &mut models,
            "models.stt_model",
            &serde_json::json!("nova-3"),
        )
        .is_ok());
        assert_eq!(models.stt_model, "nova-3");
        assert!(apply_stt_config(
            &mut models,
            "models.stt_provider",
            &serde_json::json!("whisper"),
        )
        .is_ok());
        assert_eq!(models.stt_model, "tiny");
        assert!(apply_stt_config(
            &mut models,
            "models.stt_provider",
            &serde_json::json!("deepgram"),
        )
        .is_ok());
        assert_eq!(models.stt_model, "nova-2");
        assert_eq!(
            apply_stt_config(&mut models, "models.stt_model", &serde_json::json!("  "),)
                .unwrap_err(),
            "STT model must not be empty"
        );
        assert!(
            !apply_stt_config(&mut models, "models.other", &serde_json::json!("value"),).unwrap()
        );
    }

    /// Provider and model are one transactional preference: every switch
    /// must leave `stt_model` valid for the new provider — a foreign catalog
    /// id (whisper or sherpa) resets to that provider's default, while a
    /// custom deepgram name survives.
    #[test]
    fn stt_provider_switch_never_pairs_a_foreign_catalog_model() {
        let mut models = ModelPrefs::default();
        // deepgram → sherpa adopts the sherpa catalog default.
        assert!(apply_stt_config(
            &mut models,
            "models.stt_provider",
            &serde_json::json!("sherpa"),
        )
        .unwrap());
        assert_eq!(models.stt_model, "sense-voice");
        // sherpa → deepgram must not send "sense-voice" as a hosted model id.
        assert!(apply_stt_config(
            &mut models,
            "models.stt_provider",
            &serde_json::json!("deepgram"),
        )
        .unwrap());
        assert_eq!(models.stt_model, "nova-2");
        // sherpa → whisper resets to the whisper catalog default.
        assert!(apply_stt_config(
            &mut models,
            "models.stt_provider",
            &serde_json::json!("sherpa"),
        )
        .unwrap());
        assert!(apply_stt_config(
            &mut models,
            "models.stt_provider",
            &serde_json::json!("whisper"),
        )
        .unwrap());
        assert_eq!(models.stt_model, "tiny");
        // whisper → sherpa resets to the sherpa catalog default.
        assert!(apply_stt_config(
            &mut models,
            "models.stt_provider",
            &serde_json::json!("sherpa"),
        )
        .unwrap());
        assert_eq!(models.stt_model, "sense-voice");
        // A custom (non-catalog) deepgram model is the user's own value — a
        // deepgram reselect keeps it rather than resetting to nova-2.
        models.stt_provider = "deepgram".into();
        models.stt_model = "nova-3".into();
        assert!(apply_stt_config(
            &mut models,
            "models.stt_provider",
            &serde_json::json!("deepgram"),
        )
        .unwrap());
        assert_eq!(models.stt_model, "nova-3");
    }

    #[test]
    fn load_returns_defaults_when_file_missing() {
        let tmp = tempfile_dir();
        let cfg = Config::load_from(tmp.join("config.toml")).unwrap();
        // Default order is the provider catalog; nothing disabled.
        assert_eq!(
            cfg.providers.order,
            ProviderKind::ALL
                .iter()
                .map(|k| k.as_str().to_string())
                .collect::<Vec<_>>()
        );
        assert!(cfg.providers.disabled.is_empty());
        assert_eq!(cfg.hotkeys["toggle_input"], "Cmd+Alt+Space");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let tmp = tempfile_dir();
        let mut cfg = Config::default();
        cfg.providers.order = vec!["ollama".into(), "openai".into()];
        cfg.providers.disabled = vec!["gemini".into()];
        cfg.providers
            .models
            .insert("ollama".into(), "qwen3:8b".into());
        cfg.save_to(tmp.join("config.toml")).unwrap();
        let back = Config::load_from(tmp.join("config.toml")).unwrap();
        // Order persists verbatim, gaps fill in catalog order at the end.
        assert_eq!(
            back.providers.order,
            vec![
                "ollama",
                "openai",
                "anthropic",
                "gemini",
                "openrouter",
                "compatible"
            ]
        );
        assert_eq!(back.providers.disabled, vec!["gemini"]);
        assert_eq!(back.providers.models["ollama"], "qwen3:8b");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn legacy_llm_pair_migrates_into_provider_models() {
        // A config written before `[providers]` existed keeps its pick —
        // it becomes that provider's remembered model, and the legacy
        // fields never serialize back out.
        let tmp = tempfile_dir();
        let path = tmp.join("config.toml");
        std::fs::write(
            &path,
            "[models]\nllm_provider = \"anthropic\"\nllm_model = \"claude-opus-4-1\"\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.providers.models["anthropic"], "claude-opus-4-1");
        cfg.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("llm_provider"));
        assert!(!text.contains("llm_model"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn normalize_drops_unknown_ids_and_dedupes() {
        let tmp = tempfile_dir();
        let path = tmp.join("config.toml");
        std::fs::write(
            &path,
            "[providers]\norder = [\"openai\", \"bogus\", \"openai\", \"gemini\"]\n\
             disabled = [\"bogus\", \"ollama\", \"ollama\"]\n\
             [providers.models]\nbogus = \"x\"\nopenai = \"gpt-4o-mini\"\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(
            cfg.providers.order,
            vec![
                "openai",
                "gemini",
                "anthropic",
                "openrouter",
                "ollama",
                "compatible"
            ]
        );
        assert_eq!(cfg.providers.disabled, vec!["ollama"]);
        assert!(!cfg.providers.models.contains_key("bogus"));
        assert_eq!(cfg.providers.models["openai"], "gpt-4o-mini");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn model_for_falls_back_to_static_first() {
        let cfg = Config::default();
        assert_eq!(cfg.providers.model_for("openai"), "gpt-4o");
        // Live-list providers have no static default.
        assert_eq!(cfg.providers.model_for("ollama"), "");
        let mut cfg = Config::default();
        cfg.providers
            .models
            .insert("openai".into(), "gpt-4o-mini".into());
        assert_eq!(cfg.providers.model_for("openai"), "gpt-4o-mini");
    }

    #[test]
    fn vision_section_roundtrips_and_defaults_to_off() {
        // No [vision] in the file → reader off with empty memory.
        let tmp = tempfile_dir();
        let cfg = Config::load_from(tmp.join("config.toml")).unwrap();
        assert_eq!(cfg.vision.provider, "");
        assert!(cfg.vision.models.is_empty());

        let mut cfg = Config::default();
        cfg.vision.provider = "gemini".into();
        cfg.vision
            .models
            .insert("gemini".into(), "gemini-2.5-pro".into());
        let path = tmp.join("config.toml");
        cfg.save_to(&path).unwrap();
        let back = Config::load_from(&path).unwrap();
        assert_eq!(back.vision.provider, "gemini");
        assert_eq!(back.vision.models["gemini"], "gemini-2.5-pro");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn normalize_drops_non_vision_provider_and_stale_models() {
        // Anthropic reads images but isn't in the reader's pick list —
        // a hand-edited config can't name it; unknown ids clear too.
        // Model memory keeps only vision ids with a non-empty pick.
        let tmp = tempfile_dir();
        let path = tmp.join("config.toml");
        std::fs::write(
            &path,
            "[vision]\nprovider = \"anthropic\"\n\
             [vision.models]\nanthropic = \"claude-opus-4-1\"\nopenai = \"gpt-4o-mini\"\nbogus = \"x\"\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.vision.provider, "");
        assert!(!cfg.vision.models.contains_key("anthropic"));
        assert!(!cfg.vision.models.contains_key("bogus"));
        assert_eq!(cfg.vision.models["openai"], "gpt-4o-mini");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn vision_model_for_falls_back_to_the_vision_default() {
        let cfg = Config::default();
        // Vision defaults are the cheap readers, not the chat flagships.
        assert_eq!(cfg.vision.model_for("openai"), "gpt-4o-mini");
        assert_eq!(cfg.vision.model_for("gemini"), "gemini-2.5-flash");
        assert_eq!(
            cfg.vision.model_for("openrouter"),
            "google/gemini-2.0-flash-001"
        );
        // Compatible is free-text — no static default exists.
        assert_eq!(cfg.vision.model_for("compatible"), "");
        let mut cfg = Config::default();
        cfg.vision.models.insert("openai".into(), "gpt-4o".into());
        assert_eq!(cfg.vision.model_for("openai"), "gpt-4o");
    }

    #[test]
    fn save_is_atomic_and_leaves_no_tmp_file() {
        let tmp = tempfile_dir();
        let path = tmp.join("config.toml");
        Config::default().save_to(&path).unwrap();
        assert!(path.exists());
        assert!(!tmp.join("config.toml.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o644);
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn partial_toml_fills_defaults_for_missing_keys() {
        // A config with only some keys set must keep defaults for the rest —
        // silently resetting everything on a partial file is a bug.
        let tmp = tempfile_dir();
        let path = tmp.join("config.toml");
        std::fs::write(&path, "[models]\nllm_provider = \"anthropic\"\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        // The legacy pair still deserializes (for migration), and the
        // migrated model lands on the provider — but a partial file's
        // defaults fill everything else.
        assert_eq!(cfg.models.llm_provider, "anthropic");
        assert_eq!(cfg.models.llm_model, "gpt-4o");
        assert_eq!(cfg.providers.models["anthropic"], "gpt-4o");
        assert_eq!(cfg.models.stt_provider, "deepgram");
        assert_eq!(cfg.hotkeys["toggle_input"], "Cmd+Alt+Space");
        assert!(cfg.window.bar_x.is_none());

        // Spec example writes integer positions (`bar_x = 812`) into f64 fields.
        std::fs::write(&path, "[window]\nbar_x = 812\nbar_y = 21\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.window.bar_x, Some(812.0));
        assert_eq!(cfg.window.bar_y, Some(21.0));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn partial_hotkeys_merge_with_defaults_and_drop_stale() {
        // A user override on the one rebindable action lands; names
        // this build doesn't know — including the retired
        // `toggle_visibility`/`next_step`/`screen_only`/`show_settings`
        // set — are dropped rather than kept forever.
        let tmp = tempfile_dir();
        let path = tmp.join("config.toml");
        std::fs::write(
            &path,
            "[hotkeys]\ntoggle_input = \"Ctrl+Alt+J\"\ntoggle_visibility = \"Cmd+/\"\nnext_step = \"Cmd+Enter\"\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.hotkeys["toggle_input"], "Ctrl+Alt+J");
        assert!(!cfg.hotkeys.contains_key("toggle_visibility"));
        assert!(!cfg.hotkeys.contains_key("next_step"));
        // The user override lands alongside the defaults it didn't set.
        assert_eq!(cfg.hotkeys.len(), default_hotkeys().len());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn default_hotkeys_matches_spec_table() {
        let hk = default_hotkeys();
        assert_eq!(hk.len(), 5);
        assert_eq!(hk["toggle_input"], "Cmd+Alt+Space");
        assert_eq!(hk["toggle_capture"], "Cmd+Alt+R");
        assert_eq!(hk["start_listen"], "Cmd+Alt+T");
        assert_eq!(hk["show_history"], "Cmd+Alt+H");
        assert_eq!(hk["toggle_lock"], "Cmd+Shift+L");
    }

    #[test]
    fn stt_provider_validation_accepts_supported_values_only() {
        assert!(validate_stt_provider("deepgram").is_ok());
        assert!(validate_stt_provider("whisper").is_ok());
        assert!(validate_stt_provider("assemblyai").is_err());
    }

    #[test]
    fn stt_model_validation_trims_and_rejects_blank_values() {
        assert_eq!(validate_stt_model(" nova-2 ").unwrap(), "nova-2");
        assert_eq!(
            validate_stt_model(" ggml-base.bin ").unwrap(),
            "ggml-base.bin"
        );
        assert!(validate_stt_model("   ").is_err());
    }

    #[test]
    fn stt_provider_validation_accepts_sherpa() {
        assert!(validate_stt_provider("sherpa").is_ok());
        assert!(validate_stt_provider("assemblyai").is_err());
    }

    #[test]
    fn sherpa_model_validation_uses_the_sherpa_catalog() {
        assert_eq!(
            validate_sherpa_model(" sense-voice ").unwrap(),
            "sense-voice"
        );
        assert!(validate_sherpa_model("ggml-base.bin").is_err());
        assert!(validate_sherpa_model("../x").is_err());
    }

    #[test]
    fn provider_switch_to_sherpa_defaults_the_model() {
        let mut models = ModelPrefs {
            stt_provider: "deepgram".into(),
            stt_model: "nova-2".into(),
            ..Default::default()
        };
        apply_stt_config(
            &mut models,
            "models.stt_provider",
            &serde_json::json!("sherpa"),
        )
        .unwrap();
        assert_eq!(models.stt_provider, "sherpa");
        assert_eq!(models.stt_model, "sense-voice");
    }

    #[test]
    fn stt_model_write_validates_against_the_active_provider() {
        let mut models = ModelPrefs {
            stt_provider: "sherpa".into(),
            stt_model: "sense-voice".into(),
            ..Default::default()
        };
        assert!(
            apply_stt_config(&mut models, "models.stt_model", &serde_json::json!("tiny")).is_err()
        );
        assert_eq!(models.stt_model, "sense-voice");
    }

    #[test]
    fn recording_keys_apply_and_validate() {
        let mut rec = RecordingPrefs::default();
        assert!(apply_recording_config(
            &mut rec,
            "recording.auto_screenshots",
            &serde_json::json!(false)
        )
        .unwrap());
        assert!(!rec.auto_screenshots);
        assert!(apply_recording_config(&mut rec, "recording.fps", &serde_json::json!(8)).unwrap());
        assert_eq!(rec.fps, 8);
        assert_eq!(
            apply_recording_config(&mut rec, "recording.fps", &serde_json::json!(5)).unwrap_err(),
            "unknown fps 5"
        );
        assert!(apply_recording_config(
            &mut rec,
            "recording.summary_prompt",
            &serde_json::json!("  custom  ")
        )
        .unwrap());
        assert_eq!(rec.summary_prompt, "custom");
        assert!(
            !apply_recording_config(&mut rec, "recording.other", &serde_json::json!(1)).unwrap()
        );
        assert_eq!(validate_main_language(" zh ").unwrap(), "zh");
        assert_eq!(
            validate_main_language("cn").unwrap_err(),
            "unknown language \"cn\""
        );
    }

    #[test]
    fn recording_read_interval_defaults_and_sets() {
        let prefs = RecordingPrefs::default();
        assert_eq!(prefs.read_interval_secs, 3);
        let mut cfg = Config::default();
        assert!(apply_recording_config(
            &mut cfg.recording,
            "recording.read_interval_secs",
            &serde_json::json!(5)
        )
        .unwrap());
        assert_eq!(cfg.recording.read_interval_secs, 5);
        assert!(apply_recording_config(
            &mut cfg.recording,
            "recording.read_interval_secs",
            &serde_json::json!(0)
        )
        .is_err());
    }

    #[test]
    fn normalize_clamps_language_and_fps() {
        let mut cfg = Config::default();
        cfg.app.main_language = "klingon".into();
        cfg.recording.fps = 60;
        cfg.recording.summary_prompt = "  pad  ".into();
        cfg.normalize();
        assert_eq!(cfg.app.main_language, "en");
        assert_eq!(cfg.recording.fps, 4);
        assert_eq!(cfg.recording.summary_prompt, "pad");
    }
}
