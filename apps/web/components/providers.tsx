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
          <h2>Bring your own model.</h2>
          <p
            className='lead'
            style={{ ...leadTop, fontSize: '17px' }}>
            Your key talks straight to the provider you pick — or nowhere at
            all. With Ollama, inference runs fully on-device and no API key is
            needed.
          </p>
        </div>
        <div
          className='row'
          style={{ flexWrap: 'wrap', gap: '10px' }}>
          <span className='tag'>OpenAI</span>
          <span className='tag'>Anthropic</span>
          <span className='tag'>Gemini</span>
          <span className='tag'>Ollama · local</span>
        </div>
        <p
          className='meta'
          style={{ margin: 0 }}>
          Keys are stored in <span className='num'>keys.json</span> and only
          ever shown masked — <span className='num'>…last4</span>.
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
