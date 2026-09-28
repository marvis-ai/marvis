import Bar from './views/Bar';
import AlertToast from './views/AlertToast';
import Picker from './views/Picker';
import Prefs from './views/Prefs';

/**
 * One webview bundle serves every window. `windows/mod.rs` builds each
 * `WebviewWindow` with `index.html?view=<label>` (`"bar"`, `"alert"`,
 * `"picker"`, `"prefs"`) — the query picks the view; anything
 * missing/unknown falls
 * back to Bar, the always-present unified window (chat/listen are its
 * card modes now, not separate windows).
 */
export default function App() {
  const view = new URLSearchParams(window.location.search).get('view');
  switch (view) {
    case 'alert':
      return <AlertToast />;
    case 'picker':
      return <Picker />;
    case 'prefs':
      return <Prefs />;
    default:
      return <Bar />;
  }
}
