/**
 * Card-mode window geometry for the bar.
 *
 * `cardOpen` is read off the window itself: `innerHeight > BAR_H` means
 * Rust expanded us — `set_chat_open` emits nothing by design, so the
 * `resize` event is the single open/close signal for every path
 * (tray Toggle, ask send, `ask_close`, `window_set_chat_open`).
 *
 * While the card is open the hook also reports its desired TOTAL window
 * height back to Rust (`window_adjust_height`) so streaming content
 * grows/shrinks the window live. The report is clamped to the unified
 * card band — [30%, 60%] of `screen.availHeight` — so every section
 * (chat | listen | history) opens at the floor, grows with content, and
 * scrolls its own body past the cap. The bar row is always the card's
 * bottom-anchored footer (Bar.tsx lays out `flex-col-reverse`), so no
 * grow-direction detection is needed here.
 */
import { useEffect, useState } from 'react';
import type { RefObject } from 'react';
import { windowAdjustHeight } from '@/lib/commands';

/** The bar row's height — the pill's window height in pill modes and
 *  the card header's height in card modes (spec: 64). */
const BAR_H = 64;
/** Card-open read: any window taller than the pill is a card. */
const OPEN_EPS = 2;
/** Reported-height deadband + invoke throttle (was AskPanel's). */
const HEIGHT_EPS = 4;
const HEIGHT_MS = 150;
/** Unified card band as fractions of the screen's available height —
 *  the window floor on open, the cap where the section body takes over
 *  with its own scroll. */
const CARD_MIN_FRAC = 0.3;
const CARD_MAX_FRAC = 0.6;

export const useCardGeometry = (
  cardRef: RefObject<HTMLDivElement | null>,
  stageRef: RefObject<HTMLDivElement | null>,
) => {
  const [cardOpen, setCardOpen] = useState(
    () => window.innerHeight > BAR_H + OPEN_EPS,
  );

  // Card open/close is learned from the window itself: any height
  // change — expand, collapse, drag-resize — re-reads innerHeight.
  useEffect(() => {
    const read = () => setCardOpen(window.innerHeight > BAR_H + OPEN_EPS);
    window.addEventListener('resize', read);
    read();
    return () => window.removeEventListener('resize', read);
  }, []);

  // Report the card's desired TOTAL window height: leading + trailing
  // throttle, only on a real (>EPS) change — `adjust_height` animates
  // per call. The card is `flex-1` — it fills the window so its bottom
  // edge rides the expand animation — and each section's scroll body
  // (`data-card-scroll`) is `min-h-0`, so the card's `scrollHeight`
  // only ever echoes its rendered height. The real content height
  // lives on `data-card-content` — a `flow-root` wrapper inside the
  // scroll body whose own box is exactly the content (scrollHeight
  // can't read content smaller than the scroller's box, so shrinking
  // — cleared chat, a shorter section — would go unreported without
  // it). Desired = chrome + padded content:
  // `card.scrollHeight − body.clientHeight + bodyPad + content`, then
  // clamped to the [30%, 60%] band of `screen.availHeight`; the backend
  // re-clamps to the work area's free space. Observing the wrapper's
  // box catches content changes that resize no watched element
  // (streamed tokens, transcript turns); the card's direct children
  // are observed for chrome growth that mutates no DOM (the
  // auto-sizing textarea); a MutationObserver re-resolves the marked
  // elements after section swaps. The stage's vertical padding is
  // added back (frost keeps `p-1`, glass strips it) so the report is
  // total window height under both materials.
  useEffect(() => {
    const el = cardRef.current;
    const stage = stageRef.current;
    if (!el || !cardOpen) {
      return;
    }
    let lastValue = -1;
    let lastSentAt = 0;
    let timer: number | undefined;
    const watched = new Set<Element>();
    const watch = (node: Element | null) => {
      if (node && !watched.has(node)) {
        watched.add(node);
        observer.observe(node);
      }
    };
    const report = () => {
      // Re-resolve the measured elements — section swaps replace them.
      for (const child of el.children) {
        watch(child);
      }
      const body = el.querySelector<HTMLElement>('[data-card-scroll]');
      const content = el.querySelector<HTMLElement>('[data-card-content]');
      watch(content);
      const cs = stage ? getComputedStyle(stage) : null;
      const padY = cs
        ? parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom)
        : 0;
      const bcs = body ? getComputedStyle(body) : null;
      const bodyPadY = bcs
        ? parseFloat(bcs.paddingTop) + parseFloat(bcs.paddingBottom)
        : 0;
      const measured = Math.ceil(
        (body && content
          ? el.scrollHeight -
            body.clientHeight +
            (Number.isFinite(bodyPadY) ? bodyPadY : 0) +
            content.offsetHeight
          : el.scrollHeight) + (Number.isFinite(padY) ? padY : 0),
      );
      const h = Math.min(
        Math.max(measured, window.screen.availHeight * CARD_MIN_FRAC),
        window.screen.availHeight * CARD_MAX_FRAC,
      );
      if (Math.abs(h - lastValue) <= HEIGHT_EPS) {
        return;
      }
      const wait = HEIGHT_MS - (Date.now() - lastSentAt);
      if (wait <= 0) {
        lastValue = h;
        lastSentAt = Date.now();
        void windowAdjustHeight(h).catch(() => {});
      } else if (timer === undefined) {
        timer = window.setTimeout(() => {
          timer = undefined;
          report();
        }, wait);
      }
    };
    const observer = new ResizeObserver(report);
    observer.observe(el);
    const mutations = new MutationObserver(report);
    mutations.observe(el, {
      childList: true,
      characterData: true,
      subtree: true,
    });
    // The clamp bounds key off screen geometry — window resizes
    // (monitor moves, work-area changes) must re-report too.
    window.addEventListener('resize', report);
    report();
    return () => {
      observer.disconnect();
      mutations.disconnect();
      window.removeEventListener('resize', report);
      window.clearTimeout(timer);
    };
  }, [cardOpen, cardRef, stageRef]);

  return { cardOpen };
};
