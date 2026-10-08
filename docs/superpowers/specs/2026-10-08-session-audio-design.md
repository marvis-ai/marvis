# Session Audio — Playback + Export — Design

Date: 2026-10-08
Branch: `feature/deepen_the_loop` (Phase 1)
Scope: `apps/native` — asset-protocol config, one Rust command, bar
webview player + transcript sync.

## Context

Roadmap Phase 1: "Session audio: inline playback + WAV export." Every
listen session already retains a mixed 16 kHz mono WAV —
`~/.marvis/audios/recording_*.wav`, path stored on
`sessions.audio_file`, removed by `session_delete`. `audio_file` is
already on `SessionView`, so the webview knows the path; what doesn't
exist is any way to HEAR it or copy it out. Two 37 MB real recordings
exist in the field alongside many 44-byte header-only files from
sessions that captured nothing — empty audio is a first-class case,
not an anomaly.

Follow-up to the transcript-export feature (spec
`2026-10-08-transcript-export-design.md`), which established the
patterns this reuses: `rfd` save dialog on `spawn_blocking`, a
`Gate::Main` generic command, `exportFileName` slugging, `raise` for
errors.

## Decisions

| Question | Decision |
| --- | --- |
| File access | Tauri `asset:` protocol via `convertFileSrc` — native streaming + Range seek, no byte shuttling over IPC |
| Asset scope | `$HOME/.marvis/audios/**` only — read-URL access to the recordings dir, nothing else |
| Transcript sync | Both ways: click a block's timestamp to seek; the playing block highlights during playback |
| Live sessions | Ended sessions only — `ended_at` gates the player (a recording in progress is not a playback surface) |
| Export | Third dropdown item `Save audio…`; byte-for-byte WAV copy via save dialog |
| Source path | The command takes `session_id`, not a path — Rust reads `audio_file` from the db, so a crafted invoke cannot name an arbitrary source |

## Playback surface

A slim player row in the Listen doc, below the header above the
transcript: play/pause icon, seek slider, `elapsed / duration`. One
`<audio>` element per doc, `src = convertFileSrc(audio_file)`,
`preload="metadata"`.

Visibility rule — all three required:

- `audioFile != null` — plumbed onto `ListenViewing.audioFile` for
  viewed docs and `ListenStatus.audio_file` for the live doc (see
  Webview changes; neither carries it today)
- `endedAt != null` (the live doc surfaces the player once the
  session ends; `audio_file` is set at record start, so `ended_at` —
  not the view kind — is the gate)
- the file has real audio: `loadedmetadata` reports `duration > 0`
  (covers missing files and 44-byte header-only WAVs — the row stays
  hidden silently, same degrade contract as the markdown sections)

Playback stops on doc switch and unmount.

## Transcript sync — both directions

- **Block → audio:** clicking a block's timestamp (the `0:42` the
  block header already renders — keeps text selection intact) sets
  `audio.currentTime = block.ts - started_at`, clamped `>= 0`, and
  plays.
- **Audio → block:** `timeupdate` (~4 Hz) maps
  `started_at + currentTime` to the containing block; that block gets
  a low-alpha highlight in its existing speaker color. On pause the
  highlight persists; slider scrubbing drives the same lookup.
- Pure derived math in the webview — `ts`/`started_at` are already
  loaded. No new Rust state, no events.

## Rust changes

| Where | Change |
| --- | --- |
| `lib.rs` | `save_audio_file(session_id, suggested_name) -> Result<Option<String>, String>` — Gate::Main, registered + contract-tested; `listen_status` gains `audio_file` (db read it already owns) |
| `tauri.conf.json` | `app.security.assetProtocol = { enable: true, scope: ["$HOME/.marvis/audios/**"] }` |
| `capabilities/default.json` | No new permission — `convertFileSrc` uses the configured core asset protocol; keep the capability least-privileged |

`save_audio_file`:

- Resolves `audio_file` from `sessions` by `session_id` inside the
  command — the webview never supplies a filesystem path.
- `None` audio_file or a missing/empty file on disk →
  `Err("no audio for this session")` style message → `raise`.
- Dialog + `fs::copy` on `tauri::async_runtime::spawn_blocking`;
  `set_file_name(sanitize_suggested_name(..))` (reuse) +
  `add_filter("WAV audio", &["wav"])`.
- `None` return = user cancel (or gate drop). `Some(path)` on success.
- No `AppState` lock held across the dialog or the copy.

## Webview changes

| Where | Change |
| --- | --- |
| `commands.ts` | `saveAudioFile(sessionId, suggestedName): Promise<string \| null>`; `ListenStatus` gains `audio_file: string \| null` |
| `listen/model.ts` | `audioOffset(block, startedAt)` (seek target, clamped) + `activeBlockAt(blocks, startedAt, seconds)` (highlight lookup); `exportFileName` gains an extension param (`'md'` default, `'wav'` for audio); `ListenViewing` gains `audioFile: string \| null` |
| `HistorySection.tsx` | `onOpenListen` maps `s.audio_file` → `audioFile` (the Session row already carries it) |
| `SessionPlayer.tsx` (new, `listen/`) | The player row: element, play/pause, slider, time labels, visibility/degrade rules |
| `TranscriptBlocks.tsx` | Timestamp gets click-to-seek; active-block highlight style |
| `ListenSection.tsx` | Owns the audio element + position state (or a small hook); wires seek + highlight; `Save audio…` item in the share dropdown under the same visibility rule |

The live doc's `audioFile` comes from `listen_status` (Rust already
knows the path — it wrote the row); on session end the status resync
carries it into `onSessionEnded`'s `ListenViewing`. No extra IPC.

`exportFileName(topic, startedAt, 'wav')` yields
`marvis-{slug}-YYYYMMDD-HHmm.wav` — the save-dialog suggestion is
unrelated to the stored `recording_*` name, which stays as-is (a
rename would orphan `audio_file` values for zero benefit).

## Testing

- `cargo test`: `save_audio_file` registered + gate-guarded +
  `spawn_blocking` (contract test); resolves `audio_file` from a real
  db; rejects a session with no audio or a missing file.
- `bun test`: `audioOffset` clamp + math; `activeBlockAt` boundary
  picks (before first block, gaps between blocks, past the end);
  `exportFileName` with `.wav`.
- Manual (`bun run build:dev`): player appears on an ended session
  with real audio; hidden on 44-byte recordings and live sessions;
  click-to-seek lands on the right moment and plays; highlight tracks
  playback and scrub; `Save audio…` copies a playable wav; cancel is
  silent; missing file → toast.

## Out of scope

- Waveform visualization, playback speed, volume control.
- Per-channel audio (the WAV is already mixed mono).
- Playback of in-progress recordings; live-session export stays
  markdown-only.
- Auto-scrolling the transcript to follow playback.
- Renaming `recording_*.wav`, audio dedup/cleanup, retention policy.
