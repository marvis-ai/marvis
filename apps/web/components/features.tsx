import { CommandIcon, KeyIcon, MonitorIcon } from './icons';
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
        <p className='eyebrow'>What&rsquo;s different</p>
        <h2>Built like part of your desktop, not a tab you visit.</h2>
      </div>
      <div className='grid-3'>
        <div
          className='feature card-flat'
          data-od-id='feature-screen-aware'>
          <div className='feature-mark'>
            <MonitorIcon />
          </div>
          <h3>Screen-aware answers</h3>
          <p>
            Press ⌘⏎ and Marvis attaches the latest screen frame to your prompt,
            then streams a markdown answer into the panel. Frames sit in an
            in-memory ring buffer — about 60 seconds, capped at 120 frames — and
            are never written to disk.
          </p>
          <span className='meta'>
            “explain this error” · “summarize this page” · “what changed here”
          </span>
        </div>
        <div
          className='feature card-flat'
          data-od-id='feature-byo-model'>
          <div className='feature-mark'>
            <KeyIcon />
          </div>
          <h3>No account, no proxy</h3>
          <p>
            Your API key talks straight to the provider you choose — or run
            fully local inference through Ollama with no key at all.
            There&rsquo;s no Marvis account and no proxy in the middle.
          </p>
        </div>
        <div
          className='feature card-flat'
          data-od-id='feature-always-in-reach'>
          <div className='feature-mark'>
            <CommandIcon />
          </div>
          <h3>Always in reach</h3>
          <p>
            Four global chords — toggle, ask, screenshot, settings — each
            rebindable in Settings → Hotkeys. The bar drags anywhere and
            remembers its spot; <span className='num'>marvis://</span> links
            fire an ask from any app, script, or launcher.
          </p>
          <span className='meta'>
            ⌘/ · ⌘⏎ · ⌘⇧S · ⌘, — every key remappable
          </span>
        </div>
      </div>
    </div>
  </section>
);
