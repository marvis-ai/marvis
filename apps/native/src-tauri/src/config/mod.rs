mod prefs;

pub(crate) use prefs::*;

#[cfg(test)]
mod tests;

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
    /// User-defined prompt presets — see [`PromptPrefs`].
    pub prompts: PromptPrefs,
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
            prompts: PromptPrefs::default(),
        }
    }
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

        // A hand-edited `[[prompts.custom]]` keeps its valid rows —
        // malformed rows and duplicate ids drop.
        let mut seen = std::collections::HashSet::new();
        self.prompts.custom.retain_mut(|p| {
            p.id = p.id.trim().to_string();
            p.name = p.name.trim().to_string();
            p.text = p.text.trim().to_string();
            crate::presets::validate(p).is_ok() && seen.insert(p.id.clone())
        });

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

