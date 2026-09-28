# Unified card height — Chat / Listen / History

Date: 2026-09-27
Status: approved

## Goal

One sizing contract for all three bar card sections. Today chat/listen grow to
the work area's free space with no fixed ceiling, while history is a
special-cased standalone surface fixed at 60% of the screen with no bottom
input row. Unify them:

- Card opens at **30% of screen height** (total window height, incl. the 64 px
  bar row). 30% is a floor: the card never shrinks below it while open.
- Content growth raises the window up to **60% of screen height**.
- Beyond 60% the section body scrolls (`PANEL_BODY` already does this).
- History becomes a regular section: same bottom input row (iris close, ask
  field, dictation, settings), same header pattern. No standalone chrome.

"Screen" = `screen.availHeight` in the webview and `Monitor::work_area()` in
Rust — both are the work-area height of the display holding the bar.

## Changes

| File | Change |
| --- | --- |
| `src/hooks/useCardGeometry.ts` | Drop `heightOverride` param. `report()` returns `clamp(measured, 0.30·availHeight, 0.60·availHeight)` where `measured = card.scrollHeight − body.clientHeight + body.scrollHeight + stagePad` — the card is `flex-1` (window-filling, keeps the ShineBorder edge riding the expand animation) and sections self-constrain via `min-h-0`, so card `scrollHeight` echoes rendered height; the section's `data-card-scroll` body carries the true content height. Triggers: `ResizeObserver` on the card + its direct children (chrome growth like the auto-sizing textarea mutates no DOM), `MutationObserver` on the card subtree (streaming/transcript/list updates), `resize` listener (bounds track `availHeight`). |
| `src/components/{Chat,Listen,History}Section.tsx` | Add `data-card-scroll` to each section's scroll body so the hook can read real content height; comment updates in HistorySection (no longer standalone). |
| `src/views/Bar.tsx` | Remove `historyHeight`; `row()` renders for every section; `inputRendered` drops `pinned !== 'history'`; iris label `'Close chat'` → `'Close'`; comment updates. |
| `src/components/HistorySection.tsx` | Comment updates only (no longer standalone). |
| `src-tauri/src/windows/layout.rs` | `CARD_MIN_FRACTION = 0.30`, `CARD_MAX_FRACTION = 0.60`; `expanded_rect` caps height at `min(free, 0.60·work.h)`; floor stays `bar.h + MIN_CHAT_H`. Update `expanded_rect_clamps_height_to_free_space` test. |
| `src-tauri/src/windows/mod.rs` | `set_chat_open(true)` resets `chat_height = 0.30·work.h − BAR_H` (floor `MIN_CHAT_H`) so every open animates to 30%. `sync_bar_size_limits` card band → `[max(30%·work, BAR_H+MIN_CHAT_H), max(min(free, 60%·work), min)]` so edge-drags can't leave the band. |

## Behavior notes

- Reopening a long chat animates pill→30%, then ~50 ms later the first
  `window_adjust_height` report animates 30%→clamped content height (≤60%).
  That's the literal "initialized at 30%" behavior.
- Manual edge-drags are clamped to the [30%, 60%] band via `set_min/max_size`,
  so a drag past the cap can't snap back later.
- Sending an ask from the history section already pins to chat via
  `submitAsk` → `setPinned('chat')`; dictation works there too once
  `inputRendered` includes it.
- History `onBack` still collapses the card (`windowSetChatOpen(false)`),
  matching its current semantics.

## Out of scope

- Tab strip / visible section switcher — sections are still reached via
  capsule controls, hotkeys, and in-card row taps as today.
- Persisting the card height across opens (spec resets to 30% each open).
