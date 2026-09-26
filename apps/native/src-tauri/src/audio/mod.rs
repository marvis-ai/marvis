//! Platform-neutral audio source interfaces and PCM normalization.

use std::sync::mpsc::Sender;

mod mic;
mod system;
pub use mic::MicSource;
pub use system::SystemAudioSource;

const TARGET_SAMPLE_RATE: u32 = 16_000;

/// A normalized mono PCM chunk emitted by an [`AudioSource`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmChunk {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// A source of normalized audio chunks.
pub trait AudioSource: Send {
    fn start(&mut self, output: Sender<PcmChunk>) -> anyhow::Result<()>;
    fn stop(&mut self);
    fn is_running(&self) -> bool;
    /// Fatal backend errors reported since the last poll (the stream is
    /// dead once one lands). Transient glitches are logged, never queued.
    /// `None` for sources without a status channel.
    fn try_recv_status(&self) -> Option<String> {
        None
    }
}

fn interleaved_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    samples
        .chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32)
        .collect()
}

fn resample_to_target(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() || sample_rate == TARGET_SAMPLE_RATE {
        return samples.to_vec();
    }

    let output_len = ((samples.len() as u64 * u64::from(TARGET_SAMPLE_RATE)
        + u64::from(sample_rate) / 2)
        / u64::from(sample_rate)) as usize;
    (0..output_len)
        .map(|index| {
            let position = index as f64 * f64::from(sample_rate) / f64::from(TARGET_SAMPLE_RATE);
            let left = position.floor() as usize;
            let right = (left + 1).min(samples.len() - 1);
            let fraction = (position - left as f64) as f32;
            samples[left] * (1.0 - fraction) + samples[right] * fraction
        })
        .collect()
}

fn float_to_i16(sample: f32) -> i16 {
    let sample = sample.clamp(-1.0, 1.0);
    if sample < 0.0 {
        (sample * 32_768.0).round() as i16
    } else {
        (sample * 32_767.0).round() as i16
    }
}

fn normalize_pcm(samples: &[f32], sample_rate: u32, channels: u16) -> PcmChunk {
    let mono = interleaved_to_mono(samples, channels);
    let resampled = resample_to_target(&mono, sample_rate);
    PcmChunk {
        samples: resampled.into_iter().map(float_to_i16).collect(),
        sample_rate: TARGET_SAMPLE_RATE,
        channels: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_interleaved_stereo_to_mono() {
        assert_eq!(
            interleaved_to_mono(&[1.0, 0.0, 0.25, 0.75], 2),
            vec![0.5, 0.5]
        );
    }

    #[test]
    fn clamps_float_samples_to_i16() {
        let chunk = normalize_pcm(&[-2.0, -1.0, 0.0, 1.0, 2.0], TARGET_SAMPLE_RATE, 1);
        assert_eq!(chunk.samples, vec![-32_768, -32_768, 0, 32_767, 32_767]);
    }

    #[test]
    fn resamples_known_waveform_to_sixteen_kilohertz() {
        let input: Vec<f32> = (0..48_000).map(|value| value as f32 / 48_000.0).collect();
        let chunk = normalize_pcm(&input, 48_000, 1);
        assert_eq!(chunk.sample_rate, 16_000);
        assert_eq!(chunk.channels, 1);
        assert_eq!(chunk.samples.len(), 16_000);
        assert_eq!(chunk.samples[0], 0);
        assert!((i32::from(chunk.samples[8_000]) - 16_384).abs() <= 1);
    }
}
