//! `keys.enc` — the only place API secrets ever live on disk.
//!
//! File format: `[dek_source:u8][12B nonce][ciphertext || 16B tag]`.
//! The DEK is a random 256-bit key obtained from a [`DekProvider`]
//! (`0x01` = macOS Keychain + system auth, `0x02` reserved for a future
//! server-issued key) — there is no passphrase and no KDF. AES-256-GCM
//! covers a JSON map of provider → key. Plaintext keys exist only inside
//! [`Keyring`] while [`KeystoreState::Unlocked`]; they are zeroized on
//! drop, so nothing outside this module ever holds secret material
//! longer than needed.
//!
//! Behaviour notes:
//! - `init` (re)creates the file: calling it on an existing keystore
//!   **overwrites** it — callers must check `state() == Unset` first if
//!   losing stored keys would be a bug.
//! - `unlock` on `Unset` (no file) returns [`KeystoreError::Unset`], never
//!   panics. Format checks run before the DEK fetch so a malformed or
//!   foreign file never triggers a system-auth prompt.
//! - `set_key`/`remove_key` fail with `Locked`/`Unset` unless the keystore
//!   is `Unlocked`.
//! - `reset` deletes `keys.enc` → `Unset` — the recovery path for
//!   `Obsolete`/corrupt stores (the keychain DEK item is kept).
//! - Every write is atomic: `keys.enc.tmp` → rename, tmp chmod 0600 first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::paths;
use crate::system_auth::{DekError, DekProvider, SystemAuthDekProvider, DEK_LEN};

const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// Smallest valid file: source byte + nonce + tag, empty ciphertext.
const MIN_FILE_LEN: usize = 1 + NONCE_LEN + TAG_LEN;

#[derive(Debug, Error)]
pub enum KeystoreError {
    /// AEAD open failed — wrong DEK (the keychain item was replaced) or
    /// the ciphertext/tag was tampered with; AEAD cannot distinguish.
    #[error("cannot decrypt keys.enc")]
    CannotDecrypt,
    /// File is too short or the decrypted payload is not the JSON map.
    #[error("keys.enc is corrupted")]
    Corrupt,
    /// The source byte doesn't match this provider (including the old
    /// passphrase file, whose salt occupies the same offset). The
    /// `keystore_reset` recovery path applies.
    #[error("keystore format obsolete — reset required")]
    Obsolete,
    /// No `keys.enc` exists yet — call `init` first.
    #[error("keystore has not been initialized")]
    Unset,
    /// Keys exist on disk but the keystore is locked — call `unlock` first.
    #[error("keystore is locked")]
    Locked,
    /// The DEK provider failed (auth canceled, keychain error, …).
    #[error("{0}")]
    Dek(#[from] DekError),
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
    /// `keys.enc` exists; `unlock` requires a system-auth prompt.
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
    /// Where the AES key comes from — Keychain+system-auth in prod, a
    /// fixed key in tests.
    dek_provider: Arc<dyn DekProvider>,
    /// The DEK while `Unlocked` — `Zeroizing`, dropped on `lock()`.
    dek: Option<Zeroizing<[u8; DEK_LEN]>>,
}

impl Keystore {
    /// Keystore bound to the real `~/.marvis/keys.enc` with the
    /// production system-auth DEK provider.
    pub fn new() -> Self {
        Self::at(paths::keys_file(), Arc::new(SystemAuthDekProvider::new()))
    }

    /// Keystore bound to an explicit path and DEK provider (tests inject
    /// a tmp file + `StaticDekProvider`).
    pub fn at(path: impl Into<PathBuf>, dek_provider: Arc<dyn DekProvider>) -> Self {
        let path = path.into();
        Self {
            state: if path.exists() {
                KeystoreState::Locked
            } else {
                KeystoreState::Unset
            },
            path,
            dek_provider,
            dek: None,
        }
    }

    pub fn state(&self) -> &KeystoreState {
        &self.state
    }

    /// Create a fresh keystore (empty keyring) and persist it. **Overwrites**
    /// an existing `keys.enc`; check `state()` first if that would lose keys.
    /// `get_or_create` is silent on first run — no auth prompt here.
    pub fn init(&mut self) -> Result<(), KeystoreError> {
        let dek = self.dek_provider.get_or_create()?;
        self.dek = Some(dek);
        self.state = KeystoreState::Unlocked(Keyring::default());
        if let Err(e) = self.persist() {
            self.dek = None;
            self.state = if self.path.exists() {
                KeystoreState::Locked
            } else {
                KeystoreState::Unset
            };
            return Err(e);
        }
        Ok(())
    }

    /// Decrypt `keys.enc` into memory: read the file, fetch the DEK from
    /// the provider (the Touch ID / system-password prompt lives here),
    /// AEAD-open. `CannotDecrypt` on integrity failure, `Obsolete` on a
    /// foreign format, `Corrupt` on malformed file, `Unset` if missing.
    pub fn unlock(&mut self) -> Result<(), KeystoreError> {
        let raw = match std::fs::read(&self.path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.dek = None;
                self.state = KeystoreState::Unset;
                return Err(KeystoreError::Unset);
            }
            Err(e) => return Err(KeystoreError::Io(e)),
        };
        // Parse before fetching the DEK — a file we can't read never
        // earns an auth prompt.
        if raw.len() < MIN_FILE_LEN {
            return Err(KeystoreError::Corrupt);
        }
        let (source_b, rest) = raw.split_at(1);
        if source_b[0] != self.dek_provider.source() {
            return Err(KeystoreError::Obsolete);
        }
        let (nonce_b, ct) = rest.split_at(NONCE_LEN);
        let dek = self.dek_provider.get()?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(dek.as_slice()));
        let pt = Zeroizing::new(
            cipher
                .decrypt(Nonce::from_slice(nonce_b), ct)
                .map_err(|_| KeystoreError::CannotDecrypt)?,
        );
        // The Strings are *moved* into Zeroizing wrappers — no extra
        // plaintext copies are made.
        let map: HashMap<String, String> =
            serde_json::from_slice(&pt).map_err(|_| KeystoreError::Corrupt)?;
        self.dek = Some(dek);
        self.state = KeystoreState::Unlocked(Keyring(
            map.into_iter()
                .map(|(provider, key)| (provider, Zeroizing::new(key)))
                .collect(),
        ));
        Ok(())
    }

    /// Delete `keys.enc`, the keychain DEK item(s), and any held DEK →
    /// `Unset`. The recovery path for `Obsolete`/`CannotDecrypt` stores —
    /// a missing file is fine (already `Unset`). Deleting the items
    /// guarantees the next `init` can't pick up a DEK written by a
    /// different backend (signed vs unsigned builds share `~/.marvis`).
    pub fn reset(&mut self) -> Result<(), KeystoreError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(KeystoreError::Io(e)),
        }
        self.dek_provider.delete();
        self.dek = None;
        self.state = KeystoreState::Unset;
        Ok(())
    }

    /// Drop the keyring and the DEK (both zeroize), back to `Locked`.
    /// A no-op on `Unset` — there is nothing to lock.
    pub fn lock(&mut self) {
        self.dek = None;
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
    /// Fresh random nonce on every write; the source byte re-stamps which
    /// provider class wrote the file.
    fn persist(&self) -> Result<(), KeystoreError> {
        let (ring, dek) = match (&self.state, &self.dek) {
            (KeystoreState::Unlocked(ring), Some(dek)) => (ring, dek),
            (KeystoreState::Unset, _) => return Err(KeystoreError::Unset),
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
        out.push(self.dek_provider.source());
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
    use crate::system_auth::{StaticDekProvider, DEK_SOURCE_SYSTEM_AUTH};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    const TEST_DEK: [u8; DEK_LEN] = [7; DEK_LEN];
    const OTHER_DEK: [u8; DEK_LEN] = [9; DEK_LEN];

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

    fn ks_at(path: &Path, dek: [u8; DEK_LEN]) -> Keystore {
        Keystore::at(path, Arc::new(StaticDekProvider(dek)))
    }

    #[test]
    fn init_unlock_roundtrip() {
        let p = tmp_path("keys.enc");
        let mut ks = ks_at(&p, TEST_DEK);
        assert!(matches!(ks.state(), KeystoreState::Unset));
        ks.init().unwrap();
        ks.set_key("openai", "sk-test-123").unwrap();
        ks.lock();
        assert!(matches!(ks.state(), KeystoreState::Locked));
        ks.unlock().unwrap();
        assert_eq!(ks.key("openai").unwrap(), "sk-test-123");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn wrong_dek_fails() {
        // A different DEK (keychain item replaced, provider swapped)
        // must fail AEAD open — same guarantee `wrong_passphrase` gave.
        let p = tmp_path("keys.enc");
        let mut ks = ks_at(&p, TEST_DEK);
        ks.init().unwrap();
        ks.lock();
        let mut other = ks_at(&p, OTHER_DEK);
        assert!(matches!(other.unlock(), Err(KeystoreError::CannotDecrypt)));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn tampered_file_fails() {
        let p = tmp_path("keys.enc");
        let mut ks = ks_at(&p, TEST_DEK);
        ks.init().unwrap();
        let mut raw = std::fs::read(&p).unwrap();
        let n = raw.len();
        raw[n - 1] ^= 0xff;
        std::fs::write(&p, raw).unwrap();
        ks.lock();
        assert!(matches!(ks.unlock(), Err(KeystoreError::CannotDecrypt)));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn old_passphrase_format_is_obsolete() {
        // The legacy `[16B salt][12B nonce][ct]` file: first salt byte
        // (≠ 0x01) lands in the source-byte slot → Obsolete, never a
        // decrypt attempt. This is the upgrade path's contract.
        let p = tmp_path("keys.enc");
        let mut old = vec![0xABu8; 16]; // salt
        old.extend_from_slice(&[0u8; NONCE_LEN]); // nonce
        old.extend_from_slice(&[0u8; TAG_LEN]); // shortest "ct" = tag only
        std::fs::write(&p, old).unwrap();
        let mut ks = ks_at(&p, TEST_DEK);
        let err = ks.unlock().unwrap_err();
        assert!(matches!(err, KeystoreError::Obsolete));
        assert!(err.to_string().contains("reset required"));
        // The keystore stays Locked — a failed parse changes nothing.
        assert!(matches!(ks.state(), KeystoreState::Locked));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn old_file_with_colliding_source_byte_is_cannot_decrypt() {
        // 1/256 of old passphrase files have salt[0] = 0x01 — they pass
        // the source check, then fail AEAD open. The error must be
        // `CannotDecrypt` (accurate), surfaced so the Reset affordance
        // shows — not a misleading "obsolete" or a panic.
        let p = tmp_path("keys.enc");
        let mut old = vec![0x01u8]; // colliding "salt" byte → passes check
        old.extend_from_slice(&[0u8; NONCE_LEN]);
        old.extend_from_slice(&[0u8; TAG_LEN]);
        std::fs::write(&p, old).unwrap();
        let mut ks = ks_at(&p, TEST_DEK);
        assert!(matches!(ks.unlock(), Err(KeystoreError::CannotDecrypt)));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn reserved_source_byte_is_obsolete() {
        // 0x02 is reserved for a future server-issued DEK — this provider
        // must refuse it as an unknown format, not decrypt with its key.
        let p = tmp_path("keys.enc");
        let mut raw = vec![0x02u8];
        raw.extend_from_slice(&[0u8; NONCE_LEN]);
        raw.extend_from_slice(&[0u8; TAG_LEN]);
        std::fs::write(&p, raw).unwrap();
        let mut ks = ks_at(&p, TEST_DEK);
        assert!(matches!(ks.unlock(), Err(KeystoreError::Obsolete)));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn short_file_is_corrupt() {
        let p = tmp_path("keys.enc");
        std::fs::write(&p, [0x01u8; MIN_FILE_LEN - 1]).unwrap();
        let mut ks = ks_at(&p, TEST_DEK);
        assert!(matches!(ks.unlock(), Err(KeystoreError::Corrupt)));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn reset_deletes_file_and_unsets() {
        let p = tmp_path("keys.enc");
        let mut ks = ks_at(&p, TEST_DEK);
        ks.init().unwrap();
        ks.set_key("openai", "sk-x").unwrap();
        assert!(p.exists());
        ks.reset().unwrap();
        assert!(!p.exists());
        assert!(matches!(ks.state(), KeystoreState::Unset));
        assert!(matches!(ks.unlock(), Err(KeystoreError::Unset)));
        // Reset on an already-missing file is a no-op, not an error.
        ks.reset().unwrap();
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn masked_status_never_leaks_key() {
        let p = tmp_path("keys.enc");
        let mut ks = ks_at(&p, TEST_DEK);
        ks.init().unwrap();
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
        let mut ks = ks_at(&p, TEST_DEK);
        assert!(matches!(ks.unlock(), Err(KeystoreError::Unset)));
        assert!(matches!(ks.state(), KeystoreState::Unset));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn missing_file_after_lock_clears_held_dek() {
        // If `keys.enc` vanishes while Locked, `unlock` must drop the
        // stale DEK and land on `Unset` — not retry a dead key.
        let p = tmp_path("keys.enc");
        let mut ks = ks_at(&p, TEST_DEK);
        ks.init().unwrap();
        ks.lock();
        std::fs::remove_file(&p).unwrap();
        assert!(matches!(ks.unlock(), Err(KeystoreError::Unset)));
        assert!(matches!(ks.state(), KeystoreState::Unset));
        assert!(ks.dek.is_none());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn set_and_remove_require_unlocked() {
        let p = tmp_path("keys.enc");
        let mut ks = ks_at(&p, TEST_DEK);
        assert!(matches!(
            ks.set_key("openai", "sk"),
            Err(KeystoreError::Unset)
        ));
        ks.init().unwrap();
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
        let mut ks = ks_at(&p, TEST_DEK);
        ks.init().unwrap();
        ks.set_key("openai", "sk-a").unwrap();
        ks.set_key("deepgram", "dg-b").unwrap();
        ks.remove_key("openai").unwrap();
        ks.lock();
        ks.unlock().unwrap();
        assert!(ks.key("openai").is_none());
        assert_eq!(ks.key("deepgram").unwrap(), "dg-b");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn keys_file_has_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let p = tmp_path("keys.enc");
        let mut ks = ks_at(&p, TEST_DEK);
        ks.init().unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // Atomic write must not leave the tmp file behind.
        assert!(!p.with_file_name("keys.enc.tmp").exists());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn ciphertext_never_contains_plaintext_key() {
        let p = tmp_path("keys.enc");
        let mut ks = ks_at(&p, TEST_DEK);
        ks.init().unwrap();
        ks.set_key("openai", "sk-test-123").unwrap();
        let raw = std::fs::read(&p).unwrap();
        let needle = b"sk-test-123";
        assert!(!raw.windows(needle.len()).any(|w| w == needle));
        // Layout: 1B source || 12B nonce || ciphertext || 16B tag.
        assert_eq!(raw[0], DEK_SOURCE_SYSTEM_AUTH);
        assert!(raw.len() >= MIN_FILE_LEN);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn init_overwrites_existing_file() {
        // Documented choice: `init` resets the keystore rather than erroring;
        // callers that must protect existing keys check `state()` first.
        let p = tmp_path("keys.enc");
        let mut ks = ks_at(&p, TEST_DEK);
        ks.init().unwrap();
        ks.set_key("openai", "sk-old").unwrap();
        ks.init().unwrap();
        assert!(ks.key("openai").is_none());
        ks.lock();
        // A different DEK no longer opens the re-initialized file.
        let mut other = ks_at(&p, OTHER_DEK);
        assert!(matches!(other.unlock(), Err(KeystoreError::CannotDecrypt)));
        ks.unlock().unwrap();
        assert!(ks.key("openai").is_none());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn masked_status_sorted_and_empty_when_locked() {
        let p = tmp_path("keys.enc");
        let mut ks = ks_at(&p, TEST_DEK);
        ks.init().unwrap();
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
