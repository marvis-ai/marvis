//! macOS permission checks for screen recording and microphone.
//!
//! Screen-recording status goes through the CoreGraphics preflight/request
//! pair; microphone status goes through AVFoundation's `AVCaptureDevice`
//! authorization APIs. Every entry point is safe to call at startup: a
//! missing weak-linked symbol degrades to `false`/`Denied` rather than a
//! panic.

use std::io;
use std::process::Command;
use std::sync::mpsc;

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaType, AVMediaTypeAudio};

use super::PermissionState;

// CoreGraphics exposes these as `Boolean` (unsigned char), not `bool`.
// Present on macOS 10.15+; safe to declare unconditionally.
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> u8;
    fn CGRequestScreenCaptureAccess() -> u8;
}

/// Whether the app already holds screen-recording permission.
pub fn screen_status() -> bool {
    // SAFETY: no arguments, no retained state; returns a C Boolean.
    unsafe { CGPreflightScreenCaptureAccess() != 0 }
}

/// Ask for screen-recording permission; shows the system prompt only when
/// the status is undetermined, otherwise returns the current authorization.
pub fn screen_request() -> bool {
    // SAFETY: no arguments, no retained state; returns a C Boolean.
    unsafe { CGRequestScreenCaptureAccess() != 0 }
}

/// Microphone authorization state.
pub fn mic_status() -> PermissionState {
    let Some(media_type) = audio_media_type() else {
        return PermissionState::Denied;
    };
    // SAFETY: `media_type` is a live AVFoundation media-type constant.
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
    if status == AVAuthorizationStatus::NotDetermined {
        PermissionState::NotDetermined
    } else if status == AVAuthorizationStatus::Restricted {
        PermissionState::Restricted
    } else if status == AVAuthorizationStatus::Authorized {
        PermissionState::Authorized
    } else {
        PermissionState::Denied
    }
}

/// Ask for microphone permission, blocking until the user answers. When the
/// status was already determined the completion handler fires right away,
/// so this only actually waits on a live prompt.
///
/// MUST NOT be called on the main/UI thread — AVFoundation can dispatch the
/// completion to the main queue, which would deadlock the blocked caller.
pub fn mic_request() -> bool {
    let Some(media_type) = audio_media_type() else {
        return false;
    };
    let (tx, rx) = mpsc::channel();
    let block: RcBlock<dyn Fn(Bool)> = RcBlock::new(move |granted: Bool| {
        let _ = tx.send(granted.as_bool());
    });
    // SAFETY: `media_type` is a valid media-type constant and `block`
    // matches the expected `void (^)(BOOL)` signature.
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(media_type, &block) };
    // Handler runs on an arbitrary queue; if it never fires the dropped
    // sender turns `recv` into an error, which we report as not granted.
    rx.recv().unwrap_or(false)
}

/// Open System Settings to a privacy pane; `section` is the full pane name
/// (`Privacy_ScreenCapture`, `Privacy_Microphone`, …). Fire-and-forget: the
/// `open` child is spawned without waiting, so the caller only learns that
/// the launch was attempted.
pub fn open_prefs(section: &str) -> io::Result<()> {
    Command::new("open")
        .arg(prefs_url(section))
        .spawn()
        .map(|_| ())
}

/// `x-apple.systempreferences:` URL for a privacy pane.
fn prefs_url(section: &str) -> String {
    format!("x-apple.systempreferences:com.apple.preference.security?{section}")
}

/// The audio media-type constant (weak-linked; `None` when AVFoundation's
/// symbol is unavailable).
fn audio_media_type() -> Option<&'static AVMediaType> {
    // SAFETY: reads a weak-linked extern static, which yields `None` when
    // the symbol is not present at runtime.
    unsafe { AVMediaTypeAudio }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the status queries must not panic; the values themselves
    /// are machine-dependent (CI may or may not have granted permission).
    #[test]
    fn status_queries_do_not_panic() {
        let _ = screen_status();
        let _ = mic_status();
    }
}
