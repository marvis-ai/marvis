/**
 * Settings mode — the prototype's `osx-body`: 168px sidebar + scrolling
 * main. Icons are lucide glyphs; the active item is a surface pill, not
 * a second hue.
 */
import { useState } from 'react';
import {
  InfoIcon,
  KeyboardIcon,
  KeyRoundIcon,
  PanelTopIcon,
  ShieldIcon,
  SlidersHorizontalIcon,
} from '@marvis/ui';
import { AboutTab } from './AboutTab';
import { BarTab } from './BarTab';
import { GeneralTab } from './GeneralTab';
import { HotkeysTab } from './HotkeysTab';
import { PrivacyTab } from './PrivacyTab';
import { ProvidersTab } from './ProvidersTab';
import type { PrefsData } from './types';

const TABS = [
  { id: 'general', label: 'General', icon: SlidersHorizontalIcon },
  { id: 'bar', label: 'Bar', icon: PanelTopIcon },
  { id: 'providers', label: 'Providers', icon: KeyRoundIcon },
  { id: 'hotkeys', label: 'Hotkeys', icon: KeyboardIcon },
  { id: 'privacy', label: 'Privacy & data', icon: ShieldIcon },
  { id: 'about', label: 'About', icon: InfoIcon },
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
        <span className='prf-side-foot'>
          @2026 Marvis AI
          <br />
          made with 💗
        </span>
      </nav>
      <div className='prf-main'>
        {tab === 'general' && <GeneralTab data={data} />}
        {tab === 'bar' && <BarTab />}
        {tab === 'providers' && <ProvidersTab data={data} />}
        {tab === 'hotkeys' && <HotkeysTab data={data} />}
        {tab === 'privacy' && <PrivacyTab />}
        {tab === 'about' && <AboutTab />}
      </div>
    </div>
  );
};
