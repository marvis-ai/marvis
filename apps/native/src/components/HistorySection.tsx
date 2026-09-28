/**
 * The card's history section — one chronological list of `sessions`
 * rows (chats + meetings). A chat row resumes that session and lands on
 * the Chat section; a listen row opens the finished document (or the
 * live capture when the session is still open). The list re-reads on
 * `listen:state` and on an ask run finishing (`ask:state{idle}`) — the
 * two mutations that change it while the card is up.
 */
import { useEffect, useState } from 'react';
import {
  MessageSquareTextIcon,
  MicAudioLinesIcon,
  Trash2Icon,
} from '@marvis/ui';
import {
  sessionDelete,
  sessionList,
  sessionResume,
  type Session,
} from '@/lib/commands';
import {
  EV_ASK_STATE,
  EV_LISTEN_STATE,
  useTauriEvent,
  type ListenStatePayload,
} from '@/lib/events';
import type { AskActivity } from '@/lib/bar-state';
import { CHIP, EMPTY, ICON_BTN, NUM, PANEL_BODY, cn } from '@/lib/classes';
import { CardHeader } from '@/components/shared/CardHeader';
import { ConfirmButton } from '@/components/shared/ConfirmButton';
import { relTime, type ListenViewing } from '@/components/listen/model';

export const HistorySection = ({
  askBusy,
  onOpenChat,
  onOpenListen,
  onBack,
}: {
  /** An in-flight ask run belongs to the open chat session — ask rows
   *  disable until it settles (resume would end that session). */
  askBusy: boolean;
  onOpenChat: () => void;
  /** `null` = the live session (still-open listen row) → live view. */
  onOpenListen: (v: ListenViewing | null) => void;
  /** Leaves the section — collapses the card back to the capsule. */
  onBack: () => void;
}) => {
  const [sessions, setSessions] = useState<Session[] | null>(null);
  const refresh = () => {
    void sessionList()
      .then(setSessions)
      .catch(() => {});
  };
  useEffect(refresh, []);
  // A listen session starting/stopping changes the list live.
  useTauriEvent<ListenStatePayload>(EV_LISTEN_STATE, refresh);
  useTauriEvent<{ state: AskActivity }>(EV_ASK_STATE, (p) => {
    if (p.state === 'idle') refresh();
  });

  const openRow = (s: Session) => {
    if (s.kind === 'ask') {
      if (askBusy) return; // an in-flight run belongs to the open session
      void sessionResume(s.id)
        .then((ok) => {
          if (ok) onOpenChat();
        })
        .catch(() => {});
      return;
    }
    onOpenListen(
      s.ended_at === null
        ? null // live session → live view
        : { id: s.id, startedAt: s.started_at, endedAt: s.ended_at },
    );
  };

  const removeRow = (s: Session) => {
    void sessionDelete(s.id)
      .then(() =>
        setSessions((prev) => prev?.filter((x) => x.id !== s.id) ?? prev),
      )
      .catch(() => {});
  };

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      {/* Back on the left — the header doubles as the card's drag
          region. This section drops the shared input row: it's a
          picker, not a chat surface. */}
      <CardHeader
        title='History'
        onBack={onBack}
      />
      <div
        data-card-scroll
        className={PANEL_BODY}>
        <div
          data-card-content
          className='flow-root'>
          {sessions === null ? null : sessions.length === 0 ? (
            <p className={EMPTY}>
              No history yet — ask Marvis or start listening.
            </p>
          ) : (
            sessions.map((s) => {
              const liveRow = s.ended_at === null;
              const disabled = s.kind === 'ask' && askBusy;
              return (
                <div
                  key={s.id}
                  className={cn(
                    'group/row -mx-1.5 flex items-center gap-2 rounded-lg px-1.5 py-1.5 transition-colors',
                    disabled ? 'opacity-50' : 'cursor-pointer hover:bg-fg-soft',
                  )}
                  onClick={() => !disabled && openRow(s)}>
                  {s.kind === 'listen' ? (
                    <MicAudioLinesIcon className='size-4 flex-none text-muted-foreground' />
                  ) : (
                    <MessageSquareTextIcon className='size-4 flex-none text-muted-foreground' />
                  )}
                  <span className='min-w-0 flex-1 truncate text-sm'>
                    {s.title ?? (s.kind === 'listen' ? 'Meeting' : 'Chat')}
                  </span>
                  {liveRow && (
                    <span className={cn(CHIP, 'border-accent/40 text-accent')}>
                      Live
                    </span>
                  )}
                  <span
                    className={cn(
                      NUM,
                      'flex-none text-[10px] text-muted-foreground',
                    )}>
                    {relTime(s.last_active_at)}
                  </span>
                  {!liveRow && (
                    <ConfirmButton
                      aria-label='Delete session'
                      onConfirm={() => removeRow(s)}
                      className={cn(
                        ICON_BTN,
                        'size-5 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100',
                      )}>
                      <Trash2Icon className='size-3' />
                    </ConfirmButton>
                  )}
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
};
