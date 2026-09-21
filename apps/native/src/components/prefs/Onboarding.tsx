/**
 * Onboarding — the five-step wizard, no sidebar (DESIGN.md §6). One
 * mount = one run: the shell remounts this subtree on every mode switch,
 * which is how "re-run setup" lands on step 1.
 *
 * Steps 2–3 drive the same commands the bar's gate cards use —
 * `keystore_init`/`keystore_unlock` and `permissions_request_screen` —
 * so state can never split between surfaces. Step 4's BYOK picker reuses
 * the catalog the Providers tab renders; saving a compatible provider
 * persists `compat.name` + `compat.base_url` first (validation resolves
 * the endpoint server-side), then validates, then stores the key.
 * Completion is `app.onboarding_done` — written by "Open settings" only,
 * so quitting mid-wizard replays onboarding on next launch.
 */
import { useState } from 'react';
import { Check, Fingerprint, ShieldCheck } from '@marvis/ui';
import {
  configSet,
  keystoreInit,
  keystoreSetKey,
  keystoreUnlock,
  modelListAvailable,
  modelSetSelected,
  modelValidateKey,
  permissionsRequestScreen,
  windowShowSettings,
} from '../../lib/commands';
import { providerFor, PROVIDERS } from '../../lib/providers';
import { AuthSheet } from './AuthSheet';
import type { PrefsData } from './types';

const STEP_LABELS = [
  'welcome',
  'key vault',
  'screen access',
  'bring your own key',
  'done',
];
const URL_RE = /^https?:\/\//i;

export const Onboarding = ({ data }: { data: PrefsData }) => {
  const [step, setStep] = useState(0);

  const finish = async () => {
    try {
      const c = await configSet('app.onboarding_done', true);
      data.setConfig(c);
    } catch {
      // still flip into settings — a failed write replays the wizard
      // next launch, which is the safe failure direction
    }
    void windowShowSettings().catch(() => {});
  };

  return (
    <div className='prf-ob'>
      <div className='prf-ob-progress'>
        <div
          className='prf-ob-track'
          aria-hidden='true'>
          {STEP_LABELS.map((l, i) => (
            <span
              key={l}
              className={i < step ? 'done' : i === step ? 'on' : ''}
            />
          ))}
        </div>
        <span className='prf-ob-count'>
          {step + 1} / 5 · {STEP_LABELS[step]}
        </span>
      </div>

      <div className='prf-ob-main'>
        {step === 0 && <WelcomeStep onNext={() => setStep(1)} />}
        {step === 1 && (
          <VaultStep
            data={data}
            onNext={() => setStep(2)}
            onBack={() => setStep(0)}
          />
        )}
        {step === 2 && (
          <ScreenStep
            onNext={() => setStep(3)}
            onBack={() => setStep(1)}
          />
        )}
        {step === 3 && (
          <ByokStep
            data={data}
            onNext={() => setStep(4)}
            onBack={() => setStep(2)}
          />
        )}
        {step === 4 && <DoneStep onDone={() => void finish()} />}
      </div>
    </div>
  );
};

/* ── step 1 · welcome ─────────────────────────────────────────── */

const WelcomeStep = ({ onNext }: { onNext: () => void }) => (
  <>
    <img
      className='prf-ob-mark'
      src='/marvis-mark.svg'
      alt='Marvis mark'
    />
    <h1>Welcome to Marvis</h1>
    <p className='lede'>
      A bar that floats over everything, sees what's on your screen, and answers
      in place.
    </p>
    <div className='prf-ob-list'>
      <div className='li'>A single bar that drifts with you</div>
      <div className='li'>Ask about what's on screen — or about your call</div>
      <div className='li'>Your keys, your vault, your providers</div>
    </div>
    <div className='prf-ob-actions'>
      <button
        type='button'
        className='mv-btn mv-btn-primary'
        onClick={onNext}>
        Set up Marvis
      </button>
    </div>
  </>
);

/* ── step 2 · key vault ───────────────────────────────────────── */

const VaultStep = ({
  data,
  onNext,
  onBack,
}: {
  data: PrefsData;
  onNext: () => void;
  onBack: () => void;
}) => {
  const [pending, setPending] = useState(false);
  const [err, setErr] = useState('');

  const create = async () => {
    setErr('');
    setPending(true);
    try {
      // Unset → init creates keys.enc + the Keychain DEK (first run is
      // silent; unlock's read-back is the one system prompt). Locked →
      // the vault already exists, so init refuses — unlock instead.
      // Unlocked → nothing to do.
      let s = data.status?.state;
      if (s === 'Unset') {
        const res = await keystoreInit();
        data.setStatus(res);
        s = res.state;
      }
      if (s === 'Locked') {
        data.setStatus(await keystoreUnlock());
      }
      onNext();
    } catch (e) {
      setErr(typeof e === 'string' ? e : 'Could not create the vault');
    } finally {
      setPending(false);
    }
  };

  return (
    <>
      <div className='prf-ob-ico'>
        <Fingerprint />
      </div>
      <h1>Your keys stay yours</h1>
      <p className='lede'>
        Provider keys encrypt into{' '}
        <span className='num'>~/.marvis/keys.enc</span> (AES-256-GCM). The key
        lives in the macOS Keychain, guarded by Touch ID — Marvis never sees it.
      </p>
      <div className='prf-ob-actions'>
        <button
          type='button'
          className='mv-btn mv-btn-outline'
          onClick={onBack}
          disabled={pending}>
          Back
        </button>
        <button
          type='button'
          className='mv-btn mv-btn-primary'
          onClick={() => void create()}
          disabled={pending}>
          {pending ? (
            <>
              <span className='mv-spin' />
              Waiting for Touch ID…
            </>
          ) : data.status?.state === 'Locked' ? (
            'Unlock vault'
          ) : (
            'Create vault'
          )}
        </button>
      </div>
      {err && <p className='prov-err show'>{err}</p>}
      <AuthSheet
        open={pending}
        title={
          data.status?.state === 'Locked'
            ? 'Touch ID to unlock Marvis'
            : 'Touch ID to create your vault'
        }
      />
    </>
  );
};

/* ── step 3 · screen access ───────────────────────────────────── */

const ScreenStep = ({
  onNext,
  onBack,
}: {
  onNext: () => void;
  onBack: () => void;
}) => {
  const [busy, setBusy] = useState(false);

  const grant = async () => {
    setBusy(true);
    try {
      // Reuses the bar's gate path: opens System Settings if macOS
      // already decided, captures a probe frame otherwise. Denial lands
      // the bar's permission card — onboarding continues either way.
      await permissionsRequestScreen();
    } catch {
      // same — proceed; the ask surface reports it again when needed
    } finally {
      setBusy(false);
      onNext();
    }
  };

  return (
    <>
      <div className='prf-ob-ico'>
        <ShieldCheck />
      </div>
      <h1>Grant screen access</h1>
      <p className='lede'>
        macOS asks once. After that, Marvis keeps a rolling 60-second window at
        4 fps — enough for one screenshot when you ask.
      </p>
      <div className='prf-ob-actions'>
        <button
          type='button'
          className='mv-btn mv-btn-outline'
          onClick={onBack}
          disabled={busy}>
          Back
        </button>
        <button
          type='button'
          className='mv-btn mv-btn-outline'
          onClick={onNext}
          disabled={busy}>
          Not now
        </button>
        <button
          type='button'
          className='mv-btn mv-btn-primary'
          onClick={() => void grant()}
          disabled={busy}>
          {busy ? (
            <>
              <span className='mv-spin' />
              Checking…
            </>
          ) : (
            'Grant access'
          )}
        </button>
      </div>
    </>
  );
};

/* ── step 4 · bring your own key ──────────────────────────────── */

const ByokStep = ({
  data,
  onNext,
  onBack,
}: {
  data: PrefsData;
  onNext: () => void;
  onBack: () => void;
}) => {
  const [prov, setProv] = useState('openai');
  const [key, setKey] = useState('');
  const [name, setName] = useState(data.config?.compat.name ?? '');
  const [baseUrl, setBaseUrl] = useState(data.config?.compat.base_url ?? '');
  const [model, setModel] = useState('');
  const [modelList, setModelList] = useState<string[]>([]);
  const [phase, setPhase] = useState<'idle' | 'saving' | 'saved'>('idle');
  const [err, setErr] = useState('');

  const def = providerFor(prov) ?? PROVIDERS[0];
  const isCompat = !!def.compat;
  const isLocal = !!def.keyOptional && !isCompat;

  const switchProv = (id: string) => {
    setProv(id);
    setKey('');
    setModel('');
    setModelList([]);
    setPhase('idle');
    setErr('');
  };

  const save = async () => {
    const k = key.trim();
    setErr('');

    if (isCompat && !URL_RE.test(baseUrl.trim())) {
      setErr('Add a base URL first — it must start with http:// or https://');
      return;
    }
    if (!def.keyOptional && !k) {
      return;
    }
    setPhase('saving');
    try {
      if (isCompat) {
        // Endpoint config must land before validate — the probe reads
        // `compat.base_url` server-side.
        let c = await configSet('compat.name', name.trim());
        c = await configSet('compat.base_url', baseUrl.trim());
        data.setConfig(c);
      }
      const verdict = await modelValidateKey(prov, k);
      if (!verdict.ok) {
        setErr(verdict.error);
        setPhase('idle');
        return;
      }
      if (k) {
        data.setStatus(await keystoreSetKey(prov, k));
        setKey('');
      }
      const list = await modelListAvailable(prov).catch(() => [] as string[]);
      setModelList(list);
      setModel((m) => m || list[0] || '');
      setPhase('saved');
    } catch (e) {
      setErr(typeof e === 'string' ? e : 'Validation failed');
      setPhase('idle');
    }
  };

  const cont = async () => {
    // Persisting the pick is what makes the ask surface actually use it.
    if (model.trim()) {
      try {
        data.setSelected(await modelSetSelected(prov, model.trim()));
      } catch {
        // non-fatal — providers stay saved; the ask errors inline later
      }
    }
    onNext();
  };

  const canSave =
    phase !== 'saving' &&
    (isCompat
      ? URL_RE.test(baseUrl.trim())
      : isLocal
        ? true
        : key.trim().length > 0);

  const saveLabel = isLocal
    ? 'Check daemon'
    : isCompat
      ? 'Validate endpoint'
      : 'Validate & save';

  return (
    <>
      <h1>Bring your own key</h1>
      <p className='lede'>
        Chat and listen run on whichever provider you configure. Keys go
        straight to the vault — they're never in the config file, never in logs.
      </p>

      <div className='prf-ob-pills'>
        {PROVIDERS.map((p) => (
          <button
            key={p.id}
            type='button'
            className={`prf-ob-pill${prov === p.id ? ' is-on' : ''}`}
            onClick={() => switchProv(p.id)}>
            {p.label}
          </button>
        ))}
      </div>

      {isCompat && (
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
            placeholder='Base URL — e.g. https://api.groq.com/openai/v1'
            autoComplete='off'
            spellCheck={false}
            aria-label='Compatible base URL'
          />
        </div>
      )}

      {!isLocal && (
        <input
          className='key-input prf-ob-key'
          type='password'
          autoComplete='off'
          value={key}
          onChange={(e) => setKey(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && void save()}
          placeholder={
            isCompat
              ? 'API key — optional on open endpoints'
              : def.keyPlaceholder
          }
          aria-label='API key'
        />
      )}

      <div className='prf-ob-actions prf-ob-actions-mid'>
        <button
          type='button'
          className='mv-btn mv-btn-primary'
          onClick={() => void save()}
          disabled={!canSave}>
          {phase === 'saving' ? (
            <>
              <span className='mv-spin' />
              {isLocal ? 'Checking…' : 'Validating…'}
            </>
          ) : (
            saveLabel
          )}
        </button>
      </div>
      {err && <p className='prov-err show'>{err}</p>}
      {phase === 'saved' && !isLocal && (
        <p className='prov-ok'>
          <Check /> Endpoint verified — key in vault.
        </p>
      )}
      {phase === 'saved' && isLocal && modelList.length === 0 && (
        <p className='prov-note'>
          Daemon answered but lists no models — pull one first.
        </p>
      )}

      {phase === 'saved' && (isCompat || isLocal || modelList.length > 0) && (
        <div className='prf-ob-model'>
          <span className='lbl'>Model</span>
          {isCompat ? (
            <>
              <input
                className='key-input'
                list='ob-compat-models'
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder='model id — e.g. llama-3.3-70b-versatile'
                autoComplete='off'
                aria-label='Compatible model id'
              />
              <datalist id='ob-compat-models'>
                {modelList.map((m) => (
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
              value={model}
              onChange={(e) => setModel(e.target.value)}
              aria-label='Model'>
              <option value=''>Select model</option>
              {modelList.map((m) => (
                <option
                  key={m}
                  value={m}>
                  {m}
                </option>
              ))}
            </select>
          )}
        </div>
      )}

      <div className='prf-ob-actions'>
        <button
          type='button'
          className='mv-btn mv-btn-outline'
          onClick={onBack}>
          Back
        </button>
        <button
          type='button'
          className='mv-btn mv-btn-link'
          onClick={onNext}>
          Do this later
        </button>
        <button
          type='button'
          className='mv-btn mv-btn-primary'
          onClick={() => void cont()}
          disabled={phase !== 'saved' || !model.trim()}>
          Continue
        </button>
      </div>
    </>
  );
};

/* ── step 5 · done ────────────────────────────────────────────── */

const DoneStep = ({ onDone }: { onDone: () => void }) => (
  <>
    <img
      className='prf-ob-mark'
      src='/marvis-mark.svg'
      alt='Marvis mark'
    />
    <h1>You're set</h1>
    <p className='lede'>
      Marvis is always within reach. <span className='num'>⌘/</span> shows or
      hides everything; <span className='num'>⌘⏎</span> asks about what's on
      screen.
    </p>
    <div className='prf-ob-actions'>
      <button
        type='button'
        className='mv-btn mv-btn-primary'
        onClick={onDone}>
        Open settings
      </button>
    </div>
  </>
);
