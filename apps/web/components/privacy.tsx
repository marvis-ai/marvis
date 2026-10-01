import { Fragment, type ReactNode } from 'react';
import { PreLines } from './pre-lines';
import { leadTop, sectionStack } from './styles';

/* .filetree is white-space: pre — the 14-space source indent is content,
 * emitted by PreLines, which renders literal newlines + indent inside the
 * div.filetree host. */
const FILE_TREE: ReactNode[] = [
  '~/.marvis/',
  <Fragment key='keys'>
    ├── keys.json{' '}
    <span className='dim'>
      0600 · BYOK — provider keys masked as …last4 in the UI
    </span>
  </Fragment>,
  <Fragment key='config'>
    ├── config.toml{' '}
    <span className='dim'>
      0644 · providers, models, hotkeys, window position
    </span>
  </Fragment>,
  <Fragment key='db'>
    ├── marvis.db{' '}
    <span className='dim'>
      0600 · SQLite — sessions, messages, transcripts, summaries
    </span>
  </Fragment>,
  <Fragment key='voiceprint'>
    ├── voiceprint.bin{' '}
    <span className='dim'>
      enrolled speaker embedding — pins mic&rsquo;s speaker 0 to you
    </span>
  </Fragment>,
  <Fragment key='audios'>
    ├── audios/{' '}
    <span className='dim'>
      recording_*.wav at 0600 — retained session recordings
    </span>
  </Fragment>,
  <Fragment key='models'>
    ├── models/{' '}
    <span className='dim'>
      on-device whisper + sherpa ASR/STT models, on demand
    </span>
  </Fragment>,
  <Fragment key='tmp'>
    └── tmp/ <span className='dim'>scratch audio workspace</span>
  </Fragment>,
];

export const Privacy = () => (
  <section
    className='section section-dark'
    id='privacy'
    data-od-id='privacy'>
    <div
      className='container stack'
      style={sectionStack}>
      <div style={{ maxWidth: '46ch' }}>
        <p className='eyebrow'>BYOK · on-device speech</p>
        <h2>Your keys, your voice, your machine.</h2>
        <p
          className='lead'
          style={leadTop}>
          No accounts, no sync service, no Marvis servers. Marvis is BYOK — your
          provider keys live in keys.json and go only to the LLM you choose.
          Speech is local by default: Listen and dictation transcribe on-device
          with whisper.cpp or sherpa ASR models under models/, with hosted
          Deepgram strictly opt-in. An ask sends your prompt plus one screen
          frame to your LLM; every five turns the rolling TLDR sends transcript
          text to the same provider. Nothing else leaves.
        </p>
      </div>
      <div
        className='grid-2'
        style={{ alignItems: 'start' }}>
        <div
          className='card min-w-0'
          data-od-id='marvis-dir-card'>
          <p
            className='meta'
            style={{
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
            }}>
            Everything Marvis writes — one folder you own
          </p>
          <div className='filetree'>
            <PreLines
              lines={FILE_TREE}
              indent='              '
            />
          </div>
          <p
            className='meta'
            style={{ marginTop: '16px' }}>
            The folder itself is <span className='num'>0700</span> — delete{' '}
            <span className='num'>~/.marvis</span> and every trace of Marvis is
            gone.
          </p>
        </div>
        <div
          className='stack min-w-0'
          style={{ gap: '28px' }}>
          <div
            className='stat'
            data-od-id='stat-zero'>
            <div className='stat-num num'>0</div>
            <p className='stat-label'>
              accounts, cloud syncs, or Marvis servers — keys are never proxied;
              there is nothing to phone home to.
            </p>
          </div>
          <div
            className='stat'
            data-od-id='stat-sixty'>
            <div className='stat-num num'>
              60<span className='stat-unit'>s</span>
            </div>
            <p className='stat-label'>
              of rolling screen context kept in memory — ≤120 frames, ≤64 MB,
              never on disk.
            </p>
          </div>
          <div
            className='stat'
            data-od-id='stat-one'>
            <div className='stat-num num'>1</div>
            <p className='stat-label'>
              folder holds everything Marvis writes — keys, config, history,
              recordings, models — all under{' '}
              <span className='num'>~/.marvis</span>.
            </p>
          </div>
          <p
            className='meta'
            style={{ margin: 0 }}>
            Open source under MIT — every claim on this page maps to a file in
            the repo.
          </p>
        </div>
      </div>
    </div>
  </section>
);
