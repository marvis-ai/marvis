//! Session WAV recorder — mixes per-channel PCM into one 16 kHz mono file.
//!
//! Both capture channels share one timeline. A channel's cursor starts at
//! the current end of the file on its first push (its stream "begins"
//! there — sources deliver chunks at real-time pace, so cumulative samples
//! track wall time) and advances by chunk length after that; when cursors
//! overlap, simultaneous speech sums (clamped to i16). The RIFF header is
//! rewritten after every push, so the file is a valid WAV up to the last
//! write even if the process dies mid-session.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// `SpeakerChannel::Me`/`Them` map to indices 0/1 at the call site.
pub const CHANNELS: usize = 2;
const HEADER_LEN: u64 = 44;
const SAMPLE_RATE: u32 = 16_000;

pub struct SessionRecorder {
    file: File,
    /// Mixed extent in samples — the file's logical audio length.
    data_samples: u64,
    /// Next sample offset per channel — `None` until that channel's
    /// first chunk arrives (a channel that joins later must not land at
    /// the file's beginning).
    cursors: [Option<u64>; CHANNELS],
    starts: [Option<u64>; CHANNELS],
}

impl SessionRecorder {
    /// Fresh file at `path` — refuses to overwrite (`create_new`).
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        write_header(&mut file, 0)?;
        Ok(Self {
            file,
            data_samples: 0,
            cursors: [None; CHANNELS],
            starts: [None; CHANNELS],
        })
    }

    pub fn channel_start_ms(&self, channel: usize) -> Option<u64> {
        self.starts[channel].map(|samples| samples * 1000 / u64::from(SAMPLE_RATE))
    }

    /// Mix `samples` into the shared timeline at `channel`'s cursor.
    pub fn push(&mut self, channel: usize, samples: &[i16]) -> std::io::Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        let start = match self.cursors[channel] {
            Some(cursor) => cursor,
            None => {
                self.cursors[channel] = Some(self.data_samples);
                self.starts[channel] = Some(self.data_samples);
                self.data_samples
            }
        };
        self.cursors[channel] = Some(start + samples.len() as u64);
        // The region of `samples` landing under already-written audio.
        let overlap = self
            .data_samples
            .saturating_sub(start)
            .min(samples.len() as u64) as usize;
        if start > self.data_samples {
            self.file
                .seek(SeekFrom::Start(HEADER_LEN + self.data_samples * 2))?;
            self.file
                .write_all(&vec![0u8; ((start - self.data_samples) * 2) as usize])?;
            self.data_samples = start;
        }
        if overlap > 0 {
            self.file.seek(SeekFrom::Start(HEADER_LEN + start * 2))?;
            let mut existing = vec![0u8; overlap * 2];
            self.file.read_exact(&mut existing)?;
            self.file.seek(SeekFrom::Start(HEADER_LEN + start * 2))?;
            let mut mixed = Vec::with_capacity(overlap * 2);
            for (i, sample) in samples[..overlap].iter().enumerate() {
                let current = i16::from_le_bytes([existing[i * 2], existing[i * 2 + 1]]);
                let sum = (i32::from(current) + i32::from(*sample))
                    .clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                mixed.extend_from_slice(&(sum as i16).to_le_bytes());
            }
            self.file.write_all(&mixed)?;
        }
        if overlap < samples.len() {
            self.file
                .seek(SeekFrom::Start(HEADER_LEN + (start + overlap as u64) * 2))?;
            let mut tail = Vec::with_capacity((samples.len() - overlap) * 2);
            for sample in &samples[overlap..] {
                tail.extend_from_slice(&sample.to_le_bytes());
            }
            self.file.write_all(&tail)?;
        }
        self.data_samples = self.data_samples.max(start + samples.len() as u64);
        write_header(&mut self.file, self.data_samples)
    }
}

fn write_header(file: &mut File, data_samples: u64) -> std::io::Result<()> {
    let data_len = (data_samples * 2).min(u64::from(u32::MAX)) as u32;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_len).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&SAMPLE_RATE.to_le_bytes())?;
    file.write_all(&(SAMPLE_RATE * 2).to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_len.to_le_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn tmp_path() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "marvis-recorder-test-{}-{}.wav",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn samples(path: &Path) -> Vec<i16> {
        let bytes = std::fs::read(path).unwrap();
        bytes[HEADER_LEN as usize..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    /// Channel 1 joins mid-stream — its audio must append at the current
    /// end (its own timeline start), not overwrite channel 0's head. A
    /// later channel-0 chunk then mixes over channel 1's tail.
    #[test]
    fn late_channel_appends_then_overlaps_mix() {
        let path = tmp_path();
        {
            let mut recorder = SessionRecorder::create(&path).unwrap();
            recorder.push(0, &[1000; 4]).unwrap(); // occupies 0..4
            recorder.push(1, &[500; 4]).unwrap(); // joins at 4..8
            recorder.push(0, &[100; 4]).unwrap(); // resumes at 4 → mixes
            recorder.push(1, &[50; 2]).unwrap(); // continues at 8..10
        }
        assert_eq!(
            samples(&path),
            vec![1000, 1000, 1000, 1000, 600, 600, 600, 600, 50, 50]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn channel_origin_tracks_pcm_not_elapsed_wall_time() {
        let path = tmp_path();
        let mut recorder = SessionRecorder::create(&path).unwrap();
        assert_eq!(recorder.channel_start_ms(1), None);
        recorder.push(0, &[1; 16_000]).unwrap();
        recorder.push(1, &[1; 8_000]).unwrap();
        recorder.push(0, &[1; 16_000]).unwrap();
        assert_eq!(recorder.channel_start_ms(0), Some(0));
        assert_eq!(recorder.channel_start_ms(1), Some(1000));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn mixed_samples_clamp_to_i16() {
        let path = tmp_path();
        {
            let mut recorder = SessionRecorder::create(&path).unwrap();
            recorder.push(0, &[30_000; 4]).unwrap();
            recorder.push(1, &[10_000; 4]).unwrap(); // occupies 4..8
            recorder.push(0, &[25_000; 4]).unwrap(); // mixes over 4..8
        }
        assert_eq!(
            samples(&path),
            vec![30_000, 30_000, 30_000, 30_000, 32_767, 32_767, 32_767, 32_767]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_is_a_valid_wav_after_every_push() {
        let path = tmp_path();
        let mut recorder = SessionRecorder::create(&path).unwrap();
        recorder.push(0, &[1; 3]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 6);
        let _ = std::fs::remove_file(&path);
    }
}
