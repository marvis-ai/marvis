# Screen Context Redesign — Design Spec

Date: 2026-09-28 · Status: approved-in-conversation

## Problem

Three issues with today's screen pipeline:

1. **Ask-frames are illegible.** Ambient capture downscales to
   `TARGET_HEIGHT = 384`; on a 3456×2234 display the vision model
   receives a ~594×384 thumbnail — right gist, invented details
   (verified: same model + endpoint reads a controlled image
   perfectly).
2. **Screen context is unconditional.** Every ask grabs
   `ring.latest()` whenever recording happens to be on — no intent
   concept; recording off means no screen at all.
3. **Capture is whole-primary-display only.** No way to scope
   recording to a window or app (e.g. WeChat), which is both a
   capability gap and a privacy gap.

## Goals

| # | Goal |
| --- | --- |
| 1 | Asks take a **fresh full-screen screenshot on demand** when the user asks about the screen (keywords) or presses **Cmd+Enter** — regardless of whether recording is running |
| 2 | The record button opens the **native content-sharing picker** (display / window / application) instead of silently starting a whole-display stream |
| 3 | Encoded frames are **legible** — cap the frame's long edge ≈1600px (from 384px height) |
| 4 | While recording is ON, a background **ScreenReader** describes settled frames and caches `screen_context`, so asks inject it with zero LLM wait |

## Non-goals

- No agentic tool-call loop (no `read_screen` tool for the answering
  model). Intent is deterministic; the design leaves a seam
  (`resolve_screen`) where a classifier/tool-loop could slot later.
- No persisted capture target (picker shows on every manual record
  start; auto-start keeps whole-display default).
- No multi-window aggregation beyond what the picker's Application
  mode gives.
- Windows/Linux — `capture` stays macOS-only.

## Architecture

```text
RECORDING ON (target from native picker: display | window | app)
  SCStream → extract_raw → hash dedupe → downscale ≤1600w → JPEG
      │→ RingBuffer
      └─► ScreenReader task (spawned with capture):
            frame noted → wait settle (~1s quiet)
                          && ≥ read_interval since last read
            → vision_provider.describe(latest frame)
            → cache ScreenContext{text, ts}
            failure → keep previous context (stale beats empty)

ASK (send / Cmd+Enter / screen-only / deeplink)
  resolve_screen():
    recording ON  → cached ScreenContext (any ask; intent too)
    recording OFF + intent/flag/screen-only
                  → ScreenshotManager single-shot
                    → vision describe inline → context text
                    → (no vision) attach raw frame
    recording OFF + no intent → nothing (text-only)
    no vision + recording ON  → attach latest ring frame (fallback)
  → failover chain streams over <screen_context> or attached image
```

### Recording-on + intent ask

Uses the **cached** context (≤ ~settle+interval old by construction)
rather than forcing a fresh inline read — the cache is the point of
ambient mode.

## Components

### `capture/macos.rs`

- Replace `TARGET_HEIGHT: u32 = 384` with a **width cap**
  `TARGET_WIDTH: u32 = 1600` (height follows aspect; smaller sources
  pass through unscaled). 3456×2234 → 1600×1035.
- Factor the `RawFrame → hash → resize → JPEG → Frame` tail of
  `run_worker` into a shared `encode_frame(raw) -> Option<Frame>`
  used by both the stream worker and the single-shot path.
- `MacosCapture::new(fps)` → `MacosCapture::new(filter:
  SCContentFilter, width: u32, height: u32, fps)`.
  `primary_display_filter` stays for the auto-start default and the
  on-demand shot.
- New: `shot_fullscreen() -> Option<Frame>` —
  `SCScreenshotManager::capture_sample_buffer(&primary_display_filter(),
  config-at-capped-dims)` (sync API → call inside `spawn_blocking`)
  → `extract_raw` → `encode_frame`. **Primary display only** (same
  filter as ambient's default); never writes to disk (privacy rule
  preserved).

### `screen_read.rs` (new module)

- `ScreenReader` — AppState share, mirrors `AskService`'s shape:
  - `context: Mutex<Option<ScreenContext>>`, `ScreenContext {
    text: String, ts: i64 }`.
  - `note_frame()` — called from the ring-push closure in
    `start_capture` (each changed frame).
  - Internal `tokio` task: waits until the screen is settled
    (`SETTLE_MS ≈ 1s` since last `note_frame`) **and**
    `recording.read_interval_secs` (default `3`) since the last read,
    then resolves `vision_candidate` fresh (config may change
    mid-session) and runs `describe_screen`.
  - `stop()` on `stop_capture`/`leave_main`; starts alongside
    `start_capture`.
  - `describe_screen` moves here from `ask.rs` (`pub(crate)`) — both
    the reader and the inline on-demand path call it.
  - `looks_like_screen_intent(text) -> bool` — pure keyword matcher
    (en + zh baseline: `screen`, `屏幕`, `截图`, `what's on my`…),
    unit-tested; single seam for a future classifier upgrade.

### `ask.rs`

- `Deps` gains `reader: &'a ScreenReader` (context cache) — or the
  cache mutex directly.
- `AskService::send` signature gains `with_screen: bool` (Cmd+Enter);
  `send_screen_only` folds into the same path (`with_screen = true`,
  `frame_required` → "shot must succeed").
- `kick` pre-flight unchanged except the `needs_screen` decision:
  `with_screen || looks_like_screen_intent(text) ||
  capture_is_running` (recording ON feeds every ask — via cached
  context when vision is configured, else via the latest ring frame;
  `reader.has_context()` alone would starve the no-vision fallback).
- Inside the spawned task, `resolve_screen` implements the truth
  table:

  | Recording | Intent? | Vision configured? | Result |
  | --- | --- | --- | --- |
  | ON | — | yes | cached `screen_context` (+ age) |
  | ON | — | no | latest ring `Frame` attached raw |
  | OFF | yes/flag | yes | fresh shot → inline describe → context |
  | OFF | yes/flag | no | fresh shot attached raw |
  | OFF | no | — | text-only |

- `screen_prompt` usage unchanged; `<screen_context>` gains a
  `(captured Ns ago)` hint when the cache is older than ~2× interval
  (small `prompts.rs` touch).
- Shot failure on an intent ask → `ask:error{"screenshot failed —
  check screen permission"}` + `idle` (screen-only semantics extended
  to the explicit path).

### `lib.rs`

- `AppState` gains `screen: Arc<ScreenReader>`; `Deps` extended;
  `state.deps()` updated.
- `start_capture(app, filter, w, h)` — signature takes the chosen
  filter; `enter_main` auto-start keeps `primary_display_filter`
  (picker can't appear unprompted on gate transition).
- New command `capture_pick_and_start` (same `Gate::Main` guard as
  `capture_start`): `run_on_main_thread` → `SCContentSharingPicker::
  show` with modes `[SingleWindow, SingleDisplay, Application]`,
  `set_excluded_bundle_ids` = our bundle id; callback marshals the
  picked `result.filter()` + `pixel_size()` back to `start_capture`.
- `capture:state` payload gains `target: {kind, label} | null` (from
  `SCPickedSource`) so the bar can badge what's being watched.
- `ask_send` command signature: `ask_send(text, with_screen: bool)`;
  deeplink ask passes `false`.

### Frontend (`apps/native/src`)

- `commands.ts`: `askSend(text, withScreen = false)`; new
  `capturePickAndStart()`.
- `Bar.tsx`: pill input — `Cmd+Enter` submits with
  `with_screen = true` (plain Enter unchanged); record button (idle)
  → `capturePickAndStart()` instead of `captureStart()`; stop path
  unchanged.
- Optional polish (in scope, small): badge the active capture target
  from `capture:state.target`; hint `Cmd+Enter ⌘⏎` in the pill
  placeholder.
- `askSendScreenOnly` stays (now backed by the single-shot path);
  wiring a camera affordance is out of scope.

### `config.rs`

- `recording.read_interval_secs: u64` — default `3`, min clamp `1`.
  (`SETTLE_MS` stays a code constant — not user-facing.)

## Error & edge semantics

| Case | Behavior |
| --- | --- |
| Picker cancelled | No-op; capture stays off |
| Picked window/app closed mid-record | Stream stops producing; context goes stale (age surfaces in prompt); `capture:state` resync on next boundary |
| Background read fails | Keep previous context, warn-log |
| Single-shot fails (permission etc.) | Intent asks → `ask:error`; ambient unaffected |
| No `[vision]` provider | Raw-frame fallbacks as in truth table |
| Permission revoked mid-session | Existing `capture:permission-needed` path |

## Testing

- `screen_read`: intent matcher table-test (en/zh/misses); settle +
  interval gate with `tokio::time::pause`; failure-keeps-cache; stop
  aborts in-flight read.
- `ask`: extend existing `send_chain` tests — intent flag on/off,
  recording-on cache path, recording-off single-shot path (factor
  `resolve_screen` for a `Frame` supplier seam), vision-absent
  fallbacks.
- `capture`: `encode_frame` unit test (BGRA → JPEG dims under/over
  cap); filter construction is the only untestable seam (needs macOS
  objects) — keep it thin.

## Privacy

Pixels still never hit disk or JS. Net privacy posture **improves**:
scoped capture means only the chosen window/app's pixels enter the
ring, and with the reader configured, chat providers receive text
only. Note the real trade-off for docs: recording ON now means screen
pixels go to the vision provider continuously (throttled), not just
at ask time.
