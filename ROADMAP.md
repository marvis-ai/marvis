# Marvis Roadmap

Marvis is becoming a **screen-aware copilot with a meeting assistant built
in** — one that keeps its own memory on your machine, uses tools and
skills, and runs simple agentic loops under your approval. Local-first
stays the default; hosted services arrive only as opt-ins.

This document is the launch plan, the strategy, and the public
contribution map in one. Phases are ordered by **dependency, not date** —
each unlocks what follows. We don't promise timelines.

Status markers: **shipped** · **in progress** · **planned** ·
**exploring** · **won't**

## Where we are (v0.1.x)

Already shipped, verified in the codebase:

- Overlay bar: capsule→input→card morph, edge docking, liquid glass
- Ask: streaming chat with screen frames, vision-provider screen
  descriptions, per-meeting chat context, auto-titles, provider failover
- Listen: dual-channel STT → diarized transcript → rolling summaries,
  voiceprint enrollment, retained WAV recordings
- Dictation into the Ask input; `marvis://` deep links; five rebindable
  global hotkeys; onboarding + per-OS consent gates
- BYOK providers (OpenAI · Anthropic · Gemini · OpenRouter · Ollama ·
  OpenAI-compatible) and STT engines (Deepgram · bundled whisper.cpp ·
  sherpa-onnx SenseVoice)
- Cross-platform ports (macOS / Windows / Linux) with CI-built bundles
  and a local SQLite store under `~/.marvis`

## Phase 0 — Ship-ready

Launch gates. Everything after this phase compounds on top of it —
features shipped to unsigned, un-updatable installs don't compound.

| Item | Status | Why it gates |
| --- | --- | --- |
| macOS Developer ID signing + notarization | planned | `release.yml` ships "unsigned test builds — no Gatekeeper pass"; new users can't open the app |
| Windows code signing (Azure Trusted Signing or cert) | planned | SmartScreen warnings kill installs on the platform just ported |
| Auto-update (`tauri-plugin-updater` + signed manifest on GitHub Releases) | planned | Without it every user is stranded on the build they first downloaded |
| Opt-in crash reporting (default off) | planned | Field quality is currently invisible; default-off preserves the privacy model |
| Release notes automation (per-tag notes in the release job) | planned | `release_notes.md` is manual today |
| Homebrew cask + winget manifests | planned | Friction-free installs once signed |
| Resolve orphaned waitlist code (`functions/`, `db/`, `waitlist-dialog.tsx`) | planned | Dead since the move to GitHub Releases downloads — revive or remove |

## Phase 1 — Deepen the loop

Finish what's already promised, then close the real gaps in Ask and
Listen.

| Item | Status | Notes |
| --- | --- | --- |
| Session history polish | planned | Named in the v0.1.0 notes |
| Prompt presets | shipped | Built-in catalog + custom presets in `config.toml`, glass palette picker + `/name` shorthand, per-send `{input}`/`{lang}` badges — spec: `docs/superpowers/specs/2026-10-07-prompt-presets-design.md`; becomes the seed of skill bundles (Phase 3) |
| Ollama model management | planned | Named in the v0.1.0 notes — pull/remove models from settings |
| Gemini search-grounding toggle | planned | Named in the v0.1.0 notes; first grounded-answer path |
| Summary + transcript export (markdown / clipboard) | in progress | Spec: `docs/superpowers/specs/2026-10-08-transcript-export-design.md`; implemented — markdown copy + `.md` save via `save_text_file`, pending manual QA |
| Session audio: inline playback + WAV export | planned | Follow-up to doc export — `sessions.audio_file` WAVs already retained in `~/.marvis/audios`; one audio surface: play in the doc + "Save audio…" beside it (needs a `save_file_copy`-style command next to `save_text_file`) |
| Rename diarized speakers | planned | "You" is pinned by voiceprint; speakers 1–4 stay anonymous |
| File & image attachments in Ask | exploring | Beyond the screen frame — drag-drop / picker into the composer |

## Phase 2 — Memory

Local-first recall over what Marvis already hears and sees. On-device by
default; provider-hosted embeddings only as an opt-in.

| Item | Status | Notes |
| --- | --- | --- |
| Embedding index over sessions, messages, transcripts, summaries | exploring | sqlite-vec or a bundled model; the corpus already lives in SQLite |
| Recall in Ask — "what did we decide last week?" | exploring | Retrieval wired into the ask pipeline beside `screen_context` |
| Memory browser in the history UI | exploring | Search + browse across meetings and chats |
| Optional OCR screen-memory index | exploring | Rewind-style recall; needs per-app exclusion rules before it's safe to build — biggest privacy surface in the roadmap |

## Phase 3 — Skills & tools

Let Ask act, not just answer. Tools are declared to the model, executed
in the Rust core, and gated by an in-bar approval card — consistent with
the consent-first architecture.

| Item | Status | Notes |
| --- | --- | --- |
| Tool framework in the Rust core (typed declarations + approval gate) | exploring | The execution layer everything else hangs off |
| Safe starter tools: clipboard write, open URL/app, insert text into focused field, notify | exploring | Small, auditable, high-utility |
| MCP client (stdio servers) | exploring | Adopt the industry's tool format instead of inventing one |
| Skill bundles = preset prompt + tool set | exploring | Prompt presets graduate into shareable skills |

## Phase 4 — Agentic loops

Bounded autonomy: observe → propose → approve. Every action rides the
Phase 3 approval card — nothing acts silently.

| Item | Status | Notes |
| --- | --- | --- |
| Meeting detection → suggest Listen | exploring | Frontmost-app heuristic (Zoom / Meet / Teams) |
| Post-meeting loop: summary → action items → drafted follow-ups | exploring | Builds on Listen summaries + export |
| Proactive observe→suggest cycles | exploring | Screen context + memory propose next actions; the user always confirms |

## Non-goals

- **Accounts and cloud sync** — revisit only if an opt-in hosted tier
  ships; the default install stays local
- **Mobile companion**
- **Multi-user / team features**
- **Unbounded OS automation** — agentic behavior stays inside the
  approval-card model

## How to help

- **Phase 0 items** are mostly CI/release work — good first issues for
  contributors with packaging or signing experience
- **Phase 1 items** are scoped, self-contained, and documented in the
  issue tracker — the best place to start contributing
- **Phases 2–4** are `exploring` — jump into the design discussions on
  GitHub issues before writing code; these need their own specs (see
  `docs/superpowers/specs/` for how we write them)

Bug reports and feature requests →
[GitHub issues](https://github.com/MarvisLLC/marvis/issues).
