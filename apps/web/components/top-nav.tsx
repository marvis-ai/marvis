import { Mark } from './icons';
import { WaitlistDialog } from './waitlist-dialog';

export const TopNav = () => (
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
      <WaitlistDialog
        triggerClassName='btn btn-secondary'
        triggerLabel='Download'
        triggerDataOdId='nav-cta'
      />
    </div>
  </header>
);
