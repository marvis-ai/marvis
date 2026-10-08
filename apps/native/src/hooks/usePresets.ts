/**
 * The merged preset list (built-ins + customs) — fetched on mount and
 * refetched on `config:changed` (a custom may be added/renamed/removed
 * from Settings). Shared by the composer's `/`-shorthand matcher +
 * palette seed (`Bar`), the `· name` provenance resolver
 * (`ChatSection`), and the Settings preset list (`PresetsTab`).
 */
import { useEffect, useRef, useState } from 'react';
import { presetsList, type Config, type Preset } from '@/lib/commands';
import { EV_CONFIG_CHANGED, useTauriEvent } from '@/lib/events';

export const usePresets = (): Preset[] => {
  const [presets, setPresets] = useState<Preset[]>([]);
  const requestSequence = useRef(0);
  const refresh = () => {
    const sequence = ++requestSequence.current;
    void presetsList()
      .then((list) => {
        if (sequence === requestSequence.current) setPresets(list);
      })
      .catch(() => {});
  };
  // Mount fetch covers the first render; `config:changed` drives every
  // later write.
  useEffect(() => {
    refresh();
    return () => {
      ++requestSequence.current;
    };
  }, []);
  useTauriEvent<Config>(EV_CONFIG_CHANGED, refresh);
  return presets;
};
