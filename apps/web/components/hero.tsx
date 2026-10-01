import { Badge, MicIcon, MonitorIcon, VideoIcon, XIcon } from '@marvis/ui';
import { MvBar, MvListen } from './mv';
import { WaitlistDialog } from './waitlist-dialog';

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
      <h1 className='hero-h1'>
        Ask about anything on your screen. Transcribe every meeting. Keep it all
        on your machine.
      </h1>
      <p className='lead'>
        Marvis is a small floating bar that lives above your work. It sees your
        screen only with permission, hears your meetings, and answers through
        the AI providers you choose — your keys, history, and screen data never
        leave your device.
      </p>
      <div className='hero-cta'>
        <WaitlistDialog
          triggerClassName='btn btn-primary'
          triggerLabel='Join the waitlist'
          triggerDataOdId='hero-cta-primary'
        />
        <a
          className='btn btn-ghost btn-arrow'
          href='#privacy'
          data-od-id='hero-cta-secondary'>
          How it stays private
        </a>
      </div>
      <p className='meta hero-note'>
        macOS build in private testing — you&rsquo;ll get a download link by
        email.
      </p>
      <div className='hero-meta'>
        <Badge
          variant='outline'
          className='tag'>
          Free &amp; Open Source · MIT
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
