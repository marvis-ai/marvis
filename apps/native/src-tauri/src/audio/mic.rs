use super::{normalize_pcm, AudioSource, PcmChunk};
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, Stream, StreamConfig};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

const INPUT_QUEUE_CAPACITY: usize = 8;

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
        let callback_tx = input_tx.clone();
        let status_tx = self.status_tx.clone();
        let error_callback = move |error| {
            let message = format!("microphone stream error: {error}");
            let _ = status_tx.send(message.clone());
            log::error!("{message}");
        };

        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                config,
                move |data: &[f32], _| enqueue(data, sample_rate, channels, &callback_tx),
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
                    enqueue(&converted, sample_rate, channels, &callback_tx);
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
                    enqueue(&converted, sample_rate, channels, &callback_tx);
                },
                error_callback,
                None,
            ),
            format => return Err(anyhow!("unsupported microphone sample format: {format:?}")),
        }
        .map_err(|error| anyhow!("failed to build microphone input stream: {error}"))?;

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

    /// Retrieves the next callback status error, if one was reported.
    pub fn try_recv_status(&self) -> Option<String> {
        self.status_rx.try_recv().ok()
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
        self.start_with_format(output, device, supported.into(), sample_format)
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
}

impl Drop for MicSource {
    fn drop(&mut self) {
        self.stop();
    }
}

fn enqueue<T>(data: &[T], sample_rate: u32, channels: u16, sender: &SyncSender<RawChunk>)
where
    T: cpal::Sample + Copy,
    f32: FromSample<T>,
{
    let converted: Vec<f32> = data.iter().copied().map(f32::from_sample).collect();
    match sender.try_send((converted, sample_rate, channels)) {
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
