/**
 * Providers — the vault card on top, then every provider as an
 * expandable row, then the dimmed Deepgram row (Phase 2). Nothing renders
 * until `status` and `config` are loaded — the compat fields seed from
 * `config.compat` at mount.
 */
import { PROVIDERS } from '../../lib/providers';
import { ProviderCard } from './ProviderCard';
import { VaultCard } from './VaultCard';
import type { PrefsData } from './types';

export const ProvidersTab = ({ data }: { data: PrefsData }) => (
  <>
    <h2>Providers</h2>
    <p className='sub'>
      Your keys, your models. Each key is validated before it's stored — a bad
      key never touches disk.
    </p>

    {data.status === null || data.config === null ? (
      <p className='prf-loading'>Loading…</p>
    ) : (
      <>
        <VaultCard
          status={data.status}
          setStatus={data.setStatus}
        />
        {PROVIDERS.map((p) => (
          <ProviderCard
            key={p.id}
            def={p}
            data={data}
          />
        ))}
        <div className='prf-prov is-future'>
          <div className='prf-prov-head prf-prov-head-static'>
            <span className='prf-prov-dot' />
            <span className='prf-prov-name'>Deepgram</span>
            <span className='prf-prov-state'>stt · nova-2 · Phase 2</span>
          </div>
        </div>
      </>
    )}
  </>
);
