/**
 * Recording — screen-capture cadence and the voice-summary instruction.
 * `recording.*` writes broadcast `config:changed`; an fps write
 * restarts a live capture server-side so the new rate applies now.
 */
import { useEffect, useRef, useState } from 'react';
import { configSet } from '@/lib/commands';
import {
  H2,
  MODEL_SEL,
  PRF_ROW,
  PRF_ROWS,
  PR_LABEL,
  PR_SUB,
  SUB,
  cn,
} from '@/lib/classes';
import { PrefRow, Seg, Switch } from './bits';
import type { PrefsData } from './types';

const SUMMARY_TEMPLATES = [
  {
    id: 'meeting',
    label: 'Meeting',
    text: 'Focus on decisions made, action items with owners and deadlines, and open questions.',
  },
  {
    id: 'book',
    label: 'Book / article',
    text: 'Focus on the key ideas, themes, and takeaways; note memorable claims or quotes.',
  },
  {
    id: 'lecture',
    label: 'Lecture',
    text: 'Focus on concepts taught, definitions, worked examples, and anything emphasized as important.',
  },
  {
    id: 'interview',
    label: 'Interview',
    text: "Focus on the candidate's answers, demonstrated strengths, concerns raised, and notable questions.",
  },
  {
    id: 'brainstorm',
    label: 'Brainstorm',
    text: 'Focus on ideas proposed, pros and cons discussed, and the directions the group is converging toward.',
  },
] as const;

const MEETING_TEXT = SUMMARY_TEMPLATES[0].text;

export const RecordingTab = ({ data }: { data: PrefsData }) => {
  const cfg = data.config;
  const auto = cfg?.recording.auto_screenshots ?? true;
  const fps = cfg?.recording.fps === 8 || cfg?.recording.fps === 2 ? cfg.recording.fps : 4;
  const stored = cfg?.recording.summary_prompt ?? '';
  // '' reads as the Meeting template — the textarea shows the
  // instruction the summary will actually use.
  const shown = stored.trim() === '' ? MEETING_TEXT : stored;
  const [draft, setDraft] = useState(shown);
  useEffect(() => setDraft(shown), [shown]);
  const timer = useRef<number | undefined>(undefined);
  const pending = useRef<string | null>(null);
  // Flush a pending debounced write on unmount (a mode switch remounts
  // this subtree — the prefs window itself never unmounts).
  useEffect(
    () => () => {
      window.clearTimeout(timer.current);
      if (pending.current !== null) {
        void configSet('recording.summary_prompt', pending.current).catch(() => {});
      }
    },
    [],
  );

  const writePrompt = (text: string) => {
    pending.current = text;
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      pending.current = null;
      void configSet('recording.summary_prompt', text)
        .then(data.setConfig)
        .catch(() => {});
    }, 400);
  };

  // The template whose text the draft matches, else 'custom'.
  const selected = SUMMARY_TEMPLATES.find((t) => t.text === draft)?.id ?? 'custom';

  const pickTemplate = (id: string) => {
    const t = SUMMARY_TEMPLATES.find((t) => t.id === id);
    if (!t) return; // 'custom' — the draft already is the custom text
    pending.current = null;
    window.clearTimeout(timer.current);
    setDraft(t.text);
    void configSet('recording.summary_prompt', t.text)
      .then(data.setConfig)
      .catch(() => {});
  };

  return (
    <>
      <h2 className={H2}>Recording</h2>
      <p className={SUB}>
        Screen capture runs quietly in the background; voice settings shape
        Listen summaries.
      </p>

      <div className={PRF_ROWS}>
        <PrefRow
          label='Take screenshots automatically'
          sub='Capture the screen when the app is running — the bar’s record button still works when this is off.'>
          <Switch
            ariaLabel='Take screenshots automatically'
            checked={auto}
            onChange={(on) =>
              void configSet('recording.auto_screenshots', on)
                .then(data.setConfig)
                .catch(() => {})
            }
          />
        </PrefRow>
        <PrefRow
          label='Frame rate'
          sub='How often the screen is sampled — higher rates track motion better.'>
          <Seg
            ariaLabel='Frame rate'
            value={String(fps) as '8' | '4' | '2'}
            onChange={(v) =>
              void configSet('recording.fps', Number(v))
                .then(data.setConfig)
                .catch(() => {})
            }
            options={[
              { id: '8' as const, label: '8 fps' },
              { id: '4' as const, label: '4 fps' },
              { id: '2' as const, label: '2 fps' },
            ]}
          />
        </PrefRow>
        <div className={cn(PRF_ROW, 'flex-col items-stretch gap-2.5 border-b-0')}>
          <div className='flex items-center justify-between gap-4'>
            <div>
              <div className={PR_LABEL}>Summary instruction</div>
              <div className={PR_SUB}>
                Guides Listen summaries — pick a template, then edit the text
                freely.
              </div>
            </div>
            <select
              aria-label='Summary template'
              className={cn(MODEL_SEL, 'w-40 flex-none')}
              value={selected}
              onChange={(e) => pickTemplate(e.target.value)}>
              {SUMMARY_TEMPLATES.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.label}
                </option>
              ))}
              <option value='custom'>Custom</option>
            </select>
          </div>
          <textarea
            aria-label='Summary instruction'
            className='min-h-20 w-full resize-y rounded-lg border border-border bg-input-well px-2.5 py-2 text-[12.5px] leading-relaxed text-foreground outline-none transition-[border-color,box-shadow] duration-(--motion-fast) ease-(--ease) focus:border-accent focus:shadow-(--focus-ring)'
            value={draft}
            onChange={(e) => {
              setDraft(e.target.value);
              writePrompt(e.target.value);
            }}
          />
        </div>
      </div>
    </>
  );
};
