/**
 * `?view=listen` — the ambient-transcript panel. Listen is Phase 2
 * (no `listen_*` commands exist yet), so this renders the spec's
 * waveform idiom muted and still: motion is state, and a dead waveform
 * animating would lie about being live.
 */
const ListenPanel = () => {
  return (
    <div className='p-1'>
      <div className='mv-panel items-center justify-center gap-2 px-4 py-6'>
        <span
          className='mv-wave'
          aria-hidden>
          <i />
          <i />
          <i />
          <i />
          <i />
        </span>
        <p className='mv-empty'>Listen arrives in Phase 2</p>
        <span className='mv-chip'>deepgram · stt</span>
      </div>
    </div>
  );
};

export default ListenPanel;
