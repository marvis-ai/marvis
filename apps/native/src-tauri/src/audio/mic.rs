use super::{normalize_pcm, AudioSource, PcmChunk};
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, ErrorKind, Sample, SampleFormat, Stream, StreamConfig};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// Deep enough to ride out short CPU spikes (model loads, decode bursts)
/// without the realtime callback hitting "input queue is full".
const INPUT_QUEUE_CAPACITY: usize = 64;
/// Requested input buffer: ~43ms at the common 48kHz vs ~10ms at the
/// device default — slack so a stalled scheduler tick doesn't glitch the
/// stream (xrun). Costs ~30ms of extra capture latency, irrelevant next
/// to STT turnaround. Devices that reject it fall back to the default.
const MIC_BUFFER_FRAMES: u32 = 2048;

type RawChunk = (Vec<f32>, u32, u16);

/// Microphone input backed by the platform's default cpal input device.
pub struct MicSource {
    stream: Option<Stream>,
    input_tx: Option<SyncSender<RawChunk>>,
    worker: Option<JoinHandle<()>>,
    status_rx: Receiver<String>,
    status_tx: mpsc::Sender<String>,
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for MicSource {
    fn default() -> Self {
        Self::new()
    }
}

impl MicSource {
    pub fn new() -> Self {
        let (status_tx, status_rx) = mpsc::channel();
        Self {
            stream: None,
            input_tx: None,
            worker: None,
            status_rx,
            status_tx,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn start_with_format(
        &mut self,
        output: mpsc::Sender<PcmChunk>,
        device: cpal::Device,
        config: StreamConfig,
        sample_format: SampleFormat,
    ) -> Result<()> {
        let sample_rate = config.sample_rate;
        let channels = config.channels;
        let (input_tx, input_rx) = mpsc::sync_channel(INPUT_QUEUE_CAPACITY);
        let worker = thread::spawn(move || forward_chunks(input_rx, output));
        let status_tx = self.status_tx.clone();

        // A device may reject a requested buffer size — the retry drops to
        // the default rather than failing microphone startup outright.
        let build = |config: StreamConfig| -> Result<Stream> {
            let callback_tx = input_tx.clone();
            let status_tx = status_tx.clone();
            let error_callback = move |error| report_stream_error(&error, &status_tx);
            match sample_format {
                SampleFormat::F32 => device.build_input_stream(
                    config,
                    move |data: &[f32], _| {
                        enqueue(data.to_vec(), sample_rate, channels, &callback_tx)
                    },
                    error_callback,
                    None,
                ),
                SampleFormat::I16 => device.build_input_stream(
                    config,
                    move |data: &[i16], _| {
                        let converted: Vec<f32> = data
                            .iter()
                            .map(|&sample| f32::from_sample(sample))
                            .collect();
                        enqueue(converted, sample_rate, channels, &callback_tx);
                    },
                    error_callback,
                    None,
                ),
                SampleFormat::U16 => device.build_input_stream(
                    config,
                    move |data: &[u16], _| {
                        let converted: Vec<f32> = data
                            .iter()
                            .map(|&sample| f32::from_sample(sample))
                            .collect();
                        enqueue(converted, sample_rate, channels, &callback_tx);
                    },
                    error_callback,
                    None,
                ),
                format => return Err(anyhow!("unsupported microphone sample format: {format:?}")),
            }
            .map_err(|error| anyhow!("failed to build microphone input stream: {error}"))
        };

        let stream = match build(config) {
            Ok(stream) => stream,
            Err(first_error) if config.buffer_size != BufferSize::Default => {
                log::warn!(
                    "microphone rejected the requested buffer size; retrying with the device default ({first_error})"
                );
                build(StreamConfig {
                    buffer_size: BufferSize::Default,
                    ..config
                })?
            }
            Err(error) => return Err(error),
        };

        stream
            .play()
            .map_err(|error| anyhow!("failed to start microphone input stream: {error}"))?;
        self.stream = Some(stream);
        self.input_tx = Some(input_tx);
        self.worker = Some(worker);
        self.running
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }
}

impl AudioSource for MicSource {
    fn start(&mut self, output: mpsc::Sender<PcmChunk>) -> Result<()> {
        self.stop();
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow!("no default microphone input device"))?;
        let supported = device
            .default_input_config()
            .map_err(|error| anyhow!("failed to get microphone input config: {error}"))?;
        let sample_format = supported.sample_format();
        let mut config: StreamConfig = supported.into();
        config.buffer_size = BufferSize::Fixed(MIC_BUFFER_FRAMES);
        self.start_with_format(output, device, config, sample_format)
    }

    fn stop(&mut self) {
        self.running
            .store(false, std::sync::atomic::Ordering::Release);
        self.input_tx.take();
        self.stream.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::Acquire)
    }

    fn try_recv_status(&self) -> Option<String> {
        self.status_rx.try_recv().ok()
    }
}

impl Drop for MicSource {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Routes backend stream errors: an `Xrun` is a transient glitch
/// notification (the stream keeps running), so it logs at warn and never
/// touches the status channel — only fatal kinds reach `try_recv_status`.
fn report_stream_error(error: &cpal::Error, status_tx: &mpsc::Sender<String>) {
    if error.kind() == ErrorKind::Xrun {
        log::warn!("microphone audio glitch: buffer underrun/overrun");
        return;
    }
    let message = format!("microphone stream error: {error}");
    let _ = status_tx.send(message.clone());
    log::error!("{message}");
}

fn enqueue(samples: Vec<f32>, sample_rate: u32, channels: u16, sender: &SyncSender<RawChunk>) {
    match sender.try_send((samples, sample_rate, channels)) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            log::warn!("dropping microphone audio chunk: input queue is full")
        }
        Err(TrySendError::Disconnected(_)) => {}
    }
}

fn forward_chunks(input: Receiver<RawChunk>, output: mpsc::Sender<PcmChunk>) {
    while let Ok((samples, sample_rate, channels)) = input.recv() {
        if output
            .send(normalize_pcm(&samples, sample_rate, channels))
            .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status channel exists so pumps can fail loud on a dead stream —
    /// a transient xrun (the stream is still alive) must not occupy it,
    /// or every sleep/wake glitch would tear the session down.
    #[test]
    fn transient_xrun_stays_off_the_status_channel() {
        let (tx, rx) = mpsc::channel::<String>();
        report_stream_error(&cpal::Error::new(ErrorKind::Xrun), &tx);
        assert!(rx.try_recv().is_err());
    }

    /// Fatal kinds (device lost, stream invalidated) are what consumers
    /// poll for — dropping them would leave a dead stream looking live.
    #[test]
    fn fatal_stream_errors_reach_the_status_channel() {
        let (tx, rx) = mpsc::channel::<String>();
        report_stream_error(&cpal::Error::new(ErrorKind::DeviceNotAvailable), &tx);
        report_stream_error(&cpal::Error::new(ErrorKind::StreamInvalidated), &tx);
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_ok());
    }
}
