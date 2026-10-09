    use super::*;
    use crate::stt::{Finality, SpeakerChannel, TranscriptEvent};

    fn event(channel: SpeakerChannel, text: &str, finality: Finality) -> TranscriptEvent {
        TranscriptEvent {
            audio_start_ms: None,
            channel,
            text: text.into(),
            finality,
            speaker_idx: None,
        }
    }

    #[test]
    fn interim_replaces_previous_provisional() {
        let mut a = DraftAssembler::new();
        let first = a
            .push(event(SpeakerChannel::Me, "hel", Finality::Interim))
            .unwrap();
        assert_eq!(first.text, "hel");
        assert!(!first.finality);
        let next = a
            .push(event(SpeakerChannel::Me, "hello", Finality::Interim))
            .unwrap();
        assert_eq!(next.text, "hello");
    }

    #[test]
    fn final_appends_to_committed_and_clears_provisional() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "hel", Finality::Interim));
        let draft = a
            .push(event(SpeakerChannel::Me, "hello", Finality::Final))
            .unwrap();
        assert_eq!(draft.text, "hello");
        assert!(!draft.finality);
        let draft = a
            .push(event(SpeakerChannel::Me, "wor", Finality::Interim))
            .unwrap();
        assert_eq!(draft.text, "hello wor");
    }

    #[test]
    fn whisper_style_final_chunks_accumulate_in_order() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "chunk one", Finality::Final));
        let draft = a
            .push(event(SpeakerChannel::Me, "chunk two", Finality::Final))
            .unwrap();
        assert_eq!(draft.text, "chunk one chunk two");
    }

    #[test]
    fn repeated_identical_words_are_kept() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "yes", Finality::Final));
        let draft = a
            .push(event(SpeakerChannel::Me, "yes", Finality::Final))
            .unwrap();
        assert_eq!(draft.text, "yes yes");
    }

    #[test]
    fn empty_and_whitespace_text_is_ignored() {
        let mut a = DraftAssembler::new();
        assert!(a
            .push(event(SpeakerChannel::Me, "  ", Finality::Interim))
            .is_none());
        assert!(a
            .push(event(SpeakerChannel::Me, "", Finality::Final))
            .is_none());
        assert_eq!(a.finish().text, "");
    }

    #[test]
    fn draft_text_is_trimmed() {
        let mut a = DraftAssembler::new();
        let draft = a
            .push(event(SpeakerChannel::Me, "  hello  ", Finality::Interim))
            .unwrap();
        assert_eq!(draft.text, "hello");
    }

    #[test]
    fn other_channels_do_not_change_the_draft() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "mine", Finality::Final));
        assert!(a
            .push(event(SpeakerChannel::Them, "theirs", Finality::Final))
            .is_none());
        assert!(a
            .push(event(SpeakerChannel::Them, "noise", Finality::Interim))
            .is_none());
        assert_eq!(a.finish().text, "mine");
    }

    #[test]
    fn finish_returns_complete_draft_once_and_resets() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "one", Finality::Final));
        a.push(event(SpeakerChannel::Me, "two", Finality::Interim));
        let draft = a.finish();
        assert_eq!(draft.text, "one two");
        assert!(draft.finality);
        let again = a.finish();
        assert_eq!(again.text, "");
        assert!(again.finality);
        let next = a
            .push(event(SpeakerChannel::Me, "three", Finality::Interim))
            .unwrap();
        assert_eq!(next.text, "three");
    }

    #[test]
    fn draft_serializes_final_field_name() {
        let payload = serde_json::to_value(DictationDraft {
            text: "hi".into(),
            finality: true,
        })
        .unwrap();
        assert_eq!(payload, serde_json::json!({ "text": "hi", "final": true }));
    }

    #[test]
    fn status_serializes_documented_wire_fields() {
        let status = DictationStatus {
            state: "error".into(),
            provider: Some("whisper".into()),
            error: Some(DictationError {
                message: "setup".into(),
                needs_setup: true,
            }),
        };
        let payload = serde_json::to_value(status).unwrap();
        assert_eq!(
            payload,
            serde_json::json!({
                "state": "error",
                "provider": "whisper",
                "error": { "message": "setup", "needs_setup": true },
            })
        );
    }

    #[test]
    fn is_listening_only_while_state_is_listening() {
        for (state, expected) in [("idle", false), ("error", false), ("listening", true)] {
            let status = DictationStatus {
                state: state.into(),
                provider: None,
                error: None,
            };
            assert_eq!(status.is_listening(), expected);
        }
    }

    fn service_root() -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "marvis-dictation-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    type EventLog = Arc<std::sync::Mutex<Vec<DictationEvent>>>;

    fn event_log() -> (EventLog, Arc<dyn Fn(DictationEvent) + Send + Sync>) {
        let events: EventLog = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = events.clone();
        (
            events,
            Arc::new(move |event| captured.lock().unwrap().push(event)),
        )
    }

    /// Mic denial is a setup error reported once — before any audio
    /// hardware or provider is touched.
    #[test]
    fn start_rejects_missing_mic_permission() {
        let root = service_root();
        let keystore = Keystore::at(root.join("keys.json"));
        let config = Config::default();
        let (events, emit) = event_log();
        let service = DictationService::new();

        let result = service.start(&keystore, &config, false, None, emit);

        assert!(result.is_err());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            DictationEvent::Error {
                needs_setup: true,
                ..
            }
        ));
        let status = service.status();
        assert_eq!(status.state, "error");
        assert!(!status.is_listening());
        assert_eq!(status.provider.as_deref(), Some("deepgram"));
        assert_eq!(
            status.error,
            Some(DictationError {
                message: "Microphone permission is required to dictate".into(),
                needs_setup: true,
            })
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The Deepgram key is required only when that provider is configured,
    /// and the failure mirrors Listen's single `needs_setup` event.
    #[test]
    fn start_rejects_missing_deepgram_key() {
        let root = service_root();
        let keystore = Keystore::at(root.join("keys.json"));
        let config = Config::default();
        let (events, emit) = event_log();
        let service = DictationService::new();

        let result = service.start(&keystore, &config, true, None, emit);

        assert!(result.is_err());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            DictationEvent::Error {
                needs_setup: true,
                ..
            }
        ));
        let status = service.status();
        assert_eq!(status.state, "error");
        assert_eq!(
            status.error,
            Some(DictationError {
                message: "Speech-to-text provider is not configured".into(),
                needs_setup: true,
            })
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Revalidation walks `start()`'s gate order: a denied mic still
    /// reports the mic error, then the next blocker (here the missing
    /// Deepgram key), and only a fully fixed setup clears to `idle`.
    #[test]
    fn revalidate_setup_walks_mic_then_provider_and_clears_when_fixed() {
        let root = service_root();
        let mut keystore = Keystore::at(root.join("keys.json"));
        let config = Config::default();
        let (_events, emit) = event_log();
        let service = DictationService::new();
        let sherpa_root = root.join("sherpa");

        assert!(service
            .start(&keystore, &config, false, None, emit)
            .is_err());
        assert_eq!(service.status().state, "error");

        // Mic still denied → same error, nothing to report.
        assert!(service
            .revalidate_setup(&keystore, &config, None, &sherpa_root, false)
            .is_none());

        // Mic granted → the next gate (missing key) becomes the reason.
        let next = service
            .revalidate_setup(&keystore, &config, None, &sherpa_root, true)
            .expect("a different broken reason must yield a new status");
        assert_eq!(next.state, "error");
        assert_eq!(
            next.error,
            Some(DictationError {
                message: "Speech-to-text provider is not configured".into(),
                needs_setup: true,
            })
        );

        // Key stored → the durable error clears to idle.
        keystore.set_key("deepgram", "dg-key").unwrap();
        let next = service
            .revalidate_setup(&keystore, &config, None, &sherpa_root, true)
            .expect("resolved error must yield a new status");
        assert_eq!(next.state, "idle");
        assert_eq!(next.error, None);
        assert_eq!(service.status().state, "idle");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Revalidation only touches setup failures — a runtime
    /// (`needs_setup: false`) error stays put.
    #[test]
    fn revalidate_setup_ignores_runtime_errors() {
        let root = service_root();
        let keystore = Keystore::at(root.join("keys.json"));
        let config = Config::default();
        let service = DictationService::new();
        *service.state.lock() = DictationStatus {
            state: "error".into(),
            provider: Some("deepgram".into()),
            error: Some(DictationError {
                message: "provider went away".into(),
                needs_setup: false,
            }),
        };
        assert!(service
            .revalidate_setup(&keystore, &config, None, &root.join("sherpa"), true)
            .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    /// A provider name outside the catalog must fail before the microphone
    /// is opened — the provider is constructed first on purpose.
    #[test]
    fn start_rejects_unknown_stt_provider_before_mic() {
        let root = service_root();
        let keystore = Keystore::at(root.join("keys.json"));
        let mut config = Config::default();
        config.models.stt_provider = "not-a-provider".into();
        let (events, emit) = event_log();
        let service = DictationService::new();

        let result = service.start(&keystore, &config, true, None, emit);

        assert!(result.is_err());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            DictationEvent::Error {
                message,
                needs_setup,
            } => {
                assert!(needs_setup);
                assert_eq!(message, "unsupported STT provider: not-a-provider");
            }
            other => panic!("expected error event, got {other:?}"),
        }
        assert_eq!(service.status().state, "error");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Stop is safe on an idle service and returns an empty final draft.
    #[test]
    fn stop_is_idempotent_and_returns_empty_draft_when_idle() {
        let service = DictationService::new();
        for _ in 0..2 {
            let draft = service.stop();
            assert_eq!(draft.text, "");
            assert!(draft.finality);
            assert_eq!(service.status().state, "idle");
        }
    }

    /// Each `push` that yields a snapshot is forwarded as one draft event.
    #[test]
    fn transcript_callback_emits_only_draft_changing_events() {
        let assembler = Arc::new(Mutex::new(DraftAssembler::new()));
        let (events, emit) = event_log();
        let callback = transcript_callback(&assembler, &emit);

        callback(event(SpeakerChannel::Me, "hel", Finality::Interim));
        callback(event(SpeakerChannel::Them, "theirs", Finality::Final));
        callback(event(SpeakerChannel::Me, "  ", Finality::Interim));
        callback(event(SpeakerChannel::Me, "hello", Finality::Final));

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        match &events[0] {
            DictationEvent::Draft(draft) => {
                assert_eq!(draft.text, "hel");
                assert!(!draft.finality);
            }
            other => panic!("expected draft event, got {other:?}"),
        }
        match &events[1] {
            DictationEvent::Draft(draft) => assert_eq!(draft.text, "hello"),
            other => panic!("expected draft event, got {other:?}"),
        }
    }

    /// Provider failures cancel the pump, mark the status `error`, and the
    /// emitted message is flattened/capped — newlines can never reach the
    /// webview payload.
    #[test]
    fn provider_error_callback_cancels_and_sanitizes() {
        let state = Arc::new(Mutex::new(DictationStatus {
            state: "listening".into(),
            provider: Some("deepgram".into()),
            error: None,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(AtomicU64::new(7));
        let (events, emit) = event_log();
        let callback = provider_error_callback(&state, &cancel, &epoch, 7, &emit);

        callback("first line\nsecond line with detail".to_string());

        assert!(cancel.load(Ordering::Acquire));
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            DictationEvent::Error {
                message,
                needs_setup,
            } => {
                assert_eq!(message, "first line second line with detail");
                assert!(!needs_setup);
            }
            other => panic!("expected error event, got {other:?}"),
        }
        let status = state.lock().clone();
        assert_eq!(status.state, "error");
        assert_eq!(status.provider.as_deref(), Some("deepgram"));
        assert_eq!(
            status.error,
            Some(DictationError {
                message: "first line second line with detail".into(),
                needs_setup: false,
            })
        );
    }

    /// Once the epoch moved on — a `stop()` or a newer start won the
    /// race — a late provider error still cancels the pump but must not
    /// write status or emit `dictation:error` for a dead session.
    #[test]
    fn provider_error_callback_is_suppressed_after_epoch_bump() {
        let state = Arc::new(Mutex::new(DictationStatus {
            state: "idle".into(),
            provider: None,
            error: None,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(AtomicU64::new(3));
        let (events, emit) = event_log();
        let callback = provider_error_callback(&state, &cancel, &epoch, 3, &emit);

        epoch.fetch_add(1, Ordering::AcqRel);
        callback("late failure".to_string());

        assert!(cancel.load(Ordering::Acquire));
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(state.lock().state, "idle");
        assert_eq!(state.lock().error, None);
    }

    /// A `Running` whose pump is just a cancellable sleep loop — enough
    /// to exercise commit/stop teardown without audio hardware or a
    /// real STT process.
    fn dummy_running() -> (Running, Arc<AtomicBool>, Arc<Mutex<DraftAssembler>>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let assembler = Arc::new(Mutex::new(DraftAssembler::new()));
        let worker = {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                while !cancel.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        };
        (
            Running {
                cancel: cancel.clone(),
                worker,
                assembler: assembler.clone(),
            },
            cancel,
            assembler,
        )
    }

    /// An uninterrupted commit stores the session and reports
    /// `listening`; the stop that follows still recovers the draft and
    /// leaves `idle`.
    #[test]
    fn commit_marks_listening_and_stop_recovers_the_draft() {
        let service = DictationService::new();
        let epoch = service.epoch.load(Ordering::Acquire);
        let (running, _cancel, assembler) = dummy_running();
        assembler
            .lock()
            .push(event(SpeakerChannel::Me, "hello", Finality::Final));

        service
            .commit(epoch, "deepgram".into(), running)
            .expect("uncontested commit");

        assert!(service.status().is_listening());
        assert!(service.running.lock().is_some());
        let draft = service.stop();
        assert_eq!(draft.text, "hello");
        assert!(draft.finality);
        assert_eq!(service.status().state, "idle");
        assert!(service.running.lock().is_none());
    }

    /// A `stop()` landing between a start's reset and its commit wins:
    /// the losing start cancels and joins the pump it just spawned
    /// instead of storing an untracked worker or overwriting `idle`.
    #[test]
    fn stop_during_start_commit_wins_and_leaves_no_worker() {
        let service = DictationService::new();
        // The snapshot a real `start()` takes after its internal reset.
        let epoch = service.epoch.load(Ordering::Acquire);
        // The racing stop lands while the start is still building.
        let _ = service.stop();

        let (running, cancel, _assembler) = dummy_running();
        let result = service.commit(epoch, "deepgram".into(), running);

        assert!(result.is_err());
        // The aborted start's pump was cancelled, not orphaned.
        assert!(cancel.load(Ordering::Acquire));
        assert_eq!(service.status().state, "idle");
        assert!(service.running.lock().is_none());
    }

    /// A provider error that already fired mid-build (cancel set, same
    /// epoch) also aborts the commit — no `listening` overwrite on a
    /// dead session.
    #[test]
    fn commit_aborts_when_provider_already_failed() {
        let service = DictationService::new();
        let epoch = service.epoch.load(Ordering::Acquire);
        let (running, cancel, _assembler) = dummy_running();
        cancel.store(true, Ordering::Release);

        let result = service.commit(epoch, "deepgram".into(), running);

        assert!(result.is_err());
        assert!(service.running.lock().is_none());
        assert!(!service.status().is_listening());
    }

    /// Hammering stop from several threads while a commit races in ends
    /// one way: `idle`, nothing running, every stop returning a final
    /// draft — no stale `listening`, no untracked worker.
    #[test]
    fn concurrent_stops_and_a_racing_commit_never_leave_stale_status() {
        let service = Arc::new(DictationService::new());
        let epoch = service.epoch.load(Ordering::Acquire);
        let (running, _cancel, _assembler) = dummy_running();

        let commit = {
            let service = service.clone();
            std::thread::spawn(move || service.commit(epoch, "deepgram".into(), running))
        };
        let stops: Vec<_> = (0..4)
            .map(|_| {
                let service = service.clone();
                std::thread::spawn(move || service.stop())
            })
            .collect();

        let _ = commit.join().expect("commit thread");
        for stop in stops {
            let draft = stop.join().expect("stop thread");
            assert!(draft.finality);
        }
        assert_eq!(service.status().state, "idle");
        assert!(service.running.lock().is_none());
    }

    /// Repeated stops are idempotent even when interleaved with an
    /// aborted start — the second and later stops return empty final
    /// drafts and the status stays `idle`.
    #[test]
    fn repeated_stops_after_aborted_start_stay_idle() {
        let service = DictationService::new();
        let epoch = service.epoch.load(Ordering::Acquire);
        let _ = service.stop();
        let (running, _cancel, _assembler) = dummy_running();
        assert!(service.commit(epoch, "deepgram".into(), running).is_err());
        for _ in 0..3 {
            let draft = service.stop();
            assert_eq!(draft.text, "");
            assert!(draft.finality);
            assert_eq!(service.status().state, "idle");
        }
    }
