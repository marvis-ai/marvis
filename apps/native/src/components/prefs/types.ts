import type { Config, KeystoreStatus, ModelSelection } from '@/lib/commands';

/**
 * The shared surface the prefs window loads once (`views/Prefs.tsx`) and
 * hands to both modes. Mutations come back through `keystore:changed` /
 * `config:changed` events too, so setters are also called by command
 * results for immediacy.
 */
export interface PrefsData {
  status: KeystoreStatus | null;
  config: Config | null;
  /** The resolved chain head (`model_get_selected`) — `null` when no
   * provider is usable. Reorder/switch/key writes all re-resolve it. */
  selected: ModelSelection | null;
  setStatus: (s: KeystoreStatus) => void;
  setConfig: (c: Config) => void;
  setSelected: (s: ModelSelection | null) => void;
}
