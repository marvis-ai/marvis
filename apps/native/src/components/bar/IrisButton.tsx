import { ArrowLeftIcon } from '@marvis/ui';
import { cn } from '@/lib/classes';
import { Iris } from '@/components/Iris';
import { BAR_BTN } from '@/components/bar/BarButton';

/** The capsule's leftmost control — the iris at rest, folding behind a
 *  back arrow while the bar shows its input row (`data-expanded` on
 *  `group/bar` drives the swap). A live Listen/dictation session lights
 *  it via `listen-active`. */
export const IrisButton = ({
  active,
  label,
  disabled,
  onPress,
}: {
  active: boolean;
  label: string;
  disabled?: boolean;
  onPress: () => void;
}) => (
  <button
    type='button'
    className={cn(BAR_BTN, 'relative', active && 'listen-active')}
    aria-label={label}
    onClick={onPress}
    disabled={disabled}>
    <Iris />
    <span className='pointer-events-none absolute inset-0 grid -rotate-90 scale-[0.4] place-items-center opacity-0 transition-[rotate_var(--motion-base)_var(--ease)_55ms,scale_var(--motion-base)_var(--ease)_55ms,opacity_var(--motion-fast)_var(--ease)_55ms] group-data-expanded/bar:rotate-none group-data-expanded/bar:scale-100 group-data-expanded/bar:opacity-100 motion-reduce:transition-none'>
      <ArrowLeftIcon className='size-5.5' />
    </span>
  </button>
);
