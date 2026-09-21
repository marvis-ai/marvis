/**
 * General — appearance override, bar position, deep-link surface, the
 * one accent, platform. Rows map to `pref-row` in the prototype; the
 * Appearance seg is the only control that writes config (all writes
 * broadcast `config:changed`, which lib/theme.ts applies live to every
 * window).
 */
import { configSet } from '../../lib/commands';
import { Kbd, PrefRow, Seg, Swatch, Tag } from './bits';
import type { PrefsData } from './types';

export const GeneralTab = ({ data }: { data: PrefsData }) => {
  const cfg = data.config;
  const pos = cfg?.window;

  const setAppearance = (v: string) => {
    void configSet('app.appearance', v)
      .then(data.setConfig)
      .catch(() => {});
  };

  // Re-center clears the persisted overrides — the pool falls back to
  // work-area center + BAR_TOP_OFFSET on the next place().
  const recenter = async () => {
    try {
      let c = await configSet('window.bar_x', null);
      c = await configSet('window.bar_y', null);
      data.setConfig(c);
    } catch {
      // write raced; config:changed keeps the row honest
    }
  };

  const posLabel =
    pos?.bar_x !== undefined && pos?.bar_y !== undefined
      ? `window.bar_x / bar_y — ${Math.round(pos.bar_x)} · ${Math.round(pos.bar_y)}`
      : 'window.bar_x / bar_y — not set';

  return (
    <>
      <h2>General</h2>
      <p className='sub'>
        The bar is always on top, on every space, and invisible to screen share.
      </p>

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
          label='Bar position'
          sub={
            <>
              Drag it, nudge it with <Kbd accel='Cmd+Up' />–
              <Kbd accel='Cmd+Right' />, or snap with{' '}
              <Kbd accel='Cmd+Shift+Left' />/<Kbd accel='Cmd+Shift+Right' />.
            </>
          }>
          <button
            type='button'
            className='mv-btn mv-btn-outline'
            onClick={() => void recenter()}>
            Re-center
          </button>
        </PrefRow>
        <PrefRow
          label='Remembered position'
          sub={<span className='num'>{posLabel}</span>}>
          <Tag>config.toml</Tag>
        </PrefRow>
        <PrefRow
          label='Deep link'
          sub={
            <>
              Anything under <span className='num'>marvis://</span> focuses the
              bar; <span className='num'>marvis://ask?text=…</span> asks
              directly.
            </>
          }>
          <Tag>marvis://ask?text=</Tag>
        </PrefRow>
        <PrefRow
          label='Accent color'
          sub='Marvis slate — reserved for listening, focus rings, and live states.'>
          <Swatch hex='#3a7294' />
        </PrefRow>
        <PrefRow
          label='Platform'
          sub='Phase 1 is macOS-only — ScreenCaptureKit and a local key file.'
          last>
          <span className='mv-pill'>macOS · now</span>
        </PrefRow>
      </div>
    </>
  );
};
