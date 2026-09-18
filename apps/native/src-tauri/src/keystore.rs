//! `keys.enc` — the only place API secrets ever live on disk.
//!
//! File format: `[16B salt][12B nonce][ciphertext || 16B tag]`.
//! Key = Argon2id(passphrase, salt) `m=65536, t=3, p=4` → AES-256-GCM over a
//! JSON map of provider → key. Plaintext keys exist only inside [`Keyring`]
//! while [`KeystoreState::Unlocked`]; they are zeroized on drop, so nothing
//! outside this module ever holds secret material longer than needed.
//!
//! Behaviour notes:
//! - `init` (re)creates the file: calling it on an existing keystore
//!   **overwrites** it — callers must check `state() == Unset` first if
//!   losing stored keys would be a bug.
//! - `unlock` on `Unset` (no file) returns [`KeystoreError::Unset`], never
//!   panics.
//! - `set_key`/`remove_key` fail with `Locked`/`Unset` unless the keystore
//!   is `Unlocked`.
//! - Every write is atomic: `keys.enc.tmp` → rename, tmp chmod 0600 first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::paths;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
/// Smallest valid file: salt + nonce + tag, empty ciphertext.
const MIN_FILE_LEN: usize = SALT_LEN + NONCE_LEN + TAG_LEN;

#[derive(Debug, Error)]
pub enum KeystoreError {
    /// Passphrase does not match, or the ciphertext/tag was tampered with
    /// (AEAD cannot distinguish the two).
    #[error("wrong passphrase")]
    WrongPassphrase,
    /// File is too short or the decrypted payload is not the JSON map.
    #[error("keys.enc is corrupted")]
    Corrupt,
    /// No `keys.enc` exists yet — call `init` first.
    #[error("keystore has not been initialized")]
    Unset,
    /// Keys exist on disk but the keystore is locked — call `unlock` first.
    #[error("keystore is locked")]
    Locked,
    #[error("keystore i/o: {0}")]
    Io(#[from] std::io::Error),
}

/// Provider → secret map whose memory is scrubbed on drop.
///
/// Every value is a [`Zeroizing<String>`]: replacing or removing an entry
/// zeroizes the old buffer, and dropping the keyring (on `lock()` or
/// `Keystore` drop) zeroizes them all.
#[derive(Default)]
pub struct Keyring(HashMap<String, Zeroizing<String>>);

impl std::fmt::Debug for Keyring {
    /// Provider names only — Debug must never print secret material.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|k| (k, "…")))
            .finish()
    }
}

pub enum KeystoreState {
    /// `keys.enc` exists; a passphrase is required.
    Locked,
    /// No `keys.enc` yet — call `init`.
    Unset,
    /// Decrypted keyring in memory.
    Unlocked(Keyring),
}

impl std::fmt::Debug for KeystoreState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Locked => f.write_str("Locked"),
            Self::Unset => f.write_str("Unset"),
            Self::Unlocked(ring) => f.debug_tuple("Unlocked").field(ring).finish(),
        }
    }
}

/// The one secret store. Plaintext keys never leave this type except
/// through `key()`, which callers must treat as sensitive.
pub struct Keystore {
    path: PathBuf,
    state: KeystoreState,
    /// Salt from the file header — public in the file anyway; kept while
    /// unlocked so `persist` can rewrite without re-asking the passphrase.
    salt: Option<[u8; SALT_LEN]>,
    /// Derived AES-256 key — `Zeroizing`, dropped on `lock()`.
    dek: Option<Zeroizing<[u8; KEY_LEN]>>,
}

impl Keystore {
    /// Keystore bound to the real `~/.marvis/keys.enc`.
    pub fn new() -> Self {
        Self::at(paths::keys_file())
    }

    /// Keystore bound to an explicit path (tests inject a tmp file).
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            state: if path.exists() {
                KeystoreState::Locked
            } else {
                KeystoreState::Unset
            },
            path,
            salt: None,
            dek: None,
        }
    }

    pub fn state(&self) -> &KeystoreState {
        &self.state
    }

    /// Create a fresh keystore (empty keyring) and persist it. **Overwrites**
    /// an existing `keys.enc`; check `state()` first if that would lose keys.
    pub fn init(&mut self, passphrase: &str) -> Result<(), KeystoreError> {
        let salt: [u8; SALT_LEN] = rand::random();
        let dek = derive_key(passphrase, &salt)?;
        self.salt = Some(salt);
        self.dek = Some(dek);
        self.state = KeystoreState::Unlocked(Keyring::default());
        self.persist()
    }

    /// Decrypt `keys.enc` into memory. `WrongPassphrase` on passphrase or
    /// integrity failure, `Corrupt` on malformed file, `Unset` if missing.
    pub fn unlock(&mut self, passphrase: &str) -> Result<(), KeystoreError> {
        let raw = match std::fs::read(&self.path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.state = KeystoreState::Unset;
                return Err(KeystoreError::Unset);
            }
            Err(e) => return Err(KeystoreError::Io(e)),
        };
        if raw.len() < MIN_FILE_LEN {
            return Err(KeystoreError::Corrupt);
        }
        let (salt_b, rest) = raw.split_at(SALT_LEN);
        let (nonce_b, ct) = rest.split_at(NONCE_LEN);
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(salt_b);
        let dek = derive_key(passphrase, &salt)?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(dek.as_slice()));
        let pt = Zeroizing::new(
            cipher
                .decrypt(Nonce::from_slice(nonce_b), ct)
                .map_err(|_| KeystoreError::WrongPassphrase)?,
        );
        // The Strings are *moved* into Zeroizing wrappers — no extra
        // plaintext copies are made.
        let map: HashMap<String, String> =
            serde_json::from_slice(&pt).map_err(|_| KeystoreError::Corrupt)?;
        self.salt = Some(salt);
        self.dek = Some(dek);
        self.state = KeystoreState::Unlocked(Keyring(
            map.into_iter()
                .map(|(provider, key)| (provider, Zeroizing::new(key)))
                .collect(),
        ));
        Ok(())
    }

    /// Drop the keyring and derived key (both zeroize), back to `Locked`.
    /// A no-op on `Unset` — there is nothing to lock.
    pub fn lock(&mut self) {
        self.dek = None;
        self.salt = None;
        if matches!(self.state, KeystoreState::Unlocked(_)) {
            self.state = KeystoreState::Locked;
        }
    }

    /// Insert or replace a provider key and atomically rewrite `keys.enc`.
    pub fn set_key(&mut self, provider: &str, key: &str) -> Result<(), KeystoreError> {
        match &mut self.state {
            KeystoreState::Unlocked(ring) => {
                ring.0
                    .insert(provider.to_string(), Zeroizing::new(key.to_string()));
            }
            KeystoreState::Locked => return Err(KeystoreError::Locked),
            KeystoreState::Unset => return Err(KeystoreError::Unset),
        }
        self.persist()
    }

    /// Remove a provider key and atomically rewrite `keys.enc`.
    pub fn remove_key(&mut self, provider: &str) -> Result<(), KeystoreError> {
        match &mut self.state {
            KeystoreState::Unlocked(ring) => {
                ring.0.remove(provider);
            }
            KeystoreState::Locked => return Err(KeystoreError::Locked),
            KeystoreState::Unset => return Err(KeystoreError::Unset),
        }
        self.persist()
    }

    /// `(provider, Some("…" + last4))` for each set key, sorted by provider.
    /// Empty when not `Unlocked` — the provider list lives inside the
    /// ciphertext, so it is unknowable while locked.
    pub fn masked_status(&self) -> Vec<(String, Option<String>)> {
        let ring = match &self.state {
            KeystoreState::Unlocked(ring) => ring,
            _ => return Vec::new(),
        };
        let mut out: Vec<(String, Option<String>)> = ring
            .0
            .iter()
            .map(|(provider, key)| (provider.clone(), Some(mask(key))))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The plaintext key for a provider — the only escape hatch for secret
    /// material. Callers must not log, persist, or `Debug` the result.
    pub fn key(&self, provider: &str) -> Option<String> {
        match &self.state {
            KeystoreState::Unlocked(ring) => ring.0.get(provider).map(|key| key.to_string()),
            _ => None,
        }
    }

    /// Re-encrypt the in-memory keyring and atomically replace `keys.enc`.
    /// Fresh random nonce on every write; the salt is stable per passphrase.
    fn persist(&self) -> Result<(), KeystoreError> {
        let (ring, dek, salt) = match (&self.state, &self.dek, &self.salt) {
            (KeystoreState::Unlocked(ring), Some(dek), Some(salt)) => (ring, dek, salt),
            (KeystoreState::Unset, _, _) => return Err(KeystoreError::Unset),
            _ => return Err(KeystoreError::Locked),
        };
        let map: HashMap<&str, &str> = ring
            .0
            .iter()
            .map(|(provider, key)| (provider.as_str(), key.as_str()))
            .collect();
        let pt = Zeroizing::new(serde_json::to_vec(&map).map_err(|_| KeystoreError::Corrupt)?);
        let nonce_bytes: [u8; NONCE_LEN] = rand::random();
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(dek.as_slice()));
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), pt.as_slice())
            .map_err(|_| KeystoreError::Corrupt)?;
        let mut out = Vec::with_capacity(MIN_FILE_LEN + ct.len());
        out.extend_from_slice(salt);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        write_atomic(&self.path, &out)
    }
}

impl Default for Keystore {
    fn default() -> Self {
        Self::new()
    }
}

/// Argon2id KDF — `m=65536 (64 MiB), t=3, p=4`, 32B output. The derived key
/// is wrapped in `Zeroizing` so it is scrubbed when the caller drops it.
fn derive_key(
    passphrase: &str,
    salt: &[u8; SALT_LEN],
) -> Result<Zeroizing<[u8; KEY_LEN]>, KeystoreError> {
    let params = Params::new(65_536, 3, 4, Some(KEY_LEN)).map_err(|_| KeystoreError::Corrupt)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, key.as_mut())
        .map_err(|_| KeystoreError::Corrupt)?;
    Ok(key)
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

/// `keys.enc.tmp` → rename; tmp gets 0600 *before* the rename so the file
/// never exists at a looser mode. Same pattern as `config::save_to`.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), KeystoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
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
    fn init_unlock_roundtrip() {
        let p = tmp_path("keys.enc");
        let mut ks = Keystore::at(p.clone());
        assert!(matches!(ks.state(), KeystoreState::Unset));
        ks.init("pw").unwrap();
        ks.set_key("openai", "sk-test-123").unwrap();
        ks.lock();
        assert!(matches!(ks.state(), KeystoreState::Locked));
        ks.unlock("pw").unwrap();
        assert_eq!(ks.key("openai").unwrap(), "sk-test-123");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn wrong_passphrase_fails() {
        let p = tmp_path("keys.enc");
        let mut ks = Keystore::at(p.clone());
        ks.init("pw").unwrap();
        ks.lock();
        assert!(matches!(
            ks.unlock("nope"),
            Err(KeystoreError::WrongPassphrase)
        ));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn tampered_file_fails() {
        let p = tmp_path("keys.enc");
        let mut ks = Keystore::at(p.clone());
        ks.init("pw").unwrap();
        let mut raw = std::fs::read(&p).unwrap();
        let n = raw.len();
        raw[n - 1] ^= 0xff;
        std::fs::write(&p, raw).unwrap();
        ks.lock();
        assert!(ks.unlock("pw").is_err());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn masked_status_never_leaks_key() {
        let p = tmp_path("keys.enc");
        let mut ks = Keystore::at(p.clone());
        ks.init("pw").unwrap();
        ks.set_key("openai", "sk-secret").unwrap();
        let masked = ks.masked_status();
        assert_eq!(masked, vec![("openai".into(), Some("…cret".into()))]);
        assert!(!format!("{:?}", masked).contains("sk-secret"));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn unlock_without_init_returns_unset_error() {
        // First-run path: no file yet must be a typed error, never a panic.
        let p = tmp_path("keys.enc");
        let mut ks = Keystore::at(p.clone());
        assert!(matches!(ks.unlock("pw"), Err(KeystoreError::Unset)));
        assert!(matches!(ks.state(), KeystoreState::Unset));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn set_and_remove_require_unlocked() {
        let p = tmp_path("keys.enc");
        let mut ks = Keystore::at(p.clone());
        assert!(matches!(
            ks.set_key("openai", "sk"),
            Err(KeystoreError::Unset)
        ));
        ks.init("pw").unwrap();
        ks.lock();
        assert!(matches!(
            ks.set_key("openai", "sk"),
            Err(KeystoreError::Locked)
        ));
        assert!(matches!(
            ks.remove_key("openai"),
            Err(KeystoreError::Locked)
        ));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn remove_key_persists() {
        let p = tmp_path("keys.enc");
        let mut ks = Keystore::at(p.clone());
        ks.init("pw").unwrap();
        ks.set_key("openai", "sk-a").unwrap();
        ks.set_key("deepgram", "dg-b").unwrap();
        ks.remove_key("openai").unwrap();
        ks.lock();
        ks.unlock("pw").unwrap();
        assert!(ks.key("openai").is_none());
        assert_eq!(ks.key("deepgram").unwrap(), "dg-b");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn keys_file_has_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let p = tmp_path("keys.enc");
        let mut ks = Keystore::at(p.clone());
        ks.init("pw").unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // Atomic write must not leave the tmp file behind.
        assert!(!p.with_file_name("keys.enc.tmp").exists());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn ciphertext_never_contains_plaintext_key() {
        let p = tmp_path("keys.enc");
        let mut ks = Keystore::at(p.clone());
        ks.init("pw").unwrap();
        ks.set_key("openai", "sk-test-123").unwrap();
        let raw = std::fs::read(&p).unwrap();
        let needle = b"sk-test-123";
        assert!(!raw.windows(needle.len()).any(|w| w == needle));
        // Layout: 16B salt || 12B nonce || ciphertext || 16B tag.
        assert!(raw.len() >= 16 + 12 + 16);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn init_overwrites_existing_file() {
        // Documented choice: `init` resets the keystore rather than erroring;
        // callers that must protect existing keys check `state()` first.
        let p = tmp_path("keys.enc");
        let mut ks = Keystore::at(p.clone());
        ks.init("pw1").unwrap();
        ks.set_key("openai", "sk-old").unwrap();
        ks.init("pw2").unwrap();
        assert!(ks.key("openai").is_none());
        ks.lock();
        assert!(matches!(
            ks.unlock("pw1"),
            Err(KeystoreError::WrongPassphrase)
        ));
        ks.unlock("pw2").unwrap();
        assert!(ks.key("openai").is_none());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn masked_status_sorted_and_empty_when_locked() {
        let p = tmp_path("keys.enc");
        let mut ks = Keystore::at(p.clone());
        ks.init("pw").unwrap();
        ks.set_key("openai", "sk-secret").unwrap();
        ks.set_key("deepgram", "dg-key-9999").unwrap();
        assert_eq!(
            ks.masked_status(),
            vec![
                ("deepgram".into(), Some("…9999".into())),
                ("openai".into(), Some("…cret".into()))
            ]
        );
        ks.lock();
        assert!(ks.masked_status().is_empty());
        assert!(ks.key("openai").is_none());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
