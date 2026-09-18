/**
 * `?view=listen` — meeting listen panel. Phase 2 fills this in; for now
 * it renders the translucent panel chrome so the window isn't blank.
 */
import { useEffect } from 'react';
import { Mic } from '@marvis/ui';

export default function ListenPanel() {
  useEffect(() => {
    document.body.classList.add('listen');
    return () => document.body.classList.remove('listen');
  }, []);

  return (
    <div className='h-full p-1'>
      <div className='flex h-full flex-col items-center justify-center gap-1.5 rounded-2xl border border-border bg-card/90 text-xs text-muted-foreground shadow-lg backdrop-blur'>
        <Mic className='size-4' />
        <span>Listen arrives in Phase 2</span>
      </div>
    </div>
  );
}
