import type { ReactNode } from 'react';
import { leadTop, splitGrid, surfaceBg } from './styles';

const HOTKEYS: [ReactNode, string][] = [
  ['Toggle overlay visibility', '⌘ /'],
  ['Ask — send with screen frame', '⌘ ⏎'],
  ['Move bar', '⌘ ↑ ↓ ← →'],
  ['Snap bar to display edge', '⌘ ⇧ ← →'],
  [
    <>
      Move bar to display <em>n</em>
    </>,
    '⌘ ⇧ n',
  ],
  ['Toggle click-through', '⌘ M'],
  ['Scroll Ask panel', '⌘ ⇧ ↑ ↓'],
  ['Manual screenshot ask', '⌘ ⇧ S'],
];

export const Hotkeys = () => (
  <section
    className='section'
    id='hotkeys'
    data-od-id='hotkeys'
    style={surfaceBg}>
    <div
      className='container grid-2'
      style={splitGrid}>
      <div className='stack'>
        <div style={{ maxWidth: '34ch' }}>
          <p className='eyebrow'>Always in reach</p>
          <h2>Drive it from the keyboard.</h2>
          <p
            className='lead'
            style={{ ...leadTop, fontSize: '17px' }}>
            Summon, move, snap, scroll, and click through the overlay from
            anywhere. Every shortcut is configurable under{' '}
            <span className='num'>[hotkeys]</span> in{' '}
            <span className='num'>~/.marvis/config.toml</span>.
          </p>
        </div>
        <div
          className='card'
          data-od-id='deeplink-card'
          style={{ boxShadow: 'none' }}>
          <p
            className='eyebrow'
            style={{ color: 'var(--muted)', marginBottom: '10px' }}>
            Deep links
          </p>
          <p
            className='num'
            style={{ fontSize: '14px', margin: 0, color: 'var(--fg-2)' }}>
            marvis://ask?text=explain+this+error
          </p>
          <p
            style={{
              margin: '10px 0 0',
              color: 'var(--muted)',
              fontSize: '14px',
            }}>
            Focuses the bar and fires an Ask from any app — a browser, a script,
            a launcher. Any other <span className='num'>marvis://</span> link
            just surfaces the overlay.
          </p>
        </div>
      </div>
      <table
        className='ds-table'
        data-od-id='hotkey-table'>
        <thead>
          <tr>
            <th>Action</th>
            <th>Shortcut</th>
          </tr>
        </thead>
        <tbody>
          {HOTKEYS.map(([action, keys]) => (
            <tr key={keys}>
              <td>{action}</td>
              <td>
                <span className='kbd'>{keys}</span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  </section>
);
