//! `system_auth` — where the `keys.enc` AES-256 key comes from.
//!
//! [`DekProvider`] abstracts the DEK (data-encryption key) source so the
//! keystore never knows how the key is obtained. Today there is one
//! production source: [`SystemAuthDekProvider`], a random 32-byte DEK
//! stored in the macOS Keychain behind a `SecAccessControl` ACL
//! (`kSecAccessControlUserPresence` — Touch ID / Face ID / system
//! password). Source byte `0x01` is stamped into the `keys.enc` header;
//! `0x02` is reserved for a future server-issued key (auth system).
//!
//! Two backends, chosen once per provider instance and cached:
//!
//! - `KeychainAcl` (normal): the item's ACL makes macOS prompt for user
//!   presence on every `SecItemCopyMatching` read.
//! - `LaGate` (fallback): unsigned `tauri dev` binaries can fail ACL
//!   operations with `errSecMissingEntitlement` /
//!   `errSecInteractionNotAllowed` / `errSecAuthFailed`. The fallback
//!   keeps a plain (non-ACL) keychain item under a different account and
//!   gates reads in-process with `LAContext.evaluatePolicy(
//!   .deviceOwnerAuthentication)` — same UX, weaker enforcement
//!   (in-process check instead of keychain ACL), acceptable for dev.
//!
//! The provider holds no key material at rest — every call returns a
//! fresh `Zeroizing` DEK — so `Debug` can never leak the key.

use std::ptr;
use std::sync::mpsc;
use std::sync::OnceLock;

use block2::RcBlock;
use core_foundation::base::{CFType, OSStatus, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_foundation_sys::base::CFTypeRef;
use core_foundation_sys::error::CFErrorRef;
use core_foundation_sys::string::CFStringRef;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LAContext, LAPolicy};
use security_framework_sys::access_control::{
    kSecAccessControlUserPresence, kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
    SecAccessControlCreateWithFlags,
};
use security_framework_sys::base::{errSecAuthFailed, errSecDuplicateItem, errSecItemNotFound};
use security_framework_sys::item::{
    kSecAttrAccessControl, kSecAttrAccount, kSecAttrService, kSecClass, kSecClassGenericPassword,
    kSecMatchLimit, kSecReturnData, kSecValueData,
};
use security_framework_sys::keychain_item::{SecItemAdd, SecItemCopyMatching, SecItemDelete};
use thiserror::Error;
use zeroize::Zeroizing;

/// `keys.enc` source byte for [`SystemAuthDekProvider`] (0x02 is reserved
/// for a future server-issued DEK).
pub const DEK_SOURCE_SYSTEM_AUTH: u8 = 0x01;

pub const DEK_LEN: usize = 32;

const KEYCHAIN_SERVICE: &str = "com.getmarvis.marvis";
const KEYCHAIN_ACCOUNT: &str = "keys-dek";
/// The LA-gated fallback item uses a separate account so it can never be
/// mistaken for (or shadow) the ACL item.
const KEYCHAIN_ACCOUNT_FALLBACK: &str = "keys-dek-fallback";
/// `kSecUseOperationPrompt` / LA `localizedReason` text.
const PROMPT: &str = "Unlock Marvis to access your API keys";

// `SecItem.h` constants security-framework-sys doesn't export.
const ERR_SEC_USER_CANCELED: OSStatus = -128;
const ERR_SEC_INTERACTION_NOT_ALLOWED: OSStatus = -25308;
const ERR_SEC_MISSING_ENTITLEMENT: OSStatus = -34018;
/// `kLAErrorUserCancel` from LocalAuthentication.
const LA_ERROR_USER_CANCEL: isize = -2;

#[link(name = "Security", kind = "framework")]
extern "C" {
    static kSecMatchLimitOne: CFStringRef;
    static kSecAttrAccessible: CFStringRef;
    static kSecUseOperationPrompt: CFStringRef;
}

/// Which keychain backend this provider instance settled on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    /// Signed/normal: ACL'd item — macOS enforces the user-presence
    /// prompt inside the keychain.
    KeychainAcl,
    /// Unsigned/dev fallback: plain item gated by an in-process
    /// `LAContext.evaluatePolicy` call.
    LaGate,
}

#[derive(Debug, Error)]
pub enum DekError {
    /// The user dismissed the Touch ID / password prompt.
    #[error("authentication canceled")]
    Canceled,
    /// LA policy evaluation failed for a non-cancel reason.
    #[error("system authentication failed (LA error {0})")]
    AuthFailed(isize),
    /// `keys.enc` exists but the keychain item is gone — the file can
    /// never decrypt; the user must reset.
    #[error("keystore key missing — reset required")]
    Missing,
    /// The stored item wasn't a 32-byte DEK — keychain contents corrupt.
    #[error("keystore keychain item is corrupt")]
    Corrupt,
    /// A Security-framework call returned an OSStatus we don't special-case.
    #[error("keychain error {0}")]
    Keychain(OSStatus),
    /// Marker: the ACL backend isn't usable in this environment — the
    /// provider swaps to [`Backend::LaGate`] and retries. Never surfaces
    /// to callers of `get`/`get_or_create`.
    #[error("ACL keychain unavailable (OSStatus {0})")]
    AclUnavailable(OSStatus),
    /// A `SecItemAdd` raced an existing item — the caller re-reads.
    /// Internal marker like `AclUnavailable`; never surfaces to UI.
    #[error("keychain item already exists")]
    Duplicate,
}

/// The `keys.enc` DEK source contract.
///
/// `get` may prompt the user (Keychain ACL / LA evaluation) — it is the
/// "system password / Touch ID / Face ID" moment of an unlock.
/// `get_or_create` silently creates the item on first run (`SecItemAdd`
/// never prompts); when the item already exists it degrades to `get`.
pub trait DekProvider: Send + Sync {
    /// The `keys.enc` source byte this provider stamps and accepts.
    fn source(&self) -> u8;
    /// Retrieve the DEK — may show a system-auth prompt.
    fn get(&self) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError>;
    /// Retrieve the DEK, creating the item first if none exists.
    fn get_or_create(&self) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError>;
    /// Delete the backing item(s) — the `keystore_reset` escape: clears
    /// any stored DEK so the next `init` can't pick up a divergent one
    /// (e.g. an ACL item and a fallback item written by different runs).
    /// Best-effort — failures are logged, not returned.
    fn delete(&self);
}

/// Production provider: random DEK in the macOS Keychain, gated by
/// `userPresence` ACL (or the LA-evaluatePolicy fallback on unsigned
/// builds). See module docs for the two-backend scheme.
#[derive(Debug)]
pub struct SystemAuthDekProvider {
    backend: OnceLock<Backend>,
}

impl SystemAuthDekProvider {
    pub fn new() -> Self {
        Self {
            backend: OnceLock::new(),
        }
    }

    /// Run `acl` first; on an environment-level ACL failure switch the
    /// cached backend to `LaGate` and run `la`. Once a backend has won,
    /// later calls go straight to it — the unsigned-build detection
    /// happens exactly once per provider instance.
    fn with_backend<F, G>(&self, acl: F, la: G) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError>
    where
        F: Fn() -> Result<Zeroizing<[u8; DEK_LEN]>, DekError>,
        G: Fn() -> Result<Zeroizing<[u8; DEK_LEN]>, DekError>,
    {
        if let Some(backend) = self.backend.get() {
            return match backend {
                Backend::KeychainAcl => acl(),
                Backend::LaGate => la(),
            };
        }
        match acl() {
            Ok(dek) => {
                self.choose(Backend::KeychainAcl);
                Ok(dek)
            }
            Err(DekError::AclUnavailable(status)) => {
                log::warn!("keystore: ACL keychain unusable ({status}); falling back to LA gate");
                self.choose(Backend::LaGate);
                la()
            }
            Err(e) => Err(e),
        }
    }

    fn choose(&self, backend: Backend) {
        // A race would compute the same backend anyway; first set wins.
        let _ = self.backend.set(backend);
        log::info!("keystore: DEK backend = {backend:?}");
    }

    // -- KeychainAcl backend ------------------------------------------------

    /// `SecItemCopyMatching` on the ACL item. `kSecUseOperationPrompt`
    /// sets the dialog's reason text; the `userPresence` ACL makes macOS
    /// ask for Touch ID / Face ID / system password before releasing
    /// the item — every read prompts, which is the point.
    fn acl_get(&self) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError> {
        let mut result: CFTypeRef = ptr::null();
        // SAFETY: extern `kSec*` statics are live immutable CFStringRefs;
        // `query` is a live CFDictionary; `result` is a valid out-pointer,
        // filled only on errSecSuccess (Create rule → we own it and
        // `wrap_under_create_rule` releases it).
        let status = unsafe {
            let query = CFDictionary::from_CFType_pairs(&[
                (sec_str(kSecClass), sec_str(kSecClassGenericPassword)),
                (
                    sec_str(kSecAttrService),
                    CFString::new(KEYCHAIN_SERVICE).as_CFType(),
                ),
                (
                    sec_str(kSecAttrAccount),
                    CFString::new(KEYCHAIN_ACCOUNT).as_CFType(),
                ),
                (sec_str(kSecReturnData), CFBoolean::true_value().as_CFType()),
                (sec_str(kSecMatchLimit), sec_str(kSecMatchLimitOne)),
                (
                    sec_str(kSecUseOperationPrompt),
                    CFString::new(PROMPT).as_CFType(),
                ),
            ]);
            SecItemCopyMatching(query.as_concrete_TypeRef(), &mut result)
        };
        match status {
            0 => dek_from_cftype(result),
            ERR_SEC_USER_CANCELED => Err(DekError::Canceled),
            s if s == errSecItemNotFound => Err(DekError::Missing),
            s if acl_env_failure(s) => Err(DekError::AclUnavailable(s)),
            s => Err(DekError::Keychain(s)),
        }
    }

    /// `SecItemAdd` of a fresh random DEK with the user-presence ACL.
    /// Adding never prompts — the ACL governs *reads*. A duplicate item
    /// (keys.enc reset, reinstall) degrades to `acl_get`.
    fn acl_get_or_create(&self) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError> {
        let access = sec_access_control()?;
        let dek = Zeroizing::new(rand::random::<[u8; DEK_LEN]>());
        let data = CFData::from_buffer(&dek[..]);
        // SAFETY: extern `kSec*` statics are live immutable CFStringRefs;
        // `attrs` is a live CFDictionary; result out-param unused.
        let status = unsafe {
            let attrs = CFDictionary::from_CFType_pairs(&[
                (sec_str(kSecClass), sec_str(kSecClassGenericPassword)),
                (
                    sec_str(kSecAttrService),
                    CFString::new(KEYCHAIN_SERVICE).as_CFType(),
                ),
                (
                    sec_str(kSecAttrAccount),
                    CFString::new(KEYCHAIN_ACCOUNT).as_CFType(),
                ),
                (sec_str(kSecValueData), data.as_CFType()),
                (sec_str(kSecAttrAccessControl), access),
            ]);
            SecItemAdd(attrs.as_concrete_TypeRef(), ptr::null_mut())
        };
        match status {
            0 => Ok(dek),
            // Item already exists — fetch it (this does prompt, which is
            // correct: we're about to trust that DEK).
            ERR_SEC_USER_CANCELED => Err(DekError::Canceled),
            s if s == errSecDuplicateItem => self.acl_get(),
            s if acl_env_failure(s) => Err(DekError::AclUnavailable(s)),
            s => Err(DekError::Keychain(s)),
        }
    }

    // -- LaGate fallback backend --------------------------------------------

    /// `LAContext.evaluatePolicy(.deviceOwnerAuthentication)` — blocks
    /// the calling thread until the reply block fires on LA's private
    /// queue (safe on any thread: the reply never dispatches to main).
    fn la_authenticate(&self) -> Result<(), DekError> {
        let context = unsafe { LAContext::new() };
        let (tx, rx) = mpsc::channel();
        let block = RcBlock::new(move |success: Bool, error: *mut NSError| {
            // SAFETY: `error` is a valid `NSError *` or null.
            let code = unsafe { error.as_ref().map(|e| e.code()) };
            let _ = tx.send((success.as_bool(), code));
        });
        let reason = NSString::from_str(PROMPT);
        // SAFETY: `reason` is non-nil/non-empty (mandatory); `block`
        // matches `void (^)(BOOL, NSError *)` and is Send (mpsc sender).
        unsafe {
            context.evaluatePolicy_localizedReason_reply(
                LAPolicy::DeviceOwnerAuthentication,
                &reason,
                &block,
            );
        }
        match rx.recv_timeout(std::time::Duration::from_secs(300)) {
            Ok((true, _)) => Ok(()),
            Ok((false, Some(code))) if code == LA_ERROR_USER_CANCEL => Err(DekError::Canceled),
            Ok((false, code)) => Err(DekError::AuthFailed(code.unwrap_or(0))),
            // Reply never fired or timed out — treat as failure rather
            // than holding the keystore mutex forever.
            Err(_) => Err(DekError::AuthFailed(0)),
        }
    }

    /// Plain-item read. No ACL, no prompt at the keychain layer — the
    /// in-process `la_authenticate` is the gate, so authenticate first.
    fn la_get(&self) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError> {
        self.la_authenticate()?;
        self.copy_item(KEYCHAIN_ACCOUNT_FALLBACK)
    }

    /// Silent create on first run (mirrors `SecItemAdd` on the ACL path);
    /// an existing item degrades to `la_get`.
    fn la_get_or_create(&self) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError> {
        match self.copy_item(KEYCHAIN_ACCOUNT_FALLBACK) {
            Ok(_) => self.la_get(),
            Err(DekError::Missing) => {
                let dek = Zeroizing::new(rand::random::<[u8; DEK_LEN]>());
                match self.add_plain_item(&dek) {
                    Ok(()) => Ok(dek),
                    // Lost an add-race — the winner's DEK is the truth;
                    // returning our fresh one would wedge every decrypt.
                    Err(DekError::Duplicate) => self.la_get(),
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    /// `SecItemCopyMatching` on a plain (non-ACL) generic-password item.
    fn copy_item(&self, account: &str) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError> {
        let mut result: CFTypeRef = ptr::null();
        // SAFETY: same contract as `acl_get`.
        let status = unsafe {
            let query = CFDictionary::from_CFType_pairs(&[
                (sec_str(kSecClass), sec_str(kSecClassGenericPassword)),
                (
                    sec_str(kSecAttrService),
                    CFString::new(KEYCHAIN_SERVICE).as_CFType(),
                ),
                (sec_str(kSecAttrAccount), CFString::new(account).as_CFType()),
                (sec_str(kSecReturnData), CFBoolean::true_value().as_CFType()),
                (sec_str(kSecMatchLimit), sec_str(kSecMatchLimitOne)),
            ]);
            SecItemCopyMatching(query.as_concrete_TypeRef(), &mut result)
        };
        match status {
            0 => dek_from_cftype(result),
            s if s == errSecItemNotFound => Err(DekError::Missing),
            s => Err(DekError::Keychain(s)),
        }
    }

    /// `SecItemAdd` of a plain item pinned to this device.
    fn add_plain_item(&self, dek: &[u8; DEK_LEN]) -> Result<(), DekError> {
        let data = CFData::from_buffer(dek);
        // SAFETY: extern `kSec*` statics are live immutable CFStringRefs;
        // `attrs` is a live CFDictionary; result out-param unused.
        let status = unsafe {
            let attrs = CFDictionary::from_CFType_pairs(&[
                (sec_str(kSecClass), sec_str(kSecClassGenericPassword)),
                (
                    sec_str(kSecAttrService),
                    CFString::new(KEYCHAIN_SERVICE).as_CFType(),
                ),
                (
                    sec_str(kSecAttrAccount),
                    CFString::new(KEYCHAIN_ACCOUNT_FALLBACK).as_CFType(),
                ),
                (sec_str(kSecValueData), data.as_CFType()),
                (
                    sec_str(kSecAttrAccessible),
                    sec_str(kSecAttrAccessibleWhenUnlockedThisDeviceOnly),
                ),
            ]);
            SecItemAdd(attrs.as_concrete_TypeRef(), ptr::null_mut())
        };
        match status {
            0 => Ok(()),
            s if s == errSecDuplicateItem => Err(DekError::Duplicate),
            s => Err(DekError::Keychain(s)),
        }
    }

    /// `SecItemDelete` for both accounts — best-effort, used by reset.
    fn delete_item(account: &str) {
        // SAFETY: extern `kSec*` statics are live immutable CFStringRefs;
        // `query` is a live CFDictionary. Deleting an ACL item may itself
        // prompt — acceptable on an explicit user reset.
        let status = unsafe {
            let query = CFDictionary::from_CFType_pairs(&[
                (sec_str(kSecClass), sec_str(kSecClassGenericPassword)),
                (
                    sec_str(kSecAttrService),
                    CFString::new(KEYCHAIN_SERVICE).as_CFType(),
                ),
                (sec_str(kSecAttrAccount), CFString::new(account).as_CFType()),
            ]);
            SecItemDelete(query.as_concrete_TypeRef())
        };
        if status != 0 && status != errSecItemNotFound {
            log::warn!("keystore: SecItemDelete({account}) failed: {status}");
        }
    }
}

impl Default for SystemAuthDekProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DekProvider for SystemAuthDekProvider {
    fn source(&self) -> u8 {
        DEK_SOURCE_SYSTEM_AUTH
    }

    fn get(&self) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError> {
        self.with_backend(|| self.acl_get(), || self.la_get())
    }

    fn get_or_create(&self) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError> {
        self.with_backend(|| self.acl_get_or_create(), || self.la_get_or_create())
    }

    fn delete(&self) {
        // Both accounts, whichever backend wrote them — a reset must not
        // leave a divergent DEK behind for the next `init` to pick up.
        Self::delete_item(KEYCHAIN_ACCOUNT);
        Self::delete_item(KEYCHAIN_ACCOUNT_FALLBACK);
    }
}

/// Test seam: a fixed DEK, no keychain, no prompts. `pub` so
/// `keystore`/`lib` tests can inject it — production wires
/// [`SystemAuthDekProvider`] only.
#[cfg(test)]
pub struct StaticDekProvider(pub [u8; DEK_LEN]);

#[cfg(test)]
impl std::fmt::Debug for StaticDekProvider {
    /// Never print the fixed key — same rule as `Keyring`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StaticDekProvider(…)")
    }
}

#[cfg(test)]
impl DekProvider for StaticDekProvider {
    fn source(&self) -> u8 {
        DEK_SOURCE_SYSTEM_AUTH
    }

    fn get(&self) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError> {
        Ok(Zeroizing::new(self.0))
    }

    fn get_or_create(&self) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError> {
        Ok(Zeroizing::new(self.0))
    }

    fn delete(&self) {}
}

// ---------------------------------------------------------------------------
// FFI helpers
// ---------------------------------------------------------------------------

/// Borrow a `SecItem.h` `CFStringRef` static as a `CFType` for
/// dictionary pairs (`get` rule — statics are retained, not owned).
fn sec_str(r: CFStringRef) -> CFType {
    // SAFETY: `r` is a live immutable CFStringRef exported by
    // Security.framework; get-rule retain is correct for a static.
    unsafe { CFType::wrap_under_get_rule(r as CFTypeRef) }
}

/// `SecAccessControlCreateWithFlags(kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
/// kSecAccessControlUserPresence)` — the item requires Touch ID / Face ID
/// / system password on every read and never leaves this Mac
/// (`ThisDeviceOnly` blocks iCloud keychain sync).
fn sec_access_control() -> Result<CFType, DekError> {
    let mut error: CFErrorRef = ptr::null_mut();
    // SAFETY: all args are valid constants; a null error out-pointer
    // just means "don't report the CFError". Create rule → we own the
    // result and wrap it so it is released on drop.
    let access = unsafe {
        SecAccessControlCreateWithFlags(
            ptr::null(),
            kSecAttrAccessibleWhenUnlockedThisDeviceOnly as CFTypeRef,
            kSecAccessControlUserPresence,
            &mut error,
        )
    };
    if access.is_null() {
        if !error.is_null() {
            // SAFETY: non-null CFErrorRef we own (Create rule).
            let _ = unsafe { CFType::wrap_under_create_rule(error as CFTypeRef) };
        }
        // Can't even build the ACL object — the whole ACL path is
        // suspect, so route through the fallback detection.
        return Err(DekError::AclUnavailable(-1));
    }
    // SAFETY: non-null SecAccessControlRef, owned by us (Create rule).
    Ok(unsafe { CFType::wrap_under_create_rule(access as CFTypeRef) })
}

/// OSStatus values that mean "this binary can't do ACL keychain ops"
/// (unsigned dev build, no user interaction possible) — the trigger for
/// the `LaGate` fallback. `errSecUserCanceled` is deliberately NOT here:
/// a cancel is a user action, not an environment failure. Neither is
/// `errSecAuthFailed`: on a *signed* build it can mean the user's auth
/// simply failed, and flipping the backend then would hunt for a
/// fallback item that was never written — a misleading reset wedge.
fn acl_env_failure(status: OSStatus) -> bool {
    status == ERR_SEC_MISSING_ENTITLEMENT || status == ERR_SEC_INTERACTION_NOT_ALLOWED
}

/// Extract a 32-byte DEK from a `SecItemCopyMatching` result (a
/// `CFDataRef` under the Create rule — we own it).
///
/// SAFETY: caller guarantees `result` is a valid `CFDataRef` produced by
/// a successful `SecItemCopyMatching` with `kSecReturnData`.
fn dek_from_cftype(result: CFTypeRef) -> Result<Zeroizing<[u8; DEK_LEN]>, DekError> {
    if result.is_null() {
        return Err(DekError::Corrupt);
    }
    // SAFETY: per caller contract `result` is an owned CFDataRef.
    let data = unsafe { CFData::wrap_under_create_rule(result as _) };
    let bytes = data.bytes();
    if bytes.len() != DEK_LEN {
        return Err(DekError::Corrupt);
    }
    let mut dek = Zeroizing::new([0u8; DEK_LEN]);
    dek.copy_from_slice(bytes);
    Ok(dek)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seam every keystore test injects: fixed DEK, no system calls.
    #[test]
    fn static_provider_returns_fixed_dek() {
        let p = StaticDekProvider([7; DEK_LEN]);
        assert_eq!(*p.get().unwrap(), [7; DEK_LEN]);
        assert_eq!(*p.get_or_create().unwrap(), [7; DEK_LEN]);
        assert_eq!(p.source(), DEK_SOURCE_SYSTEM_AUTH);
    }

    /// `Debug` must never print the fixed key material.
    #[test]
    fn static_provider_debug_never_leaks() {
        let p = StaticDekProvider([0xAB; DEK_LEN]);
        assert!(!format!("{p:?}").contains("171")); // 0xAB == 171
    }

    /// The env-failure trigger list is exactly the two unambiguous
    /// unsigned-build codes — user cancel and auth failure must never
    /// trigger a fallback swap (the former is a user action; the latter
    /// is ambiguous on signed builds and would wedge on a missing
    /// fallback item).
    #[test]
    fn acl_env_failure_is_exact() {
        assert!(acl_env_failure(ERR_SEC_MISSING_ENTITLEMENT));
        assert!(acl_env_failure(ERR_SEC_INTERACTION_NOT_ALLOWED));
        assert!(!acl_env_failure(errSecAuthFailed));
        assert!(!acl_env_failure(ERR_SEC_USER_CANCELED));
        assert!(!acl_env_failure(errSecItemNotFound));
        assert!(!acl_env_failure(errSecDuplicateItem));
        assert!(!acl_env_failure(0));
    }

    // NOTE: the Keychain/LA paths can't be exercised headless (they need
    // a GUI session for the prompt). They are covered by manual
    // verification on a signed and an unsigned (`tauri dev`) build.
}
