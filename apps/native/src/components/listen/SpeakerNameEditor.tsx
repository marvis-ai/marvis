import { useEffect, useRef, useState } from 'react';
import { PencilIcon } from '@marvis/ui';
import { cn } from '@/lib/classes';

/** Session-local speaker rename — inline label→input swap shared by
 *  the speaker filter chips and the transcript block headers. Click an
 *  editable label to edit; Enter or blur commits (trimmed, capped at
 *  40 Unicode scalars, empty = restore the default), Escape cancels.
 *  `editable === false` renders plain text — `You` and unlabeled
 *  speakers never get the affordance. */
export const SpeakerNameEditor = ({
  label,
  editable,
  onCommit,
}: {
  label: string;
  editable: boolean;
  /** Receives the trimmed/capped label, or '' to clear the override. */
  onCommit: (label: string) => void;
}) => {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(label);
  const inputRef = useRef<HTMLInputElement>(null);
  /** Commit and blur can both fire for one gesture (Enter unmounts the
   *  input, which then blurs) — latch so a commit can't be undone by a
   *  trailing blur and a cancel can't turn into a commit. */
  const endedRef = useRef(false);

  // A rename landing while idle (or a new session's defaults) refreshes
  // the resting draft; mid-edit the user's keystrokes win.
  useEffect(() => {
    if (!editing) setDraft(label);
  }, [label, editing]);

  useEffect(() => {
    if (!editing) return;
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [editing]);

  const commit = () => {
    if (endedRef.current) return;
    endedRef.current = true;
    onCommit(Array.from(draft.trim()).slice(0, 40).join(''));
    setEditing(false);
  };

  const cancel = () => {
    if (endedRef.current) return;
    endedRef.current = true;
    setDraft(label);
    setEditing(false);
  };

  if (!editable) return <span>{label}</span>;

  if (editing) {
    return (
      <input
        ref={inputRef}
        value={draft}
        aria-label={`Rename ${label}`}
        maxLength={80}
        onInput={(event) => setDraft(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            event.preventDefault();
            commit();
          } else if (event.key === 'Escape') {
            event.preventDefault();
            cancel();
          }
        }}
        onBlur={commit}
        onClick={(event) => event.stopPropagation()}
        className={cn(
          'w-24 max-w-full rounded-[5px] border border-accent/60 bg-input-well',
          'px-1 py-px [font:inherit] text-foreground',
          'outline-none focus:shadow-(--focus-ring)',
        )}
      />
    );
  }

  return (
    <button
      type='button'
      aria-label={`Rename ${label}`}
      title={`Rename ${label}`}
      onClick={(event) => {
        // A chip wraps this in a filter row — editing must not toggle it.
        event.stopPropagation();
        endedRef.current = false;
        setDraft(label);
        setEditing(true);
      }}
      className={cn(
        'group/edit inline-flex min-w-0 cursor-pointer items-center gap-1',
        'rounded-[5px] border-0 bg-transparent p-0 [font:inherit] text-inherit',
        'focus-visible:outline-2 focus-visible:outline-accent',
      )}>
      <span className='truncate'>{label}</span>
      <PencilIcon
        aria-hidden
        className='size-2.5 flex-none opacity-0 transition-opacity group-hover/edit:opacity-60 group-focus-visible/edit:opacity-60'
      />
    </button>
  );
};
