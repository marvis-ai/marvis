/**
 * Bootstrap-failure fallback — a transparent frameless window must never
 * render blank, so every view shows this when its initial command load
 * rejects.
 */
export const RetryCard = ({
  onRetry,
  message = 'Failed to load',
}: {
  onRetry: () => void;
  message?: string;
}) => {
  return (
    <div
      className='flex items-center justify-center gap-2'
      data-tauri-drag-region>
      <span className='text-xs text-destructive'>{message}</span>
      <button
        type='button'
        className='mv-btn mv-btn-outline'
        onClick={onRetry}>
        Retry
      </button>
    </div>
  );
};
