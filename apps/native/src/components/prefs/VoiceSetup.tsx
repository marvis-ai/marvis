import { useCallback, useEffect, useRef, useState } from 'react';
import { Input } from '@marvis/ui';
import {
  configSet,
  keystoreRemoveKey,
  keystoreSetKey,
  sherpaCancelDownload,
  sherpaDownload,
  sherpaRemoveModel,
  sherpaStatus,
  voiceEnrollCancel,
  voiceEnrollStart,
  voiceEnrollStop,
  voiceModelsCatalog,
  voiceprintRemove,
  voiceprintStatus,
  whisperCancelDownload,
  whisperDownload,
  whisperRemoveModel,
  whisperStatus,
  type SherpaStatus,
  type VoiceModelCatalogEntry,
  type VoiceprintStatus,
  type WhisperBinarySource,
  type WhisperStatus,
} from '../../lib/commands';
import {
  EV_SHERPA_DOWNLOAD_ERROR,
  EV_SHERPA_DOWNLOAD_PROGRESS,
  EV_WHISPER_DOWNLOAD_ERROR,
  EV_WHISPER_DOWNLOAD_PROGRESS,
  useTauriEvent,
  type SherpaDownloadErrorPayload,
  type SherpaDownloadProgressPayload,
  type WhisperDownloadErrorPayload,
  type WhisperDownloadProgressPayload,
} from '../../lib/events';
import {
  BTN_DANGER,
  BTN_LG,
  BTN_SM,
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
  cn,
} from '../../lib/classes';
import type { PrefsData } from './types';
import { VoiceModelGrid } from './VoiceModelGrid';

const DEEPGRAM_MODELS = [
  'nova-2',
  'nova-2-general',
  'nova-2-meeting',
  'nova-2-phonecall',
  'nova-2-finance',
  'nova-2-conversationalai',
  'nova-2-voicemail',
  'nova-2-video',
  'nova-2-medical',
  'nova-2-drivethru',
  'nova-2-automotive',
];

const safeVoiceError = (fallback: string) => fallback;

const formatVoiceBytes = (bytes: number) => {
  if (bytes >= 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  return `${Math.round(bytes / (1024 * 1024))} MB`;
};

export const whisperSourceLabel = (source: WhisperBinarySource | null) => {
  if (source === 'Bundled') return 'Bundled with Marvis';
  if (source) return 'Custom whisper-cli detected';
  return 'Whisper CLI unavailable';
};

/* Enrollment caps the take client-side; the backend still enforces its
 * own minimum length and voiced-speech check. */
const ENROLL_MAX_SECONDS = 12;
const ENROLL_TICK_MS = 500;

/* A short read-aloud prompt per `app.main_language` — the embedding model
 * is language-agnostic, so this only needs to be natural to speak. */
const ENROLL_SENTENCES: Record<string, string> = {
  en: "Good morning everyone — let's quickly run through the launch checklist and see where we stand.",
  zh: '大家好,我们现在开始开会,先简单过一下今天的议程和上周的进展。',
  ja: '皆さん、おはようございます。今日の会議を始めて、まず進捗を確認しましょう。',
  ko: '안녕하세요, 지금부터 회의를 시작하고 지난주 진행 상황부터 확인하겠습니다.',
  fr: "Bonjour à toutes et à tous, commençons la réunion par un point sur l'avancement du projet.",
  es: 'Hola a todos, empecemos la reunión repasando el progreso de la semana pasada.',
};

export interface VoiceSetupProps {
  data: PrefsData;
  showSkip?: boolean;
  onSkip?: () => void;
  onContinue?: () => void;
  onBack?: () => void;
  /** Settings keeps downloads alive across navigation; onboarding can opt in to cancellation. */
  cancelOnUnmount?: boolean;
}

export const VoiceSetup = ({
  data,
  showSkip = false,
  onSkip,
  onContinue,
  onBack,
  cancelOnUnmount = false,
}: VoiceSetupProps) => {
  const [catalog, setCatalog] = useState<VoiceModelCatalogEntry[]>([]);
  const [whisper, setWhisper] = useState<WhisperStatus | null>(null);
  const [sherpa, setSherpa] = useState<SherpaStatus | null>(null);
  const [error, setError] = useState('');
  const [downloadError, setDownloadError] = useState('');
  const [progress, setProgress] = useState<
    Record<string, WhisperDownloadProgressPayload>
  >({});
  const [sherpaProgress, setSherpaProgress] = useState<
    Record<string, SherpaDownloadProgressPayload>
  >({});
  const [deepgramKeyInput, setDeepgramKeyInput] = useState('');
  const [deepgramSaving, setDeepgramSaving] = useState(false);
  const [voiceprint, setVoiceprint] = useState<VoiceprintStatus | null>(null);
  const [recording, setRecording] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [enrollBusy, setEnrollBusy] = useState(false);
  const [enrollNote, setEnrollNote] = useState<{
    text: string;
    error: boolean;
  } | null>(null);
  const whisperRef = useRef<WhisperStatus | null>(null);
  const sherpaRef = useRef<SherpaStatus | null>(null);
  const pendingDownloadRef = useRef<string | null>(null);
  const recordingRef = useRef(false);
  const enrollStartRef = useRef(0);
  whisperRef.current = whisper;
  sherpaRef.current = sherpa;
  recordingRef.current = recording;
  const config = data.config;
  const deepgramKey =
    data.status?.keys.find(([id]) => id === 'deepgram')?.[1] ?? null;

  const refreshWhisper = useCallback(async () => {
    let status: WhisperStatus;
    try {
      status = await whisperStatus();
    } catch {
      status = {
        binary: null,
        binary_status: { available: false, source: null },
        models: [],
        download: null,
      };
    }
    setWhisper(status);
    setProgress((current) => {
      // Status is authoritative. A completed/failed/cancelled task has no
      // download, and an installed model cannot still be downloading.
      if (!status.download) return {};
      const installed = new Set(
        status.models
          .filter((entry) => entry.installed)
          .map((entry) => entry.id),
      );
      const next = { ...current };
      for (const model of installed) delete next[model];
      return next;
    });
  }, []);

  const refreshSherpa = useCallback(async () => {
    let status: SherpaStatus;
    try {
      status = await sherpaStatus();
    } catch {
      status = { models: [], download: null };
    }
    setSherpa(status);
    setSherpaProgress((current) => {
      // Status is authoritative, same as refreshWhisper.
      if (!status.download) return {};
      const installed = new Set(
        status.models
          .filter((entry) => entry.installed)
          .map((entry) => entry.id),
      );
      const next = { ...current };
      for (const model of installed) delete next[model];
      return next;
    });
  }, []);

  useEffect(() => {
    void voiceModelsCatalog()
      .then(setCatalog)
      .catch(() => setError('Could not load voice models'));
    void refreshWhisper();
    void refreshSherpa();
    void voiceprintStatus()
      .then(setVoiceprint)
      .catch(() => {});
  }, [refreshWhisper, refreshSherpa]);

  useTauriEvent<WhisperDownloadProgressPayload>(
    EV_WHISPER_DOWNLOAD_PROGRESS,
    (payload) => {
      if (payload.total > 0 && payload.received >= payload.total) {
        setProgress((current) => {
          const next = { ...current };
          delete next[payload.model];
          return next;
        });
        void refreshWhisper();
      } else {
        setProgress((current) => ({ ...current, [payload.model]: payload }));
        setWhisper((current) =>
          current ? { ...current, download: payload } : current,
        );
      }
    },
  );
  useTauriEvent<WhisperDownloadErrorPayload>(
    EV_WHISPER_DOWNLOAD_ERROR,
    (payload) => {
      setDownloadError(payload.message);
      setProgress((current) => {
        const next = { ...current };
        delete next[payload.model];
        return next;
      });
      void refreshWhisper();
    },
  );
  useTauriEvent<SherpaDownloadProgressPayload>(
    EV_SHERPA_DOWNLOAD_PROGRESS,
    (payload) => {
      if (payload.total > 0 && payload.received >= payload.total) {
        setSherpaProgress((current) => {
          const next = { ...current };
          delete next[payload.model];
          return next;
        });
        void refreshSherpa();
      } else {
        setSherpaProgress((current) => ({
          ...current,
          [payload.model]: payload,
        }));
        setSherpa((current) =>
          current ? { ...current, download: payload } : current,
        );
      }
    },
  );
  useTauriEvent<SherpaDownloadErrorPayload>(
    EV_SHERPA_DOWNLOAD_ERROR,
    (payload) => {
      setDownloadError(payload.message);
      setSherpaProgress((current) => {
        const next = { ...current };
        delete next[payload.model];
        return next;
      });
      void refreshSherpa();
    },
  );

  const stopEnrollment = useCallback(async () => {
    setEnrollBusy(true);
    try {
      const result = await voiceEnrollStop();
      setVoiceprint(await voiceprintStatus());
      setEnrollNote({
        text: `Voiceprint saved · ${Math.round(result.seconds)}s of audio`,
        error: false,
      });
    } catch (e) {
      setEnrollNote({
        text: e instanceof Error ? e.message : String(e),
        error: true,
      });
    } finally {
      setEnrollBusy(false);
      setRecording(false);
    }
  }, []);

  const startEnrollment = async () => {
    setEnrollNote(null);
    try {
      await voiceEnrollStart();
      setRecording(true);
      setElapsed(0);
      enrollStartRef.current = Date.now();
    } catch (e) {
      setEnrollNote({
        text: e instanceof Error ? e.message : String(e),
        error: true,
      });
    }
  };

  const cancelEnrollment = async () => {
    try {
      await voiceEnrollCancel();
    } catch {
      // best-effort — the take is discarded either way
    }
    setRecording(false);
  };

  const removeVoiceprint = async () => {
    setEnrollNote(null);
    try {
      await voiceprintRemove();
      setVoiceprint(await voiceprintStatus());
      setEnrollNote({ text: 'Voiceprint removed', error: false });
    } catch (e) {
      setEnrollNote({
        text: e instanceof Error ? e.message : String(e),
        error: true,
      });
    }
  };

  /* Elapsed timer + the client-side take cap while recording. */
  useEffect(() => {
    if (!recording) return;
    const tick = setInterval(() => {
      const seconds = Math.floor((Date.now() - enrollStartRef.current) / 1000);
      setElapsed(seconds);
      if (seconds >= ENROLL_MAX_SECONDS) void stopEnrollment();
    }, ENROLL_TICK_MS);
    return () => clearInterval(tick);
  }, [recording, stopEnrollment]);

  /* Navigating away mid-take must release the microphone. */
  useEffect(
    () => () => {
      if (recordingRef.current) void voiceEnrollCancel().catch(() => {});
    },
    [],
  );

  useEffect(() => {
    if (!cancelOnUnmount) return;
    return () => {
      const hasPendingDownloadRequest = pendingDownloadRef.current !== null;
      // Clear the local marker before awaiting cancellation so a request that
      // rejects because cancellation won the race cannot surface a failure.
      pendingDownloadRef.current = null;
      const hasActiveDownload = Boolean(whisperRef.current?.download);
      if (hasActiveDownload || hasPendingDownloadRequest) {
        void whisperCancelDownload().catch(() => {});
      }
      const hasActiveSherpaDownload = Boolean(sherpaRef.current?.download);
      if (hasActiveSherpaDownload || hasPendingDownloadRequest) {
        void sherpaCancelDownload().catch(() => {});
      }
    };
  }, [cancelOnUnmount]);

  if (!config)
    return <p className='text-[12.5px] text-muted-foreground'>Loading…</p>;

  const provider = config.models.stt_provider;
  const model = config.models.stt_model;
  const installedModels =
    whisper?.models.filter((entry) => entry.installed) ?? [];
  const activeDownload = whisper?.download;
  const sherpaActiveDownload = sherpa?.download;
  // Only `stt`-kind sherpa entries are selectable transcription models; the
  // speaker-embedding entry drives diarization for both local providers.
  const sherpaSttModels =
    sherpa?.models.filter((entry) => entry.kind === 'stt') ?? [];
  const speakerModel = sherpa?.models.find(
    (entry) => entry.kind === 'speaker-embedding',
  );
  const speakerProgress =
    speakerModel &&
    (sherpaActiveDownload?.model === speakerModel.id
      ? sherpaActiveDownload
      : (sherpaProgress[speakerModel.id] ?? null));
  const punctModel = sherpa?.models.find(
    (entry) => entry.kind === 'punctuation',
  );
  const punctProgress =
    punctModel &&
    (sherpaActiveDownload?.model === punctModel.id
      ? sherpaActiveDownload
      : (sherpaProgress[punctModel.id] ?? null));
  const save = (
    key: 'models.stt_provider' | 'models.stt_model',
    value: string,
  ) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    setError('');
    void configSet(key, trimmed)
      .then(data.setConfig)
      .catch(() =>
        setError(
          `Could not save ${key.endsWith('provider') ? 'provider' : 'model'}`,
        ),
      );
  };
  const saveDeepgramKey = async () => {
    const key = deepgramKeyInput.trim();
    if (!key || deepgramSaving) return;
    setDeepgramSaving(true);
    setError('');
    try {
      data.setStatus(await keystoreSetKey('deepgram', key));
      setDeepgramKeyInput('');
    } catch (e) {
      setError(safeVoiceError('Could not save Deepgram API key'));
    } finally {
      setDeepgramSaving(false);
    }
  };
  const removeDeepgramKey = async () => {
    if (deepgramSaving) return;
    setDeepgramSaving(true);
    setError('');
    try {
      data.setStatus(await keystoreRemoveKey('deepgram'));
    } catch (e) {
      setError(safeVoiceError('Could not remove Deepgram API key'));
    } finally {
      setDeepgramSaving(false);
    }
  };
  const startDownload = async (id: string) => {
    if (activeDownload || pendingDownloadRef.current !== null) return;
    // Mark the request before invoking Tauri. Onboarding can unmount before
    // whisperDownload resolves and must still cancel this not-yet-authoritative
    // request.
    pendingDownloadRef.current = id;
    setDownloadError('');
    setError('');
    try {
      await whisperDownload(id);
      await refreshWhisper();
    } catch (e) {
      if (pendingDownloadRef.current === id) {
        setDownloadError(safeVoiceError('Could not start model download'));
        await refreshWhisper();
      }
    } finally {
      if (pendingDownloadRef.current === id) pendingDownloadRef.current = null;
    }
  };
  const cancelDownload = async () => {
    const canceledModel = activeDownload?.model;
    try {
      await whisperCancelDownload();
      setProgress((current) => {
        if (!canceledModel || !(canceledModel in current)) return current;
        const next = { ...current };
        delete next[canceledModel];
        return next;
      });
      await refreshWhisper();
    } catch (e) {
      setDownloadError(safeVoiceError('Could not cancel download'));
    }
  };
  const removeModel = async (id: string) => {
    setError('');
    try {
      setWhisper(await whisperRemoveModel(id));
    } catch (e) {
      setError(safeVoiceError('Could not remove model'));
    }
  };
  const startSherpaDownload = async (id: string) => {
    if (sherpaActiveDownload || pendingDownloadRef.current !== null) return;
    // Same pending marker as whisperDownload — onboarding can unmount before
    // the invoke resolves and must still cancel this request.
    pendingDownloadRef.current = id;
    setDownloadError('');
    setError('');
    try {
      await sherpaDownload(id);
      await refreshSherpa();
    } catch (e) {
      if (pendingDownloadRef.current === id) {
        setDownloadError(safeVoiceError('Could not start model download'));
        await refreshSherpa();
      }
    } finally {
      if (pendingDownloadRef.current === id) pendingDownloadRef.current = null;
    }
  };
  const cancelSherpaDownload = async () => {
    const canceledModel = sherpaActiveDownload?.model;
    try {
      await sherpaCancelDownload();
      setSherpaProgress((current) => {
        if (!canceledModel || !(canceledModel in current)) return current;
        const next = { ...current };
        delete next[canceledModel];
        return next;
      });
      await refreshSherpa();
    } catch (e) {
      setDownloadError(safeVoiceError('Could not cancel download'));
    }
  };
  const removeSherpaModel = async (id: string) => {
    setError('');
    try {
      setSherpa(await sherpaRemoveModel(id));
    } catch (e) {
      setError(safeVoiceError('Could not remove model'));
    }
  };

  return (
    <div>
      <div className={cn(PROV_CARD, 'border-border')}>
        <div className='flex flex-wrap items-center gap-2'>
          <span
            className={cn(
              'size-1.75 flex-none rounded-full',
              provider === 'deepgram'
                ? deepgramKey
                  ? 'bg-accent'
                  : 'bg-[color-mix(in_oklch,var(--fg)_20%,transparent)]'
                : provider === 'whisper'
                  ? whisper?.binary && installedModels.length > 0
                    ? 'bg-accent'
                    : 'bg-[color-mix(in_oklch,var(--fg)_20%,transparent)]'
                  : sherpa?.models.some(
                        (entry) => entry.kind === 'stt' && entry.installed,
                      )
                    ? 'bg-accent'
                    : 'bg-[color-mix(in_oklch,var(--fg)_20%,transparent)]',
            )}
          />
          <span className={LBL}>Provider</span>
          <select
            className={MODEL_SEL}
            value={provider}
            onChange={(e) => {
              save('models.stt_provider', e.target.value);
              if (e.target.value === 'whisper') void refreshWhisper();
              if (e.target.value === 'sherpa') void refreshSherpa();
            }}
            aria-label='Speech-to-text provider'>
            <option value='deepgram'>Deepgram</option>
            <option value='whisper'>Whisper (local)</option>
            <option value='sherpa'>Sherpa (local)</option>
          </select>
        </div>
        {provider === 'deepgram' ? (
          <>
            <div className='mt-2.5 flex items-center gap-2'>
              <span className={LBL}>Model</span>
              <Input
                className={FIELD}
                list='deepgram-stt-models'
                defaultValue={model}
                onBlur={(e) => save('models.stt_model', e.target.value)}
                onKeyDown={(e) =>
                  e.key === 'Enter' &&
                  save('models.stt_model', e.currentTarget.value)
                }
                placeholder='model id — e.g. nova-2'
                autoComplete='off'
                spellCheck={false}
                aria-label='Deepgram speech-to-text model'
              />
              <datalist id='deepgram-stt-models'>
                {DEEPGRAM_MODELS.map((name) => (
                  <option
                    key={name}
                    value={name}
                  />
                ))}
              </datalist>
            </div>
            <div className='mt-2.5 flex items-center gap-1.5'>
              <Input
                className={FIELD}
                type='password'
                value={deepgramKeyInput}
                onChange={(e) => setDeepgramKeyInput(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && void saveDeepgramKey()}
                placeholder={
                  deepgramKey ? 'Replace Deepgram API key' : 'Deepgram API key'
                }
                autoComplete='off'
                aria-label='Deepgram API key'
              />
              <button
                type='button'
                className={cn(BTN_SM, BTN_OUTLINE)}
                disabled={deepgramSaving || !deepgramKeyInput.trim()}
                onClick={() => void saveDeepgramKey()}>
                {deepgramSaving
                  ? 'Saving…'
                  : deepgramKey
                    ? 'Replace'
                    : 'Save key'}
              </button>
            </div>
            <p className={PROV_NOTE}>
              {deepgramKey ? (
                <>
                  Stored after non-empty shape validation · {deepgramKey} ·
                  masked only.{' '}
                  <button
                    type='button'
                    className='underline underline-offset-2'
                    disabled={deepgramSaving}
                    onClick={() => void removeDeepgramKey()}>
                    Remove key
                  </button>
                </>
              ) : (
                'Enter a Deepgram API key to use hosted transcription. It is never stored in config.toml.'
              )}
            </p>
          </>
        ) : provider === 'whisper' ? (
          <>
            <p className={PROV_NOTE}>
              Choose a local Whisper model below. The app downloads models only,
              never the whisper-cli binary.
            </p>
            <VoiceModelGrid
              entries={catalog}
              isInstalled={(id) => {
                const entry = catalog.find((item) => item.id === id);
                return (
                  whisper?.models.find(
                    (item) =>
                      item.id === id || item.filename === entry?.filename,
                  )?.installed ?? false
                );
              }}
              isSelected={(id) => {
                const entry = catalog.find((item) => item.id === id);
                return (
                  provider === 'whisper' &&
                  (model === id || model === entry?.filename)
                );
              }}
              progress={
                activeDownload
                  ? { [activeDownload.model]: activeDownload, ...progress }
                  : progress
              }
              activeDownload={activeDownload?.model ?? null}
              onDownload={(id) => void startDownload(id)}
              onCancel={() => void cancelDownload()}
              onSelect={(id) => save('models.stt_model', id)}
              onRemove={(id) => void removeModel(id)}
            />
            {whisper && (
              <p className={PROV_NOTE}>
                {whisperSourceLabel(whisper.binary_status.source)}
              </p>
            )}
            {whisper && !whisper.binary_status.source && (
              <p className={PROV_NOTE}>
                For local transcription during development, install a compatible{' '}
                <span className={NUM}>whisper-cli</span> and restart the app.
              </p>
            )}
          </>
        ) : (
          <>
            <p className={PROV_NOTE}>
              Download the SenseVoice model below — it runs fully on-device; no
              separate binary is needed.
            </p>
            <VoiceModelGrid
              entries={sherpaSttModels}
              isInstalled={(id) =>
                sherpa?.models.find((item) => item.id === id)?.installed ??
                false
              }
              isSelected={(id) => provider === 'sherpa' && model === id}
              progress={
                sherpaActiveDownload
                  ? {
                      [sherpaActiveDownload.model]: sherpaActiveDownload,
                      ...sherpaProgress,
                    }
                  : sherpaProgress
              }
              activeDownload={sherpaActiveDownload?.model ?? null}
              onDownload={(id) => void startSherpaDownload(id)}
              onCancel={() => void cancelSherpaDownload()}
              onSelect={(id) => save('models.stt_model', id)}
              onRemove={(id) => void removeSherpaModel(id)}
            />
          </>
        )}
        {(error || downloadError) && (
          <p className={PROV_ERR}>{error || downloadError}</p>
        )}
      </div>
      {provider === 'sherpa' && punctModel && (
        <div className={cn(PROV_CARD, 'border-border')}>
          <div className='flex items-center gap-2'>
            <span
              className={cn(
                'size-1.75 flex-none rounded-full',
                punctModel.installed
                  ? 'bg-accent'
                  : 'bg-[color-mix(in_oklch,var(--fg)_20%,transparent)]',
              )}
            />
            <span className={LBL}>Punctuation & casing</span>
            <span
              className={cn(
                NUM,
                'ml-auto text-[10.5px] text-muted-foreground',
              )}>
              {formatVoiceBytes(punctModel.bytes)}
            </span>
          </div>
          <p className={PROV_NOTE}>
            {punctModel.description} Optional — without it, English speech is
            transcribed in ALL CAPS with no punctuation.
          </p>
          {punctProgress && (
            <div className='mt-2'>
              <div className='flex justify-between text-[10px] text-muted-foreground'>
                <span>Downloading…</span>
                <span className={NUM}>
                  {formatVoiceBytes(punctProgress.received)} /{' '}
                  {formatVoiceBytes(punctProgress.total)}
                </span>
              </div>
              <progress
                className='mt-1 h-1.5 w-full accent-accent'
                value={punctProgress.received}
                max={punctProgress.total}
              />
            </div>
          )}
          <div className='mt-2 flex gap-1.5'>
            {sherpaActiveDownload?.model === punctModel.id ? (
              <button
                type='button'
                className={cn(BTN_LG, BTN_OUTLINE)}
                onClick={() => void cancelSherpaDownload()}>
                Cancel
              </button>
            ) : !punctModel.installed ? (
              <button
                type='button'
                className={cn(BTN_LG, BTN_PRIMARY)}
                disabled={Boolean(sherpaActiveDownload)}
                onClick={() => void startSherpaDownload(punctModel.id)}>
                Download
              </button>
            ) : (
              <button
                type='button'
                className={cn(BTN_LINK_LG, BTN_DANGER)}
                onClick={() => void removeSherpaModel(punctModel.id)}>
                Remove
              </button>
            )}
          </div>
        </div>
      )}
      {provider !== 'deepgram' && speakerModel && (
        <div className={cn(PROV_CARD, 'border-border')}>
          <div className='flex items-center gap-2'>
            <span
              className={cn(
                'size-1.75 flex-none rounded-full',
                speakerModel.installed
                  ? 'bg-accent'
                  : 'bg-[color-mix(in_oklch,var(--fg)_20%,transparent)]',
              )}
            />
            <span className={LBL}>Speaker diarization</span>
            <span
              className={cn(
                NUM,
                'ml-auto text-[10.5px] text-muted-foreground',
              )}>
              {formatVoiceBytes(speakerModel.bytes)}
            </span>
          </div>
          <p className={PROV_NOTE}>
            {speakerModel.description} Optional — shared by Whisper and Sherpa;
            the transcript works without it, just without per-voice labels.
          </p>
          {speakerProgress && (
            <div className='mt-2'>
              <div className='flex justify-between text-[10px] text-muted-foreground'>
                <span>Downloading…</span>
                <span className={NUM}>
                  {formatVoiceBytes(speakerProgress.received)} /{' '}
                  {formatVoiceBytes(speakerProgress.total)}
                </span>
              </div>
              <progress
                className='mt-1 h-1.5 w-full accent-accent'
                value={speakerProgress.received}
                max={speakerProgress.total}
              />
            </div>
          )}
          <div className='mt-2 flex gap-1.5'>
            {sherpaActiveDownload?.model === speakerModel.id ? (
              <button
                type='button'
                className={cn(BTN_LG, BTN_OUTLINE)}
                onClick={() => void cancelSherpaDownload()}>
                Cancel
              </button>
            ) : !speakerModel.installed ? (
              <button
                type='button'
                className={cn(BTN_LG, BTN_PRIMARY)}
                disabled={Boolean(sherpaActiveDownload)}
                onClick={() => void startSherpaDownload(speakerModel.id)}>
                Download
              </button>
            ) : (
              <button
                type='button'
                className={cn(BTN_LINK_LG, BTN_DANGER)}
                onClick={() => void removeSherpaModel(speakerModel.id)}>
                Remove
              </button>
            )}
          </div>
        </div>
      )}
      {provider !== 'deepgram' && speakerModel && (
        <div className={cn(PROV_CARD, 'border-border')}>
          <div className='flex items-center gap-2'>
            <span
              className={cn(
                'size-1.75 flex-none rounded-full',
                voiceprint?.enrolled
                  ? 'bg-accent'
                  : 'bg-[color-mix(in_oklch,var(--fg)_20%,transparent)]',
              )}
            />
            <span className={LBL}>Your voice</span>
            {recording && (
              <span
                className={cn(
                  NUM,
                  'ml-auto flex items-center gap-1.5 text-[10.5px] text-muted-foreground',
                )}>
                <span className='size-1.5 animate-pulse rounded-full bg-red-500' />
                {elapsed}s
              </span>
            )}
          </div>
          <p className={PROV_NOTE}>
            Record a short sample so Marvis recognizes your voice — your turns
            always label as You, and someone else on your mic becomes a guest.
            Applies to new sessions.
          </p>
          {recording && (
            <p className='mt-2 text-[12.5px] leading-[1.6] text-foreground'>
              “
              {ENROLL_SENTENCES[config.app.main_language] ??
                ENROLL_SENTENCES.en}
              ”
            </p>
          )}
          {enrollNote && (
            <p className={enrollNote.error ? PROV_ERR : PROV_NOTE}>
              {enrollNote.text}
            </p>
          )}
          <div className='mt-2 flex items-center gap-1.5'>
            {!speakerModel.installed ? (
              <p className={PROV_NOTE}>
                Download the speaker diarization model above first.
              </p>
            ) : recording ? (
              <>
                <button
                  type='button'
                  className={cn(BTN_LG, BTN_PRIMARY)}
                  disabled={enrollBusy}
                  onClick={() => void stopEnrollment()}>
                  {enrollBusy ? 'Saving…' : 'Stop & save'}
                </button>
                <button
                  type='button'
                  className={cn(BTN_LG, BTN_OUTLINE)}
                  disabled={enrollBusy}
                  onClick={() => void cancelEnrollment()}>
                  Cancel
                </button>
              </>
            ) : voiceprint?.enrolled ? (
              <>
                <button
                  type='button'
                  className={cn(BTN_LG, BTN_OUTLINE)}
                  onClick={() => void startEnrollment()}>
                  Re-record
                </button>
                <button
                  type='button'
                  className={cn(BTN_LINK_LG, BTN_DANGER)}
                  onClick={() => void removeVoiceprint()}>
                  Remove
                </button>
              </>
            ) : (
              <button
                type='button'
                className={cn(BTN_LG, BTN_PRIMARY)}
                onClick={() => void startEnrollment()}>
                Record
              </button>
            )}
          </div>
        </div>
      )}
      {showSkip && (
        <div className='mt-3 flex justify-end gap-2'>
          <button
            type='button'
            className={cn(BTN_LG, BTN_OUTLINE)}
            onClick={onBack}>
            Back
          </button>
          <button
            type='button'
            className={cn(BTN_LINK_LG, BTN_OUTLINE)}
            onClick={onSkip}>
            Skip for now
          </button>
          <button
            type='button'
            className={cn(BTN_LG, BTN_PRIMARY)}
            onClick={onContinue}>
            Continue
          </button>
        </div>
      )}
    </div>
  );
};
