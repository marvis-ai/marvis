export type AskActivity = 'idle' | 'loading' | 'streaming';
export type SpeechActivity = 'idle' | 'listening' | 'paused' | 'error';

/** The bar row's control set per surface — collapsed shows the
 *  background/Listen recorders, expanded shows dictation + settings.
 *  Pure so the layout contract is testable without a Tauri window. */
export type BarControl =
  | 'iris'
  | 'capture'
  | 'listen'
  | 'dictation'
  | 'settings';

export const barControls = (expanded: boolean): BarControl[] =>
  expanded ? ['iris', 'dictation', 'settings'] : ['iris', 'capture', 'listen'];

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
