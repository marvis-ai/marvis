//! WASAPI loopback: capture whatever the default render device is
//! playing. The worker thread owns every COM object (`initialize_client`
//! on a render endpoint + `Direction::Capture` + shared mode sets
//! `AUDCLNT_STREAMFLAGS_LOOPBACK` for us). Polling mode — a ~5 ms idle
//! sleep beats wiring an event handle for transcription-grade audio.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Result};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode};

use super::{
    interleaved_pcm_to_f32, normalize_pcm, status_channel, warn_unsupported, AudioSource, PcmChunk,
};

/// Requested WASAPI buffer capacity (20 ms in 100 ns units).
const BUFFER_DURATION_HNS: i64 = 200_000;
/// Idle sleep between polls when no packet is queued.
const POLL_IDLE: Duration = Duration::from_millis(5);

/// System audio via the default render endpoint's loopback tap.
/// All WASAPI calls happen on the worker thread (COM apartment rules);
/// `start` blocks only on a one-shot init handshake so setup failures
/// still surface as `Err`, same as the macOS `SCStream` path.
pub struct SystemAudioSource {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
    status_tx: mpsc::Sender<String>,
    status_rx: Receiver<String>,
}

impl SystemAudioSource {
    pub fn new() -> Result<Self> {
        let (status_tx, status_rx) = status_channel();
        Ok(Self {
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            running: Arc::new(AtomicBool::new(false)),
            status_tx,
            status_rx,
        })
    }
}

impl AudioSource for SystemAudioSource {
    fn start(&mut self, output: Sender<PcmChunk>) -> Result<()> {
        self.stop();
        let stop = Arc::clone(&self.stop);
        stop.store(false, Ordering::Relaxed);
        let running = Arc::clone(&self.running);
        let status_tx = self.status_tx.clone();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let worker = thread::spawn(move || {
            loopback_worker(output, stop, running, status_tx, ready_tx);
        });
        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.worker = Some(worker);
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = worker.join();
                Err(anyhow!("system audio init failed: {e}"))
            }
            Err(_) => {
                let _ = worker.join();
                Err(anyhow!("system audio worker died during init"))
            }
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.running.store(false, Ordering::Relaxed);
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    fn try_recv_status(&self) -> Option<String> {
        self.status_rx.try_recv().ok()
    }
}

impl Drop for SystemAudioSource {
    fn drop(&mut self) {
        self.stop();
    }
}

fn loopback_worker(
    output: Sender<PcmChunk>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    status_tx: mpsc::Sender<String>,
    ready_tx: mpsc::Sender<Result<(), String>>,
) {
    // COM apartment init is per-thread; a second MTA init is a no-op.
    let _ = wasapi::initialize_mta();
    let setup = (|| -> Result<_, wasapi::WasapiError> {
        let enumerator = DeviceEnumerator::new()?;
        let device = enumerator.get_default_device(&Direction::Render)?;
        let mut client = device.get_iaudioclient()?;
        let format = client.get_mixformat()?;
        client.initialize_client(
            &format,
            &Direction::Capture,
            &StreamMode::PollingShared {
                autoconvert: false,
                buffer_duration_hns: BUFFER_DURATION_HNS,
            },
        )?;
        let capture = client.get_audiocaptureclient()?;
        client.start_stream()?;
        Ok((client, capture, format))
    })();
    let (client, capture, format) = match setup {
        Ok(parts) => {
            let _ = ready_tx.send(Ok(()));
            parts
        }
        Err(e) => {
            let _ = ready_tx.send(Err(e.to_string()));
            return;
        }
    };
    running.store(true, Ordering::Release);

    let block_align = format.get_blockalign() as usize;
    let sample_rate = format.get_samplespersec();
    let channels = format.get_nchannels();
    let bits = format.get_bitspersample();
    let is_float = matches!(format.get_subformat(), Ok(SampleType::Float));
    if !is_float && !matches!(bits, 16 | 32) {
        let _ = status_tx.send(format!(
            "system audio: unsupported sample width {bits} bits"
        ));
    }
    let warned = AtomicBool::new(false);
    let mut buf: Vec<u8> = Vec::new();

    while !stop.load(Ordering::Relaxed) {
        let frames = match capture.get_next_packet_size() {
            Ok(Some(0)) | Ok(None) => {
                thread::sleep(POLL_IDLE);
                continue;
            }
            Ok(Some(frames)) => frames,
            Err(e) => {
                let _ = status_tx.send(format!("system audio read failed: {e}"));
                break;
            }
        };
        buf.resize(frames as usize * block_align, 0);
        match capture.read_from_device(&mut buf) {
            Ok((0, _)) => continue,
            Ok((frames, info)) => {
                let bytes = &buf[..frames as usize * block_align];
                // A SILENT packet's bytes are undefined — emit zeros so
                // transcript timing stays continuous through quiet audio.
                let samples = if info.flags.silent {
                    Some(vec![0.0; frames as usize * usize::from(channels)])
                } else {
                    interleaved_pcm_to_f32(bytes, bits, is_float)
                };
                match samples {
                    Some(samples) => {
                        if output
                            .send(normalize_pcm(&samples, sample_rate, channels))
                            .is_err()
                        {
                            break;
                        }
                    }
                    None => warn_unsupported(&warned, "packet layout rejected"),
                }
            }
            Err(e) => {
                let _ = status_tx.send(format!("system audio read failed: {e}"));
                break;
            }
        }
    }
    if let Err(e) = client.stop_stream() {
        log::warn!("system audio stop_stream failed: {e}");
    }
    running.store(false, Ordering::Release);
}
