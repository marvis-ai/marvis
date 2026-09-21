/**
 * The dark vault card at the top of Providers — `keys.enc` state and its
 * one action. Unset → create (silent first run), Locked → unlock (system
 * auth prompt; the AuthSheet covers the window while it's up), Unlocked
 * → lock. Errors surface inline under the card.
 */
import { useState } from 'react';
import { Lock } from '@marvis/ui';
import {
  keystoreInit,
  keystoreLock,
  keystoreUnlock,
  type KeystoreStatus,
} from '../../lib/commands';
import { AuthSheet } from './AuthSheet';

export const VaultCard = ({
  status,
  setStatus,
}: {
  status: KeystoreStatus | null;
  setStatus: (s: KeystoreStatus) => void;
}) => {
  const [pending, setPending] = useState(false);
  const [err, setErr] = useState('');

  const state = status?.state ?? 'Unset';
  const unlocked = state === 'Unlocked';

  const run = async (op: () => Promise<KeystoreStatus>) => {
    setErr('');
    setPending(true);
    try {
      setStatus(await op());
    } catch (e) {
      setErr(typeof e === 'string' ? e : 'Vault operation failed');
    } finally {
      setPending(false);
    }
  };

  // First run: init creates keys.enc + the Keychain DEK silently, then
  // unlock's read-back is the one system-auth prompt.
  const primary =
    state === 'Unset'
      ? () => run(keystoreInit)
      : unlocked
        ? () => run(keystoreLock)
        : () => run(keystoreUnlock);

  return (
    <>
      <div
        className='prf-vault'
        style={unlocked ? undefined : { opacity: 0.75 }}>
        <span className='prf-vault-ico'>
          <Lock />
        </span>
        <div>
          <div className='v-name'>
            {state === 'Unset'
              ? 'No key vault yet'
              : unlocked
                ? 'Key vault unlocked'
                : 'Key vault locked'}
          </div>
          <div className='v-sub'>keys.enc · AES-256-GCM · DEK in Keychain, Touch ID</div>
          {err && <div className='v-err'>{err}</div>}
        </div>
        <button
          type='button'
          className='mv-btn mv-btn-outline v-ctl'
          disabled={pending}
          onClick={() => void primary()}>
          {pending
            ? 'Waiting…'
            : state === 'Unset'
              ? 'Create vault'
              : unlocked
                ? 'Lock keys'
                : 'Unlock'}
        </button>
      </div>
      <AuthSheet
        open={pending && state !== 'Unset'}
        title={
          unlocked ? 'Touch ID to lock Marvis' : 'Touch ID to unlock Marvis'
        }
      />
    </>
  );
};
