import { Button } from "@marvis/ui";

/**
 * Bootstrap-failure fallback — a transparent frameless window must never
 * render blank, so every view shows this when its initial command load
 * rejects.
 */
export function RetryCard({
  onRetry,
  message = "Failed to load",
}: {
  onRetry: () => void;
  message?: string;
}) {
  return (
    <div
      className="flex items-center justify-center gap-2"
      data-tauri-drag-region
    >
      <span className="text-xs text-destructive">{message}</span>
      <Button size="xs" variant="outline" onClick={onRetry}>
        Retry
      </Button>
    </div>
  );
}
