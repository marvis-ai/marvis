/**
 * Providers — three surfaces under one tab strip (`Tabs` from
 * `@marvis/ui`, styled onto the overlay palette: `bg-muted` is a TEXT
 * color here, so the list/triggers ride `fg-soft`/`surface` instead).
 *
 * - LLM: the failover list. Row order IS the priority chain
 *   (`providers.order`): the topmost enabled, configured provider
 *   answers, and a failed/unusable one falls through to the next. Drag
 *   a row's grip to re-rank; the switch on the right toggles a provider
 *   out of the chain without deleting its key (`providers.disabled`).
 *   Native HTML5 drag-and-drop — the grip is the drag source (whole-row
 *   `draggable` would break the body's text inputs); ↑/↓ on a focused
 *   grip moves the row for keyboard users.
 * - Vision: the dedicated screen reader (`[vision]`) — see
 *   `VisionSection`.
 * - Voice: the STT provider — the Deepgram row that used to be pinned
 *   at the bottom of the LLM list (Phase 2, not part of the chain).
 *
 * `data.selected` marks the chain head ("primary" tag) — it's
 * `model_get_selected`'s resolved answer, recomputed server-side on
 * every reorder/switch/model write.
 */
import { useState } from 'react';
import type { DragEvent, KeyboardEvent } from 'react';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@marvis/ui';
import {
  modelGetSelected,
  providersReorder,
  providerSetEnabled,
} from '../../lib/commands';
import { orderedProviders } from '../../lib/providers';
import { H2, PROV_CARD, SUB, cn } from '../../lib/classes';
import { ProviderCard } from './ProviderCard';
import { VisionSection } from './VisionSection';
import type { PrefsData } from './types';

/** Every mutation that can change the chain head also re-resolves it —
 * reorder/enable writes return the config but not the selection, and a
 * new head is exactly what the "primary" tag needs to reflect. */
const refreshSelection = (data: PrefsData) => {
  void modelGetSelected()
    .then(data.setSelected)
    .catch(() => {});
};

type SubTab = 'llm' | 'vision' | 'voice';

const SUB_TABS: { id: SubTab; label: string }[] = [
  { id: 'llm', label: 'LLM' },
  { id: 'vision', label: 'Vision' },
  { id: 'voice', label: 'Voice' },
];

export const ProvidersTab = ({ data }: { data: PrefsData }) => {
  const { config } = data;
  const [sub, setSub] = useState<SubTab>('llm');
  const [dragId, setDragId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);

  if (data.status === null || config === null) {
    return (
      <>
        <h2 className={H2}>Providers</h2>
        <p className='text-[12.5px] text-muted-foreground'>Loading…</p>
      </>
    );
  }

  const defs = orderedProviders(config.providers.order);
  const disabled = new Set(config.providers.disabled);

  /** Re-rank: `ids` must be a permutation of the catalog — the backend
   * validates and broadcasts `config:changed`; the resolved config comes
   * back so the UI settles immediately. */
  const commitOrder = (ids: string[]) => {
    void providersReorder(ids)
      .then((c) => {
        data.setConfig(c);
        refreshSelection(data);
      })
      .catch(() => {});
  };

  const move = (id: string, delta: -1 | 1) => {
    const ids = defs.map((d) => d.id);
    const from = ids.indexOf(id);
    const to = from + delta;
    if (from < 0 || to < 0 || to >= ids.length) {
      return;
    }
    [ids[from], ids[to]] = [ids[to], ids[from]];
    commitOrder(ids);
  };

  const drop = (targetId: string) => {
    const sourceId = dragId;
    setDragId(null);
    setOverId(null);
    if (!sourceId || sourceId === targetId) {
      return;
    }
    const ids = defs.map((d) => d.id);
    const from = ids.indexOf(sourceId);
    const to = ids.indexOf(targetId);
    if (from < 0 || to < 0) {
      return;
    }
    const [moved] = ids.splice(from, 1);
    if (moved === undefined) {
      return;
    }
    ids.splice(to, 0, moved);
    commitOrder(ids);
  };

  return (
    <>
      <h2 className={H2}>Providers</h2>
      <p className={SUB}>Your keys, your models.</p>

      <Tabs
        value={sub}
        onValueChange={(v) => setSub(v as SubTab)}
        className='gap-3'>
        <TabsList className='h-auto w-full gap-0.5 rounded-lg border border-border bg-fg-soft p-0.5'>
          {SUB_TABS.map((t) => (
            <TabsTrigger
              key={t.id}
              value={t.id}
              className='h-6.5 rounded-md text-[11.5px] font-[550] text-muted-foreground transition-[background,color] duration-(--motion-fast) ease-(--ease) hover:text-foreground data-active:bg-surface data-active:text-foreground data-active:shadow-[0_1px_2px_color-mix(in_oklch,var(--fg)_20%,transparent)] motion-reduce:transition-none'>
              {t.label}
            </TabsTrigger>
          ))}
        </TabsList>

        <TabsContent
          value='llm'
          keepMounted>
          <p className={SUB}>
            Drag to set failover priority — the top provider answers, the rest
            back it up. Switched-off providers are skipped; their keys stay put.
          </p>

          {defs.map((def) => (
            <ProviderCard
              key={def.id}
              def={def}
              data={data}
              enabled={!disabled.has(def.id)}
              isPrimary={data.selected?.provider === def.id}
              onToggleEnabled={(on) => {
                void providerSetEnabled(def.id, on)
                  .then((c) => {
                    data.setConfig(c);
                    refreshSelection(data);
                  })
                  .catch(() => {});
              }}
              drag={{
                grip: {
                  onDragStart: (e: DragEvent) => {
                    e.dataTransfer.effectAllowed = 'move';
                    e.dataTransfer.setData('text/plain', def.id);
                    // Ghost image is the whole row, not the 13px grip.
                    const row = (e.target as HTMLElement).closest(
                      '[data-prov-row]',
                    );
                    if (row instanceof HTMLElement) {
                      e.dataTransfer.setDragImage(row, 24, 16);
                    }
                    setDragId(def.id);
                  },
                  onDragEnd: () => {
                    setDragId(null);
                    setOverId(null);
                  },
                  onKeyDown: (e: KeyboardEvent) => {
                    if (e.key === 'ArrowUp') {
                      e.preventDefault();
                      move(def.id, -1);
                    } else if (e.key === 'ArrowDown') {
                      e.preventDefault();
                      move(def.id, 1);
                    }
                  },
                },
                row: {
                  onDragOver: (e: DragEvent) => {
                    // Unconditional: a drop is only ever meaningful when a
                    // row drag is in flight, and gating on `dragId` risks a
                    // state flush lagging the first dragover. An external
                    // drag still no-ops in `drop` (dragId stays null).
                    e.preventDefault();
                    if (dragId && dragId !== def.id) {
                      e.dataTransfer.dropEffect = 'move';
                      setOverId(def.id);
                    }
                  },
                  onDragLeave: (e: DragEvent) => {
                    // Only a real exit clears the target — crossing a child
                    // boundary keeps `relatedTarget` inside the row.
                    if (!e.currentTarget.contains(e.relatedTarget as Node)) {
                      setOverId((cur) => (cur === def.id ? null : cur));
                    }
                  },
                  onDrop: (e: DragEvent) => {
                    e.preventDefault();
                    drop(def.id);
                  },
                },
                active: dragId === def.id,
                over: overId === def.id,
              }}
            />
          ))}
        </TabsContent>

        <TabsContent
          value='vision'
          keepMounted>
          <VisionSection data={data} />
        </TabsContent>

        <TabsContent value='voice'>
          <p className={SUB}>
            Speech-to-text for listen mode — Deepgram Nova-2 lands with Phase 2.
            Nothing to configure yet.
          </p>
          <div className={cn(PROV_CARD, 'border-border opacity-55')}>
            <div className='flex cursor-default items-center gap-2.5'>
              <span className='size-1.75 flex-none rounded-full bg-[color-mix(in_oklch,var(--fg)_20%,transparent)]' />
              <span className='text-[13.5px] font-semibold'>Deepgram</span>
              <span className='ml-auto max-w-65 truncate font-mono text-[10.5px] text-muted-foreground'>
                stt · nova-2 · Phase 2
              </span>
            </div>
          </div>
        </TabsContent>
      </Tabs>
    </>
  );
};
