import type { HTMLAttributes, ReactNode } from 'react';
import { CloseIcon, GearIcon, MicIcon, MvLogo, ShieldIcon } from './icons';

type MvProps = HTMLAttributes<HTMLDivElement>;

/* 353×47 translucent pill — the always-on-top bar, in its three gates
 * (Bar.tsx): main input, needs_unlock, needs_permission. */
type MvBarProps = MvProps & {
  gate?: 'main' | 'unlock' | 'permission';
};

export const MvBar = ({
  gate = 'main',
  className = '',
  ...rest
}: MvBarProps) => (
  <div
    className={`mv mv-bar ${className}`}
    {...rest}>
    <div className='mv-bar-inner'>
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
      {gate === 'unlock' && (
        <>
          <MvLogo />
          <span className='mv-perm'>Unlock with Touch ID / password</span>
          <button className='mv-btn mv-btn-primary'>Unlock</button>
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

/* Settings — 240px panel, provider rows with masked keys (SettingsPanel.tsx). */
const PROVIDER_ROWS: {
  name: string;
  note: ReactNode;
  keyPlaceholder?: string;
  models?: string[];
  dim?: boolean;
}[] = [
  {
    name: 'OpenAI',
    note: (
      <span>
        <span className='mv-row-note'>…7B2q</span>
        <a className='mv-link'>Remove</a>
      </span>
    ),
    keyPlaceholder: 'Replace key',
    models: ['gpt-4o', 'gpt-4o-mini'],
  },
  {
    name: 'Anthropic',
    note: <span className='mv-row-note'>no key set</span>,
    keyPlaceholder: 'API key',
    models: ['claude-sonnet-4-5'],
  },
  {
    name: 'Gemini',
    note: <span className='mv-row-note'>no key set</span>,
    keyPlaceholder: 'API key',
    models: ['gemini-2.0-flash'],
  },
  {
    name: 'Ollama',
    note: <span className='mv-row-note'>local · no key needed</span>,
    models: ['llama3.2', 'qwen2.5'],
  },
  {
    name: 'Deepgram',
    note: <span className='mv-row-note'>Phase 2</span>,
    dim: true,
  },
];

export const MvSettings = ({ className = '', ...rest }: MvProps) => (
  <div
    className={`mv mv-panel mv-settings ${className}`}
    {...rest}>
    <div className='mv-panel-head'>
      <p>Settings</p>
      <span
        className='mv-icon-btn'
        title='Close'>
        <CloseIcon />
      </span>
    </div>
    <div className='mv-settings-body'>
      {PROVIDER_ROWS.map((provider) => (
        <div
          key={provider.name}
          className={`mv-row${provider.dim ? ' is-dim' : ''}`}>
          <div className='mv-row-top'>
            <span className='mv-row-name'>{provider.name}</span>
            {provider.note}
          </div>
          {provider.keyPlaceholder && (
            <div className='mv-keyline'>
              <input
                className='mv-key-input'
                placeholder={provider.keyPlaceholder}
                aria-label={`${provider.name} API key`}
              />
              <button className='mv-btn mv-btn-primary'>Save</button>
            </div>
          )}
          {provider.models && (
            <select
              className='mv-select'
              aria-label={`${provider.name} model`}>
              {provider.models.map((model) => (
                <option key={model}>{model}</option>
              ))}
            </select>
          )}
        </div>
      ))}
      <div className='mv-row-foot'>
        <button className='mv-btn mv-btn-outline'>Lock keys</button>
      </div>
    </div>
  </div>
);

/* Listen — 400px window; today it ships only its Phase-2 chrome
 * (ListenPanel.tsx), so the mock reproduces exactly that. */
export const MvListen = ({ className = '', ...rest }: MvProps) => (
  <div
    className={`mv mv-listen ${className}`}
    {...rest}>
    <div className='mv-listen-chrome'>
      <MicIcon />
      <span>Listen arrives in Phase 2</span>
    </div>
  </div>
);
