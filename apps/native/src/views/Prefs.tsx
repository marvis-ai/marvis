/**
 * `?view=prefs` — the decorated 720×520 settings + onboarding window
 * (`PREFS_LABEL` in windows/mod.rs). Native traffic lights, opaque, not
 * always-on-top: it's an app window, not overlay chrome. The webview
 * hosts both modes; the backend pushes `prefs:mode` on every
 * `show_prefs` and this view re-reads `prefs_mode` on mount, so a show
 * that raced the load still lands.
 *
 * Shared state (keystore status, config, model selection) loads once
 * here — both modes touch the same surface, and every mutation also
 * arrives via `keystore:changed` / `config:changed`.
 *
 * Mode switches remount the mode subtree (`key={mode}`), which is the
 * spec's "mode switch resets to step 1" — a re-run wizard starts at
 * welcome with fields re-initialized from live config.
 */
import { useCallback, useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  configGet,
  keystoreStatus,
  modelGetSelected,
  prefsMode,
  type Config,
  type KeystoreStatus,
  type ModelSelection,
} from '../lib/commands';
import {
  EV_CONFIG_CHANGED,
  EV_KEYSTORE_CHANGED,
  EV_PREFS_MODE,
  useTauriEvent,
} from '../lib/events';
import { SettingsMode } from '../components/prefs/SettingsMode';
import { Onboarding } from '../components/prefs/Onboarding';
import { RetryCard } from '../components/RetryCard';

type Mode = 'settings' | 'onboarding';

const TITLES: Record<Mode, string> = {
  settings: 'Marvis — Settings',
  onboarding: 'Marvis — Set up',
};

const Prefs = () => {
  const [mode, setMode] = useState<Mode | null>(null);
  const [status, setStatus] = useState<KeystoreStatus | null>(null);
  const [config, setConfig] = useState<Config | null>(null);
  const [selected, setSelected] = useState<ModelSelection | null>(null);
  const [bootError, setBootError] = useState(false);

  const bootstrap = useCallback(async () => {
    try {
      const [m, ks, cfg, sel] = await Promise.all([
        prefsMode(),
        keystoreStatus(),
        configGet(),
        modelGetSelected(),
      ]);
      setMode(m === 'onboarding' ? 'onboarding' : 'settings');
      setStatus(ks);
      setConfig(cfg);
      setSelected(sel);
      setBootError(false);
    } catch {
      setBootError(true);
    }
  }, []);

  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  useTauriEvent<{ mode: string }>(EV_PREFS_MODE, (p) =>
    setMode(p.mode === 'onboarding' ? 'onboarding' : 'settings'),
  );
  useTauriEvent<KeystoreStatus>(EV_KEYSTORE_CHANGED, setStatus);
  useTauriEvent<Config>(EV_CONFIG_CHANGED, setConfig);

  // The backend sets the title on show_prefs; this keeps it right for
  // in-window switches (sidebar "Re-run setup", wizard "Open settings").
  useEffect(() => {
    if (mode) {
      void getCurrentWindow()
        .setTitle(TITLES[mode])
        .catch(() => {});
    }
  }, [mode]);

  const data = {
    status,
    config,
    selected,
    setStatus,
    setConfig,
    setSelected,
  };

  return (
    <div className='prefs-shell relative flex h-full flex-col overflow-hidden bg-background text-foreground'>
      {bootError ? (
        <div className='grid flex-1 place-items-center p-5'>
          <RetryCard onRetry={() => void bootstrap()} />
        </div>
      ) : mode === 'onboarding' ? (
        <Onboarding
          key='onboarding'
          data={data}
        />
      ) : mode === 'settings' ? (
        <SettingsMode
          key='settings'
          data={data}
        />
      ) : null}
    </div>
  );
};

export default Prefs;
