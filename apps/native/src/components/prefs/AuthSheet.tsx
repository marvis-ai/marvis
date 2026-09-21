/**
 * The "Touch ID" sheet (DESIGN.md §6): a frosted card on a dim scrim
 * over the whole window, shown while a keystore command is in flight —
 * macOS draws the real biometric prompt itself, so this is the in-window
 * "waiting for system auth" affordance, not a fake prompt. Slides in on
 * `--motion-base`; hidden state is `visibility`ed so it can't be focused.
 */
import { Fingerprint } from '@marvis/ui';

export const AuthSheet = ({
  open,
  title = 'Touch ID to unlock Marvis',
  sub = 'keys.enc · Keychain DEK',
}: {
  open: boolean;
  title?: string;
  sub?: string;
}) => (
  <div
    className={`prf-auth${open ? ' is-open' : ''}`}
    aria-hidden={!open}>
    <div className='prf-auth-card'>
      <Fingerprint />
      <div className='t-title'>{title}</div>
      <div className='t-sub'>{sub}</div>
    </div>
  </div>
);
