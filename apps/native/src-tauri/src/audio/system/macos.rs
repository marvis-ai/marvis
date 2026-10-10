mod tap;

use super::{forward_chunks, warn_unsupported, PcmChunk, RawChunk, AUDIO_QUEUE_CAPACITY};
use crate::audio::AudioSource;
use anyhow::{anyhow, Result};
use screencapturekit::cm::{CMSampleBuffer, CMSampleBufferExt};
use screencapturekit::stream::configuration::SCStreamConfiguration;
use screencapturekit::stream::output_type::SCStreamOutputType;
use screencapturekit::stream::sc_stream::SCStream;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

const FLAG_FLOAT: u32 = 1;
const FLAG_BIG_ENDIAN: u32 = 2;
const FLAG_NON_INTERLEAVED: u32 = 0x20;

#[derive(Debug, Clone, Copy)]
struct AudioBufferBytes<'a> {
    data: &'a [u8],
    channels: usize,
}

/// System audio for the "them" channel. Prefers a CoreAudio process tap
/// (system-audio consent only — no "Currently Sharing" screen panel);
/// falls back to a ScreenCaptureKit audio-only stream on older macOS or
/// tap failures. ScreenCaptureKit types stay private; consumers receive
/// normalized chunks.
pub struct SystemAudioSource {
    tap: Option<tap::TapCapture>,
    stream: Option<SCStream>,
    input_tx: Option<SyncSender<RawChunk>>,
    worker: Option<JoinHandle<()>>,
    running: bool,
}

impl SystemAudioSource {
    /// Lazy construction — the ScreenCaptureKit content filter is only
    /// built on the fallback path, so the tap never touches screen
    /// consent or the sharing indicator.
    pub fn new() -> Result<Self> {
        Ok(Self {
            tap: None,
            stream: None,
            input_tx: None,
            worker: None,
            running: false,
        })
    }

    /// ScreenCaptureKit fallback — the audio-only `SCStream` the tap
    /// replaced. macOS still shows the "Currently Sharing" panel for it
    /// (any live SCStream counts as sharing), which is why it is a
    /// fallback rather than the primary path.
    fn start_sck(&mut self, tx: SyncSender<RawChunk>) -> Result<()> {
        let (filter, _, _) = crate::capture::primary_display_source()?;
        let config = SCStreamConfiguration::new()
            .with_captures_audio(true)
            .with_excludes_current_process_audio(true);
        let callback_tx = tx;
        let warned = Arc::new(AtomicBool::new(false));
        let callback_warned = warned.clone();
        let mut stream = SCStream::new(&filter, &config);
        if stream
            .add_output_handler(
                move |sample: CMSampleBuffer, _| {
                    Self::enqueue_sample(&sample, &callback_tx, &callback_warned)
                },
                SCStreamOutputType::Audio,
            )
            .is_none()
        {
            return Err(anyhow!("SCStream rejected the audio output handler"));
        }
        stream
            .start_capture()
            .map_err(|error| anyhow!("failed to start system audio capture: {error}"))?;
        self.stream = Some(stream);
        Ok(())
    }

    fn enqueue_sample(sample: &CMSampleBuffer, tx: &SyncSender<RawChunk>, warned: &AtomicBool) {
        let Some(format) = sample.format_description() else {
            warn_unsupported(warned, "missing format description");
            return;
        };
        let Some(sample_rate) = format.audio_sample_rate() else {
            warn_unsupported(warned, "missing sample rate");
            return;
        };
        let Some(bits) = format.audio_bits_per_channel() else {
            warn_unsupported(warned, "missing bits-per-channel metadata");
            return;
        };
        let Some(flags) = format.audio_format_flags() else {
            warn_unsupported(warned, "missing audio format flags");
            return;
        };
        if flags & FLAG_BIG_ENDIAN != 0 || !matches!(bits, 16 | 32) {
            warn_unsupported(warned, "unsupported byte order or sample width");
            return;
        }
        let channels = format.audio_channel_count().unwrap_or(1) as usize;
        let Some(list) = sample.audio_buffer_list() else {
            warn_unsupported(warned, "missing audio buffer list");
            return;
        };
        let buffers: Vec<_> = list
            .iter()
            .map(|buffer| AudioBufferBytes {
                data: buffer.data(),
                channels: buffer.number_channels() as usize,
            })
            .collect();
        let Some(samples) = decode_pcm(
            &buffers,
            bits as usize,
            flags & FLAG_FLOAT != 0,
            flags & FLAG_NON_INTERLEAVED != 0,
            channels,
        ) else {
            warn_unsupported(warned, "audio buffer layout or PCM data was rejected");
            return;
        };
        match tx.try_send((samples, sample_rate.round() as u32, channels as u16)) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => log::debug!("dropping stale system-audio chunk"),
        }
    }
}

impl AudioSource for SystemAudioSource {
    fn start(&mut self, output: mpsc::Sender<PcmChunk>) -> Result<()> {
        self.stop();
        let (tx, rx) = mpsc::sync_channel(AUDIO_QUEUE_CAPACITY);
        let worker = thread::spawn(move || forward_chunks(rx, output));
        let result = match tap::TapCapture::start(tx.clone()) {
            Ok(capture) => {
                self.tap = Some(capture);
                Ok(())
            }
            Err(tap_error) => {
                log::info!(
                    "system audio: process tap unavailable ({tap_error}); \
                     falling back to ScreenCaptureKit"
                );
                self.start_sck(tx.clone())
            }
        };
        if let Err(error) = result {
            drop(tx);
            let _ = worker.join();
            return Err(error);
        }
        self.input_tx = Some(tx);
        self.worker = Some(worker);
        self.running = true;
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(tap) = self.tap.as_mut() {
            tap.stop();
        }
        self.tap = None;
        if let Some(stream) = self.stream.take() {
            if let Err(error) = stream.stop_capture() {
                log::warn!("SCStream system-audio stop failed: {error}");
            }
            drop(stream);
        }
        drop(self.input_tx.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.running = false;
    }

    fn is_running(&self) -> bool {
        self.running
    }
}

impl Drop for SystemAudioSource {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Decode the AudioBufferList layout used by ScreenCaptureKit v10.
/// Interleaved lists contain one buffer with N channels; non-interleaved lists
/// contain one buffer per channel and are woven into frame-major samples.
fn decode_pcm(
    buffers: &[AudioBufferBytes<'_>],
    bits: usize,
    is_float: bool,
    non_interleaved: bool,
    channels: usize,
) -> Option<Vec<f32>> {
    if buffers.is_empty() || channels == 0 || !matches!(bits, 16 | 32) {
        return None;
    }
    let width = bits / 8;
    let decode = |bytes: &[u8]| -> Option<f32> {
        if bytes.len() != width {
            return None;
        }
        Some(if is_float {
            f32::from_le_bytes(bytes.try_into().ok()?)
        } else if bits == 16 {
            f32::from(i16::from_le_bytes(bytes.try_into().ok()?)) / 32_768.0
        } else {
            i32::from_le_bytes(bytes.try_into().ok()?) as f32 / 2_147_483_648.0
        })
    };
    if non_interleaved {
        if buffers.len() < channels {
            return None;
        }
        let frames = buffers
            .iter()
            .take(channels)
            .map(|buffer| buffer.data.len() / width)
            .min()?;
        let mut output = Vec::with_capacity(frames * channels);
        for frame in 0..frames {
            for buffer in buffers.iter().take(channels) {
                output.push(decode(&buffer.data[frame * width..(frame + 1) * width])?);
            }
        }
        Some(output)
    } else {
        let buffer = buffers.first()?;
        if buffer.channels != channels || buffer.data.len() % (width * channels) != 0 {
            return None;
        }
        buffer.data.chunks_exact(width).map(decode).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_interleaved_float_audio_buffer_bytes() {
        let bytes = [0.25f32.to_le_bytes(), (-0.5f32).to_le_bytes()].concat();
        let result = decode_pcm(
            &[AudioBufferBytes {
                data: &bytes,
                channels: 2,
            }],
            32,
            true,
            false,
            2,
        );
        assert_eq!(result, Some(vec![0.25, -0.5]));
    }

    #[test]
    fn decodes_non_interleaved_signed_audio_buffer_bytes() {
        let left = [i16::MAX.to_le_bytes(), 0i16.to_le_bytes()].concat();
        let right = [0i16.to_le_bytes(), i16::MIN.to_le_bytes()].concat();
        let result = decode_pcm(
            &[
                AudioBufferBytes {
                    data: &left,
                    channels: 1,
                },
                AudioBufferBytes {
                    data: &right,
                    channels: 1,
                },
            ],
            16,
            false,
            true,
            2,
        );
        assert_eq!(result, Some(vec![0.9999695, 0.0, 0.0, -1.0]));
    }

    #[test]
    fn rejects_empty_or_malformed_audio_buffers() {
        assert_eq!(decode_pcm(&[], 32, true, false, 1), None);
        assert_eq!(
            decode_pcm(
                &[AudioBufferBytes {
                    data: &[0, 1, 2],
                    channels: 1
                }],
                16,
                false,
                false,
                1
            ),
            None
        );
    }
}
