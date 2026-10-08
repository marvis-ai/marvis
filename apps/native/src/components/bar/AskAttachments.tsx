import { XIcon } from '@marvis/ui';
import { cn } from '@/lib/classes';
import type { PendingAskImage } from '@/lib/image-attachments';

/** The composer's pending-attachment strip — one thumbnail chip per
 *  normalized image (`previewUrl` is the same JPEG that `askSend`
 *  ships, so the chip is a true WYSIWYG preview) plus the latest
 *  validation error. Two placements: the default wraps as a row above
 *  the composer (card mode); `inline` collapses to a nowrap,
 *  scrollable sliver inside the fixed-height pill row.
 *  Stateless — `Bar` owns the pending list. */
export const AskAttachments = ({
  images,
  error,
  onRemove,
  inline = false,
}: {
  images: PendingAskImage[];
  /** Latest add/drop rejection; `''`/`null` renders no error line. */
  error: string | null;
  onRemove: (index: number) => void;
  /** Pill mode — a horizontal scroll sliver inside the composer row. */
  inline?: boolean;
}) => {
  if (images.length === 0 && !error) return null;
  return (
    <div
      className={cn(
        'flex items-center gap-1.5',
        inline
          ? 'max-w-40 flex-none flex-nowrap overflow-x-auto scrollbar-none [&::-webkit-scrollbar]:hidden'
          : 'flex-wrap px-2.75 pt-2',
      )}>
      {error ? (
        <p
          className={cn(
            'text-[10px] leading-tight text-destructive',
            inline ? 'truncate whitespace-nowrap' : 'w-full',
          )}>
          {error}
        </p>
      ) : null}
      {images.map((image, index) => (
        <span
          key={image.previewUrl}
          className='flex flex-none items-center gap-1 rounded-md border border-border bg-surface py-0.5 pr-1 pl-0.5'>
          <img
            src={image.previewUrl}
            alt={image.name}
            className='size-5 flex-none rounded-[4px] object-cover'
          />
          <span
            title={image.name}
            className='max-w-16 truncate text-[10px] text-muted-foreground'>
            {image.name}
          </span>
          <button
            type='button'
            aria-label={`Remove ${image.name}`}
            title={`Remove ${image.name}`}
            onClick={() => onRemove(index)}
            className={cn(
              'grid size-3.5 flex-none cursor-pointer place-items-center',
              'rounded-full border-0 bg-transparent p-0 text-muted-foreground',
              'transition-colors duration-(--motion-fast)',
              'enabled:hover:bg-fg-soft enabled:hover:text-foreground',
              'focus-visible:outline-2 focus-visible:outline-accent',
            )}>
            <XIcon aria-hidden className='size-2.5' />
          </button>
        </span>
      ))}
    </div>
  );
};
