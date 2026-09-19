import { Fragment, type ReactNode } from 'react';
import { MvAskPanel, MvBar } from './mv';
import { PreLines } from './pre-lines';

/* The mock editor's code pane relies on white-space: pre-wrap, so the
 * 16-space source indent is content — PreLines emits it explicitly. */
const EDITOR_LINES: ReactNode[] = [
  <span
    key='l1'
    className='c-meta'>
    {'// ring buffer — screen frames live in memory only'}
  </span>,
  <Fragment key='l2'>
    <span className='c-mut'>const</span> HORIZON: Duration =
    Duration::from_secs(<span className='num'>60</span>);
  </Fragment>,
  <Fragment key='l3'>
    <span className='c-mut'>const</span> MAX_FRAMES: usize ={' '}
    <span className='num'>120</span>;{' '}
    <span className='c-meta'>{'// ≤ 64 MB cap'}</span>
  </Fragment>,
  null,
  <Fragment key='l5'>
    <span className='c-mut'>fn</span> on_ask(&self, prompt: String) {'{'}
  </Fragment>,
  <Fragment key='l6'>
    <span className='c-mut'>let</span> frame = self.ring.latest();
  </Fragment>,
  <Fragment key='l7'>
    self.llm.stream(prompt, frame);{' '}
    <span className='c-meta'>{'// never hits disk'}</span>
  </Fragment>,
  <Fragment key='l8'>{'}'}</Fragment>,
  null,
  <Fragment key='l10'>
    <span className='c-mut'>fn</span> lock(&mut self) {'{'}
  </Fragment>,
  <Fragment key='l11'>
    self.capture.stop();{' '}
    <span className='c-meta'>{'// tear down capture'}</span>
  </Fragment>,
  <Fragment key='l12'>
    self.panels.hide_all();{' '}
    <span className='c-meta'>{'// hide every panel'}</span>
  </Fragment>,
  <Fragment key='l13'>
    self.requests.abort_all();{' '}
    <span className='c-meta'>{'// cancel in-flight asks'}</span>
  </Fragment>,
  <Fragment key='l14'>{'}'}</Fragment>,
];

export const Hero = () => (
  <section
    className='section hero'
    id='top'
    data-od-id='hero'>
    <div className='container hero-center'>
      <p className='eyebrow'>Private / Personal AI for All</p>
      <h1 className='hero-h1'>
        Ask anything about what&rsquo;s on your screen.
      </h1>
      <p className='lead'>
        Marvis is a small translucent bar that floats above your workspace. It
        sees your screen — only with your permission — and streams answers into
        an overlay panel, while your keys, history, and screen data never leave
        your machine.
      </p>
      <div className='hero-cta'>
        <a
          className='btn btn-primary'
          href='#download'
          data-od-id='hero-cta-primary'>
          Download for macOS
        </a>
        <a
          className='btn btn-ghost btn-arrow'
          href='#privacy'
          data-od-id='hero-cta-secondary'>
          How it stays private
        </a>
      </div>
      <div className='hero-meta'>
        <span className='tag'>Free & open source · MIT</span>
        <span className='tag'>No account</span>
        <span className='tag'>No cloud sync</span>
      </div>
    </div>

    <div className='container'>
      <div
        className='scene'
        data-od-id='hero-scene'>
        <MvBar
          className='marvis-bar'
          data-od-id='marvis-bar'
        />
        <div className='editor'>
          <div className='editor-chrome'>
            <span className='dot' />
            <span className='dot' />
            <span className='dot' />
            <span className='meta'>
              capture.rs — marvis/apps/native/src-tauri
            </span>
          </div>
          <div className='editor-body'>
            <aside className='editor-side'>
              <div className='file'>ask.rs</div>
              <div className='file active'>capture.rs</div>
              <div className='file'>hotkeys.rs</div>
              <div className='file'>keystore.rs</div>
              <div className='file'>listen.rs</div>
              <div className='file'>windows.rs</div>
              <div className='file'>config.toml</div>
            </aside>
            <div className='editor-code'>
              <PreLines
                lines={EDITOR_LINES}
                indent='                '
              />
              {'\n              '}
            </div>
            <MvAskPanel
              className='marvis-panel'
              data-od-id='ask-panel'
              question='Why does ring.latest() return None on launch?'>
              <p>
                The buffer only fills after the first captured frame lands —
                once Screen Recording is granted. Until then,{' '}
                <code>latest()</code> is empty by design: Marvis never keeps
                frames from before you asked.
                <span className='mv-caret' />
              </p>
            </MvAskPanel>
          </div>
        </div>
      </div>
    </div>
  </section>
);
