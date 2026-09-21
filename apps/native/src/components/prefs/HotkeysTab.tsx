/**
 * Hotkeys — the `hotkeys.*` bindings rendered live from config (rebind
 * via `config_set`), plus the fixed shortcuts that never reach config
 * (tagged "fixed"). The four nudges and two scrolls collapse into single
 * rows like the prototype's `hk-table`.
 */
import { Kbd, Tag } from './bits';
import type { PrefsData } from './types';

interface HotkeyRow {
  label: string;
  accels: string[]; // `'|'` renders the `–` range separator
  fixed?: boolean;
}

export const HotkeysTab = ({ data }: { data: PrefsData }) => {
  const hk = data.config?.hotkeys ?? {};

  const rows: HotkeyRow[] = [
    {
      label: 'Show / hide everything',
      accels: hk.toggle_visibility ? [hk.toggle_visibility] : [],
    },
    { label: 'Send ask', accels: hk.next_step ? [hk.next_step] : [] },
    {
      label: 'Move bar · 40 px',
      accels: ['move_up', 'move_down', 'move_left', 'move_right']
        .map((a) => hk[a])
        .filter(Boolean),
    },
    {
      label: 'Click-through',
      accels: hk.toggle_click_through ? [hk.toggle_click_through] : [],
    },
    {
      label: 'Scroll answer',
      accels: ['scroll_up', 'scroll_down'].map((a) => hk[a]).filter(Boolean),
    },
    { label: 'Screenshot → ask', accels: ['Cmd+Shift+S'], fixed: true },
    {
      label: 'Snap to edge',
      accels: ['Cmd+Shift+Left', 'Cmd+Shift+Right'],
      fixed: true,
    },
    {
      label: 'Jump to display n',
      accels: ['Cmd+Shift+1', '|', 'Cmd+Shift+9'],
      fixed: true,
    },
  ];

  return (
    <>
      <h2>Hotkeys</h2>
      <p className='sub'>
        Global, registered at the OS level. Rebind the nine configurable actions
        in <span className='num'>config.toml</span>.
      </p>

      <div className='prf-rows'>
        {rows.map((r, i) => (
          <div
            key={r.label}
            className='prf-row'
            style={i === rows.length - 1 ? { borderBottom: 0 } : undefined}>
            <div>
              <div className='pr-label'>{r.label}</div>
            </div>
            <span className='pr-ctl prf-hk-ctl'>
              <span className='prf-hk-keys'>
                {r.accels.map((a, j) =>
                  a === '|' ? (
                    <span
                      key={j}
                      className='num'>
                      –
                    </span>
                  ) : (
                    <Kbd
                      key={j}
                      accel={a}
                    />
                  ),
                )}
              </span>
              <Tag>{r.fixed ? 'fixed' : 'config'}</Tag>
            </span>
          </div>
        ))}
      </div>
      <p
        className='meta'
        style={{ marginTop: 14 }}>
        while locked, only show/hide + edge/display moves stay live — nothing
        that touches your key or screen.
      </p>
    </>
  );
};
