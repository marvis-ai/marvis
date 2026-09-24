import { cn } from '@/lib/classes';

/** Compact dictation waveform — reuses `--animate-waveform`; the
 *  reduced-motion query stills it without layout change. */
export const DictationWaveform = () => (
  <span
    className='flex h-3.5 items-center gap-0.5 text-accent'
    aria-hidden='true'>
    {['h-1', 'h-1.75', 'h-2.75', 'h-2', 'h-1.25'].map((height, index) => (
      <span
        key={height}
        className={cn(height, 'w-0.5 animate-waveform rounded-xs bg-current')}
        style={{ animationDelay: `${index * 90}ms` }}
      />
    ))}
  </span>
);
