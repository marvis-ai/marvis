import { Fragment, type ReactNode } from 'react';
import { PreLines } from './pre-lines';
import { leadTop, sectionStack } from './styles';

/* .filetree is white-space: pre — the 14-space source indent is content,
 * emitted by PreLines (same constraint as the hero editor). */
const FILE_TREE: ReactNode[] = [
  '~/.marvis/',
  <Fragment key='keys'>
    ├── keys.json{' '}
    <span className='dim'>
      0600 · provider keys, masked as …last4 in the UI
    </span>
  </Fragment>,
  <Fragment key='config'>
    ├── config.toml{' '}
    <span className='dim'>0644 · models, hotkeys, window position</span>
  </Fragment>,
  <Fragment key='db'>
    ├── marvis.db <span className='dim'>0600 · SQLite sessions & messages</span>
  </Fragment>,
  <Fragment key='models'>
    └── models/ <span className='dim'>local model files (planned)</span>
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
        <p className='eyebrow'>Local-first architecture</p>
        <h2>What&rsquo;s on your machine stays on your machine.</h2>
        <p
          className='lead'
          style={leadTop}>
          No accounts, no sync service, no Marvis servers. The only thing that
          ever leaves your device is the request you send — your prompt plus one
          screen frame — straight to the LLM provider you chose.
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
              accounts, cloud syncs, or Marvis servers — there is nothing to
              phone home to.
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
            data-od-id='stat-three'>
            <div className='stat-num num'>3</div>
            <p className='stat-label'>
              files hold everything Marvis writes — keys, config, and history —
              all under <span className='num'>~/.marvis</span>.
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
