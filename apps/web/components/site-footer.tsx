import Link from 'next/link';
import { Mark } from './icons';
import { GITHUB_RELEASES, GITHUB_REPO } from '@/lib/constants';

export const SiteFooter = () => (
  <footer
    className='pagefoot'
    data-od-id='footer'>
    <div
      className='container row-between'
      style={{ flexWrap: 'wrap', gap: '16px' }}>
      <span
        className='row'
        style={{ gap: '10px' }}>
        <Mark
          width={20}
          height={20}
        />
        <span className='wordmark'>Marvis</span>
      </span>
      <div
        className='row'
        style={{
          gap: '20px',
          fontSize: 'var(--text-sm)',
          color: 'var(--muted)',
          flexWrap: 'wrap',
        }}>
        <a
          href={GITHUB_RELEASES}
          target='_blank'
          rel='noopener noreferrer'>
          Releases
        </a>
        <a
          href={GITHUB_REPO}
          target='_blank'
          rel='noopener noreferrer'>
          GitHub
        </a>
        <a
          href='https://www.apache.org/licenses/LICENSE-2.0'
          target='_blank'
          rel='noopener noreferrer'>
          Apache-2.0 License
        </a>
        <Link href='/llms.txt'>llms.txt</Link>
      </div>
      <span
        className='meta'
        style={{ fontSize: 'var(--text-sm)' }}>
        © 2026 Marvis AI LLC
      </span>
    </div>
  </footer>
);
