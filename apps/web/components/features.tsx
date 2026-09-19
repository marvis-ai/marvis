import { KeyIcon, LockIcon, MonitorIcon } from './icons';
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
        </div>
        <div
          className='feature card-flat'
          data-od-id='feature-byo-model'>
          <div className='feature-mark'>
            <KeyIcon />
          </div>
          <h3>Bring your own model</h3>
          <p>
            Point Marvis at OpenAI, Anthropic, or Gemini with your own API key —
            or skip keys entirely and run fully local inference through Ollama.
            There&rsquo;s no Marvis account and no proxy in the middle.
          </p>
        </div>
        <div
          className='feature card-flat'
          data-od-id='feature-keystore'>
          <div className='feature-mark'>
            <LockIcon />
          </div>
          <h3>Your keys stay sealed</h3>
          <p>
            Provider keys live in <span className='num'>keys.enc</span>,
            encrypted with AES-256-GCM under a passphrase-derived Argon2id key
            and validated before they&rsquo;re stored. The UI only ever sees
            masked values.
          </p>
          <span className='meta'>openai: set ••••1234</span>
        </div>
      </div>
    </div>
  </section>
);
