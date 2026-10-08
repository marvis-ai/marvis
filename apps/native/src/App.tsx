import { Suspense, lazy } from 'react';
import Bar from './views/Bar';

const AlertToast = lazy(() => import('./views/AlertToast'));
const Picker = lazy(() => import('./views/Picker'));
const Palette = lazy(() => import('./views/Palette'));
const Prefs = lazy(() => import('./views/Prefs'));

/**
 * One webview bundle serves every window. `windows/mod.rs` builds each
 * `WebviewWindow` with `index.html?view=<label>` (`"bar"`, `"alert"`,
 * `"picker"`, `"palette"`, `"prefs"`) — the query picks the view;
 * anything missing/unknown falls
 * back to Bar, the always-present unified window (chat/listen are its
 * card modes now, not separate windows).
 *
 * The non-Bar views are lazy so each window only parses its own chunk —
 * the alert toast shouldn't pay for the settings tree (or vice versa).
 */
const pickView = (view: string | null) => {
  switch (view) {
    case 'alert':
      return <AlertToast />;
    case 'picker':
      return <Picker />;
    case 'palette':
      return <Palette />;
    case 'prefs':
      return <Prefs />;
    default:
      return <Bar />;
  }
};

const App = () => {
  const view = new URLSearchParams(window.location.search).get('view');
  return <Suspense fallback={null}>{pickView(view)}</Suspense>;
};

export default App;
