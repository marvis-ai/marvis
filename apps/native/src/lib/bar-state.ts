export type AskActivity = 'idle' | 'loading' | 'streaming';
export type SpeechActivity = 'idle' | 'listening' | 'paused' | 'error';

/** The bar row's control set per surface — collapsed shows the
 *  background/Listen recorders + the History opener, expanded shows
 *  dictation, and an open card's input row carries dictation only
 *  (the section header owns Back/Close; Settings lives in the tray
 *  menu + Cmd/Ctrl+,).
 *  Pure so the layout contract is testable without a Tauri window. */
export type BarControl =
  | 'iris'
  | 'capture'
  | 'listen'
  | 'history'
  | 'dictation';

export const barControls = (
  expanded: boolean,
  cardOpen = false,
): BarControl[] =>
  cardOpen
    ? ['dictation']
    : expanded
      ? ['iris', 'dictation']
      : ['iris', 'capture', 'listen', 'history'];

/** `captureRunning` is deliberately absent: capture starts by default
 *  in the main gate, so counting it would pulse the shell permanently
 *  — the capture toggle's ping badge carries that signal instead. */
export interface BarActivityState {
  ask: AskActivity;
  listen: SpeechActivity;
  dictation: SpeechActivity;
}

export const hasActiveWork = ({ ask, listen, dictation }: BarActivityState) =>
  ask === 'loading' ||
  ask === 'streaming' ||
  listen === 'listening' ||
  dictation === 'listening';
