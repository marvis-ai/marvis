/**
 * Presets — the Ask preset list. Built-ins ship with the app
 * (read-only); custom presets persist as `prompts.custom` and apply
 * per-send from the composer's wand palette or `/name` shorthand.
 * There's no category to pick: `{input}` in the text expands into the
 * sent message and `{lang}` becomes an editable language badge;
 * without `{input}` the text steers the reply silently.
 */
import { useEffect, useRef, useState } from 'react';
import { Trash2Icon } from '@marvis/ui';
import { configSet, type Preset } from '@/lib/commands';
import { usePresets } from '@/hooks/usePresets';
import {
  BTN_OUTLINE,
  BTN_PRIMARY,
  BTN_SM,
  H2,
  ICON_BTN,
  PRF_ROW,
  PRF_ROWS,
  PROV_ERR,
  PR_LABEL,
  PR_SUB,
  SUB,
  cn,
} from '@/lib/classes';
import { PrefRow } from './bits';
import type { PrefsData } from './types';

const INPUT =
  'w-full rounded-lg border border-border bg-input-well px-2.5 py-1.5 text-[12.5px] text-foreground outline-none transition-[border-color,box-shadow] duration-(--motion-fast) ease-(--ease) focus:border-accent focus:shadow-(--focus-ring)';

const mintId = () => `u:${Math.random().toString(36).slice(2, 10)}`;

export const PresetsTab = ({ data }: { data: PrefsData }) => {
  const customs = data.config?.prompts.custom ?? [];
  const builtins = usePresets().filter((p) => p.id.startsWith('b:'));
  const confirmedCustoms = useRef(customs);
  const writes = useRef(Promise.resolve());
  useEffect(() => {
    confirmedCustoms.current = data.config?.prompts.custom ?? [];
  }, [data.config]);

  const [editing, setEditing] = useState<Preset | null>(null);
  const [isNew, setIsNew] = useState(false);
  /** A rejected `prompts.custom` write — the server string is already
   *  user-surfaceable (`presets::validate_custom`). */
  const [saveError, setSaveError] = useState('');

  const write = (update: (current: Preset[]) => Preset[]) => {
    // Build the next list only after the previous write has confirmed it.
    const result = writes.current.then(async () => {
      const config = await configSet(
        'prompts.custom',
        update(confirmedCustoms.current),
      );
      confirmedCustoms.current = config.prompts.custom;
      data.setConfig(config);
    });
    // A rejected write leaves the confirmed list intact and must not
    // prevent subsequent saves or deletions from running.
    writes.current = result.catch(() => {});
    return result;
  };

  const startNew = () => {
    setIsNew(true);
    setSaveError('');
    setEditing({ id: mintId(), name: '', text: '' });
  };
  const save = () => {
    if (!editing) return;
    const clean = {
      ...editing,
      name: editing.name.trim(),
      text: editing.text.trim(),
    };
    if (!clean.name || !clean.text) return;
    // Close only on success — a rejected write keeps the editor open so
    // the draft isn't silently lost, and says why.
    setSaveError('');
    void write((current) =>
      isNew
        ? [...current, clean]
        : current.map((p) => (p.id === clean.id ? clean : p)),
    )
      .then(() => {
        setEditing(null);
      })
      .catch((e) => setSaveError(typeof e === 'string' ? e : 'Save failed'));
  };

  return (
    <>
      <h2 className={H2}>Presets</h2>
      <p className={SUB}>
        Presets apply to one Ask send — pick them from the composer's wand
        palette, or type <code>/</code> + a name (like <code>/summarize</code>).
      </p>

      <div className={PRF_ROWS}>
        {builtins.map((p) => (
          <PrefRow
            key={p.id}
            label={p.name}
            sub={p.text}
          />
        ))}
      </div>

      <h3 className='mt-6 mb-2 text-[13px] font-[550]'>Your presets</h3>
      <div className={PRF_ROWS}>
        {customs.map((p) => (
          <PrefRow
            key={p.id}
            label={p.name}
            sub={p.text}>
            <button
              type='button'
              className={cn(BTN_SM, BTN_OUTLINE)}
              onClick={() => {
                setIsNew(false);
                setSaveError('');
                setEditing(p);
              }}>
              Edit
            </button>
            <button
              type='button'
              aria-label={`Delete ${p.name}`}
              className={ICON_BTN}
              onClick={() =>
                void write((current) => current.filter((x) => x.id !== p.id))
                  .catch(() => {})
              }>
              <Trash2Icon className='size-3.5' />
            </button>
          </PrefRow>
        ))}
        {customs.length === 0 && !editing && (
          <div className={cn(PRF_ROW, 'border-b-0')}>
            <div className={PR_SUB}>No custom presets yet.</div>
          </div>
        )}

        {editing ? (
          <div
            className={cn(
              PRF_ROW,
              'flex-col items-stretch gap-2.5 border-b-0',
            )}>
            <input
              aria-label='Preset name'
              maxLength={24}
              placeholder='Name — also the /name'
              className={cn(INPUT, 'w-48')}
              value={editing.name}
              onChange={(e) => setEditing({ ...editing, name: e.target.value })}
            />
            <textarea
              aria-label='Preset text'
              maxLength={2000}
              placeholder='Prompt text — e.g. Answer like a skeptical reviewer.'
              className={cn(INPUT, 'min-h-24 resize-y leading-relaxed')}
              value={editing.text}
              onChange={(e) => setEditing({ ...editing, text: e.target.value })}
            />
            <div className={PR_SUB}>
              <code>{'{input}'}</code> expands to the typed message,{' '}
              <code>{'{lang}'}</code> to your main language (an editable badge
              when the preset is armed). Include <code>{'{input}'}</code> and
              the preset expands into your message; leave it out and the text
              steers the reply silently. Other <code>{'{…}'}</code> placeholders
              aren't expanded.
            </div>
            {saveError && <p className={PROV_ERR}>{saveError}</p>}
            <div className='flex justify-end gap-2'>
              <button
                type='button'
                className={cn(BTN_SM, BTN_OUTLINE)}
                onClick={() => setEditing(null)}>
                Cancel
              </button>
              <button
                type='button'
                className={cn(BTN_SM, BTN_PRIMARY)}
                disabled={!editing.name.trim() || !editing.text.trim()}
                onClick={save}>
                {isNew ? 'Add preset' : 'Save'}
              </button>
            </div>
          </div>
        ) : (
          <div className={cn(PRF_ROW, 'border-b-0')}>
            <div>
              <div className={PR_LABEL}>New preset</div>
              <div className={PR_SUB}>
                Name it something short — it becomes the <code>/name</code>{' '}
                shorthand too (<code>Reply nicely</code> →{' '}
                <code>/reply-nicely</code>).
              </div>
            </div>
            <span className='inline-flex flex-none items-center gap-2'>
              <button
                type='button'
                className={cn(BTN_SM, BTN_OUTLINE)}
                onClick={startNew}>
                New preset
              </button>
            </span>
          </div>
        )}
      </div>
    </>
  );
};
