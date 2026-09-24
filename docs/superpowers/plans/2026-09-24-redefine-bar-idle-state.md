# Redefine the Bar Idle State Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the native floating bar show explicit screen-capture, Listen recording, dictation, and background-work states with the requested collapsed/expanded controls and animations.

**Architecture:** Reuse the existing `MacosCapture` frame source, but centralize capture lifecycle operations in idempotent native helpers used by automatic gate transitions and new Tauri commands. Synchronize capture and Ask activity into `Bar.tsx`, keep Listen visible only in the collapsed row, and keep dictation visible only in the expanded row. Use typed events and existing Tailwind/CSS animation conventions rather than introducing a new state library.

**Tech Stack:** React 19, TypeScript 6, Bun tests, Tauri 2, Rust 2021, ScreenCaptureKit, Tailwind CSS v4, `@marvis/ui`, lucide-react.

## Global Constraints

- Use `bun` for package management and scripts.
- Use arrow functions for React components, hooks, and context providers.
- Use named exports for non-UI components in `src/components/*`.
- Keep `page.tsx` files as Server Components; this work is in the native app and does not add a page directive.
- Import/export Lucide icons only through `@marvis/ui` using `Icon`-suffixed names.
- Preserve existing Ask, Listen, and dictation backend semantics unless this plan explicitly changes the bar affordance.
- Do not change the bar's glass, positioning, dimensions, or window-growth behavior.
- All animation changes must respect `prefers-reduced-motion: reduce`.
- Screen capture remains enabled by default after screen permission and main-gate entry.
- The screen control toggles capture only and never submits an Ask request.
- The expanded input row contains only `MicIcon` and `SettingsIcon` as auxiliary icons; `MonitorDotIcon` and `MicAudioLinesIcon` are collapsed-only.

---

### Task 1: Add the icon surface and typed capture contract

**Files:**

- Modify: `packages/ui/src/index.ts:22-43`
- Modify: `apps/native/src/lib/commands.ts:436-455`
- Modify: `apps/native/src/lib/events.ts:90-98`
- Test: `apps/native/src/lib/bar-state.test.ts` (create)

**Interfaces:**

- Produces `MonitorDotIcon` and `MicAudioLinesIcon` from `@marvis/ui`.
- Produces `CaptureStatus { running: boolean; frames: number }` and commands `captureStart(): Promise<CaptureStatus>`, `captureStop(): Promise<CaptureStatus>`, `captureStatus(): Promise<CaptureStatus>`.
- Produces `EV_CAPTURE_STATE = 'capture:state'` and `CaptureStatePayload { running: boolean; frames: number }`.
- Produces a pure `hasActiveWork` helper that accepts Ask, capture, Listen, and dictation states and returns a boolean; this keeps the animation derivation testable without mounting Tauri.

- [ ] **Step 1: Write the failing state-derivation tests**

Create `apps/native/src/lib/bar-state.test.ts` with tests that encode the user-visible rule:

```ts
import { describe, expect, test } from 'bun:test';
import { hasActiveWork } from './bar-state';

describe('hasActiveWork', () => {
  test('is false when the bar is idle', () => {
    expect(
      hasActiveWork({
        ask: 'idle',
        captureRunning: false,
        listen: 'idle',
        dictation: 'idle',
      }),
    ).toBe(false);
  });

  test('is true for each active background or recording state', () => {
    for (const state of [
      { ask: 'loading', captureRunning: false, listen: 'idle', dictation: 'idle' },
      { ask: 'streaming', captureRunning: false, listen: 'idle', dictation: 'idle' },
      { ask: 'idle', captureRunning: true, listen: 'idle', dictation: 'idle' },
      { ask: 'idle', captureRunning: false, listen: 'listening', dictation: 'idle' },
      { ask: 'idle', captureRunning: false, listen: 'idle', dictation: 'listening' },
    ] as const) {
      expect(hasActiveWork(state)).toBe(true);
    }
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `bun test apps/native/src/lib/bar-state.test.ts`

Expected: FAIL because `./bar-state` and `hasActiveWork` do not exist.

- [ ] **Step 3: Add the icon exports and frontend capture contract**

Add `MicAudioLinesIcon` and `MonitorDotIcon` to the lucide export list in `packages/ui/src/index.ts`, next to the existing suffixed icon exports. In `commands.ts`, retain `CaptureStatus` and add:

```ts
export const captureStart = () =>
  invoke<CaptureStatus>('capture_start');
export const captureStop = () =>
  invoke<CaptureStatus>('capture_stop');
export const captureStatus = () =>
  invoke<CaptureStatus>('capture_status');
```

In `events.ts`, add:

```ts
export const EV_CAPTURE_STATE = 'capture:state';
export interface CaptureStatePayload {
  running: boolean;
  frames: number;
}
```

Create `apps/native/src/lib/bar-state.ts`:

```ts
export type AskActivity = 'idle' | 'loading' | 'streaming';
export type SpeechActivity = 'idle' | 'listening' | 'error';

export interface BarActivityState {
  ask: AskActivity;
  captureRunning: boolean;
  listen: SpeechActivity;
  dictation: SpeechActivity;
}

export const hasActiveWork = ({
  ask,
  captureRunning,
  listen,
  dictation,
}: BarActivityState) =>
  ask === 'loading' ||
  ask === 'streaming' ||
  captureRunning ||
  listen === 'listening' ||
  dictation === 'listening';
```

- [ ] **Step 4: Run the focused test to verify it passes**

Run: `bun test apps/native/src/lib/bar-state.test.ts`

Expected: PASS with both tests passing.

- [ ] **Step 5: Commit the contract and pure state helper**

```bash
git add packages/ui/src/index.ts apps/native/src/lib/commands.ts apps/native/src/lib/events.ts apps/native/src/lib/bar-state.ts apps/native/src/lib/bar-state.test.ts
git commit -m "feat: define bar capture and activity contracts"
```

---

### Task 2: Centralize native capture start/stop and expose Tauri commands

**Files:**

- Modify: `apps/native/src-tauri/src/lib.rs:257-309, 1418-1427, 1790-1810`
- Modify: `apps/native/src-tauri/src/capture/mod.rs:10-14, 128-137`
- Create: `apps/native/src-tauri/src/capture/controller.rs`
- Test: `apps/native/src-tauri/src/capture/controller.rs`

**Interfaces:**

- Consumes `AppState.capture`, `AppState.ring`, `MacosCapture::new`, and `FrameSource::start/stop`.
- Produces idempotent `capture_start` and `capture_stop` Tauri commands returning `serde_json::Value` with `{ running, frames }`.
- Produces `capture:state` events through the existing `AppHandle::emit` path.
- Automatic `enter_main`, `leave_main`, permission revocation, and app teardown all use the same lifecycle helpers.

- [ ] **Step 1: Write the failing Rust lifecycle tests**

Create `apps/native/src-tauri/src/capture/controller.rs` with tests that reference a not-yet-defined pure lifecycle decision type. Do not add production implementation in this red step:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_is_idempotent_when_capture_is_running() {
        let mut state = CaptureLifecycle::default();
        assert_eq!(state.start_decision(), StartDecision::Create);
        state.mark_running();
        assert_eq!(state.start_decision(), StartDecision::Noop);
    }

    #[test]
    fn stop_is_idempotent_when_capture_is_stopped() {
        let mut state = CaptureLifecycle::default();
        assert_eq!(state.stop_decision(), StopDecision::Noop);
        state.mark_running();
        assert_eq!(state.stop_decision(), StopDecision::Stop);
    }
}
```

```rust
#[test]
fn start_is_idempotent_when_capture_is_running() {
    let mut state = CaptureLifecycle::default();
    assert_eq!(state.start_decision(), StartDecision::Create);
    state.mark_running();
    assert_eq!(state.start_decision(), StartDecision::Noop);
}

#[test]
fn stop_is_idempotent_when_capture_is_stopped() {
    let mut state = CaptureLifecycle::default();
    assert_eq!(state.stop_decision(), StopDecision::Noop);
    state.mark_running();
    assert_eq!(state.stop_decision(), StopDecision::Stop);
}
```

- [ ] **Step 2: Run the focused Rust test to verify it fails**

Run: `cargo test --manifest-path apps/native/src-tauri/Cargo.toml capture`

Expected: FAIL because the lifecycle abstraction/tests do not exist yet.

- [ ] **Step 3: Implement one shared capture lifecycle boundary**

Implement the controller used by the focused tests in `apps/native/src-tauri/src/capture/controller.rs`:

```rust
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CaptureLifecycle {
    running: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StartDecision {
    Create,
    Noop,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StopDecision {
    Stop,
    Noop,
}

impl CaptureLifecycle {
    pub(crate) fn start_decision(&self) -> StartDecision {
        if self.running { StartDecision::Noop } else { StartDecision::Create }
    }

    pub(crate) fn stop_decision(&self) -> StopDecision {
        if self.running { StopDecision::Stop } else { StopDecision::Noop }
    }

    pub(crate) fn mark_running(&mut self) { self.running = true; }
    pub(crate) fn mark_stopped(&mut self) { self.running = false; }
}
```

Then refactor the body currently in `enter_main` into a helper with this responsibility:

```rust
fn start_capture(app: &AppHandle) -> CaptureStatus
```

The helper must:

1. Lock `state.capture`.
2. Return the current status if a capture exists and is running.
3. Construct `MacosCapture::new()` when no capture exists.
4. Start it with the existing callback that pushes frames into `state.ring`.
5. Store it only when `is_running()` is true.
6. Return the current running flag and frame count.

Add a corresponding:

```rust
fn stop_capture(app: &AppHandle) -> CaptureStatus
```

It must take the capture out of the mutex before calling `stop()`, matching the existing deadlock avoidance in `leave_main`, then return the status.

Add `mod controller;` to `capture/mod.rs` so the decision contract is compiled with the capture module. Emit `capture:state` after each command and automatic transition. Use the existing warning/error logging style and do not serialize frame bytes.

- [ ] **Step 4: Add the Tauri commands and reuse them from gate transitions**

Add commands named `capture_start` and `capture_stop` that call the shared helpers and return the status object. Keep `capture_status` as the read-only status command. Register all three in `tauri::generate_handler!`.

Update `enter_main` to call the start helper. Update `leave_main` to call the stop helper. Keep the existing ask cancellation, dictation stop, alert hiding, and gate behavior unchanged.

- [ ] **Step 5: Run the focused Rust tests and formatting**

Run:

```bash
cargo fmt --manifest-path apps/native/src-tauri/Cargo.toml -- --check
cargo test --manifest-path apps/native/src-tauri/Cargo.toml capture
```

Expected: formatting check passes and capture tests pass without requiring a live macOS capture session.

- [ ] **Step 6: Commit the native lifecycle boundary**

```bash
git add apps/native/src-tauri/src/lib.rs apps/native/src-tauri/src/capture
 git commit -m "feat: expose idempotent screen capture controls"
```

---

### Task 3: Separate collapsed Listen controls from expanded dictation controls

**Files:**

- Modify: `apps/native/src/views/Bar.tsx:37-49, 167-190, 748-1043`
- Modify: `apps/native/src/index.css:85-180`
- Test: `apps/native/src/lib/bar-state.test.ts` (extend pure behavior coverage)

**Interfaces:**

- Consumes `captureStart`, `captureStop`, `captureStatus`, `EV_CAPTURE_STATE`, `CaptureStatePayload`, and `hasActiveWork` from Task 1.
- Consumes native `capture_start`, `capture_stop`, and lifecycle event behavior from Task 2.
- Produces a bar with collapsed Iris + MonitorDot + MicAudioLines controls, and expanded Iris + input + Mic + Settings controls.

- [ ] **Step 1: Add failing pure tests for visibility and labels**

Extend `bar-state.test.ts` with a pure control-layout helper test. The helper should expose the intended layout without requiring a Tauri window:

```ts
test('shows Listen only while collapsed and dictation only while expanded', () => {
  expect(barControls(false)).toEqual(['iris', 'capture', 'listen']);
  expect(barControls(true)).toEqual(['iris', 'dictation', 'settings']);
});
```

Add `barControls(expanded: boolean)` to `bar-state.ts` with the exact return type `BarControl[]`, where `BarControl = 'iris' | 'capture' | 'listen' | 'dictation' | 'settings'`.

- [ ] **Step 2: Run the focused test to verify it fails**

Run: `bun test apps/native/src/lib/bar-state.test.ts`

Expected: FAIL because `barControls` does not exist.

- [ ] **Step 3: Implement the pure layout helper and bar state**

Implement:

```ts
export type BarControl =
  | 'iris'
  | 'capture'
  | 'listen'
  | 'dictation'
  | 'settings';

export const barControls = (expanded: boolean): BarControl[] =>
  expanded
    ? ['iris', 'dictation', 'settings']
    : ['iris', 'capture', 'listen'];
```

In `Bar.tsx`:

1. Import `MonitorDotIcon`, `MicAudioLinesIcon`, and `MicIcon`; remove `CameraIcon`.
2. Import `captureStart`, `captureStop`, `captureStatus`, `EV_CAPTURE_STATE`, `CaptureStatePayload`, `hasActiveWork`, and the Ask activity type.
3. Add `captureRunning` state initialized to `false` and `askState` initialized to `'idle'`.
4. On mount, call `captureStatus()` and update `captureRunning`; surface only user-actionable failures through the existing `raise` helper.
5. Subscribe to `EV_CAPTURE_STATE` and update `captureRunning`.
6. Update the existing `EV_ASK_STATE` listener to set `askState` on every `loading`, `streaming`, and `idle` event while preserving the existing Listen-to-chat switch on `loading`.
7. Derive `hasActiveWork({ ask: askState, captureRunning, listen: listenState, dictation: dictationState })` and apply `animate-pulse` to the floating bar's outer class only while true.

Replace the current screen button behavior with a capture toggle:

```tsx
const toggleCapture = () => {
  if (busy) return;
  setBusy(true);
  const transition = captureRunning ? captureStop() : captureStart();
  void transition
    .then((next) => setCaptureRunning(next.running))
    .catch(() => raise(captureRunning ? 'Screen recording stop failed' : 'Screen recording start failed'))
    .finally(() => setBusy(false));
};
```

Render this button only when `!showInputRow`, with `MonitorDotIcon`, `aria-pressed={captureRunning}`, a state-specific label/title, accent highlight while active, and an absolutely positioned ping element so the icon itself remains legible.

Keep the collapsed Listen button visible only when `!showInputRow`, using `MicAudioLinesIcon` and the existing Listen start/stop logic. Remove the old dual-purpose `MicIcon` button from the collapsed row.

When `showInputRow` is true, render only:

- The existing Iris/back control.
- The textarea.
- The dictation `MicIcon` button with the existing dictation start/stop logic.
- The existing settings button.

The expanded row must not render either `MonitorDotIcon` or `MicAudioLinesIcon`. Preserve the current dictation range and pending-stop code; move only its button affordance to the expanded-only `MicIcon`.

- [ ] **Step 4: Add the dictation waveform and reduced-motion wiring**

Use a compact inline waveform next to the expanded `MicIcon` while `dictationState === 'listening'`, using five narrow spans with `animate-waveform` and staggered delays. Add a selector or utility rule in `index.css` so the waveform is hidden or still under reduced motion without affecting layout. Reuse `--animate-waveform` and `@keyframes mv-waveform`; do not create a second waveform animation.

Add `animate-pulse` to the bar's outer floating shell, not the form row, so row sizing and control placement do not change. Ensure the capture ping is suppressed under the existing reduced-motion media query.

- [ ] **Step 5: Run the frontend tests and typecheck**

Run:

```bash
bun test apps/native/src/lib/bar-state.test.ts apps/native/src/lib/dictation.test.ts
bun run --filter @marvis/native check-types
```

Expected: all focused tests pass and TypeScript reports no errors.

- [ ] **Step 6: Commit the bar interaction changes**

```bash
git add apps/native/src/views/Bar.tsx apps/native/src/index.css apps/native/src/lib/bar-state.ts apps/native/src/lib/bar-state.test.ts
git commit -m "feat: redefine floating bar recording controls"
```

---

### Task 4: Verify the integrated native app and regression surface

**Files:**

- Modify only if verification exposes an issue: files from Tasks 1-3.
- Test: existing native TypeScript/Rust test suites and build scripts.

**Interfaces:**

- Consumes the complete capture command/event contract and Bar UI from Tasks 1-3.
- Produces verified behavior for default-on capture, collapsed/expanded controls, independent recording modes, and activity animation.

- [ ] **Step 1: Run the complete native frontend test suite**

Run:

```bash
bun run --filter @marvis/native test
```

Expected: Bun exits with status 0; existing dictation tests and the new bar-state tests pass.

- [ ] **Step 2: Run the complete Rust test suite and formatting check**

Run:

```bash
cargo fmt --manifest-path apps/native/src-tauri/Cargo.toml -- --check
cargo test --manifest-path apps/native/src-tauri/Cargo.toml
```

Expected: formatting check passes and Rust tests exit 0. Existing macOS capture tests must remain platform-safe.

- [ ] **Step 3: Run workspace lint and type checks**

Run:

```bash
bun run lint
bun run check-types
```

Expected: Turbo completes both tasks without lint or type errors.

- [ ] **Step 4: Run the native development build**

Run: `bun run build:dev`

Expected: Vite starts successfully, the Rust dev profile compiles, and the native app launches. Stop the long-running process with the normal interrupt after startup verification; do not treat the expected interrupted dev process as a build failure.

- [ ] **Step 5: Manually verify the interaction checklist**

With screen permission granted:

1. Confirm the collapsed bar starts with capture active and the `MonitorDotIcon` highlighted/pinging.
2. Click `MonitorDotIcon`; confirm capture stops, the highlight/ping disappear, and no Ask request is sent.
3. Click it again; confirm capture restarts and the state returns.
4. Confirm collapsed controls are Iris + MonitorDot + MicAudioLines.
5. Start Listen from collapsed; confirm the Listen card opens and the voice control reflects active recording.
6. Expand the input; confirm only Iris + input + MicIcon + Settings are present.
7. Start dictation; confirm the input receives drafts and the compact waveform appears.
8. Confirm active Ask, capture, Listen, and dictation states pulse the outer bar.
9. Confirm reduced-motion settings remove pulse, ping, and waveform motion.

- [ ] **Step 6: Review the final diff and commit any verification fixes**

Run:

```bash
git status --short
git diff --check
git diff HEAD~3..HEAD --stat
```

Expected: only the planned spec/plan and implementation files are changed; `git diff --check` reports no whitespace errors. If verification fixes are needed, run the affected focused test again and commit with a message describing the fix.
