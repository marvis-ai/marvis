'use client';

import { useTheme } from 'next-themes';
import { useSyncExternalStore } from 'react';
import { MarvisAppIcon, MoonIcon, SunIcon } from './icons';
import { MvAskPanel, MvBar, MvListen } from './mv';
import { leadTop, mutedBody, sectionStack } from './styles';

const noopSubscribe = () => () => {};

/* The bar's real states, faithful to Bar.tsx (design.md §6): the resting
 * capsule plus the two gates — main and needs_permission. */
const GATES: {
  id: string;
  title: string;
  desc: string;
  gate: 'main' | 'mini' | 'permission';
  label: string;
}[] = [
  {
    id: 'shot-state-mini',
    title: 'At rest — a capsule',
    desc: 'The bar idles as a 104px capsule: iris, camera, mic. Click it or just start typing — it morphs open.',
    gate: 'mini',
    label: 'state: mini',
  },
  {
    id: 'shot-gate-main',
    title: 'Ready to ask',
    desc: 'The default gate. Type, press ⌘⏎, and the latest screen frame rides along with your prompt.',
    gate: 'main',
    label: 'gate: main',
  },
  {
    id: 'shot-gate-permission',
    title: 'Permission needed',
    desc: 'Without Screen Recording, Marvis can’t see your screen — so the bar asks before anything else.',
    gate: 'permission',
    label: 'gate: needs_permission',
  },
];

export const InterfaceSection = () => {
  const { theme, setTheme } = useTheme();
  /* mounted comes from useSyncExternalStore: getServerSnapshot holds 'light'
   * through SSR and the hydration pass, getSnapshot flips true right after —
   * the saved theme lands post-hydration without an effect-driven setState. */
  const mounted = useSyncExternalStore(
    noopSubscribe,
    () => true,
    () => false,
  );
  const active = mounted && theme === 'dark' ? 'dark' : 'light';

  return (
    <section
      className='section'
      id='interface'
      data-od-id='interface'>
      <div
        className='container stack shots'
        style={sectionStack}>
        <div className='iface-head'>
          <div style={{ maxWidth: '42ch' }}>
            <p className='eyebrow'>The interface</p>
            <h2>One bar, two panels — nothing else.</h2>
            <p
              className='lead'
              style={leadTop}>
              Everything floats above your work — a capsule at rest, an input
              bar for asks, a panel for answers. These are the app&rsquo;s real
              views, recreated 1:1 at actual size. The toggle swaps in its dark
              theme.
            </p>
          </div>
          <div
            className='seg'
            role='group'
            aria-label='Interface theme'
            data-od-id='interface-theme-toggle'>
            <button
              type='button'
              className={`seg-btn ${active === 'light' ? 'is-on' : ''}`}
              data-theme-btn='light'
              aria-pressed={active === 'light'}
              onClick={() => setTheme('light')}>
              <SunIcon />
              Light
            </button>
            <button
              type='button'
              className={`seg-btn ${active === 'dark' ? 'is-on' : ''}`}
              data-theme-btn='dark'
              aria-pressed={active === 'dark'}
              onClick={() => setTheme('dark')}>
              <MoonIcon />
              Dark
            </button>
          </div>
        </div>

        <figure
          className='shot'
          data-od-id='shot-desktop'>
          <div className='shot-desktop'>
            <div className='shot-menubar'>
              <span>Finder</span>
              <span className='dim'>File</span>
              <span className='dim'>Edit</span>
              <span className='dim'>View</span>
              <span className='dim max-[480px]:hidden'>Go</span>
              <span className='dim max-[480px]:hidden'>Window</span>
              <span className='dim max-[480px]:hidden'>Help</span>
              <span className='shot-clock whitespace-nowrap'>Fri 9:41 AM</span>
            </div>
            <div
              className='ghost-win'
              aria-hidden='true'>
              <div className='gw-chrome'>
                <i />
                <i />
                <i />
                <span className='gw-url'>github.com/marvis-ai/marvis</span>
              </div>
              <div className='gw-lines'>
                <i style={{ width: '82%' }} />
                <i style={{ width: '64%' }} />
                <i style={{ width: '71%' }} />
                <i style={{ width: '48%' }} />
                <i style={{ width: '77%' }} />
                <i style={{ width: '58%' }} />
              </div>
            </div>
            <MvBar className='shot-bar' />
            <MvAskPanel
              className='shot-ask'
              question='Where do my API keys actually live?'>
              <p>
                In <code>keys.json</code> inside <code>~/.marvis</code> — a
                plaintext file at <code>0600</code> in a <code>0700</code>{' '}
                folder, with no copy anywhere else. The UI only ever shows the
                masked form, like <code>…7B2q</code>, so a typed key is never
                echoed back on screen.
                <span className='mv-caret' />
              </p>
            </MvAskPanel>
            <div
              className='shot-dock'
              aria-hidden='true'>
              <i />
              <i />
              <i />
              <i />
              <i />
              <MarvisAppIcon />
            </div>
          </div>
          <figcaption className='shot-cap'>
            <span>bar · 353×47 · always on top</span>
            <span>ask panel · 600px · drops 8px below the bar</span>
            <span>frameless translucent webviews</span>
          </figcaption>
        </figure>

        <div className='grid-3'>
          {GATES.map((gate) => (
            <figure
              key={gate.id}
              className='shot'
              data-od-id={gate.id}>
              <div className='shot-pad'>
                <h3>{gate.title}</h3>
                <p>{gate.desc}</p>
              </div>
              <div className='mv-stage'>
                <MvBar
                  gate={gate.gate}
                  style={{ marginInline: 'auto' }}
                />
              </div>
              <figcaption className='gate-label'>{gate.label}</figcaption>
            </figure>
          ))}
        </div>

        <div
          className='grid-2'
          style={{ alignItems: 'center' }}>
          <figure
            className='shot'
            data-od-id='shot-listen'>
            <div className='mv-stage-center'>
              <MvListen />
            </div>
            <figcaption className='shot-cap'>
              <span>listen · 400px wide · drops 8px below the bar</span>
              <span>docks 8px left of ask when both are open</span>
            </figcaption>
          </figure>
          <div
            className='stack'
            style={{ maxWidth: '34ch' }}>
            <div>
              <p className='eyebrow'>Listen, 400px wide</p>
              <h3>Docks off Ask&rsquo;s left edge.</h3>
            </div>
            <p style={mutedBody}>
              When both panels are open, Listen snaps 8px to the left of the
              600px Ask panel — the layout engine keeps every panel clear of the
              others. Meeting transcription lands in Phase 2 on Deepgram; the
              window and its chrome already ship.
            </p>
            <p
              className='meta'
              style={{ margin: 0 }}>
              Centers under the bar when Ask is closed
            </p>
          </div>
        </div>
      </div>
    </section>
  );
};
