import { ShieldAlertIcon } from '@marvis/ui';
import { permissionsOpenPrefs } from '@/lib/commands';
import { BTN_LINK, BTN_LINK_SM, BTN_PRIMARY, BTN_SM, cn } from '@/lib/classes';
import { Grip } from '@/components/bar/Grip';

/** The `needs_permission` gate row — screen recording is the bar's
 *  only hard requirement. */
export const PermissionRow = ({
  className,
  busy,
  onGrant,
}: {
  className?: string;
  busy: boolean;
  onGrant: () => void;
}) => (
  <div
    className={className}
    data-tauri-drag-region>
    <Grip />
    <ShieldAlertIcon
      className='size-4.5 flex-none text-muted-foreground'
      data-tauri-drag-region
    />
    <span
      className='min-w-0 flex-1 truncate text-xs text-muted-foreground'
      title='Marvis needs screen recording to see your screen'
      data-tauri-drag-region>
      Screen recording needed
    </span>
    <button
      type='button'
      className={cn(BTN_SM, BTN_PRIMARY)}
      onClick={onGrant}
      disabled={busy}>
      Grant
    </button>
    <button
      type='button'
      className={cn(BTN_LINK_SM, BTN_LINK)}
      onClick={() => void permissionsOpenPrefs('Privacy_ScreenCapture')}>
      Open settings
    </button>
  </div>
);
