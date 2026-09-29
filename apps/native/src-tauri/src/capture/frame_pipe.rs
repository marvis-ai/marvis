//! `frame_pipe` — the shared half of every platform capture backend.
//!
//! Whatever produced the pixels (ScreenCaptureKit dispatch queue,
//! Windows.Graphics.Capture callback, PipeWire `process` event) hands a
//! [`RawFrame`] — owned BGRA bytes plus `bytes_per_row` — into
//! [`run_worker`], which owns everything expensive: changed-frame dedupe
//! by row-folded hash, downscale to `TARGET_WIDTH`, JPEG encode, and the
//! `on_frame` emit. Producer callbacks stay memcpy-only so they can never
//! stall the OS's delivery thread.
//!
//! ```text
//! platform source ──> extract RawFrame (lock, memcpy BGRA, unlock)
//!                ──mpsc──> run_worker: hash → dedupe → resize → JPEG q80
//!                ──> on_frame(Frame)
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{GenericImageView, Rgba};

use super::{frame_hash_rows, Frame};

/// Longest-side cap for the encoded frame: width is capped at 1600 px,
/// height follows aspect. Screen reading needs legible text — the old
/// 384 px height cap made on-screen text illegible and the vision model
/// confabulated details. `Frame` widths below the cap pass through.
pub(crate) const TARGET_WIDTH: u32 = 1600;
/// `frame_hash` samples every 4096th byte of the raw BGRA buffer.
const HASH_STRIDE: usize = 4096;
const JPEG_QUALITY: u8 = 80;
/// Worker polls the channel this often so `stop()` can interrupt a quiet
/// stream even while the sender is still alive.
const POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// Raw BGRA frame handed from the platform callback to the worker.
/// `data` is a full pixel-buffer copy; `bytes_per_row` may exceed
/// `width * 4` (row alignment padding).
pub(crate) struct RawFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub bytes_per_row: usize,
}

/// Zero-copy strided view over raw BGRA rows. `get_pixel` swizzles
/// B,G,R,A → R,G,B,A so `resize` and `JpegEncoder` see true RGBA ordering.
struct BgraView<'a> {
    data: &'a [u8],
    width: u32,
    height: u32,
    bytes_per_row: usize,
}

impl GenericImageView for BgraView<'_> {
    type Pixel = Rgba<u8>;

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn get_pixel(&self, x: u32, y: u32) -> Rgba<u8> {
        let i = y as usize * self.bytes_per_row + x as usize * 4;
        Rgba([
            self.data[i + 2],
            self.data[i + 1],
            self.data[i],
            self.data[i + 3],
        ])
    }
}

/// BGRA → `(jpeg, out_w, out_h)` width-capped to `target_w`,
/// aspect preserved, q80 — shared by the stream/shot path
/// (`TARGET_WIDTH`) and picker thumbnails.
pub(crate) fn jpeg_at(raw: &RawFrame, target_w: u32) -> Option<(Vec<u8>, u32, u32)> {
    let view = BgraView {
        data: &raw.data,
        width: raw.width,
        height: raw.height,
        bytes_per_row: raw.bytes_per_row,
    };
    let mut jpeg = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY);
    let (out_w, out_h, encoded) = if raw.width > target_w {
        let out_h = (u64::from(raw.height) * u64::from(target_w) / u64::from(raw.width)) as u32;
        let out_h = out_h.max(1);
        let resized = imageops::resize(&view, target_w, out_h, FilterType::Triangle);
        (target_w, out_h, encoder.encode_image(&resized))
    } else {
        (raw.width, raw.height, encoder.encode_image(&view))
    };
    match encoded {
        Ok(()) => Some((jpeg, out_w, out_h)),
        Err(e) => {
            log::warn!("jpeg encode failed ({out_w}x{out_h}): {e}");
            None
        }
    }
}

/// BGRA → JPEG `Frame` at `TARGET_WIDTH` — stream worker + one-shot
/// share this so both produce identical `Frame`s. `hash` is the
/// caller's dedupe hash (0 for single-shots).
pub(crate) fn encode_frame(raw: &RawFrame, hash: u64) -> Option<Frame> {
    let (jpeg, width, height) = jpeg_at(raw, TARGET_WIDTH)?;
    Some(Frame {
        jpeg,
        width,
        height,
        ts: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        hash,
    })
}

/// Worker loop: dedupe by raw-BGRA hash, downscale, JPEG-encode, emit.
/// Exits when `stop` is set or the sender disconnects.
pub(crate) fn run_worker(
    rx: mpsc::Receiver<RawFrame>,
    stop: Arc<AtomicBool>,
    on_frame: Box<dyn Fn(Frame) + Send>,
) {
    let mut last_hash: Option<u64> = None;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let raw = match rx.recv_timeout(POLL) {
            Ok(raw) => raw,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        // Hash pixel bytes only — `bytes_per_row` padding is pool garbage
        // that would defeat dedupe (identical screens → different hashes).
        let hash = frame_hash_rows(
            &raw.data,
            raw.width,
            raw.height,
            raw.bytes_per_row,
            HASH_STRIDE,
        );
        if Some(hash) == last_hash {
            continue; // changed-frames-only: identical screen, drop entirely
        }

        // Leave last_hash untouched on encode failure so the next
        // identical frame retries (unchanged rule).
        if let Some(frame) = encode_frame(&raw, hash) {
            last_hash = Some(hash);
            on_frame(frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_frame_caps_width_at_target() {
        // 3200x2000 solid-black BGRA (bytes_per_row == width * 4).
        let raw = RawFrame {
            data: vec![0u8; 3200 * 2000 * 4],
            width: 3200,
            height: 2000,
            bytes_per_row: 3200 * 4,
        };
        let frame = encode_frame(&raw, 0).expect("encode succeeds");
        assert_eq!(frame.width, 1600);
        assert_eq!(frame.height, 1000);
        assert!(!frame.jpeg.is_empty());
        assert_eq!(frame.hash, 0);
    }

    #[test]
    fn encode_frame_passes_small_sources_through() {
        let raw = RawFrame {
            data: vec![0u8; 800 * 600 * 4],
            width: 800,
            height: 600,
            bytes_per_row: 800 * 4,
        };
        let frame = encode_frame(&raw, 7).expect("encode succeeds");
        assert_eq!((frame.width, frame.height), (800, 600));
        assert_eq!(frame.hash, 7);
    }

    #[test]
    fn jpeg_at_caps_width_at_target() {
        // 800x400 BGRA → 480 wide must produce a 480x240 jpeg.
        let raw = RawFrame {
            data: vec![0u8; 800 * 400 * 4],
            width: 800,
            height: 400,
            bytes_per_row: 800 * 4,
        };
        let (_jpeg, w, h) = jpeg_at(&raw, 480).expect("encode failed");
        assert_eq!((w, h), (480, 240));
    }

    #[test]
    fn jpeg_at_passes_small_sources_through() {
        let raw = RawFrame {
            data: vec![0u8; 320 * 200 * 4],
            width: 320,
            height: 200,
            bytes_per_row: 320 * 4,
        };
        let (_jpeg, w, h) = jpeg_at(&raw, 480).expect("encode failed");
        assert_eq!((w, h), (320, 200));
    }
}
