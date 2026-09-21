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
  /** Mirrors `llm::static_models` first entry — what the provider answers
   * with when `providers.models.<id>` is empty. Absent for live-list
   * providers (Ollama/compatible), which are unusable until a model is
   * picked. */
  defaultModel?: string;
}

export const PROVIDERS: ProviderDef[] = [
  {
    id: 'openai',
    label: 'OpenAI',
    keyPlaceholder: 'sk-…',
    defaultModel: 'gpt-4o',
  },
  {
    id: 'anthropic',
    label: 'Anthropic',
    keyPlaceholder: 'sk-ant-…',
    defaultModel: 'claude-sonnet-4-5',
  },
  {
    id: 'gemini',
    label: 'Gemini',
    keyPlaceholder: 'AIza…',
    defaultModel: 'gemini-2.5-pro',
  },
  {
    id: 'openrouter',
    label: 'OpenRouter',
    keyPlaceholder: 'sk-or-…',
    defaultModel: 'openai/gpt-4o-mini',
  },
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

/**
 * The catalog in the user's failover order (`config.providers.order`).
 * Unknown ids are skipped; catalog entries missing from `order` append
 * at the end — mirroring `Config::normalize`, so the list never drops a
 * provider even before the first reorder lands.
 */
export const orderedProviders = (
  order: string[] | undefined,
): ProviderDef[] => {
  if (!order) {
    return PROVIDERS;
  }
  const seen = new Set<string>();
  const out: ProviderDef[] = [];
  for (const id of order) {
    const def = providerFor(id);
    if (def && !seen.has(id)) {
      seen.add(id);
      out.push(def);
    }
  }
  for (const def of PROVIDERS) {
    if (!seen.has(def.id)) {
      out.push(def);
    }
  }
  return out;
};

/** Display name for a provider row — the compat endpoint gets its
 * configured name (or a host-derived one), the rest use the catalog label. */
export const providerLabel = (id: string, compatName: string): string => {
  if (id === 'compatible') {
    return compatName || 'OpenAI-compatible';
  }
  return providerFor(id)?.label ?? id;
};
