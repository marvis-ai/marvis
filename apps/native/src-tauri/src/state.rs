//! `AppState` + [`Gate`] — the shared state commands, hotkey dispatch,
//! and deep links touch; managed via `app.manage(...)` in `crate::run`'s
//! `setup` and reached through `State<'_, AppState>` / `AppHandle`.

use crate::*;

/// Frame ring caps from the spec: 120 frames / 64 MB (~60 s horizon).
pub(crate) const RING_MAX_FRAMES: usize = 120;
pub(crate) const RING_MAX_BYTES: usize = 64 * 1024 * 1024;

/// Which UI state the bar may show. The card and capture exist only in
/// `Main` — the single global hotkey (show/hide) is chrome-level and
/// registered at startup either way. (Onboarding isn't a gate state —
/// it's a visibility overlay on top: `onboarding_done` is one of `Main`'s
/// two preconditions, so the wizard can never share the screen with the
/// bar's live machinery.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Onboarding incomplete OR no screen-recording permission — the
    /// permission card covers both (during onboarding the bar is hidden
    /// anyway, so the label never misleads).
    NeedsPermission,
    /// Fully live: card, capture.
    Main,
}

impl Gate {
    /// The `app:state` payload value.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Gate::NeedsPermission => "needs_permission",
            Gate::Main => "main",
        }
    }
}

/// Everything commands, hotkey dispatch, and deep links touch. Field
/// types follow the consumers: `Deps` borrows `&Mutex<…>` (so `keystore`,
/// `config`, `pool` are plain `Mutex`es) while `db`/`ring`/`ask`/`lifecycle`
/// are `Arc`s shared with spawned tasks and the capture callback.
pub struct AppState {
    pub(crate) keystore: Mutex<Keystore>,
    /// `Arc`d like `db`/`ring`: the scheduled memory extraction
    /// re-reads `[memory].enabled` from inside its spawned task —
    /// the snapshot `prepare_hook` checked can go stale while an ask
    /// streams or the extraction queues on the service gate.
    pub(crate) config: Arc<Mutex<Config>>,
    pub(crate) db: Arc<Db>,
    /// Shared per-session handoff boundary — Ask/compaction leases are
    /// coordinated with the production session deletion command.
    pub(crate) lifecycle: Arc<SessionLifecycle>,
    pub(crate) ring: Arc<Mutex<RingBuffer>>,
    pub(crate) capture: Mutex<Option<PlatformCapture>>,
    /// The settled-screen describer — fed by the capture callback and
    /// started/stopped alongside capture, so the ambient `[vision]`
    /// read only runs while frames actually flow.
    pub(crate) screen_reader: Arc<screen_read::ScreenReader>,
    /// The picker-selected scope reported by `capture:state` (`None` on
    /// the auto primary-display path). Written under the `capture`
    /// lock — it nests inside it, never the reverse.
    pub(crate) capture_target: Mutex<Option<CaptureTarget>>,
    /// Restart inputs, nested under `capture` just like `capture_target`.
    pub(crate) capture_restart: Mutex<Option<CaptureRestart>>,
    pub(crate) ask: Arc<AskService>,
    /// Serialized memory extraction (`[memory]`-gated) — shared with
    /// every ask send's `MemoryHook` schedule. Owned like `ask`/`listen`.
    pub(crate) memory: Arc<memory::MemoryService>,
    /// Serialized session compaction — shared with every ask send's
    /// detached `CompactHook` schedule.
    pub(crate) compact: Arc<ask::CompactService>,
    pub(crate) listen: Arc<ListenService>,
    pub(crate) dictation: Arc<DictationService>,
    /// Whisper CLI staged beside the executable by `externalBin`.
    pub(crate) bundled_whisper: Option<std::path::PathBuf>,
    pub(crate) pool: Mutex<WindowPool>,
    /// Currently live set — delta-swapped in place by [`swap_hotkeys`]
    /// (shared pairs are never re-registered; macOS refuses duplicates).
    pub(crate) hotkeys: Mutex<Option<RegisteredHotkeys>>,
    pub(crate) gate: Mutex<Gate>,
    /// Serializes `transition_gate` — the gate swap, its side-effects
    /// (enter/leave Main), and the `app:state` emit must be one critical
    /// section or two overlapping transitions can interleave (stale
    /// emit landing last, or hotkeys/capture from a dead gate state).
    /// The other holders — `capture_start`, `toggle_capture`, and the
    /// picker's `Picked` callback — keep their gate check + capture
    /// start/stop atomic against a concurrent leave-transition the same
    /// way. All holders take locks in the order
    /// `gate_transition` → `gate` → `capture` → `ring`
    /// (`capture_target` nests inside `capture`), so they cannot
    /// deadlock.
    pub(crate) gate_transition: Mutex<()>,
    /// Serializes `listen_start`/`dictation_start` end-to-end — the peer
    /// status check, the mic-permission await, and the service start
    /// must be one critical section or two concurrent first-starts can
    /// both see the peer idle and double-open the microphone. Async so
    /// the guard can be held across the permission await while the
    /// command futures stay `Send`. Listen stop also takes it; dictation's
    /// start/stop epoch already makes a mid-flight start lose safely.
    pub(crate) speech_lifecycle: tokio::sync::Mutex<()>,
    /// Payload of the alert toast currently on screen (`None` when
    /// dismissed). Kept server-side so the toast can re-read it on mount
    /// via `alert_current` — an `alert:show` emit that races the
    /// webview's listener would otherwise be lost.
    pub(crate) alert: Mutex<Option<serde_json::Value>>,
    pub(crate) voice_models: voice_models::VoiceModelManager,
    pub(crate) sherpa_models: sherpa_models::SherpaModelManager,
    /// Owns the voice-enrollment recording lifecycle (mic capture +
    /// voiceprint persistence). Self-contained — no cross-field locking.
    pub(crate) voice_enroll: voiceprint::VoiceEnroll,
}

impl AppState {
    /// The ask pipeline's borrow bundle: `db`/`ring`/`reader`/`memory`/
    /// `lifecycle`/`config` clone their `Arc`s (the spawned stream outlives the
    /// call), the rest are short-lived `&Mutex` borrows used only in
    /// pre-flight. `capture_running` snapshots the capture slot so
    /// `resolve_screen` knows whether ring frames are fresh.
    pub(crate) fn deps(&self) -> ask::Deps<'_> {
        ask::Deps {
            db: Arc::clone(&self.db),
            lifecycle: Arc::clone(&self.lifecycle),
            ring: Arc::clone(&self.ring),
            reader: Arc::clone(&self.screen_reader),
            memory: Arc::clone(&self.memory),
            compact: Arc::clone(&self.compact),
            config: Arc::clone(&self.config),
            capture_running: self
                .capture
                .lock()
                .as_ref()
                .is_some_and(PlatformCapture::is_running),
            keystore: &self.keystore,
            pool: &self.pool,
        }
    }

    /// `app.onboarding_done` — the wizard-completion flag that both the
    /// gate (`app_gate`) and the bar's visibility rule read.
    pub(crate) fn onboarding_done(&self) -> bool {
        self.config.lock().app.onboarding_done
    }

    /// The gate read `WindowPool::set_chat_open` needs — opening the
    /// card is `Main`-only.
    pub(crate) fn gate_is_main(&self) -> bool {
        *self.gate.lock() == Gate::Main
    }

    /// The configured `#rrggbb` accent — the bar's liquid-glass tint
    /// derives from it, so `config_set` re-applies the effect on
    /// `app.accent` writes.
    pub(crate) fn accent(&self) -> String {
        self.config.lock().app.accent.clone()
    }

    /// The single source of truth for bar visibility. Re-reads
    /// `onboarding_done` and asks the pool to reconcile — the bar floats
    /// only when onboarding is done AND the wizard isn't on screen
    /// (a re-run hides it again until the prefs window closes). Called by
    /// every prefs show/hide path and by `config_set` writes to
    /// `app.onboarding_done`.
    pub(crate) fn sync_bar_visibility(&self) {
        let done = self.onboarding_done();
        self.pool.lock().set_bar_shown(done);
    }

    /// Headless constructor for tests: filesystem-bound fields bind under
    /// `root` so tests never touch `~/.marvis`; windows/capture/hotkeys
    /// need a runtime, so the pool is empty and those slots stay `None`.
    /// The keystore is a plain plaintext file — no crypto in tests.
    #[cfg(test)]
    pub(crate) fn for_test(root: &std::path::Path) -> Self {
        Self {
            keystore: Mutex::new(Keystore::at(root.join("keys.json"))),
            config: Arc::new(Mutex::new(
                Config::load_from(root.join("config.toml")).unwrap_or_default(),
            )),
            db: Arc::new(Db::at(root.join("marvis.db")).expect("test db")),
            lifecycle: SessionLifecycle::new(),
            ring: Arc::new(Mutex::new(RingBuffer::new(RING_MAX_FRAMES, RING_MAX_BYTES))),
            capture: Mutex::new(None),
            screen_reader: Arc::new(screen_read::ScreenReader::new()),
            capture_target: Mutex::new(None),
            capture_restart: Mutex::new(None),
            ask: Arc::new(AskService::new()),
            memory: memory::MemoryService::new(),
            compact: ask::CompactService::new(),
            listen: Arc::new(ListenService::new()),
            dictation: Arc::new(DictationService::new()),
            bundled_whisper: None,
            pool: Mutex::new(WindowPool::new_empty()),
            hotkeys: Mutex::new(None),
            gate: Mutex::new(Gate::NeedsPermission),
            gate_transition: Mutex::new(()),
            speech_lifecycle: tokio::sync::Mutex::new(()),
            alert: Mutex::new(None),
            voice_models: voice_models::VoiceModelManager::at(
                root.join("models").join("whisper").join("models"),
            ),
            sherpa_models: sherpa_models::SherpaModelManager::at(
                root.join("models").join("sherpa").join("models"),
            ),
            voice_enroll: voiceprint::VoiceEnroll::new(),
        }
    }
}
