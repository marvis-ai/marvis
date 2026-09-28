/**
 * `?view=picker` — the share-picker surface (760×560 borderless glass,
 * `windows/mod.rs` centers it on the pointer's display). Replaces the
 * native SCContentSharingPicker: `capture_pick_list` returns the
 * candidate meta fast, then `picker:thumb` emits fill card images in
 * as each ~480px JPEG lands. Esc or Cancel → `capture_pick_cancel`
 * (Rust re-shows the bar); a card click → `capture_pick_select`
 * (Rust stops any live capture, starts scoped, re-shows the bar).
 * Stale ids reject — show the error and refetch.
 */
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { MonitorIcon, AppWindowIcon, LayoutGridIcon, XIcon } from '@marvis/ui';
import {
  capturePickCancel,
  capturePickList,
  capturePickSelect,
  openDevTools,
  type PickCandidate,
} from '../lib/commands';
import {
  EV_PICKER_OPEN,
  EV_PICKER_THUMB,
  useTauriEvent,
  type PickerThumbPayload,
} from '../lib/events';
import { groupCandidates } from '../lib/pick-groups';

const Picker = () => {
  const [cands, setCands] = useState<PickCandidate[]>([]);
  const [thumbs, setThumbs] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    setError(null);
    setThumbs({});
    void capturePickList()
      .then(setCands)
      .catch(() => setError('Could not list shareable content'));
  }, []);

  // Mount covers the first open (the emit can race this webview's
  // load); picker:open drives every later open.
  useEffect(() => refresh(), [refresh]);
  useTauriEvent(EV_PICKER_OPEN, refresh);
  useTauriEvent<PickerThumbPayload>(EV_PICKER_THUMB, (p) => {
    setThumbs((t) => ({ ...t, [p.id]: p.jpeg }));
  });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        void capturePickCancel().catch(() => {});
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const pick = (id: string) => {
    void capturePickSelect(id).catch((e) => {
      // refresh() clears error synchronously — refetch first, then set
      // so the stale-id message actually paints.
      refresh();
      setError(
        typeof e === 'string' ? `${e} — pick again` : 'Pick failed — try again',
      );
    });
  };
  const cancel = () => void capturePickCancel().catch(() => {});

  const groups = groupCandidates(cands);
  const section = (title: string, icon: ReactNode, items: PickCandidate[]) =>
    items.length > 0 && (
      <section className='mb-3'>
        <header className='mb-1.5 flex items-center gap-1.5 px-0.5 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground'>
          {icon}
          {title}
          <span className='font-normal'>({items.length})</span>
        </header>
        <div className='grid grid-cols-3 gap-2'>
          {items.map((item) => (
            <button
              key={item.id}
              type='button'
              onClick={() => pick(item.id)}
              className='group rounded-lg border border-transparent p-1 text-left transition-colors hover:border-[color-mix(in_oklch,var(--accent)_45%,var(--border))] hover:bg-[color-mix(in_oklch,var(--fg)_6%,transparent)] focus-visible:border-accent focus-visible:outline-none'>
              <div className='aspect-video w-full overflow-hidden rounded-md bg-[color-mix(in_oklch,var(--fg)_7%,transparent)]'>
                {thumbs[item.id] ? (
                  <img
                    src={`data:image/jpeg;base64,${thumbs[item.id]}`}
                    alt=''
                    className='h-full w-full object-cover object-top'
                  />
                ) : (
                  <div className='h-full w-full animate-pulse' />
                )}
              </div>
              <p className='mt-1 truncate text-[12px] font-medium leading-tight'>
                {item.label}
              </p>
              {item.sub && (
                <p className='truncate text-[10.5px] text-muted-foreground'>
                  {item.sub}
                </p>
              )}
            </button>
          ))}
        </div>
      </section>
    );

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
        <header className='flex items-center gap-2 px-3.5 pt-3 pb-2'>
          <p className='flex-1 text-[13.5px] font-semibold'>
            Choose what to record
          </p>
          <button
            type='button'
            onClick={cancel}
            className='rounded-md p-1 text-muted-foreground transition-colors hover:bg-[color-mix(in_oklch,var(--fg)_8%,transparent)] hover:text-foreground'
            title='Cancel (Esc)'
            aria-label='Cancel'>
            <XIcon className='size-4' />
          </button>
        </header>
        <div className='min-h-0 flex-1 overflow-y-auto px-3 pb-2'>
          {section(
            'Screens',
            <MonitorIcon className='size-3.5' />,
            groups.screens,
          )}
          {section(
            'Windows',
            <AppWindowIcon className='size-3.5' />,
            groups.windows,
          )}
          {section(
            'Apps',
            <LayoutGridIcon className='size-3.5' />,
            groups.apps,
          )}
          {cands.length === 0 && !error && (
            <p className='py-10 text-center text-xs text-muted-foreground'>
              Looking for shareable content…
            </p>
          )}
          {error && (
            <p className='py-3 text-center text-xs text-destructive'>{error}</p>
          )}
        </div>
      </div>
    </div>
  );
};

export default Picker;
