import { CaptionsIcon } from '@marvis/ui';
import { LockIcon, MonitorIcon } from './icons';
import { sectionStack } from './styles';

export const Features = () => (
  <section
    className='section'
    id='features'
    data-od-id='features'>
    <div
      className='container stack'
      style={sectionStack}>
      <div style={{ maxWidth: '40ch' }}>
        <p className='eyebrow'>What it does</p>
        <h2>Two jobs, done quietly — over everything you open.</h2>
      </div>
      <div className='grid-3'>
        <div
          className='feature card-flat'
          data-od-id='feature-screen-aware'>
          <div className='feature-mark'>
            <MonitorIcon />
          </div>
          <h3>Answers with your screen in context</h3>
          <p>
            Press ⌘⏎ and the latest screen frame rides along with your prompt —
            “explain this”, “what changed”, “compare these”. The answer streams
            back as markdown into the overlay.
          </p>
          <span className='meta'>
            frames live in a 60-second memory ring — never on disk
          </span>
        </div>
        <div
          className='feature card-flat'
          data-od-id='feature-listen'>
          <div className='feature-mark'>
            <CaptionsIcon strokeWidth={1.6} />
          </div>
          <h3>Meetings, transcribed as they happen</h3>
          <p>
            Mic plus system audio become a speaker-labeled transcript while the
            call runs — pause and resume anytime, filter by voice, copy the
            whole thing. Every five turns, a rolling TLDR lands with follow-up
            questions you can ask in one click.
          </p>
          <span className='meta'>
            deepgram · whisper.cpp · sherpa — your engine, fully local if you
            want
          </span>
        </div>
        <div
          className='feature card-flat'
          data-od-id='feature-private'>
          <div className='feature-mark'>
            <LockIcon />
          </div>
          <h3>Private by design</h3>
          <p>
            No account, no sync, no Marvis servers. Keys live in{' '}
            <span className='num'>~/.marvis</span> masked to{' '}
            <span className='num'>…last4</span>, and calls go straight to the
            provider you pick — or to Ollama, fully on-device.
          </p>
          <span className='meta'>free &amp; open source · MIT</span>
        </div>
      </div>
    </div>
  </section>
);
