'use client';

import { useSyncExternalStore } from 'react';
import { MarvisAppIcon, MoonIcon, SunIcon } from './icons';
import { MvAskPanel, MvBar, MvListen, MvSettings } from './mv';
import { leadTop, mutedBody, sectionStack } from './styles';

const THEME_KEY = 'marvis-iface-theme';
type Theme = 'light' | 'dark';

/* Interface shots can run in the app's real dark theme (design.md §6).
 * The toggle persists to localStorage[marvis-iface-theme], validated to
 * light/dark with light default; storage access is try/catch-wrapped so
 * storage-less contexts degrade gracefully. localStorage is read through
 * useSyncExternalStore — light renders server-side, the saved value lands
 * post-hydration (mirrors the original inline script). */
const listeners = new Set<() => void>();

const readTheme = (): Theme => {
  try {
    return localStorage.getItem(THEME_KEY) === 'dark' ? 'dark' : 'light';
  } catch {
    return 'light';
  }
};

const themeStore = {
  get: (): Theme => (typeof window === 'undefined' ? 'light' : readTheme()),
  set: (theme: Theme) => {
    try {
      localStorage.setItem(THEME_KEY, theme);
    } catch {
      /* storage-less preview context */
    }
    listeners.forEach((listener) => listener());
  },
  subscribe: (listener: () => void) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
};

/* The three bar gates, faithful to Bar.tsx (design.md §6). */
const GATES: {
  id: string;
  title: string;
  desc: string;
  gate: 'main' | 'unlock' | 'permission';
  label: string;
}[] = [
  {
    id: 'shot-gate-main',
    title: 'Ready to ask',
    desc: 'The default gate. Type, press ⌘⏎, and the latest screen frame rides along with your prompt.',
    gate: 'main',
    label: 'gate: main',
  },
  {
    id: 'shot-gate-unlock',
    title: 'Locked keystore',
    desc: 'Keys stay sealed until you unlock with Touch ID, Face ID, or your Mac password.',
    gate: 'unlock',
    label: 'gate: needs_unlock',
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
  const theme = useSyncExternalStore(
    themeStore.subscribe,
    themeStore.get,
    () => 'light' as Theme,
  );
  const apply = themeStore.set;

  return (
    <section
      className='section'
      id='interface'
      data-od-id='interface'>
      <div
        className='container stack shots'
        data-theme={theme}
        style={sectionStack}>
        <div className='iface-head'>
          <div style={{ maxWidth: '42ch' }}>
            <p className='eyebrow'>The interface</p>
            <h2>One bar, three panels — nothing else.</h2>
            <p
              className='lead'
              style={leadTop}>
              Recreated 1:1 from the app&rsquo;s own views: a 353×47 bar, a
              600px Ask panel, a 400px Listen panel, and a 240px Settings panel
              — frameless, frosted overlays, not browser windows. The toggle
              swaps in the app&rsquo;s real dark theme.
            </p>
          </div>
          <div
            className='seg'
            role='group'
            aria-label='Interface theme'
            data-od-id='interface-theme-toggle'>
            <button
              type='button'
              className={`seg-btn ${theme === 'light' ? 'is-on' : ''}`}
              data-theme-btn='light'
              aria-pressed={theme === 'light'}
              onClick={() => apply('light')}>
              <SunIcon />
              Light
            </button>
            <button
              type='button'
              className={`seg-btn ${theme === 'dark' ? 'is-on' : ''}`}
              data-theme-btn='dark'
              aria-pressed={theme === 'dark'}
              onClick={() => apply('dark')}>
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
              <span className='dim'>Go</span>
              <span className='dim'>Window</span>
              <span className='dim'>Help</span>
              <span className='shot-clock'>Fri 9:41 AM</span>
            </div>
            <div
              className='ghost-win'
              aria-hidden='true'>
              <div className='gw-chrome'>
                <i />
                <i />
                <i />
                <span className='gw-url'>github.com/MarvisLLC/marvis</span>
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
                In <code>keys.enc</code> inside <code>~/.marvis</code> — sealed
                with AES-256-GCM and unlocked by Touch ID or your password. The
                UI only ever shows the masked form, like <code>…7B2q</code>, so
                a typed key is never echoed back on screen.
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
          <div
            className='stack'
            style={{ maxWidth: '34ch' }}>
            <div>
              <p className='eyebrow'>Settings, 240px wide</p>
              <h3>Provider keys, masked.</h3>
            </div>
            <p style={mutedBody}>
              Keys are validated against the provider before they&rsquo;re
              stored, then only ever shown as{' '}
              <span className='num'>…last4</span>. Ollama needs no key — it
              talks to the local daemon. Deepgram lands with Listen in Phase 2.
            </p>
            <p
              className='meta'
              style={{ margin: 0 }}>
              Save clears the input · Lock keys tears down capture
            </p>
          </div>
          <figure
            className='shot'
            data-od-id='shot-settings'>
            <div className='mv-stage-center'>
              <MvSettings />
            </div>
            <figcaption className='shot-cap'>
              <span>settings · 240px wide · ≤400px tall, scrolls</span>
              <span>anchors to the bar&rsquo;s right edge</span>
            </figcaption>
          </figure>
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
