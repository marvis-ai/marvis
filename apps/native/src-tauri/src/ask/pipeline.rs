use super::*;
use super::attachments::*;
use super::history::*;
use super::screen::*;
use super::stream::*;
use super::title::*;

/// The per-run fields `send_chain` consumes — resolved by `kick`
/// (bundled the same way `ScreenInput` bundles the screen side):
/// `fresh_session` (the send found the card closed) ends the open ask
/// session so this run mints a fresh row; `regenerate` re-asks the
/// session's last user row; `listen_id` binds the run to a listen
/// doc's own ask session — one chat per doc; `language` is the
/// `config.app.main_language` snapshot — the reply language;
/// `instruction`/`preset_id` are the armed `instruct` preset's text
/// and id.
#[derive(Default)]
pub(crate) struct ChainOpts<'a> {
    pub text: &'a str,
    pub fresh_session: bool,
    pub regenerate: bool,
    pub listen_id: Option<i64>,
    pub language: &'a str,
    /// Resolved `instruct` preset text — appended to the system prompt.
    pub instruction: Option<&'a str>,
    /// The armed preset id — `None` when it didn't resolve; persisted
    /// on the user row so `retry` re-resolves it.
    pub preset_id: Option<&'a str>,
    /// The composer's normalized images for this send — decoded,
    /// written, and row-linked before the first provider call; empty
    /// on regenerates (their attachments re-load from disk).
    pub attachments: Vec<AskAttachmentInput>,
    /// Managed-file root — `None` resolves to
    /// `paths::attachments_dir()`; tests pass a tmp dir so runs never
    /// touch the real `~/.marvis`.
    pub attachments_root: Option<&'a Path>,
}

/// The testable core: persist → walk the failover chain → persist,
/// emitting the `ask:*` protocol through `emit`. No `AppHandle`/keystore/
/// pool inside — [`AskService::send`] gathers those deps and delegates.
///
/// Order of operations:
/// 0. `fresh_session` (the send found the card closed): end the open
///    ask session so this run starts a new conversation. `listen_id`
///    (a send from the listen doc) instead binds the run to that doc's
///    own ask session — one chat per doc.
/// 1. Persist the user message FIRST — it was already sent, so it must
///    be recorded even when every candidate later errors or is cancelled.
/// 2. `ask:state{loading}` once (the chain is one run).
/// 3. Each [`ProviderCandidate`] streams via [`stream_candidate`]:
///    `Done` → persist assistant + `ask:done{full, provider, model,
///    usage}` +
///    `ask:state{idle}`, then the title sidecar ([`maybe_title_session`])
///    on a still-untitled session; `Failed` → warn-log, re-emit
///    `loading` (the card resets its buffer — a dead provider's partial
///    chunks must not bleed into the next attempt), and try the NEXT
///    candidate; `Cancelled` → `ask:state{idle}` and stop immediately —
///    the user asked to stop, so no failover may start a new request.
/// 4. Every candidate failed → `ask:error{message}` (the LAST failure's
///    message — the most actionable one) + `ask:state{idle}`.
///
/// Db failures are `log::warn`ed and ignored — a storage hiccup must
/// never block the stream.
///
/// Resolves to the full assistant text. Cancellation before an answer returns a
/// `status:0`/`"cancelled"` [`LlmError::Http`] sentinel — the events, not
/// the return value, drive the UI.
/// Exhausting the chain returns the last provider error, or
/// [`LlmError::NoModel`] for an empty chain. Success waits for the title
/// attempt after emitting `idle`; title failures do not change the answer,
/// and title usage is excluded from the reported turn usage.
///
/// `regenerate` (ask_retry): the run re-asks the session's last user
/// row instead of persisting a new one, and the rejected reply's row is
/// deleted — the stream's spend (vision read included) lands on the
/// replacement row, which `ask:done` also reports.
///
/// `screen_input` feeds [`resolve_screen`], which runs BEFORE the
/// `loading` emit so an explicit ask's captured frame can persist as a
/// user attachment and ride the payload's `attachments` list. Its
/// `Err` ends the run as `loading` → `ask:error` → `idle` (the row is
/// already painted so the question survives the toast).
pub(crate) async fn send_chain(
    candidates: Vec<ProviderCandidate>,
    vision: Option<ProviderCandidate>,
    db: &Db,
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
    screen_input: &ScreenInput<'_>,
    cancel: &CancellationToken,
    opts: ChainOpts<'_>,
) -> Result<String, LlmError> {
    let ChainOpts {
        text,
        fresh_session,
        regenerate,
        listen_id,
        language,
        instruction,
        preset_id,
        attachments,
        attachments_root,
    } = opts;
    // Contract guard: a crafted invoke past the composer cap is an
    // attachment error — no row persists, no provider is called.
    if attachments.len() > MAX_ATTACHMENTS {
        let message = format!("At most {MAX_ATTACHMENTS} images can be attached per send");
        return Err(attachment_error(emit, message));
    }
    // Decode and sniff before ANY persistence — a malformed payload
    // must not leave a user row that claims images it doesn't have.
    let mut pending = Vec::with_capacity(attachments.len());
    for input in &attachments {
        match decode_attachment(input) {
            Ok(p) => pending.push(p),
            Err(message) => return Err(attachment_error(emit, message)),
        }
    }
    let attachments_root: PathBuf = attachments_root
        .map(Path::to_path_buf)
        .unwrap_or_else(crate::paths::attachments_dir);
    // A send that arrived with the card closed is a new conversation:
    // end the still-open ask session so get_or_create mints a fresh row.
    // A linked send resolves its own session below instead.
    if fresh_session && listen_id.is_none() {
        if let Ok(Some(id)) = db.session_active_id("ask") {
            if let Err(error) = db.session_end(id) {
                log::warn!("ask: session_end before fresh send failed: {error}");
            }
        }
    }
    // Session resolution: a linked send lands in the listen doc's own
    // ask session (`ask_session_for_listen` reopens it or mints one);
    // a regenerate keeps the active session — its question IS that
    // session's last user row; anything else is the open ask session.
    let session_id = match (regenerate, listen_id) {
        (false, Some(lid)) => match db.ask_session_for_listen(lid) {
            Ok(id) => Some(id),
            Err(error) => {
                log::warn!("ask: linked session resolve failed: {error}");
                None
            }
        },
        _ => open_ask_session(db),
    };
    // Order matters: history is read BEFORE the new user row persists —
    // the new turn is appended separately so it can carry the frame. A
    // regenerate skips the write: its question is the session's last
    // user row, and history ends BEFORE it (a missing tail — shouldn't
    // happen, `retry` resolved the question from that row — degrades
    // to a normal send).
    let (history_rows, re_asked, regen_row_id, regen_atts) = if regenerate {
        match regenerate_tail_rows(db, session_id) {
            Some((rows, mid, atts)) => (rows, true, Some(mid), atts),
            None => (message_rows(db, session_id), false, None, Vec::new()),
        }
    } else {
        (message_rows(db, session_id), false, None, Vec::new())
    };
    // History turns replay their persisted attachments as image parts;
    // a regenerate's turn images come from the re-asked row's managed
    // files. Blocking reads run off the async executor; a missing or
    // corrupt file is a user-facing attachment error — the prompt must
    // never silently lose images.
    let loaded = {
        let root = attachments_root.clone();
        let regen_atts = regen_atts.clone();
        tokio::task::spawn_blocking(
            move || -> Result<(Vec<ChatMessage>, Vec<Vec<u8>>), String> {
                let history = rows_to_history_at(&history_rows, &root)?;
                let images = regen_atts
                    .iter()
                    .map(|a| read_attachment(&root, a))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((history, images))
            },
        )
        .await
    };
    let (history, regen_images) = match loaded {
        Ok(Ok(pair)) => pair,
        Ok(Err(message)) => return Err(attachment_error(emit, message)),
        Err(e) => return Err(attachment_error(emit, format!("Couldn't load attachments: {e}"))),
    };
    // The effective doc link: the send's explicit one, else the resolved
    // session's stored link — a continued doc chat (or its retry) keeps
    // the doc's context even though the caller passed none.
    let listen_id =
        listen_id.or_else(|| session_id.and_then(|sid| db.session_listen_id(sid).ok().flatten()));
    let listen_history = load_listen_context(db, listen_id);
    let mut user_images: Vec<Vec<u8>> = regen_images;
    // The run's user-turn attachments — the `loading` payload advertises
    // them so the card/resync renders the persisted message correctly.
    let mut run_attachments: Vec<MessageAttachment> = regen_atts;
    // The user row's id — persisted fresh for sends, resolved for
    // regens (the re-asked row's). `None` when storage hiccuped.
    let message_id = if re_asked {
        regen_row_id
    } else {
        persist_user_message(db, session_id, text, preset_id)
    };
    if !re_asked && !pending.is_empty() {
        // Attachments need the user row to link to — a storage
        // hiccup that ate the row is a hard failure for an
        // attachment-bearing send, not a degrade to text.
        let Some(mid) = message_id else {
            return Err(attachment_error(
                emit,
                "Attachments couldn't be saved — the message wasn't recorded".into(),
            ));
        };
        match persist_attachments(db, &attachments_root, mid, pending).await {
            Ok((images, meta)) => {
                user_images = images;
                run_attachments = meta;
            }
            Err(message) => {
                // The row claims images it never got — remove it so
                // history never silently loses the attachments.
                if let Err(e) = db.message_delete(mid) {
                    log::warn!("ask: attachment-failure row cleanup failed: {e}");
                }
                return Err(attachment_error(emit, message));
            }
        }
    }

    // The screen-material truth table (spec §Ask flow) — resolves
    // BEFORE the `loading` emit so an explicit ask's captured frame
    // lands in the payload's attachment list (the only pre-chain wait
    // left is the one-shot capture). A failed REQUIRED read ends the
    // run as ask:error.
    let resolved = match resolve_screen(screen_input, vision.as_ref(), emit).await {
        Ok(r) => r,
        Err(msg) => {
            // The persisted row never painted — `loading` first so the
            // card still shows the question under the error toast.
            emit(EV_STATE, make_loading(text, preset_id, &run_attachments));
            emit(EV_ERROR, json!({ "message": msg }));
            emit(EV_STATE, json!({"state": "idle"}));
            return Err(LlmError::Http {
                status: 0,
                message: msg,
            });
        }
    };
    let (frame, screen, shot) = match resolved {
        Some(ScreenMaterial::Text(t)) => (None, Some(t), None),
        Some(ScreenMaterial::Frame(f)) if screen_input.explicit => (None, None, Some(f)),
        Some(ScreenMaterial::Frame(f)) => (Some(f), None, None),
        None => (None, None, None),
    };
    // An explicit screen ask's frame joins the run like a user-picked
    // image: persisted on the user row (history + retries reload it),
    // advertised on `loading`, and sent as pixels — the attachment
    // fallback's vision describe covers text-only providers. A persist
    // hiccup sends the pixels unpersisted rather than blinding the ask.
    if let Some(shot) = shot {
        let jpeg = shot.jpeg;
        match message_id {
            Some(mid) => {
                match persist_attachments(
                    db,
                    &attachments_root,
                    mid,
                    vec![PendingImage {
                        name: "screenshot.jpg".into(),
                        jpeg: jpeg.clone(),
                    }],
                )
                .await
                {
                    Ok((images, meta)) => {
                        user_images.extend(images);
                        run_attachments.extend(meta);
                    }
                    Err(message) => {
                        log::warn!("ask: screenshot persist failed ({message}); sending unpersisted");
                        user_images.push(jpeg);
                    }
                }
            }
            None => user_images.push(jpeg),
        }
    }
    // `loading` announces the run's full attachment list — composer
    // picks plus any just-persisted screenshot.
    let loading = make_loading(text, preset_id, &run_attachments);
    emit(EV_STATE, loading.clone());
    // The turn's spend = the answering attempt plus any attachment
    // describe a text-only retry arms (a failed candidate's usage is
    // unknowable — errors carry none).
    let mut usage = TokenUsage::default();
    // The multimodal fallback: when a chat candidate rejects image
    // input, the vision reader describes the user's attachments ONCE —
    // every later candidate's text-only retry reuses the block.
    let mut image_fallback = ImageFallback {
        vision: vision.as_ref(),
        described: None,
    };

    let mut last_err: Option<LlmError> = None;
    for (i, cand) in candidates.iter().enumerate() {
        // A failover hand-off re-announces `loading` so the card drops
        // the failed attempt's partial chunks before the next stream.
        if i > 0 {
            emit(EV_STATE, loading.clone());
        }
        match stream_candidate(
            &*cand.provider,
            emit,
            &history,
            &listen_history,
            text,
            &user_images,
            frame.as_ref(),
            screen.as_deref(),
            &mut image_fallback,
            &mut usage,
            cancel,
            language,
            instruction,
        )
        .await
        {
            CandidateOutcome::Done(reply) => {
                if let Some(u) = reply.usage {
                    usage.add(&u);
                }
                let usage = (!usage.is_empty()).then_some(usage);
                persist_assistant_message(
                    db,
                    session_id,
                    &reply.full,
                    &cand.id,
                    &cand.model,
                    usage,
                );
                emit(
                    EV_DONE,
                    json!({
                        "full": reply.full,
                        "provider": cand.id,
                        "model": cand.model,
                        "usage": usage
                            .map(|u| json!({"input": u.input, "output": u.output})),
                    }),
                );
                emit(EV_STATE, json!({"state": "idle"}));
                // The sidecar runs after `idle` so the stream's end never
                // waits on it; its spend isn't part of the turn's usage
                // (done/persisted already).
                maybe_title_session(db, &*cand.provider, session_id, text, emit, cancel).await;
                return Ok(reply.full);
            }
            // User row stays — it was already sent. Never fall over on
            // cancel: the user asked to stop, so the chain stops here.
            CandidateOutcome::Cancelled => {
                emit(EV_STATE, json!({"state": "idle"}));
                return Err(LlmError::Http {
                    status: 0,
                    message: "cancelled".to_string(),
                });
            }
            CandidateOutcome::Failed(e) => {
                log::warn!("ask: provider {} failed ({e}); trying next", cand.id);
                last_err = Some(e);
            }
        }
    }
    // Chain exhausted — surface the last failure (most actionable).
    let e = last_err.unwrap_or(LlmError::NoModel);
    emit(EV_ERROR, json!({"message": e.to_string()}));
    emit(EV_STATE, json!({"state": "idle"}));
    Err(e)
}

