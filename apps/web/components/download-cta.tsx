import { surfaceBg } from './styles';
import { APP_VERSION, GITHUB_RELEASES, GITHUB_REPO } from '@/lib/constants';

export const DownloadCta = () => (
  <section
    className='section'
    id='download'
    data-od-id='cta-strip'
    style={{ textAlign: 'center', ...surfaceBg }}>
    <div
      className='container'
      style={{ maxWidth: '620px' }}>
      <h2>Download Marvis for macOS, Windows, and Linux</h2>
      <p
        className='lead'
        style={{ margin: '18px auto 32px' }}>
        Free and open source under Apache-2.0. Runs completely on your device
        with zero telemetry. Builds ship directly on GitHub Releases.
      </p>
      <div className='hero-cta'>
        <a
          className='btn btn-primary'
          href={GITHUB_RELEASES}
          target='_blank'
          rel='noopener noreferrer'
          data-od-id='cta-primary'>
          Download Marvis
        </a>
        <a
          className='btn btn-ghost btn-arrow'
          href={GITHUB_REPO}
          target='_blank'
          rel='noopener noreferrer'
          data-od-id='cta-github'>
          View source on GitHub
        </a>
      </div>
      <p
        className='meta'
        style={{ marginTop: 'var(--space-4)' }}>
        Latest release · v{APP_VERSION}
      </p>
    </div>
  </section>
);
