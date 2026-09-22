/**
 * The card's listen section — the Phase-2 waveform stub, unchanged
 * visually from the retired `?view=listen` panel: a dead waveform
 * animating would lie about being live, so it renders muted and still.
 */
import { CHIP, EMPTY } from '../lib/classes';

export const ListenSection = () => {
  return (
    <div className='flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-4 py-6'>
      <span
        className='flex h-4.5 items-center gap-0.75'
        aria-hidden>
        <i className='h-2 w-0.75 rounded-xs bg-muted' />
        <i className='h-3.5 w-0.75 rounded-xs bg-muted' />
        <i className='h-4.5 w-0.75 rounded-xs bg-muted' />
        <i className='h-3 w-0.75 rounded-xs bg-muted' />
        <i className='h-1.75 w-0.75 rounded-xs bg-muted' />
      </span>
      <p className={EMPTY}>Listen arrives in Phase 2</p>
      <span className={CHIP}>deepgram · stt</span>
    </div>
  );
};
