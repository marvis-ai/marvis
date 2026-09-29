//! Linux permission checks.
//!
//! The XDG screencast portal IS the consent flow — there is no
//! independent "permission" flag to read. `screen_status` reports
//! whether a persisted `PersistMode::ExplicitlyRevoked` restore token
//! exists (written by the first successful portal pick); `screen_request`
//! runs the portal pick itself, since the dialog is the only way consent
//! is granted. Microphone over PulseAudio needs no consent in a
//! non-sandboxed install.

use std::io;

use super::PermissionState;
use crate::paths;

/// Whether screen sharing has been consented to at least once — a
/// restore token on disk means the portal can grant silently.
pub fn screen_status() -> bool {
    paths::portal_token_file().exists()
}

/// The portal dialog IS the request: run the pick, keep the granted
/// restore token, drop the session (a real capture recreates one).
/// Returns whether consent now exists.
pub fn screen_request() -> bool {
    match crate::capture::portal_pick_blocking() {
        Ok(_) => screen_status(),
        Err(e) => {
            log::warn!("permissions: portal screen request failed: {e}");
            false
        }
    }
}

/// PulseAudio captures the mic without OS consent (non-sandboxed), so
/// mic is always authorized.
pub fn mic_status() -> PermissionState {
    PermissionState::Authorized
}

/// No-op on Linux — see [`mic_status`].
pub fn mic_request() -> bool {
    true
}

/// There is no portable privacy pane on Linux (GNOME/KDE/Xfce settings
/// URIs all differ); warn and report success — the same fire-and-forget
/// contract as the other platforms.
pub fn open_prefs(_section: &str) -> io::Result<()> {
    log::warn!("permissions: no portable privacy pane on Linux");
    Ok(())
}
