/**
 * Memory — the consent-gated local profile. When enabled, each
 * successful Ask send hands ONLY the typed user text to the dedicated
 * extraction model configured here (independent of the Ask failover
 * chain); the parsed facts land in `marvis.db` and a bounded profile
 * block joins later Ask system prompts. The rows below are a local
 * CRUD surface — a manual edit pins `source = 'manual'` server-side so
 * extraction can never overwrite it.
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  configSet,
  memoryDelete,
  memoryList,
  memoryUpdate,
  modelListAvailable,
  type Memory,
  type ModelSelection,
} from '@/lib/commands';
import { EV_MEMORY_CHANGED, useTauriEvent } from '@/lib/events';
import { orderedProviders, providerLabel } from '@/lib/providers';
import { sessionDateLabel } from '@/components/listen/model';
import {
  BTN_DANGER,
  BTN_OUTLINE,
  BTN_PRIMARY,
  BTN_SM,
  H2,
  LBL,
  MODEL_SEL,
  PRF_ROW,
  PRF_ROWS,
  PROV_CARD,
  PROV_ERR,
  PROV_NOTE,
  PR_LABEL,
  PR_SUB,
  SUB,
  cn,
} from '@/lib/classes';
import { PrefRow } from './bits';
import type { PrefsData } from './types';

const CONFIRM_MS = 4000;
const INPUT =
  'w-full rounded-lg border border-border bg-input-well px-2.5 py-1.5 text-[12.5px] text-foreground outline-none transition-[border-color,box-shadow] duration-(--motion-fast) ease-(--ease) focus:border-accent focus:shadow-(--focus-ring)';

export const MemoryTab = ({ data }: { data: PrefsData }) => {
  const { config } = data;
  const memory = config?.memory ?? { enabled: false, provider: '', model: '' };

  /** The current Ask selection — suggested while `memory.provider` /
   * `memory.model` are empty. `data.selected` is the resolved chain
   * head; before it lands, fall back to the first enabled provider's
   * remembered model. Suggestion only — enabling still writes it
   * explicitly. */
  const suggested: ModelSelection | null = useMemo(() => {
    if (data.selected) return data.selected;
    const head = config?.providers.order.find(
      (id) => !config.providers.disabled.includes(id),
    );
    const m = head ? config?.providers.models[head] : undefined;
    return head && m ? { provider: head, model: m } : null;
  }, [data.selected, config]);

  const [facts, setFacts] = useState<Memory[]>([]);
  const [provider, setProvider] = useState(
    memory.provider || suggested?.provider || '',
  );
  const [model, setModel] = useState(memory.model || suggested?.model || '');
  const [models, setModels] = useState<string[]>([]);
  const [confirmingDisable, setConfirmingEnable] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [editingId, setEditingId] = useState<number | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState<number | null>(null);
  const editValue = useRef<HTMLInputElement | null>(null);
  const disableTimer = useRef<number | undefined>(undefined);
  const deleteTimer = useRef<number | undefined>(undefined);

  const refreshFacts = () => {
    void memoryList()
      .then(setFacts)
      .catch(() => setError('Could not load memories'));
  };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(refreshFacts, []);
  useTauriEvent(EV_MEMORY_CHANGED, refreshFacts);

  // Reseed the pickers when the saved selection (re)loads — the
  // suggestion stands in while memory has no provider/model of its own.
  useEffect(() => {
    setProvider(memory.provider || suggested?.provider || '');
    setModel(memory.model || suggested?.model || '');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [memory.provider, memory.model]);

  // A new provider needs a fresh model list.
  useEffect(() => {
    setModels([]);
    if (!provider) return;
    let stale = false;
    modelListAvailable(provider)
      .then((list) => {
        if (!stale) setModels(list);
      })
      .catch(() => {
        if (!stale) setModels([]);
      });
    return () => {
      stale = true;
    };
  }, [provider]);

  /** Keep a saved model visible when the live list doesn't name it. */
  const modelOptions =
    model && !models.includes(model) ? [model, ...models] : models;

  const saveSelection = async () => {
    if (!provider || !model) return;
    setSaving(true);
    setError('');
    try {
      data.setConfig(await configSet('memory.provider', provider));
      data.setConfig(await configSet('memory.model', model));
    } catch (e) {
      setError(typeof e === 'string' ? e : 'Could not save the memory model');
    } finally {
      setSaving(false);
    }
  };

  const toggleMemory = async () => {
    if (memory.enabled) {
      // Disabling is the destructive-feeling direction for the user —
      // their profile stops learning — so it arms, then confirms.
      if (!confirmingDisable) {
        setConfirmingEnable(true);
        setError('');
        window.clearTimeout(disableTimer.current);
        disableTimer.current = window.setTimeout(
          () => setConfirmingEnable(false),
          CONFIRM_MS,
        );
        return;
      }
      window.clearTimeout(disableTimer.current);
      setConfirmingEnable(false);
      try {
        data.setConfig(await configSet('memory.enabled', false));
      } catch {
        setError('Could not disable memory');
      }
      return;
    }
    // Enabling is one click — the privacy disclosure is always on the
    // row, and the resolved pick writes before the consent bit.
    const p = provider || suggested?.provider || '';
    const m = model || suggested?.model || '';
    if (!p || !m) {
      setError('Choose a provider and model before enabling memory.');
      return;
    }
    setSaving(true);
    setError('');
    try {
      // `memory.enabled` is rejected server-side without both fields —
      // write the resolved selection first, then flip the consent bit.
      data.setConfig(await configSet('memory.provider', p));
      data.setConfig(await configSet('memory.model', m));
      data.setConfig(await configSet('memory.enabled', true));
    } catch (e) {
      setError(typeof e === 'string' ? e : 'Could not enable memory');
    } finally {
      setSaving(false);
    }
  };

  const saveEdit = async (id: number) => {
    const value = editValue.current?.value.trim() ?? '';
    if (!value) return;
    try {
      const updated = await memoryUpdate(id, value);
      setFacts((rows) => rows.map((r) => (r.id === id ? updated : r)));
      setEditingId(null);
    } catch (e) {
      setError(typeof e === 'string' ? e : 'Could not save the memory');
    }
  };

  const removeFact = async (id: number) => {
    if (confirmingDelete !== id) {
      setConfirmingDelete(id);
      window.clearTimeout(deleteTimer.current);
      deleteTimer.current = window.setTimeout(
        () => setConfirmingDelete(null),
        CONFIRM_MS,
      );
      return;
    }
    window.clearTimeout(deleteTimer.current);
    setConfirmingDelete(null);
    try {
      await memoryDelete(id);
      setFacts((rows) => rows.filter((r) => r.id !== id));
    } catch (e) {
      setError(typeof e === 'string' ? e : 'Could not delete the memory');
    }
  };

  return (
    <>
      <h2 className={H2}>Memory</h2>
      <p className={SUB}>
        When enabled, each Ask you send can teach Marvis durable facts — your
        name, role, and preferences — kept locally in marvis.db and shown to
        future replies. Off by default; every row is editable.
      </p>

      <div className={PRF_ROWS}>
        <PrefRow
          label={memory.enabled ? 'Memory is on' : 'Memory is off'}
          sub='Extraction runs after a successful Ask, on the typed text only.'
          last={memory.enabled && !confirmingDisable}>
          <button
            type='button'
            aria-label={memory.enabled ? 'Disable memory' : 'Enable memory'}
            className={cn(
              BTN_SM,
              memory.enabled || confirmingDisable ? BTN_OUTLINE : BTN_PRIMARY,
            )}
            disabled={saving}
            onClick={() => void toggleMemory()}>
            {confirmingDisable
              ? 'Click to confirm'
              : memory.enabled
                ? 'Disable memory'
                : 'Enable memory'}
          </button>
        </PrefRow>
        {confirmingDisable ? (
          <p className={cn(PROV_NOTE, 'border-b border-border pb-3')}>
            Disabling stops extraction — your stored facts stay in marvis.db and
            keep shaping replies until you delete them.
          </p>
        ) : (
          !memory.enabled && (
            <p className={cn(PROV_NOTE, 'border-b border-border pb-3')}>
              Your new Ask text is sent to the Memory model below to extract
              facts. Facts stay in local SQLite, but hosted providers receive
              the source text — Ollama stays local and may use your CPU/GPU.
              Screen frames, attachments, Listen transcripts, assistant replies,
              and past sessions are never analyzed.
            </p>
          )
        )}
      </div>

      <div className={cn(PROV_CARD, 'mt-3.5 border-border')}>
        <div className='flex items-center gap-2'>
          <span className={cn(LBL, 'w-14')}>Provider</span>
          <select
            className={MODEL_SEL}
            value={provider}
            disabled={saving}
            onChange={(e) => setProvider(e.target.value)}
            aria-label='Memory provider'>
            <option value=''>Select provider</option>
            {orderedProviders(config?.providers.order).map((d) => (
              <option
                key={d.id}
                value={d.id}>
                {providerLabel(d.id, config?.compat.name ?? '')}
              </option>
            ))}
          </select>
        </div>
        <div className='mt-2.5 flex items-center gap-2'>
          <span className={cn(LBL, 'w-14')}>Model</span>
          <select
            className={MODEL_SEL}
            value={model}
            disabled={saving || !provider}
            onChange={(e) => setModel(e.target.value)}
            aria-label='Memory model'>
            <option value=''>
              {provider ? 'Select model' : 'Pick a provider first'}
            </option>
            {modelOptions.map((m) => (
              <option
                key={m}
                value={m}>
                {m}
              </option>
            ))}
          </select>
          <button
            type='button'
            className={cn(BTN_SM, BTN_PRIMARY)}
            disabled={saving || !provider || !model}
            onClick={() => void saveSelection()}>
            Save model
          </button>
        </div>
        <p className={PROV_NOTE}>
          {memory.provider
            ? 'Memory runs on this provider and model — separate from the Ask failover chain.'
            : suggested
              ? `Suggested from your Ask setup: ${providerLabel(suggested.provider, config?.compat.name ?? '')} · ${suggested.model}.`
              : 'Pick a provider and model — memory runs on its own selection, not the Ask chain.'}
        </p>
      </div>

      <h3 className='mt-6 mb-2 text-[13px] font-[550]'>Profile facts</h3>
      <div className={PRF_ROWS}>
        {facts.map((f) =>
          editingId === f.id ? (
            <div
              key={f.id}
              className={cn(PRF_ROW, 'flex-col items-stretch gap-2.5')}>
              <div className={PR_LABEL}>{f.attribute}</div>
              <input
                ref={editValue}
                aria-label='Memory value'
                maxLength={500}
                defaultValue={f.value}
                className={INPUT}
              />
              <div className='flex justify-end gap-2'>
                <button
                  type='button'
                  className={cn(BTN_SM, BTN_OUTLINE)}
                  onClick={() => setEditingId(null)}>
                  Cancel
                </button>
                <button
                  type='button'
                  className={cn(BTN_SM, BTN_PRIMARY)}
                  onClick={() => void saveEdit(f.id)}>
                  Save memory
                </button>
              </div>
            </div>
          ) : (
            <PrefRow
              key={f.id}
              label={f.attribute}
              sub={
                <>
                  {f.value}
                  <span className='mt-1 block font-mono text-[10.5px]'>
                    {f.category} · {f.source} · {f.basis} ·{' '}
                    {Math.round(f.confidence * 100)}% · updated{' '}
                    {sessionDateLabel(f.updated_at)}
                  </span>
                </>
              }>
              <button
                type='button'
                aria-label={`Edit ${f.attribute}`}
                className={cn(BTN_SM, BTN_OUTLINE)}
                onClick={() => {
                  setEditingId(f.id);
                  setError('');
                }}>
                Edit
              </button>
              <button
                type='button'
                aria-label={`Delete ${f.attribute}`}
                className={cn(
                  BTN_SM,
                  BTN_OUTLINE,
                  confirmingDelete === f.id && BTN_DANGER,
                )}
                onClick={() => void removeFact(f.id)}>
                {confirmingDelete === f.id ? 'Click to confirm' : 'Delete'}
              </button>
            </PrefRow>
          ),
        )}
        {facts.length === 0 && (
          <div className={cn(PRF_ROW, 'border-b-0')}>
            <div>
              <div className={PR_LABEL}>Nothing remembered yet</div>
              <div className={PR_SUB}>
                Facts appear here after enabled Ask sends — and you can edit or
                delete any of them.
              </div>
            </div>
          </div>
        )}
      </div>
      {error && <p className={PROV_ERR}>{error}</p>}
    </>
  );
};
