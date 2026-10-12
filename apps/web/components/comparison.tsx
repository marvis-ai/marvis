import { CheckIcon, XIcon } from '@marvis/ui';
import { leadTop, sectionStack } from './styles';

interface ComparisonRow {
  dimension: string;
  marvis: string;
  others: string;
  marvisGood: boolean;
}

const COMPARISONS: ComparisonRow[] = [
  {
    dimension: 'Architecture & Storage',
    marvis: '100% on-device; 0 cloud servers or sync',
    others: 'Cloud servers, remote storage & telemetry',
    marvisGood: true,
  },
  {
    dimension: 'Screen Context',
    marvis: '60s rolling RAM ring (≤64 MB); never on disk',
    others: 'Stored in cloud database or disk history',
    marvisGood: true,
  },
  {
    dimension: 'Meeting Transcription',
    marvis: 'On-device whisper.cpp & sherpa ASR (offline ready)',
    others: 'Live audio streamed to third-party APIs',
    marvisGood: true,
  },
  {
    dimension: 'Model Choice',
    marvis: 'BYOK: OpenAI, Claude, Gemini, Ollama & local servers',
    others: 'Locked to single vendor subscription',
    marvisGood: true,
  },
  {
    dimension: 'Account & Identity',
    marvis: '0 accounts; no sign-in or tracking required',
    others: 'Mandatory user login, profile & billing',
    marvisGood: true,
  },
  {
    dimension: 'License & Cost',
    marvis: 'Free & Open Source (Apache-2.0)',
    others: '$20 – $30+ / user / month subscription',
    marvisGood: true,
  },
];

export const Comparison = () => (
  <section
    className='section'
    id='comparison'
    data-od-id='comparison'
    aria-labelledby='comparison-heading'>
    <div
      className='container stack'
      style={sectionStack}>
      <div style={{ maxWidth: '44ch' }}>
        <p className='eyebrow'>Built Different</p>
        <h2 id='comparison-heading'>Local-first vs. Cloud AI Copilots</h2>
        <p
          className='lead'
          style={leadTop}>
          Why privacy-first architecture matters for your screen, your meetings,
          and your proprietary work.
        </p>
      </div>

      <div
        className='card min-w-0'
        style={{ padding: 0, overflowX: 'auto' }}>
        <table
          className='ds-table'
          style={{ minWidth: '600px', margin: 0 }}>
          <thead>
            <tr>
              <th style={{ width: '28%' }}>Dimension</th>
              <th style={{ width: '38%', color: 'var(--fg-2)' }}>
                Marvis AI (Local &amp; BYOK)
              </th>
              <th style={{ width: '34%' }}>Cloud AI Copilots</th>
            </tr>
          </thead>
          <tbody>
            {COMPARISONS.map((row) => (
              <tr key={row.dimension}>
                <td style={{ fontWeight: 500, color: 'var(--fg-2)' }}>
                  {row.dimension}
                </td>
                <td style={{ color: 'var(--fg)' }}>
                  <span
                    style={{
                      display: 'inline-flex',
                      alignItems: 'center',
                      gap: '8px',
                    }}>
                    <CheckIcon
                      style={{
                        width: '16px',
                        height: '16px',
                        color: 'var(--success)',
                        flexShrink: 0,
                      }}
                    />
                    <span>{row.marvis}</span>
                  </span>
                </td>
                <td style={{ color: 'var(--muted)', textAlign: 'left' }}>
                  <span
                    style={{
                      display: 'inline-flex',
                      alignItems: 'center',
                      gap: '8px',
                    }}>
                    <XIcon
                      style={{
                        width: '16px',
                        height: '16px',
                        color: 'var(--danger)',
                        flexShrink: 0,
                      }}
                    />
                    <span>{row.others}</span>
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  </section>
);
