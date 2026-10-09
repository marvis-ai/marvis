    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn catalog_is_the_approved_file_set() {
        assert_eq!(catalog().len(), 4);
        let entry = &catalog()[0];
        assert_eq!(entry.id.as_str(), "sense-voice");
        assert_eq!(entry.dirname, "sense-voice");
        assert_eq!(entry.kind, SherpaModelKind::Stt);
        assert_eq!(
            entry.files.iter().map(|f| f.filename).collect::<Vec<_>>(),
            ["model.int8.onnx", "tokens.txt", "silero_vad.onnx"]
        );
        assert!(entry.files.iter().all(|f| f.url.starts_with("https://")
            && f.sha256.len() == 64
            && f.sha256.chars().all(|c| c.is_ascii_hexdigit())));
        let speaker = &catalog()[1];
        assert_eq!(speaker.id.as_str(), "speaker-id");
        assert_eq!(speaker.dirname, "speaker-id");
        assert_eq!(speaker.kind, SherpaModelKind::SpeakerEmbedding);
        assert_eq!(speaker.files.len(), 1);
        assert!(speaker.files.iter().all(|f| f.url.starts_with("https://")
            && f.sha256.len() == 64
            && f.sha256.chars().all(|c| c.is_ascii_hexdigit())));
        let punct = &catalog()[2];
        assert_eq!(punct.id.as_str(), "punct-en");
        assert_eq!(punct.dirname, "punct-en");
        assert_eq!(punct.kind, SherpaModelKind::Punctuation);
        assert_eq!(
            punct.files.iter().map(|f| f.filename).collect::<Vec<_>>(),
            ["model.int8.onnx", "bpe.vocab"]
        );
        assert!(punct.files.iter().all(|f| f.url.starts_with("https://")
            && f.sha256.len() == 64
            && f.sha256.chars().all(|c| c.is_ascii_hexdigit())));
        let punct_zh = &catalog()[3];
        assert_eq!(punct_zh.id.as_str(), "punct-zh");
        assert_eq!(punct_zh.dirname, "punct-zh");
        assert_eq!(punct_zh.kind, SherpaModelKind::Punctuation);
        assert_eq!(
            punct_zh.files.iter().map(|f| f.filename).collect::<Vec<_>>(),
            ["model.int8.onnx"]
        );
        assert!(punct_zh.files.iter().all(|f| f.url.starts_with("https://")
            && f.sha256.len() == 64
            && f.sha256.chars().all(|c| c.is_ascii_hexdigit())));
    }

    /// Non-STT entries are downloadable but must never be selectable as the
    /// transcription model — `stt_entry_for_value` filters them while the
    /// general lookup still resolves them for download/remove.
    #[test]
    fn aux_models_are_downloadable_but_never_an_stt_model() {
        for id in ["speaker-id", "punct-en", "punct-zh"] {
            assert!(entry_for_value(id).is_some());
            assert!(stt_entry_for_value(id).is_none());
            assert!(crate::config::validate_sherpa_model(id).is_err());
        }
        assert!(stt_entry_for_value("sense-voice").is_some());
        assert_eq!(
            crate::config::validate_sherpa_model("sense-voice").unwrap(),
            "sense-voice"
        );
    }

    #[test]
    fn lookup_rejects_arbitrary_ids_urls_and_paths() {
        assert_eq!(
            entry_for_id(" SenseVoice ").unwrap().id,
            SherpaModelId::SenseVoice
        );
        for value in [
            "https://example.com/model.onnx",
            "../sense-voice",
            "nested/model.int8.onnx",
            "tiny",
        ] {
            assert!(entry_for_value(value).is_none());
        }
    }

    #[test]
    fn installed_requires_every_file() {
        let root = temp_root();
        let entry = &catalog()[0];
        assert!(!entry_installed_at(&root, entry));
        let dir = entry_dir(&root, entry);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("model.int8.onnx"), b"x").unwrap();
        fs::write(dir.join("tokens.txt"), b"x").unwrap();
        assert!(!entry_installed_at(&root, entry)); // silero still missing
        fs::write(dir.join("silero_vad.onnx"), b"x").unwrap();
        assert!(entry_installed_at(&root, entry));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn status_dto_has_only_safe_public_fields() {
        let status = serde_json::to_value(SherpaStatus {
            models: vec![SherpaInstalledModel {
                id: "sense-voice",
                label: "SenseVoice",
                description: "d",
                bytes: 1,
                kind: "stt",
                installed: true,
            }],
            download: None,
        })
        .unwrap();
        assert!(status.get("binary").is_none());
        assert_eq!(status["models"][0]["id"], "sense-voice");
        assert!(status["models"][0].get("url").is_none());
        assert!(status["models"][0].get("sha256").is_none());
    }

    fn fixture(body: Vec<u8>, delay: Duration) -> TestSource {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let expected = body.clone();
        std::thread::spawn(move || {
            // Loop-accept: a source may be re-fetched — the punctuation
            // chain downloads more than one catalog entry per manager.
            while let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0; 1024];
                let _ = stream.read(&mut request);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    expected.len()
                );
                let _ = stream.write_all(header.as_bytes());
                for chunk in expected.chunks(2) {
                    let _ = stream.write_all(chunk);
                    let _ = stream.flush();
                    std::thread::sleep(delay);
                }
            }
        });
        let mut sha = Sha256::new();
        sha.update(&body);
        let digest = sha
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        TestSource {
            url: format!("http://{address}/model"),
            bytes: body.len() as u64,
            sha256: digest,
        }
    }

    /// One one-shot server per file — files download sequentially, so each
    /// entry file needs its own listener. The returned vec overrides the
    /// catalog specs by index via `with_test_files`, which requires one
    /// source per entry file.
    fn fixture_set(bodies: &[Vec<u8>], delay: Duration) -> Vec<TestSource> {
        bodies
            .iter()
            .map(|body| fixture(body.clone(), delay))
            .collect()
    }

    fn temp_root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir();
        loop {
            let root = base.join(format!(
                "marvis-sherpa-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
            ));
            if fs::create_dir(&root).is_ok() {
                return root;
            }
        }
    }

    #[test]
    fn sync_start_download_uses_the_tauri_runtime() {
        let root = temp_root();
        // File 1 lands quickly; files 2/3 stream slowly so the download is
        // still in flight when the synchronous cancel below arrives.
        let sources = vec![
            fixture(b"sync start fixture".to_vec(), Duration::ZERO),
            fixture(b"sync start tokens".to_vec(), Duration::from_millis(100)),
            fixture(b"sync start vad".to_vec(), Duration::from_millis(100)),
        ];
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, sources);

        manager.start_download(SherpaModelId::SenseVoice).unwrap();
        tauri::async_runtime::block_on(manager.cancel_download()).unwrap();

        let dir = entry_dir(&root, &catalog()[0]);
        for file in catalog()[0].files {
            assert!(!dir.join(format!("{}.tmp", file.filename)).exists());
        }
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn lifecycle_verifies_cleans_preserves_and_serializes_downloads() {
        let bodies = [
            b"sense voice model fixture".to_vec(),
            b"tokens fixture".to_vec(),
            b"vad fixture".to_vec(),
        ];
        let sources = fixture_set(&bodies, Duration::from_millis(10));
        assert!(sources[0].url.starts_with("http://127.0.0.1:"));
        let root = temp_root();
        let dir = entry_dir(&root, &catalog()[0]);
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, sources);
        manager.start_download(SherpaModelId::SenseVoice).unwrap();
        assert!(matches!(
            manager.start_download(SherpaModelId::SenseVoice),
            Err(VoiceDownloadError::Busy)
        ));
        manager.cancel_download().await.unwrap();
        for file in catalog()[0].files {
            assert!(!dir.join(format!("{}.tmp", file.filename)).exists());
        }

        // A failed sha256 must not clobber an already-installed file.
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("model.int8.onnx"), b"keep me").unwrap();
        let mut bad_sources = fixture_set(&bodies, Duration::ZERO);
        bad_sources[0].sha256 = "00".repeat(32);
        let bad = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, bad_sources);
        bad.start_download(SherpaModelId::SenseVoice).unwrap();
        while bad.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert_eq!(fs::read(dir.join("model.int8.onnx")).unwrap(), b"keep me");
        assert!(!dir.join("model.int8.onnx.tmp").exists());

        // A size far outside the ±10% band aborts the install as well.
        let mut bad_size_sources = fixture_set(&bodies, Duration::ZERO);
        bad_size_sources[0].bytes = u64::MAX / 2;
        let bad_size = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, bad_size_sources);
        bad_size.start_download(SherpaModelId::SenseVoice).unwrap();
        while bad_size.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(!dir.join("model.int8.onnx.tmp").exists());
        assert_eq!(fs::read(dir.join("model.int8.onnx")).unwrap(), b"keep me");

        // Success requires every file in the set to land, content intact.
        let success_sources = fixture_set(&bodies, Duration::ZERO);
        let success = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, success_sources);
        success.start_download(SherpaModelId::SenseVoice).unwrap();
        while success.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(entry_installed_at(&root, &catalog()[0]));
        for (i, file) in catalog()[0].files.iter().enumerate() {
            assert_eq!(fs::read(dir.join(file.filename)).unwrap(), bodies[i]);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("model.int8.onnx"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_reclaims_only_catalog_temps() {
        let root = temp_root();
        let dir = entry_dir(&root, &catalog()[0]);
        fs::create_dir_all(&dir).unwrap();
        for file in catalog()[0].files {
            fs::write(dir.join(format!("{}.tmp", file.filename)), b"stale").unwrap();
        }
        fs::write(dir.join("foreign.tmp"), b"keep").unwrap();
        fs::write(root.join("foreign.tmp"), b"keep root").unwrap();
        let _manager = SherpaModelManager::at(root.clone());
        for file in catalog()[0].files {
            assert!(!dir.join(format!("{}.tmp", file.filename)).exists());
        }
        assert!(dir.join("foreign.tmp").exists());
        assert!(root.join("foreign.tmp").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn active_model_removal_is_rejected() {
        let manager = SherpaModelManager::at(temp_root());
        manager.set_selected_model(Some(SherpaModelId::SenseVoice));
        assert!(matches!(
            manager.remove_model(SherpaModelId::SenseVoice),
            Err(VoiceDownloadError::ActiveModel)
        ));
        manager.set_selected_model(None);
        let _ = fs::remove_dir_all(manager.root);
    }

    #[tokio::test]
    async fn removal_is_rejected_while_download_is_active() {
        let root = temp_root();
        let bodies = [
            b"fixture a".to_vec(),
            b"fixture b".to_vec(),
            b"fixture c".to_vec(),
        ];
        let sources = fixture_set(&bodies, Duration::from_millis(100));
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, sources);
        manager.start_download(SherpaModelId::SenseVoice).unwrap();
        assert!(matches!(
            manager.remove_model(SherpaModelId::SenseVoice),
            Err(VoiceDownloadError::Busy)
        ));
        manager.cancel_download().await.unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancellation_racing_final_install_has_one_authoritative_outcome() {
        let root = temp_root();
        let bodies = [b"race a".to_vec(), b"race b".to_vec(), b"race c".to_vec()];
        let sources = fixture_set(&bodies, Duration::ZERO);
        let manager = Arc::new(SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, sources));
        manager.start_download(SherpaModelId::SenseVoice).unwrap();
        let canceller = Arc::clone(&manager);
        canceller.cancel_download().await.unwrap();
        let dir = entry_dir(&root, &catalog()[0]);
        for file in catalog()[0].files {
            assert!(!dir.join(format!("{}.tmp", file.filename)).exists());
        }
        if entry_installed_at(&root, &catalog()[0]) {
            for (i, file) in catalog()[0].files.iter().enumerate() {
                assert_eq!(fs::read(dir.join(file.filename)).unwrap(), bodies[i]);
            }
        }
        assert!(manager.status().download.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancellation_does_not_remove_preexisting_temp_file() {
        let root = temp_root();
        let dir = entry_dir(&root, &catalog()[0]);
        fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("foreign.tmp");
        fs::write(&tmp, b"owned by someone else").unwrap();
        // Slow fixtures keep the download in flight so the cancel below
        // deterministically lands mid-stream.
        let bodies = [
            b"fixture a".to_vec(),
            b"fixture b".to_vec(),
            b"fixture c".to_vec(),
        ];
        let sources = fixture_set(&bodies, Duration::from_millis(100));
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, sources);
        manager.start_download(SherpaModelId::SenseVoice).unwrap();
        manager.cancel_download().await.unwrap();
        assert_eq!(fs::read(tmp).unwrap(), b"owned by someone else");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn ensure_punct_downloads_once_when_stt_installed() {
        let root = temp_root();
        // "Installed" stt entry: all its catalog files present.
        let stt_dir = entry_dir(&root, &catalog()[0]);
        fs::create_dir_all(&stt_dir).unwrap();
        for f in catalog()[0].files {
            fs::write(stt_dir.join(f.filename), b"x").unwrap();
        }
        let sources = fixture_set(&[b"punct model".to_vec(), b"bpe vocab".to_vec()], Duration::ZERO);
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::PunctEn, sources);
        manager.ensure_punct("sherpa", "sense-voice");
        while manager.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(entry_installed_at(&root, entry_for_id("punct-en").unwrap()));
        // Each further call queues the next missing add-on — the zh
        // punctuator here (it consumes the same positional fixtures).
        manager.ensure_punct("sherpa", "sense-voice");
        while manager.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(entry_installed_at(&root, entry_for_id("punct-zh").unwrap()));
        // Once everything is installed the call is a no-op — no Busy
        // error, nothing re-queued.
        manager.ensure_punct("sherpa", "sense-voice");
        assert!(manager.status().download.is_none());
        let _ = fs::remove_dir_all(root);
    }

    /// A non-sherpa provider, an stt value outside the sherpa STT catalog,
    /// and a not-yet-installed STT model all skip the fetch — and none of
    /// those skips burns the once-per-run flag, so the first refresh after
    /// SenseVoice lands still heals.
    #[tokio::test]
    async fn ensure_punct_only_fires_for_sherpa_with_stt_installed() {
        let root = temp_root();
        let stt_dir = entry_dir(&root, &catalog()[0]);
        fs::create_dir_all(&stt_dir).unwrap();
        for f in catalog()[0].files {
            fs::write(stt_dir.join(f.filename), b"x").unwrap();
        }
        let sources = fixture_set(&[b"punct model".to_vec(), b"bpe vocab".to_vec()], Duration::ZERO);
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::PunctEn, sources);
        manager.ensure_punct("whisper", "sense-voice");
        manager.ensure_punct("sherpa", "tiny");
        assert!(manager.status().download.is_none());
        manager.ensure_punct("sherpa", "sense-voice");
        while manager.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(entry_installed_at(&root, entry_for_id("punct-en").unwrap()));
        let _ = fs::remove_dir_all(root);

        let root = temp_root();
        let sources = fixture_set(&[b"punct model".to_vec(), b"bpe vocab".to_vec()], Duration::ZERO);
        let missing = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::PunctEn, sources);
        missing.ensure_punct("sherpa", "sense-voice");
        assert!(missing.status().download.is_none());
        let stt_dir = entry_dir(&root, &catalog()[0]);
        fs::create_dir_all(&stt_dir).unwrap();
        for f in catalog()[0].files {
            fs::write(stt_dir.join(f.filename), b"x").unwrap();
        }
        missing.ensure_punct("sherpa", "sense-voice");
        while missing.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(entry_installed_at(&root, entry_for_id("punct-en").unwrap()));
        let _ = fs::remove_dir_all(root);
    }

    /// A task that died before reaching its finishing arm leaves a
    /// completed handle registered as `active`. `ensure_punct` must reap
    /// it like `status`/`start_download` do — otherwise the chain stalls
    /// until something else happens to reap.
    #[tokio::test]
    async fn ensure_punct_reaps_a_finished_active_before_deciding() {
        let root = temp_root();
        let stt_dir = entry_dir(&root, &catalog()[0]);
        fs::create_dir_all(&stt_dir).unwrap();
        for f in catalog()[0].files {
            fs::write(stt_dir.join(f.filename), b"x").unwrap();
        }
        let sources = fixture_set(&[b"punct model".to_vec(), b"bpe vocab".to_vec()], Duration::ZERO);
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::PunctEn, sources);
        // The state a dead task leaves behind: a finished handle still
        // named in `active`.
        let task = tauri::async_runtime::spawn(async {});
        manager.state.lock().active = Some(ActiveDownload {
            cancel: CancellationToken::new(),
            task: Some(task),
            progress: Arc::new(Mutex::new(SherpaDownloadProgress {
                model: "punct-en",
                received: 0,
                total: 1,
            })),
        });
        // Let the spawned task finish — `status()` would reap, so wait on
        // the raw handle instead.
        loop {
            let done = manager
                .state
                .lock()
                .active
                .as_ref()
                .and_then(|a| a.task.as_ref())
                .is_some_and(|t| t.inner().is_finished());
            if done {
                break;
            }
            tokio::task::yield_now().await;
        }
        manager.ensure_punct("sherpa", "sense-voice");
        while manager.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(entry_installed_at(&root, entry_for_id("punct-en").unwrap()));
        let _ = fs::remove_dir_all(root);
    }

    /// The cancelled task's finishing arm can free `active` and a new
    /// download register before the stale cancel resumes — its final
    /// clear must be identity-checked or it orphans the newer download.
    #[tokio::test]
    async fn cancel_does_not_orphan_a_download_registered_after_the_old_one() {
        let root = temp_root();
        let manager = Arc::new(SherpaModelManager::at(root.clone()));
        // A registered download whose completion the test controls — the
        // shape `active` takes while a download is in flight.
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel::<()>();
        let a_progress = Arc::new(Mutex::new(SherpaDownloadProgress {
            model: "sense-voice",
            received: 0,
            total: 1,
        }));
        manager.state.lock().active = Some(ActiveDownload {
            cancel: CancellationToken::new(),
            task: Some(tauri::async_runtime::spawn(async move {
                let _ = finish_rx.await;
            })),
            progress: a_progress,
        });
        let canceller = Arc::clone(&manager);
        let cancel = tokio::spawn(async move { canceller.cancel_download().await });
        // Wait until the cancel has lifted the task handle and parked on
        // its completion.
        loop {
            let parked = manager
                .state
                .lock()
                .active
                .as_ref()
                .is_some_and(|a| a.task.is_none());
            if parked {
                break;
            }
            tokio::task::yield_now().await;
        }
        // A "finished on its own" mid-cancel: the slot freed and a new
        // download registered before the stale cancel resumed.
        let b_progress = Arc::new(Mutex::new(SherpaDownloadProgress {
            model: "punct-en",
            received: 0,
            total: 1,
        }));
        manager.state.lock().active = Some(ActiveDownload {
            cancel: CancellationToken::new(),
            task: None,
            progress: Arc::clone(&b_progress),
        });
        let _ = finish_tx.send(());
        cancel.await.unwrap().unwrap();
        // B's registration survives — the stale cancel only cleared its own.
        assert!(manager
            .state
            .lock()
            .active
            .as_ref()
            .is_some_and(|a| Arc::ptr_eq(&a.progress, &b_progress)));
        let _ = fs::remove_dir_all(root);
    }
