/**
 * `?view=settings` — provider keys + model selection panel (240 px wide,
 * transparent, frameless, non-resizable; `Panel::Settings` in
 * windows/mod.rs caps it at 400 px tall).
 *
 * Key hygiene: the API-key inputs are `type="password"`, cleared the
 * moment a save succeeds, and the masked display only ever renders the
 * backend's `…last4` string — a typed key is never echoed back into the
 * DOM. Save flow is validate-then-store: `model_validate_key` probes the
 * key without persisting (failures are data, `{ok:false,error}`, not
 * invoke errors), and only `keystore_set_key` writes to `keys.enc` (it
 * re-validates server-side, so a stale success can never store a dead
 * key either).
 *
 * While the keystore isn't `Unlocked` the panel renders a compact
 * notice — the bar owns all passphrase UX. `keystore_lock` additionally
 * drops the app out of the `Main` gate, which hides every panel, so the
 * unlocked UI below only ever runs against an unlocked store.
 *
 * Ollama is special-cased: it needs no key (its `validate` probes the
 * local daemon's `/api/tags`), so its row shows "local, no key needed"
 * and a model dropdown only. `model_list_available("ollama")` maps a
 * down daemon to `[]`, surfaced as an inline note.
 *
 * Height is reported like AskPanel's — `window_adjust_height` on a
 * throttled ResizeObserver — and the window hides itself 250 ms after a
 * blur (debounced so devtools/IME focus churn can't flicker it away).
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Button, ShieldAlert, X } from '@marvis/ui';
import {
  keystoreLock,
  keystoreRemoveKey,
  keystoreSetKey,
  keystoreStatus,
  modelGetSelected,
  modelListAvailable,
  modelSetSelected,
  modelValidateKey,
  windowAdjustHeight,
  windowHideSettings,
  type KeystoreStatus,
  type ModelSelection,
} from '../lib/commands';
import { EV_KEYSTORE_CHANGED, useTauriEvent } from '../lib/events';
import { RetryCard } from '../components/RetryCard';

/** `Panel::Settings::max_height` (windows/mod.rs) — mirrored client-side. */
const WINDOW_CAP = 400;
/** Outer `p-1` wrapper: 4 px top + bottom of transparent chrome. */
const CHROME_PX = 8;
/** Panel's own ceiling so the reported height never exceeds the cap. */
const PANEL_MAX = WINDOW_CAP - CHROME_PX;
/** Reported-height deadband + invoke throttle (same tuning as AskPanel). */
const HEIGHT_EPS = 4;
const HEIGHT_MS = 150;
/** Blur must hold this long before the window hides itself. */
const BLUR_DEBOUNCE_MS = 250;

interface ProviderDef {
  /** `ProviderKind::as_str` — the id every command expects. */
  id: string;
  label: string;
  /** Local daemon (Ollama): no key input, model dropdown only. */
  local?: boolean;
}

const PROVIDERS: ProviderDef[] = [
  { id: 'openai', label: 'OpenAI' },
  { id: 'anthropic', label: 'Anthropic' },
  { id: 'gemini', label: 'Gemini' },
  { id: 'ollama', label: 'Ollama', local: true },
];

export default function SettingsPanel() {
  const [status, setStatus] = useState<KeystoreStatus | null>(null);
  const [selected, setSelected] = useState<ModelSelection | null>(null);
  const [models, setModels] = useState<Record<string, string[]>>({});
  /** Transient key input per provider — cleared after a successful save. */
  const [inputs, setInputs] = useState<Record<string, string>>({});
  /** Inline per-provider validation/save error. */
  const [errors, setErrors] = useState<Record<string, string>>({});
  /** Provider id with an in-flight save/remove, else null. */
  const [saving, setSaving] = useState<string | null>(null);
  const [locking, setLocking] = useState(false);
  const [bootError, setBootError] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  const busy = saving !== null || locking;

  useEffect(() => {
    document.body.classList.add('settings');
    return () => document.body.classList.remove('settings');
  }, []);

  // Report content height: leading + trailing throttle, only on a real
  // (>EPS) change — `adjust_height` animates the window per call.
  useEffect(() => {
    const el = panelRef.current;
    if (!el) {
      return;
    }
    let lastValue = -1;
    let lastSentAt = 0;
    let timer: number | undefined;
    const report = () => {
      const h = Math.min(Math.ceil(el.offsetHeight + CHROME_PX), WINDOW_CAP);
      if (Math.abs(h - lastValue) <= HEIGHT_EPS) {
        return;
      }
      const wait = HEIGHT_MS - (Date.now() - lastSentAt);
      if (wait <= 0) {
        lastValue = h;
        lastSentAt = Date.now();
        void windowAdjustHeight('settings', h).catch(() => {});
      } else if (timer === undefined) {
        timer = window.setTimeout(() => {
          timer = undefined;
          report();
        }, wait);
      }
    };
    const observer = new ResizeObserver(report);
    observer.observe(el);
    return () => {
      observer.disconnect();
      window.clearTimeout(timer);
    };
  }, []);

  // Hide on blur, debounced: clicking elsewhere dismisses the panel,
  // but brief devtools/IME focus churn re-focuses before the timer runs.
  useEffect(() => {
    let timer: number | undefined;
    const unlisten = getCurrentWindow().onFocusChanged(
      ({ payload: focused }) => {
        window.clearTimeout(timer);
        timer = undefined;
        if (focused) {
          return;
        }
        timer = window.setTimeout(() => {
          void windowHideSettings().catch(() => {});
        }, BLUR_DEBOUNCE_MS);
      },
    );
    return () => {
      window.clearTimeout(timer);
      void unlisten.then((u) => u());
    };
  }, []);

  const bootstrap = useCallback(async () => {
    try {
      const [ks, sel, ...lists] = await Promise.all([
        keystoreStatus(),
        modelGetSelected(),
        ...PROVIDERS.map((p) => modelListAvailable(p.id)),
      ]);
      setStatus(ks);
      setSelected(sel);
      setModels(
        Object.fromEntries(PROVIDERS.map((p, i) => [p.id, lists[i] ?? []])),
      );
      setBootError(false);
    } catch {
      setBootError(true);
    }
  }, []);

  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  // Every keystore mutation broadcasts the fresh masked status.
  useTauriEvent<KeystoreStatus>(EV_KEYSTORE_CHANGED, setStatus);

  const maskedFor = (id: string): string | null =>
    status?.keys.find(([p]) => p === id)?.[1] ?? null;

  /** Model options for a row; keep a stale configured selection visible. */
  const modelOptions = (id: string): string[] => {
    const list = models[id] ?? [];
    const sel = selected?.provider === id ? selected.model : '';
    return sel && !list.includes(sel) ? [sel, ...list] : list;
  };

  const setRowError = (id: string, message: string) =>
    setErrors((e) => ({ ...e, [id]: message }));

  /** Validate without storing, then store; clear the input on success. */
  const saveKey = async (id: string) => {
    const key = (inputs[id] ?? '').trim();
    if (!key || busy) {
      return;
    }
    setSaving(id);
    setRowError(id, '');
    try {
      const verdict = await modelValidateKey(id, key);
      if (!verdict.ok) {
        setRowError(id, verdict.error);
        return;
      }
      setStatus(await keystoreSetKey(id, key));
      setInputs((i) => ({ ...i, [id]: '' }));
    } catch (err) {
      setRowError(id, typeof err === 'string' ? err : 'Save failed');
    } finally {
      setSaving(null);
    }
  };

  const removeKey = async (id: string) => {
    if (busy) {
      return;
    }
    setSaving(id);
    setRowError(id, '');
    try {
      setStatus(await keystoreRemoveKey(id));
    } catch (err) {
      setRowError(id, typeof err === 'string' ? err : 'Remove failed');
    } finally {
      setSaving(null);
    }
  };

  /** `model_set_selected` makes this provider+model the active LLM pair. */
  const chooseModel = (id: string, model: string) => {
    if (!model) {
      return;
    }
    void modelSetSelected(id, model)
      .then(setSelected)
      .catch(() => setRowError(id, 'Could not save model selection'));
  };

  const lock = async () => {
    if (busy) {
      return;
    }
    setLocking(true);
    try {
      setStatus(await keystoreLock());
    } catch {
      // Lock failing leaves the store as it was — nothing to surface.
    } finally {
      setLocking(false);
    }
  };

  const hide = () => void windowHideSettings().catch(() => {});

  if (bootError) {
    return (
      <div className='p-1'>
        <div className='flex items-center justify-center rounded-2xl border border-border bg-card/90 px-3 py-6 shadow-lg backdrop-blur'>
          <RetryCard onRetry={() => void bootstrap()} />
        </div>
      </div>
    );
  }

  return (
    <div className='p-1'>
      <div
        ref={panelRef}
        style={{ maxHeight: PANEL_MAX }}
        className='flex flex-col overflow-hidden rounded-2xl border border-border bg-card/90 shadow-lg backdrop-blur'>
        <header className='flex items-center gap-1 border-b border-border px-3 py-2'>
          <p className='min-w-0 flex-1 text-xs leading-5 font-medium text-foreground'>
            Settings
          </p>
          <Button
            type='button'
            size='icon-xs'
            variant='ghost'
            title='Close'
            className='-mt-0.5 shrink-0 text-muted-foreground'
            onClick={hide}>
            <X />
          </Button>
        </header>

        {status === null ? (
          <p className='px-3 py-3 text-xs text-muted-foreground'>Loading…</p>
        ) : status.state !== 'Unlocked' ? (
          // Passphrase UX belongs to the bar — this is only a pointer.
          <div className='flex items-center gap-2 px-3 py-3'>
            <ShieldAlert className='size-4 shrink-0 text-muted-foreground' />
            <p className='min-w-0 flex-1 text-xs text-muted-foreground'>
              Keystore {status.state === 'Unset' ? 'not set up yet' : 'locked'}{' '}
              — unlock in the bar.
            </p>
            <Button
              size='xs'
              variant='outline'
              onClick={hide}>
              Hide
            </Button>
          </div>
        ) : (
          <div className='min-h-0 flex-1 overflow-y-auto'>
            {PROVIDERS.map((p) => {
              const masked = maskedFor(p.id);
              const opts = modelOptions(p.id);
              const rowError = errors[p.id];
              return (
                <div
                  key={p.id}
                  className='border-b border-border px-3 py-2'>
                  <div className='flex items-baseline justify-between gap-2'>
                    <span className='text-xs font-medium text-foreground'>
                      {p.label}
                    </span>
                    {p.local ? (
                      <span className='text-[10px] text-muted-foreground'>
                        local · no key needed
                      </span>
                    ) : masked ? (
                      <span className='flex shrink-0 items-center gap-1'>
                        <span className='text-[10px] text-muted-foreground'>
                          {masked}
                        </span>
                        <Button
                          type='button'
                          size='xs'
                          variant='link'
                          className='h-auto px-0.5 text-[10px]'
                          disabled={busy}
                          onClick={() => void removeKey(p.id)}>
                          Remove
                        </Button>
                      </span>
                    ) : (
                      <span className='text-[10px] text-muted-foreground'>
                        not set
                      </span>
                    )}
                  </div>

                  {!p.local && (
                    <div className='mt-1.5 flex items-center gap-1'>
                      <input
                        type='password'
                        autoComplete='off'
                        value={inputs[p.id] ?? ''}
                        onChange={(e) =>
                          setInputs((i) => ({ ...i, [p.id]: e.target.value }))
                        }
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            void saveKey(p.id);
                          }
                        }}
                        placeholder={masked ? 'Replace key' : 'API key'}
                        className='h-6 min-w-0 flex-1 rounded-md border border-border bg-input/30 px-2 text-xs text-foreground outline-none placeholder:text-muted-foreground focus:border-ring'
                      />
                      <Button
                        type='button'
                        size='xs'
                        disabled={busy || !(inputs[p.id] ?? '').trim()}
                        onClick={() => void saveKey(p.id)}>
                        {saving === p.id ? 'Saving…' : 'Save'}
                      </Button>
                    </div>
                  )}
                  {rowError && (
                    <p className='mt-1 text-[10px] leading-3 break-words text-destructive'>
                      {rowError}
                    </p>
                  )}

                  <select
                    value={selected?.provider === p.id ? selected.model : ''}
                    disabled={busy || opts.length === 0}
                    onChange={(e) => chooseModel(p.id, e.target.value)}
                    className='mt-1.5 h-6 w-full rounded-md border border-border bg-input/30 px-1.5 text-xs text-foreground outline-none focus:border-ring disabled:opacity-50'>
                    <option value=''>
                      {opts.length === 0 ? 'No models found' : 'Select model'}
                    </option>
                    {opts.map((m) => (
                      <option
                        key={m}
                        value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                  {p.id === 'ollama' && opts.length === 0 && (
                    <p className='mt-1 text-[10px] leading-3 text-muted-foreground'>
                      No models — is the Ollama daemon running?
                    </p>
                  )}
                </div>
              );
            })}

            {/* STT, not an LLM provider — static placeholder until Phase 2. */}
            <div className='flex items-baseline justify-between gap-2 border-b border-border px-3 py-2 opacity-60'>
              <span className='text-xs font-medium text-foreground'>
                Deepgram
              </span>
              <span className='text-[10px] text-muted-foreground'>Phase 2</span>
            </div>

            <div className='px-3 py-2'>
              <Button
                type='button'
                size='xs'
                variant='outline'
                className='w-full'
                disabled={busy}
                onClick={() => void lock()}>
                {locking ? 'Locking…' : 'Lock keys'}
              </Button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
