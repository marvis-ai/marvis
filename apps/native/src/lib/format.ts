/**
 * Render a `hotkeys.<action>` accelerator (`"Cmd+Shift+Up"`) as macOS
 * glyph tokens (`["⌘","⇧","↑"]`) for `.kbd` chips. Unknown tokens render
 * verbatim — the config file is user-editable, so the table must degrade
 * rather than drop a binding it doesn't recognize.
 */
const GLYPHS: Record<string, string> = {
  Cmd: '⌘',
  Command: '⌘',
  Shift: '⇧',
  Alt: '⌥',
  Option: '⌥',
  Ctrl: '⌃',
  Control: '⌃',
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
