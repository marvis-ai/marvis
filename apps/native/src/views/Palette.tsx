/**
 * `?view=palette` — the preset palette: a 300×320 borderless glass
 * overlay `windows/mod.rs` anchors to the bar's inward side (the
 * wand's pick surface, and the list a future skills section joins).
 * One flat preset list — `is_template` is invisible here; a pick arms
 * its badge in the composer either way. ↑/↓ + Enter navigate, a click
 * picks: Rust hides the palette, refocuses the bar, and emits
 * `bar:preset-pick`. Esc (or the click-away blur Rust listens for)
 * closes it menu-style.
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

const Palette = () => {
  const [presets, setPresets] = useState<Preset[]>([]);
  const [sel, setSel] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  const refresh = useCallback(() => {
    setSel(0);
    void presetsList().then(setPresets).catch(() => {});
  }, []);

  // Mount covers the first open (the emit can race this webview's
  // load); palette:open drives every later open.
  useEffect(() => refresh(), [refresh]);
  useTauriEvent(EV_PALETTE_OPEN, refresh);

  // The palette owns focus while open — one window listener is the
  // whole keyboard contract.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        void presetsPaletteClose().catch(() => {});
      } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        setSel((s) =>
          presets.length === 0
            ? 0
            : (s + (e.key === 'ArrowDown' ? 1 : -1) + presets.length) %
              presets.length,
        );
      } else if (e.key === 'Home' || e.key === 'End') {
        e.preventDefault();
        setSel(e.key === 'Home' ? 0 : Math.max(0, presets.length - 1));
      } else if (e.key === 'Enter') {
        e.preventDefault();
        const p = presets[sel];
        if (p) void presetsPaletteSelect(p.id).catch(() => {});
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [presets, sel]);

  // Keep the selected row visible during key nav.
  useEffect(() => {
    listRef.current
      ?.querySelector('[data-sel="true"]')
      ?.scrollIntoView({ block: 'nearest' });
  }, [sel]);

  const idx = presets.length === 0 ? 0 : Math.min(sel, presets.length - 1);

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
        <header className='flex items-center gap-1.5 px-3 pt-2.5 pb-1.5 text-[11px] font-semibold tracking-wide text-muted-foreground uppercase'>
          <WandSparklesIcon className='size-3.5' />
          Presets
        </header>
        <div ref={listRef} className='min-h-0 flex-1 overflow-y-auto px-1.5 pb-1'>
          {presets.map((p, i) => (
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
          {presets.length === 0 && (
            <p className='py-8 text-center text-[11.5px] text-muted-foreground'>
              No presets yet.
            </p>
          )}
        </div>
        <footer className='flex items-center justify-between border-t border-border px-3 py-1.5 text-[10.5px] text-muted-foreground'>
          <span>↑↓ choose · ↵ pick · esc close</span>
          <span>{presets.length}</span>
        </footer>
      </div>
    </div>
  );
};

export default Palette;
