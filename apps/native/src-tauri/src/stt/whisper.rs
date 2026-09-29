//! `whisper-cli` STT adapter. Models are user-installed; the executable may be bundled.

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
use crate::voice_models::entry_for_value;

use super::{Finality, SpeakerChannel, SttProvider, TranscriptEvent};

const SAMPLE_RATE: u32 = 16_000;
const WINDOW_SAMPLES: usize = SAMPLE_RATE as usize * 3;
const SILENCE_RMS: f64 = 0.01;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_CONSECUTIVE_FAILURES: usize = 3;

/// Where the selected Whisper executable was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WhisperBinarySource {
    Bundled,
    Path,
    Homebrew,
    User,
}

/// Source-safe binary availability for local Settings status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhisperBinaryStatus {
    pub available: bool,
    pub source: Option<WhisperBinarySource>,
}

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
    diarize: bool,
    input: Option<mpsc::Sender<PcmChunk>>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    worker: Option<JoinHandle<()>>,
}

impl WhisperProvider {
    pub fn new(
        model: impl Into<String>,
        channel: SpeakerChannel,
        bundled: Option<&Path>,
        diarize: bool,
    ) -> Self {
        Self {
            model: model.into(),
            channel,
            binary: Self::discover_with_bundled(bundled).map(|(path, _)| path),
            diarize,
            input: None,
            stop: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
            worker: None,
        }
    }

    /// Find `whisper-cli` through the non-bundled fallback chain.
    pub fn discover() -> Option<PathBuf> {
        resolve_fallback(
            std::env::var_os("PATH").as_deref(),
            &paths::whisper_bin_dir(),
        )
        .map(|(path, _)| path)
    }

    /// Resolve a bundled candidate before PATH, Homebrew, and user-local paths.
    /// This only inspects filesystem metadata and never starts the executable.
    pub fn discover_with_bundled(bundled: Option<&Path>) -> Option<(PathBuf, WhisperBinarySource)> {
        bundled
            .filter(|path| is_bundled_candidate(path))
            .map(|path| (path.to_path_buf(), WhisperBinarySource::Bundled))
            .or_else(|| {
                resolve_fallback(
                    std::env::var_os("PATH").as_deref(),
                    &paths::whisper_bin_dir(),
                )
            })
    }

    pub fn binary_status(bundled: Option<&Path>) -> WhisperBinaryStatus {
        let resolved = Self::discover_with_bundled(bundled);
        WhisperBinaryStatus {
            available: resolved.is_some(),
            source: resolved.map(|(_, source)| source),
        }
    }

    /// Report paths and model names only; this never reads credentials or runs a process.
    pub fn status() -> WhisperStatus {
        Self::status_with_bundled(None)
    }

    pub fn status_with_bundled(bundled: Option<&Path>) -> WhisperStatus {
        WhisperStatus {
            binary: Self::discover_with_bundled(bundled)
                .map(|(path, _)| path.display().to_string()),
            models: list_models(&paths::whisper_models_dir()),
        }
    }

    pub fn model_filename(model: &str) -> anyhow::Result<&'static str> {
        entry_for_value(model)
            .map(|entry| entry.filename)
            .ok_or_else(|| anyhow::anyhow!("invalid whisper model name: {model}"))
    }

    fn model_path(&self) -> anyhow::Result<PathBuf> {
        let filename = Self::model_filename(&self.model)?;
        Ok(paths::whisper_models_dir().join(filename))
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

        // Unbounded on purpose: a whisper-cli window takes seconds, so a
        // bounded queue overflows during speech and every dropped chunk is
        // a permanent hole in the transcript — a backlog is only lag.
        let (sender, receiver) = mpsc::channel();
        self.input = Some(sender);
        self.stop.store(false, Ordering::Release);
        let stop = Arc::clone(&self.stop);
        let child = Arc::clone(&self.child);
        let channel = self.channel;
        let diarize = self.diarize;
        self.worker = Some(thread::spawn(move || {
            run_chunks(
                receiver,
                callback,
                error_callback,
                binary,
                model,
                channel,
                diarize,
                stop,
                child,
            )
        }));
        Ok(())
    }

    fn enqueue(&self, chunk: PcmChunk) -> bool {
        self.input
            .as_ref()
            .is_some_and(|sender| sender.send(chunk).is_ok())
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
    diarize: bool,
    stop: Arc<AtomicBool>,
    child_slot: Arc<Mutex<Option<Child>>>,
) {
    // Diarization is progressive enhancement: a missing/unloadable
    // embedding model leaves `speaker_idx` unset rather than failing STT.
    let mut tracker = diarize
        .then(|| crate::stt::speaker::tracker_if_installed(channel))
        .flatten();
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
                    let f32_samples: Vec<f32> =
                        window.iter().map(|s| f32::from(*s) / 32_768.0).collect();
                    callback(TranscriptEvent {
                        channel,
                        text,
                        finality: Finality::Final,
                        speaker_idx: tracker.as_mut().and_then(|t| t.assign(&f32_samples)),
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
        .args(whisper_cli_args(model, &wav.path))
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

/// Per-window whisper-cli arguments. `-l auto` detects the spoken language
/// and transcribes in it; without `-l` the CLI decodes every window as
/// English, so non-English speech comes out as English-looking text.
fn whisper_cli_args(model: &Path, wav: &Path) -> Vec<String> {
    vec![
        "-m".into(),
        model.to_string_lossy().into_owned(),
        "-f".into(),
        wav.to_string_lossy().into_owned(),
        "-l".into(),
        "auto".into(),
        "--no-timestamps".into(),
        "--output-txt".into(),
    ]
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

#[cfg(test)]
fn is_model_file_name(model: &str) -> bool {
    let path = Path::new(model);
    !model.is_empty()
        && !model.contains(['/', '\\'])
        && path.file_name().is_some_and(|name| name == model)
        && model != "."
        && model != ".."
}

/// The on-disk executable name — `whisper-cli.exe` on Windows (PATH dirs
/// and the user dir both carry the real file name).
#[cfg(target_os = "windows")]
const WHISPER_EXE_NAME: &str = "whisper-cli.exe";
#[cfg(not(target_os = "windows"))]
const WHISPER_EXE_NAME: &str = "whisper-cli";

fn resolve_fallback(
    path: Option<&std::ffi::OsStr>,
    user_dir: &Path,
) -> Option<(PathBuf, WhisperBinarySource)> {
    if let Some(path) = path {
        for directory in std::env::split_paths(path) {
            let candidate = directory.join(WHISPER_EXE_NAME);
            if is_usable_candidate(&candidate) {
                return Some((candidate, WhisperBinarySource::Path));
            }
        }
    }
    // Package-manager installs only exist on Unix (Homebrew/local bin);
    // Windows/Linux users land on PATH or the user dir.
    #[cfg(unix)]
    for homebrew in [
        Path::new("/opt/homebrew/bin/whisper-cli"),
        Path::new("/usr/local/bin/whisper-cli"),
    ] {
        if is_usable_candidate(homebrew) {
            return Some((homebrew.to_path_buf(), WhisperBinarySource::Homebrew));
        }
    }
    let candidate = user_dir.join(WHISPER_EXE_NAME);
    is_usable_candidate(&candidate).then_some((candidate, WhisperBinarySource::User))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// Every candidate must be executable and, on macOS, contain a Mach-O slice
/// that can run in this process. Discovery never launches the candidate.
fn is_usable_candidate(path: &Path) -> bool {
    is_executable_file(path) && architecture_matches(path)
}

fn is_bundled_candidate(path: &Path) -> bool {
    is_usable_candidate(path)
}

#[cfg(target_os = "macos")]
fn architecture_matches(path: &Path) -> bool {
    let expected = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        return false;
    };
    std::process::Command::new("file")
        .arg("-b")
        .arg(path)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|description| description.contains(expected))
}

#[cfg(not(target_os = "macos"))]
fn architecture_matches(_path: &Path) -> bool {
    true
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
    fn whisper_status_serializes_documented_wire_field_names() {
        let status = WhisperStatus {
            binary: Some("/tmp/whisper-cli".into()),
            models: vec!["base.en.bin".into()],
        };
        let payload = serde_json::to_value(status).unwrap();
        assert_eq!(
            payload
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["binary", "models"]
        );
        assert_eq!(payload["binary"], "/tmp/whisper-cli");
        assert_eq!(payload["models"][0], "base.en.bin");
    }

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
            false,
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
        );
        assert_eq!(
            errors.lock().unwrap().as_slice(),
            &["Whisper provider failed repeatedly and is no longer usable"]
        );
    }

    /// Transcripts must stay in the spoken language: `-l auto` runs
    /// whisper.cpp's language detection per window, and no translate flag
    /// may ever reach the CLI.
    #[test]
    fn cli_args_detect_language_and_never_translate() {
        let args = whisper_cli_args(Path::new("/model.bin"), Path::new("/in.wav"));
        let language = args
            .windows(2)
            .find(|pair| pair[0] == "-l" || pair[0] == "--language")
            .map(|pair| pair[1].as_str());
        assert_eq!(language, Some("auto"));
        assert!(!args.iter().any(|arg| arg == "-tr" || arg == "--translate"));
    }

    #[test]
    fn cleans_whisper_artifacts() {
        assert_eq!(parse_output(b" hello\n"), Some("hello".into()));
        assert_eq!(parse_output(b""), None);
        assert_eq!(parse_output(b"[BLANK_AUDIO]"), None);
        assert_eq!(parse_output(b"(silence)"), None);
    }

    #[test]
    fn bundled_binary_wins_and_reports_source() {
        let root = tempfile_dir();
        let path_dir = root.join("path");
        let bundled = root.join("bundled").join("whisper-cli");
        fs::create_dir_all(&path_dir).unwrap();
        fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        let path_binary = path_dir.join(WHISPER_EXE_NAME);
        make_test_executable(&path_binary);
        make_test_executable(&bundled);
        let path = std::ffi::OsString::from(path_dir);
        assert_eq!(
            WhisperProvider::discover_with_bundled(Some(&bundled)),
            Some((bundled.clone(), WhisperBinarySource::Bundled))
        );
        assert_eq!(
            WhisperProvider::binary_status(Some(&bundled)),
            WhisperBinaryStatus {
                available: true,
                source: Some(WhisperBinarySource::Bundled),
            }
        );
        assert_eq!(
            resolve_fallback(Some(&path), &root.join("user")),
            Some((path_binary, WhisperBinarySource::Path))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rejects_bundled_executable_with_wrong_macho_architecture() {
        let root = tempfile_dir();
        let candidate = root.join("whisper-cli");
        make_executable(&candidate);
        assert!(!architecture_matches(&candidate));
        assert!(!is_bundled_candidate(&candidate));
    }

    #[test]
    fn fallback_order_and_invalid_candidates_are_source_safe() {
        let root = tempfile_dir();
        let path_dir = root.join("path");
        let user_dir = root.join("user");
        fs::create_dir_all(&path_dir).unwrap();
        fs::create_dir_all(&user_dir).unwrap();
        let path_binary = path_dir.join("whisper-cli");
        fs::write(&path_binary, b"").unwrap();
        assert_eq!(
            resolve_fallback(Some(std::ffi::OsStr::new("/missing")), &user_dir),
            None
        );
        make_test_executable(&user_dir.join(WHISPER_EXE_NAME));
        assert_eq!(
            resolve_fallback(None, &user_dir),
            Some((user_dir.join(WHISPER_EXE_NAME), WhisperBinarySource::User))
        );
        assert!(!is_executable_file(&path_binary));
        assert!(!is_usable_candidate(&path_binary));
        #[cfg(target_os = "macos")]
        {
            make_executable(&path_binary);
            assert_eq!(
                resolve_fallback(Some(path_dir.as_os_str()), &user_dir),
                Some((user_dir.join(WHISPER_EXE_NAME), WhisperBinarySource::User))
            );
        }
        fs::create_dir(path_dir.join("not-a-file")).unwrap();
        assert!(!is_executable_file(&path_dir.join("not-a-file")));
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, b"").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(target_os = "macos")]
    fn make_test_executable(path: &Path) {
        fs::copy(std::env::current_exe().unwrap(), path).unwrap();
    }

    #[cfg(not(target_os = "macos"))]
    fn make_test_executable(path: &Path) {
        make_executable(path);
    }

    #[cfg(not(unix))]
    fn make_executable(path: &Path) {
        fs::write(path, b"").unwrap();
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
