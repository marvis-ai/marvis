# Screen-Context Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the attach-latest-frame-on-ask flow with a cached
`screen_context` produced by a settle-gated background reader, plus
on-demand single-shot screenshots and a native content picker for
capture scope.

**Architecture:** A new `screen_read` module owns the vision read
(`describe_screen` moves there), a keyword intent matcher, and a
`ScreenReader` background task that reads the newest settled frame at a
bounded cadence and caches `{text, ts}`. `send_chain` resolves screen
material via a `resolve_screen` truth table (cache / raw ring frame /
fresh single-shot / text-only). `MacosCapture` takes an injected
`SCContentFilter`, enabling both the existing primary-display default
and picker-produced window/app scopes. All frame paths share one
`encode_frame` (BGRA → downscale ≤1600w → JPEG q80); pixels never hit
disk or JS.

**Tech Stack:** Rust + Tauri 2, `screencapturekit` 10.0.3
(`SCScreenshotManager`, `SCContentSharingPicker`), `image` 0.25, tokio
(`Notify`, `time::pause` tests), React/TS frontend (`bun`).

Spec: `docs/superpowers/specs/2026-09-28-screen-context-redesign-design.md`

## Global Constraints

- Pixel privacy: frame bytes never touch disk and are never serialized
  to JavaScript (`capture:state` carries `{running, frames, target}`
  only).
- Markdown prose wraps at 80 chars (`.markdownlint.json` MD013; tables
  exempt).
- `bun` for all package scripts; `cargo` commands run in
  `apps/native/src-tauri`.
- Rust diagnostics use `log::warn!` / `log::error!` (no `info!` —
  `env_logger` sits at its default).
- React components/hooks are arrow functions; `Icon`-suffixed lucide
  names; named exports for `src/components/*`.
- Screen-intent matching is deterministic keywords + `Cmd+Enter` flag —
  no LLM/agentic routing (Rule 5).
- `screencapturekit` API notes (verified against 10.0.3):
  `SCScreenshotManager::capture_sample_buffer(&filter, &config)`
  returns `CMSampleBuffer` (same shape as stream output);
  `SCContentSharingPicker::show(&cfg, cb)` takes
  `F: FnOnce(SCPickerOutcome) + Send + 'static`;
  `SCPickerResult::{filter(), pixel_size(), source()}`;
  `SCPickedSource::{Window(String), Display(u32), Application(String),
  Unknown}`;
  `SCContentSharingPickerMode::{SingleWindow, SingleDisplay,
  SingleApplication}`; `set_excluded_bundle_ids(&[&str])` with
  `"com.getmarvis.marvis"`.

---

### Task 1: Width cap + shared `encode_frame` (`capture/macos.rs`)

**Files:**

- Modify: `apps/native/src-tauri/src/capture/macos.rs`

**Interfaces:**

- Produces: `fn encode_frame(raw: &RawFrame, hash: u64) ->
  Option<Frame>` — the single BGRA→JPEG path later used by
  `shot_fullscreen` (Task 2). `TARGET_WIDTH: u32 = 1600`.

- [ ] **Step 1: Write the failing test** — add to the existing
  `mod tests` in `macos.rs`:

```rust
    #[test]
    fn encode_frame_caps_width_at_target() {
        // 3200x2000 solid-black BGRA (bytes_per_row == width * 4).
        let raw = RawFrame {
            data: vec![0u8; 3200 * 2000 * 4],
            width: 3200,
            height: 2000,
            bytes_per_row: 3200 * 4,
        };
        let frame = super::encode_frame(&raw, 0).expect("encode succeeds");
        assert_eq!(frame.width, 1600);
        assert_eq!(frame.height, 1000);
        assert!(!frame.jpeg.is_empty());
        assert_eq!(frame.hash, 0);
    }

    #[test]
    fn encode_frame_passes_small_sources_through() {
        let raw = RawFrame {
            data: vec![0u8; 800 * 600 * 4],
            width: 800,
            height: 600,
            bytes_per_row: 800 * 4,
        };
        let frame = super::encode_frame(&raw, 7).expect("encode succeeds");
        assert_eq!((frame.width, frame.height), (800, 600));
        assert_eq!(frame.hash, 7);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/native/src-tauri && cargo test encode_frame`
Expected: FAIL — `cannot find function encode_frame`.

- [ ] **Step 3: Implement** — replace `TARGET_HEIGHT` with
  `TARGET_WIDTH` and factor the encode tail out of `run_worker`:

```rust
/// Longest-side cap for the encoded frame: width is capped at 1600 px,
/// height follows aspect. Screen reading needs legible text — the old
/// 384 px height cap made on-screen text illegible and the vision model
/// confabulated details. `Frame` widths below the cap pass through.
const TARGET_WIDTH: u32 = 1600;
```

Extract the shared encoder (module-private; returns `None` on encode
failure so callers decide retry policy):

```rust
/// BGRA → JPEG `Frame`: optional width-cap downscale, q80 encode, stamp.
/// Shared by the stream worker and the one-shot screenshot so both
/// produce identical `Frame`s. `hash` is the caller's dedupe hash (0
/// for single-shots).
fn encode_frame(raw: &RawFrame, hash: u64) -> Option<Frame> {
    let view = BgraView {
        data: &raw.data,
        width: raw.width,
        height: raw.height,
        bytes_per_row: raw.bytes_per_row,
    };
    let mut jpeg = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY);
    let (out_w, out_h, encoded) = if raw.width > TARGET_WIDTH {
        let out_h = (u64::from(raw.height) * u64::from(TARGET_WIDTH)
            / u64::from(raw.width)) as u32;
        let out_h = out_h.max(1);
        let resized =
            imageops::resize(&view, TARGET_WIDTH, out_h, FilterType::Triangle);
        (TARGET_WIDTH, out_h, encoder.encode_image(&resized))
    } else {
        (raw.width, raw.height, encoder.encode_image(&view))
    };
    match encoded {
        Ok(()) => Some(Frame {
            jpeg,
            width: out_w,
            height: out_h,
            ts: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            hash,
        }),
        Err(e) => {
            log::warn!("jpeg encode failed ({out_w}x{out_h}): {e}");
            None
        }
    }
}
```

In `run_worker`, replace the `view`/`jpeg`/`encoder`/`match encoded`
block (old lines ~299-333) with:

```rust
        // Leave last_hash untouched on encode failure so the next
        // identical frame retries (unchanged rule).
        if let Some(frame) = encode_frame(&raw, hash) {
            last_hash = Some(hash);
            on_frame(frame);
        }
```

- [ ] **Step 4: Run tests**

Run: `cd apps/native/src-tauri && cargo test capture::macos`
Expected: PASS — `encode_frame_caps_width_at_target`,
`encode_frame_passes_small_sources_through`, `frame_interval_maps…`.

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/capture/macos.rs
git commit -m "capture: cap frame width at 1600px, factor shared encode_frame"
```

---

### Task 2: `shot_fullscreen` one-shot capture (`capture/macos.rs` + `mod.rs`)

**Files:**

- Modify: `apps/native/src-tauri/src/capture/macos.rs`
- Modify: `apps/native/src-tauri/src/capture/mod.rs`

**Interfaces:**

- Consumes: `encode_frame` (Task 1), `primary_display_filter`,
  `extract_raw`.
- Produces: `crate::capture::shot_fullscreen() ->
  anyhow::Result<Option<Frame>>` — used by ask `resolve_screen`
  (Task 8). `Ok(None)` = the shot produced no pixel data;
  `Err` = permission/API failure.

- [ ] **Step 1: Add the one-shot function** to `macos.rs`:

```rust
/// One-shot screenshot of the primary display via
/// `SCScreenshotManager` — the "read my screen" path when ambient
/// recording is off. Same filter and encode path as the stream, so a
/// single-shot `Frame` is indistinguishable from a ring frame.
/// `Ok(None)` means the call succeeded but delivered no pixels.
pub(crate) fn shot_fullscreen() -> anyhow::Result<Option<Frame>> {
    let (filter, width, height) = primary_display_filter()?;
    let config = SCStreamConfiguration::new()
        .with_width(width)
        .with_height(height)
        .with_pixel_format(PixelFormat::BGRA)
        .with_shows_cursor(false);
    let sample =
        SCScreenshotManager::capture_sample_buffer(&filter, &config)
            .map_err(|e| anyhow::anyhow!("screenshot failed: {e}"))?;
    let Some(raw) = extract_raw(&sample) else {
        return Ok(None);
    };
    Ok(encode_frame(&raw, 0))
}
```

Add the import: `use screencapturekit::screenshot_manager::SCScreenshotManager;`

- [ ] **Step 2: Re-export** in `capture/mod.rs` alongside the existing
  `MacosCapture`/`RingBuffer` re-exports:

```rust
pub(crate) use macos::shot_fullscreen;
```

- [ ] **Step 3: Compile-check** (the API call needs a real display —
  no unit test; the ignored manual capture test convention applies):

Run: `cd apps/native/src-tauri && cargo test capture --no-run`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add apps/native/src-tauri/src/capture/
git commit -m "capture: one-shot shot_fullscreen via SCScreenshotManager"
```

---

### Task 3: `screen_read` module — types, intent matcher, `describe_screen`

**Files:**

- Create: `apps/native/src-tauri/src/screen_read.rs`
- Modify: `apps/native/src-tauri/src/ask.rs` (move `describe_screen`
  out; call `screen_read::describe_screen`)
- Modify: `apps/native/src-tauri/src/lib.rs` (`mod screen_read;`)

**Interfaces:**

- Produces (consumed by Tasks 5 + 8):
  - `pub(crate) struct ScreenContext { pub text: String, pub ts: i64 }`
  - `pub(crate) fn looks_like_screen_intent(text: &str) -> bool`
  - `pub(crate) async fn describe_screen(provider: &dyn Provider,
    frame: &Frame, cancel: &CancellationToken) ->
    Result<Option<StreamReply>, LlmError>` — `Ok(None)` = cancelled;
    moves the vision call + prompt from `ask.rs`.
  - `pub(crate) fn screen_prompt() -> String` — the existing
    `prompts::screen_prompt` re-exported or moved here (check which
    module owns it today; if `prompts.rs`, just import it inside
    `screen_read`).

- [ ] **Step 1: Write the failing intent tests** — in
  `screen_read.rs` `mod tests`:

```rust
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test screen_read` → FAIL (module doesn't exist).

- [ ] **Step 3: Implement `screen_read.rs`:**

```rust
//! Screen reading: intent detection, the vision-model describe call,
//! and the cached `screen_context` the ask chain consumes.

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

/// Deterministic screen-intent heuristic — `Cmd+Enter` bypasses this
/// entirely (explicit flag on `ask_send`).
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
```

(`ChatMessage` import from `crate::llm` as in `ask.rs` today.)

- [ ] **Step 4: Update `ask.rs`** — delete the local `describe_screen`
  (old lines ~608-629); at the vision-read call site replace the
  `StreamOutcome` match with the new result shape:

```rust
    if let (Some(f), Some(vis)) = (frame, vision.as_ref()) {
        let read =
            crate::screen_read::describe_screen(&*vis.provider, f, cancel)
                .await;
        match read {
            Ok(Some(reply)) => {
                screen = Some(reply.full);
                if let Some(u) = reply.usage {
                    usage.add(&u);
                }
                frame = None;
            }
            Ok(None) => {
                emit(EV_STATE, json!({"state": "idle"}));
                return Err(LlmError::Http {
                    status: 0,
                    message: "cancelled".to_string(),
                });
            }
            Err(e) => {
                log::warn!(
                    "ask: vision read via {} failed ({e}); \
                     attaching the frame to the chain",
                    vis.id
                );
            }
        }
    }
```

(This block moves again in Task 8 — this step keeps the tree compiling
and tests passing in between.)

- [ ] **Step 5:** add `mod screen_read;` to `lib.rs` next to the other
  `mod` declarations; `cargo test screen_read` → PASS; `cargo test
  ask` → existing tests still green.

- [ ] **Step 6: Commit**

```bash
git add apps/native/src-tauri/src/
git commit -m "screen_read: intent matcher + describe_screen moved out of ask"
```

---

### Task 4: `recording.read_interval_secs` config (`config.rs`)

**Files:**

- Modify: `apps/native/src-tauri/src/config.rs`

**Interfaces:**

- Produces: `Config.recording.read_interval_secs: u64` (default 3,
  min 1) — consumed by `start_capture` when it starts the reader
  (Task 6) and by `resolve_screen` for the stale-age hint (Task 8).

- [ ] **Step 1: Failing test** in `config.rs`'s existing test module:

```rust
    #[test]
    fn recording_read_interval_defaults_and_sets() {
        let prefs = RecordingPrefs::default();
        assert_eq!(prefs.read_interval_secs, 3);
        let mut cfg = Config::default();
        assert!(apply_recording_config(
            &mut cfg.recording,
            "recording.read_interval_secs",
            &serde_json::json!(5)
        )
        .unwrap());
        assert_eq!(cfg.recording.read_interval_secs, 5);
        assert!(apply_recording_config(
            &mut cfg.recording,
            "recording.read_interval_secs",
            &serde_json::json!(0)
        )
        .is_err());
    }
```

- [ ] **Step 2:** `cargo test recording_read_interval` → FAIL.

- [ ] **Step 3: Implement** — field + default + `apply_recording_config`
  arm:

```rust
    /// Minimum seconds between background screen reads (settle gate is
    /// fixed at ~1s); min 1.
    pub read_interval_secs: u64,
```

`Default`: `read_interval_secs: 3`. Config arm:

```rust
        "recording.read_interval_secs" => {
            let secs = value
                .as_u64()
                .ok_or("recording.read_interval_secs must be a number")?;
            if secs < 1 {
                return Err("recording.read_interval_secs must be >= 1".into());
            }
            recording.read_interval_secs = secs;
            Ok(true)
        }
```

- [ ] **Step 4:** `cargo test config` → PASS.

- [ ] **Step 5: Commit** — `config: recording.read_interval_secs
  (default 3s, min 1)`.

---

### Task 5: `ScreenReader` settle-gated loop (`screen_read.rs`)

**Files:**

- Modify: `apps/native/src-tauri/src/screen_read.rs`

**Interfaces:**

- Produces (consumed by `lib.rs` Task 6 + `ask.rs` Task 8):
  - `pub(crate) struct ScreenReader` — `new()`, `context() ->
    Option<ScreenContext>`, `note_frame(Frame)`, `start(Arc<Self>,
    Describer, Duration)`, `stop()`.
  - `pub(crate) type Describer = Arc<dyn Fn(Frame) ->
    BoxFuture<'static, Result<Option<String>, LlmError>> + Send +
    Sync>` — injected so tests don't need providers.
  - `pub(crate) const SETTLE: Duration = Duration::from_millis(1000)`.

- [ ] **Step 1: Failing tests** — `tokio::time::pause` + injected
  describer counting calls through a `std::sync::mpsc` channel:

```rust
#[cfg(test)]
mod reader_tests {
    use super::*;
    use futures::future::BoxFuture;
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
```

- [ ] **Step 2:** `cargo test screen_read` → FAIL (types missing).

- [ ] **Step 3: Implement** — fields + loop:

```rust
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use futures::future::BoxFuture;
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::time::{self, Instant};

/// Quiet period before the screen counts as settled.
pub(crate) const SETTLE: Duration = Duration::from_millis(1000);

/// Injected describe seam — production wiring passes a closure that
/// resolves `[vision]` and calls [`describe_screen`]; tests pass fakes.
pub(crate) type Describer =
    Arc<dyn Fn(Frame) -> BoxFuture<'static, Result<Option<String>, LlmError>>
        + Send
        + Sync>;

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

    pub fn context(&self) -> Option<ScreenContext> {
        self.context.lock().clone()
    }

    /// Called from the capture callback — stores the newest frame and
    /// wakes the loop. A Frame the loop is mid-read on is unaffected.
    pub fn note_frame(&self, frame: Frame) {
        *self.pending.lock() = Some((frame, Instant::now()));
        self.wake.notify_one();
    }

    /// Test hook for seeding the cache without running the loop.
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
        tauri::async_runtime::spawn(async move {
            reader.run(epoch, describe, interval).await;
        });
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
                let pending_at = self.pending.lock().map(|(_, t)| t);
                let settle_left = pending_at
                    .map(|t| SETTLE.saturating_sub(t.elapsed()))
                    .unwrap_or(Duration::ZERO);
                let interval_left = last_read
                    .map(|t| interval.saturating_sub(t.elapsed()))
                    .unwrap_or(Duration::ZERO);
                settle_left.max(interval_left)
            };
            if !wait.is_zero() {
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
```

`LlmError` variants are `Http{status,message}`, `Auth`, `NoModel`,
`NoEndpoint`, `MultimodalUnsupported`, `Network` — synthesized failures
use `Http{status: 0, message: …}` (the codebase's existing non-HTTP
convention, e.g. the cancelled marker).

- [ ] **Step 4:** `cargo test screen_read` → PASS (3 reader tests +
  intent tests).

- [ ] **Step 5: Commit** — `screen_read: settle-gated ScreenReader
  with injectable describer`.

---

### Task 6: Filter-parameterized capture + reader wiring

(`lib.rs`, `macos.rs`)

**Files:**

- Modify: `apps/native/src-tauri/src/capture/macos.rs`
- Modify: `apps/native/src-tauri/src/lib.rs`

**Interfaces:**

- `MacosCapture::new(filter: SCContentFilter, width: u32, height: u32,
  fps: u32)` — callers own filter construction.
- `start_capture(app: &AppHandle, filter: SCContentFilter, width: u32,
  height: u32, target: Option<CaptureTarget>) -> CaptureStatus` —
  consumed by Task 7's picker.
- `pub(crate) struct CaptureTarget { pub kind: &'static str, pub
  label: String }` (`Serialize`) — `capture:state` payload gains
  `"target"` (null on the auto-display path).
- `AppState.screen_reader: Arc<ScreenReader>` and
  `AppState.capture_target: Mutex<Option<CaptureTarget>>`.

- [ ] **Step 1: Refactor `MacosCapture::new`** — replace body with:

```rust
    /// Wrap a caller-built filter (primary display via
    /// [`primary_display_filter`], or a picker result) with the shared
    /// stream configuration. Does not start capturing.
    pub fn new(
        filter: SCContentFilter,
        width: u32,
        height: u32,
        fps: u32,
    ) -> anyhow::Result<Self> {
        let config = SCStreamConfiguration::new()
            .with_width(width)
            .with_height(height)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(false)
            .with_minimum_frame_interval(&CMTime::from_seconds(
                frame_interval_secs(fps),
                600,
            ))
            .with_queue_depth(3);
        Ok(Self {
            state: Mutex::new(CaptureState { filter, config, running: None }),
        })
    }

    /// The auto-start path: whole primary display, Marvis's own windows
    /// excluded (a picker can't appear without user interaction).
    pub fn for_display(fps: u32) -> anyhow::Result<Self> {
        let (filter, width, height) = primary_display_filter()?;
        Self::new(filter, width, height, fps)
    }
```

Update the ignored manual test to `MacosCapture::for_display(4)`.

- [ ] **Step 2: `lib.rs`** — `CaptureTarget` + `AppState` fields +
  `capture_snapshot` gains `target`:

```rust
/// The picker-selected capture scope shown by `capture:state` —
/// `None` on the auto primary-display path.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureTarget {
    /// "display" | "window" | "app"
    pub kind: &'static str,
    pub label: String,
}
```

`AppState` fields:

```rust
    screen_reader: Arc<screen_read::ScreenReader>,
    capture_target: Mutex<Option<CaptureTarget>>,
```

`app.manage(AppState { … })` init adds:

```rust
                screen_reader: Arc::new(screen_read::ScreenReader::new()),
                capture_target: Mutex::new(None),
```

`capture_snapshot` adds `"target": state.capture_target.lock().clone()`.

- [ ] **Step 3: `start_capture` signature + reader lifecycle:**

```rust
fn start_capture(
    app: &AppHandle,
    filter: SCContentFilter,
    width: u32,
    height: u32,
    target: Option<CaptureTarget>,
) -> CaptureStatus {
    let state = app.state::<AppState>();
    // Config reads precede the capture lock (existing rule).
    let (fps, read_interval_secs) = {
        let cfg = state.config.lock();
        (cfg.recording.fps, cfg.recording.read_interval_secs)
    };
    {
        let mut slot = state.capture.lock();
        let mut lifecycle = CaptureLifecycle::default();
        if slot.as_ref().is_some_and(MacosCapture::is_running) {
            lifecycle.mark_running();
        }
        if lifecycle.start_decision() == StartDecision::Create {
            match MacosCapture::new(filter, width, height, fps) {
                Ok(capture) => {
                    let ring = Arc::clone(&state.ring);
                    let reader = Arc::clone(&state.screen_reader);
                    capture.start(Box::new(move |frame| {
                        reader.note_frame(frame.clone());
                        ring.lock().push(frame);
                    }));
                    if capture.is_running() {
                        *slot = Some(capture);
                        *state.capture_target.lock() = target;
                        lifecycle.mark_running();
                    } else {
                        log::warn!("gate: screen capture failed to start");
                    }
                }
                Err(e) => log::warn!("gate: capture init failed: {e}"),
            }
        }
    }
    // Background reader: resolve [vision] per read so provider changes
    // take effect live; a missing vision config just skips reads.
    if state
        .capture
        .lock()
        .as_ref()
        .is_some_and(MacosCapture::is_running)
    {
        let app2 = app.clone();
        let describe: screen_read::Describer = Arc::new(move |frame| {
            let app = app2.clone();
            Box::pin(async move {
                let state = app.state::<AppState>();
                let vision = {
                    let cfg = state.config.lock();
                    let ks = state.keystore.lock();
                    crate::vision_candidate(&cfg, &ks)
                };
                match vision {
                    Some(v) => crate::screen_read::describe_screen(
                        &*v.provider,
                        &frame,
                        &tokio_util::sync::CancellationToken::new(),
                    )
                    .await
                    .map(|o| o.map(|r| r.full)),
                    None => Err(crate::llm::LlmError::Http {
                        status: 0,
                        message: "no vision provider configured".into(),
                    }),
                }
            })
        });
        state.screen_reader.start(
            describe,
            Duration::from_secs(read_interval_secs.max(1)),
        );
    }
    let status = capture_snapshot(&state);
    emit_capture_state(app, &status);
    status
}
```

`stop_capture` adds after `capture.stop()`:

```rust
    state.screen_reader.stop();
    *state.capture_target.lock() = None;
```

- [ ] **Step 4: Update the three `start_capture` call sites**
  (`enter_main`, `capture_start` command, `toggle_capture`) — each
  resolves the default display filter first:

```rust
    match primary_display_filter() {
        Ok((filter, w, h)) => start_capture(app, filter, w, h, None),
        Err(e) => log::warn!("capture: display filter failed: {e}"),
    }
```

(`primary_display_filter` is `pub(crate)` in `capture::macos` —
re-export through `capture::` if needed.)

- [ ] **Step 5:** `cargo test` — full suite green (existing lifecycle
  tests exercise `start_capture`; update any call sites/tests that used
  the old signature — e.g. the gate-guard source test asserting
  `capture_start` body still matches).

- [ ] **Step 6: Commit** — `capture: injectable filter + picker
  target state + background reader wiring`.

---

### Task 7: `capture_pick_and_start` command (`lib.rs`)

**Files:**

- Modify: `apps/native/src-tauri/src/lib.rs`

**Interfaces:**

- Consumes: `start_capture(app, filter, w, h, Some(target))` (Task 6).
- Produces: `capture_pick_and_start` Tauri command; JS:
  `invoke('capture_pick_and_start')` → picker UI; cancel = no-op.

- [ ] **Step 1: Failing source-guard test** — mirror the existing
  `capture_start_is_gate_guarded…` test's shape (it reads lib.rs source
  and asserts the gate check exists):

```rust
    #[test]
    fn capture_pick_and_start_is_gate_guarded() {
        let src = include_str!("lib.rs");
        let body = src
            .split("fn capture_pick_and_start")
            .nth(1)
            .expect("command exists");
        assert!(body.contains("Gate::Main"), "picker start must be Main-gated");
    }
```

- [ ] **Step 2:** `cargo test capture_pick_and_start` → FAIL.

- [ ] **Step 3: Implement** (picker must run on the main thread —
  `run_on_main_thread` hops; `show`'s callback is `Send`-capable):

```rust
/// Idle-state record button: the native macOS content-sharing picker
/// (window / display / application) — same UI Zoom shows. Marvis's own
/// bundle id is excluded so it can never offer itself. Cancel is a
/// silent no-op; `capture:state` reports the picked `target`.
#[tauri::command]
fn capture_pick_and_start(app: AppHandle) {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("capture_pick_and_start dropped while gate != Main");
        return;
    }
    let app2 = app.clone();
    if let Err(e) = app.run_on_main_thread(move |_| {
        use screencapturekit::content_sharing_picker::*;
        let mut cfg = SCContentSharingPickerConfiguration::new();
        cfg.set_allowed_picker_modes(&[
            SCContentSharingPickerMode::SingleWindow,
            SCContentSharingPickerMode::SingleDisplay,
            SCContentSharingPickerMode::SingleApplication,
        ]);
        cfg.set_excluded_bundle_ids(&["com.getmarvis.marvis"]);
        SCContentSharingPicker::show(&cfg, move |outcome| {
            if let SCPickerOutcome::Picked(result) = outcome {
                let (w, h) = result.pixel_size();
                let target = match result.source() {
                    SCPickedSource::Window(t) => CaptureTarget {
                        kind: "window",
                        label: t,
                    },
                    SCPickedSource::Display(id) => CaptureTarget {
                        kind: "display",
                        label: format!("Display {id}"),
                    },
                    SCPickedSource::Application(n) => CaptureTarget {
                        kind: "app",
                        label: n,
                    },
                    SCPickedSource::Unknown => CaptureTarget {
                        kind: "app",
                        label: "Screen".into(),
                    },
                };
                start_capture(&app2, result.filter(), w, h, Some(target));
            }
        });
    }) {
        log::warn!("capture_pick_and_start: main-thread hop failed: {e}");
    }
}
```

Register in `invoke_handler!` next to `capture_start`. Import
`Serialize` is already present (`serde::Serialize` for CaptureTarget).

- [ ] **Step 4:** `cargo test capture_pick` → PASS; `cargo check`
  clean. Manual-verify picker UI via `bun run build:dev` later (native
  UI can't be unit-tested — same convention as the ignored capture
  test).

- [ ] **Step 5: Commit** — `capture: capture_pick_and_start via
  SCContentSharingPicker`.

---

### Task 8: Ask pipeline — `with_screen` + `resolve_screen`

(`ask.rs`, `lib.rs`)

**Files:**

- Modify: `apps/native/src-tauri/src/ask.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` (`Deps` construction,
  `ask_send`/`ask_send_screen_only` commands, deeplink call)

**Interfaces:**

- `Deps` gains `pub reader: Arc<screen_read::ScreenReader>` and `pub
  capture_running: bool`.
- `AskService::send(app, deps, text, with_screen: bool)`;
  `kick(app, deps, text, with_screen, screen_required, regenerate)`.
- `pub(crate) struct ScreenInput<'a>` + `resolve_screen` (below).
- `send_chain` signature: `vision` param REMOVED; `frame:
  Option<&Frame>` replaced by `screen_input: &ScreenInput`.

- [ ] **Step 1: Failing tests** for the truth table — drive
  `resolve_screen` directly. `ScreenInput` deliberately holds NO
  `AppHandle` (unconstructable in tests): the permission check and the
  `capture:permission-needed` emit are injected seams, and the mock
  vision provider reuses the tests module's `MockProvider`/`candidate`/
  `recorder` helpers:

```rust
    fn test_frame() -> Frame {
        Frame { jpeg: vec![1, 2, 3], width: 8, height: 8, ts: 0, hash: 0 }
    }

    fn input<'a>(
        reader: &'a screen_read::ScreenReader,
        ring: &'a Mutex<RingBuffer>,
    ) -> ScreenInput<'a> {
        ScreenInput {
            reader,
            ring,
            capture_running: false,
            needs_screen: false,
            required: false,
            read_interval_secs: 3,
            screen_permission: || true,
            shot: || Ok(Some(test_frame())),
        }
    }

    #[tokio::test]
    async fn recording_on_with_vision_uses_cached_context() {
        let reader = screen_read::ScreenReader::new();
        reader.seed_context("ide with errors");
        let ring = Mutex::new(RingBuffer::new(4, 1 << 20));
        let mut input = input(&reader, &ring);
        input.capture_running = true;
        let (_ev, emit) = recorder();
        let vis = candidate("vis", MockProvider::new(vec![]));
        let (mat, _u) =
            resolve_screen(&input, Some(&vis), &emit, &CancellationToken::new())
                .await
                .unwrap();
        let Some(ScreenMaterial::Text(t)) = mat else {
            panic!("expected cached text");
        };
        assert!(t.contains("ide with errors"));
    }

    #[tokio::test]
    async fn recording_on_without_vision_attaches_ring_frame() {
        let reader = screen_read::ScreenReader::new();
        let ring = Mutex::new(RingBuffer::new(4, 1 << 20));
        ring.lock().push(test_frame());
        let mut input = input(&reader, &ring);
        input.capture_running = true;
        let (_ev, emit) = recorder();
        let (mat, _u) =
            resolve_screen(&input, None, &emit, &CancellationToken::new())
                .await
                .unwrap();
        assert!(matches!(mat, Some(ScreenMaterial::Frame(_))));
    }

    #[tokio::test]
    async fn recording_off_no_intent_is_text_only() {
        let reader = screen_read::ScreenReader::new();
        let ring = Mutex::new(RingBuffer::new(4, 1 << 20));
        let input = input(&reader, &ring);
        let (_ev, emit) = recorder();
        let (mat, _u) =
            resolve_screen(&input, None, &emit, &CancellationToken::new())
                .await
                .unwrap();
        assert!(mat.is_none(), "no intent + no recording → nothing attached");
    }

    #[tokio::test]
    async fn recording_off_intent_shot_describes_inline() {
        let reader = screen_read::ScreenReader::new();
        let ring = Mutex::new(RingBuffer::new(4, 1 << 20));
        let mut input = input(&reader, &ring);
        input.needs_screen = true;
        let (_ev, emit) = recorder();
        let vis = candidate(
            "vis",
            MockProvider::new(vec![Behavior::Tokens(vec![
                "screen text".into(),
            ])]),
        );
        let (mat, _u) =
            resolve_screen(&input, Some(&vis), &emit, &CancellationToken::new())
                .await
                .unwrap();
        let Some(ScreenMaterial::Text(t)) = mat else {
            panic!("expected inline read text");
        };
        assert_eq!(t, "screen text");
    }

    #[tokio::test]
    async fn recording_off_required_shot_failure_errors() {
        let reader = screen_read::ScreenReader::new();
        let ring = Mutex::new(RingBuffer::new(4, 1 << 20));
        let mut input = input(&reader, &ring);
        input.needs_screen = true;
        input.required = true;
        input.shot = || Err(anyhow::anyhow!("denied"));
        let (_ev, emit) = recorder();
        let result = resolve_screen(
            &input,
            None,
            &emit,
            &CancellationToken::new(),
        )
        .await;
        assert!(result.is_err());
    }
```

The seams: `shot` is a `fn` pointer (`crate::capture::shot_fullscreen`
in prod, a stub in tests — `spawn_blocking` swallows panics into
`JoinError`, which resolve treats as failure), and `screen_permission`
wraps `crate::permissions::screen_status` so tests don't call macOS.

- [ ] **Step 2:** `cargo test resolve_screen` → FAIL.

- [ ] **Step 3: Implement.**

`ScreenInput` + material enum + resolve (in `ask.rs`):

```rust
/// What the ask chain attaches: the truth table's three outcomes.
pub(crate) enum ScreenMaterial {
    /// Cached or inline-read description → `<screen_context>` text.
    Text(String),
    /// Raw JPEG frame → image part (no vision configured, or the
    /// inline read failed).
    Frame(Frame),
}

/// Everything `resolve_screen` needs — bundled so `send_chain` keeps
/// one param instead of seven. No `AppHandle`: side-effects are injected
/// seams so the truth table is unit-testable.
pub(crate) struct ScreenInput<'a> {
    pub reader: &'a screen_read::ScreenReader,
    pub ring: &'a Mutex<RingBuffer>,
    /// `state.capture` is live — ring frames are fresh.
    pub capture_running: bool,
    /// with_screen || screen_required || looks_like_screen_intent(text)
    pub needs_screen: bool,
    /// screen_only: a failed one-shot errors instead of degrading.
    pub required: bool,
    pub read_interval_secs: u64,
    /// `crate::permissions::screen_status` in prod; stubbed in tests.
    pub screen_permission: fn() -> bool,
    /// `crate::capture::shot_fullscreen` in prod; stubbed in tests.
    pub shot: fn() -> anyhow::Result<Option<Frame>>,
}

/// The screen-material truth table (spec §Ask flow):
/// recording ON  → cached context (vision) / ring frame (no vision);
/// recording OFF + intent → one-shot → inline describe or raw attach;
/// OFF + no intent → None. `Err` = required shot failed → ask:error.
/// `emit` is the ask task's gen-guarded sender — reused for the
/// `capture:permission-needed` broadcast (it targets the bar window,
/// which is the toast's only consumer).
pub(crate) async fn resolve_screen(
    input: &ScreenInput<'_>,
    vision: Option<&ProviderCandidate>,
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
    cancel: &CancellationToken,
) -> Result<(Option<ScreenMaterial>, TokenUsage), String> {
    let mut usage = TokenUsage::default();
    if input.capture_running {
        if vision.is_some() {
            return Ok((
                input.reader.context().map(|c| {
                    let age = unix_now() - c.ts;
                    let text = if age > (input.read_interval_secs * 2) as i64 {
                        format!("{}\n(captured ~{}s ago)", c.text, age)
                    } else {
                        c.text
                    };
                    ScreenMaterial::Text(text)
                }),
                usage,
            ));
        }
        // No vision reader: attach the freshest ring frame. Permission
        // revoked mid-session → drop the stale frame and warn the UI.
        let frame = input.ring.lock().latest();
        if frame.is_some() && !(input.screen_permission)() {
            emit(
                "capture:permission-needed",
                json!({ "permission": "screen" }),
            );
            return Ok((None, usage));
        }
        return Ok((frame.map(ScreenMaterial::Frame), usage));
    }
    if !input.needs_screen {
        return Ok((None, usage));
    }
    // One-shot: SCScreenshotManager is a sync Cocoa call — keep it off
    // the async executor.
    let shot = input.shot;
    let frame = match tokio::task::spawn_blocking(move || shot()).await {
        Ok(Ok(Some(f))) => f,
        Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
            if input.required {
                return Err(
                    "Screenshot failed — check screen permission".into(),
                );
            }
            log::warn!("ask: one-shot unavailable; answering text-only");
            return Ok((None, usage));
        }
    };
    if let Some(vis) = vision {
        let read = crate::screen_read::describe_screen(
            &*vis.provider,
            &frame,
            cancel,
        )
        .await;
        match read {
            Ok(Some(reply)) => {
                if let Some(u) = reply.usage {
                    usage.add(&u);
                }
                return Ok((Some(ScreenMaterial::Text(reply.full)), usage));
            }
            Ok(None) => return Err("cancelled".into()),
            Err(e) => {
                log::warn!("ask: inline read failed ({e}); attach frame");
            }
        }
    }
    Ok((Some(ScreenMaterial::Frame(frame)), usage))
}
```

`kick` changes:

- `send(app, deps, text, with_screen)` →
  `self.kick(app, deps, text, with_screen, false, false)`;
  `send_screen_only` → `(SCREEN_ONLY_PROMPT, true, true, false)`;
  `retry` → `(text, false, false, true)`.
- Delete the `frame = deps.ring.lock().latest()` + permission block +
  `frame_required` pre-flight error (moved into resolve).
- Before spawn:

```rust
        let needs_screen = with_screen
            || screen_required
            || screen_read::looks_like_screen_intent(&text)
            || deps.capture_running;
        let read_interval_secs =
            deps.config.lock().recording.read_interval_secs;
        let reader = Arc::clone(&deps.reader);
        // Deps.ring becomes Arc — see below.
        let ring = Arc::clone(&deps.ring);
        let capture_running = deps.capture_running;
```

  `Deps` update (matches the `db: Arc` idiom — owned clone for the
  spawn):

```rust
pub struct Deps<'a> {
    pub db: Arc<Db>,
    pub ring: Arc<Mutex<RingBuffer>>, // was &'a — Arc'd for the spawn
    pub reader: Arc<screen_read::ScreenReader>,
    pub capture_running: bool,
    pub keystore: &'a Mutex<Keystore>,
    pub config: &'a Mutex<Config>,
    pub pool: &'a Mutex<WindowPool>,
}
```

  `AppState::deps()` fills `ring: Arc::clone(&self.ring)`,
  `reader: Arc::clone(&self.screen_reader)`,
  `capture_running: self.capture.lock().as_ref()
  .is_some_and(MacosCapture::is_running)`.

- Inside the spawned task, replace `frame.as_ref()` with:

```rust
            let screen_input = ScreenInput {
                reader: &reader,
                ring: &ring,
                capture_running,
                needs_screen,
                required: screen_required,
                read_interval_secs,
                screen_permission: crate::permissions::screen_status,
                shot: crate::capture::shot_fullscreen,
            };
            let _ = send_chain(
                candidates,
                db.as_ref(),
                &emit,
                &text,
                &screen_input,
                &cancel,
                fresh_session,
                regenerate,
                &language,
            )
            .await;
```

`send_chain` changes:

- Signature: drop `vision`; `frame` → `screen_input: &ScreenInput`.
- After `emit(EV_STATE loading)` (keeps the bubble appearing before the
  inline read — same ordering as today), replace the vision block with:

```rust
    let resolved =
        match resolve_screen(screen_input, vision.as_ref(), emit, cancel)
            .await
        {
            Ok(r) => r,
            Err(msg) if msg == "cancelled" => {
                emit(EV_STATE, json!({"state": "idle"}));
                return Err(LlmError::Http {
                    status: 0,
                    message: "cancelled".into(),
                });
            }
            Err(msg) => {
                emit(EV_ERROR, json!({ "message": msg }));
                emit(EV_STATE, json!({"state": "idle"}));
                return Err(LlmError::Http { status: 0, message: msg });
            }
        };
    let (frame, screen) = match resolved.0 {
        Some(ScreenMaterial::Text(t)) => (None, Some(t)),
        Some(ScreenMaterial::Frame(f)) => (Some(f), None),
        None => (None, None),
    };
    let mut usage = resolved.1;
```

  Keep `vision` as a `send_chain` param (resolve needs it): signature
  becomes `(candidates, vision, db, emit, text, screen_input, cancel,
  fresh, regen, language)` — pass `vision.as_ref()` into resolve.
  Also add a `fn unix_now() -> i64` helper (same
  `SystemTime::now()…as_secs() as i64` pattern as
  `encode_frame`/`seed_context`) beside `resolve_screen`.

- `stream_candidate`/`build_messages` signatures unchanged
  (`Option<&Frame>`, `Option<&str>`) — pass `frame.as_ref()`,
  `screen.as_deref()`.

`lib.rs` command + call-site updates:

```rust
async fn ask_send(app: AppHandle, text: String, with_screen: Option<bool>) {
    …
    state.ask.send(&app, &deps, &text, with_screen.unwrap_or(false));
}
```

`ask_send_screen_only` unchanged (`send_screen_only` already implies
`with_screen`); deeplink dispatch adds `false` arg.

- [ ] **Step 4:** update existing `send_chain` test call sites for the
  new signature (they currently pass `vision`/`frame` positionally —
  wrap a `ScreenInput` with `needs_screen: false` dummies or a ring).
  `cargo test ask` → green.

- [ ] **Step 5: Commit** — `ask: resolve_screen truth table +
  with_screen flag`.

---

### Task 9: Frontend — Cmd+Enter flag + picker record button

(`commands.ts`, `AskInput.tsx`, `Bar.tsx`, `events.ts`)

**Files:**

- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts`
- Modify: `apps/native/src/components/bar/AskInput.tsx`
- Modify: `apps/native/src/views/Bar.tsx`

- [ ] **Step 1: `commands.ts`** — flag param + new command + target type:

```ts
export const askSend = (text: string, withScreen = false) =>
  invoke<void>('ask_send', { text, withScreen });

/** `capture_status` return — `target` is the picked scope label,
 *  null on the auto primary-display path. */
export type CaptureStatus = {
  running: boolean;
  frames: number;
  target: { kind: 'display' | 'window' | 'app'; label: string } | null;
};

export const capturePickAndStart = () =>
  invoke<CaptureStatus>('capture_pick_and_start');
```

- [ ] **Step 2: `events.ts`** — `EV_CAPTURE_STATE` payload comment gains
  `{ running, frames, target }`.

- [ ] **Step 3: `AskInput.tsx`** — `onSubmit` gains the flag:

```ts
  onSubmit: (withScreen: boolean) => void;
// …
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        onSubmit(e.metaKey || e.ctrlKey);
      }
```

Update the doc comment: `Cmd/Ctrl+Enter` submits with a forced screen
read (plain Enter submits normally).

- [ ] **Step 4: `Bar.tsx`** —

```ts
const sendAsk = (withScreen = false) => {
  // …existing trim/busy/gate guards…
  void askSend(t, withScreen).catch(/* existing */);
};
// AskInput prop: onSubmit={sendAsk}   (sendAsk(withScreen))
// form onSubmit stays → sendAsk() (withScreen defaults false)
```

Record button — replace `toggleCapture`'s start half:

```ts
const pressCapture = () => {
  if (captureRunning) {
    void captureStop().catch(() => {});
  } else {
    void capturePickAndStart().catch(() => {});
  }
};
```

(`toggleCapture` at ~line 367: keep the `wantRunning` early-return
shape if it exists, but route start → `capturePickAndStart`.) Track
`captureTarget` from the `capture:state` payload alongside
`captureRunning` and set the record `BarButton`'s `title` to
`captureTarget?.label` while running (optional badge per spec — one
prop).

- [ ] **Step 5:** `cd apps/native && bun run build` (tsc/vite) — clean.
No vitest setup exists for these components; verification is the type
check + manual `bun run build:dev` pass.

- [ ] **Step 6: Commit** — `bar: Cmd+Enter screen ask + native
  picker record flow`.

---

## Deferred / Out of Scope

- Settings UI for `read_interval_secs` — the TOML/`config_set` knob is
  sufficient; a prefs control can follow if needed.
- Per-target auto-restart when a picked app exits (stream just goes
  quiet — `stop/start` covers it for now).
- `start_paused` tokio tests require tokio's `test-util` feature — if
  it's not already enabled, add `tokio = { features = ["test-util"] }`
  to `[dev-dependencies]` in `src-tauri/Cargo.toml` during Task 5.

## Verification (after all tasks)

```bash
cd apps/native/src-tauri && cargo test            # full suite
cd apps/native && bun run build                   # frontend typecheck/build
bun run build:dev      # manual: record → picker → ask
```

Manual pass checklist: record via picker (window + display), ask with
`Cmd+Enter`, ask "what's on my screen" with recording off, stop
capture mid-read (cancel path), revoke permission mid-session.
