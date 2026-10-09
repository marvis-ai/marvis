/**
 * General — the appearance override and the accent hue. Both write
 * `config_set`, which broadcasts `config:changed` — lib/theme.ts applies
 * both live to every window (the accent lands on `--accent`; every other
 * token `color-mix`es off it).
 */
import { configSet } from '@/lib/commands';
import { H2, MODEL_SEL, PRF_ROWS, cn } from '@/lib/classes';
import { LANGUAGES } from '@/lib/languages';
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
      <h2 className={H2}>General</h2>

      <div className={PRF_ROWS}>
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
          label='Main language'
          sub='The default for chat replies, meeting summaries, and dictation — unless you ask for another language in the moment.'>
          <select
            aria-label='Main language'
            className={cn(MODEL_SEL, 'w-40 flex-none')}
            value={cfg?.app.main_language ?? 'en'}
            onChange={(e) =>
              void configSet('app.main_language', e.target.value)
                .then(data.setConfig)
                .catch(() => {})
            }>
            {LANGUAGES.map((l) => (
              <option
                key={l.id}
                value={l.id}>
                {l.label}
              </option>
            ))}
          </select>
        </PrefRow>
        <PrefRow
          label='Accent color'
          sub='The one hue — live states, switches, and focus rings all derive from it.'
          last>
          <span className='inline-flex items-center gap-2.5'>
            <span className='font-mono text-xs tabular-nums'>{accent}</span>
            <input
              type='color'
              aria-label='Accent color'
              className='h-6 w-10 cursor-pointer rounded-[7px] border border-border bg-surface p-0.5 [&::-webkit-color-swatch-wrapper]:p-px [&::-webkit-color-swatch]:rounded [&::-webkit-color-swatch]:border-0'
              value={accent}
              onChange={(e) => setAccent(e.target.value)}
            />
          </span>
        </PrefRow>
      </div>
    </>
  );
};
