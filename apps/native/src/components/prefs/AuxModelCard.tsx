import {
  BTN_DANGER,
  BTN_LG,
  BTN_LINK_LG,
  BTN_OUTLINE,
  BTN_PRIMARY,
  LBL,
  NUM,
  PROV_CARD,
  PROV_NOTE,
  cn,
} from '@/lib/classes';

export interface AuxModelEntry {
  id: string;
  description: string;
  bytes: number;
  installed: boolean;
}

export interface AuxModelCardProps {
  entry: AuxModelEntry;
  /** Display title — the catalog label or a friendlier name like
   * "Speaker diarization". */
  title: string;
  /** Trailing sentence after the catalog description in the note line. */
  note: string;
  progress: { received: number; total: number } | null;
  /** id of the entry currently downloading — drives Cancel vs the
   * Download/Remove pair. */
  activeDownload: string | null;
  onDownload: (id: string) => void;
  onCancel: () => void;
  onRemove: (id: string) => void;
}

const formatBytes = (bytes: number) => {
  if (bytes >= 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  return `${Math.round(bytes / (1024 * 1024))} MB`;
};

/** Card for a non-STT sherpa add-on (punctuation, speaker embedding) —
 * status dot, byte size, optional progress, and the
 * Cancel/Download/Remove trio. */
export const AuxModelCard = ({
  entry,
  title,
  note,
  progress,
  activeDownload,
  onDownload,
  onCancel,
  onRemove,
}: AuxModelCardProps) => (
  <div className={cn(PROV_CARD, 'border-border')}>
    <div className='flex items-center gap-2'>
      <span
        className={cn(
          'size-1.75 flex-none rounded-full',
          entry.installed
            ? 'bg-accent'
            : 'bg-[color-mix(in_oklch,var(--fg)_20%,transparent)]',
        )}
      />
      <span className={LBL}>{title}</span>
      <span className={cn(NUM, 'ml-auto text-[10.5px] text-muted-foreground')}>
        {formatBytes(entry.bytes)}
      </span>
    </div>
    <p className={PROV_NOTE}>
      {entry.description} {note}
    </p>
    {progress && (
      <div className='mt-2'>
        <div className='flex justify-between text-[10px] text-muted-foreground'>
          <span>Downloading…</span>
          <span className={NUM}>
            {formatBytes(progress.received)} / {formatBytes(progress.total)}
          </span>
        </div>
        <progress
          className='mt-1 h-1.5 w-full accent-accent'
          value={progress.received}
          max={progress.total}
        />
      </div>
    )}
    <div className='mt-2 flex gap-1.5'>
      {activeDownload === entry.id ? (
        <button
          type='button'
          className={cn(BTN_LG, BTN_OUTLINE)}
          onClick={() => onCancel()}>
          Cancel
        </button>
      ) : !entry.installed ? (
        <button
          type='button'
          className={cn(BTN_LG, BTN_PRIMARY)}
          disabled={Boolean(activeDownload)}
          onClick={() => onDownload(entry.id)}>
          Download
        </button>
      ) : (
        <button
          type='button'
          className={cn(BTN_LINK_LG, BTN_DANGER)}
          onClick={() => onRemove(entry.id)}>
          Remove
        </button>
      )}
    </div>
  </div>
);
