# Marvis — Listen, Phase 2 Design

Date: 2026-09-22
Status: Approved design (pending written-spec review)
Branch: `feature/implement_voice_listening_part`
Reference: `~/Codes/OpenSource/glass` (`src/features/listen/*`, `src/ui/listen/*`)

## Goal

Implement Glass's Listen feature inside Marvis's architecture: mic +
system audio → streaming STT → live transcript (me/them) + structured
summary every 5 turns, all persisted to the `listen` session — and the
transcript finally fills `{{CONVERSATION_HISTORY}}`, so Ask answers with
meeting context instead of a placeholder.

## User decisions

| Question | Decision |
| --- | --- |
| Scope | Full Listen (Glass parity): dual-channel STT, live transcript, every-5-turns summary, persistence |
| STT backends | Deepgram (WS streaming) **and** local whisper.cpp — user picks via `models.stt_provider` |
| Whisper runtime | `whisper-cli` sidecar (chunked), **not** in-process whisper-rs |
| Provisioning | Detection only — the app never downloads anything. User installs `whisper-cli` (brew/PATH) and drops `ggml-*.bin` models into `~/.marvis/models/whisper/models/` |
| Keystore | `keys.json` (plaintext, 0600) via the existing `Keystore::key("deepgram")` — `keys.enc` is retired |

## Architecture

All new code lives in `apps/native/src-tauri/src/`:

```text
src-tauri/src/
├── audio/
│   ├── mod.rs        # PcmChunk type, AudioSource trait (start/stop → mpsc)
│   ├── mic.rs        # cpal input → resample → 16 kHz mono s16
│   └── system.rs     # second SCStream, SCStreamOutputType::Audio
├── stt/
│   ├── mod.rs        # SttProvider trait, TranscriptEvent, provider factory
│   ├── deepgram.rs   # WSS streaming session (interim + final)
│   └── whisper.rs    # whisper-cli sidecar (final-only chunks)
├── listen.rs         # ListenService: turn assembly, persistence, summary
└── prompts.rs        # + SUMMARY_USER_PROMPT (structured-format message)
```

The webview changes only `ListenSection.tsx`, a small STT picker in
prefs, `commands.ts`/`events.ts` wrappers, and `Bar.tsx`'s mic button.

## Data flow

```text
mic (cpal) ──┐                          ┌─ deepgram WS ("me" conn)
             ├─ 16kHz mono s16 ──► stt ─┤
SCStream ────┘   mpsc chunks            └─ deepgram WS ("them" conn)
 Audio          (per channel)                or whisper-cli chunks

TranscriptEvent{channel, text, final}
   → ListenService: interim → listen:turn{final:false} (deepgram only)
                    final  → open/extend channel turn
   → silence gap ~1.5 s or channel switch → close turn
        → transcripts row + listen:turn{final:true}
        → turns_since_summary == 5 → summary task → summaries row
                                                  + listen:summary
```

Speaker = channel, never diarization: mic → `"me"`, system → `"them"`.

## Audio capture

- `audio/mic.rs`: `cpal` default input device, any native rate/format →
  resample to 16 kHz mono `i16` (`rubato` or linear — implementation
  detail), emit ~100 ms `PcmChunk`s on an `mpsc` channel.
- `audio/system.rs`: a second `SCStream` on the same content filter as
  the screen stream — `captures_audio(true)`,
  `excludes_own_process_audio(true)`, `SCStreamOutputType::Audio`
  callback → extract `CMSampleBuffer` PCM (48 kHz f32 stereo typical) →
  resample → same `PcmChunk` shape. No helper binary, no disk.
- Permissions: mic via existing `permissions_request_mic`; system audio
  rides the screen-recording grant (already required for `Main`).
- **Mic denied ≠ fatal**: `listen_start` requests mic; denied → run
  system-audio-only and include `"mic": false` in `listen:state`.

## STT providers

`stt/mod.rs`:

```rust
enum TranscriptEvent { Interim(String), Final(String) }

trait SttProvider: Send {
    fn start(&self, on_event: Box<dyn Fn(TranscriptEvent) + Send>);
    fn send_pcm(&self, chunk: &[i16]);   // non-blocking enqueue
    fn stop(&self);
}
```

One provider instance per channel (two connections/two chunk streams).

### `deepgram.rs`

- `wss://api.deepgram.com/v1/listen?model=<stt_model>&encoding=linear16
  &sample_rate=16000&channels=1&interim_results=true&punctuate=true
  &smart_format=true` — auth via `Authorization: Token <key>` header;
  key from `keystore.key("deepgram")`.
- tokio-tungstenite session task: PCM sender + JSON receiver →
  `TranscriptEvent` (`is_final` → `Final`, else `Interim`).
- Keepalive (Deepgram `KeepAlive` control msg on idle) and **session
  renewal at ~20 min** — Glass parity for the provider hard timeout:
  transparent re-open, turn state unaffected.
- Missing `deepgram` key → provider fails fast at `start` with a
  "configure in Settings" error (same handling as LLM providers).

### `whisper.rs`

- Binary resolution order: `PATH` → `/opt/homebrew/bin/whisper-cli` →
  `~/.marvis/models/whisper/bin/whisper-cli`.
- Model: `models.stt_model` names a `ggml-*.bin` inside
  `~/.marvis/models/whisper/models/` (picker lists dir contents; empty
  dir → setup hint).
- Chunk loop per channel: accumulate PCM; every ~3 s window with RMS
  above a silence threshold → write `~/.marvis/tmp/listen-<ch>-<n>.wav`
  (0600) → `whisper-cli -m <model> -f <wav> --no-timestamps
  --output-txt` → parse stdout → `Final` segment → **delete the wav**.
  Below threshold → drop the window (it only buys whisper
  hallucinations).
- No `Interim` events — whisper is chunked; latency (~1–3 s/window on
  tiny/base) is shown as-is, not faked.
- Missing binary or model → `whisper_status` reports it; `listen_start`
  with `stt_provider="whisper"` and nothing found → `listen:error` with
  a setup hint; if a `deepgram` key exists, offer Deepgram fallback
  (UI-level choice, no silent switch).

## Turn assembly & persistence (`listen.rs`)

- `ListenService` holds per-channel open turns. A `Final` appends to the
  channel's open turn; an `Interim` updates it in place (event only,
  never persisted).
- A turn closes after **~1.5 s of silence** on that channel, or when the
  other channel produces a final (Glass's debounced-turn model). On
  close: `transcripts` insert + `listen:turn{speaker, text, ts,
  final:true}` + history push + `turns_since_summary += 1`.
- `turns_since_summary == 5` → reset counter → spawn summary task
  (below). Turns that arrive during a summary still count.
- `listen_start` → `session_get_or_create_active("listen")`; both audio
  sources + both STT sessions spawn. `listen_stop` → stop all, then
  `session_end`. The service is **independent of card mode** — collapsing
  or switching to chat does not stop it; the mic button toggles it.

## Live summary

Every 5 closed turns:

1. `system_prompt(formatted_history)` — the same
   `pickle_glass_analysis` template with the last ~20 turns substituted.
2. User message = `SUMMARY_USER_PROMPT` (port of Glass's structured
   request: Summary Overview / Key Topic / Extended Explanation /
   Suggested Questions) + prior-analysis context block when a previous
   summary exists.
3. Walk `provider_candidates` (first success wins, non-streaming
   `stream_chat` into a sink — the `describe_screen` pattern).
4. Parse the structured response (Glass's `parseResponseText` port:
   summary bullets, topic, extended text, follow-ups; merge-cap at 5
   bullets, keep prior on parse failure) → `summaries` insert →
   `listen:summary{tldr, bullets, follow_ups, topic}`.

Failures warn-log and skip the cycle — the transcript never blocks on
the summary.

## Ask integration

`ask.rs::build_messages` today calls `system_prompt("")`. Now it
substitutes the formatted tail of the active listen session's
transcript (`me: …`/`them: …` lines, last ~20 turns) — read through a
`listen::conversation_history()` accessor that returns the fallback
string when no listen session is active. No new plumbing: `Deps` gains
nothing; `build_messages` takes the history string as a parameter
gathered in `send_chain` alongside `load_history`.

## Schema (`marvis.db`, additive)

```sql
CREATE TABLE IF NOT EXISTS transcripts (
    id         INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    speaker    TEXT NOT NULL,   -- 'me' | 'them'
    text       TEXT NOT NULL,
    ts         INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS summaries (
    id         INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    tldr       TEXT NOT NULL,
    bullets    TEXT NOT NULL,   -- JSON array
    follow_ups TEXT NOT NULL,   -- JSON array
    topic      TEXT,
    ts         INTEGER NOT NULL
);
```

`Db` gains: `transcript_add`, `transcripts_for(session_id, limit)`,
`summary_add`, `summary_latest(session_id)`. `session_list` already
covers `type='listen'` rows.

## Commands / events

Commands (webview → Rust):

| Command | Purpose |
| --- | --- |
| `listen_start` | Gate=`Main`; resolves STT provider, requests mic, starts capture + sessions. Returns `listen_status` payload. |
| `listen_stop` | Stops everything, ends the `listen` session. |
| `listen_status` | `{state, provider, session_id, turns}` — resync for a reopened card. |
| `whisper_status` | `{binary: path | null, models: [ggml-*.bin…]}` — prefs + fallback hints. |
| ~~`listen_stub`~~ | Removed. |

`config_set` gains writable `models.stt_provider` (`"deepgram"|"whisper"`)
and `models.stt_model` (deepgram model id or ggml filename) — both emit
`config:changed` like every write.

Events (Rust → `bar` window):

| Event | Payload |
| --- | --- |
| `listen:state` | `{state: "listening" | "idle" | "error", provider, mic: bool}` |
| `listen:turn` | `{speaker, text, final, ts}` — `final:false` = interim repaint |
| `listen:summary` | `{tldr, bullets[], follow_ups[], topic}` |
| `listen:error` | `{message}` — also folds `state:"error"` into `listen:state` |

## UI

- **`Bar.tsx`**: the mic button becomes the session toggle — idle →
  `listen_start` + `setListenWanted(true)` + `windowSetChatOpen(true)`;
  listening → `listen_stop` (card stays open on the finished
  transcript). A live `listen:state` drives the button's active styling.
- **`ListenSection.tsx`**: transcript list — `me`/`them` rows, interim
  text rendered dimmed and replaced by the final; an Insights block
  below (or a Transcript/Insights toggle — match `ChatSection`'s
  density): TLDR line, bullet list, follow-up chips. Status row:
  `provider · model` chip, listening dot, Stop → `listen_stop`.
  Mount → `listen_status` resync + `transcripts`/`summaries` replay.
  The reserved `--animate-waveform` bars and `--animate-iris-listen`
  keyframes in `index.css` now wire to real state: waveform animates
  only while `listen:state` is `listening` (the stub's warning about
  a dead waveform lying stays honored).
- **Prefs**: the STT picker replaces the existing placeholder line in
  `ProvidersTab.tsx` ("Deepgram Nova-2 lands with Phase 2") — provider
  select (`deepgram`/`whisper`) + per-provider model field:
  `deepgram` → text/datalist (`nova-2`, `nova-3`, …); `whisper` →
  dropdown of detected `ggml-*.bin` + the `whisper_status` hint line
  when binary/models are missing.
- `commands.ts`/`events.ts` wrappers for all of the above.

## Error handling

- Mic denied → system-audio-only (`mic:false` in state), warn-logged.
- Deepgram WS drop → backoff reconnect (≤3 tries) inside the session;
  renewal timer resets. Hard failure → `listen:error` + `state:"error"`,
  capture keeps running so a provider switch resumes without data loss.
- whisper-cli spawn/parse failure → warn-log, skip window; repeated
  failures (>3 consecutive) → `listen:error`.
- Summary failure → warn-log, skipped; next 5-turn boundary retries.
- STT not configured at all → `listen_start` returns an error payload,
  `listen:error{needs_setup}` + card points to Settings.
- All commands `Result<T, String>`-serializable; no panics on audio/STT
  threads (a dead thread → `listen:error`, never a crash).

## Testing

`cargo test`:

- Turn assembly: debounce close on silence, channel-switch close,
  interim-then-final replacement, empty-text turns dropped.
- History formatting (`me:`/`them:` tail cap) — the string
  `build_messages` substitutes.
- Summary cadence: fires exactly on turn 5/10/…, survives summary-task
  failure.
- Whisper: stdout parse fixtures (clean text, empty, whisper's
  `[BLANK_AUDIO]`/`(silence)` artifacts → dropped), binary/model
  resolution order.
- Schema: transcripts/summaries round-trip, cascade delete with session.
- `config_set` accepts/rejects `models.stt_*` correctly.

Manual: Deepgram live session (interim repaint, me/them split, 20-min
renewal), whisper path (brew install + ggml-base.bin), mic-denied
fallback, summary appearing in Insights, ask answering with transcript
context, card collapse keeps listening.

## Out of scope

- Gemini/streaming STT providers, whisper.cpp binary/model downloads
  (Phase 3 local-AI UX), Ollama management.
- Speaker diarization beyond the channel split.
- Session history UI, presets editor, `Cmd+[`/`Cmd+]` response nav —
  all Phase 3.
- Transcript editing/export.

## Open risks

1. **cpal + SCK audio resampling**: two different native formats → one
   16 kHz pipeline; the resampler choice (`rubato` vs. naive linear)
   gets validated against whisper/Deepgram accuracy during bring-up.
2. **whisper-cli variance**: brew builds rename the binary occasionally
   (`whisper-cli` vs `main`); resolution order + `whisper_status`
   surfaces whatever is found rather than guessing silently.
3. **Whisper temp WAVs**: PCM briefly on disk under `~/.marvis/tmp/`
   (0600, deleted post-parse) — accepted tradeoff of the sidecar
   approach; noted for the privacy posture.
4. **Deepgram auth header** via tokio-tungstenite: if the crate's
   header support fights us, fallback is `access_token` query param —
   same security posture over WSS.
5. **Echo**: system audio contains the user's own voice on speaker
   calls (Glass has this too) — accepted Phase-2 behavior; AEC is a
   Phase-3+ problem if ever.
