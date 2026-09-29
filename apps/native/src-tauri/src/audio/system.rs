//! `audio::system` — the loopback "them" channel, per OS.
//!
//! Each backend produces interleaved `f32` samples plus the source rate
//! and channel count (`RawChunk`); the shared worker normalizes to
//! mono/16 kHz `PcmChunk`s via [`super::normalize_pcm`]. Only the PCM
//! contract is shared — the acquisition is entirely per-platform:
//!
//! - macOS: ScreenCaptureKit `SCStream` audio output on the primary
//!   display's content filter (`excludes_current_process_audio`).
//! - Windows: WASAPI shared-mode loopback on the default render device.
//! - Linux: PulseAudio monitor source of the default sink (the
//!   `pipewire-pulse` shim exposes the same protocol under PipeWire).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};

use super::{normalize_pcm, PcmChunk};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::SystemAudioSource;
#[cfg(target_os = "macos")]
pub use macos::SystemAudioSource;
#[cfg(target_os = "windows")]
pub use windows::SystemAudioSource;

/// Deep enough to ride out short CPU spikes (model loads, decode bursts)
/// without dropping samples destined for transcription.
const AUDIO_QUEUE_CAPACITY: usize = 64;

/// Interleaved `f32` samples + `(sample_rate, channels)`.
type RawChunk = (Vec<f32>, u32, u16);

/// Normalize worker: `RawChunk` → mono/16 kHz `PcmChunk`, forwards to the
/// session's `output` until either end disconnects.
fn forward_chunks(rx: Receiver<RawChunk>, output: mpsc::Sender<PcmChunk>) {
    while let Ok((samples, sample_rate, channels)) = rx.recv() {
        if output
            .send(normalize_pcm(&samples, sample_rate, channels))
            .is_err()
        {
            break;
        }
    }
}

/// Log once per stream when a chunk's layout is rejected — the samples
/// are dropped (the stream keeps running), so silence would be silent.
fn warn_unsupported(warned: &AtomicBool, reason: &str) {
    if !warned.swap(true, Ordering::Relaxed) {
        log::warn!("dropping system-audio samples: unsupported or rejected format ({reason})");
    }
}

/// Interleaved little-endian PCM bytes → `f32` samples. Supports the
/// layouts loopback/monitor sources deliver: f32, s16, s32. Returns
/// `None` on any byte-count mismatch rather than a partial decode.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn interleaved_pcm_to_f32(bytes: &[u8], bits: u16, is_float: bool) -> Option<Vec<f32>> {
    let width = usize::from(bits / 8);
    if bytes.is_empty() || bytes.len() % width != 0 {
        return None;
    }
    let decode = |b: &[u8]| -> Option<f32> {
        Some(match (is_float, bits) {
            (true, 32) => f32::from_le_bytes(b.try_into().ok()?),
            (false, 16) => f32::from(i16::from_le_bytes(b.try_into().ok()?)) / 32_768.0,
            (false, 32) => i32::from_le_bytes(b.try_into().ok()?) as f32 / 2_147_483_648.0,
            _ => return None,
        })
    };
    bytes.chunks_exact(width).map(decode).collect()
}

/// A silent `RawChunk` for `frames` frames — WASAPI flags muted packets
/// instead of filling them; emitting zeros keeps transcript timing
/// continuous through desktop silence.
#[cfg(target_os = "windows")]
fn silent_chunk(frames: usize, channels: u16, sample_rate: u32) -> RawChunk {
    (
        vec![0.0; frames * usize::from(channels)],
        sample_rate,
        channels,
    )
}

/// Channel for the backend → [`super::AudioSource::try_recv_status`] fatal
/// errors (init failures, device loss). Matches `MicSource`'s contract:
/// only fatal errors land here; glitches log-and-continue.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn status_channel() -> (mpsc::Sender<String>, mpsc::Receiver<String>) {
    mpsc::channel()
}
