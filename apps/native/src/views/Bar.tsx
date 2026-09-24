/**
 * The always-on-top bar (`?view=bar`) — the UNIFIED window. Two shapes:
 *
 *  - Pill modes (mini | input | permission): the 140⇄600×64 capsule⇄
 *    input morph — the capsule IS the window under liquid glass, so
 *    `expanded` reports to `window_set_bar_expanded` and Rust animates
 *    the width change. Unchanged mechanics.
 *  - Card modes (chat | listen): the same window grown to
 *    600×(64+content) — the bar row becomes the card's header, pinned
 *    to the anchored edge (`flex-col-reverse` when growing up puts the
 *    row at the bottom and it never visually jumps).
 *
 * `cardOpen`/`growDir` live in `useCardGeometry` — read off the window
 * itself (`resize`/`tauri://move` are the only open/close signals).
 * Dictation is `useDictation`; the permission gate is `useGate`; the
 * background-activity mirrors are `useBarActivity`.
 *
 * `data-tauri-drag-region` lives on the bar ROW only in card mode — a
 * stage-level region would intercept text selection in the scrollable
 * conversation. In pill mode it stays on the capsule chrome as before.
 *
 * The mic affordance splits by surface: collapsed shows the Listen
 * recorder (`MicAudioLinesIcon`, opens the card into meeting Listen)
 * next to the screen-capture toggle (`MonitorDotIcon`); expanded shows
 * dictation (`MicIcon`) into the Ask field (`dictation:*` — transient,
 * nothing persists).
 *
 * Errors go to the `alert` window (`raise`) — the pill has no room.
 */
import { useEffect, useRef, useState } from 'react';
import type { SubmitEvent } from 'react';
import {
  MicAudioLinesIcon,
  MicIcon,
  MonitorDotIcon,
  SettingsIcon,
  ShineBorder,
  cn,
} from '@marvis/ui';
import {
  askClose,
  askSend,
  captureStart,
  captureStop,
  listenStart,
  listenStop,
  raise,
  windowFocusBar,
  windowSetBarExpanded,
  windowSetChatOpen,
  windowShowSettings,
} from '@/lib/commands';
import { EV_BAR_TOGGLE_INPUT, useTauriEvent } from '@/lib/events';
import { barControls, hasActiveWork } from '@/lib/bar-state';
import { useBarActivity } from '@/hooks/useBarActivity';
import { useCardGeometry } from '@/hooks/useCardGeometry';
import { useDictation } from '@/hooks/useDictation';
import { useGate } from '@/hooks/useGate';
import { AskInput } from '@/components/bar/AskInput';
import { BarButton } from '@/components/bar/BarButton';
import { BootErrorRow } from '@/components/bar/BootErrorRow';
import { DictationWaveform } from '@/components/bar/DictationWaveform';
import { IrisButton } from '@/components/bar/IrisButton';
import { PermissionRow } from '@/components/bar/PermissionRow';
import { ChatSection } from '@/components/ChatSection';
import { LaunchIntro } from '@/components/LaunchIntro';
import { ListenSection } from '@/components/ListenSection';
import { PANEL } from '@/lib/classes';

const Bar = () => {
  const { gate, bootError, busy, setBusy, bootstrap, grantScreen } = useGate();
  const [text, setTextState] = useState('');
  /** Live mirror of `text` for Tauri event handlers — they fire outside
   *  React's batching, so the state variable (and the DOM) can lag a
   *  queued `setText`; the ref is written with every update and always
   *  reads back the latest value. */
  const textRef = useRef('');
  const setText = (value: string) => {
    textRef.current = value;
    setTextState(value);
  };
  const [open, setOpen] = useState(false);
  /** The launch wordmark plays once per webview mount. Reduced-motion
   *  users skip it entirely — the capsule just opens on the icon row. */
  const [introDone, setIntroDone] = useState(
    () => window.matchMedia('(prefers-reduced-motion: reduce)').matches,
  );
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  /** Serializes mic-button start/stop across both speech modes — a new
   *  start must never race an in-flight stop (dictation and Listen are
   *  mutually exclusive server-side). */
  const speechBusy = useRef(false);

  const { cardOpen, growDir } = useCardGeometry(cardRef, stageRef);
  const {
    listenWanted,
    setListenWanted,
    listenState,
    setListenState,
    captureRunning,
    setCaptureRunning,
    askState,
  } = useBarActivity();

  // Icon-row ⇄ input-row swap: the gate card and boot errors count as
  // expanded; `main` rests as the icon row until the iris opens it or
  // the user starts typing on the focused window.
  const expanded = bootError
    ? true
    : gate === 'main'
      ? open || text.length > 0
      : gate !== null;
  /** The row shows the input row in card modes regardless of `open` —
   *  it's the card header and the follow-up field. */
  const showInputRow = expanded || cardOpen;
  /** The launch wordmark covers the collapsed idle capsule only —
   *  `expanded` is already true for the gate cards and boot error, so
   *  `!showInputRow` keeps it off every non-idle surface. */
  const showIntro = !introDone && !showInputRow;
  /** Whether the Ask `<input>` is actually mounted: the permission
   *  card and the boot-error retry replace the whole row while
   *  `showInputRow` stays true — dictation keys off this. */
  const inputRendered =
    showInputRow && !bootError && gate !== 'needs_permission';
  /** The row's control set for this surface — `bar-state.ts` owns the
   *  contract, the conditionals below consume it so the two can't
   *  drift. */
  const controls = barControls(showInputRow);

  const dictation = useDictation({
    text,
    textRef,
    setText,
    inputRef,
    inputRendered,
  });

  const section: 'chat' | 'listen' | null = !cardOpen
    ? null
    : listenWanted
      ? 'listen'
      : 'chat';
  /** Any live work pulses the floating shell — specific controls keep
   *  their stronger active affordances on top of it. */
  const activeWork = hasActiveWork({
    ask: askState,
    listen: listenState,
    dictation: dictation.state,
  });

  // The boot splash (index.html) is a sibling of #root — outside React —
  // so it needs imperative removal once the intro has finished, was
  // skipped (reduced-motion), or was interrupted mid-flight.
  useEffect(() => {
    if (!showIntro) {
      document.getElementById('boot-splash')?.remove();
    }
  }, [showIntro]);

  // The `toggle_input` global hotkey (lib.rs `hotkey_dispatch` emits
  // `bar:toggle-input`): morph capsule ⇄ input pill only — it never
  // opens the card; an open card counts as "shown" and collapses.
  useTauriEvent(EV_BAR_TOGGLE_INPUT, () => {
    if (gate !== 'main') {
      return;
    }
    if (cardOpen) {
      void windowSetChatOpen(false).catch(() => {});
      return;
    }
    if (open) {
      collapse();
      return;
    }
    setOpen(true);
    // A global-hotkey show must focus the window too, or the user's
    // typing lands in whatever app was frontmost.
    void windowFocusBar().catch(() => {});
  });

  // Listen is an independent session: collapsing the card must not change
  // which active section is shown when it is reopened.

  // The capsule IS the window under liquid glass — the pill⇄input morph
  // resizes it (idle 140 ⇄ 600). While the card is open the morph is
  // dormant: the width report is skipped so `bar_rect` (the canonical
  // pill) restores verbatim on collapse.
  useEffect(() => {
    if (!cardOpen) {
      void windowSetBarExpanded(expanded).catch(() => {});
    }
  }, [expanded, cardOpen]);

  // Focus the field whenever the input row shows.
  useEffect(() => {
    if (showInputRow && gate === 'main') {
      inputRef.current?.focus();
    }
  }, [showInputRow, gate]);

  // Type-to-wake on the collapsed pill; Esc collapses input → capsule,
  // and collapses the card via `ask_close` (cancel + `set_chat_open`).
  // `Cmd+,` opens settings — a bar-local key (fires only while this
  // window is focused), not a global hotkey.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (
        e.key === ',' &&
        e.metaKey &&
        !e.ctrlKey &&
        !e.altKey &&
        !e.shiftKey
      ) {
        e.preventDefault();
        void windowShowSettings().catch(() => {});
        return;
      }
      if (e.key === 'Escape') {
        if (cardOpen) {
          void askClose().catch(() => {});
          return;
        }
        // Esc discards the field — a pending stop's returned draft must
        // not land in the cleared text, and a live anchor stops without
        // applying.
        dictation.discard();
        setText('');
        setOpen(false);
        inputRef.current?.blur();
        return;
      }
      if (
        gate !== 'main' ||
        open ||
        cardOpen ||
        e.metaKey ||
        e.ctrlKey ||
        e.altKey
      ) {
        return;
      }
      if (e.key.length === 1) {
        // Type-to-wake replaces the input — drop any tracked anchor so
        // the wake character isn't treated as an edit inside it, and
        // discard a pending stop's draft so it can't splice into the
        // replacement text either.
        dictation.discard();
        setText(e.key);
        setOpen(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [gate, open, cardOpen]);

  const collapse = () => {
    setOpen(false);
    inputRef.current?.blur();
  };

  /** Send the field's current text — read off `textRef` so the Enter
   *  queued behind a settling stop sees the applied final draft, not
   *  the render-time `text`. */
  const sendAsk = () => {
    const t = textRef.current.trim();
    if (!t) {
      return;
    }
    setText('');
    void askSend(t).catch(() => raise('Send failed'));
  };

  // Submit = ask (a follow-up while the card is open). The backend
  // expands the window itself — no local collapse needed either way.
  const submitAsk = (e?: SubmitEvent<HTMLFormElement>) => {
    e?.preventDefault();
    dictation.submit(sendAsk);
  };

  /** Toggle continuous screen capture — a pure recorder switch that
   *  never submits an Ask request. `busy` serializes against the
   *  permission grant sharing the same flag. */
  const toggleCapture = () => {
    if (busy) return;
    setBusy(true);
    // The state this press is trying to reach — pinned at click time so
    // a `capture:state` event flipping the flag mid-flight can't skew
    // the failure check.
    const wantRunning = !captureRunning;
    const failure = wantRunning
      ? 'Screen recording start failed'
      : 'Screen recording stop failed';
    const transition = wantRunning ? captureStart() : captureStop();
    void transition
      .then((next) => {
        // The commands resolve `{ running, frames }` rather than
        // rejecting, so a failed transition comes back short of the
        // target — surface it, then resync as usual.
        if (next.running !== wantRunning) {
          raise(failure);
        }
        setCaptureRunning(next.running);
      })
      .catch(() => raise(failure))
      .finally(() => setBusy(false));
  };

  const rowCls = cn(
    'flex min-h-0 w-full flex-none items-center gap-1.5',
    cardOpen
      ? cn(
          'min-h-16 border-border px-2.75',
          // The divider sits between the row and the section — which
          // side depends on the grow direction (the row is bottom-
          // pinned under `flex-col-reverse` when growing up).
          growDir === 'up' ? 'border-t' : 'border-b',
        )
      : showInputRow
        ? 'px-2.75'
        : 'justify-center px-1.75',
  );

  /** The mic affordance's next action: stop the live mode first,
   *  dictate into the visible Ask input, or start meeting Listen from
   *  the collapsed capsule. Labels the collapsed Listen control and
   *  the expanded dictation control alike. */
  const micLabel =
    dictation.state === 'listening'
      ? 'Stop dictation'
      : listenState === 'listening'
        ? 'Stop listening'
        : showInputRow
          ? 'Dictate'
          : 'Listen';

  /** The collapsed screen-capture toggle's label — the control flips
   *  between starting and stopping the recorder. */
  const captureLabel = captureRunning
    ? 'Stop screen recording'
    : 'Start screen recording';

  /** Shared press route for the split mic controls — collapsed Listen
   *  (`MicAudioLinesIcon`) and expanded dictation (`MicIcon`). A live
   *  session always stops first under `speechBusy` serialization; the
   *  `showInputRow` branch then lands on whichever control is mounted. */
  const pressMic = () => {
    if (speechBusy.current) return;
    speechBusy.current = true;
    // An active session stops first — `speechBusy` stays held
    // until it settles so a follow-up press can't start the
    // other mode mid-teardown.
    const stopping = dictation.stopIfActive();
    if (stopping !== null) {
      void stopping.finally(() => {
        speechBusy.current = false;
      });
      return;
    }
    if (listenState === 'listening') {
      void listenStop()
        .catch(() => raise('Stop failed'))
        .finally(() => {
          speechBusy.current = false;
        });
      return;
    }
    if (showInputRow) {
      void dictation.start().finally(() => {
        speechBusy.current = false;
      });
      return;
    }
    setListenWanted(true);
    void windowSetChatOpen(true).catch(() => {});
    void listenStart()
      .then((next) => setListenState(next.state))
      .catch((e: unknown) =>
        // The invoke message is already curated ('no audio source
        // available', a needs_setup reason) — surface it like
        // useDictation does instead of a bare 'Listen failed'.
        raise(typeof e === 'string' && e ? e : 'Listen failed'),
      )
      .finally(() => {
        speechBusy.current = false;
      });
  };

  const row = () => {
    if (bootError) {
      return (
        <BootErrorRow
          className={rowCls}
          onRetry={() => void bootstrap()}
        />
      );
    }
    if (gate === 'needs_permission') {
      return (
        <PermissionRow
          className={rowCls}
          busy={busy}
          onGrant={() => void grantScreen()}
        />
      );
    }
    // `main` (and the null boot frame — the same capsule, inert until
    // the gate resolves).
    return (
      <form
        onSubmit={submitAsk}
        className={rowCls}
        data-tauri-drag-region>
        <IrisButton
          active={
            listenState === 'listening' || dictation.state === 'listening'
          }
          label={
            cardOpen ? 'Close chat' : open ? 'Back to capsule' : 'Ask Marvis'
          }
          onPress={() =>
            cardOpen
              ? void askClose().catch(() => {})
              : open
                ? collapse()
                : setOpen(true)
          }
          disabled={gate !== 'main'}
        />
        <AskInput
          ref={inputRef}
          value={text}
          cardOpen={cardOpen}
          visible={showInputRow}
          onChange={dictation.handleChange}
          onSelect={dictation.handleSelect}
          onFocus={() => gate === 'main' && setOpen(true)}
          onSubmit={submitAsk}
        />
        {/* Collapsed-only recorders (`barControls(false)`): the screen
            capture toggle and meeting Listen. The expanded row renders
            neither — it gets dictation + settings instead. */}
        {controls.includes('capture') && (
          <BarButton
            label={captureLabel}
            pressed={captureRunning}
            disabled={gate !== 'main'}
            onPress={toggleCapture}
            className={cn(
              'relative',
              captureRunning && 'bg-accent-soft text-accent',
            )}>
            <MonitorDotIcon className='size-5' />
            {/* Recording badge — the shell pulse deliberately ignores
                capture (it runs by default, so it would pulse
                permanently); this corner ping carries the signal. */}
            {captureRunning && (
              <span
                aria-hidden
                className='animate-capture-ping absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-accent/50'
              />
            )}
          </BarButton>
        )}
        {controls.includes('listen') && (
          <BarButton
            label={micLabel}
            pressed={listenState === 'listening'}
            disabled={gate !== 'main'}
            onPress={pressMic}>
            <MicAudioLinesIcon className='size-5' />
          </BarButton>
        )}
        {/* Expanded-only dictation (`barControls(true)`) — the same
            `pressMic` route, landing on its `showInputRow` branch. */}
        {showInputRow && dictation.state === 'listening' && (
          <DictationWaveform />
        )}

        {controls.includes('dictation') && (
          <BarButton
            label={micLabel}
            pressed={dictation.state === 'listening'}
            disabled={gate !== 'main'}
            onPress={pressMic}>
            <MicIcon className='size-5' />
          </BarButton>
        )}

        {/* Only rendered in the input row — the idle capsule has no
            room for a fourth control (tray menu + Cmd+, reach it
            anyway). */}
        {controls.includes('settings') && (
          <BarButton
            label='Settings'
            title='Settings (⌘,)'
            disabled={gate !== 'main'}
            onPress={() => void windowShowSettings().catch(() => {})}>
            <SettingsIcon className='size-5' />
          </BarButton>
        )}
      </form>
    );
  };

  return (
    <div
      ref={stageRef}
      className={cn(
        'group/stage glass-stage flex h-full flex-col p-1',
        growDir === 'up' ? 'justify-end' : 'justify-start',
      )}
      data-pos={growDir === 'up' ? 'bottom' : 'top'}
      data-dir={growDir}>
      <div
        ref={cardRef}
        className={cn(
          'group/bar glass-surface relative flex w-full select-none',
          cardOpen
            ? // `flex-1 min-h-0` (not `flex-none`): the card tracks the
              // window through the expand animation, so its bottom edge —
              // and the ShineBorder ring — is never clipped mid-grow; the
              // scroll body absorbs the slack.
              cn(
                PANEL,
                'min-h-0 flex-1',
                growDir === 'up' ? 'flex-col-reverse' : 'flex-col',
              )
            : 'h-full flex-none flex-col justify-center rounded-full bg-[color-mix(in_oklch,var(--surface)_80%,transparent)] backdrop-blur-[14px] transition-[border-color,box-shadow] duration-(--motion-base) ease-(--ease) motion-reduce:transition-none',
          // Activity pulse is scoped to the collapsed pill — never the
          // form row or the open card — so row sizing, control placement,
          // and card content don't move. `.animate-pulse` is stilled by
          // the reduced-motion query.
          activeWork && !cardOpen && 'animate-pulse',
        )}
        data-expanded={showInputRow || undefined}
        data-tauri-drag-region={cardOpen ? undefined : 'deep'}>
        {row()}
        {showIntro && <LaunchIntro onDone={() => setIntroDone(true)} />}
        {section === 'chat' && <ChatSection />}
        {section === 'listen' && <ListenSection />}
        {/* Capsule shimmer — accent duotone follows light/dark via the
            tokens; masked to the border ring, pointer-events-none. */}
        <ShineBorder
          shineColor={['#A07CFE', '#FE8FB5', '#FFBE7B', 'var(--accent)']}
          borderWidth={1.8}
        />
      </div>
    </div>
  );
};

export default Bar;
