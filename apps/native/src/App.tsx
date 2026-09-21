import Bar from './views/Bar';
import AlertToast from './views/AlertToast';
import AskPanel from './views/AskPanel';
import ListenPanel from './views/ListenPanel';
import Prefs from './views/Prefs';

/**
 * One webview bundle serves every window. `windows/mod.rs` builds each
 * `WebviewWindow` with `index.html?view=<label>` (`"bar"`, `"alert"`,
 * `"ask"`, `"listen"`, `"prefs"`) — the query picks the view; anything
 * missing/unknown falls back to Bar, the always-present window.
 */
export default function App() {
  const view = new URLSearchParams(window.location.search).get('view');
  switch (view) {
    case 'alert':
      return <AlertToast />;
    case 'ask':
      return <AskPanel />;
    case 'listen':
      return <ListenPanel />;
    case 'prefs':
      return <Prefs />;
    default:
      return <Bar />;
  }
}
