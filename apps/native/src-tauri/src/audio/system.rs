use super::{normalize_pcm, AudioSource, PcmChunk};
use anyhow::{anyhow, Result};
use screencapturekit::cm::{CMSampleBuffer, CMSampleBufferExt};
use screencapturekit::stream::configuration::SCStreamConfiguration;
use screencapturekit::stream::content_filter::SCContentFilter;
use screencapturekit::stream::output_type::SCStreamOutputType;
use screencapturekit::stream::sc_stream::SCStream;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

const AUDIO_QUEUE_CAPACITY: usize = 8;
const FLAG_FLOAT: u32 = 1;
const FLAG_BIG_ENDIAN: u32 = 2;
const FLAG_NON_INTERLEAVED: u32 = 0x20;

#[derive(Debug, Clone, Copy)]
struct AudioBufferBytes<'a> {
    data: &'a [u8],
    channels: usize,
}

/// System audio captured from the primary display's ScreenCaptureKit stream.
/// ScreenCaptureKit types stay private; consumers receive normalized chunks.
pub struct SystemAudioSource {
    filter: Option<SCContentFilter>,
    stream: Option<SCStream>,
    input_tx: Option<SyncSender<PcmChunk>>,
    worker: Option<JoinHandle<()>>,
    running: bool,
}

impl SystemAudioSource {
    pub fn new() -> Result<Self> {
        let (filter, _, _) = crate::capture::primary_display_filter()?;
        Ok(Self {
            filter: Some(filter),
            stream: None,
            input_tx: None,
            worker: None,
            running: false,
        })
    }

    fn enqueue_sample(sample: &CMSampleBuffer, tx: &SyncSender<PcmChunk>) {
        let Some(format) = sample.format_description() else {
            return;
        };
        let Some(sample_rate) = format.audio_sample_rate() else {
            return;
        };
        let Some(bits) = format.audio_bits_per_channel() else {
            return;
        };
        let Some(flags) = format.audio_format_flags() else {
            return;
        };
        if flags & FLAG_BIG_ENDIAN != 0 || !matches!(bits, 16 | 32) {
            return;
        }
        let channels = format.audio_channel_count().unwrap_or(1) as usize;
        let Some(list) = sample.audio_buffer_list() else {
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
            return;
        };
        let chunk = normalize_pcm(&samples, sample_rate.round() as u32, channels as u16);
        if chunk.samples.is_empty() {
            return;
        }
        match tx.try_send(chunk) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => log::debug!("dropping stale system-audio chunk"),
        }
    }
}

impl AudioSource for SystemAudioSource {
    fn start(&mut self, output: mpsc::Sender<PcmChunk>) -> Result<()> {
        self.stop();
        let filter = self
            .filter
            .as_ref()
            .ok_or_else(|| anyhow!("system audio filter unavailable"))?;
        let config = SCStreamConfiguration::new()
            .with_captures_audio(true)
            .with_excludes_current_process_audio(true);
        let (tx, rx) = mpsc::sync_channel(AUDIO_QUEUE_CAPACITY);
        let worker = thread::spawn(move || forward_chunks(rx, output));
        let callback_tx = tx.clone();
        let mut stream = SCStream::new(filter, &config);
        if stream
            .add_output_handler(
                move |sample: CMSampleBuffer, _| Self::enqueue_sample(&sample, &callback_tx),
                SCStreamOutputType::Audio,
            )
            .is_none()
        {
            drop(stream);
            drop(tx);
            let _ = worker.join();
            return Err(anyhow!("SCStream rejected the audio output handler"));
        }
        if let Err(error) = stream.start_capture() {
            drop(stream);
            drop(tx);
            let _ = worker.join();
            return Err(anyhow!("failed to start system audio capture: {error}"));
        }
        self.stream = Some(stream);
        self.input_tx = Some(tx);
        self.worker = Some(worker);
        self.running = true;
        Ok(())
    }

    fn stop(&mut self) {
        let Some(stream) = self.stream.take() else {
            self.running = false;
            return;
        };
        if let Err(error) = stream.stop_capture() {
            log::warn!("SCStream system-audio stop failed: {error}");
        }
        drop(stream);
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

fn forward_chunks(rx: Receiver<PcmChunk>, output: mpsc::Sender<PcmChunk>) {
    while let Ok(chunk) = rx.recv() {
        if output.send(chunk).is_err() {
            break;
        }
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
