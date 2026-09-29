//! Windows permission checks.
//!
//! Screen capture has no OS preflight — Windows.Graphics.Capture is the
//! consent boundary itself (apps may opt out, everything else is
//! capturable), so `screen_status`/`screen_request` are always granted.
//! Microphone consent lives under `DeviceAccessInformation`; there is no
//! explicit "request" API, so `mic_request` performs a real
//! `MediaCapture::InitializeAsync` — the call that surfaces the OS
//! prompt — then drops the device immediately.

use std::io;
use std::process::Command;

use windows::Devices::Enumeration::{DeviceAccessInformation, DeviceAccessStatus, DeviceClass};
use windows::Media::Capture::MediaCapture;

use super::PermissionState;

/// Whether the app already holds screen-recording permission — always
/// true: WGC asks no preflight, and the picker/capture is the consent.
pub fn screen_status() -> bool {
    true
}

/// No-op on Windows — see [`screen_status`].
pub fn screen_request() -> bool {
    true
}

/// Microphone authorization state via `DeviceAccessInformation`.
pub fn mic_status() -> PermissionState {
    let status = DeviceAccessInformation::CreateFromDeviceClass(DeviceClass::AudioCapture)
        .and_then(|info| info.CurrentStatus());
    match status {
        Ok(DeviceAccessStatus::Allowed) => PermissionState::Authorized,
        Ok(DeviceAccessStatus::DeniedBySystem | DeviceAccessStatus::DeniedByUser) => {
            PermissionState::Denied
        }
        // `Unspecified` means never asked; errors degrade to
        // NotDetermined rather than a false "denied".
        _ => PermissionState::NotDetermined,
    }
}

/// Ask for microphone permission by initializing a `MediaCapture` — the
/// API that triggers the OS consent prompt. Runs on the calling thread
/// (callers use `spawn_blocking`); the capture device is dropped
/// immediately after the answer lands.
pub fn mic_request() -> bool {
    match mic_status() {
        PermissionState::Authorized => return true,
        PermissionState::Denied | PermissionState::Restricted => return false,
        _ => {}
    }
    // `MediaCapture` is a WinRT activation (RoActivateInstance) — no
    // explicit COM apartment init needed, and `join()` on the async op
    // blocks the calling (spawn_blocking) thread until consent resolves.
    MediaCapture::new()
        .and_then(|capture| capture.InitializeAsync())
        .and_then(|op| op.join())
        .is_ok()
}

/// Open Settings to a privacy pane; `section` keeps the macOS pane names
/// (`Privacy_Microphone`, `Privacy_ScreenCapture`, …) and maps onto the
/// nearest `ms-settings:` URI. Fire-and-forget like the macOS `open`.
pub fn open_prefs(section: &str) -> io::Result<()> {
    let pane = if section.contains("Microphone") {
        "ms-settings:privacy-microphone"
    } else {
        "ms-settings:privacy"
    };
    Command::new("cmd")
        .args(["/c", "start", "", pane])
        .spawn()
        .map(|_| ())
}
