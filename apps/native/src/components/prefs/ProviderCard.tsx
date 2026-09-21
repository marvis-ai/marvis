/**
 * One expandable provider row (DESIGN.md §6 "Saved endpoints register in
 * Settings → Providers with masked key"): dot + name + state in the head,
 * body holds the key field, endpoint fields for the compatible provider,
 * and the model picker.
 *
 * Key hygiene (same contract as the retired mini panel): the input is
 * `type="password"`, cleared on successful save, and only ever renders
 * the backend's `…last4` mask — a typed key never echoes back into the
 * DOM. Save is validate-then-store: `model_validate_key` probes without
 * persisting; only `keystore_set_key` writes `keys.enc`.
 *
 * The compatible provider differs in three ways: a name + base URL pair
 * in `config.toml` (`compat.*`), an optional key (open local endpoints),
 * and a free-text model field with its live `/models` listing attached
 * as a datalist — no static catalog exists for an arbitrary endpoint.
 *
 * While the vault isn't `Unlocked` the body collapses to a notice; the
 * VaultCard above owns the unlock UX.
 */
import { useEffect, useState } from 'react';
import { ChevronRight } from '@marvis/ui';
import {
  configSet,
  keystoreRemoveKey,
  keystoreSetKey,
  modelListAvailable,
  modelSetSelected,
  modelValidateKey,
} from '../../lib/commands';
import { providerLabel, type ProviderDef } from '../../lib/providers';
import type { PrefsData } from './types';

const URL_RE = /^https?:\/\//i;

export const ProviderCard = ({
  def,
  data,
}: {
  def: ProviderDef;
  data: PrefsData;
}) => {
  const { status, config, selected } = data;
  const locked = status?.state !== 'Unlocked';

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
    selected?.provider === def.id ? selected.model : '',
  );

  const masked = status?.keys.find(([p]) => p === def.id)?.[1] ?? null;
  const compatConfigured = !!config?.compat.base_url;
  const isSet =
    def.id === 'ollama' ? false : def.compat ? compatConfigured : !!masked;
  const selModel = selected?.provider === def.id ? selected.model : '';

  // Prototype state text: masked key + chosen model (`…7B2q · gpt-4o`).
  const stateText =
    def.id === 'ollama'
      ? 'local · no key needed'
      : def.compat
        ? compatConfigured
          ? `${masked ?? 'no key'} · ${hostLabel(config!.compat.base_url)}`
          : 'not configured'
        : masked
          ? `${masked}${selModel ? ` · ${selModel}` : ''}`
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
        // with it when present. The selection (if it was compatible) is
        // left in place — ask reports the missing endpoint inline.
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
    void modelSetSelected(def.id, model)
      .then(data.setSelected)
      .catch(() => setError('Could not save model selection'));
  };

  /** Model options; keep a stale configured selection visible. */
  const modelOptions = (): string[] => {
    const list = models ?? [];
    const sel = selected?.provider === def.id ? selected.model : '';
    return sel && !list.includes(sel) ? [sel, ...list] : list;
  };

  const saveLabel =
    def.id === 'ollama'
      ? 'Check daemon'
      : def.compat
        ? 'Validate endpoint'
        : masked
          ? 'Replace'
          : 'Validate & save';

  return (
    <div className={`prf-prov${open ? ' is-open' : ''}`}>
      <button
        type='button'
        className='prf-prov-head'
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}>
        <span
          className={`prf-prov-dot${isSet || def.id === 'ollama' ? ' set' : ''}`}
        />
        <span className='prf-prov-name'>
          {providerLabel(def.id, name || config?.compat.name || '')}
        </span>
        <span className={`prf-prov-state${isSet ? ' set' : ''}`}>
          {stateText}
        </span>
        <span className='prf-prov-expand'>
          <ChevronRight />
        </span>
      </button>

      <div className='prf-prov-body'>
        {locked ? (
          <p className='prov-note'>Unlock the key vault to change keys.</p>
        ) : (
          <>
            {def.compat && (
              <>
                {compatConfigured && (
                  <p
                    className='prov-note'
                    style={{ marginTop: 0, marginBottom: 10 }}>
                    endpoint ·{' '}
                    <span className='num'>{config!.compat.base_url}</span>
                  </p>
                )}
                <div className='prf-fields'>
                  <input
                    className='key-input'
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder='Provider name — e.g. Groq, OpenRouter, vLLM'
                    autoComplete='off'
                    aria-label='Compatible provider name'
                  />
                  <input
                    className='key-input'
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
              <div className='key-row'>
                <input
                  className='key-input'
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
                  aria-label={`${providerLabel(def.id, name)} API key`}
                />
                <button
                  type='button'
                  className='mv-btn mv-btn-primary'
                  disabled={saving || (!def.compat && !keyInput.trim())}
                  onClick={() => void save()}>
                  {saving ? (
                    <>
                      <span className='mv-spin' />
                      Validating…
                    </>
                  ) : (
                    saveLabel
                  )}
                </button>
              </div>
            )}
            {def.id === 'ollama' && (
              <div className='key-row'>
                <span
                  className='prov-note'
                  style={{ marginTop: 0, flex: 1 }}>
                  Reads the daemon's <span className='num'>/api/tags</span> —
                  nothing leaves this Mac.
                </span>
                <button
                  type='button'
                  className='mv-btn mv-btn-outline'
                  disabled={saving}
                  onClick={() => void save()}>
                  {saving ? (
                    <>
                      <span className='mv-spin' />
                      Checking…
                    </>
                  ) : (
                    saveLabel
                  )}
                </button>
              </div>
            )}

            {masked && !def.compat && (
              <p className='prov-note'>
                Shown masked — the plaintext key never re-enters the DOM.{' '}
                <button
                  type='button'
                  className='mv-btn mv-btn-link'
                  disabled={saving}
                  onClick={() => void remove()}>
                  Remove key
                </button>
              </p>
            )}
            {def.compat && compatConfigured && (
              <p className='prov-note'>
                Posts to{' '}
                <span className='num'>
                  {config!.compat.base_url}/chat/completions
                </span>
                {masked ? ` · key ${masked}` : ' · no key — open endpoint'}.{' '}
                <button
                  type='button'
                  className='mv-btn mv-btn-link'
                  disabled={saving}
                  onClick={() => void remove()}>
                  Remove endpoint
                </button>
              </p>
            )}
            {error && <p className='prov-err show'>{error}</p>}
            {def.id === 'ollama' &&
              ollamaChecked &&
              (models?.length ?? 0) === 0 && (
                <p className='prov-note'>
                  No models — is the Ollama daemon running?
                </p>
              )}

            <div className='prf-prov-model'>
              <span className='lbl'>Model</span>
              {def.compat ? (
                <>
                  <input
                    className='key-input'
                    list={`models-${def.id}`}
                    value={modelText}
                    onChange={(e) => setModelText(e.target.value)}
                    onBlur={() =>
                      modelText.trim() && chooseModel(modelText.trim())
                    }
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
                  className='model-sel'
                  value={selected?.provider === def.id ? selected.model : ''}
                  disabled={
                    saving ||
                    modelOptions().length === 0 ||
                    (!isSet && def.id !== 'ollama')
                  }
                  onChange={(e) => chooseModel(e.target.value)}
                  aria-label={`${providerLabel(def.id, name)} model`}>
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
          </>
        )}
      </div>
    </div>
  );
};

/** `https://api.groq.com/openai/v1` → `api.groq.com` — the row's compact
 * endpoint label (no scheme, no path). */
const hostLabel = (url: string): string =>
  url.replace(/^https?:\/\//i, '').split(/[/:?#]/)[0] ?? url;
