import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { surfaceMaterial } from './lib/commands';
import { initTheme } from './lib/theme';
import './index.css';

// Appearance: `app.appearance` in config.toml (auto|light|dark); `auto`
// follows macOS. initTheme applies `is-dark`/`dark` and tracks
// `config:changed` + the system scheme.
initTheme();

// Native material marker: 'glass' | 'vibrancy' | 'none'. Under a real
// material the CSS frost is stripped (double-frosting looks muddy);
// 'none' — browser dev, non-macOS — keeps it.
void surfaceMaterial()
  .then((m) => {
    document.documentElement.dataset.material = m;
  })
  .catch(() => {});

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
