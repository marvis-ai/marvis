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
      <a
        className='btn btn-primary'
        href='#'
        data-od-id='cta-primary'>
        Download for macOS
      </a>
    </div>
  </section>
);
