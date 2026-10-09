use super::*;

/// Transcript callback: fold each `me` event into the draft and emit the
/// live snapshot; events that can't change the draft emit nothing.
pub(super) fn transcript_callback(
    assembler: &Arc<Mutex<DraftAssembler>>,
    emit: &Arc<dyn Fn(DictationEvent) + Send + Sync>,
) -> Box<dyn Fn(TranscriptEvent) + Send + Sync> {
    let assembler = assembler.clone();
    let emit = emit.clone();
    Box::new(move |event| {
        let draft = assembler.lock().push(event);
        if let Some(draft) = draft {
            (emit)(DictationEvent::Draft(draft));
        }
    })
}

/// Terminal provider-error callback: cancel the pump, mark the durable
/// status `error`, and emit one sanitized `dictation:error`. The cancel
/// always lands (it also makes `commit` abort a start whose provider
/// died mid-build), but the status write and emit are skipped once the
/// epoch moved on — a late error from a session a `stop()` already tore
/// down must not clobber the winner's status. The epoch check runs
/// while holding `state` (same pattern as `fail()`): `stop()` bumps the
/// epoch before its own `idle` write, so checking under the lock keeps
/// that write from slipping between the check and the `error` write.
pub(super) fn provider_error_callback(
    state: &Arc<Mutex<DictationStatus>>,
    cancel: &Arc<AtomicBool>,
    epoch: &Arc<AtomicU64>,
    start_epoch: u64,
    emit: &Arc<dyn Fn(DictationEvent) + Send + Sync>,
) -> Box<dyn Fn(String) + Send + Sync> {
    let state = state.clone();
    let cancel = cancel.clone();
    let epoch = epoch.clone();
    let emit = emit.clone();
    Box::new(move |message| {
        cancel.store(true, Ordering::Release);
        let error = DictationError {
            message: sanitize_provider_error(&message),
            needs_setup: false,
        };
        {
            let mut status = state.lock();
            if epoch.load(Ordering::Acquire) != start_epoch {
                return;
            }
            status.state = "error".into();
            status.error = Some(error.clone());
        }
        (emit)(DictationEvent::Error {
            message: error.message,
            needs_setup: error.needs_setup,
        });
    })
}

/// Pump mic PCM into the provider until cancelled or the source
/// disconnects, then stop provider and source. Mirrors the Listen pump
/// minus turn flushing — dictation has no silence cadence.
pub(super) fn spawn_pump(
    rx: mpsc::Receiver<PcmChunk>,
    mut stt: Box<dyn SttProvider>,
    mut source: MicSource,
    cancel: Arc<AtomicBool>,
    report_error: Box<dyn Fn(String) + Send>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut dropped_chunks = 0usize;
        while !cancel.load(Ordering::Acquire) {
            // A fatal source error means the stream is dead even though
            // `is_running` still reads true — fail instead of dictating silence.
            if let Some(message) = source.try_recv_status() {
                log::error!("dictation microphone died mid-session: {message}");
                report_error("The microphone stopped working".to_string());
                break;
            }
            match rx.recv_timeout(WORKER_TICK) {
                Ok(chunk) => {
                    if !stt.enqueue(chunk) {
                        dropped_chunks += 1;
                        if dropped_chunks.is_multiple_of(100) {
                            log::warn!("dictation audio enqueue dropped {dropped_chunks} chunks");
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        stt.stop();
        source.stop();
    })
}

