use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::paths;

/// Non-secret preferences, persisted as `~/.marvis/config.toml` (0644).
///
/// Every section and field is `#[serde(default)]`: a partial or missing
/// section deserializes to defaults instead of erroring or resetting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub models: ModelPrefs,
    #[serde(
        default = "default_hotkeys",
        deserialize_with = "merge_default_hotkeys"
    )]
    pub hotkeys: BTreeMap<String, String>,
    pub window: WindowPrefs,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            models: ModelPrefs::default(),
            hotkeys: default_hotkeys(),
            window: WindowPrefs::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelPrefs {
    pub llm_provider: String,
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

/// Default hotkey bindings (spec table). `Cmd+Shift+S` (manual screenshot),
/// `Cmd+Shift+<n>` (display n) and `Cmd+Shift+Left/Right` (snap edge) are
/// hardcoded elsewhere and deliberately absent.
pub fn default_hotkeys() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("toggle_visibility".into(), "Cmd+/".into()),
        ("next_step".into(), "Cmd+Enter".into()),
        ("move_up".into(), "Cmd+Up".into()),
        ("move_down".into(), "Cmd+Down".into()),
        ("move_left".into(), "Cmd+Left".into()),
        ("move_right".into(), "Cmd+Right".into()),
        ("toggle_click_through".into(), "Cmd+M".into()),
        ("scroll_up".into(), "Cmd+Shift+Up".into()),
        ("scroll_down".into(), "Cmd+Shift+Down".into()),
    ])
}

/// A `[hotkeys]` table may list only user overrides; fill in the spec
/// defaults for every action it doesn't mention.
fn merge_default_hotkeys<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut map = BTreeMap::<String, String>::deserialize(deserializer)?;
    for (action, accel) in default_hotkeys() {
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
        Ok(toml::from_str(&text)?)
    }

    /// Atomic write: `config.toml.tmp` then rename — the same pattern
    /// `keys.enc` will use.
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
        assert_eq!(cfg.models.llm_provider, "openai");
        assert_eq!(cfg.hotkeys["toggle_visibility"], "Cmd+/");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let tmp = tempfile_dir();
        let mut cfg = Config::default();
        cfg.models.llm_provider = "anthropic".into();
        cfg.save_to(tmp.join("config.toml")).unwrap();
        let back = Config::load_from(tmp.join("config.toml")).unwrap();
        assert_eq!(back.models.llm_provider, "anthropic");
        let _ = std::fs::remove_dir_all(&tmp);
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
        assert_eq!(cfg.models.llm_provider, "anthropic");
        assert_eq!(cfg.models.llm_model, "gpt-4o");
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
    fn partial_hotkeys_merge_with_defaults() {
        // User overrides one hotkey; the other spec actions still get defaults.
        let tmp = tempfile_dir();
        let path = tmp.join("config.toml");
        std::fs::write(&path, "[hotkeys]\nnext_step = \"Cmd+Shift+Enter\"\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.hotkeys["next_step"], "Cmd+Shift+Enter");
        assert_eq!(cfg.hotkeys["toggle_visibility"], "Cmd+/");
        assert_eq!(cfg.hotkeys["scroll_down"], "Cmd+Shift+Down");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn default_hotkeys_matches_spec_table() {
        let hk = default_hotkeys();
        let expected = [
            ("toggle_visibility", "Cmd+/"),
            ("next_step", "Cmd+Enter"),
            ("move_up", "Cmd+Up"),
            ("move_down", "Cmd+Down"),
            ("move_left", "Cmd+Left"),
            ("move_right", "Cmd+Right"),
            ("toggle_click_through", "Cmd+M"),
            ("scroll_up", "Cmd+Shift+Up"),
            ("scroll_down", "Cmd+Shift+Down"),
        ];
        for (action, accel) in expected {
            assert_eq!(hk[action], accel, "hotkey {action}");
        }
        // Hardcoded elsewhere — must NOT be in the configurable map.
        assert!(!hk.contains_key("manual_screenshot"));
    }
}
