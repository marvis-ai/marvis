# Marvis — Ask Input Dictation Design

Date: 2026-09-23
Status: Approved design
Branch: `feature/implement_voice_listening_part`
Related spec: `docs/superpowers/specs/2026-09-22-listen-phase-2-design.md`

## Goal

Give the mic button two clear behaviors:

1. **Ask-input dictation**: when the Ask input is visible, transcribe the user's
   microphone speech directly into that input for review and sending.
2. **Meeting Listen**: when the bar is collapsed/idle, keep the existing
   conversation-listening behavior that captures microphone and system/meeting
   audio into the local transcript database.

Dictation is transient and composes an Ask prompt. Meeting Listen remains the
persistent meeting-history feature.

## User decisions

| Question | Decision |
| --- | --- |
| Mic behavior | Input open means dictation; collapsed idle bar means meeting Listen. |
| Dictation insertion | Append/insert at the current caret; selected text is replaced. |
| Live updates | Show the draft live inside the Ask input. |
| After Stop | Leave the transcript in the input for review; Enter sends it. |
| Persistence | Do not persist dictation transcript rows, summaries, or meeting sessions. |
| Meeting audio | Meeting Listen captures `me` and `them`/meeting audio as it does today. |
| Architecture | Add a dedicated Rust `DictationService`; do not overload `ListenService`. |

## Current-state context

`Bar.tsx` owns the Ask input and the mic button. `ListenService` currently owns
all speech sessions. It creates a persistent `listen` session, starts microphone
and system audio, assembles `me`/`them` turns, persists transcripts, and emits
`listen:*` events.

The current UI treats the mic button as the meeting Listen toggle in every
state. As a result, speech never fills the Ask input.

The bundled `whisper-cli` is now self-contained and validated for static
whisper/ggml linkage. Dictation can reuse the same configured provider/model and
bundled binary resolution without introducing another speech stack.

## Architecture

Add a dedicated Rust-owned `DictationService` beside `ListenService`.

```text
src-tauri/src/
├── audio/mic.rs       # reused microphone source
├── stt/               # reused provider trait/factory
├── dictation.rs       # new mic-only transient draft service
└── listen.rs          # existing persistent meeting service
```

`DictationService` responsibilities:

- Start/stop microphone-only STT.
- Reuse `MicSource`, `SttProvider`, provider configuration, and bundled
  `whisper-cli` resolution.
- Maintain one transient draft for the `me` channel.
- Emit draft/status/error events.
- Return the final draft on Stop.
- Never write transcript, summary, or session rows.

`ListenService` remains responsible for meeting capture, turn assembly,
persistence, summaries, and `{{CONVERSATION_HISTORY}}` context.

## Mode selection

The mic button chooses behavior from the current UI state:

| Current state | Mic press result |
| --- | --- |
| Dictation active | Stop dictation and leave the draft in the Ask input. |
| Meeting Listen active | Stop Listen and leave the transcript in the Listen panel. |
| Ask input visible, no speech session | Start dictation. |
| Collapsed idle capsule, no speech session | Open the card and start meeting Listen. |

The visible Ask input includes the expanded collapsed-bar input and the input
row shown while the card is open. Once a meeting Listen session is active, Stop
takes precedence over starting dictation.

The button label/title reflects the next action:

- `Dictate` when the Ask input is visible.
- `Listen` when the bar is collapsed.
- `Stop dictation` or `Stop listening` while active.

The existing active mic styling may be shared by both modes.

## Dictation data flow

```text
mic button + Ask input visible
  → dictation_start
  → request microphone permission
  → MicSource only
  → configured STT provider/model
  → dictation:draft { text, final: false }
  → dictation_stop
  → final draft returned/emitted
  → input remains editable for review
```

Provider behavior is normalized by the draft event:

- Deepgram emits interim and final transcript events; the draft updates
  continuously.
- `whisper-cli` emits final-only chunk results after each audio window; the
  draft updates in chunks. No fake interim text is generated.

## Ask-input editing contract

On `dictation_start`, the frontend records the input caret or selected range.

Each `dictation:draft` update replaces only the live dictated range:

- Text before and after the dictated range is preserved.
- A selected range is replaced by the dictated draft.
- The caret remains after the dictated draft.
- Stop preserves the final text and focuses the input for review.
- Dictation never auto-submits the Ask prompt.

If the user edits inside the live dictated range, dictation stops and preserves
the current text. Edits outside the dictated range remain valid. Collapsing or
closing the Ask input also stops dictation.

## Commands and events

Commands:

| Command | Purpose |
| --- | --- |
| `dictation_start` | Requests microphone permission and starts mic-only STT. Returns current dictation status. |
| `dictation_stop` | Stops capture/STT and returns the final draft. Safe to call repeatedly. |
| `dictation_status` | Returns idle/listening/error state for UI resynchronization. |

Events emitted to the `bar` window:

| Event | Payload |
| --- | --- |
| `dictation:state` | `{ state, provider, error }` |
| `dictation:draft` | `{ text, final }` |
| `dictation:error` | `{ message, needs_setup }` |

Events must not expose binary paths, provider credentials, model download URLs,
raw audio, or process output.

## Mutual exclusion and lifecycle

Dictation and meeting Listen are mutually exclusive.

- `dictation_start` while meeting Listen is active returns a safe conflict
  error unless the UI already stopped Listen through the mic toggle.
- `listen_start` while dictation is active returns a safe conflict error unless
  the UI already stopped dictation through the mic toggle.
- The mic button's normal behavior is to stop the active mode first.
- Closing the Ask input stops dictation.
- Closing/collapsing the card does not stop meeting Listen.

## Errors and privacy

Dictation sends microphone audio only to the configured STT backend.

- Microphone permission denied → sanitized `dictation:error`; input remains
  editable.
- Missing provider configuration, model, or `whisper-cli` → safe setup error.
- Provider disconnect/failure → dictation stops and preserves the latest draft.
- Repeated Stop calls leave the input unchanged.
- Dictation does not write to `transcripts`, `summaries`, or `sessions`.
- Dictated text is persisted only when the user submits it as a normal Ask
  message.
- Meeting Listen persistence and privacy behavior remain unchanged.

## Testing

Rust tests must cover:

- Draft assembly for interim and final STT events.
- Whisper-style final-only chunks.
- Stop flushing the current draft once.
- Empty/silence output producing an empty draft.
- Sanitized provider/setup errors.
- Dictation cannot start while meeting Listen is active.
- Meeting Listen cannot start while dictation is active.
- Dictation creates no transcript, summary, or session rows.
- Existing Listen turn assembly, persistence, and summary tests keep passing.

Frontend/manual checks must cover:

- Mic press with the Ask input visible starts dictation, not meeting Listen.
- Live draft inserts at the caret.
- Existing prefix/suffix text is preserved.
- Selected text is replaced by the dictated draft.
- Stop leaves the text editable without auto-submitting.
- Editing inside the live dictated range stops dictation safely.
- Collapsing the input stops dictation.
- Mic press from the collapsed capsule starts meeting Listen.
- Meeting Listen still records `me`/`them` history and appears in
  `ListenSection`.
- Provider setup failure does not clear existing input.

Verification commands:

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

Manual validation will use the self-contained bundled `whisper-cli` and a
downloaded Tiny model. Deepgram validation requires a configured local test
key.

## Out of scope

- Automatically submitting dictated text after Stop.
- Speaker diarization or meeting summaries during dictation.
- Persisting dictation drafts to Listen history.
- Changing the existing meeting Listen panel.
- Changing provider/model selection UI.
- Replacing `whisper-cli` with a streaming Whisper implementation.
