export type AskActivity = 'idle' | 'loading' | 'streaming';
export type SpeechActivity = 'idle' | 'listening' | 'error';

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
