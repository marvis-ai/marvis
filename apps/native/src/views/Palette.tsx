/**
 * `?view=palette` — the preset palette: a 300×320 borderless glass
 * overlay `windows/mod.rs` anchors left-aligned to the composer
 * caret, above/below the bar by free space (the wand's pick surface,
 * and the list a future skills section joins). One flat preset list —
 * `is_template` is invisible here; a pick arms its badge in the
 * composer either way. The header field owns the `/` query the whole
 * time the palette is open (a typed `/` opens it focused): typing
 * filters, ↑/↓ + Enter/Tab navigate and pick, Backspace on an empty
 * filter untypes the `/`, Esc writes `/query` back into the composer
 * — both close paths ride `presets_palette_close` →
 * `bar:palette-closed`. Click-away blur is the menu-style dismiss.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { WandSparklesIcon } from '@marvis/ui';
import {
  openDevTools,
  presetsList,
  presetsPaletteClose,
  presetsPaletteSelect,
  type Preset,
} from '@/lib/commands';
import { EV_PALETTE_OPEN, useTauriEvent } from '@/lib/events';
import { presetToken } from '@/lib/presets';

/** Filter predicate — case-insensitive substring on the display name
 *  OR its `/token` slug (`translate` hits `Translate`, `reply-nicely`
 *  hits `Reply nicely`). A leading `/` in the field is stripped —
 *  the composer keeps the trigger, the field holds only the query. */
const normalize = (q: string): string =>
  q.trim().replace(/^\/+/, '').toLowerCase();

const matches = (p: Preset, q: string): boolean =>
  p.name.toLowerCase().includes(q) || presetToken(p.name).includes(q);

const Palette = () => {
  const [presets, setPresets] = useState<Preset[]>([]);
  const [query, setQuery] = useState('');
  const [sel, setSel] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const fieldRef = useRef<HTMLInputElement>(null);

  const refresh = useCallback((seed?: string | null) => {
    setSel(0);
    setQuery(seed ?? '');
    void presetsList()
      .then(setPresets)
      .catch(() => {});
    // autoFocus covers only the window's first mount — a reopened
    // palette (hidden, not rebuilt) re-focuses the field explicitly.
    requestAnimationFrame(() => fieldRef.current?.focus());
  }, []);

  // Mount covers the first open (the emit can race this webview's
  // load); palette:open drives every later open and carries the
  // `/token` seed.
  useEffect(() => refresh(), [refresh]);
  useTauriEvent<{ query: string | null }>(EV_PALETTE_OPEN, ({ query }) =>
    refresh(query),
  );

  const filtered =
    normalize(query) === ''
      ? presets
      : presets.filter((p) => matches(p, normalize(query)));

  // The palette owns focus while open — one window listener is the
  // whole keyboard contract (the filter field's keys bubble here).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // An in-flight IME composition owns its keys — Enter commits the
      // marked text, Backspace edits it, Esc cancels it; none of those
      // may reach the menu (same 229 signal as the composer).
      if (e.isComposing || e.keyCode === 229) return;
      if (e.key === 'Escape') {
        e.preventDefault();
        // Esc keeps the typed filter — the composer gets `/query`
        // back so typing resumes where the menu left off.
        void presetsPaletteClose(normalize(query)).catch(() => {});
      } else if (e.key === 'Backspace' && normalize(query) === '') {
        e.preventDefault();
        // Backspace on an empty filter untypes the `/` trigger —
        // `null` drops the composer's whole `/token`.
        void presetsPaletteClose(null).catch(() => {});
      } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        setSel((s) =>
          filtered.length === 0
            ? 0
            : (s + (e.key === 'ArrowDown' ? 1 : -1) + filtered.length) %
              filtered.length,
        );
      } else if (e.key === 'Home' || e.key === 'End') {
        e.preventDefault();
        setSel(e.key === 'Home' ? 0 : Math.max(0, filtered.length - 1));
      } else if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        const p = filtered[Math.min(sel, filtered.length - 1)];
        if (p) {
          void presetsPaletteSelect(p.id).catch(() => {});
        } else if (e.key === 'Enter') {
          // Nothing to pick — dismiss and hand `/query` back so the
          // composer can send it as literal text.
          void presetsPaletteClose(normalize(query)).catch(() => {});
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [filtered, sel, query]);

  // Keep the selected row visible during key nav.
  useEffect(() => {
    listRef.current
      ?.querySelector('[data-sel="true"]')
      ?.scrollIntoView({ block: 'nearest' });
  }, [sel, filtered.length]);

  const idx = filtered.length === 0 ? 0 : Math.min(sel, filtered.length - 1);

  return (
    <div
      className='glass-stage h-full p-1.5'
      onContextMenu={(e) => {
        if (import.meta.env.DEV) {
          e.preventDefault();
          void openDevTools().catch(() => {});
        }
      }}>
      <div className='glass-surface flex h-full flex-col rounded-2xl border border-border bg-[color-mix(in_oklch,var(--surface)_94%,transparent)] shadow-[0_24px_60px_-20px_color-mix(in_oklch,var(--fg)_40%,transparent)] backdrop-blur-xl'>
        <header className='flex items-center gap-1.5 px-3 pt-2.5 pb-1.5'>
          <WandSparklesIcon className='size-3.5 flex-none text-muted-foreground' />
          <input
            ref={fieldRef}
            autoFocus
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setSel(0);
            }}
            placeholder='Filter presets…'
            aria-label='Filter presets'
            className='min-w-0 flex-1 bg-transparent text-[12px] font-medium text-foreground caret-accent outline-none placeholder:font-normal placeholder:text-muted-foreground'
          />
        </header>
        <div
          ref={listRef}
          className='min-h-0 flex-1 overflow-y-auto px-1.5 pb-1'>
          {filtered.map((p, i) => (
            <button
              key={p.id}
              type='button'
              data-sel={i === idx || undefined}
              onMouseEnter={() => setSel(i)}
              onClick={() => void presetsPaletteSelect(p.id).catch(() => {})}
              title={p.text}
              className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[12.5px] transition-colors duration-(--motion-fast) ease-(--ease) ${
                i === idx ? 'bg-fg-soft text-foreground' : 'text-foreground/90'
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
                : `No match for “${normalize(query)}”.`}
            </p>
          )}
        </div>
        <footer className='flex items-center justify-between border-t border-border px-3 py-1.5 text-[10.5px] text-muted-foreground'>
          <span>↑↓ choose · ↵ pick · esc close</span>
          <span>{filtered.length}</span>
        </footer>
      </div>
    </div>
  );
};

export default Palette;
