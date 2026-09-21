/**
 * General — the appearance override and the accent hue. Both write
 * `config_set`, which broadcasts `config:changed` — lib/theme.ts applies
 * both live to every window (the accent lands on `--accent`; every other
 * token `color-mix`es off it).
 */
import { configSet } from '../../lib/commands';
import { PrefRow, Seg } from './bits';
import type { PrefsData } from './types';

export const GeneralTab = ({ data }: { data: PrefsData }) => {
  const cfg = data.config;
  const accent = /^#[0-9a-f]{6}$/i.test(cfg?.app.accent ?? '')
    ? cfg!.app.accent
    : '#3a7294';

  const setAppearance = (v: string) => {
    void configSet('app.appearance', v)
      .then(data.setConfig)
      .catch(() => {});
  };

  const setAccent = (v: string) => {
    void configSet('app.accent', v)
      .then(data.setConfig)
      .catch(() => {});
  };

  return (
    <>
      <h2>General</h2>

      <div className='prf-rows'>
        <PrefRow
          label='Appearance'
          sub='Follows macOS by default — the capsule, chatbox, and this window share one setting.'>
          <Seg
            ariaLabel='Appearance'
            value={
              cfg?.app.appearance === 'light' || cfg?.app.appearance === 'dark'
                ? cfg.app.appearance
                : 'auto'
            }
            onChange={setAppearance}
            options={[
              { id: 'auto', label: 'Auto' },
              { id: 'light', label: 'Light' },
              { id: 'dark', label: 'Dark' },
            ]}
          />
        </PrefRow>
        <PrefRow
          label='Accent color'
          sub='The one hue — live states, switches, and focus rings all derive from it.'
          last>
          <span className='prf-color'>
            <input
              type='color'
              aria-label='Accent color'
              value={accent}
              onChange={(e) => setAccent(e.target.value)}
            />
            <span className='num'>{accent}</span>
          </span>
        </PrefRow>
      </div>
    </>
  );
};
