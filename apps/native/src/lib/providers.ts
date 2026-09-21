/**
 * Provider catalog — the ids mirror `ProviderKind::as_str` on the Rust
 * side and are what every `keystore_*`/`model_*` command expects.
 * Shared by the prefs Providers tab and the onboarding BYOK step so the
 * two surfaces can never drift.
 */
export interface ProviderDef {
  /** `ProviderKind::as_str` — the id every command expects. */
  id: string;
  label: string;
  /** Placeholder for the key input (empty when no key field renders). */
  keyPlaceholder: string;
  /** Runs without a stored key — Ollama's daemon, open compat endpoints. */
  keyOptional?: boolean;
  /** OpenAI-compatible endpoint: name + base URL fields, free-text model. */
  compat?: boolean;
}

export const PROVIDERS: ProviderDef[] = [
  { id: 'openai', label: 'OpenAI', keyPlaceholder: 'sk-…' },
  { id: 'anthropic', label: 'Anthropic', keyPlaceholder: 'sk-ant-…' },
  { id: 'gemini', label: 'Gemini', keyPlaceholder: 'AIza…' },
  { id: 'openrouter', label: 'OpenRouter', keyPlaceholder: 'sk-or-…' },
  { id: 'ollama', label: 'Ollama', keyPlaceholder: '', keyOptional: true },
  {
    id: 'compatible',
    label: 'OpenAI-compatible',
    keyPlaceholder: 'key — optional on open endpoints',
    keyOptional: true,
    compat: true,
  },
];

export const providerFor = (id: string): ProviderDef | undefined =>
  PROVIDERS.find((p) => p.id === id);

/** Display name for a provider row — the compat endpoint gets its
 * configured name (or a host-derived one), the rest use the catalog label. */
export const providerLabel = (id: string, compatName: string): string => {
  if (id === 'compatible') {
    return compatName || 'OpenAI-compatible';
  }
  return providerFor(id)?.label ?? id;
};
