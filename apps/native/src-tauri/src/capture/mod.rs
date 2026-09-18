//! `capture` — screen-frame acquisition for ambient observation.
//!
//! Privacy boundary: captured pixels are reduced to JPEG [`Frame`]s held in
//! an in-memory [`RingBuffer`]. Frame bytes are never written to disk and
//! never serialized to JavaScript — only the local LLM pipeline consumes
//! them.
//!
//! [`MacosCapture`] (ScreenCaptureKit) is the production [`FrameSource`];
//! `RingBuffer` and [`frame_hash`] are platform-pure and unit-tested here.

mod macos;
pub use macos::MacosCapture;

use std::collections::VecDeque;

/// One captured screen moment: a JPEG-encoded downscale plus the content
/// hash that deduplicated it against the previous frame.
#[derive(Debug, Clone)]
pub struct Frame {
    /// JPEG bytes (quality 80), `width`×`height` after downscale.
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Unix epoch seconds when the frame was encoded.
    pub ts: i64,
    /// [`frame_hash`] of the pre-resize BGRA buffer.
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
}
