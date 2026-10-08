/**
 * `?view=palette` — the preset palette: a 300px-wide borderless glass
 * overlay `windows/mod.rs` anchors left-aligned to the composer
 * caret, above/below the bar by free space (the wand's pick surface,
 * and the list a future skills section joins). One flat preset list —
 * `is_template` is invisible here; a pick arms its badge in the
 * composer either way. Height auto-fits: this view reports its
 * natural content height (`presets_palette_height`) and the window
 * hugs it, capped at `PALETTE_MAX_H` where the list scrolls.
 *
 * Two focus modes: the wand/right-click open takes key focus (this
 * window's keydown drives nav, click-away blur dismisses); a
 * `/`-typed open stays UNFOCUSED — the composer keeps the `/token`,
 * which streams in as `palette:query` (the filter lives in the input,
 * not here) while `palette:key` carries forwarded ↑↓/Enter/Tab/Esc.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { WandSparklesIcon } from '@marvis/ui';
import {
  openDevTools,
  presetsList,
  presetsPaletteClose,
  presetsPaletteHeight,
  presetsPaletteSelect,
  type Preset,
} from '@/lib/commands';
import {
  EV_PALETTE_KEY,
  EV_PALETTE_OPEN,
  EV_PALETTE_QUERY,
  useTauriEvent,
} from '@/lib/events';
import { PALETTE_KEYS, presetToken } from '@/lib/presets';

/** Filter predicate — case-insensitive substring on the display name
 *  OR its `/token` slug (`trans` hits `Translate`, `reply-nicely`
 *  hits `Reply nicely`). */
const matches = (p: Preset, q: string): boolean =>
  p.name.toLowerCase().includes(q) || presetToken(p.name).includes(q);

const Palette = () => {
  const [presets, setPresets] = useState<Preset[]>([]);
  const [query, setQuery] = useState('');
  const [sel, setSel] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  /** Latest-refs for the key handler — `palette:key` and the window
   *  listener share `handleKey`, and event callbacks registered once
   *  must not go stale. */
  const filteredRef = useRef<Preset[]>([]);
  const selRef = useRef(0);

  // Auto-fit: report the natural content height and Rust sizes the
  // window to it. The surface is `h-fit` BUT `max-h-full` caps it at
  // the window when content overflows — capped boxes then never
  // resize again, so measuring THEM would ratchet the window stuck
  // small with a scrolling list. `list.scrollHeight` always reports
  // the full content height (it's the scroller), and the `flow-root`
  // wrapper inside the scroll port keeps a resizable box for the
  // observer while capped. Report = list content + surface chrome
  // (header + borders) + the stage's padding — measured, not
  // hardcoded, because a native material strips `.glass-stage`'s
  // `p-1.5` to 0 (a phantom +12 left bare glass below the surface).
  useEffect(() => {
    const el = surfaceRef.current;
    const content = contentRef.current;
    if (!el || !content) return;
    const report = () => {
      const stage = getComputedStyle(el.parentElement as HTMLElement);
      const padY =
        parseFloat(stage.paddingTop) + parseFloat(stage.paddingBottom) || 0;
      const list = listRef.current;
      if (!list) return;
      const chrome = el.offsetHeight - list.offsetHeight;
      void presetsPaletteHeight(list.scrollHeight + chrome + padY).catch(
        () => {},
      );
    };
    const ro = new ResizeObserver(report);
    ro.observe(el);
    ro.observe(content);
    report();
    return () => ro.disconnect();
  }, []);

  const refresh = useCallback((seed?: string | null) => {
    setSel(0);
    setQuery(seed ?? '');
    void presetsList()
      .then(setPresets)
      .catch(() => {});
  }, []);

  // Mount covers the first open (the emit can race this webview's
  // load); palette:open drives every later open and carries the
  // `/token` seed.
  useEffect(() => refresh(), [refresh]);
  useTauriEvent<{ query: string | null }>(EV_PALETTE_OPEN, ({ query }) =>
    refresh(query),
  );
  useTauriEvent<{ query: string }>(EV_PALETTE_QUERY, ({ query }) => {
    setQuery(query);
    setSel(0);
  });

  const q = query.trim().toLowerCase();
  const filtered = q === '' ? presets : presets.filter((p) => matches(p, q));
  const idx = filtered.length === 0 ? 0 : Math.min(sel, filtered.length - 1);
  filteredRef.current = filtered;
  selRef.current = idx;

  /** One keyboard contract for both modes — focused-mode keys arrive
   *  on the window listener, `/`-mode keys arrive forwarded on
   *  `palette:key`. */
  const handleKey = useCallback((key: string) => {
    const list = filteredRef.current;
    if (key === 'Escape') {
      void presetsPaletteClose().catch(() => {});
    } else if (key === 'ArrowDown' || key === 'ArrowUp') {
      setSel((s) =>
        list.length === 0
          ? 0
          : (s + (key === 'ArrowDown' ? 1 : -1) + list.length) % list.length,
      );
    } else if (key === 'Home' || key === 'End') {
      setSel(key === 'Home' ? 0 : Math.max(0, list.length - 1));
    } else if (key === 'Enter' || key === 'Tab') {
      const p = list[selRef.current];
      if (p) void presetsPaletteSelect(p.id).catch(() => {});
    }
  }, []);

  useTauriEvent<{ key: string }>(EV_PALETTE_KEY, ({ key }) => handleKey(key));

  // Focused-mode keys — the unfocused `/` session never focuses this
  // window, so nothing double-fires.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing || e.keyCode === 229) return;
      if (PALETTE_KEYS.includes(e.key)) {
        e.preventDefault();
        handleKey(e.key);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [handleKey]);

  // Keep the selected row visible during key nav.
  useEffect(() => {
    listRef.current
      ?.querySelector('[data-sel="true"]')
      ?.scrollIntoView({ block: 'nearest' });
  }, [sel, filtered.length]);

  return (
    <div
      className='glass-stage h-full p-1.5'
      onContextMenu={(e) => {
        if (import.meta.env.DEV) {
          e.preventDefault();
          void openDevTools().catch(() => {});
        }
      }}>
      <div
        ref={surfaceRef}
        className='glass-surface flex h-fit max-h-full flex-col rounded-2xl border border-border bg-[color-mix(in_oklch,var(--surface)_94%,transparent)] shadow-[0_24px_60px_-20px_color-mix(in_oklch,var(--fg)_40%,transparent)] backdrop-blur-xl'>
        <header className='flex items-center gap-1.5 px-3 pt-2.5 pb-1.5 text-[11px] font-semibold tracking-wide text-muted-foreground uppercase'>
          <WandSparklesIcon className='size-3.5' />
          Presets
          {q !== '' && (
            <span className='ml-auto font-mono text-[10.5px] font-normal normal-case tracking-normal'>
              /{q}
            </span>
          )}
        </header>
        <div
          ref={listRef}
          className='min-h-0 flex-1 overflow-y-auto px-1.5 pb-1'>
          <div
            ref={contentRef}
            className='flow-root'>
            {filtered.map((p, i) => (
              <button
                key={p.id}
                type='button'
                data-sel={i === idx || undefined}
                onMouseEnter={() => setSel(i)}
                onClick={() => void presetsPaletteSelect(p.id).catch(() => {})}
                title={p.text}
                className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[12.5px] transition-colors duration-(--motion-fast) ease-(--ease) ${
                  i === idx
                    ? 'bg-fg-soft text-foreground'
                    : 'text-foreground/90'
                }`}>
                <span className='min-w-0 flex-1 truncate font-medium'>
                  {p.name}
                </span>
                <span className='flex-none font-mono text-[10.5px] text-muted-foreground'>
                  /{presetToken(p.name)}
                </span>
              </button>
            ))}
            {filtered.length === 0 && (
              <p className='py-8 text-center text-[11.5px] text-muted-foreground'>
                {presets.length === 0
                  ? 'No presets yet.'
                  : `No match for “${q}”.`}
              </p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

export default Palette;
