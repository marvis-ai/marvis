'use client';

import { useTheme } from 'next-themes';
import { useSyncExternalStore } from 'react';
import { MoonIcon, SunIcon } from '@marvis/ui';
import { MarvisAppIcon } from './icons';
import { MvAskPanel, MvBar, MvListen } from './mv';
import { leadTop, sectionStack } from './styles';

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
    desc: 'The bar idles as a 172px capsule: iris, screen-capture toggle, the Listen recorder, and history. Click the iris or just start typing — it morphs open.',
    gate: 'mini',
    label: 'state: mini',
  },
  {
    id: 'shot-gate-main',
    title: 'Ready to ask',
    desc: 'The default gate. Type — or dictate with the mic — and ⌘⏎ sends the latest screen frame with your question.',
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
            <h2>One bar. Cards when you need them.</h2>
            <p
              className='lead'
              style={leadTop}>
              Everything floats above your work — a capsule at rest, an input
              for asks, and cards for chat, listen, and history in one grown
              window. These are the app&rsquo;s real views, recreated
              faithfully. The toggle swaps in its dark theme.
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
              aria-pressed={active === 'light'}
              onClick={() => setTheme('light')}>
              <SunIcon />
              Light
            </button>
            <button
              type='button'
              className={`seg-btn ${active === 'dark' ? 'is-on' : ''}`}
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
                <span className='gw-url'>Design sync — agenda</span>
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
            <span>bar · 172px idle → 600px grown · always on top</span>
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

        <figure
          className='shot'
          data-od-id='shot-listen'>
          <div className='shot-pad'>
            <p className='eyebrow'>Listen</p>
            <h3>The transcript writes itself.</h3>
            <p>
              Mic and system audio become a speaker-labeled document while the
              call runs — pause and resume, filter by voice, copy it all. Every
              five turns a TLDR lands with follow-ups you can fire straight into
              the session&rsquo;s own chat. Finished meetings stay readable from
              History.
            </p>
          </div>
          <div className='mv-stage-center'>
            <MvListen />
          </div>
          <figcaption className='shot-cap'>
            <span>listen card · 600px</span>
            <span>
              one window, grown — chat · listen · history share the band
            </span>
            <span>rolling summary every 5 turns</span>
          </figcaption>
        </figure>
      </div>
    </section>
  );
};
