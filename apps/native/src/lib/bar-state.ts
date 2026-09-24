export type AskActivity = 'idle' | 'loading' | 'streaming';
export type SpeechActivity = 'idle' | 'listening' | 'error';

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

export interface BarActivityState {
  ask: AskActivity;
  captureRunning: boolean;
  listen: SpeechActivity;
  dictation: SpeechActivity;
}

export const hasActiveWork = ({
  ask,
  captureRunning,
  listen,
  dictation,
}: BarActivityState) =>
  ask === 'loading' ||
  ask === 'streaming' ||
  captureRunning ||
  listen === 'listening' ||
  dictation === 'listening';
