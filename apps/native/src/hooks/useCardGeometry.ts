/**
 * Card-mode window geometry for the bar.
 *
 * `cardOpen` is read off the window itself: `innerHeight > BAR_H` means
 * Rust expanded us — `set_chat_open` emits nothing by design, so the
 * `resize` event is the single open/close signal for every path
 * (tray Toggle, ask send, `ask_close`, `window_set_chat_open`).
 *
 * `growDir` is detected at expand time: the anchored edge is fixed for
 * grow-down and rises for grow-up, so the first expanded y-read compared
 * to the last collapsed y gives the direction. While collapsed the
 * baseline refreshes on every `tauri://move`/`resize` tick.
 *
 * While the card is open the hook also reports its desired TOTAL window
 * height back to Rust (`window_adjust_height`) so streaming content
 * grows/shrinks the window live.
 */
import { useEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
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
) => {
  const [cardOpen, setCardOpen] = useState(
    () => window.innerHeight > BAR_H + OPEN_EPS,
  );
  const [growDir, setGrowDir] = useState<'up' | 'down'>('down');
  /** Last collapsed-mode outer y — the baseline the expand direction
   *  is detected against. */
  const collapsedY = useRef<number | null>(null);

  // Card open/close is learned from the window itself; grow direction
  // from the y-delta at expand time. While collapsed the baseline y
  // refreshes on move + resize ticks (a drag moves without resizing).
  useEffect(() => {
    const win = getCurrentWindow();
    let alive = true;
    const unMove = win.onMoved((e) => {
      if (window.innerHeight <= BAR_H + OPEN_EPS) {
        collapsedY.current = e.payload.y;
      }
    });
    const read = () => {
      const openNow = window.innerHeight > BAR_H + OPEN_EPS;
      void win
        .outerPosition()
        .then((p) => {
          if (!alive) {
            return;
          }
          setCardOpen((was) => {
            if (openNow && !was && collapsedY.current !== null) {
              setGrowDir(p.y < collapsedY.current - 0.5 ? 'up' : 'down');
            }
            return openNow;
          });
          if (!openNow) {
            collapsedY.current = p.y;
          }
        })
        .catch(() => setCardOpen(openNow));
    };
    window.addEventListener('resize', read);
    read();
    return () => {
      alive = false;
      window.removeEventListener('resize', read);
      void unMove.then((u) => u());
    };
  }, []);

  // Report the card's desired TOTAL window height: leading + trailing
  // throttle, only on a real (>EPS) change — `adjust_height` animates
  // per call. The observer watches the CARD element (content-sized,
  // capped at CARD_MAX) — NOT the window-fixed `h-full` stage, whose
  // box only changes on real window resizes, so streaming content
  // growth/shrink actually triggers reports. `scrollHeight` reads the
  // uncapped content height (overflow counts); the backend clamps to
  // min(900, free). The stage's vertical padding is added back (frost
  // keeps `p-1`, glass strips it) so the report is total window height
  // under both materials.
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
      const h = Math.min(
        Math.ceil(el.scrollHeight + (Number.isFinite(padY) ? padY : 0)),
        900,
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
    report();
    return () => {
      observer.disconnect();
      window.clearTimeout(timer);
    };
  }, [cardOpen, cardRef, stageRef]);

  return { cardOpen, growDir };
};
