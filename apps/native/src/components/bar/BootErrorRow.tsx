import { cn } from '@/lib/classes';
import { RetryCard } from '@/components/RetryCard';
import { Grip } from '@/components/bar/Grip';

/** The bootstrap-failure row — a centered RetryCard in place of the
 *  normal controls. */
export const BootErrorRow = ({
  className,
  onRetry,
}: {
  className?: string;
  onRetry: () => void;
}) => (
  <div
    className={cn(className, 'justify-center')}
    data-tauri-drag-region>
    <Grip />
    <RetryCard onRetry={onRetry} />
  </div>
);
