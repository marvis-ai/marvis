/**
 * About — the supplied wordmark (never redrawn), one-line pitch, and
 * metadata rows. Version resolves from the bundle via `getVersion()`
 * with the Cargo.toml value as fallback. "Re-run setup" re-opens the
 * same window in onboarding mode through `window_show_onboarding`
 * (backend emits `prefs:mode`, the shell remounts — the wizard resets
 * to step 1 per spec).
 */
import { useEffect, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { windowShowOnboarding } from '@/lib/commands';
import { BTN_LG, BTN_OUTLINE, PRF_ROWS, cn } from '@/lib/classes';
import { PrefRow, Tag } from './bits';

export const AboutTab = () => {
  const [version, setVersion] = useState('0.1.0');

  useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch(() => {});
  }, []);

  return (
    <>
      <img
        className='mt-2 mb-2.5 block h-8.5'
        src='/marvis-logo.svg'
        alt='Marvis'
      />
      <p className='mb-4 max-w-[46ch] text-[12.5px] leading-normal text-muted-foreground'>
        Sees your screen. Answers in place. Nothing else.
      </p>

      <div className={PRF_ROWS}>
        <PrefRow label='Version'>
          <Tag>{version}</Tag>
        </PrefRow>
        <PrefRow label='License'>
          <Tag>Apache-2.0</Tag>
        </PrefRow>
        <PrefRow label='Platforms'>
          <Tag>macOS now · windows / linux planned</Tag>
        </PrefRow>
        <PrefRow
          label='Setup'
          sub='Run the onboarding wizard again.'
          last>
          <button
            type='button'
            className={cn(BTN_LG, BTN_OUTLINE)}
            onClick={() => void windowShowOnboarding().catch(() => {})}>
            Re-run setup
          </button>
        </PrefRow>
      </div>
    </>
  );
};
