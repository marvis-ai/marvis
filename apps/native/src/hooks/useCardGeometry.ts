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
 * grows/shrinks the window live. The bar row is always the card's
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

export const useCardGeometry = (
  cardRef: RefObject<HTMLDivElement | null>,
  stageRef: RefObject<HTMLDivElement | null>,
  /** Replaces the scrollHeight read while set — the standalone
   *  history card's fixed 60%-of-screen height. Called inside
   *  `report()` so it re-evaluates `screen.availHeight` per report. */
  heightOverride: (() => number) | null = null,
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
  // per call. The observer watches the CARD element — NOT the
  // window-fixed `h-full` stage, whose box only changes on real window
  // resizes, so streaming content growth/shrink actually triggers
  // reports. `scrollHeight` reads the uncapped content height (overflow
  // counts — and a user-stretched window reads as its rendered height,
  // so a manual resize sticks); the backend clamps to the work area's
  // free space. The stage's vertical padding is added back (frost keeps
  // `p-1`, glass strips it) so the report is total window height under
  // both materials.
  useEffect(() => {
    const el = cardRef.current;
    const stage = stageRef.current;
    if (!el || !cardOpen) {
      return;
    }
    let lastValue = -1;
    let lastSentAt = 0;
    let timer: number | undefined;
    const report = () => {
      const cs = stage ? getComputedStyle(stage) : null;
      const padY = cs
        ? parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom)
        : 0;
      const h =
        heightOverride?.() ??
        Math.ceil(el.scrollHeight + (Number.isFinite(padY) ? padY : 0));
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
    // An override keys off screen geometry, not element size — window
    // resizes (monitor moves, work-area changes) must re-report too.
    if (heightOverride) {
      window.addEventListener('resize', report);
    }
    report();
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', report);
      window.clearTimeout(timer);
    };
  }, [cardOpen, cardRef, stageRef, heightOverride]);

  return { cardOpen };
};
