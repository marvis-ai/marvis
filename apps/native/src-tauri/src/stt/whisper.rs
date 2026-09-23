//! `whisper-cli` STT adapter. Binaries and models are always user-installed.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::Serialize;

use crate::audio::PcmChunk;
use crate::paths;

use super::{Finality, SpeakerChannel, SttProvider, TranscriptEvent};

const SAMPLE_RATE: u32 = 16_000;
const WINDOW_SAMPLES: usize = SAMPLE_RATE as usize * 3;
const SILENCE_RMS: f64 = 0.01;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_CONSECUTIVE_FAILURES: usize = 3;

/// The locally available whisper executable and models.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhisperStatus {
    pub binary: Option<String>,
    pub models: Vec<String>,
}

/// A chunked, final-only whisper-cli provider.
pub struct WhisperProvider {
    model: String,
    channel: SpeakerChannel,
    binary: Option<PathBuf>,
    input: Option<mpsc::SyncSender<PcmChunk>>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    worker: Option<JoinHandle<()>>,
}

impl WhisperProvider {
    pub fn new(model: impl Into<String>, channel: SpeakerChannel) -> Self {
        Self {
            model: model.into(),
            channel,
            binary: Self::discover(),
            input: None,
            stop: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
            worker: None,
        }
    }

    /// Find `whisper-cli` without starting it or creating installation files.
    pub fn discover() -> Option<PathBuf> {
        resolve_binary(
            std::env::var_os("PATH").as_deref(),
            &paths::whisper_bin_dir(),
        )
    }

    /// Report paths and model names only; this never reads credentials or runs a process.
    pub fn status() -> WhisperStatus {
        WhisperStatus {
            binary: Self::discover().map(|path| path.display().to_string()),
            models: list_models(&paths::whisper_models_dir()),
        }
    }

    fn model_path(&self) -> anyhow::Result<PathBuf> {
        if !is_model_file_name(&self.model) {
            anyhow::bail!("invalid whisper model name: {}", self.model);
        }
        Ok(paths::whisper_models_dir().join(&self.model))
    }
}

impl SttProvider for WhisperProvider {
    fn start(
        &mut self,
        callback: Box<dyn Fn(TranscriptEvent) + Send + Sync>,
        error_callback: Box<dyn Fn(String) + Send + Sync>,
    ) -> anyhow::Result<()> {
        if self.worker.is_some() {
            anyhow::bail!("whisper provider is already running");
        }
        let binary = self.binary.clone().ok_or_else(|| {
            anyhow::anyhow!("whisper-cli was not found; install it and try again")
        })?;
        let model = self.model_path()?;
        if !model.is_file() {
            anyhow::bail!("whisper model was not found: {}", model.display());
        }

        let (sender, receiver) = mpsc::sync_channel(4);
        self.input = Some(sender);
        self.stop.store(false, Ordering::Release);
        let stop = Arc::clone(&self.stop);
        let child = Arc::clone(&self.child);
        let channel = self.channel;
        self.worker = Some(thread::spawn(move || {
            run_chunks(
                receiver,
                callback,
                error_callback,
                binary,
                model,
                channel,
                stop,
                child,
            )
        }));
        Ok(())
    }

    fn enqueue(&self, chunk: PcmChunk) -> bool {
        self.input
            .as_ref()
            .is_some_and(|sender| sender.try_send(chunk).is_ok())
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut child) = self.child.lock() {
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
        }
        self.input.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Ok(mut child) = self.child.lock() {
            *child = None;
        }
    }
}

impl Drop for WhisperProvider {
    fn drop(&mut self) {
        self.stop();
    }
}

#[allow(clippy::too_many_arguments)]
fn run_chunks(
    receiver: mpsc::Receiver<PcmChunk>,
    callback: Box<dyn Fn(TranscriptEvent) + Send + Sync>,
    error_callback: Box<dyn Fn(String) + Send + Sync>,
    binary: PathBuf,
    model: PathBuf,
    channel: SpeakerChannel,
    stop: Arc<AtomicBool>,
    child_slot: Arc<Mutex<Option<Child>>>,
) {
    let mut buffer = Vec::with_capacity(WINDOW_SAMPLES);
    let mut consecutive_failures = 0;
    while !stop.load(Ordering::Acquire) {
        let chunk = match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => chunk,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        buffer.extend(chunk.samples);
        while buffer.len() >= WINDOW_SAMPLES {
            let window: Vec<i16> = buffer.drain(..WINDOW_SAMPLES).collect();
            if rms(&window) < SILENCE_RMS || stop.load(Ordering::Acquire) {
                continue;
            }
            match transcribe_window(
                &window,
                &binary,
                &model,
                channel,
                &stop,
                Arc::clone(&child_slot),
            ) {
                Ok(Some(text)) => {
                    consecutive_failures = 0;
                    callback(TranscriptEvent {
                        channel,
                        text,
                        finality: Finality::Final,
                    });
                }
                Ok(None) => consecutive_failures = 0,
                Err(_error) if stop.load(Ordering::Acquire) => return,
                Err(_) => {
                    consecutive_failures += 1;
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        error_callback(
                            "Whisper provider failed repeatedly and is no longer usable"
                                .to_string(),
                        );
                        return;
                    }
                }
            }
        }
    }
}

fn transcribe_window(
    samples: &[i16],
    binary: &Path,
    model: &Path,
    channel: SpeakerChannel,
    stop: &AtomicBool,
    child_slot: Arc<Mutex<Option<Child>>>,
) -> anyhow::Result<Option<String>> {
    let wav = WavGuard::create(samples, channel)?;
    let mut child = Command::new(binary)
        .args([
            "-m",
            &model.to_string_lossy(),
            "-f",
            &wav.path.to_string_lossy(),
            "--no-timestamps",
            "--output-txt",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("whisper-cli stdout was not captured"))?;
    *child_slot
        .lock()
        .map_err(|_| anyhow::anyhow!("whisper child lock poisoned"))? = Some(child);
    let reader_slot = Arc::clone(&child_slot);
    let reader = thread::spawn(move || {
        let result = read_bounded(stdout);
        if result.is_err() {
            if let Ok(mut slot) = reader_slot.lock() {
                if let Some(child) = slot.as_mut() {
                    let _ = child.kill();
                }
            }
        }
        result
    });

    let status = loop {
        if stop.load(Ordering::Acquire) {
            if let Ok(mut slot) = child_slot.lock() {
                if let Some(child) = slot.as_mut() {
                    let _ = child.kill();
                }
            }
        }
        let done = child_slot.lock().ok().and_then(|mut slot| {
            slot.as_mut()
                .and_then(|child| child.try_wait().ok().flatten())
        });
        if let Some(status) = done {
            break status;
        }
        thread::sleep(Duration::from_millis(20));
    };
    let output = reader
        .join()
        .map_err(|_| anyhow::anyhow!("whisper output reader panicked"));
    let _ = child_slot
        .lock()
        .map_err(|_| anyhow::anyhow!("whisper child lock poisoned"))?
        .take();
    let output = output?;
    if stop.load(Ordering::Acquire) {
        return Ok(None);
    }
    if !status.success() {
        return Err(anyhow::anyhow!("whisper-cli exited unsuccessfully"));
    }
    Ok(parse_output(&output?))
}

fn read_bounded(reader: impl Read) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .take((MAX_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut output)?;
    if output.len() > MAX_OUTPUT_BYTES {
        return Err(io::Error::other("whisper output exceeded limit"));
    }
    Ok(output)
}

fn parse_output(output: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(output).trim().to_string();
    if text.is_empty()
        || text.eq_ignore_ascii_case("[blank_audio]")
        || text.eq_ignore_ascii_case("(silence)")
    {
        None
    } else {
        Some(text)
    }
}

fn rms(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples
        .iter()
        .map(|sample| {
            let value = f64::from(*sample) / 32_768.0;
            value * value
        })
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt()
}

fn is_model_file_name(model: &str) -> bool {
    let path = Path::new(model);
    !model.is_empty()
        && !model.contains(['/', '\\'])
        && path.file_name().is_some_and(|name| name == model)
        && model != "."
        && model != ".."
}

fn resolve_binary(path: Option<&std::ffi::OsStr>, bundled_dir: &Path) -> Option<PathBuf> {
    if let Some(path) = path {
        for directory in std::env::split_paths(path) {
            let candidate = directory.join("whisper-cli");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let homebrew = Path::new("/opt/homebrew/bin/whisper-cli");
    if homebrew.is_file() {
        return Some(homebrew.to_path_buf());
    }
    let candidate = bundled_dir.join("whisper-cli");
    candidate.is_file().then_some(candidate)
}

fn list_models(directory: &Path) -> Vec<String> {
    let mut models = fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            (path.is_file()
                && path.file_name()?.to_str()?.starts_with("ggml-")
                && path.extension()? == "bin")
                .then(|| path.file_name()?.to_str().map(str::to_owned))
                .flatten()
        })
        .collect::<Vec<_>>();
    models.sort();
    models
}

struct WavGuard {
    path: PathBuf,
}
impl WavGuard {
    fn create(samples: &[i16], channel: SpeakerChannel) -> anyhow::Result<Self> {
        let directory = paths::audio_tmp_dir();
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!(
            "listen-{}-{}-{}.wav",
            channel_name(channel),
            std::process::id(),
            unique_suffix()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        if let Err(error) = write_wav(&mut file, samples) {
            let _ = fs::remove_file(&path);
            return Err(error.into());
        }
        Ok(Self { path })
    }
}

fn channel_name(channel: SpeakerChannel) -> &'static str {
    match channel {
        SpeakerChannel::Me => "me",
        SpeakerChannel::Them => "them",
    }
}

impl Drop for WavGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

fn write_wav(file: &mut File, samples: &[i16]) -> io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
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
    for sample in samples {
        file.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Mutex};

    #[test]
    fn reports_repeated_worker_failures_but_not_silence() {
        let (sender, receiver) = mpsc::sync_channel(4);
        let errors = Arc::new(Mutex::new(Vec::new()));
        let errors_for_callback = Arc::clone(&errors);
        let samples = vec![1000; WINDOW_SAMPLES];
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            sender
                .send(PcmChunk {
                    samples: samples.clone(),
                    sample_rate: SAMPLE_RATE,
                    channels: 1,
                })
                .unwrap();
        }
        drop(sender);
        run_chunks(
            receiver,
            Box::new(|_| {}),
            Box::new(move |error| errors_for_callback.lock().unwrap().push(error)),
            PathBuf::from("/missing/whisper-cli"),
            PathBuf::from("/missing/model.bin"),
            SpeakerChannel::Me,
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
        );
        assert_eq!(
            errors.lock().unwrap().as_slice(),
            &["Whisper provider failed repeatedly and is no longer usable"]
        );
    }

    #[test]
    fn cleans_whisper_artifacts() {
        assert_eq!(parse_output(b" hello\n"), Some("hello".into()));
        assert_eq!(parse_output(b""), None);
        assert_eq!(parse_output(b"[BLANK_AUDIO]"), None);
        assert_eq!(parse_output(b"(silence)"), None);
    }

    #[test]
    fn resolves_binary_in_documented_order() {
        let root = tempfile_dir();
        let path_dir = root.join("path");
        let bundled = root.join("bundled");
        fs::create_dir_all(&path_dir).unwrap();
        fs::create_dir_all(&bundled).unwrap();
        fs::write(path_dir.join("whisper-cli"), b"").unwrap();
        fs::write(bundled.join("whisper-cli"), b"").unwrap();
        let path = std::ffi::OsString::from(path_dir.clone());
        assert_eq!(
            resolve_binary(Some(&path), &bundled),
            Some(path_dir.join("whisper-cli"))
        );
    }

    #[test]
    fn rejects_stdout_over_limit() {
        let output = std::io::Cursor::new(vec![b'x'; MAX_OUTPUT_BYTES + 1]);
        assert!(read_bounded(output).is_err());
    }

    #[test]
    fn rejects_model_paths_before_joining_models_directory() {
        for model in [
            "../outside.bin",
            "nested/model.bin",
            r"nested\\model.bin",
            ".",
            "..",
        ] {
            assert!(
                !is_model_file_name(model),
                "accepted invalid model {model:?}"
            );
        }
        assert!(is_model_file_name("ggml-base.bin"));
    }

    #[test]
    fn lists_only_regular_ggml_bin_models() {
        let root = tempfile_dir();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("ggml-base.bin"), b"").unwrap();
        fs::write(root.join("ggml-base.txt"), b"").unwrap();
        fs::write(root.join("model.bin"), b"").unwrap();
        fs::create_dir(root.join("ggml-dir.bin")).unwrap();
        assert_eq!(list_models(&root), vec!["ggml-base.bin"]);
    }

    #[cfg(unix)]
    #[test]
    fn wav_guard_sets_private_mode_and_removes_file_on_drop() {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let root = tempfile_dir();
        let path = root.join("private.wav");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = options.open(&path).unwrap();
        write_wav(&mut file, &[1, -1]).unwrap();
        drop(file);

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let guard = WavGuard { path: path.clone() };
        drop(guard);
        assert!(!path.exists());
    }

    fn tempfile_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("marvis-whisper-test-{}", unique_suffix()));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
