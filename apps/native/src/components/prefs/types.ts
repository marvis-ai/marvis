import type { Config, KeystoreStatus, ModelSelection } from '../../lib/commands';

/**
 * The shared surface the prefs window loads once (`views/Prefs.tsx`) and
 * hands to both modes. Mutations come back through `keystore:changed` /
 * `config:changed` events too, so setters are also called by command
 * results for immediacy.
 */
export interface PrefsData {
  status: KeystoreStatus | null;
  config: Config | null;
  selected: ModelSelection | null;
  setStatus: (s: KeystoreStatus) => void;
  setConfig: (c: Config) => void;
  setSelected: (s: ModelSelection) => void;
}
