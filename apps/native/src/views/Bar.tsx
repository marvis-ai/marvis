/**
 * The always-on-top bar (`?view=bar`) — the UNIFIED window. Two shapes:
 *
 *  - Pill modes (mini | input | permission): the 172⇄600×64 capsule⇄
 *    input morph — the capsule IS the window under liquid glass, so
 *    `expanded` reports to `window_set_bar_expanded` and Rust animates
 *    the width change. Unchanged mechanics.
 *  - Card modes (chat | listen | history): the same window grown to
 *    600×clamped(64+content) — one sizing band for every section,
 *    [30%, 60%] of the screen's available height (useCardGeometry
 *    reports it; Rust re-clamps). The bar row is the card's
 *    bottom-anchored footer under `flex-col-reverse`, with the section
 *    above it regardless of which way the window physically grows.
 *
 * `cardOpen` lives in `useCardGeometry` — read off the window
 * itself (`resize` is the only open/close signal).
 * Dictation is `useDictation`; the permission gate is `useGate`; the
 * background-activity mirrors are `useBarActivity`.
 *
 * `data-tauri-drag-region` lives on the card's chrome in card mode —
 * the bar ROW and the section CardHeader ('deep', so padding and
 * non-interactive children drag too) — a stage-level region would
 * intercept text selection in the scrollable conversation. In pill
 * mode it stays on the capsule chrome as before.
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
import {
  HistoryIcon,
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
  barContextMenu,
  capturePickBegin,
  captureStop,
  configGet,
  listenStart,
  raise,
  windowFocusBar,
  windowSetBarExpanded,
  windowSetChatOpen,
  windowShowSettings,
  type Config,
} from '@/lib/commands';
import {
  EV_BAR_SHOW_HISTORY,
  EV_BAR_START_LISTEN,
  EV_BAR_TOGGLE_INPUT,
  EV_CONFIG_CHANGED,
  useTauriEvent,
} from '@/lib/events';
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
import { HistorySection } from '@/components/HistorySection';
import { LaunchIntro } from '@/components/LaunchIntro';
import { ListenSection } from '@/components/ListenSection';
import type { ListenViewing } from '@/components/listen/model';
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

  // Section pin: an explicit user choice (send → chat, listen-start →
  // listen, stop → the finished doc, history capsule button → the
  // session list) overrides the `listenWanted` activity mirror.
  const [pinned, setPinned] = useState<'chat' | 'listen' | 'history' | null>(
    null,
  );
  const [listenViewing, setListenViewing] = useState<ListenViewing | null>(
    null,
  );

  const { cardOpen } = useCardGeometry(cardRef, stageRef);
  /** Live mirror of `cardOpen` for async callbacks — the card can
   *  collapse while a `listenStop` invoke is in flight, and a
   *  `viewing`/`pinned` write landing after that would outlive the
   *  close-time reset (the `cardOpen` effect only refires on
   *  transitions). */
  const cardOpenRef = useRef(cardOpen);
  cardOpenRef.current = cardOpen;
  const {
    listenWanted,
    setListenWanted,
    listenState,
    setListenState,
    captureRunning,
    setCaptureRunning,
    captureTarget,
    setCaptureTarget,
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
  const section: 'chat' | 'listen' | 'history' | null = !cardOpen
    ? null
    : (pinned ?? (listenWanted ? 'listen' : 'chat'));
  /** Whether the Ask `<input>` is actually mounted: the permission
   *  card and the boot-error retry replace the whole row while
   *  `showInputRow` stays true, and the history card drops the row —
   *  dictation keys off this. */
  const inputRendered =
    showInputRow &&
    section !== 'history' &&
    !bootError &&
    gate !== 'needs_permission';
  /** The row's control set for this surface — `bar-state.ts` owns the
   *  contract, the conditionals below consume it so the two can't
   *  drift. */
  const controls = barControls(showInputRow, cardOpen);

  const dictation = useDictation({
    text,
    textRef,
    setText,
    inputRef,
    inputRendered,
  });
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

  // The `start_listen` hotkey and the shared menu's Start Listening
  // item (lib.rs emits `bar:start-listen`). Start-only — the menu
  // disables the item while a session is live; a racing press no-ops.
  useTauriEvent(EV_BAR_START_LISTEN, () => {
    if (gate !== 'main') {
      return;
    }
    startListenSession();
  });

  // The `show_history` hotkey and the shared menu's History item —
  // the same surface as the capsule's History button.
  useTauriEvent(EV_BAR_SHOW_HISTORY, () => {
    if (gate !== 'main') {
      return;
    }
    setPinned('history');
    void windowSetChatOpen(true).catch(() => {});
  });

  // `window.bar_locked` — the persisted position lock. The Lock menu
  // item and the `toggle_lock` hotkey write it through
  // `set_bar_locked`, which broadcasts `config:changed`; read once on
  // mount for emits that raced the webview's load.
  const [barLocked, setBarLocked] = useState(false);
  useEffect(() => {
    void configGet()
      .then((cfg) => setBarLocked(cfg.window.bar_locked ?? false))
      .catch(() => {});
  }, []);
  useTauriEvent<Config>(EV_CONFIG_CHANGED, (cfg) => {
    setBarLocked(cfg.window.bar_locked ?? false);
  });

  // Every card open starts unpinned with no viewed session.
  useEffect(() => {
    if (!cardOpen) {
      setPinned(null);
      setListenViewing(null);
    }
  }, [cardOpen]);

  // Listen is an independent session: collapsing the card must not change
  // which active section is shown when it is reopened.

  // The capsule IS the window under liquid glass — the pill⇄input morph
  // resizes it (idle 172 ⇄ 600). While the card is open the morph is
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
   *  the render-time `text`. `withScreen` (the field's Cmd/Ctrl+Enter)
   *  forces a screen read even when the text shows no intent. */
  const sendAsk = (withScreen = false) => {
    const t = textRef.current.trim();
    if (!t) {
      return;
    }
    setText('');
    void askSend(t, withScreen).catch(() => raise('Send failed'));
  };

  // Submit = ask (a follow-up while the card is open). The backend
  // expands the window itself — no local collapse needed either way.
  // The flag only rides along when the handshake actually sends — a
  // live dictation still stops for review first, never auto-submits.
  const submitAsk = (withScreen = false) => {
    setPinned('chat');
    dictation.submit(() => sendAsk(withScreen));
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
    if (wantRunning) {
      // The custom share picker: hides the bar and shows candidate
      // thumbs — the pick (or cancel) lands via capture_pick_select /
      // capture_pick_cancel, so there's no status to check here.
      void capturePickBegin()
        .catch(() => raise(failure))
        .finally(() => setBusy(false));
      return;
    }
    // `capture_stop` resolves `{ running, frames, target }` rather than
    // rejecting, so a failed transition comes back short of the target
    // — surface it, then resync as usual.
    void captureStop()
      .then((next) => {
        if (next.running !== wantRunning) {
          raise(failure);
        }
        setCaptureRunning(next.running);
        setCaptureTarget(next.target);
      })
      .catch(() => raise(failure))
      .finally(() => setBusy(false));
  };

  const rowCls = cn(
    'flex min-h-0 w-full flex-none items-center gap-1.5',
    cardOpen
      ? cn(
          'min-h-16 border-border px-2.75',
          // The row is bottom-pinned under `flex-col-reverse`, so the
          // divider is always its top edge.
          'border-t',
        )
      : showInputRow
        ? 'px-2.75'
        : 'justify-center px-1.75',
  );

  /** The mic affordance's next action: stop the live dictation first,
   *  return to a live Listen's view (the section header owns Stop),
   *  dictate into the visible Ask input, or start meeting Listen from
   *  the collapsed capsule. Labels the collapsed Listen control and
   *  the expanded dictation control alike. */
  const micLabel =
    dictation.state === 'listening'
      ? 'Stop dictation'
      : listenState === 'listening' || listenState === 'paused'
        ? 'Show live listen'
        : showInputRow
          ? 'Dictate'
          : 'Listen';
  /** The capsule Listen control's "recording" mark — a paused session
   *  is still open (it resumes), so it stays lit too. The expanded
   *  dictation control shows the inverse: Listen owns the mic, so it
   *  greys out instead of marking itself live. */
  const micLive = listenState === 'listening' || listenState === 'paused';

  /** The collapsed screen-capture toggle's label — the control flips
   *  between starting and stopping the recorder. */
  const captureLabel = captureRunning
    ? 'Stop screen recording'
    : 'Start screen recording';

  /** Begin a meeting-Listen session — the tail of `pressMic`'s collapsed
   *  branch and `startListenSession`'s start path. The caller holds
   *  `speechBusy`; this chain releases it. */
  const beginListen = () => {
    setListenWanted(true);
    setPinned('listen');
    // A live start must never inherit a viewed doc — drop any stale one
    // so the section can't mount walled behind a finished session.
    setListenViewing(null);
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

  /** Always mints a fresh session — 'Start new' on a viewed doc is
   *  reachable while another session is live, and `listen_start`
   *  stops it server-side anyway. Live dictation owns the mic, so it
   *  is stopped first rather than left to reject the start. */
  const startNewListen = () => {
    if (speechBusy.current) return;
    speechBusy.current = true;
    const stopping = dictation.stopIfActive();
    if (stopping !== null) {
      void stopping.then(beginListen).catch(() => {
        speechBusy.current = false;
      });
      return;
    }
    beginListen();
  };

  /** Meeting-Listen start shared by the capsule's mic button and the
   *  `bar:start-listen` event (the hotkey + the shared menu item).
   *  Start-only — a live or paused session is left alone. */
  const startListenSession = () => {
    if (listenState === 'listening' || listenState === 'paused') return;
    startNewListen();
  };

  /** Shared press route for the split mic controls — collapsed Listen
   *  (`MicAudioLinesIcon`) and expanded dictation (`MicIcon`). A live
   *  dictation stops first under `speechBusy` serialization; a live
   *  Listen session navigates back to its view instead of stopping —
   *  leaving the section never stopped the recorder, so the button is
   *  the way back in (Stop stays on the listen header). */
  const pressMic = () => {
    if (speechBusy.current) return;
    speechBusy.current = true;
    // An active dictation stops first — `speechBusy` stays held
    // until it settles so a follow-up press can't start the
    // other mode mid-teardown.
    const stopping = dictation.stopIfActive();
    if (stopping !== null) {
      void stopping.finally(() => {
        speechBusy.current = false;
      });
      return;
    }
    if (listenState === 'listening' || listenState === 'paused') {
      // A paused session is still live (backend `is_listening()`) —
      // `null` viewing lands the card on the live session, not a doc.
      setListenViewing(null);
      setPinned('listen');
      void windowSetChatOpen(true).catch(() => {});
      speechBusy.current = false;
      return;
    }
    if (showInputRow) {
      void dictation.start().finally(() => {
        speechBusy.current = false;
      });
      return;
    }
    beginListen();
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
        onSubmit={(e) => {
          e.preventDefault();
          submitAsk();
        }}
        className={rowCls}
        data-tauri-drag-region='deep'>
        {/* Absent while the card is open — the section header owns
            Back/Close, so the footer's row is input + dictation only. */}
        {controls.includes('iris') && (
          <IrisButton
            active={
              listenState === 'listening' ||
              listenState === 'paused' ||
              dictation.state === 'listening'
            }
            label={open ? 'Back to capsule' : 'Ask Marvis'}
            onPress={() => (open ? collapse() : setOpen(true))}
            disabled={gate !== 'main'}
          />
        )}
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
            title={captureRunning ? captureTarget?.label : undefined}
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
            pressed={micLive}
            disabled={gate !== 'main'}
            onPress={pressMic}
            className={cn('relative', micLive && 'bg-accent-soft text-accent')}>
            <MicAudioLinesIcon className='size-5' />
            {/* Same corner-ping idiom as the capture badge — a live
                session keeps recording after the card collapses, so
                the idle pill must still show it. */}
            {listenState === 'listening' && (
              <span
                aria-hidden
                className='animate-capture-ping absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-accent/50'
              />
            )}
          </BarButton>
        )}
        {/* Collapsed-only history opener — the card opens on the
            session list. */}
        {controls.includes('history') && (
          <BarButton
            label='History'
            disabled={gate !== 'main'}
            onPress={() => {
              setPinned('history');
              void windowSetChatOpen(true).catch(() => {});
            }}>
            <HistoryIcon className='size-5' />
          </BarButton>
        )}
        {/* Expanded-only dictation (`barControls(true)`) — the same
            `pressMic` route, landing on its `showInputRow` branch. */}
        {showInputRow && dictation.state === 'listening' && (
          <DictationWaveform />
        )}

        {controls.includes('dictation') && (
          <BarButton
            label={
              dictation.state === 'listening' ? 'Stop dictation' : 'Dictate'
            }
            title={
              micLive ? 'Dictate — a listen session owns the mic' : undefined
            }
            pressed={dictation.state === 'listening'}
            disabled={gate !== 'main' || micLive}
            onPress={pressMic}
            className={cn(
              'relative',
              dictation.state === 'listening' && 'bg-accent-soft text-accent',
            )}>
            <MicIcon className='size-5' />
            {dictation.state === 'listening' && (
              <span
                aria-hidden
                className='animate-capture-ping absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-accent/50'
              />
            )}
          </BarButton>
        )}

        {/* Only rendered in the pill's input row — the idle capsule
            has no room for a fourth control and the card header
            carries its own Settings (tray menu + Cmd+, reach it
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
      onContextMenu={(e) => {
        e.preventDefault();
        // The shared menu is the idle capsule's surface — expanded
        // rows and open cards have their own chrome.
        if (gate === 'main' && !showInputRow) {
          void barContextMenu().catch(() => {});
        }
      }}
      onMouseDown={(e) => {
        // Locked bar: Tauri drags on a document-level `mousedown`
        // bubble listener (src/window/scripts/drag.js) — stopping the
        // event here keeps it from reaching that handler, covering
        // every drag region at once. Children see the mousedown first,
        // so buttons and text selection are unaffected.
        if (barLocked) {
          e.stopPropagation();
        }
      }}
      className={cn(
        'group/stage glass-stage flex h-full flex-col justify-end p-1',
      )}>
      <div
        ref={cardRef}
        className={cn(
          'group/bar glass-surface relative flex w-full select-none',
          cardOpen
            ? // `flex-1 min-h-0` (not `flex-none`): the card tracks the
              // window through the expand animation, so its bottom edge —
              // and the ShineBorder ring — is never clipped mid-grow; the
              // scroll body absorbs the slack.
              cn(PANEL, 'min-h-0 flex-1 flex-col-reverse')
            : 'h-full flex-none flex-col justify-center rounded-full bg-[color-mix(in_oklch,var(--surface)_80%,transparent)] backdrop-blur-[14px] transition-[border-color,box-shadow] duration-(--motion-base) ease-(--ease) motion-reduce:transition-none',
          // Activity pulse is scoped to the collapsed pill — never the
          // form row or the open card — so row sizing, control placement,
          // and card content don't move. `.animate-pulse` is stilled by
          // the reduced-motion query.
          activeWork && !cardOpen && 'animate-pulse',
        )}
        data-expanded={showInputRow || undefined}
        data-tauri-drag-region={cardOpen ? undefined : 'deep'}>
        {/* The expanded sections share the bottom input row — history
            is a pure picker, so its card drops the row (its own header
            carries Back + Settings). */}
        {section !== 'history' && row()}
        {showIntro && <LaunchIntro onDone={() => setIntroDone(true)} />}
        {section === 'chat' && (
          <ChatSection
            onBack={() => void windowSetChatOpen(false).catch(() => {})}
          />
        )}
        {section === 'listen' && (
          <ListenSection
            viewing={listenViewing}
            onStartNew={startNewListen}
            onSessionEnded={(v) => {
              if (!cardOpenRef.current) return;
              setListenViewing(v);
              setPinned('listen');
            }}
            onBack={() => {
              if (listenViewing) {
                // A finished doc can only be reached from History —
                // Back returns to the list.
                setListenViewing(null);
                setPinned('history');
                return;
              }
              void windowSetChatOpen(false).catch(() => {});
            }}
          />
        )}
        {section === 'history' && (
          <HistorySection
            askBusy={askState !== 'idle'}
            onOpenChat={() => setPinned('chat')}
            onOpenListen={(v) => {
              setListenViewing(v);
              setPinned('listen');
            }}
            onBack={() => {
              // Back collapses the card to the idle capsule.
              void windowSetChatOpen(false).catch(() => {});
            }}
          />
        )}
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
