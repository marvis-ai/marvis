import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { initTheme } from './lib/theme';
import './index.css';

// Appearance: `app.appearance` in config.toml (auto|light|dark); `auto`
// follows macOS. initTheme applies `is-dark`/`dark` and tracks
// `config:changed` + the system scheme.
initTheme();

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
