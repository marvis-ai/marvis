//! Permission checks for screen recording and microphone, per OS.
//!
//! The public surface is uniform — `screen_status`, `screen_request`,
//! `mic_status`, `mic_request`, `open_prefs` — but the consent model is
//! not:
//!
//! - macOS: CoreGraphics `CGPreflightScreenCaptureAccess`/`CGRequest…`
//!   for the screen half, AVFoundation `AVCaptureDevice` authorization
//!   for the mic, `x-apple.systempreferences:` for the privacy panes.
//! - Windows: WGC is consent-at-capture (no OS screen preflight — always
//!   `true`); mic goes through `DeviceAccessInformation` +
//!   `MediaCapture::InitializeAsync` (the call that surfaces the OS
//!   consent prompt); `ms-settings:` URIs open Settings.
//! - Linux: the XDG screencast portal IS the consent flow —
//!   `screen_status` reports whether a persisted restore token exists,
//!   `screen_request` runs the portal pick (the dialog IS the request);
//!   PulseAudio mic needs no consent; there is no portable privacy pane.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(target_os = "windows")]
pub use windows::*;

use serde::{Deserialize, Serialize};

/// Authorization state for a media type, serialized for the webview bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionState {
    NotDetermined,
    Restricted,
    Denied,
    Authorized,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PermissionState` must survive a JSON roundtrip so the webview bar
    /// can serialize states back for diagnostics without loss.
    #[test]
    fn permission_state_serde_roundtrip() {
        for state in [
            PermissionState::NotDetermined,
            PermissionState::Restricted,
            PermissionState::Denied,
            PermissionState::Authorized,
        ] {
            let json = serde_json::to_string(&state).unwrap();
            let back: PermissionState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state);
        }
    }
}
