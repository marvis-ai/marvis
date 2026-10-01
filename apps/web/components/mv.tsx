import type { HTMLAttributes, ReactNode } from 'react';
import {
  CaptionsIcon,
  ChevronDownIcon,
  CopyIcon,
  HistoryIcon,
  MicAudioLinesIcon,
  MicIcon,
  MonitorDotIcon,
  PauseIcon,
  SettingsIcon,
  ShieldAlertIcon,
  SquareIcon,
  TimerIcon,
  XIcon,
} from '@marvis/ui';
import { MvLogo } from './icons';

type MvProps = HTMLAttributes<HTMLDivElement>;

/* The always-on-top bar (Bar.tsx): the window is the pill — 172×64 at
 * rest, grown to 600×64 for expanded content. Two gates exist: main and
 * needs_permission. 'mini' is not a gate — it's the resting capsule
 * (iris · capture · listen · history) the bar idles in until the iris
 * is clicked or a type-to-wake keypress morphs it into the input. */
type MvBarProps = MvProps & {
  gate?: 'main' | 'permission' | 'mini';
};

export const MvBar = ({
  gate = 'main',
  className = '',
  ...rest
}: MvBarProps) => (
  <div
    className={`mv mv-bar${gate === 'mini' ? ' is-mini' : ''} ${className}`}
    {...rest}>
    <div className='mv-bar-inner'>
      {gate === 'mini' && (
        <>
          <MvLogo />
          <span
            className='mv-icon-btn'
            title='Screen capture'>
            <MonitorDotIcon />
          </span>
          <span
            className='mv-icon-btn'
            title='Start listening'>
            <MicAudioLinesIcon />
          </span>
          <span
            className='mv-icon-btn'
            title='History'>
            <HistoryIcon />
          </span>
        </>
      )}
      {gate === 'main' && (
        <>
          <MvLogo />
          <input
            className='mv-input'
            placeholder='Ask Marvis…'
            aria-label='Ask Marvis'
          />
          <span
            className='mv-icon-btn'
            title='Dictate'>
            <MicIcon />
          </span>
          <span
            className='mv-icon-btn'
            title='Settings'>
            <SettingsIcon />
          </span>
        </>
      )}
      {gate === 'permission' && (
        <>
          <ShieldAlertIcon className='mv-shield' />
          <span className='mv-perm'>Screen recording needed</span>
          <button className='mv-btn mv-btn-primary'>Grant</button>
          <button className='mv-btn mv-btn-link'>Open settings</button>
        </>
      )}
    </div>
  </div>
);

/* Ask panel — 600px frosted card; header is the submitted question, body a
 * markdown stream ending in the block caret (AskPanel.tsx). */
type MvAskPanelProps = MvProps & {
  question: string;
  children: ReactNode;
};

export const MvAskPanel = ({
  question,
  children,
  className = '',
  ...rest
}: MvAskPanelProps) => (
  <div
    className={`mv mv-panel ${className}`}
    {...rest}>
    <div className='mv-panel-head'>
      <p>{question}</p>
      <span
        className='mv-icon-btn'
        title='Close'>
        <XIcon />
      </span>
    </div>
    <div className='mv-panel-body'>
      <div className='mv-md'>{children}</div>
    </div>
  </div>
);

/* Listen — the card's live capture surface (ListenSection.tsx): header
 * (title · sources/engine · LISTENING badge · pause/stop), speaker filter
 * row, timestamped speaker blocks with a dimmed interim caret, and the
 * pinned TLDR strip. Card modes share the 600px band. */
export const MvListen = ({ className = '', ...rest }: MvProps) => (
  <div
    className={`mv mv-panel mv-listen ${className}`}
    {...rest}>
    <div className='mv-listen-head'>
      <div className='mv-listen-id'>
        <p className='mv-listen-title'>Design sync</p>
        <p className='mv-listen-sub'>mic + system audio · deepgram nova-2</p>
      </div>
      <span className='mv-badge'>
        <i />
        LISTENING
      </span>
      <span
        className='mv-icon-btn'
        title='Pause'>
        <PauseIcon />
      </span>
      <span
        className='mv-icon-btn'
        title='Stop'>
        <SquareIcon />
      </span>
    </div>
    <div className='mv-filter'>
      <span className='mv-spk is-on'>all</span>
      <span className='mv-spk mv-spk-you'>
        <i />
        you
      </span>
      <span className='mv-spk mv-spk-1'>
        <i />
        speaker 1
      </span>
      <span className='mv-filter-meta'>
        <CaptionsIcon />3 lines
      </span>
      <span className='mv-filter-meta'>
        <TimerIcon />
        1:12
      </span>
      <span
        className='mv-icon-btn'
        title='Copy transcript'>
        <CopyIcon />
      </span>
    </div>
    <div className='mv-tt'>
      <div className='mv-tt-block mv-tt-you'>
        <div className='mv-tt-head'>
          <span className='mv-tt-time'>0:04</span>
          <i className='mv-tt-dot' />
          <span className='mv-tt-name'>You</span>
        </div>
        <p className='mv-tt-text'>
          Before we dive in — did everyone see the new launch date?
        </p>
      </div>
      <div className='mv-tt-block mv-tt-1'>
        <div className='mv-tt-head'>
          <span className='mv-tt-time'>0:41</span>
          <i className='mv-tt-dot' />
          <span className='mv-tt-name'>Speaker 1</span>
        </div>
        <p className='mv-tt-text'>
          Yes — moved to the 14th. Website copy needs to land by Friday, though.
        </p>
      </div>
      <div className='mv-tt-block mv-tt-you'>
        <div className='mv-tt-head'>
          <span className='mv-tt-time'>1:12</span>
          <i className='mv-tt-dot' />
          <span className='mv-tt-name'>You</span>
        </div>
        <p className='mv-tt-text'>
          I can take the landing page — draft this week,{' '}
          <span className='mv-tt-interim'>
            then share it Friday
            <span className='mv-caret' />
          </span>
        </p>
      </div>
    </div>
    <div className='mv-tldr'>
      <div className='mv-tldr-head'>
        <span>TLDR · Launch timing</span>
        <ChevronDownIcon />
      </div>
      <p className='mv-tldr-text'>
        Launch moved to the 14th; landing-page copy is due Friday; you&rsquo;re
        drafting it.
      </p>
    </div>
  </div>
);
