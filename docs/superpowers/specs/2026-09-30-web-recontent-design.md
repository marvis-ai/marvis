# Marvis Web — Landing Re-content (Meeting Hero + Current-Feature Copy)

Date: 2026-09-30
Status: Draft (pending written-spec review)
Scope: `apps/web` + `DESIGN.md` (doc updated where it describes the old
scenes; its tokens/components stay authoritative). `assets/marvis-landing.html`
is a standalone prototype and is not part of this pass.

## Goal

Re-content the Next.js landing page so every claim matches the app as it
ships today, to a broadened audience ("Private / Personal AI for All" —
meetings, docs, browsing; no longer dev-led). The hero's code-editor scene
is replaced by a meeting scene foregrounding Listen.

## User decisions

| Question | Decision |
| --- | --- |
| Hero scene | Meeting scene — bar + live-transcript Listen card over a generic meeting window |
| Audience | Broaden — developers no longer the primary persona |
| Approach | A — full re-content: faithful mocks + every stale claim fixed |
| Design authority | `DESIGN.md` governs; stale sections updated rather than frozen |

## Stale claims on the live page (ground truth from apps/native)

| Page says | App actually does |
| --- | --- |
| Hero scene = code editor (`capture/mod.rs`), lead addresses "developers and power users" | App is a general overlay — Ask + Listen + dictation for anyone |
| Listen "arrives in Phase 2" (`MvListen` = muted waveform chrome) | Listen ships: mic + system audio → live speaker-labeled transcript → rolling TLDR every 5 turns, pause/resume/stop, speaker filter, copy, finished docs in History |
| Capsule = camera + mic "Coming soon" | Capsule = iris + screen-capture toggle (`MonitorDotIcon`) + Listen recorder (`MicAudioLinesIcon`); expanded bar has working dictation |
| "Four global chords" — `⌘/` `⌘⏎` `⌘⇧S` `⌘,` | Five rebindable globals: `⌘⌥Space` input · `⌘⌥R` capture · `⌘⌥T` listen · `⌘⌥H` history · `⌘⇧L` lock. In-bar fixed: `⌘,` settings · `Enter` send · `⇧Enter` newline · `⌘Enter` send+frame |
| Providers: OpenAI/Anthropic/Gemini/Ollama | + OpenRouter + OpenAI-compatible; drag-to-rank failover chain; optional vision provider; STT = Deepgram (nova-2 default) / local whisper.cpp (tiny·base·small) / sherpa (sense-voice) |
| `models/` "planned" in `~/.marvis` tree | Real — whisper ggml models downloaded on demand; `marvis.db` also holds `transcripts` + `summaries` |
| Listen panel is a separate 400px window | Card modes (chat · listen · history) are one 600px band grown inside the bar window |

## Hero — meeting scene

Keeps `.scene` + `.marvis-bar` float; `.editor` becomes `.meet`:

```text
.scene
  MvBar.marvis-bar            (floating capsule→bar, unchanged mechanics)
  .meet                       (replaces .editor — same card chrome treatment)
    .meet-chrome              (dots + "Design sync" — generic, no trademarks)
    .meet-body
      .meet-grid              (4 video tiles: avatar initials + name labels,
                              defocused like .ghost-win)
      .meet-toolbar           (mic / cam / leave silhouettes)
    MvListen.marvis-panel     (absolute bottom-right, same anchoring as before)
```

The ask demo moves: the panel is now `MvListen` (live meeting transcript),
not `MvAskPanel` over code. Ask remains demoed in `#interface`.

## `mv.tsx` changes

| Mock | Change |
| --- | --- |
| `MvListen` | Rebuilt to `ListenSection.tsx`: header (title `Design sync` + subtitle `mic + system audio · deepgram nova-2` + `LISTENING` chip with ping dot) → speaker-filter row (`all` `you` `speaker 1` chips + `{n} lines` + `m:ss` + copy icon) → 3 transcript blocks (`You` in slate accent, `Speaker 1` in speaker-1 teal, last block carries dimmed interim + caret) → `TLDR · topic` strip (2-line clamp) |
| `MvBar` mini | iris + monitor (capture toggle) + mic-audio-lines (Listen), per current `Bar.tsx` capsule |
| `MvBar` main | keep input + dictation mic (no longer `is-off`/"Coming soon") + gear |

New `.mv-*` CSS: transcript block (`m:ss` mono stamp · colored dot · name ·
paragraph), speaker chips, `LISTENING` chip + ping, TLDR strip — plus
`--mv-accent: oklch(0.53 0.08 237)` and `--mv-speaker-1..4` copied verbatim
from `apps/native/src/index.css` (light set; dark swap under
`.shots[data-theme="dark"]`).

## Section-by-section copy

| Section | Change |
| --- | --- |
| Hero | h1 → screen + meetings framing for everyone (e.g. "Ask anything about what's on your screen — and transcribe every meeting. All of it stays on your machine."); lead drops "developers and power users"; tags keep Free/MIT · No Account · No Cloud Sync |
| Features | 3 real pillars: **Ask** (⌘⏎ screen-aware, streams markdown) · **Listen** (mic + system audio → speaker labels → TLDR every 5 turns) · **Private by design** (no account, keys masked `…last4`, BYO providers) |
| Privacy (dark) | Copy stands; filetree fixes — `models/whisper/` = downloaded ggml models (no "planned"), `marvis.db` comment adds transcripts + summaries |
| Interface | Lead + Listen copy updated ("ships today"); composite shot keeps real dims; Listen figure swaps the Phase-2 chrome for the new transcript mock at its real card width; desktop ghost window swaps the github ghost for a generic doc/notes window (broad-audience neutral) |
| Hotkeys | Table rebuilt: two groups — *Global (rebindable)* `⌘⌥Space` input / `⌘⌥R` capture / `⌘⌥T` listen / `⌘⌥H` history / `⌘⇧L` lock; *In the bar (fixed)* `Enter` send / `⇧Enter` newline / `⌘Enter` send+frame / `⌘,` settings. Deep-link card stays |
| Providers | Tags += OpenRouter + "OpenAI-compatible"; copy covers failover chain (drag-rank, per-provider toggle), optional vision provider; new STT row: Deepgram streaming · bundled whisper.cpp · sherpa (sense-voice). Platform rows unchanged (macOS now · Win/Linux in dev) |
| Nav | Unchanged — Features · Privacy · Interface · Hotkeys · Providers (Listen surfaces inside Hero + Interface, no new anchor) |
| Download CTA | h2 stays or "Your screen. Your meetings. Your machine."; lead unchanged in substance |

## CSS

- Delete `.editor*` block (`.editor`, `-chrome`, `-body`, `-side`, `-code`,
  syntax tints) — no longer referenced.
- Add `.meet*` (chrome/body/grid/tile/toolbar) + transcript `.mv-*` styles in
  the same token/motion idiom; `.marvis-panel` anchoring reused as-is.
- `prefers-reduced-motion` still kills caret/ping/blink.

## DESIGN.md updates

§6 (`.mv-*` geometry: Listen is a 600px card mode, not a 400px window) ·
§7 (hero scene → meeting; shot inventory) · §9 (provenance — speaker colors,
STT list, hotkey set) · §10 (section order — hero over meeting scene).
All other tokens/typography/motion rules unchanged and binding.

## Verification

`bun run lint` + `bun run check-types` + `bun run build` in `apps/web` clean;
visual pass at 1180/920/640 breakpoints; every product claim traceable to
`apps/native` source (no invented metrics).

## Out of scope

- `assets/marvis-landing.html` (prototype frozen as shipped)
- `apps/native` code
- New sections (no dedicated Listen section — story lives in Hero + Features + Interface)
