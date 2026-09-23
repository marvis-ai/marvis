import { useCallback, useEffect, useRef, useState } from 'react';
import { Input } from '@marvis/ui';
import {
  configSet,
  keystoreRemoveKey,
  keystoreSetKey,
  voiceModelsCatalog,
  whisperCancelDownload,
  whisperDownload,
  whisperRemoveModel,
  whisperStatus,
  type VoiceModelCatalogEntry,
  type WhisperStatus,
} from '../../lib/commands';
import {
  EV_WHISPER_DOWNLOAD_ERROR,
  EV_WHISPER_DOWNLOAD_PROGRESS,
  useTauriEvent,
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

const formatBytes = (bytes: number) => {
  if (bytes >= 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  return `${Math.round(bytes / (1024 * 1024))} MB`;
};

export interface VoiceSetupProps {
  data: PrefsData;
  showSkip?: boolean;
  onSkip?: () => void;
  onContinue?: () => void;
  /** Settings keeps downloads alive across navigation; onboarding can opt in to cancellation. */
  cancelOnUnmount?: boolean;
}

export const VoiceSetup = ({
  data,
  showSkip = false,
  onSkip,
  onContinue,
  cancelOnUnmount = false,
}: VoiceSetupProps) => {
  const [catalog, setCatalog] = useState<VoiceModelCatalogEntry[]>([]);
  const [whisper, setWhisper] = useState<WhisperStatus | null>(null);
  const [error, setError] = useState('');
  const [downloadError, setDownloadError] = useState('');
  const [progress, setProgress] = useState<
    Record<string, WhisperDownloadProgressPayload>
  >({});
  const [deepgramKeyInput, setDeepgramKeyInput] = useState('');
  const [deepgramSaving, setDeepgramSaving] = useState(false);
  const whisperRef = useRef<WhisperStatus | null>(null);
  whisperRef.current = whisper;
  const config = data.config;
  const deepgramKey =
    data.status?.keys.find(([id]) => id === 'deepgram')?.[1] ?? null;

  const refreshWhisper = useCallback(async () => {
    try {
      setWhisper(await whisperStatus());
    } catch {
      setWhisper({ binary: null, models: [], download: null });
    }
  }, []);

  useEffect(() => {
    void voiceModelsCatalog()
      .then(setCatalog)
      .catch(() => setError('Could not load voice models'));
    void refreshWhisper();
  }, [refreshWhisper]);

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

  useEffect(() => {
    if (!cancelOnUnmount) return;
    return () => {
      if (whisperRef.current?.download)
        void whisperCancelDownload().catch(() => {});
    };
  }, [cancelOnUnmount]);

  if (!config)
    return <p className='text-[12.5px] text-muted-foreground'>Loading…</p>;

  const provider = config.models.stt_provider;
  const model = config.models.stt_model;
  const installedModels =
    whisper?.models.filter((entry) => entry.installed) ?? [];
  const activeDownload = whisper?.download;
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
      setError(typeof e === 'string' ? e : 'Could not save Deepgram API key');
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
      setError(typeof e === 'string' ? e : 'Could not remove Deepgram API key');
    } finally {
      setDeepgramSaving(false);
    }
  };
  const startDownload = async (id: string) => {
    if (activeDownload) return;
    setDownloadError('');
    setError('');
    try {
      await whisperDownload(id);
      await refreshWhisper();
    } catch (e) {
      setDownloadError(
        typeof e === 'string' ? e : 'Could not start model download',
      );
      await refreshWhisper();
    }
  };
  const cancelDownload = async () => {
    try {
      await whisperCancelDownload();
      await refreshWhisper();
    } catch (e) {
      setDownloadError(typeof e === 'string' ? e : 'Could not cancel download');
    }
  };
  const removeModel = async (id: string) => {
    setError('');
    try {
      setWhisper(await whisperRemoveModel(id));
    } catch (e) {
      setError(typeof e === 'string' ? e : 'Could not remove model');
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
                : whisper?.binary && installedModels.length > 0
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
            }}
            aria-label='Speech-to-text provider'>
            <option value='deepgram'>Deepgram</option>
            <option value='whisper'>Whisper (local)</option>
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
        ) : (
          <>
            <p className={PROV_NOTE}>
              Choose a local Whisper model below. The app downloads models only,
              never the whisper-cli binary.
            </p>
            <div className='mt-2.5 grid gap-2'>
              {catalog.map((entry) => {
                const installed =
                  whisper?.models.find(
                    (item) =>
                      item.id === entry.id || item.filename === entry.filename,
                  )?.installed ?? false;
                const selected =
                  provider === 'whisper' &&
                  (model === entry.id || model === entry.filename);
                const currentProgress =
                  progress[entry.id] ??
                  (activeDownload?.model === entry.id ? activeDownload : null);
                const downloading = activeDownload?.model === entry.id;
                return (
                  <div
                    key={entry.id}
                    className={cn(
                      'rounded-lg border p-2.5',
                      selected ? 'border-accent' : 'border-border',
                    )}>
                    <div className='flex items-start justify-between gap-2'>
                      <div>
                        <div className='text-[12.5px] font-semibold'>
                          {entry.label}
                          {selected && (
                            <span className='ml-1.5 text-[10px] text-accent-text'>
                              Selected
                            </span>
                          )}
                        </div>
                        <p className={PROV_NOTE}>{entry.description}</p>
                      </div>
                      <span
                        className={cn(
                          NUM,
                          'text-[10.5px] text-muted-foreground',
                        )}>
                        {formatBytes(entry.bytes)}
                      </span>
                    </div>
                    <p className={PROV_NOTE}>
                      Source: {entry.source} ·{' '}
                      {installed ? 'Installed' : 'Not installed'}
                    </p>
                    {currentProgress && (
                      <div className='mt-2'>
                        <div className='flex justify-between text-[10px] text-muted-foreground'>
                          <span>Downloading…</span>
                          <span className={NUM}>
                            {formatBytes(currentProgress.received)} /{' '}
                            {formatBytes(currentProgress.total)}
                          </span>
                        </div>
                        <progress
                          className='mt-1 h-1.5 w-full accent-accent'
                          value={currentProgress.received}
                          max={currentProgress.total}
                        />
                      </div>
                    )}
                    <div className='mt-2 flex flex-wrap gap-1.5'>
                      {downloading ? (
                        <button
                          type='button'
                          className={cn(BTN_LG, BTN_OUTLINE)}
                          onClick={() => void cancelDownload()}>
                          Cancel
                        </button>
                      ) : !installed ? (
                        <button
                          type='button'
                          className={cn(BTN_LG, BTN_PRIMARY)}
                          disabled={Boolean(activeDownload)}
                          onClick={() => void startDownload(entry.id)}>
                          Download
                        </button>
                      ) : (
                        <>
                          <button
                            type='button'
                            className={cn(BTN_LG, BTN_PRIMARY)}
                            disabled={selected}
                            onClick={() => save('models.stt_model', entry.id)}>
                            {selected ? 'Using this model' : 'Use this model'}
                          </button>
                          <button
                            type='button'
                            className={cn(BTN_LINK_LG, BTN_DANGER)}
                            disabled={selected}
                            title={
                              selected
                                ? 'The active model cannot be removed'
                                : undefined
                            }
                            onClick={() => void removeModel(entry.id)}>
                            Remove
                          </button>
                        </>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
            {(!whisper?.binary || installedModels.length === 0) && (
              <p className={PROV_NOTE}>
                You must install <span className={NUM}>whisper-cli</span>{' '}
                separately to use local transcription. Marvis does not download
                binaries; install a compatible binary and restart the app.
              </p>
            )}
            {whisper?.binary && (
              <p className={PROV_NOTE}>whisper-cli detected.</p>
            )}
          </>
        )}
        {(error || downloadError) && (
          <p className={PROV_ERR}>{error || downloadError}</p>
        )}
      </div>
      {showSkip && (
        <div className='mt-3 flex justify-end gap-2'>
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
