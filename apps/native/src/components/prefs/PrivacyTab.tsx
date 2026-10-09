/**
 * Privacy — the local file tree (paths.rs is the source of truth), the
 * capture budget as top-bordered stat cells, and Clear history. Clearing
 * iterates `session_list` → `session_delete`; deletes are permanent, so
 * the button is two-step: first click arms it, second within the timeout
 * deletes.
 */
import { useRef, useState } from 'react';
import { sessionDelete, sessionList } from '@/lib/commands';
import {
  BTN_DANGER,
  BTN_LG,
  BTN_OUTLINE,
  H2,
  NUM,
  PROV_NOTE,
  PRF_ROWS,
  SUB,
  cn,
} from '@/lib/classes';
import { PrefRow } from './bits';

const CONFIRM_MS = 4000;

export const PrivacyTab = () => {
  const [confirming, setConfirming] = useState(false);
  const [note, setNote] = useState('');
  const timer = useRef<number | undefined>(undefined);

  const clearHistory = async () => {
    if (!confirming) {
      setConfirming(true);
      setNote('');
      timer.current = window.setTimeout(() => setConfirming(false), CONFIRM_MS);
      return;
    }
    window.clearTimeout(timer.current);
    setConfirming(false);
    try {
      const sessions = await sessionList();
      await Promise.all(sessions.map((s) => sessionDelete(s.id)));
      setNote(
        `Deleted ${sessions.length} session${sessions.length === 1 ? '' : 's'}.`,
      );
    } catch {
      setNote('Clear failed — try again.');
    }
  };

  return (
    <>
      <h2 className={H2}>Privacy &amp; data</h2>
      <p className={SUB}>
        One folder on disk, nothing in a cloud. Windows are content-protected —
        they don't appear in screenshots or screen share.
      </p>

      <div className='mb-3.5 overflow-x-auto rounded-[10px] border border-border bg-[color-mix(in_oklch,var(--bg)_60%,var(--surface))] px-3.5 py-3 font-mono text-[11.5px] leading-[1.9] whitespace-pre text-foreground [&_em]:not-italic [&_em]:text-muted-foreground'>
        {'~/.marvis/\n├── config.toml      '}
        <em>0644 — models, hotkeys, bar position</em>
        {'\n├── keys.json        '}
        <em>0600 — provider keys, plaintext, this Mac only</em>
        {'\n├── marvis.db        '}
        <em>sessions + messages + memories, sqlite</em>
        {'\n├── voiceprint.bin   '}
        <em>enrolled speaker embedding</em>
        {'\n├── audios/          '}
        <em>retained listen recordings (recording_*.wav)</em>
        {'\n└── attachments/     '}
        <em>ask composer images, normalized JPEGs</em>
      </div>

      <p className={cn(PROV_NOTE, 'mt-0 mb-3.5')}>
        Memory (off by default) stores profile facts in marvis.db — edit or
        delete them under Memory. Extraction runs only after you enable it and
        only sends your new Ask text to the selected Memory LLM: hosted
        providers receive that text, Ollama stays local (it may use your
        CPU/GPU). Screen frames, attachments, Listen transcripts, assistant
        replies, and past sessions are never analyzed.
      </p>

      <div className='my-3.5 grid grid-cols-3 gap-2.5'>
        <div className='border-t border-foreground pt-2'>
          <div
            className={cn(NUM, 'text-[22px] font-semibold tracking-[-0.02em]')}>
            4 fps
          </div>
          <div className='mt-0.5 text-[11px] text-muted-foreground'>
            screen capture, hash-deduped — unchanged frames drop
          </div>
        </div>
        <div className='border-t border-foreground pt-2'>
          <div
            className={cn(NUM, 'text-[22px] font-semibold tracking-[-0.02em]')}>
            60 s
          </div>
          <div className='mt-0.5 text-[11px] text-muted-foreground'>
            ring buffer — 120 frames, capped at 64 MB
          </div>
        </div>
        <div className='border-t border-foreground pt-2'>
          <div
            className={cn(NUM, 'text-[22px] font-semibold tracking-[-0.02em]')}>
            0
          </div>
          <div className='mt-0.5 text-[11px] text-muted-foreground'>
            accounts, sync services, or Marvis-side servers
          </div>
        </div>
      </div>

      <div className={PRF_ROWS}>
        <PrefRow
          label='Session history'
          sub='Clear all in marvis.db. Deletes are permanent.'
          last>
          <button
            type='button'
            className={cn(BTN_LG, BTN_OUTLINE, BTN_DANGER)}
            onClick={() => void clearHistory()}>
            {confirming ? 'Click to confirm' : 'Clear history'}
          </button>
        </PrefRow>
        {note && <p className={PROV_NOTE}>{note}</p>}
      </div>
    </>
  );
};
