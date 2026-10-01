import { leadTop, splitGrid } from './styles';

export const Providers = () => (
  <section
    className='section'
    id='providers'
    data-od-id='providers'>
    <div
      className='container grid-2'
      style={splitGrid}>
      <div className='stack'>
        <div style={{ maxWidth: '34ch' }}>
          <p className='eyebrow'>Your model, your call</p>
          <h2>Bring your own everything.</h2>
          <p
            className='lead'
            style={{ ...leadTop, fontSize: '17px' }}>
            API keys talk straight to the providers you pick — no Marvis proxy
            in the middle. Order them into a failover chain: a failed call hands
            off to the next configured provider.
          </p>
        </div>
        <div
          className='row'
          style={{ flexWrap: 'wrap', gap: '10px' }}>
          <span className='tag'>OpenAI</span>
          <span className='tag'>Anthropic</span>
          <span className='tag'>Gemini</span>
          <span className='tag'>OpenRouter</span>
          <span className='tag'>Ollama · local</span>
          <span className='tag'>OpenAI-compatible</span>
        </div>
        <div>
          <p
            className='meta'
            style={{
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
              marginBottom: '10px',
            }}>
            Transcription
          </p>
          <div
            className='row'
            style={{ flexWrap: 'wrap', gap: '10px' }}>
            <span className='tag'>Deepgram · streaming</span>
            <span className='tag'>whisper.cpp · local</span>
            <span className='tag'>sherpa · local</span>
          </div>
        </div>
        <p
          className='meta'
          style={{ margin: 0 }}>
          Keys live in <span className='num'>keys.json</span>, shown masked —{' '}
          <span className='num'>…last4</span>. An optional vision provider reads
          each frame first, so any text model can still answer about your
          screen.
        </p>
      </div>
      <div data-od-id='platform-list'>
        <article
          className='log-row'
          data-od-id='platform-macos'>
          <h3>macOS</h3>
          <span className='meta meta-desc'>
            Apple Silicon & Intel · ScreenCaptureKit
          </span>
          <span className='pull'>
            <span className='pill pill-green'>Available now</span>
          </span>
        </article>
        <article
          className='log-row'
          data-od-id='platform-windows'>
          <h3>Windows</h3>
          <span className='meta meta-desc'>Same Rust core, native capture</span>
          <span className='pull'>
            <span className='tag'>In development</span>
          </span>
        </article>
        <article
          className='log-row'
          data-od-id='platform-linux'>
          <h3>Linux</h3>
          <span className='meta meta-desc'>Same Rust core, native capture</span>
          <span className='pull'>
            <span className='tag'>In development</span>
          </span>
        </article>
      </div>
    </div>
  </section>
);
