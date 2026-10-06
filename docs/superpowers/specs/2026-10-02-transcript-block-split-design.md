# Listen transcript quality — design

Two fixes to the Listen transcript view: bounded block splitting and Chinese
punctuation restoration.

## Part 1 — Block splitting (done)

`buildBlocks` merged every consecutive same-speaker turn into one `TurnBlock`
with a single `m:ss` header — a 3–4 min monologue rendered as one giant
paragraph. Now a new block starts when any of:

- speaker identity changes (existing)
- `turn.ts - block.ts > 30` (`BLOCK_MAX_SPAN_SECS`) — span cap
- `turn.ts - lastActivityTs > 15` (`BLOCK_PAUSE_SECS`) — pause gap, where
  `lastActivityTs` is the riding interim's `ts`, else the last final's `ts`,
  else the block's `ts`

Display-model only; persisted turns untouched; live + viewed sessions share
the path. Implemented in `apps/native/src/components/listen/model.ts` with
tests in `model.test.ts`.

## Part 2 — Chinese punctuation restoration

### Root cause

SenseVoice `use_itn` is a no-op with the `sherpa-onnx-sense-voice-…-int8-
2025-09-09` model the catalog pins — reproduced locally (identical output
for itn on/off, `auto`/`zh`) and confirmed upstream (k2-fsa/sherpa-onnx#2742:
ITN works on the 2024-07-17 export, broken on 2025-09-09). The `punct-en`
CNN-BiLSTM model deliberately skips non-Latin text, so zh segments arrive
completely unpunctuated. Verified: the zh-en CT-Transformer punct model
(`sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8`,
72MB) restores `，。？` on real transcript text but leaves English ALL-CAPS —
so `punct-en` stays for Latin text.

### Changes

**`sherpa_models.rs`** — new catalog entry `punct-zh`
(`SherpaModelId::PunctZh`, kind `Punctuation`, single `model.int8.onnx`,
sha256 `65a3fb9f…`, verified byte-identical to the official GitHub release
tarball). `punctuation_paths(root)` hands the worker the installed en/zh
pair.
`ManagerState.punct_attempted` becomes a `Vec<SherpaModelId>`;
`ensure_punct` attempts the first missing, unattempted punctuation entry.
`ManagerState` moves behind `Arc<Mutex<_>>` so the download task can clear
`state.active` (identity-checked by `Arc::ptr_eq` on the progress cell)
*before* calling `refresh_speech_setup` — the re-entrant `ensure_punct`
then chains to the next missing punct entry in the same wake instead of
waiting a full run.

**`sherpa.rs`** — load `OfflinePunctuation` (CT-Transformer) alongside the
existing `OnlinePunctuation`. `restore_punct_and_case` routes: any CJK
ideograph → zh model (passthrough if absent); else ASCII letters → en
lowercase+punct path; else passthrough. Dictation inherits the fix via the
shared provider path.

**`VoiceSetup.tsx`** — punct card renders per punctuation entry
(`.find` → `.filter` + map); labels come from the catalog.

### Plan

1. `sherpa_models.rs` — catalog entry + path accessor + ensure_punct
   chain. Verify: existing tests updated, new chain test.
2. `sherpa.rs` — dual punctuators + routing. Verify: unit tests on the
   routing function (no-model passthrough), `cargo test`.
3. `VoiceSetup.tsx` — per-entry punct cards. Verify: `bun run
   check-types`.
