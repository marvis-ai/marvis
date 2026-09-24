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
import { cn } from '../../lib/classes';
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
    <div className='grid min-h-0 flex-1 grid-cols-[168px_1fr]'>
      <nav
        className='flex flex-col gap-0.5 border-r border-border bg-[color-mix(in_oklch,var(--bg)_55%,var(--surface))] px-2 pt-9 pb-2.5'
        aria-label='Settings sections'>
        {TABS.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            type='button'
            className={cn(
              'flex w-full items-center gap-2 rounded-[7px] border-0 bg-transparent px-2.25 py-1.5 text-left text-[12.5px] font-medium text-foreground transition-[background,color] duration-(--motion-fast) ease-(--ease) hover:bg-fg-soft motion-reduce:transition-none',
              tab === id && 'bg-[color-mix(in_oklch,var(--fg)_8%,transparent)]',
            )}
            onClick={() => setTab(id)}>
            <Icon
              className={cn(
                'size-3.75 flex-none',
                tab === id ? 'text-accent-text' : 'text-muted-foreground',
              )}
            />
            {label}
          </button>
        ))}
        <span className='flex-1' />
        <span className='px-2.25 pt-2 pb-0.5 text-center font-mono text-[10px] leading-[1.7] text-muted-foreground'>
          @2026 Marvis AI
          <br />
          made with 💗
        </span>
      </nav>
      <div className='min-w-0 overflow-y-auto px-6 pt-8 pb-5.5'>
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
