/**
 * The merged preset list (built-ins + customs) — fetched on mount and
 * refetched on `config:changed` (a custom may be added/renamed/removed
 * from Settings). Shared by the composer's `/`-shorthand matcher +
 * palette seed (`Bar`), the `· name` provenance resolver
 * (`ChatSection`), and the Settings preset list (`PresetsTab`).
 */
import { useEffect, useState } from 'react';
import { presetsList, type Config, type Preset } from '@/lib/commands';
import { EV_CONFIG_CHANGED, useTauriEvent } from '@/lib/events';

export const usePresets = (): Preset[] => {
  const [presets, setPresets] = useState<Preset[]>([]);
  const refresh = () => {
    void presetsList()
      .then(setPresets)
      .catch(() => {});
  };
  // Mount fetch covers the first render; `config:changed` drives every
  // later write.
  useEffect(refresh, []);
  useTauriEvent<Config>(EV_CONFIG_CHANGED, refresh);
  return presets;
};
