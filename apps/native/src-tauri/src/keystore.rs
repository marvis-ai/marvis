//! `keys.json` — where provider API keys live on disk.
//!
//! Plaintext JSON map of provider → key, mode 0600, inside the already
//! 0700 `~/.marvis` directory. There is no encryption, no Keychain DEK,
//! and no lock state: the store is always readable, so `key()` answers
//! from the map loaded at construction. Every mutation rewrites the file
//! atomically (`keys.json.tmp` → rename, tmp chmod 0600 first).
//!
//! Plaintext keys never leave this type except through `key()`, which
//! callers must treat as sensitive — `masked_status()` is the only
//! UI-facing view.
//!
//! A corrupt file is never silently destroyed: on parse failure it is
//! renamed to `keys.json.corrupt` and the store starts empty.

use std::collections::HashMap;
use std::path::PathBuf;

use thiserror::Error;

use crate::paths;

#[derive(Debug, Error)]
pub enum KeystoreError {
    /// The file exists but isn't the JSON provider→key map.
    #[error("keys.json is corrupted")]
    Corrupt,
    #[error("keystore i/o: {0}")]
    Io(#[from] std::io::Error),
}

/// The one secret store. Plaintext keys never leave this type except
/// through `key()`, which callers must not log, persist, or `Debug`.
#[derive(Clone)]
pub struct Keystore {
    path: PathBuf,
    keys: HashMap<String, String>,
}

impl Keystore {
    /// Keystore bound to the real `~/.marvis/keys.json`.
    pub fn new() -> Self {
        Self::at(paths::keys_file())
    }

    /// Keystore bound to an explicit path (tests use a tmp file). Loads
    /// eagerly — a missing file is an empty store; an unparseable one is
    /// renamed aside (`*.corrupt`) and also starts empty.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut keys = HashMap::new();
        match std::fs::read(&path) {
            Ok(raw) => match serde_json::from_slice::<HashMap<String, String>>(&raw) {
                Ok(map) => keys = map,
                Err(e) => {
                    log::warn!(
                        "keystore: {} unreadable ({e}); starting empty",
                        path.display()
                    );
                    let mut corrupt = path.as_os_str().to_os_string();
                    corrupt.push(".corrupt");
                    let _ = std::fs::rename(&path, PathBuf::from(corrupt));
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                log::warn!(
                    "keystore: cannot read {}: {e}; starting empty",
                    path.display()
                );
            }
        }
        Self { path, keys }
    }

    /// The plaintext key for a provider — the only escape hatch for
    /// secret material. Callers must not log, persist, or `Debug` the
    /// result.
    pub fn key(&self, provider: &str) -> Option<String> {
        self.keys.get(provider).cloned()
    }

    /// Insert or replace a provider key and atomically rewrite the file.
    pub fn set_key(&mut self, provider: &str, key: &str) -> Result<(), KeystoreError> {
        self.keys.insert(provider.to_string(), key.to_string());
        self.persist()
    }

    /// Remove a provider key and atomically rewrite the file.
    pub fn remove_key(&mut self, provider: &str) -> Result<(), KeystoreError> {
        self.keys.remove(provider);
        self.persist()
    }

    /// `(provider, Some("…" + last4))` for each set key, sorted by provider.
    pub fn masked_status(&self) -> Vec<(String, Option<String>)> {
        let mut out: Vec<(String, Option<String>)> = self
            .keys
            .iter()
            .map(|(provider, key)| (provider.clone(), Some(mask(key))))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// `keys.json.tmp` → rename; tmp gets 0600 *before* the rename so
    /// the file never exists at a looser mode.
    fn persist(&self) -> Result<(), KeystoreError> {
        let bytes = serde_json::to_vec_pretty(&self.keys).map_err(|_| KeystoreError::Corrupt)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut tmp = self.path.as_os_str().to_os_string();
        tmp.push(".tmp");
        std::fs::write(&tmp, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

impl std::fmt::Debug for Keystore {
    /// Provider names only — Debug must never print secret material.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keystore")
            .field("path", &self.path)
            .field("keys", &self.keys.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Default for Keystore {
    fn default() -> Self {
        Self::new()
    }
}

/// `…` + last 4 chars. Keys of ≤4 chars get the bare ellipsis so the mask
/// can never reveal an entire (short) secret.
fn mask(key: &str) -> String {
    let n = key.chars().count();
    if n <= 4 {
        return '…'.to_string();
    }
    let tail: String = key.chars().skip(n - 4).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn tmp_path(name: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "marvis-keystore-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn set_remove_roundtrip() {
        let p = tmp_path("keys.json");
        let mut ks = Keystore::at(&p);
        assert_eq!(ks.key("openai"), None);
        ks.set_key("openai", "sk-test-123").unwrap();
        ks.set_key("ollama-side", "k").unwrap();
        assert_eq!(ks.key("openai").unwrap(), "sk-test-123");
        // Replace in place.
        ks.set_key("openai", "sk-new").unwrap();
        assert_eq!(ks.key("openai").unwrap(), "sk-new");
        ks.remove_key("ollama-side").unwrap();
        assert_eq!(ks.key("ollama-side"), None);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn writes_a_plaintext_json_map() {
        // The file is honest plaintext JSON — that's the point of this
        // store; nothing about it is encrypted anymore.
        let p = tmp_path("keys.json");
        let mut ks = Keystore::at(&p);
        ks.set_key("openai", "sk-plain").unwrap();
        let map: HashMap<String, String> =
            serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(map["openai"], "sk-plain");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn a_new_store_loads_what_another_wrote() {
        let p = tmp_path("keys.json");
        let mut ks = Keystore::at(&p);
        ks.set_key("anthropic", "sk-ant-9").unwrap();
        drop(ks);
        let ks = Keystore::at(&p);
        assert_eq!(ks.key("anthropic").unwrap(), "sk-ant-9");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn corrupt_file_is_renamed_aside_and_store_starts_empty() {
        // A mangled file must never panic or be silently truncated — it
        // is moved to `.corrupt` and the store starts empty instead.
        let p = tmp_path("keys.json");
        std::fs::write(&p, b"{not json").unwrap();
        let ks = Keystore::at(&p);
        assert_eq!(ks.key("openai"), None);
        assert!(!p.exists());
        assert!(p.with_file_name("keys.json.corrupt").exists());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn masked_status_never_leaks_key() {
        let p = tmp_path("keys.json");
        let mut ks = Keystore::at(&p);
        ks.set_key("openai", "sk-secret").unwrap();
        ks.set_key("tiny", "abc").unwrap();
        let masked = ks.masked_status();
        assert_eq!(
            masked,
            vec![
                ("openai".into(), Some("…cret".into())),
                ("tiny".into(), Some("…".into())),
            ]
        );
        assert!(!format!("{:?}", masked).contains("sk-secret"));
        assert!(!format!("{ks:?}").contains("sk-secret"));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn file_is_written_0600() {
        use std::os::unix::fs::PermissionsExt;
        let p = tmp_path("keys.json");
        let mut ks = Keystore::at(&p);
        ks.set_key("openai", "sk-x").unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
