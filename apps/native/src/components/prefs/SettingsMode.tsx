/**
 * Settings mode — the prototype's `osx-body`: 168px sidebar + scrolling
 * main. Icons are lucide glyphs; the active item is a surface pill, not
 * a second hue. "Re-run setup" re-opens the same window in onboarding
 * mode through `window_show_onboarding` (backend emits `prefs:mode`,
 * the shell remounts — the wizard resets to step 1 per spec).
 */
import { useState } from 'react';
import {
  Info,
  Keyboard,
  KeyRound,
  RotateCcw,
  Shield,
  SlidersHorizontal,
} from '@marvis/ui';
import { windowShowOnboarding } from '../../lib/commands';
import { AboutTab } from './AboutTab';
import { GeneralTab } from './GeneralTab';
import { HotkeysTab } from './HotkeysTab';
import { PrivacyTab } from './PrivacyTab';
import { ProvidersTab } from './ProvidersTab';
import type { PrefsData } from './types';

const TABS = [
  { id: 'general', label: 'General', icon: SlidersHorizontal },
  { id: 'providers', label: 'Providers', icon: KeyRound },
  { id: 'hotkeys', label: 'Hotkeys', icon: Keyboard },
  { id: 'privacy', label: 'Privacy & data', icon: Shield },
  { id: 'about', label: 'About', icon: Info },
] as const;

type TabId = (typeof TABS)[number]['id'];

export const SettingsMode = ({ data }: { data: PrefsData }) => {
  const [tab, setTab] = useState<TabId>('general');

  return (
    <div className='prf-body'>
      <nav
        className='prf-side'
        aria-label='Settings sections'>
        {TABS.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            type='button'
            className={`prf-side-item${tab === id ? ' is-on' : ''}`}
            onClick={() => setTab(id)}>
            <Icon />
            {label}
          </button>
        ))}
        <span className='prf-side-spacer' />
        <button
          type='button'
          className='prf-side-item prf-side-muted'
          onClick={() => void windowShowOnboarding().catch(() => {})}>
          <RotateCcw />
          Re-run setup
        </button>
        <span className='prf-side-foot'>tauri · rust</span>
      </nav>
      <div className='prf-main'>
        {tab === 'general' && <GeneralTab data={data} />}
        {tab === 'providers' && <ProvidersTab data={data} />}
        {tab === 'hotkeys' && <HotkeysTab data={data} />}
        {tab === 'privacy' && <PrivacyTab />}
        {tab === 'about' && <AboutTab />}
      </div>
    </div>
  );
};
