//! `capture` — screen-frame acquisition for ambient observation.
//!
//! Privacy boundary: captured pixels are reduced to JPEG [`Frame`]s held in
//! an in-memory [`RingBuffer`]. Frame bytes are never written to disk and
//! never serialized to JavaScript — only the local LLM pipeline consumes
//! them. The one deliberate exception is the share picker: small JPEG
//! thumbs go to Marvis's own content-protected `picker` window so the
//! user can see what they're choosing.
//!
//! [`MacosCapture`] (ScreenCaptureKit) is the production [`FrameSource`];
//! `RingBuffer` and [`frame_hash`] are platform-pure and unit-tested here.

pub(crate) mod controller;
pub(crate) mod macos;
pub(crate) use macos::primary_display_filter;
pub(crate) use macos::shot_fullscreen;
pub use macos::MacosCapture;
pub(crate) use macos::{pick_candidates, resolve_candidate, thumb_for};

use std::collections::VecDeque;

use serde::Serialize;

/// One shareable target offered by the picker window — meta only;
/// thumbnails arrive over `picker:thumb` emits.
#[derive(Debug, Clone, Serialize)]
pub struct PickCandidate {
    /// Opaque resolver key: `"d:<display_id>"`, `"w:<window_id>"`,
    /// `"a:<bundle_id>"` — re-resolved against fresh content on pick.
    pub id: String,
    /// `"display" | "window" | "app"` — matches `CaptureTarget.kind`.
    pub kind: &'static str,
    /// Primary card text — `"Screen N"`, window title, or app name.
    pub label: String,
    /// Secondary line — owning app name for window cards.
    pub sub: Option<String>,
    /// Aspect hint (points/pixels) for the card's thumbnail frame.
    pub w: u32,
    pub h: u32,
    /// `"app"` only: the `"w:..."` id whose thumbnail this card reuses
    /// — an app capture composites at display size, so a real app
    /// thumb would be a mostly-empty display shot.
    pub thumb_of: Option<String>,
}

/// A picker id resolved against fresh `SCShareableContent` — the
/// filter, stream dims, and `capture:state` target fields.
pub struct PickResolution {
    pub filter: screencapturekit::stream::content_filter::SCContentFilter,
    pub w: u32,
    pub h: u32,
    /// `"display" | "window" | "app"`
    pub kind: &'static str,
    pub label: String,
}

/// One captured screen moment: a JPEG-encoded downscale plus the content
/// hash that deduplicated it against the previous frame.
#[derive(Debug, Clone)]
pub struct Frame {
    /// JPEG bytes (quality 80), `width`×`height` after downscale.
    pub jpeg: Vec<u8>,
    /// Frame metadata below is produced for later consumers (status/debug
    /// UIs); only `jpeg` feeds the ask pipeline today.
    #[allow(dead_code)]
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
    /// Unix epoch seconds when the frame was encoded.
    #[allow(dead_code)]
    pub ts: i64,
    /// [`frame_hash`] of the pre-resize BGRA buffer.
    #[allow(dead_code)]
    pub hash: u64,
}

/// Frame ring bounded by both frame count and total JPEG bytes.
///
/// `push` appends, then evicts from the front while over *either* cap —
/// always keeping at least one frame even when a single frame exceeds the
/// byte cap.
pub struct RingBuffer {
    frames: VecDeque<Frame>,
    max_frames: usize,
    max_bytes: usize,
    bytes: usize,
}

impl RingBuffer {
    pub fn new(max_frames: usize, max_bytes: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            max_frames,
            max_bytes,
            bytes: 0,
        }
    }

    pub fn push(&mut self, frame: Frame) {
        self.bytes += frame.jpeg.len();
        self.frames.push_back(frame);
        while self.frames.len() > 1
            && (self.frames.len() > self.max_frames || self.bytes > self.max_bytes)
        {
            if let Some(evicted) = self.frames.pop_front() {
                self.bytes -= evicted.jpeg.len();
            }
        }
    }

    /// Newest frame, cloned so callers may hold it while the buffer mutates.
    pub fn latest(&self) -> Option<Frame> {
        self.frames.back().cloned()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Sum of `jpeg.len()` over all held frames.
    #[allow(dead_code)] // diagnostic accessor; status consumers land later
    pub fn total_bytes(&self) -> usize {
        self.bytes
    }
}

/// Strided FNV-1a over every `stride`-th byte — a cheap screen-change
/// detector. Samples span the whole buffer so an edit anywhere (head,
/// middle, tail) flips the hash without hashing megabytes per frame.
pub fn frame_hash(bgra: &[u8], stride: usize) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for &byte in bgra.iter().step_by(stride.max(1)) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Strided FNV-1a over each row's *pixel* bytes only, folded into one
/// hash. `bytes_per_row` padding is skipped: pool-rotated capture buffers
/// carry different garbage there, so hashing it would make identical
/// screens hash differently and silently defeat changed-frames-only
/// dedupe. `stride` matches `frame_hash`'s semantics (samples per row).
pub fn frame_hash_rows(
    data: &[u8],
    width: u32,
    height: u32,
    bytes_per_row: usize,
    stride: usize,
) -> u64 {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let row_len = width as usize * 4;
    let mut acc = 0xcbf2_9ce4_8422_2325u64;
    for y in 0..height as usize {
        let start = y * bytes_per_row;
        if start >= data.len() {
            break;
        }
        let end = (start + row_len).min(data.len());
        acc ^= frame_hash(&data[start..end], stride);
        acc = acc.wrapping_mul(PRIME);
    }
    acc
}

/// A frame producer with a lifecycle. `Send + Sync` so the app layer can
/// share one source across threads.
pub trait FrameSource: Send + Sync {
    /// Begin producing; `on_frame` fires on a worker thread once per
    /// *changed* frame.
    fn start(&self, on_frame: Box<dyn Fn(Frame) + Send>);
    /// Stop producing and join the worker thread.
    fn stop(&self);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_frame(hash: u64, jpeg_len: usize) -> Frame {
        Frame {
            jpeg: vec![0; jpeg_len],
            width: 4,
            height: 4,
            ts: 0,
            hash,
        }
    }

    #[test]
    fn evicts_oldest_past_frame_cap() {
        let mut rb = RingBuffer::new(120, 64 * 1024 * 1024);
        for i in 0..125 {
            rb.push(fake_frame(i, 100));
        }
        assert_eq!(rb.len(), 120);
        assert_eq!(rb.latest().unwrap().hash, 124);
    }

    #[test]
    fn evicts_oldest_past_byte_cap() {
        let mut rb = RingBuffer::new(120, 1_000);
        for i in 0..5 {
            rb.push(fake_frame(i, 400));
        }
        assert!(rb.total_bytes() <= 1_000);
        assert_eq!(rb.len(), 2);
    }

    #[test]
    fn identical_hash_means_changed_detection_works() {
        let a = vec![0xabu8; 1_000_000];
        assert_eq!(frame_hash(&a, 4), frame_hash(&a, 4));
        let mut b = a.clone();
        b[500_000] = 0xcd;
        assert_ne!(frame_hash(&a, 4), frame_hash(&b, 4));
    }

    #[test]
    fn row_hash_ignores_row_padding() {
        // Two buffers with identical pixels but different `bytes_per_row`
        // padding garbage (pool-rotated capture buffers) must hash equal —
        // otherwise changed-frames-only dedupe silently never fires.
        let (w, h, bpr) = (4u32, 4u32, 20usize); // 16 px bytes + 4 pad/row
        let mut a = vec![0u8; bpr * h as usize];
        for y in 0..h as usize {
            for x in 0..16 {
                a[y * bpr + x] = 0x5a;
            }
            for x in 16..bpr {
                a[y * bpr + x] = 0xde; // padding garbage A
            }
        }
        let mut b = a.clone();
        for y in 0..h as usize {
            for x in 16..bpr {
                b[y * bpr + x] = 0x77; // padding garbage B
            }
        }
        assert_eq!(
            frame_hash_rows(&a, w, h, bpr, 4),
            frame_hash_rows(&b, w, h, bpr, 4)
        );
        // ...while a real pixel change still flips it.
        b[2 * bpr] ^= 0xff;
        assert_ne!(
            frame_hash_rows(&a, w, h, bpr, 4),
            frame_hash_rows(&b, w, h, bpr, 4)
        );
    }
}
