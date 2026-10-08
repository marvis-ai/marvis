# Transcript Export — Design

Date: 2026-10-08
Branch: `feature/deepen_the_loop` (Phase 1, item 2 of 6 for the user)
Scope: `apps/native` — one Rust command, bar webview.

## Context

Roadmap Phase 1: "Summary + transcript export (markdown / clipboard) —
data already sits in `marvis.db`; there's no egress path at all." The
note is now half-stale: per-block copy (`blockCopyText`) and
whole-document plain-text copy (`transcriptCopyText` + the
`SpeakerFilter` copy button) shipped with the listen redesign. What is
still missing is a FILE egress and a markdown-formatted document. Chat
sessions and History rows are out of scope (user decision) — export
lives only on the Listen document, live capture and viewed doc alike.

## Decisions

| Question | Decision |
| --- | --- |
| Surfaces | The Listen doc only — `SpeakerFilter` row, next to the existing copy button |
| Destination | Both: `Save .md…` via a native save dialog AND `Copy markdown` to the clipboard |
| Plumbing | Webview builds the markdown; a dumb generic Rust command owns dialog + write (no new plugins, no fs capability) |
| Live sessions | Export is available on the live doc too — a snapshot of the finals captured so far, same reach as the copy button |

## Markdown document

`transcriptMarkdown(blocks, meta, summary)` — a new pure builder in
`src/components/listen/model.ts` beside `transcriptCopyText`. Reuses
`buildBlocks` output, `speakerName`, `elapsedLabel`/`timeLabel` — the
speaker rules (mic guests, unlabeled `them`) and the stamp rule stay
single-source.

```md
# { topic ?? 'Listen session' }

_{date 'MMM d, yyyy'} · {start wall-clock 'HH:mm'}{ · stt engine when known}_

## Summary

{ tldr }

- bullet
- bullet

### Follow-ups

- item
- item

## Transcript

- **[0:42] You:** text text text
- **[2:14] Speaker 1:** text
```

Rules:

- Stamps are elapsed `m:ss` off `startedAt` when known, else the
  block's wall-clock `HH:mm` — identical to `transcriptCopyText`.
- Sections degrade: no summary → both summary sections dropped (a
  summary always has a `tldr`; `follow_ups` adds its sub-heading only
  when non-empty); no turns → the `## Transcript` section is dropped.
- Transcript lines are markdown bullets so pasted output renders as a
  list in any md-aware target; the `[stamp]` is inside the bold span.
- Interims are excluded — export is finals + summary only.
- The meta line carries the STT engine label when `viewing.stt` (or the
  live engine) is known — provenance, like the doc's own subtitle.

## Rust changes

| Where | Change |
| --- | --- |
| `Cargo.toml` | `rfd = "0.15"` — new dep, native file dialogs without the `tauri-plugin-dialog` capability surface |
| `lib.rs` | `save_text_file(suggested_name, contents) -> Result<Option<String>, String>` — Gate::Main-guarded, registered in `generate_handler!`, contract test updated |

`save_text_file` is deliberately generic (a future chat export uses it
unchanged):

- `suggested_name` sanitized server-side: `/` `\` and control chars
  stripped, capped at 80 chars, empty → `marvis-export.md`.
- The dialog runs on `tauri::async_runtime::spawn_blocking` (it blocks
  the calling thread — never the async executor), `rfd::FileDialog`
  with `set_file_name(suggested)` + `add_filter("Markdown", &["md"])`.
- `None` return = user cancel (not an error). `Some` → `fs::write`,
  errors mapped `Result<_, String>` — path and IO error text only, no
  internals.
- No `AppState` lock is held across the dialog or the write.

## Webview changes

| Where | Change |
| --- | --- |
| `commands.ts` | `saveTextFile(name, contents): Promise<string \| null>` |
| `listen/model.ts` | `transcriptMarkdown(...)` + `exportFileName(topic, startedAt)` — `marvis-{slug}-YYYYMMDD-HHmm.md`, slug = topic lowercased, non-`[a-z0-9]`→`-`, ≤40 chars, fallback `listen` |
| `SpeakerFilter.tsx` | New `onCopyMarkdown`/`onSaveMarkdown`/`exported` props; a `ShareIcon` button beside the copy button opening a `ChatMsgMenu`-pattern dropdown (backdrop + absolute menu): **Copy markdown**, **Save .md…** |
| `ListenSection.tsx` | `exportOpen` state; `copyMarkdown()` = `transcriptMarkdown` → `navigator.clipboard` + the shared check-flash; `exportMd()` = `saveTextFile(exportFileName(...), transcriptMarkdown(...))`, cancel silent, write error → `raise` (the fire-and-forget `alert_show` helper) |

Success feedback: the check-flash icon swap (1.5 s) on the export
button for both paths — the file dialog + written file is its own
confirmation; cancel is silent; a failed write surfaces through the
alert toast.

## Testing

- `bun test` (`apps/native`): `transcriptMarkdown` — section presence
  and degradation, elapsed vs wall-clock stamps, bullet shape, slug +
  filename rules, no-summary/no-turns docs.
- `cargo test` (`src-tauri`): the `lib.rs` contract test asserts
  `save_text_file` is registered and `Gate::Main`-guarded; the
  suggested-name sanitizer (separators, cap, fallback).
- Manual (`bun run build:dev`): export a live session and a viewed doc;
  cancel leaves nothing; `.md` opens in an editor rendering
  title/summary/transcript; `Copy markdown` pastes identical content;
  export on a no-turns, no-summary session still writes a document.

## Out of scope

- Chat-session export and History-row export affordances.
- SRT/VTT/PDF/JSON formats, auto-export on stop, a default export
  directory preference.
- WAV/audio export — separate roadmap item (in-app playback first).
