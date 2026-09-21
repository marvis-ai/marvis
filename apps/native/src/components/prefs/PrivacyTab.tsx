/**
 * Privacy — the local file tree (paths.rs is the source of truth), the
 * capture budget as top-bordered stat cells, and Clear history. Clearing
 * iterates `session_list` → `session_delete`; deletes are permanent, so
 * the button is two-step: first click arms it, second within the timeout
 * deletes.
 */
import { useRef, useState } from 'react';
import { sessionDelete, sessionList } from '../../lib/commands';
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
      <h2>Privacy &amp; data</h2>
      <p className='sub'>
        Three files on disk, nothing in a cloud. Windows are content-protected —
        they don't appear in screenshots or screen share.
      </p>

      <div className='prf-filetree'>
        {'~/.marvis/\n├── config.toml   '}
        <em>0644 — models, hotkeys, bar position</em>
        {'\n├── keys.enc      '}
        <em>AES-256-GCM vault; DEK lives in Keychain</em>
        {'\n└── marvis.db     '}
        <em>sessions + messages, sqlite</em>
      </div>

      <div className='prf-statrow'>
        <div className='prf-stat'>
          <div className='s-num num'>4 fps</div>
          <div className='s-lbl'>
            screen capture, hash-deduped — unchanged frames drop
          </div>
        </div>
        <div className='prf-stat'>
          <div className='s-num num'>60 s</div>
          <div className='s-lbl'>ring buffer — 120 frames, capped at 64 MB</div>
        </div>
        <div className='prf-stat'>
          <div className='s-num num'>0</div>
          <div className='s-lbl'>
            accounts, sync services, or Marvis-side servers
          </div>
        </div>
      </div>

      <div className='prf-rows'>
        <PrefRow
          label='Session history'
          sub='ask and listen sessions in marvis.db. Deletes are permanent.'
          last>
          <button
            type='button'
            className='mv-btn mv-btn-outline mv-btn-danger'
            onClick={() => void clearHistory()}>
            {confirming ? 'Click to confirm' : 'Clear history'}
          </button>
        </PrefRow>
        {note && <p className='prov-note'>{note}</p>}
      </div>
    </>
  );
};
