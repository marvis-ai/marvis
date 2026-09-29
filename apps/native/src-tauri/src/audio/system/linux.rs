//! PulseAudio monitor tap on the default sink — the loopback "them"
//! channel on Linux. Under PipeWire the `pipewire-pulse` shim speaks the
//! same protocol, so one backend covers both sound servers. The worker
//! thread owns the PA mainloop (libpulse objects are `!Send`); `start`
//! blocks only on a one-shot init handshake so setup failures surface
//! as `Err`, same as the macOS `SCStream` path.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Result};
use libpulse_binding::context::{Context, FlagSet as ContextFlagSet, State as ContextState};
use libpulse_binding::def::BufferAttr;
use libpulse_binding::mainloop::standard::{IterateResult, Mainloop};
use libpulse_binding::operation::State as OperationState;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::{
    FlagSet as StreamFlagSet, PeekResult, State as StreamState, Stream,
};

use super::{interleaved_pcm_to_f32, normalize_pcm, status_channel, warn_unsupported};
use crate::audio::{AudioSource, PcmChunk};

/// Requested capture format — PulseAudio resamples/remixes server-side,
/// so this is fixed regardless of the sink's native layout.
const SPEC: Spec = Spec {
    format: Format::F32le,
    rate: 48_000,
    channels: 2,
};
/// ~20 ms fragments (48_000 × 2ch × 4B = 384 kB/s → 20 ms ≈ 7.7 kB).
const FRAG_BYTES: u32 = 7_680;
/// Idle sleep between mainloop iterations when nothing is readable.
const POLL_IDLE: Duration = Duration::from_millis(5);

/// System audio via the default sink's monitor source.
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
        self.stop.store(false, Ordering::Relaxed);
        let stop = Arc::clone(&self.stop);
        let running = Arc::clone(&self.running);
        let status_tx = self.status_tx.clone();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let worker = thread::spawn(move || {
            pulse_worker(output, stop, running, status_tx, ready_tx);
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

/// Pump the mainloop once; `false` on quit/error so every caller's
/// `while` collapses the same way.
fn pump(mainloop: &Rc<RefCell<Mainloop>>) -> bool {
    match mainloop.borrow_mut().iterate(false) {
        IterateResult::Success(_) => true,
        IterateResult::Err(_) | IterateResult::Quit(_) => false,
    }
}

fn pulse_worker(
    output: Sender<PcmChunk>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    status_tx: mpsc::Sender<String>,
    ready_tx: mpsc::Sender<Result<(), String>>,
) {
    // Every init-stage failure reports through `ready_tx` (start() returns
    // it) and exits the worker — it must never send twice or hang.
    macro_rules! fail {
        ($msg:expr) => {{
            let message: String = $msg;
            log::warn!("{message}");
            let _ = ready_tx.send(Err(message));
            return;
        }};
    }

    let Some(mainloop) = Mainloop::new() else {
        fail!("system audio: failed to create the PulseAudio mainloop".into());
    };
    let mainloop = Rc::new(RefCell::new(mainloop));
    let context = {
        let borrowed = mainloop.borrow();
        match Context::new(&*borrowed, "marvis") {
            Some(context) => Rc::new(RefCell::new(context)),
            None => fail!("system audio: failed to create the PulseAudio context".into()),
        }
    };
    if context
        .borrow_mut()
        .connect(None, ContextFlagSet::NOFLAGS, None)
        .is_err()
    {
        fail!("system audio: PulseAudio connect failed".into());
    }
    loop {
        if !pump(&mainloop) {
            fail!("system audio: PulseAudio mainloop quit".into());
        }
        match context.borrow().get_state() {
            ContextState::Ready => break,
            ContextState::Failed | ContextState::Terminated => {
                fail!("system audio: PulseAudio connection failed".into());
            }
            _ => {}
        }
        thread::sleep(POLL_IDLE);
    }

    // The monitor source of the default sink (`<sink>.monitor`) —
    // `pavucontrol`'s "Monitor of …" entries.
    let sink: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let operation = {
        let sink = Rc::clone(&sink);
        let context = context.borrow();
        context.introspect().get_server_info(move |info| {
            *sink.borrow_mut() = info.default_sink_name.as_deref().map(str::to_owned);
        })
    };
    while operation.get_state() == OperationState::Running {
        if !pump(&mainloop) {
            fail!("system audio: PulseAudio mainloop quit".into());
        }
        thread::sleep(POLL_IDLE);
    }
    let Some(sink_name) = sink.borrow().clone() else {
        fail!("system audio: no default sink to monitor".into());
    };
    let monitor = format!("{sink_name}.monitor");

    let stream = {
        let mut context = context.borrow_mut();
        debug_assert!(SPEC.is_valid());
        match Stream::new(&mut context, "marvis system audio", &SPEC, None) {
            Some(stream) => Rc::new(RefCell::new(stream)),
            None => fail!("system audio: failed to create the record stream".into()),
        }
    };
    let attr = BufferAttr {
        maxlength: u32::MAX,
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: FRAG_BYTES,
    };
    if stream
        .borrow_mut()
        .connect_record(
            Some(&monitor),
            Some(&attr),
            StreamFlagSet::ADJUST_LATENCY | StreamFlagSet::DONT_MOVE,
        )
        .is_err()
    {
        fail!(format!("system audio: connect_record to {monitor} failed"));
    }
    loop {
        if !pump(&mainloop) {
            fail!("system audio: PulseAudio mainloop quit".into());
        }
        match stream.borrow().get_state() {
            StreamState::Ready => break,
            StreamState::Failed | StreamState::Terminated => {
                fail!(format!("system audio: monitor stream {monitor} failed"));
            }
            _ => {}
        }
        thread::sleep(POLL_IDLE);
    }
    let _ = ready_tx.send(Ok(()));
    running.store(true, Ordering::Release);
    let warned = AtomicBool::new(false);

    while !stop.load(Ordering::Relaxed) {
        if !pump(&mainloop) {
            let _ = status_tx.send("system audio: PulseAudio mainloop quit".into());
            break;
        }
        let mut drained = false;
        loop {
            let mut stream = stream.borrow_mut();
            match stream.peek() {
                Ok(PeekResult::Data(bytes)) => {
                    if let Some(samples) = interleaved_pcm_to_f32(bytes, 32, true) {
                        if output
                            .send(normalize_pcm(&samples, SPEC.rate, u16::from(SPEC.channels)))
                            .is_err()
                        {
                            return;
                        }
                    } else {
                        warn_unsupported(&warned, "monitor packet rejected");
                    }
                    let _ = stream.discard();
                    drained = true;
                }
                // A hole is dropped data — skip it, timing continues.
                Ok(PeekResult::Hole(_)) => {
                    let _ = stream.discard();
                    drained = true;
                }
                Ok(PeekResult::Empty) => break,
                Err(e) => {
                    let _ = status_tx.send(format!("system audio read failed: {e}"));
                    return;
                }
            }
        }
        if !drained {
            thread::sleep(POLL_IDLE);
        }
    }
    running.store(false, Ordering::Release);
}
