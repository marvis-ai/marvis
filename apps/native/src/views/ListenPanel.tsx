/**
 * `?view=listen` — the ambient-transcript panel. Listen is Phase 2
 * (no `listen_*` commands exist yet), so this renders the spec's
 * waveform idiom muted and still: motion is state, and a dead waveform
 * animating would lie about being live.
 */
import { CHIP, EMPTY, PANEL, cn } from '../lib/classes';

const ListenPanel = () => {
  return (
    <div className='p-1'>
      <div className={cn(PANEL, 'items-center justify-center gap-2 px-4 py-6')}>
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
    </div>
  );
};

export default ListenPanel;
