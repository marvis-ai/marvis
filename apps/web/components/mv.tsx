import type { HTMLAttributes, ReactNode } from 'react';
import {
  CameraIcon,
  CloseIcon,
  GearIcon,
  MicIcon,
  MvLogo,
  ShieldIcon,
} from './icons';

type MvProps = HTMLAttributes<HTMLDivElement>;

/* 353×47 translucent pill — the always-on-top bar (Bar.tsx). Two gates
 * exist: main and needs_permission. 'mini' is not a gate — it's the
 * 104px capsule the bar rests in until the iris is clicked or a
 * type-to-wake keypress morphs it into the input. */
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
            title='Screenshot ask'>
            <CameraIcon />
          </span>
          <span
            className='mv-icon-btn is-off'
            title='Coming soon'>
            <MicIcon />
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
            className='mv-icon-btn is-off'
            title='Coming soon'>
            <MicIcon />
          </span>
          <span
            className='mv-icon-btn'
            title='Settings'>
            <GearIcon />
          </span>
        </>
      )}
      {gate === 'permission' && (
        <>
          <ShieldIcon className='mv-shield' />
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
        <CloseIcon />
      </span>
    </div>
    <div className='mv-panel-body'>
      <div className='mv-md'>{children}</div>
    </div>
  </div>
);

/* Listen — 400px window; it ships only its Phase-2 chrome
 * (ListenPanel.tsx): a muted waveform, the label, and the
 * deepgram · stt chip. The mock reproduces exactly that. */
export const MvListen = ({ className = '', ...rest }: MvProps) => (
  <div
    className={`mv mv-listen ${className}`}
    {...rest}>
    <div className='mv-listen-chrome'>
      <span
        className='mv-wave'
        aria-hidden='true'>
        <i />
        <i />
        <i />
        <i />
        <i />
      </span>
      <span className='mv-empty'>Listen arrives in Phase 2</span>
      <span className='mv-chip'>deepgram · stt</span>
    </div>
  </div>
);
