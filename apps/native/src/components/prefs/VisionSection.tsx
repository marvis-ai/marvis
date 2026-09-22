/**
 * Screen reading — the dedicated vision reader (`[vision]` config), the
 * Vision tab under Settings → Providers. When a provider is picked, an
 * ask with a frame sends the screenshot to this model FIRST: its reply
 * is a text description the failover chain then answers over, so chat
 * providers never need image support. "Off" attaches the frame to the
 * answering provider directly, as before.
 *
 * The provider pick is a `ToggleGroup` (single-select; "Off" writes
 * `vision.provider = ''`). The group's `bg-muted`/`border-input`
 * defaults are overridden onto the overlay palette — `--muted` is a
 * text color here, not a surface.
 *
 * Keys and the compatible endpoint are shared with the provider rows on
 * the LLM tab — this pick only chooses which configured provider reads
 * the screen, and it ignores the chain's enable switches. Usability
 * mirrors the backend's `vision_candidate`: a hosted pick needs its
 * saved key, compatible needs `compat.base_url`, and every pick needs a
 * resolvable model — an unusable pick shows an "Inactive" note and the
 * ask falls back to attaching the frame.
 *
 * Model memory is per-provider (`vision.models.<id>`) — switching the
 * reader away and back restores its pick; an empty memory resolves to
 * the provider's vision default (`VISION_DEFAULT_MODEL`, mirrored from
 * `llm::vision_default_model`). Compatible gets no default: free-text
 * field with the live `/models` listing as a datalist, like its row.
 */
import { useEffect, useState } from 'react';
import { Input, ToggleGroup, ToggleGroupItem } from '@marvis/ui';
import { configSet, modelListAvailable } from '../../lib/commands';
import {
  VISION_DEFAULT_MODEL,
  VISION_PROVIDERS,
  providerLabel,
} from '../../lib/providers';
import {
  FIELD,
  LBL,
  MODEL_SEL,
  NUM,
  PROV_CARD,
  PROV_ERR,
  PROV_NOTE,
  SUB,
  cn,
} from '../../lib/classes';
import type { PrefsData } from './types';

/** ToggleGroupItem onto the overlay palette: bordered pills; hover and
 * the pressed pick ride the user-settable accent (`accent-soft`/
 * `accent-text` mix live off `--accent`). Base UI marks a pressed
 * toggle with BOTH `aria-pressed` and `data-pressed` — the group's own
 * `aria-pressed:bg-muted` (toggleVariants) is overridden by matching
 * its variant, `data-pressed:` covers the group-level selector, so the
 * accent wins deterministically instead of by stylesheet order. */
const TOGGLE_ITEM =
  'h-7.5 rounded-lg border-border bg-transparent px-2.5 text-[11.5px] font-[550] text-muted-foreground hover:bg-accent-soft hover:text-accent-text aria-pressed:border-accent aria-pressed:bg-accent-soft aria-pressed:text-accent-text data-pressed:border-accent data-pressed:bg-accent-soft data-pressed:text-accent-text';

export const VisionSection = ({ data }: { data: PrefsData }) => {
  const { config, status } = data;
  const provider = config?.vision?.provider ?? '';
  const isCompat = provider === 'compatible';

  const [models, setModels] = useState<string[] | null>(null);
  const [modelText, setModelText] = useState('');
  const [error, setError] = useState('');

  const masked = status?.keys.find(([p]) => p === provider)?.[1] ?? null;
  const compatUrl = config?.compat.base_url ?? '';
  const remembered = provider ? (config?.vision?.models?.[provider] ?? '') : '';
  /** What the reader answers with — memory or the vision default (''
   * for compatible until the user types an id). */
  const resolvedModel = remembered || VISION_DEFAULT_MODEL[provider] || '';
  const usable =
    provider !== '' &&
    resolvedModel !== '' &&
    (isCompat ? compatUrl !== '' : masked !== null);

  // A new provider needs a fresh model list and its remembered free text.
  useEffect(() => {
    setModels(null);
    setError('');
    setModelText(config?.vision?.models?.[provider] ?? '');
    if (provider) {
      modelListAvailable(provider)
        .then(setModels)
        .catch(() => setModels([]));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider]);

  const pickProvider = (id: string) => {
    void configSet('vision.provider', id)
      .then(data.setConfig)
      .catch(() => setError('Could not save provider'));
  };

  const chooseModel = (model: string) => {
    if (!provider || !model) {
      return;
    }
    void configSet(`vision.models.${provider}`, model)
      .then(data.setConfig)
      .catch(() => setError('Could not save model'));
  };

  /** Model options; keep a stale/default selection visible. */
  const modelOptions = (): string[] => {
    const list = models ?? [];
    return resolvedModel && !list.includes(resolvedModel)
      ? [resolvedModel, ...list]
      : list;
  };

  const note = () => {
    if (isCompat) {
      if (!compatUrl) {
        return 'Inactive — set the OpenAI-compatible endpoint on the LLM tab.';
      }
      if (!resolvedModel) {
        return 'Inactive — type the endpoint’s vision model id.';
      }
      return (
        <>
          Posts to <span className={NUM}>{compatUrl}/chat/completions</span>
          {masked ? ` · key ${masked}` : ' · no key — open endpoint'}.
        </>
      );
    }
    if (!masked) {
      return `Inactive — add a ${providerLabel(provider, '')} key on the LLM tab.`;
    }
    return `Reads with your ${providerLabel(provider, '')} key (${masked}).`;
  };

  return (
    <>
      <p className={SUB}>
        A separate vision model describes each screenshot and hands the text to
        your chat provider — the LLM chain never needs image support. Keys and
        the compatible endpoint come from the LLM tab.
      </p>

      <div className={cn(PROV_CARD, 'border-border')}>
        <div className='flex flex-wrap items-center gap-2'>
          <span
            className={cn(
              'size-1.75 flex-none rounded-full',
              usable
                ? 'bg-accent'
                : 'bg-[color-mix(in_oklch,var(--fg)_20%,transparent)]',
            )}
          />
          <span className={LBL}>Provider</span>
          <ToggleGroup
            value={[provider === '' ? 'off' : provider]}
            onValueChange={(v) =>
              pickProvider(
                v[0] === 'off' || v[0] === undefined ? '' : String(v[0]),
              )
            }
            variant='outline'
            size='sm'
            spacing={2}
            className='min-w-0 flex-wrap'
            aria-label='Screen-reading provider'>
            <ToggleGroupItem
              value='off'
              className={TOGGLE_ITEM}>
              Off
            </ToggleGroupItem>
            {VISION_PROVIDERS.map((d) => (
              <ToggleGroupItem
                key={d.id}
                value={d.id}
                className={TOGGLE_ITEM}>
                {providerLabel(d.id, config?.compat.name ?? '')}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        </div>

        {provider !== '' && (
          <>
            <div className='mt-2.5 flex items-center gap-2'>
              <span className={LBL}>Model</span>
              {isCompat ? (
                <>
                  <Input
                    className={FIELD}
                    list='vision-models-compatible'
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
                    placeholder='model id — e.g. qwen2.5-vl-72b-instruct'
                    autoComplete='off'
                    spellCheck={false}
                    aria-label='Vision model id'
                  />
                  <datalist id='vision-models-compatible'>
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
                  disabled={modelOptions().length === 0}
                  onChange={(e) => chooseModel(e.target.value)}
                  aria-label='Vision model'>
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
            <p className={PROV_NOTE}>{note()}</p>
            {error && <p className={PROV_ERR}>{error}</p>}
          </>
        )}
      </div>
    </>
  );
};
