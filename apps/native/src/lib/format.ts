/**
 * Render a `hotkeys.<action>` accelerator (`"Cmd+Shift+Up"`) as macOS
 * glyph tokens (`["⌘","⇧","↑"]`) for `.kbd` chips. Unknown tokens render
 * verbatim — the config file is user-editable, so the table must degrade
 * rather than drop a binding it doesn't recognize.
 */
/** `CmdOrCtrl` resolves per platform — ⌘ on macOS, Ctrl elsewhere —
 *  matching the expansion in `hotkey.rs`. `Ctrl` likewise renders as
 *  text off macOS, where ⌃ is an Apple-only convention. */
const IS_MAC = navigator.userAgent.includes('Mac');

const GLYPHS: Record<string, string> = {
  Cmd: '⌘',
  Command: '⌘',
  CmdOrCtrl: IS_MAC ? '⌘' : 'Ctrl',
  Shift: '⇧',
  Alt: '⌥',
  Option: '⌥',
  Ctrl: IS_MAC ? '⌃' : 'Ctrl',
  Control: IS_MAC ? '⌃' : 'Ctrl',
  Enter: '⏎',
  Return: '⏎',
  Up: '↑',
  Down: '↓',
  Left: '←',
  Right: '→',
  Slash: '/',
  Comma: ',',
  Space: 'Space',
};

export const kbdTokens = (accel: string): string[] =>
  accel
    .split('+')
    .map((t) => t.trim())
    .filter(Boolean)
    .map((t) => GLYPHS[t] ?? t);
