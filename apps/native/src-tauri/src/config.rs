use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::llm::ProviderKind;
use crate::paths;

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
    #[serde(
        default = "default_hotkeys",
        deserialize_with = "merge_default_hotkeys"
    )]
    pub hotkeys: BTreeMap<String, String>,
    pub window: WindowPrefs,
    pub compat: CompatPrefs,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app: AppPrefs::default(),
            models: ModelPrefs::default(),
            providers: ProviderPrefs::default(),
            hotkeys: default_hotkeys(),
            window: WindowPrefs::default(),
            compat: CompatPrefs::default(),
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
    /// `#rrggbb` accent — the single hue the whole UI derives from
    /// (`--accent` in index.css; `--primary`, the soft tint, the text
    /// variant, and the focus ring all `color-mix` off it). Validated by
    /// `config_set`; `""` in a file reads as the default.
    pub accent: String,
}

/// The spec's slate accent — `#3a7294` (DESIGN.md §2).
pub const DEFAULT_ACCENT: &str = "#3a7294";

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            onboarding_done: false,
            appearance: "auto".into(),
            accent: DEFAULT_ACCENT.into(),
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
}

/// Default hotkey bindings — the whole configurable set (Settings →
/// Hotkeys rebinds any of them). Bar movement/snap is pointer-driven,
/// so there are deliberately no move/scroll/click-through actions.
pub fn default_hotkeys() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("toggle_visibility".into(), "Cmd+/".into()),
        ("next_step".into(), "Cmd+Enter".into()),
        ("screen_only".into(), "Cmd+Shift+S".into()),
        ("show_settings".into(), "Cmd+,".into()),
    ])
}

/// A `[hotkeys]` table may list only user overrides; fill in the spec
/// defaults for every action it doesn't mention, and drop names this
/// build doesn't know (a stale `move_up` from an older config would
/// otherwise sit in the file forever — nothing binds it).
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
        assert_eq!(cfg.hotkeys["toggle_visibility"], "Cmd+/");
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
        assert_eq!(cfg.hotkeys["toggle_visibility"], "Cmd+/");
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
        // User overrides one hotkey; the other actions still get
        // defaults — and a stale name (`move_up`, removed from the
        // configurable set) is dropped rather than kept forever.
        let tmp = tempfile_dir();
        let path = tmp.join("config.toml");
        std::fs::write(
            &path,
            "[hotkeys]\nnext_step = \"Cmd+Shift+Enter\"\nmove_up = \"Cmd+Up\"\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.hotkeys["next_step"], "Cmd+Shift+Enter");
        assert_eq!(cfg.hotkeys["toggle_visibility"], "Cmd+/");
        assert!(!cfg.hotkeys.contains_key("move_up"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn default_hotkeys_matches_spec_table() {
        let hk = default_hotkeys();
        let expected = [
            ("toggle_visibility", "Cmd+/"),
            ("next_step", "Cmd+Enter"),
            ("screen_only", "Cmd+Shift+S"),
            ("show_settings", "Cmd+,"),
        ];
        assert_eq!(hk.len(), expected.len());
        for (action, accel) in expected {
            assert_eq!(hk[action], accel, "hotkey {action}");
        }
    }
}
