/**
 * Appearance plumbing (DESIGN.md §6 — Auto / Light / Dark lives in
 * General). `app.appearance` is persisted in config.toml; `auto` follows
 * `prefers-color-scheme`. Every window runs `initTheme()` at boot and
 * re-applies on `config:changed`, so a flip in the prefs window
 * propagates to the bar and panels immediately.
 *
 * Two classes move together: `is-dark` drives the overlay token layer
 * (index.css), `dark` keeps the ui package's `dark:` utilities in sync
 * (its `@custom-variant` is class-scoped).
 *
 * `app.accent` overrides `--accent` on the root — every derived token
 * (`--primary`, `--accent-soft`, `--accent-text`, `--focus-ring`,
 * `--ring`) is a `color-mix`/`var()` off it, so one property re-hues
 * the whole UI.
 */
import { listen } from '@tauri-apps/api/event';
import { configGet, type Config } from './commands';

type Appearance = 'auto' | 'light' | 'dark';

/** Mirrors `config::DEFAULT_ACCENT` (`#3a7294`). */
const DEFAULT_ACCENT = '#3a7294';
const ACCENT_RE = /^#[0-9a-f]{6}$/i;

let appearance: Appearance = 'auto';
let accent = DEFAULT_ACCENT;
const scheme = window.matchMedia('(prefers-color-scheme: dark)');

const apply = () => {
  const dark =
    appearance === 'dark' || (appearance === 'auto' && scheme.matches);
  const root = document.documentElement;
  root.classList.toggle('is-dark', dark);
  root.classList.toggle('dark', dark);
  root.style.setProperty('--accent', accent);
};

const fromConfig = (cfg: Config) => {
  const next = cfg.app?.appearance;
  appearance = next === 'light' || next === 'dark' ? next : 'auto';
  const a = cfg.app?.accent;
  accent = ACCENT_RE.test(a ?? '') ? a : DEFAULT_ACCENT;
  apply();
};

export const initTheme = () => {
  apply();
  scheme.addEventListener('change', apply);
  // The backend is authoritative — read once so a non-default appearance
  // applies before the first paint settles, then track live writes.
  void configGet()
    .then(fromConfig)
    .catch(() => {});
  void listen<Config>('config:changed', (e) => fromConfig(e.payload));
};
