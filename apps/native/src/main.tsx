import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './index.css';

// Overlay appearance follows macOS (DESIGN.md: Appearance defaults to
// Auto). The token layer flips via `prefers-color-scheme`; the `.dark`
// class only keeps the ui package's `dark:` utilities in sync.
const scheme = window.matchMedia('(prefers-color-scheme: dark)');
const syncScheme = () =>
  document.documentElement.classList.toggle('dark', scheme.matches);
syncScheme();
scheme.addEventListener('change', syncScheme);

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
