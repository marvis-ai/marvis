import { GithubMark, Mark } from './icons';
import { GITHUB_REPO } from '@/lib/constants';

/* Live stargazer count for the nav GitHub button — ISR-cached for an
 * hour; the count silently drops if the API is down or rate-limited. */
const starCount = async (): Promise<string | null> => {
  try {
    const res = await fetch(
      `https://api.github.com/repos${new URL(GITHUB_REPO).pathname}`,
      { next: { revalidate: 3600 } },
    );
    if (!res.ok) return null;
    const { stargazers_count: n } = (await res.json()) as {
      stargazers_count?: number;
    };
    if (typeof n !== 'number') return null;
    return n >= 1000 ? `${(n / 1000).toFixed(1).replace(/\.0$/, '')}k` : `${n}`;
  } catch {
    return null;
  }
};

export const TopNav = async () => {
  const stars = await starCount();
  return (
    <header
      className='topnav'
      data-od-id='topnav'>
      <div className='container topnav-inner'>
        <a
          className='brand'
          href='#top'
          data-od-id='brand'
          aria-label='Marvis home'>
          <Mark />
          <span className='wordmark'>Marvis</span>
        </a>
        <nav>
          <a href='#features'>Features</a>
          <a href='#privacy'>Privacy</a>
          <a href='#interface'>Interface</a>
          <a href='#hotkeys'>Hotkeys</a>
          <a href='#providers'>Providers</a>
        </nav>
        <a
          className='btn btn-secondary'
          href={GITHUB_REPO}
          target='_blank'
          rel='noopener noreferrer'
          data-od-id='nav-cta'
          aria-label={
            stars ? `Marvis on GitHub — ${stars} stars` : 'Marvis on GitHub'
          }>
          <GithubMark
            width={16}
            height={16}
          />
          {stars ?? 'GitHub'}
        </a>
      </div>
    </header>
  );
};
