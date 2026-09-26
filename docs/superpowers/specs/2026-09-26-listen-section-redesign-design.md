# Marvis Native — Listen Section Redesign (Structured Transcript + Pinned Summary)

Date: 2026-09-26
Status: Draft (pending written-spec review)
Branch: feature/redesign_listening_with_summary
Reference: attached structured-transcript mockup (title/subtitle header,
state badge + elapsed timer + copy in the right cluster, speaker filter
chips, per-block `m:ss` timestamps)

## Goal

Rebuild `ListenSection` as a structured meeting document: a real header
(title + sources + state + controls), a speaker filter, per-block elapsed
timestamps, per-row and whole-transcript copy, and a summary strip PINNED
above the input row (2-line clamp, expandable) instead of an inline block
that drifts with the transcript scroll.

## User decisions

| Question | Decision |
| --- | --- |
| Stop result | Stay on the finished doc — STOPPED badge, transcript + last summary remain, same surface as a History-opened session |
| Pause model | Soft-pause — audio sources keep running, workers drop chunks; open turn flushes at the boundary (no STT reconnect, no hard stream teardown) |
| Timer | Elapsed RECORDING time (`m:ss`, uncapped like the mockup's 107:36) — freezes while paused |
| Summary placement | Pinned 2-line strip above the input row + expand control; never inside the transcript scroll |
| Clipboard | `navigator.clipboard.writeText` from click handlers (no plugin); fall back to `tauri-plugin-clipboard-manager` only if WKWebView blocks it |

## Section layout

```text
┌ header ─ title (summary topic ?? 'Listen')
│          subtitle `mic + system audio · {provider} {model}`
│          right: [STATE pill] [m:ss] [pause/resume] [stop] [copy]
├ filter ─ ( all ) ( you ) ( speaker 1 ) ( speaker 2 ) …   `{n} rows`
├ transcript scroll ─ per block: `m:ss` · NAME (colored dot)
│                     paragraph (finals + dim interim caret)
│                     hover copy icon per block
├ summary strip (pinned, border-t) ─ 2-line TLDR clamp + expand ▾
│                     expanded: bullets + follow-up chips
└ (input row — the card's existing bottom footer)
```

- Timer sits in the right cluster next to the badge as drawn in the
  mockup (the "left side" note conflicts with the image — mockup wins).
- Controls by state: `listening` → pause + stop; `paused` → resume +
  stop; `stopped`/viewing → none (badge + timer + copy only).
- `mic + system audio` is honest metadata: `status.mic === false` renders
  `system audio only`.

## Transcript blocks

- Timestamp per block = `ts − session.started_at` rendered `m:ss`
  (fall back to wall-clock `HH:MM` when `started_at` is unavailable, e.g.
  old sessions in viewing mode).
- Speaker filter: chips derive from distinct `(speaker, speaker_idx)`
  pairs actually present — `all` + each identity via the existing
  `speakerName`/`speakerColor` helpers. Right-aligned `{n} rows` =
  filtered block count. Filter is display-only; resets on session change.
- Per-block copy (hover icon): `{Name}: {block text}`.
- Copy-all (header): `[m:ss] Name: text` per block; when a summary exists
  it appends `---`, `TLDR: …`, and `-` bullet lines. Button flips to a
  check for ~1.5 s.

## Pinned summary strip

- Bottom of the section (directly above the input row): `TLDR{ · topic}`
  label + tldr clamped to 2 lines + a chevron; expanding reveals bullets
  and follow-up chips. The card's existing ResizeObserver →
  `window_adjust_height` grows/shrinks the window on toggle — no new
  window plumbing.
- Live mode: latest `listen:summary` payload (existing state). Viewing /
  stopped mode: `summary_latest(viewing.id)`.
- Strip renders only when a summary exists — an early live session shows
  just the transcript.

## Backend changes (`listen.rs` / `lib.rs` / `storage.rs`)

| Change | Detail |
| --- | --- |
| `ListenStatus.state` | adds `"paused"`; `is_listening()` → `listening \|\| paused` so dictation mutual exclusion holds through a pause |
| `ListenStatus` fields | `+ started_at: Option<i64>`, `+ paused_secs: i64`, `+ paused_since: Option<i64>` (serialized-keys test updated) |
| `storage.rs` | `session_started_at(id) -> Option<i64>` — `SELECT started_at FROM sessions WHERE id=?1` |
| `Running` | `+ paused: Arc<AtomicBool>`; worker loop drops dequeued chunks while set (sources keep streaming, STT is starved — nothing transcribed). ⚠ Remote STT risk: Deepgram-style websockets can idle-timeout when starved — if observed, the provider needs periodic keep-alive/silence frames while paused (local whisper-cli is unaffected) |
| `ListenService::pause()` | set flag → `assembler.flush()` + persist (turns close at the pause edge) → `paused_since = now`, state `"paused"` → emit `listen:state` |
| `ListenService::resume()` | clear flag → `paused_secs += now − paused_since`, `paused_since = None` → state `"listening"` → emit |
| `stop()` during pause | unchanged — cancel, join, flush, `session_end` |
| Commands | `listen_pause`, `listen_resume` → call service + emit `listen:state` (same shape as start/stop) |
| `listen_start` | populates `started_at` from the minted session row |

Elapsed recording time (frontend): `(paused_since ?? now) − started_at
− paused_secs`.

## Frontend changes

| Area | Detail |
| --- | --- |
| `commands.ts` | `ListenStatus`: `state += 'paused'`, `+ started_at`, `+ paused_since`, `+ paused_secs`; `listenPause()`, `listenResume()` |
| `ListenSection` | rebuilt per §Section layout; block renderer keeps `speakerName`/`speakerColor`; new `useElapsed` tick (1 s); filter + copy state local |
| `viewing` prop | `{ id, startedAt, endedAt }` (extends the history spec's `{id, startedAt}`) — drives STOPPED badge, duration timer, `transcripts_for` + `summary_latest` loads |
| Stop button | `listenStop()` then `setListenViewing({id, startedAt, endedAt: now})` on the just-ended session — the card stays open on the finished document |
| Mic affordance | bar's Listen button unchanged (starts + pins `'listen'`) |

## Testing

- Rust: pause sets `paused`/flushes/state; resume accumulates `paused_secs`;
  `is_listening()` true while paused; stop-from-paused clean; status wire
  keys updated.
- Verify: `cargo test`, `cargo clippy`, `bun run check-types` +
  `bun run build` in `apps/native`.
- Manual: live header/timer/pause-resume-stop; speaker filter incl.
  interim; per-row + copy-all paste formats; summary clamp ⇄ expand;
  stop → finished doc; reopen same session from History (identical doc).

## Out of scope

- Hard-pause (stopping audio streams / STT reconnect)
- Named speaker enrollment beyond voiceprint `You`/`Guest N`
- Per-row audio playback (`audio_file` segments)
- Transcript search, file export, editing turns
