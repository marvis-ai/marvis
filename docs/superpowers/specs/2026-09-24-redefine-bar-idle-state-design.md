# Redefine the Bar Idle State

## Goal

Make the floating native bar communicate whether Marvis is idle, processing, capturing the screen, recording voice, or dictating, while separating the screen-capture, voice-recording, and dictation actions.

## Scope

This change covers the native bar in `apps/native/src/views/Bar.tsx`, its Tauri command/event contract, the macOS capture lifecycle boundary, and the bar's animation/accessibility states. Existing Ask, Listen, and dictation semantics remain intact unless explicitly described below.

## Interaction model

### Collapsed bar

The collapsed floating bar displays three controls:

1. The existing Iris control, preserving its current behavior.
2. `MonitorDotIcon` for continuous screen capture.
3. `MicAudioLinesIcon` for Listen voice recording with speaker diarization.

### Expanded input bar

The expanded input row displays:

1. The Iris/back control.
2. The Ask input.
3. `MonitorDotIcon` for continuous screen capture.
4. `MicAudioLinesIcon` for Listen voice recording.
5. `MicIcon` for dictation into the Ask input.
6. The existing settings control.

The latest interaction decision takes precedence over the earlier requirement that the recording mic be hidden while collapsed: `MicAudioLinesIcon` is visible in both collapsed and expanded layouts.

## Control behavior

### Screen capture

- Screen capture is enabled by default after screen permission is available and the app enters the main gate.
- `MonitorDotIcon` toggles continuous capture only; it never submits an Ask request.
- The active state uses an accent highlight and `animate-ping`.
- The inactive state is neutral and has no ping animation.
- Capture state is initialized through `capture_status()` and kept current through a native lifecycle event.
- Permission loss and main-gate exit stop capture using the existing cleanup behavior.

### Listen voice recording

- `MicAudioLinesIcon` is off by default from the bar's perspective.
- Clicking it starts or stops the existing Listen flow with speaker diarization.
- Starting Listen from the collapsed bar opens the Listen card, matching the current behavior.
- Starting Listen from the expanded input keeps the expanded bar and uses Listen as the active recording mode.
- The existing Listen state/error handling remains authoritative.

### Dictation

- `MicIcon` is rendered only in the expanded input row.
- Clicking it starts or stops dictation at the current caret/selection.
- The existing dictation range anchoring, live draft replacement, edit reconciliation, Enter-to-stop review behavior, and no-auto-submit contract remain unchanged.
- Active dictation displays a compact waveform animation associated with the dictation control.

## Activity and animation states

The bar receives the `animate-pulse` treatment when any active work is present:

- Ask loading or streaming.
- Continuous screen capture.
- Listen voice recording.
- Dictation.

Specific control states remain visually stronger than the generic bar pulse:

- Screen capture: highlighted `MonitorDotIcon` with `animate-ping`.
- Dictation: active `MicIcon` with a compact waveform.
- Listen: existing active Listen/Iris and waveform behavior where applicable.

All new animation classes must follow the native app's existing reduced-motion handling. No new animation should cause layout shifts or change the bar's window sizing contract.

## Architecture and data flow

### Native capture boundary

Extract the current capture startup and teardown responsibilities into reusable native helpers or a focused controller boundary. Add explicit Tauri commands for:

- `capture_start`
- `capture_stop`
- `capture_status` (existing command, retained and extended only if necessary)

The commands must be idempotent: starting an already-running capture and stopping an already-stopped capture must not create duplicate workers or fail unnecessarily.

Emit a typed `capture:state` event after successful transitions and terminal capture errors. The payload should expose the current running state and preserve the existing frame-count information where useful to the UI.

Automatic startup after permission/main-gate entry remains in place, but it must call the same capture-start boundary used by the UI command. Main-gate exit, permission revocation, and application shutdown must use the same stop boundary.

### Frontend synchronization

- Add typed capture status and event payloads to `commands.ts` and `events.ts`.
- Initialize capture state from `captureStatus()` when the bar mounts.
- Subscribe to `capture:state` and update the screen control immediately.
- Add local Ask activity state from `EV_ASK_STATE` so loading and streaming participate in the overall activity calculation.
- Derive one `hasActiveWork` value from Ask activity, capture state, Listen state, and dictation state.
- Keep speech-mode serialization so Listen and dictation cannot start concurrently or race during teardown.

### Icon conventions

Use the `@marvis/ui` barrel and the project's suffixed Lucide icon convention:

- `MonitorDotIcon`
- `MicAudioLinesIcon`
- `MicIcon`

Remove the old `CameraIcon` usage from the bar. The existing `MicIcon` usage must be moved to the dictation-only control rather than reused for Listen.

## Accessibility

Each toggle must provide:

- `aria-pressed` matching its current active state.
- A state-specific accessible label, such as `Start screen recording` or `Stop screen recording`.
- A matching title where the existing bar pattern uses titles.

The bar and controls remain keyboard reachable. Reduced-motion users must not receive ping, pulse, or waveform motion.

## Testing and verification

Add deterministic coverage for the capture controller's idempotent start/stop transitions where the native capture implementation permits it. Extend frontend logic coverage for:

- Capture default-on synchronization.
- Screen capture toggling without sending an Ask request.
- Independent Listen and dictation controls.
- Active-work pulse derivation.
- Dictation's existing anchored draft behavior remaining unchanged.

Run the relevant test suite, TypeScript checks, lint, and development build using Bun before implementation is considered complete.

## Non-goals

- Do not redesign the bar's glass, positioning, dimensions, or window-growth behavior.
- Do not change Ask submission semantics.
- Do not add a new recording format, persistence layer, or settings preference for the default capture state.
- Do not replace the existing Listen or dictation backend implementations.
