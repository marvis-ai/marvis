# Ask Input Dictation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the mic button two explicit behaviors: dictation into the visible Ask input and persistent meeting Listen from the collapsed idle bar.

**Architecture:** Add a dedicated Rust `DictationService` that reuses `MicSource`, the configured `SttProvider`, and bundled `whisper-cli` resolution. Dictation emits a normalized live draft to the `bar` window without creating transcript, summary, or session rows. `Bar.tsx` routes the mic button by UI state and replaces only the tracked dictated input range. Existing `ListenService` remains responsible for meeting capture and persistence.

**Tech Stack:** Rust, Tauri 2 commands/events, cpal microphone capture, Deepgram/whisper-cli STT adapters, React 19, TypeScript, Bun tests.

**Approved spec:** `docs/superpowers/specs/2026-09-23-dictation-mode-design.md`

## Global Constraints

- Input-visible mic press starts dictation; collapsed idle-bar mic press starts meeting Listen.
- Dictation uses microphone input only; it never starts `SystemAudioSource`.
- Dictation is transient and must not create `sessions`, `transcripts`, or `summaries` rows.
- Dictated text inserts at the caret; a selected input range is replaced.
- Live draft updates replace only the tracked dictated range and preserve prefix/suffix text.
- Stop leaves text in the input for review; dictation never auto-submits.
- Dictation and meeting Listen are mutually exclusive.
- Use the configured STT provider/model and bundled-first Whisper resolution.
- Events must not expose binary paths, provider keys, model URLs, raw audio, or process output.
- Do not change Deepgram/Whisper provider behavior beyond reuse through the existing `SttProvider` trait.
- Do not refactor unrelated Listen, Ask, window, or settings code.

---

## File Map

### Rust files to create or modify

- Create: `apps/native/src-tauri/src/dictation.rs` — transient draft assembler, service state, microphone/STT orchestration, tests.
- Modify: `apps/native/src-tauri/src/lib.rs` — `mod dictation`, `AppState.dictation`, commands, event emission, handler registration, mutual exclusion.
- Modify: `apps/native/src-tauri/src/stt/whisper.rs` or `apps/native/src-tauri/src/stt/mod.rs` — only if the existing Whisper setup check needs a shared helper.
- Modify: `apps/native/src-tauri/src/listen.rs` — only if moving the existing Whisper setup error helper avoids duplication.

### Frontend files to create or modify

- Create: `apps/native/src/lib/dictation.ts` — pure dictated-range update/edit helpers.
- Create: `apps/native/src/lib/dictation.test.ts` — Bun tests for insertion, selection replacement, and edit-range rules.
- Modify: `apps/native/src/lib/commands.ts` — dictation command wrappers/types.
- Modify: `apps/native/src/lib/events.ts` — `dictation:*` event constants/payload types.
- Modify: `apps/native/src/views/Bar.tsx` — mode-aware mic routing, live input insertion, Stop/collapse/edit lifecycle.

---

## Task 1: Add the transient dictation draft model

**Files:**

- Create: `apps/native/src-tauri/src/dictation.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` — add only `mod dictation;` so the new test module compiles
- Test: `dictation.rs` unit tests

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictationStatus {
    pub state: String, // "idle" | "listening" | "error"
    pub provider: Option<String>,
    pub error: Option<DictationError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictationError {
    pub message: String,
    pub needs_setup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictationDraft {
    pub text: String,
    #[serde(rename = "final")]
    pub finality: bool,
}
```

Use a small internal draft assembler rather than meeting `TurnAssembler` so
dictation has no turn closing, persistence cadence, or speaker switching:

```text
committed + provisional → current draft
Interim event → replace provisional
Final event → append to committed and clear provisional
finish() → committed + provisional, then reset
```

- [ ] **Step 1: Write failing draft assembler tests.** Create `dictation.rs` with the intended `DraftAssembler` API under test, add `mod dictation;` in `lib.rs`, and cover interim replacement, final append, repeated finals, empty text, and `finish()` returning the complete draft once.
- [ ] **Step 2: Run focused tests and verify failure.**

```bash
cd apps/native/src-tauri
cargo test dictation -- --nocapture
```

Expected: FAIL because `dictation.rs` does not exist.

- [ ] **Step 3: Implement the draft assembler and serializable DTOs.** Keep all text handling local to the service; do not persist or emit raw STT internals.
- [ ] **Step 4: Run focused tests.**

```bash
cargo test dictation -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add apps/native/src-tauri/src/dictation.rs apps/native/src-tauri/src/lib.rs
git commit -m "feat(native): add dictation draft model"
```

---

## Task 2: Implement DictationService orchestration and commands

**Files:**

- Modify: `apps/native/src-tauri/src/dictation.rs`
- Modify: `apps/native/src-tauri/src/lib.rs`
- Modify: `apps/native/src-tauri/src/stt/whisper.rs` or `stt/mod.rs` only if needed for shared Whisper setup validation
- Test: `dictation.rs`, `lib.rs` registration/mutual-exclusion tests

**Interfaces:**

```rust
pub enum DictationEvent {
    Draft(DictationDraft),
    Error { message: String, needs_setup: bool },
}

pub struct DictationService { /* status + running worker */ }

impl DictationService {
    pub fn status(&self) -> DictationStatus;
    pub fn start(
        &self,
        keystore: &Keystore,
        config: &Config,
        mic_allowed: bool,
        bundled_whisper: Option<&Path>,
        emit: Arc<dyn Fn(DictationEvent) + Send + Sync>,
    ) -> anyhow::Result<()>;
    pub fn stop(&self) -> DictationDraft;
}
```

Commands/events:

```text
dictation_start  -> DictationStatus
dictation_stop   -> DictationDraft
dictation_status -> DictationStatus

dictation:state  -> DictationStatus
dictation:draft  -> DictationDraft
dictation:error  -> DictationError
```

- [ ] **Step 1: Write failing service/command tests.** Cover:
  - `dictation_start` rejects inactive gate and missing mic permission.
  - `dictation_start` rejects while `ListenStatus.state == "listening"`.
  - `listen_start` rejects while `DictationStatus.state == "listening"`.
  - `dictation_stop` is idempotent and returns the final draft.
  - Whisper setup uses the same bundled-aware binary/model validation as Listen.
  - Extend the existing command-contract source test so `dictation_start`, `dictation_stop`, and `dictation_status` must be registered in `invoke_handler`.
- [ ] **Step 2: Run focused tests and verify failure.**

```bash
cd apps/native/src-tauri
cargo test dictation -- --nocapture
cargo test command_contract -- --nocapture
```

Expected: FAIL before the service/commands exist.

- [ ] **Step 3: Implement `DictationService::start`.**
  - Stop any previous dictation run.
  - Require `mic_allowed`; do not start system audio.
  - Resolve the Deepgram key only when `stt_provider == "deepgram"`.
  - Reuse the existing Whisper setup validation or extract it to a shared helper so bundled/model checks stay identical to Listen.
  - Start `MicSource` → `mpsc` → `make_stt_provider(..., SpeakerChannel::Me, bundled_whisper)`.
  - Update status to `listening` only after both source and provider start successfully.
  - Sanitize provider errors before emitting `dictation:error`.
- [ ] **Step 4: Implement worker/stop lifecycle.** The worker forwards PCM to STT, exits on cancellation/disconnect, then stops provider/source. `stop()` joins the worker, finalizes the draft once, resets state to idle, and returns the final draft.
- [ ] **Step 5: Add Tauri wiring.** Add `AppState.dictation`, event constants/emission, the three commands, mutual-exclusion checks in `dictation_start`/`listen_start`, and `invoke_handler` registration. Extend app-exit cleanup so it stops dictation as well as meeting Listen. Preserve the existing comment explaining Listen's setup-failure event contract; use the same pattern for dictation.
- [ ] **Step 6: Run Rust checks.**

```bash
cargo test dictation -- --nocapture
cargo test listen -- --nocapture
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo check
```

- [ ] **Step 7: Commit.**

```bash
git add apps/native/src-tauri/src/dictation.rs apps/native/src-tauri/src/lib.rs apps/native/src-tauri/src/stt/whisper.rs apps/native/src-tauri/src/stt/mod.rs apps/native/src-tauri/src/listen.rs
git commit -m "feat(native): add dictation speech service"
```

---

## Task 3: Add frontend dictation contracts and insertion helpers

**Files:**

- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts`
- Create: `apps/native/src/lib/dictation.ts`
- Create: `apps/native/src/lib/dictation.test.ts`
- Test: `bun test apps/native/src/lib/dictation.test.ts`

**Interfaces:**

```ts
export interface DictationStatus {
  state: 'idle' | 'listening' | 'error';
  provider: string | null;
  error: DictationErrorPayload | null;
}

export interface DictationDraftPayload {
  text: string;
  final: boolean;
}
```

Pure helper contract:

```ts
export interface DictationRange {
  start: number;
  length: number;
}

export const applyDictationDraft = (
  input: string,
  range: DictationRange,
  draft: string,
) => {
  value: string;
  caret: number;
  range: DictationRange;
};
```

Add a second helper only as needed to detect whether a user edit intersects the
live dictated range and to shift the range for edits before it.

- [ ] **Step 1: Write failing helper tests.** Cover caret insertion, selected-range replacement, repeated live updates, prefix/suffix preservation, empty draft, edits before the dictated range, and edits inside the dictated range being reported as a stop condition.
- [ ] **Step 2: Run focused tests and verify failure.**

```bash
cd apps/native
bun test src/lib/dictation.test.ts
```

Expected: FAIL because the helper does not exist.

- [ ] **Step 3: Implement `dictation.ts` helpers without React or Tauri dependencies.** Keep them deterministic and easy to unit test.
- [ ] **Step 4: Add commands/events.** Add `dictationStart`, `dictationStop`, `dictationStatus`, `EV_DICTATION_STATE`, `EV_DICTATION_DRAFT`, `EV_DICTATION_ERROR`, and payload types. Mirror the existing `listen:*` naming/serialization style.
- [ ] **Step 5: Run focused checks.**

```bash
cd apps/native
bun test src/lib/dictation.test.ts
bun x tsc --noEmit
```

- [ ] **Step 6: Commit.**

```bash
git add apps/native/src/lib/commands.ts apps/native/src/lib/events.ts apps/native/src/lib/dictation.ts apps/native/src/lib/dictation.test.ts
git commit -m "feat(native): add dictation frontend contracts"
```

---

## Task 4: Route the mic button by mode and update the Ask input

**Files:**

- Modify: `apps/native/src/views/Bar.tsx`
- Test: `bun test` plus focused manual verification

**Behavior:**

- `dictationState === 'listening'` → `dictation_stop`.
- `listenState === 'listening'` → `listen_stop`.
- `showInputRow` with no active speech session → `dictation_start`.
- collapsed/no input with no active speech session → existing `windowSetChatOpen(true)` + `listen_start`.
- Existing `listenBusy` protection must cover both modes or become a shared `speechBusy` ref.

- [ ] **Step 1: Write a failing frontend assertion/test for mode routing if practical.** If DOM testing infrastructure is absent, keep this as a documented manual check and rely on the pure insertion tests plus TypeScript.
- [ ] **Step 2: Add dictation state/resync.** On mount, call `dictationStatus`; subscribe to `dictation:state`, `dictation:draft`, and `dictation:error`.
- [ ] **Step 3: Implement tracked input insertion.** On start, capture `selectionStart`/`selectionEnd`; on each draft, call `applyDictationDraft`; keep the caret after the dictated text. Do not modify text outside the tracked range.
- [ ] **Step 4: Implement lifecycle edge cases.**
  - Manual edit intersecting the dictated range → stop dictation and preserve the user's edit.
  - Edit before the range → shift the tracked insertion anchor.
  - `showInputRow` becomes false → stop dictation.
  - Submit while dictation is active → stop dictation and do not send on that keypress; the next Enter submits the reviewed text.
  - Stop response applies the authoritative final draft once.
- [ ] **Step 5: Update mic accessibility labels.** Use `Dictate`, `Stop dictation`, `Listen`, or `Stop listening` according to the next action while retaining the existing active styling.
- [ ] **Step 6: Run frontend checks.**

```bash
cd apps/native
bun x tsc --noEmit
bun test --pass-with-no-tests
bun run build
```

- [ ] **Step 7: Commit.**

```bash
git add apps/native/src/views/Bar.tsx
git commit -m "feat(native): dictate into Ask input"
```

---

## Task 5: End-to-end verification and hardening

**Files:**

- Modify only files needed by test/review findings.
- Test: full Rust/frontend/package checks and local manual smoke.

- [ ] **Step 1: Run full automated verification.**

```bash
cd apps/native/src-tauri
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo check
cd ../..
bun run build
bun test --pass-with-no-tests
git diff --check
```

- [ ] **Step 2: Verify dev staging and bundled binary.**

```bash
cd apps/native
bash src-tauri/scripts/build-marvis.sh --dev-stage --target aarch64-apple-darwin
bash src-tauri/scripts/check-task-2-packaging.sh --target aarch64-apple-darwin
file src-tauri/target/debug/whisper-cli-aarch64-apple-darwin
otool -L src-tauri/target/debug/whisper-cli-aarch64-apple-darwin
```

Expected: arm64 Mach-O, no `libwhisper`/`libggml` dynamic dependencies.

- [ ] **Step 3: Manual Whisper dictation smoke.** Run `bun run build:dev`, open the Ask input, press Dictate, speak, and confirm live chunks appear in the input. Stop and confirm the text remains editable and does not submit automatically.
- [ ] **Step 4: Manual meeting Listen regression.** Collapse to the idle capsule, press Listen, and confirm `ListenSection` still receives/persists `me`/`them` turns without dictating into the input.
- [ ] **Step 5: Review the diff.** Check event names, serialization (`final`), setup errors, no persistence writes, no binary paths/secrets in events, and no unrelated changes.
- [ ] **Step 6: Commit any hardening fixes.**

```bash
git add apps/native/src-tauri/src/dictation.rs apps/native/src-tauri/src/lib.rs apps/native/src/views/Bar.tsx apps/native/src/lib/dictation.ts apps/native/src/lib/dictation.test.ts apps/native/src/lib/commands.ts apps/native/src/lib/events.ts
git commit -m "fix(native): harden dictation mode"
```

---

## Known Risks

1. **Whisper cadence:** `whisper-cli` updates only after each ~3-second chunk; this is expected and not a frontend bug.
2. **Caret tracking:** User edits inside the live dictated range can conflict with incoming drafts; the design resolves this by stopping dictation and preserving the edit.
3. **Process failures:** A dynamically linked or missing `whisper-cli` must remain a setup/runtime error, not a panic. The bundled binary is now validated as self-contained.
4. **Mutual exclusion:** The backend must enforce the invariant even if a stale frontend sends a conflicting command.
