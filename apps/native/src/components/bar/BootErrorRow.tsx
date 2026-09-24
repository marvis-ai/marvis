import { cn } from '@/lib/classes';
import { RetryCard } from '@/components/RetryCard';

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
    <RetryCard onRetry={onRetry} />
  </div>
);
