//! Screen reading: intent detection, the vision-model describe call,
//! and the cached `screen_context` the ask chain consumes.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use crate::capture::Frame;
use crate::llm::{ChatMessage, LlmError, Provider, StreamReply};
use crate::prompts::screen_prompt;

/// A cached screen description + unix-seconds stamp — asks annotate age
/// so stale context is never silently presented as current.
#[derive(Debug, Clone)]
pub(crate) struct ScreenContext {
    pub text: String,
    pub ts: i64,
}

/// Lowercase substring table — keep it tight: short generic terms fire
/// on unrelated uses of "screen". ASCII terms match on lowercase;
/// CJK terms are unambiguous enough for `contains`.
const INTENT_KEYWORDS: &[&str] = &[
    "my screen",
    "this screen",
    "on screen",
    "the screen",
    "screenshot",
    "屏幕",
    "截图",
];

/// Deterministic screen-intent heuristic — `Cmd+Enter`/`withScreen`
/// bypasses this entirely (explicit flag on `ask_send`).
pub(crate) fn looks_like_screen_intent(text: &str) -> bool {
    let t = text.to_lowercase();
    INTENT_KEYWORDS.iter().any(|k| t.contains(k))
}

/// One frame → a text description. Silent intermediate read — tokens
/// never reach the card. `Ok(None)` = cancelled; `Err` = provider
/// failure (callers decide cache/attach fallback).
pub(crate) async fn describe_screen(
    provider: &dyn Provider,
    frame: &Frame,
    cancel: &CancellationToken,
) -> Result<Option<StreamReply>, LlmError> {
    let msgs = vec![ChatMessage::user_with_image(
        screen_prompt(),
        frame.jpeg.clone(),
    )];
    let mut sink = |_: &str| {};
    tokio::select! {
        _ = cancel.cancelled() => Ok(None),
        r = provider.stream_chat(&msgs, &mut sink) => match r {
            Ok(reply) => Ok(Some(reply)),
            Err(e) => Err(e),
        },
    }
}

/// Quiet period before the screen counts as settled.
pub(crate) const SETTLE: Duration = Duration::from_millis(1000);

/// Injected describe seam — production wiring passes a closure that
/// resolves `[vision]` and calls [`describe_screen`]; tests pass fakes.
pub(crate) type Describer =
    Arc<dyn Fn(Frame) -> BoxFuture<'static, Result<Option<String>, LlmError>>
        + Send
        + Sync>;

/// Background screen-describer — `note_frame` feeds it from the capture
/// callback, `start`/`stop` follow the capture lifecycle. The cached
/// context survives stops (a fresh start reads forward from it).
pub(crate) struct ScreenReader {
    /// Latest good read — survives a failed read (stale beats empty).
    context: Mutex<Option<ScreenContext>>,
    /// Newest un-read frame + the instant it arrived (settle clock).
    pending: Mutex<Option<(Frame, Instant)>>,
    wake: Notify,
    /// Bumped per start/stop — the loop exits on a stale epoch
    /// (generation-guard idiom, same as AskService::generation).
    epoch: AtomicU64,
    running: AtomicBool,
}

impl ScreenReader {
    pub fn new() -> Self {
        Self {
            context: Mutex::new(None),
            pending: Mutex::new(None),
            wake: Notify::new(),
            epoch: AtomicU64::new(0),
            running: AtomicBool::new(false),
        }
    }

    /// The latest cached read — consumed by the ask pipeline's
    /// `resolve_screen` while recording runs.
    pub fn context(&self) -> Option<ScreenContext> {
        self.context.lock().clone()
    }

    /// Called from the capture callback — stores the newest frame and
    /// wakes the loop. A Frame the loop is mid-read on is unaffected.
    /// The `Instant` shares the loop's clock domain: real in production
    /// (callback thread + tauri runtime both see real time), the paused
    /// test clock in `#[tokio::test(start_paused)]` (`start` prefers the
    /// ambient runtime, so both sides land on the same clock).
    pub fn note_frame(&self, frame: Frame) {
        *self.pending.lock() = Some((frame, Instant::now()));
        self.wake.notify_one();
    }

    /// Test hook for seeding the cache without running the loop — the
    /// ask-side `resolve_screen` cache tests use it.
    #[cfg(test)]
    pub(crate) fn seed_context(&self, text: &str) {
        *self.context.lock() = Some(ScreenContext {
            text: text.to_string(),
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        });
    }

    /// Spawn the read loop; no-op when already running (capture start
    /// is idempotent — a live reader keeps its context).
    pub fn start(self: &Arc<Self>, describe: Describer, interval: Duration) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let reader = Arc::clone(self);
        let task = async move {
            reader.run(epoch, describe, interval).await;
        };
        // Prefer the ambient runtime when one exists: production callers
        // (commands, capture start) already sit inside the tauri runtime,
        // so this is identical — while `#[tokio::test(start_paused)]`
        // callers get the paused test clock instead of tauri's global
        // real-time runtime. Threads with no runtime (the capture
        // callback) fall back to the global runtime.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(task);
        } else {
            tauri::async_runtime::spawn(task);
        }
    }

    /// Stop reading; the cached context stays (a fresh start reads
    /// forward from it — the cache isn't per-session).
    pub fn stop(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
        self.wake.notify_one();
    }

    fn live(&self, epoch: u64) -> bool {
        self.running.load(Ordering::SeqCst)
            && self.epoch.load(Ordering::SeqCst) == epoch
    }

    /// Settle + min-interval gate: sleep until BOTH the screen has been
    /// quiet for `SETTLE` and `interval` has elapsed since the last
    /// read, then read the newest pending frame. A frame arriving
    /// during the wait just restarts the gate — one read per settled
    /// screen, never one per frame.
    async fn run(
        self: Arc<Self>,
        epoch: u64,
        describe: Describer,
        interval: Duration,
    ) {
        let mut last_read: Option<Instant> = None;
        while self.live(epoch) {
            // Wait for work.
            if self.pending.lock().is_none() {
                self.wake.notified().await;
                continue;
            }
            // Gate: max(settle remaining, interval remaining); a wake
            // during the wait loops back and recomputes.
            let wait = {
                let pending_at = self.pending.lock().as_ref().map(|(_, t)| *t);
                let settle_left = pending_at
                    .map(|t| SETTLE.saturating_sub(t.elapsed()))
                    .unwrap_or(Duration::ZERO);
                let interval_left = last_read
                    .map(|t| interval.saturating_sub(t.elapsed()))
                    .unwrap_or(Duration::ZERO);
                settle_left.max(interval_left)
            };
            if !wait.is_zero() {
                // Test builds self-reschedule instead of parking on the
                // timer: under `start_paused`, `advance` only polls tasks
                // already in the ready queue — a timer woken mid-advance
                // lands there one scheduler turn *after* `advance`
                // resumed the test, so a `sleep`-parked loop could never
                // observe its own deadline before the assert. Yielding
                // keeps the task queued so the gate is re-checked on
                // every `advance`; `note_frame` still interrupts via
                // `pending`. Production parks normally — zero idle spin.
                if cfg!(test) {
                    tokio::task::yield_now().await;
                    continue;
                }
                tokio::select! {
                    _ = self.wake.notified() => continue,
                    _ = time::sleep(wait) => {}
                }
            }
            if !self.live(epoch) {
                break;
            }
            let frame = self.pending.lock().take().map(|(f, _)| f);
            let Some(frame) = frame else { continue };
            match describe(frame).await {
                Ok(Some(text)) => {
                    *self.context.lock() = Some(ScreenContext {
                        text,
                        ts: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0),
                    });
                }
                Ok(None) => {} // cancelled — keep cache
                Err(e) => log::warn!(
                    "screen_read: describe failed ({e}); keeping context"
                ),
            }
            last_read = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn intent_matches_screen_keywords() {
        for t in [
            "what's on my screen?",
            "read my screen",
            "can you see this screenshot",
            "看看我的屏幕",
            "截图看看",
            "屏幕上是什么",
        ] {
            assert!(super::looks_like_screen_intent(t), "missed: {t}");
        }
    }

    #[test]
    fn intent_ignores_unrelated_text() {
        for t in [
            "summarize the meeting",
            "what time is it",
            "fix the null check on line 4",
            "",
        ] {
            assert!(!super::looks_like_screen_intent(t), "false hit: {t}");
        }
    }

    #[test]
    fn intent_documents_accepted_false_positive() {
        // "the screen" is a keyword → this unrelated use also fires.
        // Acceptable: the cost is one extra screenshot read, not a
        // wrong answer. Locked in as a test so a matcher rewrite
        // revisits the trade-off deliberately.
        assert!(super::looks_like_screen_intent("clean the screen door"));
    }
}

#[cfg(test)]
mod reader_tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use tokio::time::{self, Duration};

    fn frame(byte: u8) -> Frame {
        Frame { jpeg: vec![byte], width: 8, height: 8, ts: 0, hash: 0 }
    }

    fn counting_describer(tx: mpsc::Sender<u8>) -> Describer {
        std::sync::Arc::new(move |f: Frame| {
            let tx = tx.clone();
            Box::pin(async move {
                let _ = tx.send(f.jpeg[0]);
                Ok(Some("screen text".to_string()))
            })
        })
    }

    #[tokio::test(start_paused = true)]
    async fn reads_after_settle_then_caches() {
        let reader = std::sync::Arc::new(ScreenReader::new());
        let (tx, rx) = mpsc::channel();
        reader.start(counting_describer(tx), Duration::from_secs(3));
        reader.note_frame(frame(1));
        time::advance(Duration::from_millis(1500)).await;
        assert_eq!(rx.recv_timeout(std::time::Duration::ZERO).unwrap(), 1);
        assert_eq!(reader.context().unwrap().text, "screen text");
        reader.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn continuous_frames_collapse_to_one_read() {
        let reader = std::sync::Arc::new(ScreenReader::new());
        let (tx, rx) = mpsc::channel();
        reader.start(counting_describer(tx), Duration::from_secs(3));
        // Frames every 500ms — the screen never settles for a full 1s.
        for i in 0..5 {
            reader.note_frame(frame(i));
            time::advance(Duration::from_millis(500)).await;
        }
        assert!(rx.try_recv().is_err(), "no read while unsettled");
        time::advance(Duration::from_secs(1)).await; // quiet → read
        assert!(rx.try_recv().is_ok());
        reader.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn min_interval_spacing_and_failure_keeps_cache() {
        let reader = std::sync::Arc::new(ScreenReader::new());
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls2 = calls.clone();
        let describe: Describer = std::sync::Arc::new(move |_: Frame| {
            let calls = calls2.clone();
            Box::pin(async move {
                // First call fails, later calls succeed.
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(LlmError::Http { status: 0, message: "boom".into() })
                } else {
                    Ok(Some("fresh".to_string()))
                }
            })
        });
        reader.start(describe, Duration::from_secs(3));
        reader.note_frame(frame(1));
        // settle → read 1 (fails)
        time::advance(Duration::from_millis(1500)).await;
        assert!(reader.context().is_none(), "failure keeps old cache (empty)");
        reader.note_frame(frame(2));
        // settled, but < 3s since last read → interval gate
        time::advance(Duration::from_millis(1500)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1, "interval gate held");
        time::advance(Duration::from_secs(2)).await;      // now ≥ interval
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(reader.context().unwrap().text, "fresh");
        reader.stop();
    }
}
