import { Badge } from '@marvis/ui';
import { Fragment, type ReactNode } from 'react';
import { MvAskPanel, MvBar } from './mv';
import { PreLines } from './pre-lines';
import { WaitlistDialog } from './waitlist-dialog';

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
    <span className='c-meta'>
      {'/// newest frame — cloned, never serialized'}
    </span>
  </Fragment>,
  <Fragment key='l11'>
    <span className='c-mut'>pub fn</span> latest(&self) -&gt;
    Option&lt;Frame&gt; {'{'}
  </Fragment>,
  <Fragment key='l12'>
    self.frames.back().cloned(){' '}
    <span className='c-meta'>{'// None until the first capture lands'}</span>
  </Fragment>,
  <Fragment key='l13'>{'}'}</Fragment>,
];

export const Hero = () => (
  <section
    className='section hero'
    id='top'
    data-od-id='hero'>
    <div className='container hero-center'>
      <p className='eyebrow'>Private / Personal AI for All</p>
      <h1 className='hero-h1'>
        Ask anything about what&rsquo;s on your screen — and keep every frame
        on your machine.
      </h1>
      <p className='lead'>
        A floating bar for developers and power users who live at the keyboard.
        It sees your screen — only with permission — and your keys, history,
        and screen data never leave your machine.
      </p>
      <div className='hero-cta'>
        <WaitlistDialog
          triggerClassName='btn btn-primary'
          triggerLabel='Join the waitlist'
          triggerDataOdId='hero-cta-primary'
        />
        <a
          className='btn btn-ghost btn-arrow'
          href='#privacy'
          data-od-id='hero-cta-secondary'>
          How it stays private
        </a>
      </div>
      <p className='meta hero-note'>
        macOS build in private testing — you&rsquo;ll get a download link by
        email.
      </p>
      <div className='hero-meta'>
        <Badge
          variant='outline'
          className='tag'>
          Free &amp; Open Source · MIT
        </Badge>
        <Badge
          variant='outline'
          className='tag'>
          No Account
        </Badge>
        <Badge
          variant='outline'
          className='tag'>
          No Cloud Sync
        </Badge>
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
              capture/mod.rs — marvis/apps/native/src-tauri
            </span>
          </div>
          <div className='editor-body'>
            <aside className='editor-side'>
              <div className='file'>ask.rs</div>
              <div className='file active'>mod.rs</div>
              <div className='file'>macos.rs</div>
              <div className='file'>keystore.rs</div>
              <div className='file'>hotkey.rs</div>
              <div className='file'>deeplink.rs</div>
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
                frames from before permission.
                <span className='mv-caret' />
              </p>
            </MvAskPanel>
          </div>
        </div>
      </div>
    </div>
  </section>
);
