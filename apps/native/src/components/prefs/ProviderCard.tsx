/**
 * One provider row in the failover list (DESIGN.md §6 "Saved endpoints
 * register in Settings → Providers with masked key"). The head is a flex
 * row: drag grip (failover priority — the parent list owns the DnD
 * wiring) · dot + name + state + chevron (the expander) · "primary" tag
 * on the provider that would answer right now · the enabled switch.
 * The body holds the key field, endpoint fields for the compatible
 * provider, and the model picker.
 *
 * Key hygiene (same contract as the retired mini panel): the input is
 * `type="password"`, cleared on successful save, and only ever renders
 * the backend's `…last4` mask — a typed key never echoes back into the
 * DOM. Save is validate-then-store: `model_validate_key` probes without
 * persisting; only `keystore_set_key` writes `keys.json`.
 *
 * The compatible provider differs in three ways: a name + base URL pair
 * in `config.toml` (`compat.*`), an optional key (open local endpoints),
 * and a free-text model field with its live `/models` listing attached
 * as a datalist — no static catalog exists for an arbitrary endpoint.
 *
 * Model memory is per-provider (`providers.models.<id>`): the picker
 * persists this row's model; which provider actually answers is decided
 * by `providers.order` — the `primary` tag marks that row.
 */
import { useEffect, useState } from 'react';
import type { DragEvent, KeyboardEvent } from 'react';
import { ChevronRightIcon, GripVerticalIcon } from '@marvis/ui';
import {
  configSet,
  keystoreRemoveKey,
  keystoreSetKey,
  modelGetSelected,
  modelListAvailable,
  modelSetSelected,
  modelValidateKey,
} from '../../lib/commands';
import { providerLabel, type ProviderDef } from '../../lib/providers';
import {
  BTN_LG,
  BTN_LINK,
  BTN_LINK_LG,
  BTN_OUTLINE,
  BTN_PRIMARY,
  FIELD,
  LBL,
  MODEL_SEL,
  NUM,
  PROV_CARD,
  PROV_ERR,
  PROV_NOTE,
  SPIN,
  cn,
} from '../../lib/classes';
import { Switch, Tag } from './bits';
import type { PrefsData } from './types';

const URL_RE = /^https?:\/\//i;

/** The DnD wiring `ProvidersTab` hands each row — the grip is the drag
 * source (whole-row `draggable` would break the body's text inputs) and
 * the row is the drop target. */
export interface ProviderDrag {
  grip: {
    onDragStart: (e: DragEvent) => void;
    onDragEnd: (e: DragEvent) => void;
    onKeyDown: (e: KeyboardEvent) => void;
  };
  row: {
    onDragOver: (e: DragEvent) => void;
    onDragLeave: (e: DragEvent) => void;
    onDrop: (e: DragEvent) => void;
  };
  /** This row is the one in flight — dims it. */
  active: boolean;
  /** This row is the current drop target — accent edge. */
  over: boolean;
}

export const ProviderCard = ({
  def,
  data,
  enabled,
  isPrimary,
  onToggleEnabled,
  drag,
}: {
  def: ProviderDef;
  data: PrefsData;
  enabled: boolean;
  isPrimary: boolean;
  onToggleEnabled: (on: boolean) => void;
  drag: ProviderDrag;
}) => {
  const { status, config } = data;

  const [open, setOpen] = useState(false);
  const [keyInput, setKeyInput] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [models, setModels] = useState<string[] | null>(null);
  const [ollamaChecked, setOllamaChecked] = useState(false);

  // Compat endpoint fields — seeded from the loaded config (the tab only
  // renders once `config` is non-null).
  const [name, setName] = useState(config?.compat.name ?? '');
  const [baseUrl, setBaseUrl] = useState(config?.compat.base_url ?? '');
  const [modelText, setModelText] = useState(
    config?.providers.models[def.id] ?? '',
  );

  const masked = status?.keys.find(([p]) => p === def.id)?.[1] ?? null;
  const compatConfigured = !!config?.compat.base_url;
  const isSet =
    def.id === 'ollama' ? false : def.compat ? compatConfigured : !!masked;
  /** This provider's remembered model (`providers.models.<id>`). */
  const remembered = config?.providers.models[def.id] ?? '';
  /** What the provider would answer with — memory or the static default
   * (hosted only; live-list providers have no static default). */
  const resolvedModel = remembered || def.defaultModel || '';

  /** Mirrors the backend's chain-usability rule: a switch can only turn
   * ON when the provider could actually answer — key where one is
   * required (hosted providers always resolve a model via the static
   * first), `base_url` + a picked model for a compatible endpoint, a
   * picked model for Ollama. Turning OFF is always allowed. */
  const usable =
    def.id === 'ollama'
      ? remembered !== ''
      : def.compat
        ? compatConfigured && remembered !== ''
        : !!masked;
  /** Why the switch won't turn on — surfaced as the tooltip. */
  const enableHint = usable
    ? undefined
    : def.id === 'ollama'
      ? 'Pick a model first — expand and check the daemon'
      : def.compat
        ? compatConfigured
          ? 'Pick a model first'
          : 'Set the endpoint first'
        : 'Save an API key first';

  // Prototype state text: masked key + chosen model (`…7B2q · gpt-4o`).
  const stateText =
    def.id === 'ollama'
      ? 'local · no key needed'
      : def.compat
        ? compatConfigured
          ? `${masked ?? 'no key'} · ${hostLabel(config!.compat.base_url)}`
          : 'not configured'
        : masked
          ? `${masked} · ${resolvedModel}`
          : 'not set';

  const refreshModels = async () => {
    try {
      setModels(await modelListAvailable(def.id));
    } catch {
      setModels([]);
    }
  };

  // Models load lazily on first expand and again after a save (a fresh
  // key/endpoint can change the list).
  useEffect(() => {
    if (open && models === null) {
      void refreshModels();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const save = async () => {
    if (saving) {
      return;
    }
    const key = keyInput.trim();
    setError('');

    if (def.id === 'ollama') {
      setSaving(true);
      setOllamaChecked(true);
      try {
        await refreshModels();
      } finally {
        setSaving(false);
      }
      return;
    }

    if (def.compat) {
      const url = baseUrl.trim();
      if (!URL_RE.test(url)) {
        setError(
          'Base URL must start with http:// or https:// — e.g. https://api.groq.com/openai/v1',
        );
        return;
      }
      setSaving(true);
      try {
        // Persist the endpoint BEFORE validating — `model_validate_key`
        // reads `compat.base_url` server-side to build the probe.
        let c = await configSet('compat.name', name.trim());
        c = await configSet('compat.base_url', url);
        data.setConfig(c);
        const verdict = await modelValidateKey(def.id, key);
        if (!verdict.ok) {
          setError(verdict.error);
          return;
        }
        if (key) {
          data.setStatus(await keystoreSetKey(def.id, key));
          setKeyInput('');
        }
        setModels(null); // refetch — a keyed endpoint lists more models
        await refreshModels();
        // A newly-usable provider may now head the chain.
        data.setSelected(await modelGetSelected());
      } catch (e) {
        setError(typeof e === 'string' ? e : 'Save failed');
      } finally {
        setSaving(false);
      }
      return;
    }

    if (!key) {
      return;
    }
    setSaving(true);
    try {
      const verdict = await modelValidateKey(def.id, key);
      if (!verdict.ok) {
        setError(verdict.error);
        return;
      }
      data.setStatus(await keystoreSetKey(def.id, key));
      setKeyInput('');
      await refreshModels();
      data.setSelected(await modelGetSelected());
    } catch (e) {
      setError(typeof e === 'string' ? e : 'Save failed');
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (saving) {
      return;
    }
    setSaving(true);
    setError('');
    try {
      if (def.compat) {
        // Clearing the endpoint un-registers the provider; the key goes
        // with it when present.
        if (masked) {
          data.setStatus(await keystoreRemoveKey(def.id));
        }
        let c = await configSet('compat.base_url', '');
        c = await configSet('compat.name', '');
        data.setConfig(c);
        setName('');
        setBaseUrl('');
        setModels(null);
      } else {
        data.setStatus(await keystoreRemoveKey(def.id));
      }
      // Losing a key/endpoint can change the chain head — re-resolve it
      // so the "primary" tag stays honest.
      data.setSelected(await modelGetSelected());
    } catch (e) {
      setError(typeof e === 'string' ? e : 'Remove failed');
    } finally {
      setSaving(false);
    }
  };

  const chooseModel = (model: string) => {
    if (!model) {
      return;
    }
    // Returns the resolved (chain-head) selection — a non-primary row's
    // write must not relabel `data.selected`.
    void modelSetSelected(def.id, model)
      .then(data.setSelected)
      .catch(() => setError('Could not save model selection'));
  };

  /** Model options; keep a stale configured selection visible. */
  const modelOptions = (): string[] => {
    const list = models ?? [];
    return remembered && !list.includes(remembered)
      ? [remembered, ...list]
      : list;
  };

  const saveLabel =
    def.id === 'ollama'
      ? 'Check daemon'
      : def.compat
        ? 'Validate endpoint'
        : masked
          ? 'Replace'
          : 'Validate & save';

  const label = providerLabel(def.id, name || config?.compat.name || '');

  return (
    <div
      data-prov-row
      className={cn(
        PROV_CARD,
        drag.over
          ? 'border-accent'
          : open
            ? 'border-[color-mix(in_oklch,var(--accent)_50%,var(--border))]'
            : 'border-border',
        drag.active ? 'opacity-40' : !enabled && 'opacity-55',
      )}
      {...drag.row}>
      <div className='flex items-center gap-2.5'>
        <span
          className='grid flex-none cursor-grab place-items-center rounded text-muted-foreground select-none [-webkit-user-drag:element] active:cursor-grabbing focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-1'
          draggable
          role='button'
          tabIndex={0}
          title='Drag to set failover priority — ↑/↓ also moves'
          aria-label={`Reorder ${label}`}
          {...drag.grip}>
          <GripVerticalIcon className='size-3.25' />
        </span>
        <button
          type='button'
          className='flex min-w-0 flex-1 cursor-pointer items-center gap-2.5 border-0 bg-transparent p-0 text-left text-inherit'
          aria-expanded={open}
          onClick={() => setOpen((o) => !o)}>
          <span
            className={cn(
              'size-1.75 flex-none rounded-full',
              isSet || def.id === 'ollama'
                ? 'bg-accent'
                : 'bg-[color-mix(in_oklch,var(--fg)_20%,transparent)]',
            )}
          />
          <span className='text-[13.5px] font-semibold'>{label}</span>
          <span
            className={cn(
              'ml-auto max-w-65 truncate font-mono text-[10.5px]',
              isSet ? 'text-foreground' : 'text-muted-foreground',
            )}>
            {stateText}
          </span>
          <span className='grid flex-none place-items-center text-muted-foreground'>
            <ChevronRightIcon
              className={cn(
                'size-3 transition-transform duration-(--motion-fast) ease-(--ease) motion-reduce:transition-none',
                open && 'rotate-90',
              )}
            />
          </span>
        </button>
        {isPrimary && <Tag>primary</Tag>}
        {/* The hint rides a wrapper — tooltips on a `disabled` button
            don't fire reliably. */}
        <span title={enableHint}>
          <Switch
            checked={enabled}
            onChange={onToggleEnabled}
            disabled={!enabled && !usable}
            ariaLabel={`${enabled ? 'Disable' : 'Enable'} ${label}`}
          />
        </span>
      </div>

      <div className={cn('pt-3', open ? 'animate-fade-in' : 'hidden')}>
        {def.compat && (
          <>
            {compatConfigured && (
              <p className={cn(PROV_NOTE, 'mt-0 mb-2.5')}>
                endpoint ·{' '}
                <span className={NUM}>{config!.compat.base_url}</span>
              </p>
            )}
            <div className='mb-2 flex flex-col gap-2'>
              <input
                className={FIELD}
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder='Provider name — e.g. Groq, OpenRouter, vLLM'
                autoComplete='off'
                aria-label='Compatible provider name'
              />
              <input
                className={FIELD}
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && void save()}
                placeholder='Base URL — e.g. https://api.groq.com/openai/v1'
                autoComplete='off'
                spellCheck={false}
                aria-label='Compatible base URL'
              />
            </div>
          </>
        )}

        {def.id !== 'ollama' && (
          <div className='mt-1.5 flex items-center gap-1.5'>
            <input
              className={FIELD}
              type='password'
              autoComplete='off'
              value={keyInput}
              onChange={(e) => setKeyInput(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && void save()}
              placeholder={
                def.compat
                  ? def.keyPlaceholder
                  : masked
                    ? 'Replace key'
                    : def.keyPlaceholder
              }
              aria-label={`${label} API key`}
            />
            <button
              type='button'
              className={cn(BTN_LG, BTN_PRIMARY)}
              disabled={saving || (!def.compat && !keyInput.trim())}
              onClick={() => void save()}>
              {saving ? (
                <>
                  <span className={SPIN} />
                  Validating…
                </>
              ) : (
                saveLabel
              )}
            </button>
          </div>
        )}
        {def.id === 'ollama' && (
          <div className='mt-1.5 flex items-center gap-1.5'>
            <span className={cn(PROV_NOTE, 'mt-0 flex-1')}>
              Reads the daemon's <span className={NUM}>/api/tags</span> —
              nothing leaves this Mac.
            </span>
            <button
              type='button'
              className={cn(BTN_LG, BTN_OUTLINE)}
              disabled={saving}
              onClick={() => void save()}>
              {saving ? (
                <>
                  <span className={SPIN} />
                  Checking…
                </>
              ) : (
                saveLabel
              )}
            </button>
          </div>
        )}

        {masked && !def.compat && (
          <p className={PROV_NOTE}>
            Shown masked — the plaintext key never re-enters the DOM.{' '}
            <button
              type='button'
              className={cn(BTN_LINK_LG, BTN_LINK)}
              disabled={saving}
              onClick={() => void remove()}>
              Remove key
            </button>
          </p>
        )}
        {def.compat && compatConfigured && (
          <p className={PROV_NOTE}>
            Posts to{' '}
            <span className={NUM}>
              {config!.compat.base_url}/chat/completions
            </span>
            {masked ? ` · key ${masked}` : ' · no key — open endpoint'}.{' '}
            <button
              type='button'
              className={cn(BTN_LINK_LG, BTN_LINK)}
              disabled={saving}
              onClick={() => void remove()}>
              Remove endpoint
            </button>
          </p>
        )}
        {error && <p className={PROV_ERR}>{error}</p>}
        {def.id === 'ollama' &&
          ollamaChecked &&
          (models?.length ?? 0) === 0 && (
            <p className={PROV_NOTE}>
              No models — is the Ollama daemon running?
            </p>
          )}

        <div className='mt-2.5 flex items-center gap-2'>
          <span className={LBL}>Model</span>
          {def.compat ? (
            <>
              <input
                className={FIELD}
                list={`models-${def.id}`}
                value={modelText}
                onChange={(e) => setModelText(e.target.value)}
                onBlur={() => modelText.trim() && chooseModel(modelText.trim())}
                onKeyDown={(e) =>
                  e.key === 'Enter' &&
                  modelText.trim() &&
                  chooseModel(modelText.trim())
                }
                placeholder='model id — e.g. llama-3.3-70b-versatile'
                autoComplete='off'
                aria-label='Compatible model id'
              />
              <datalist id={`models-${def.id}`}>
                {(models ?? []).map((m) => (
                  <option
                    key={m}
                    value={m}
                  />
                ))}
              </datalist>
            </>
          ) : (
            <select
              className={MODEL_SEL}
              value={resolvedModel}
              disabled={
                saving ||
                modelOptions().length === 0 ||
                (!isSet && def.id !== 'ollama')
              }
              onChange={(e) => chooseModel(e.target.value)}
              aria-label={`${label} model`}>
              <option value=''>
                {models === null
                  ? 'Loading…'
                  : modelOptions().length === 0
                    ? 'No models found'
                    : 'Select model'}
              </option>
              {modelOptions().map((m) => (
                <option
                  key={m}
                  value={m}>
                  {m}
                </option>
              ))}
            </select>
          )}
        </div>
      </div>
    </div>
  );
};

/** `https://api.groq.com/openai/v1` → `api.groq.com` — the row's compact
 * endpoint label (no scheme, no path). */
const hostLabel = (url: string): string =>
  url.replace(/^https?:\/\//i, '').split(/[/:?#]/)[0] ?? url;
