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
import {
  Suspense,
  lazy,
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
} from 'react';
import {
  HistoryIcon,
  MicAudioLinesIcon,
  MicIcon,
  MonitorDotIcon,
  SettingsIcon,
  ShineBorder,
  WandSparklesIcon,
  XIcon,
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
  listenStatus,
  presetsPaletteClose,
  presetsPaletteKey,
  presetsPaletteOpen,
  presetsPaletteQuery,
  raise,
  windowFocusBar,
  windowSetBarExpanded,
  windowSetChatOpen,
  windowShowSettings,
  type Config,
  type Preset,
} from '@/lib/commands';
import {
  EV_BAR_SHOW_HISTORY,
  EV_BAR_START_LISTEN,
  EV_BAR_TOGGLE_INPUT,
  EV_CONFIG_CHANGED,
  EV_PALETTE_CLOSED,
  EV_PRESET_PICK,
  useTauriEvent,
} from '@/lib/events';
import { barControls, hasActiveWork } from '@/lib/bar-state';
import {
  normalizeImageFile,
  type PendingAskImage,
} from '@/lib/image-attachments';
import { useBarActivity } from '@/hooks/useBarActivity';
import { useCardGeometry } from '@/hooks/useCardGeometry';
import { useDictation } from '@/hooks/useDictation';
import { useGate } from '@/hooks/useGate';
import { usePresets } from '@/hooks/usePresets';
import { AskAttachments } from '@/components/bar/AskAttachments';
import { AskInput } from '@/components/bar/AskInput';
import { BarButton } from '@/components/bar/BarButton';
import { BootErrorRow } from '@/components/bar/BootErrorRow';
import { DictationWaveform } from '@/components/bar/DictationWaveform';
import { IrisButton } from '@/components/bar/IrisButton';
import { PermissionRow } from '@/components/bar/PermissionRow';
import { ChatSection } from '@/components/ChatSection';
import { HistorySection } from '@/components/HistorySection';
import { ListenSection } from '@/components/ListenSection';
import type { ListenViewing } from '@/components/listen/model';
import { PANEL } from '@/lib/classes';
import { caretViewportX } from '@/lib/caret';
import {
  expandTemplate,
  hasLangParam,
  isTemplate,
  langName,
  resolveSlash,
  slashQuery,
  slashToken,
  stripSlashToken,
} from '@/lib/presets';

const LaunchIntro = lazy(() =>
  import('@/components/LaunchIntro').then((m) => ({
    default: m.LaunchIntro,
  })),
);

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
  /** Composer image attachments — normalized JPEGs awaiting the next
   *  send. The ref mirror is the race-safe read for `addFiles`'s async
   *  loop (same pattern as `textRef`), and `dropActive` drives the
   *  field ring while a file drag hovers the form. */
  const [pendingImages, setPendingImagesState] = useState<PendingAskImage[]>(
    [],
  );
  const pendingImagesRef = useRef<PendingAskImage[]>([]);
  const setPendingImages = (value: PendingAskImage[]) => {
    pendingImagesRef.current = value;
    setPendingImagesState(value);
  };
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const [dropActive, setDropActive] = useState(false);
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
  /** The merged preset list (built-ins + customs) behind the palette
   *  and the `/name` shorthand — `usePresets` fetches on mount and
   *  refetches on `config:changed`. `armed` is the per-send preset
   *  badge, `langParam` its editable `{lang}` value (null when the
   *  armed text has no `{lang}`). `mainLang` seeds the badge's
   *  default. */
  const presets = usePresets();
  const [armedPreset, setArmedPresetState] = useState<Preset | null>(null);
  const [langParam, setLangParamState] = useState<string | null>(null);
  const [editingLang, setEditingLang] = useState(false);
  /** Whether the preset palette is up — set on our opens, cleared by
   *  `bar:palette-closed` (every hide emits it). While true the
   *  composer's `/token` is its live filter (`palette:query`) and its
   *  nav keys forward over `palette:key`. */
  const [paletteOpen, setPaletteOpen] = useState(false);
  const paletteOpenRef = useRef(false);
  const setPalette = (v: boolean) => {
    paletteOpenRef.current = v;
    setPaletteOpen(v);
  };
  /** Live mirrors for `sendAsk`/`onDisarm` — `dictation.submit` can
   *  defer the send behind a settling stop, so render-time state
   *  would read stale there (same shape as `textRef`). */
  const armedPresetRef = useRef<Preset | null>(null);
  const langParamRef = useRef<string | null>(null);
  /** The pre-edit `{lang}` value — the mini field's Esc reverts to it. */
  const langRevert = useRef<string | null>(null);
  const setArmedPreset = (p: Preset | null) => {
    armedPresetRef.current = p;
    setArmedPresetState(p);
  };
  const setLangParam = (v: string | null) => {
    langParamRef.current = v;
    setLangParamState(v);
  };
  /** Arm a preset: its badge plus — when the text carries `{lang}` —
   *  a second, editable language badge seeded from the main language. */
  const armPreset = (p: Preset) => {
    setArmedPreset(p);
    setLangParam(hasLangParam(p) ? langName(mainLang) : null);
    setEditingLang(false);
  };
  /** Full disarm — the badge group goes together (✕, Esc, a caret-0
   *  Backspace/Delete, or a fired send). */
  const disarmPreset = () => {
    setArmedPreset(null);
    setLangParam(null);
    setEditingLang(false);
  };
  const [mainLang, setMainLang] = useState('en');

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
  /** Live mirror of `inputRendered` for `bar:preset-pick` — the event
   *  can land between a render and the listener's closure refresh
   *  (a pick on a surface whose composer just unmounted must no-op,
   *  same as the context-menu gate). */
  const inputRenderedRef = useRef(inputRendered);
  inputRenderedRef.current = inputRendered;
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
      .then((cfg) => {
        setBarLocked(cfg.window.bar_locked ?? false);
        setMainLang(cfg.app.main_language);
      })
      .catch(() => {});
  }, []);
  useTauriEvent<Config>(EV_CONFIG_CHANGED, (cfg) => {
    setBarLocked(cfg.window.bar_locked ?? false);
    setMainLang(cfg.app.main_language);
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
  // `Cmd`/`Ctrl+,` opens settings — a bar-local key (fires only while
  // this window is focused), not a global hotkey.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (
        e.key === ',' &&
        (e.metaKey || e.ctrlKey) &&
        !e.altKey &&
        !e.shiftKey
      ) {
        e.preventDefault();
        void windowShowSettings().catch(() => {});
        return;
      }
      if (e.key === 'Escape') {
        // An open palette owns Esc — dismisses it first, never the
        // bar. The composer's keydown already forwards it (and stops
        // bubbling); this covers Esc landing while focus sits
        // elsewhere in the bar.
        if (paletteOpenRef.current) {
          void presetsPaletteKey('Escape').catch(() => {});
          return;
        }
        if (cardOpen) {
          void askClose().catch(() => {});
          return;
        }
        // Esc discards the field — a pending stop's returned draft must
        // not land in the cleared text, and a live anchor stops without
        // applying. The armed preset badges disarm with the field.
        dictation.discard();
        disarmPreset();
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
        // `/` wakes straight into the preset palette — the field
        // mounts on this render, so the caret anchor waits a frame.
        if (e.key === '/') {
          requestAnimationFrame(() => openPalette(false));
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [gate, open, cardOpen]);

  const collapse = () => {
    setOpen(false);
    inputRef.current?.blur();
    // The field unmounts under an open palette — its `/token` is gone.
    if (paletteOpenRef.current) {
      void presetsPaletteClose().catch(() => {});
    }
  };

  /** Open the preset palette — left-aligned to the input caret (the
   *  Rust side turns the viewport x into a screen anchor; a missing
   *  field falls back to the pointer/bar center). `focused`: wand and
   *  right-click opens take key focus; a `/`-typed open leaves the
   *  composer key so the `/token` keeps filtering live (`query` seeds
   *  it from the field's current token). */
  const openPalette = (focused: boolean) => {
    // The Rust side gate-checks too, but an off-Main invoke would drop
    // the open — and a stuck `paletteOpen` swallows composer keys.
    if (gate !== 'main' || !inputRenderedRef.current) return;
    setPalette(true);
    const el = inputRef.current;
    // The card's composer is a bottom-anchored footer, so its row top
    // is the palette's pop-above edge (viewport px — Rust adds the
    // window's screen position, same as the caret x). Collapsed opens
    // send it too; the pill's side pick ignores it.
    const anchorY = el?.closest('form')?.getBoundingClientRect().top;
    void presetsPaletteOpen(
      el ? caretViewportX(el) : undefined,
      anchorY,
      slashQuery(textRef.current) ?? undefined,
      focused,
    ).catch(() => setPalette(false));
  };
  /** Palette hid itself (pick, Esc, blur, bar blur) — stop key
   *  forwarding and query pushes. */
  useTauriEvent(EV_PALETTE_CLOSED, () => setPalette(false));

  /** Place the caret at the end of the field after a programmatic
   *  setText — the DOM value lands on render, so selection waits a
   *  frame. */
  const focusFieldEnd = () => {
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    });
  };

  /** Palette pick (`bar:preset-pick`) — every preset arms as a badge
   *  (+ its `{lang}` param badge when the text has one); expansion
   *  waits for send. A `/token` sitting in the field is the palette's
   *  trigger text — the badge replaces it, and a `/name args`+Enter
   *  sends straight through (the pick is the run — coding-agent
   *  parity); a bare `/name` just arms for editing. Paused while
   *  dictation owns the field. */
  const applyPreset = (p: Preset) => {
    dictation.discard();
    armPreset(p);
    if (textRef.current.startsWith('/')) {
      const rest = stripSlashToken(textRef.current);
      setText(rest);
      focusFieldEnd();
      if (rest.trim()) {
        submitAsk(false, { skipSlashResolution: true });
      }
      return;
    }
    inputRef.current?.focus();
  };
  useTauriEvent<Preset>(EV_PRESET_PICK, (p) => {
    // The palette outlives the webview's render — a pick landing
    // after the composer unmounted (history card, gate/error rows)
    // must no-op rather than mutate a hidden field.
    if (dictation.state === 'listening' || !inputRenderedRef.current) {
      return;
    }
    applyPreset(p);
  });

  /** Slash shorthand: an exact `/name` token followed by a space
   *  applies on the spot (end-of-text tokens wait for send — a prefix
   *  name can't swallow a longer one mid-typing). A `/` arriving at
   *  caret-0 pops the palette UNFOCUSED — this field keeps focus and
   *  the token streams over `palette:query` as its live filter,
   *  coding-agent slash-menu style; leaving the `/` prefix closes it. */
  const onFieldChange = (e: ChangeEvent<HTMLTextAreaElement>) => {
    dictation.handleChange(e);
    if (dictation.state === 'listening') return;
    const v = e.target.value;
    if (v.startsWith('/') && !text.startsWith('/')) {
      openPalette(false);
      return;
    }
    if (paletteOpenRef.current) {
      const s = slashToken(v);
      if (!s) {
        // The `/` was deleted (or the caret context left it) — the
        // menu's reason to exist is gone.
        void presetsPaletteClose().catch(() => {});
      } else {
        void presetsPaletteQuery(s.token).catch(() => {});
      }
    }
    const hit = resolveSlash(v, presets, false);
    if (!hit) return;
    dictation.discard();
    armPreset(hit.preset);
    setText(hit.rest);
    if (paletteOpenRef.current) {
      void presetsPaletteClose().catch(() => {});
    }
  };

  /** Normalize picked/dropped files into the pending strip. Each file
   *  is checked against the pending count as it lands (`next.length`)
   *  so a multi-file drop stops at four; a rejection aborts the rest of
   *  the batch and surfaces its reason beside the chips. Attaching
   *  wakes the input row — same affordance as typing a character. */
  const addFiles = async (files: File[]) => {
    if (files.length === 0) return;
    if (gate === 'main') setOpen(true);
    setAttachmentError(null);
    const next = [...pendingImagesRef.current];
    try {
      for (const file of files) {
        next.push(await normalizeImageFile(file, next.length));
      }
      setPendingImages(next);
    } catch (error) {
      setAttachmentError(
        error instanceof Error ? error.message : 'Could not add image',
      );
    }
  };

  const removePendingImage = (index: number) =>
    setPendingImages(pendingImagesRef.current.filter((_, i) => i !== index));

  /** Send a question — the field's text by default, or an explicit one
   *  (`question`, e.g. a summary follow-up chip — the field's draft is
   *  untouched then). Field text reads off `textRef` so the Enter
   *  queued behind a settling stop sees the applied final draft, not
   *  the render-time `text`. `withScreen` (the field's Cmd/Ctrl+Enter)
   *  forces a screen read even when the text shows no intent. A send
   *  from the listen card binds to that doc's own chat — the viewed
   *  session's id, else the live session's. */
  const sendAsk = async (
    withScreen = false,
    question?: string,
    { skipSlashResolution = false } = {},
  ) => {
    let preset = armedPresetRef.current;
    let slashInput: string | undefined;
    if (question === undefined && !skipSlashResolution) {
      const raw = textRef.current;
      // Bare `/` opens the preset palette instead of sending a slash.
      if (raw.trim() === '/') {
        openPalette(false);
        return;
      }
      const hit = resolveSlash(raw, presets, true);
      if (hit) {
        const rest = hit.rest.trim();
        dictation.discard();
        if (!rest) {
          // `/name` alone arms the badge — the request is still to
          // come; the send below only runs once there's text.
          armPreset(hit.preset);
          setText('');
          inputRef.current?.focus();
          return;
        }
        preset = hit.preset;
        slashInput = rest;
        setText('');
      }
    }
    const t0 = (slashInput ?? question ?? textRef.current).trim();
    if (!t0) {
      return;
    }
    // An armed `{input}` preset expands around the typed text — the
    // bubble shows the resolved request (WYSIWYG at send, not pick).
    const t =
      preset && isTemplate(preset)
        ? expandTemplate(
            preset.text,
            t0,
            langParamRef.current ?? langName(mainLang),
          ).trim()
        : t0;
    if (question === undefined) {
      setText('');
    }
    const presetLang =
      preset && hasLangParam(preset)
        ? (langParamRef.current ?? undefined)
        : undefined;
    disarmPreset(); // one-shot: the badges clear when a send fires
    let listenId = section === 'listen' ? listenViewing?.id : undefined;
    if (section === 'listen' && listenId === undefined) {
      listenId =
        (await listenStatus()
          .then((s) => s.session_id)
          .catch(() => null)) ?? undefined;
    }
    // Pending images ride along with the question — strip the
    // previewUrl so only the normalized payload crosses IPC. They
    // clear once the send is actually initiated; a rejected invoke
    // keeps them so a retry doesn't silently lose user attachments.
    const attachments = pendingImagesRef.current.map(
      ({ name, jpegBase64 }) => ({ name, jpegBase64 }),
    );
    void askSend(t, {
      withScreen,
      listenId,
      presetId: preset?.id,
      presetLang,
      attachments,
    })
      .then(() => setPendingImages([]))
      .catch(() => raise('Send failed'));
  };

  // Submit = ask (a follow-up while the card is open). The backend
  // expands the window itself — no local collapse needed either way.
  // The flag only rides along when the handshake actually sends — a
  // live dictation still stops for review first, never auto-submits.
  const submitAsk = (
    withScreen = false,
    options: { skipSlashResolution?: boolean } = {},
  ) => {
    setPinned('chat');
    dictation.submit(() => void sendAsk(withScreen, undefined, options));
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
  /** The composer form's own chrome — a column so the pending-image
   *  strip can sit above the row in card mode; the row layout itself
   *  moves to the inner div (`innerRowCls`). */
  const formCls = cn(
    'flex min-h-0 w-full flex-none flex-col',
    cardOpen && 'border-t border-border',
  );
  const innerRowCls = cn(
    'flex min-w-0 flex-1 items-center gap-1.5',
    cardOpen
      ? 'min-h-16 px-2.75'
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

  /** One mic action under `speechBusy` — no-ops while a start/stop is
   *  in flight, then stops a live dictation before running `next`.
   *  `stoppedDictation` marks a press whose whole work was that stop;
   *  whatever runs last owns the release. */
  const withSpeechLock = (next: (stoppedDictation: boolean) => void) => {
    if (speechBusy.current) return;
    speechBusy.current = true;
    const stopping = dictation.stopIfActive();
    if (stopping !== null) {
      void stopping
        .then(() => next(true))
        .catch(() => {
          speechBusy.current = false;
        });
      return;
    }
    next(false);
  };

  /** Always mints a fresh session. `listen_start` stops an existing
   *  session server-side, while live dictation owns the mic and is stopped
   *  first rather than left to reject the start. */
  const startNewListen = () => {
    withSpeechLock(() => beginListen());
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
    withSpeechLock((stoppedDictation) => {
      if (stoppedDictation) {
        // The press's whole action was stopping the live dictation.
        speechBusy.current = false;
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
        onSubmit={(e) => {
          e.preventDefault();
          submitAsk();
        }}
        /* Image drops land on the composer. `preventDefault` on
           dragover is what allows the DOM drop event at all, and
           `stopPropagation` keeps the drop from reaching Tauri's
           window-level file handling (or the deep drag region). */
        onDragOver={(e) => {
          e.preventDefault();
          e.stopPropagation();
          e.dataTransfer.dropEffect = 'copy';
          setDropActive(true);
        }}
        onDragEnter={(e) => {
          e.preventDefault();
          setDropActive(true);
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
            setDropActive(false);
          }
        }}
        onDrop={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setDropActive(false);
          void addFiles(
            Array.from(e.dataTransfer?.files ?? []).filter((file) =>
              file.type.startsWith('image/'),
            ),
          );
        }}
        className={formCls}
        data-tauri-drag-region='deep'>
        {/* Card mode: the pending strip rides above the row inside the
            bordered composer block. */}
        {cardOpen && (
          <AskAttachments
            images={pendingImages}
            error={attachmentError}
            onRemove={removePendingImage}
          />
        )}
        <div className={innerRowCls}>
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
          {/* The armed preset's badges — the name in accent, then its
            editable `{lang}` param in neutral when the text carries
            one. One-shot: ✕/Esc/caret-0 Backspace-Delete disarms, a
            fired send clears them. */}
          {showInputRow && armedPreset && (
            <span className='flex flex-none items-center gap-1 self-center rounded-full bg-accent-soft px-2 py-0.75 text-[11.5px] font-medium text-accent-text'>
              {armedPreset.name}
              <button
                type='button'
                aria-label={`Remove ${armedPreset.name} preset`}
                onClick={disarmPreset}
                className='-mr-0.5 rounded-full p-px text-accent-text/70 transition-colors duration-(--motion-fast) hover:text-accent-text focus-visible:outline-2 focus-visible:outline-accent'>
                <XIcon className='size-3' />
              </button>
            </span>
          )}
          {showInputRow && armedPreset && langParam !== null && (
            <span className='flex flex-none items-center self-center'>
              {editingLang ? (
                <input
                  autoFocus
                  size={Math.max(4, langParam.length + 1)}
                  value={langParam}
                  aria-label='Preset language'
                  onChange={(e) => setLangParam(e.target.value)}
                  onBlur={() => {
                    setEditingLang(false);
                  }}
                  onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      setEditingLang(false);
                      inputRef.current?.focus();
                    } else if (e.key === 'Escape') {
                      e.preventDefault();
                      if (langRevert.current !== null) {
                        setLangParam(langRevert.current);
                      }
                      setEditingLang(false);
                      inputRef.current?.focus();
                    }
                  }}
                  className='rounded-full bg-fg-soft px-2 py-0.75 text-[11.5px] font-medium text-foreground outline-none focus:shadow-(--focus-ring)'
                />
              ) : (
                <button
                  type='button'
                  title='Language for {lang} — click to change'
                  onClick={() => {
                    langRevert.current = langParam;
                    setEditingLang(true);
                  }}
                  className='rounded-full bg-fg-soft px-2 py-0.75 text-[11.5px] font-medium text-foreground transition-colors duration-(--motion-fast) hover:bg-[color-mix(in_oklch,var(--fg)_14%,transparent)] focus-visible:outline-2 focus-visible:outline-accent'>
                  {langParam}
                </button>
              )}
            </span>
          )}
          {/* Pill mode: no vertical room above the row — the same strip
            goes inline as a horizontal scroll sliver. */}
          {!cardOpen && showInputRow && (
            <AskAttachments
              inline
              images={pendingImages}
              error={attachmentError}
              onRemove={removePendingImage}
            />
          )}
          <AskInput
            ref={inputRef}
            value={text}
            cardOpen={cardOpen}
            visible={showInputRow}
            paletteOpen={paletteOpen}
            onPaletteKey={(key) => void presetsPaletteKey(key).catch(() => {})}
            onChange={onFieldChange}
            onSelect={dictation.handleSelect}
            onFocus={() => gate === 'main' && setOpen(true)}
            onSubmit={submitAsk}
            onDisarm={() => {
              if (!armedPresetRef.current) return false;
              dictation.discard();
              disarmPreset();
              return true;
            }}
            attachments={
              gate === 'main'
                ? {
                    onPick: () => setOpen(true),
                    onFiles: (files) => void addFiles(files),
                    dropActive,
                  }
                : undefined
            }
          />
          {/* Preset palette — the styled glass overlay beside the bar;
            picks arrive as bar:preset-pick. */}
          {showInputRow && (
            <BarButton
              label='Prompt presets'
              disabled={gate !== 'main'}
              onPress={() => openPalette(true)}>
              <WandSparklesIcon className='size-5' />
            </BarButton>
          )}
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
              className={cn(
                'relative',
                micLive && 'bg-accent-soft text-accent',
              )}>
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
            carries its own Settings (tray menu + Cmd/Ctrl+, reach it
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
        </div>
      </form>
    );
  };

  return (
    <div
      ref={stageRef}
      onContextMenu={(e) => {
        e.preventDefault();
        if (gate !== 'main') return;
        // The composer's right-click is the preset palette; surfaces
        // without one (idle capsule, history card, gate/error rows)
        // get the shared menu — a pick needs a field to land in.
        if (inputRendered) {
          openPalette(true);
        } else {
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
        {showIntro && (
          <Suspense fallback={null}>
            <LaunchIntro onDone={() => setIntroDone(true)} />
          </Suspense>
        )}
        {section === 'chat' && (
          <ChatSection
            onBack={() => void windowSetChatOpen(false).catch(() => {})}
          />
        )}
        {section === 'listen' && (
          <ListenSection
            viewing={listenViewing}
            onFollowUp={(q) => {
              // A summary chip asks the doc's own chat — `sendAsk`
              // resolves this session's `listenId` while the section
              // is still 'listen'.
              setPinned('chat');
              void sendAsk(false, q);
            }}
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
