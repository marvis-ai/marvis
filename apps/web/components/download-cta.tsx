import { surfaceBg } from './styles';

export const DownloadCta = () => (
  <section
    className='section'
    id='download'
    data-od-id='cta-strip'
    style={{ textAlign: 'center', ...surfaceBg }}>
    <div
      className='container'
      style={{ maxWidth: '620px' }}>
      <h2>Your screen. Your keys. Your machine.</h2>
      <p
        className='lead'
        style={{ margin: '18px auto 32px' }}>
        Free and open source under MIT. The macOS build is available now —
        Windows and Linux are in development.
      </p>
      <div className='hero-cta'>
        <a
          className='btn btn-primary'
          href='#'
          data-od-id='cta-primary'>
          Download for macOS
        </a>
        <a
          className='btn btn-ghost btn-arrow'
          href='https://github.com/MarvisLLC/marvis'
          data-od-id='cta-github'>
          View source on GitHub
        </a>
      </div>
    </div>
  </section>
);
