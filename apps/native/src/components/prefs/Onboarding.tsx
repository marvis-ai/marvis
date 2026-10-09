/**
 * Onboarding — the six-step wizard, no sidebar (DESIGN.md §6). One
 * mount = one run: the shell remounts this subtree on every mode switch,
 * which is how "re-run setup" lands on step 1.
 *
 * Step 2's language pick doubles as the STT recommendation — English
 * seeds Whisper, every other language seeds Sherpa's SenseVoice — and
 * the voice step can still override. Step 3 drives the same
 * `permissions_request_screen` command the bar's gate card uses, so
 * state can never split between surfaces. Step 4's BYOK picker reuses
 * the catalog the Providers tab renders; saving a compatible provider
 * persists `compat.name` + `compat.base_url` first (validation resolves
 * the endpoint server-side), then validates, then stores the key in
 * `keys.json`. A successful save also promotes the provider to the head
 * of `providers.order` — the wizard's pick should be the one that
 * answers — and clears its disabled flag.
 * Completion is `app.onboarding_done` — written by "Open settings" only,
 * so quitting mid-wizard replays onboarding on next launch. The backend
 * reveals the bar on that write.
 */
import { useState } from 'react';
import { CheckIcon, ShieldCheckIcon } from '@marvis/ui';
import {
  configSet,
  keystoreSetKey,
  modelGetSelected,
  modelListAvailable,
  modelSetSelected,
  modelValidateKey,
  permissionsRequestScreen,
  providersReorder,
  providerSetEnabled,
  windowShowSettings,
} from '@/lib/commands';
import { providerFor, PROVIDERS } from '@/lib/providers';
import { LANGUAGES, recommendedSttProvider } from '@/lib/languages';
import { VoiceSetup } from './VoiceSetup';
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
  PROV_ERR,
  PROV_NOTE,
  SPIN,
  cn,
} from '@/lib/classes';
import type { PrefsData } from './types';

const STEP_LABELS = [
  'welcome',
  'main language',
  'screen access',
  'bring your own key',
  'voice',
  'done',
];
const URL_RE = /^https?:\/\//i;

/* onboarding-mode text atoms (the 480px step column) */
const H1 = 'mb-2 text-[21px] font-bold tracking-[-0.02em]';
const LEDE = 'max-w-[44ch] text-[13px] leading-[1.6] text-muted-foreground';
const ACTIONS = 'mt-5.5 flex items-center gap-2.5';
const MARK = 'mb-3.5 size-11';

/* The pill picker shared by the language + BYOK steps. */
const chipCls = (active: boolean) =>
  cn(
    'rounded-full border bg-transparent px-3 py-1.5 text-xs font-[550] transition-[border-color,color,background] duration-(--motion-fast) ease-(--ease) motion-reduce:transition-none',
    active
      ? 'border-primary bg-primary text-primary-foreground'
      : 'border-border text-foreground hover:border-[color-mix(in_oklch,var(--fg)_30%,var(--border))]',
  );

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
    <div className='flex min-h-0 flex-1 flex-col bg-background'>
      <div className='flex flex-none items-center gap-3.5 px-6 pt-9'>
        <div
          className='flex flex-1 gap-1.25'
          aria-hidden='true'>
          {STEP_LABELS.map((l, i) => (
            <span
              key={l}
              className={cn(
                'h-0.75 flex-1 rounded-full transition-colors duration-(--motion-base) ease-(--ease) motion-reduce:transition-none',
                i <= step
                  ? 'bg-primary'
                  : 'bg-[color-mix(in_oklch,var(--fg)_10%,transparent)]',
              )}
            />
          ))}
        </div>
        <span className='flex-none font-mono text-[10.5px] whitespace-nowrap text-muted-foreground'>
          {step + 1} / {STEP_LABELS.length} · {STEP_LABELS[step]}
        </span>
      </div>

      <div className='min-h-0 flex-1 overflow-y-auto p-6'>
        {/* 480px step column; keying on step re-runs the fade per page */}
        <div
          key={step}
          className='mx-auto max-w-120 animate-fade-in'>
          {step === 0 && <WelcomeStep onNext={() => setStep(1)} />}
          {step === 1 && (
            <LanguageStep
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
          {step === 4 && (
            <VoiceStep
              data={data}
              onSkip={() => setStep(5)}
              onContinue={() => setStep(5)}
              onBack={() => setStep(3)}
            />
          )}
          {step === 5 && <DoneStep onDone={() => void finish()} />}
        </div>
      </div>
    </div>
  );
};

const VoiceStep = ({
  data,
  onSkip,
  onContinue,
  onBack,
}: {
  data: PrefsData;
  onSkip: () => void;
  onContinue: () => void;
  onBack: () => void;
}) => (
  <>
    <h1 className={H1}>Set up voice</h1>
    <p className={cn(LEDE, 'mb-4')}>
      Voice is optional.{' '}
      {recommendedSttProvider(data.config?.app.main_language ?? 'en') ===
      'whisper'
        ? 'For English, Whisper is the recommended local engine — pre-selected below.'
        : 'For your language, SenseVoice (Sherpa) is the recommended local engine — pre-selected below.'}{' '}
      Configure it now, or set it up later in Settings.
    </p>
    <VoiceSetup
      data={data}
      showSkip={true}
      onSkip={onSkip}
      onContinue={onContinue}
      onBack={onBack}
      cancelOnUnmount={true}
    />
  </>
);

/* ── step 1 · welcome ─────────────────────────────────────────── */

const WelcomeStep = ({ onNext }: { onNext: () => void }) => (
  <>
    <img
      className={MARK}
      src='/marvis-mark.svg'
      alt='Marvis mark'
    />
    <h1 className={H1}>Welcome to Marvis</h1>
    <p className={LEDE}>
      A bar that floats over everything, sees what's on your screen, and answers
      in place.
    </p>
    <div className='mt-4 flex flex-col gap-2.5'>
      <div className='flex items-start gap-2.5 text-[12.5px] text-muted-foreground'>
        A single bar that drifts with you
      </div>
      <div className='flex items-start gap-2.5 text-[12.5px] text-muted-foreground'>
        Ask about what's on screen — or about your call
      </div>
      <div className='flex items-start gap-2.5 text-[12.5px] text-muted-foreground'>
        Your keys stay on this Mac, under your providers
      </div>
    </div>
    <div className={ACTIONS}>
      <button
        type='button'
        className={cn(BTN_LG, BTN_PRIMARY)}
        onClick={onNext}>
        Set up Marvis
      </button>
    </div>
  </>
);

/* ── step 2 · main language ───────────────────────────────────── */

const LanguageStep = ({
  data,
  onNext,
  onBack,
}: {
  data: PrefsData;
  onNext: () => void;
  onBack: () => void;
}) => {
  const [lang, setLang] = useState(data.config?.app.main_language ?? 'en');
  const [busy, setBusy] = useState(false);

  const cont = async () => {
    setBusy(true);
    try {
      // The language pick doubles as the STT recommendation — English
      // seeds Whisper, every other language seeds SenseVoice (Sherpa).
      // The backend re-pairs `stt_model` to a valid catalog default, and
      // the voice step can still override the provider.
      let c = await configSet('app.main_language', lang);
      c = await configSet('models.stt_provider', recommendedSttProvider(lang));
      data.setConfig(c);
    } catch {
      // non-fatal — defaults hold; the voice step shows whatever landed
    } finally {
      setBusy(false);
      onNext();
    }
  };

  return (
    <>
      <h1 className={H1}>Main language</h1>
      <p className={LEDE}>
        Chat replies, meeting summaries, and dictation default to this language.
      </p>
      <div className='mt-3.5 flex flex-wrap gap-1.5'>
        {LANGUAGES.map((l) => (
          <button
            key={l.id}
            type='button'
            className={chipCls(lang === l.id)}
            onClick={() => setLang(l.id)}>
            {l.label}
          </button>
        ))}
      </div>
      <p className={PROV_NOTE}>
        {lang === 'en'
          ? 'Voice transcription will default to local Whisper — best on English. Switchable in the voice step.'
          : 'Voice transcription will default to SenseVoice (Sherpa) — the multilingual engine. Switchable in the voice step.'}
      </p>
      <div className={ACTIONS}>
        <button
          type='button'
          className={cn(BTN_LG, BTN_OUTLINE)}
          onClick={onBack}
          disabled={busy}>
          Back
        </button>
        <button
          type='button'
          className={cn(BTN_LG, BTN_PRIMARY)}
          onClick={() => void cont()}
          disabled={busy}>
          Continue
        </button>
      </div>
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
      <div className='mb-3.5 grid size-11 place-items-center rounded-xl bg-fg-2 text-surface'>
        <ShieldCheckIcon className='size-5' />
      </div>
      <h1 className={H1}>Grant screen access</h1>
      <p className={LEDE}>
        macOS asks once. After that, Marvis keeps a rolling 60-second window at
        4 fps — enough for one screenshot when you ask.
      </p>
      <div className={ACTIONS}>
        <button
          type='button'
          className={cn(BTN_LG, BTN_OUTLINE)}
          onClick={onBack}
          disabled={busy}>
          Back
        </button>
        <button
          type='button'
          className={cn(BTN_LG, BTN_OUTLINE)}
          onClick={onNext}
          disabled={busy}>
          Not now
        </button>
        <button
          type='button'
          className={cn(BTN_LG, BTN_PRIMARY)}
          onClick={() => void grant()}
          disabled={busy}>
          {busy ? (
            <>
              <span className={SPIN} />
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
      // The wizard's pick should be the one that answers: switch it on
      // and move it to the head of the failover order (the backend
      // normalizes + appends any missing ids, so existing order holds).
      let cfg = data.config;
      if (cfg?.providers.disabled.includes(prov)) {
        cfg = await providerSetEnabled(prov, true);
      }
      const rest = (cfg?.providers.order ?? []).filter((id) => id !== prov);
      cfg = await providersReorder([prov, ...rest]);
      data.setConfig(cfg);
      // The promoted provider is almost certainly the chain head now —
      // resolve it so `data.selected` agrees before the Done step.
      data.setSelected(await modelGetSelected());
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
        const s = await modelSetSelected(prov, model.trim());
        if (s) {
          data.setSelected(s);
        }
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
      <h1 className={H1}>Bring your own key</h1>
      <p className={LEDE}>
        Chat and listen run on whichever provider you configure. Keys live in{' '}
        <span className={NUM}>~/.marvis/keys.json</span> — readable only by you,
        never in the config file, never in logs.
      </p>

      <div className='mt-3.5 mb-3 flex flex-wrap gap-1.5'>
        {PROVIDERS.map((p) => (
          <button
            key={p.id}
            type='button'
            className={chipCls(prov === p.id)}
            onClick={() => switchProv(p.id)}>
            {p.label}
          </button>
        ))}
      </div>

      {isCompat && (
        <div className='mb-2 flex flex-col gap-2'>
          <input
            className={FIELD}
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder='Provider name — e.g. Groq, Together, vLLM'
            autoComplete='off'
            aria-label='Compatible provider name'
          />
          <input
            className={FIELD}
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
          className={FIELD}
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

      <div className='mt-2.5 flex items-center gap-2.5'>
        <button
          type='button'
          className={cn(BTN_LG, BTN_PRIMARY)}
          onClick={() => void save()}
          disabled={!canSave}>
          {phase === 'saving' ? (
            <>
              <span className={SPIN} />
              {isLocal ? 'Checking…' : 'Validating…'}
            </>
          ) : (
            saveLabel
          )}
        </button>
      </div>
      {err && <p className={PROV_ERR}>{err}</p>}
      {phase === 'saved' && !isLocal && (
        <p className='mt-2 flex items-center gap-1.5 text-[11px] text-accent-text'>
          <CheckIcon className='size-3.25' /> Endpoint verified — key stored.
        </p>
      )}
      {phase === 'saved' && isLocal && modelList.length === 0 && (
        <p className={PROV_NOTE}>
          Daemon answered but lists no models — pull one first.
        </p>
      )}

      {phase === 'saved' && (isCompat || isLocal || modelList.length > 0) && (
        <div className='mt-2.5 flex items-center gap-2'>
          <span className={LBL}>Model</span>
          {isCompat ? (
            <>
              <input
                className={FIELD}
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
              className={MODEL_SEL}
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

      <div className={ACTIONS}>
        <button
          type='button'
          className={cn(BTN_LG, BTN_OUTLINE)}
          onClick={onBack}>
          Back
        </button>
        <button
          type='button'
          className={cn(BTN_LINK_LG, BTN_LINK)}
          onClick={onNext}>
          Do this later
        </button>
        <button
          type='button'
          className={cn(BTN_LG, BTN_PRIMARY)}
          onClick={() => void cont()}
          disabled={phase !== 'saved' || !model.trim()}>
          Continue
        </button>
      </div>
    </>
  );
};

/* ── step 6 · done ────────────────────────────────────────────── */

const DoneStep = ({ onDone }: { onDone: () => void }) => (
  <>
    <img
      className={MARK}
      src='/marvis-mark.svg'
      alt='Marvis mark'
    />
    <h1 className={H1}>You're set</h1>
    <p className={LEDE}>
      Marvis is always within reach. <span className={NUM}>⌘/</span> shows or
      hides everything; <span className={NUM}>⌘⏎</span> asks about what's on
      screen.
    </p>
    <div className={ACTIONS}>
      <button
        type='button'
        className={cn(BTN_LG, BTN_PRIMARY)}
        onClick={onDone}>
        Open settings
      </button>
    </div>
  </>
);
