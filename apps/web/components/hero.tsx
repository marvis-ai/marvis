import { Badge, MicIcon, MonitorIcon, VideoIcon, XIcon } from '@marvis/ui';
import { MvBar, MvListen } from './mv';
import { APP_VERSION, GITHUB_RELEASES } from '@/lib/constants';
import Link from 'next/link';

const TILES: { initials: string; name: string; cls: string }[] = [
  { initials: 'AK', name: 'Ava', cls: 'av-1' },
  { initials: 'JO', name: 'Jonah', cls: 'av-2' },
  { initials: 'MR', name: 'Meera', cls: 'av-3' },
  { initials: 'RS', name: 'Rui', cls: 'av-4' },
];

export const Hero = () => (
  <section
    className='section hero'
    id='top'
    data-od-id='hero'>
    <div className='container hero-center'>
      <p className='eyebrow'>Private / Personal AI for All</p>
      <h1 className='hero-h1'>Your screen, your meetings, your machine.</h1>
      <p className='lead'>
        Marvis is a small floating bar that lives above your work. It sees your
        screen only with permission, hears your meetings, and answers through
        the AI providers you choose — your keys, history, and screen data never
        leave your device.
      </p>
      <div className='hero-cta'>
        <Link
          className='flex flex-col items-center gap-0.5 btn-primary text-primary-foreground rounded-xl px-8 py-2'
          href={GITHUB_RELEASES}
          target='_blank'
          rel='noopener noreferrer'
          data-od-id='hero-cta-primary'>
          <span className='font-semibold'>Download</span>
          <span className='text-xs text-muted-foreground/10'>
            Latest version: {APP_VERSION}
          </span>
        </Link>
        <a
          className='btn btn-ghost btn-arrow'
          href='#privacy'
          data-od-id='hero-cta-secondary'>
          How it stays private
        </a>
      </div>
      <p className='meta hero-note'>
        macOS, Windows, and Linux — the latest builds ship on GitHub Releases.
      </p>
      <div className='hero-meta'>
        <Badge
          variant='outline'
          className='tag'>
          Free &amp; Open Source · Apache-2.0
        </Badge>
        <Badge
          variant='outline'
          className='tag'>
          No Account
        </Badge>
        <Badge
          variant='outline'
          className='tag'>
          No Cloud Sync
        </Badge>
      </div>
    </div>

    <div className='container'>
      <div
        className='scene'
        data-od-id='hero-scene'>
        <MvBar
          className='marvis-bar'
          data-od-id='marvis-bar'
        />
        <div className='meet'>
          <div className='meet-chrome'>
            <span className='dot' />
            <span className='dot' />
            <span className='dot' />
            <span className='meta'>Design sync — 4 participants</span>
          </div>
          <div
            className='meet-body'
            aria-hidden='true'>
            {TILES.map((t) => (
              <div
                key={t.name}
                className='meet-tile'>
                <span className={`meet-av ${t.cls}`}>{t.initials}</span>
                <span className='meet-name'>{t.name}</span>
              </div>
            ))}
          </div>
          <div
            className='meet-bar'
            aria-hidden='true'>
            <span className='meet-btn'>
              <MicIcon />
            </span>
            <span className='meet-btn'>
              <VideoIcon />
            </span>
            <span className='meet-btn'>
              <MonitorIcon />
            </span>
            <span className='meet-btn is-leave'>
              <XIcon />
            </span>
          </div>
          <MvListen
            className='marvis-panel'
            data-od-id='listen-panel'
          />
        </div>
      </div>
    </div>
  </section>
);
