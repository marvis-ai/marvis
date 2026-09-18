/**
 * The 353×47 always-on-top bar (`?view=bar`). The window is transparent,
 * frameless and non-resizable, so every state renders inside the same
 * compact single-row pill — gate states can't grow the window, and
 * `window_adjust_height` is panel-only (windows/mod.rs).
 *
 * `data-tauri-drag-region` goes on the container chrome and
 * non-interactive children only: Tauri's drag walk treats bare
 * attributes as "direct clicks drag" and clickable descendants
 * (input/button) block it, so inputs and buttons stay usable.
 */
import { useCallback, useEffect, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { Button, Mic, Settings, ShieldAlert } from '@marvis/ui';
import {
  askSend,
  keystoreInit,
  keystoreReset,
  keystoreStatus,
  keystoreUnlock,
  permissionsOpenPrefs,
  permissionsRequestScreen,
  permissionsStatus,
  windowShowSettings,
  type AppStatePayload,
  type Gate,
  type KeystoreStatus,
  type PermissionsStatus,
} from '../lib/commands';
import {
  EV_APP_STATE,
  EV_CAPTURE_PERMISSION_NEEDED,
  EV_KEYSTORE_CHANGED,
  useTauriEvent,
} from '../lib/events';
import { LogoMark } from '../components/LogoMark';
import { RetryCard } from '../components/RetryCard';

/** Mirrors `app_gate` in lib.rs so the first render doesn't wait on `app:state`. */
function gateFor(ks: KeystoreStatus, perms: PermissionsStatus): Gate {
  if (ks.state !== 'Unlocked') {
    return 'needs_unlock';
  }
  return perms.screen ? 'main' : 'needs_permission';
}

/** Translucent pill chrome shared by every bar state. */
function Shell({ children }: { children: ReactNode }) {
  return (
    <div
      className='h-full p-1'
      data-tauri-drag-region>
      <div
        data-tauri-drag-region='deep'
        className='flex h-full flex-col justify-center gap-0.5 rounded-full border border-border bg-card/80 px-3 shadow-lg backdrop-blur select-none'>
        {children}
      </div>
    </div>
  );
}

export default function Bar() {
  const [gate, setGate] = useState<Gate | null>(null);
  const [keystore, setKeystore] = useState<KeystoreStatus | null>(null);
  const [bootError, setBootError] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [text, setText] = useState('');

  const bootstrap = useCallback(async () => {
    try {
      const [ks, perms] = await Promise.all([
        keystoreStatus(),
        permissionsStatus(),
      ]);
      setKeystore(ks);
      setGate(gateFor(ks, perms));
      setBootError(false);
    } catch {
      setBootError(true);
    }
  }, []);

  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  useTauriEvent<AppStatePayload>(EV_APP_STATE, (p) => {
    setGate(p.gate);
    setError(null);
  });
  useTauriEvent<KeystoreStatus>(EV_KEYSTORE_CHANGED, setKeystore);
  // Mid-session screen-permission revocation (ask.rs detects it when a
  // stale frame would have shipped): flip back to the permission card.
  useTauriEvent<{ permission: string }>(EV_CAPTURE_PERMISSION_NEEDED, () =>
    setGate('needs_permission'),
  );

  const unlockKeystore = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      // First run creates the Keychain DEK silently, then the uniform
      // unlock path shows the one system-auth prompt.
      const ks =
        keystore?.state === 'Unset'
          ? await keystoreInit().then(() => keystoreUnlock())
          : await keystoreUnlock();
      setKeystore(ks);
      // `transition_gate` emits `app:state`, but resync anyway in case
      // the emit raced us.
      await bootstrap();
    } catch (err) {
      setError(typeof err === 'string' ? err : 'Unlock failed');
    } finally {
      setBusy(false);
    }
  };

  const grantScreen = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await permissionsRequestScreen();
      await bootstrap();
    } catch {
      setError('Permission request failed');
    } finally {
      setBusy(false);
    }
  };

  const submitAsk = (e: FormEvent) => {
    e.preventDefault();
    const t = text.trim();
    if (!t) {
      return;
    }
    setText('');
    void askSend(t).catch(() => setError('Send failed'));
  };

  if (bootError) {
    return (
      <Shell>
        <RetryCard onRetry={() => void bootstrap()} />
      </Shell>
    );
  }

  if (gate === null) {
    return (
      <Shell>
        <div
          className='flex items-center gap-2'
          data-tauri-drag-region>
          <LogoMark className='size-4 shrink-0 text-foreground' />
          <span
            className='text-xs text-muted-foreground'
            data-tauri-drag-region>
            Loading…
          </span>
        </div>
      </Shell>
    );
  }

  if (gate === 'needs_unlock') {
    return (
      <Shell>
        <div
          className='flex items-center gap-2'
          data-tauri-drag-region>
          <LogoMark className='size-4 shrink-0 text-foreground' />
          <span
            className='min-w-0 flex-1 truncate text-xs text-muted-foreground'
            title='Marvis unlocks with Touch ID, Face ID, or your Mac password'
            data-tauri-drag-region>
            Unlock with Touch ID / password
          </span>
          <Button
            size='xs'
            onClick={() => void unlockKeystore()}
            disabled={busy}>
            Unlock
          </Button>
        </div>
        {error && (
          <div className='flex items-center justify-center gap-1.5'>
            <p className='truncate text-[10px] leading-3 text-destructive'>
              {error}
            </p>
            {!error.includes('cancel') && (
              <button
                type='button'
                className='shrink-0 text-[10px] leading-3 text-muted-foreground underline hover:text-foreground'
                onClick={() => {
                  // Obsolete/corrupt store or a lost DEK — the only way
                  // forward. Deletes keys.enc + the keychain item(s).
                  setError(null);
                  void keystoreReset()
                    .then(setKeystore)
                    .catch(() => setError('Reset failed'));
                }}>
                Reset
              </button>
            )}
          </div>
        )}
      </Shell>
    );
  }

  if (gate === 'needs_permission') {
    return (
      <Shell>
        <div
          className='flex items-center gap-2'
          data-tauri-drag-region>
          <ShieldAlert
            className='size-4 shrink-0 text-muted-foreground'
            data-tauri-drag-region
          />
          <span
            className='min-w-0 flex-1 truncate text-xs text-muted-foreground'
            title='Marvis needs screen recording to see your screen'
            data-tauri-drag-region>
            Screen recording needed
          </span>
          <Button
            size='xs'
            onClick={() => void grantScreen()}
            disabled={busy}>
            Grant
          </Button>
          <Button
            size='xs'
            variant='link'
            onClick={() => void permissionsOpenPrefs('Privacy_ScreenCapture')}>
            Open settings
          </Button>
        </div>
        {error && (
          <p className='truncate text-center text-[10px] leading-3 text-destructive'>
            {error}
          </p>
        )}
      </Shell>
    );
  }

  // gate === "main"
  return (
    <Shell>
      <form
        onSubmit={submitAsk}
        className='flex items-center gap-1.5'
        data-tauri-drag-region>
        <LogoMark className='size-4 shrink-0 text-foreground' />
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder='Ask Marvis…'
          className='min-w-0 flex-1 select-text bg-transparent px-1 text-sm text-foreground outline-none placeholder:text-muted-foreground'
        />
        <Button
          type='button'
          size='icon-xs'
          variant='ghost'
          disabled
          title='Coming soon'>
          <Mic />
        </Button>
        <Button
          type='button'
          size='icon-xs'
          variant='ghost'
          title='Settings'
          onClick={() => void windowShowSettings()}>
          <Settings />
        </Button>
      </form>
      {error && (
        <p className='truncate text-center text-[10px] leading-3 text-destructive'>
          {error}
        </p>
      )}
    </Shell>
  );
}
