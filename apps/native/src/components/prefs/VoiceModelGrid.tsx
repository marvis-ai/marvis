import {
  BTN_DANGER,
  BTN_LG,
  BTN_LINK_LG,
  BTN_OUTLINE,
  BTN_PRIMARY,
  NUM,
  PROV_NOTE,
  cn,
} from '../../lib/classes';

export interface VoiceModelEntry {
  id: string;
  label: string;
  description: string;
  bytes: number;
  source: string;
}

export interface VoiceModelGridProps {
  entries: VoiceModelEntry[];
  isInstalled: (id: string) => boolean;
  isSelected: (id: string) => boolean;
  progress: Record<string, { received: number; total: number }>;
  activeDownload: string | null;
  onDownload: (id: string) => void;
  onCancel: () => void;
  onSelect: (id: string) => void;
  onRemove: (id: string) => void;
}

const formatBytes = (bytes: number) => {
  if (bytes >= 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  return `${Math.round(bytes / (1024 * 1024))} MB`;
};

export const VoiceModelGrid = ({
  entries,
  isInstalled,
  isSelected,
  progress,
  activeDownload,
  onDownload,
  onCancel,
  onSelect,
  onRemove,
}: VoiceModelGridProps) => (
  <div className='mt-2.5 grid gap-2'>
    {entries.map((entry) => {
      const installed = isInstalled(entry.id);
      const selected = isSelected(entry.id);
      const currentProgress = progress[entry.id] ?? null;
      const downloading = activeDownload === entry.id;
      return (
        <div
          key={entry.id}
          className={cn(
            'rounded-lg border p-2.5',
            selected ? 'border-accent' : 'border-border',
          )}>
          <div className='flex items-start justify-between gap-2'>
            <div>
              <div className='text-[12.5px] font-semibold'>
                {entry.label}
                {selected && (
                  <span className='ml-1.5 text-[10px] text-accent-text'>
                    Selected
                  </span>
                )}
              </div>
              <p className={PROV_NOTE}>{entry.description}</p>
            </div>
            <span
              className={cn(NUM, 'text-[10.5px] text-muted-foreground')}>
              {formatBytes(entry.bytes)}
            </span>
          </div>
          <p className={PROV_NOTE}>
            {entry.source} {installed ? 'Installed' : 'Not installed'}
          </p>
          {currentProgress && (
            <div className='mt-2'>
              <div className='flex justify-between text-[10px] text-muted-foreground'>
                <span>Downloading…</span>
                <span className={NUM}>
                  {formatBytes(currentProgress.received)} /{' '}
                  {formatBytes(currentProgress.total)}
                </span>
              </div>
              <progress
                className='mt-1 h-1.5 w-full accent-accent'
                value={currentProgress.received}
                max={currentProgress.total}
              />
            </div>
          )}
          <div className='mt-2 flex flex-wrap gap-1.5'>
            {downloading ? (
              <button
                type='button'
                className={cn(BTN_LG, BTN_OUTLINE)}
                onClick={() => onCancel()}>
                Cancel
              </button>
            ) : !installed ? (
              <button
                type='button'
                className={cn(BTN_LG, BTN_PRIMARY)}
                disabled={Boolean(activeDownload)}
                onClick={() => onDownload(entry.id)}>
                Download
              </button>
            ) : (
              <>
                <button
                  type='button'
                  className={cn(BTN_LG, BTN_PRIMARY)}
                  disabled={selected}
                  onClick={() => onSelect(entry.id)}>
                  {selected ? 'Using this model' : 'Use this model'}
                </button>
                <button
                  type='button'
                  className={cn(BTN_LINK_LG, BTN_DANGER)}
                  disabled={selected}
                  title={
                    selected ? 'The active model cannot be removed' : undefined
                  }
                  onClick={() => onRemove(entry.id)}>
                  Remove
                </button>
              </>
            )}
          </div>
        </div>
      );
    })}
  </div>
);
