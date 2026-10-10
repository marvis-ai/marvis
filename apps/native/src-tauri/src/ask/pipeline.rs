use super::*;
use super::attachments::*;
use super::history::*;
use super::screen::*;
use super::stream::*;
use super::title::*;
use crate::session_lifecycle::{SessionHandoff, SessionLifecycle};

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
    /// The session this run writes into — pre-resolved by `kick` so the
    /// run registers under it before spawning. `None` falls back to
    /// resolving here from `fresh_session`/`listen_id` (the direct
    /// `send_chain` tests' path).
    pub session_id: Option<i64>,
    /// Fallback-resolution input only (see `session_id`): the send
    /// found the card closed, so the open ask session ends and a fresh
    /// one mints.
    pub fresh_session: bool,
    pub regenerate: bool,
    /// Fallback-resolution input AND meeting-context source: the doc
    /// this send is bound to.
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
    /// The consent-gated extraction job — `Some` only when
    /// `[memory].enabled` resolved a usable dedicated provider in
    /// `kick`. Scheduled once, on success only.
    pub memory: Option<MemoryHook>,
    /// The detached session compaction job — scheduled only after a
    /// candidate answers successfully and a plan exists.
    pub compact: Option<CompactHook>,
    /// The production per-session handoff boundary captured synchronously by
    /// `AskService::kick`. Direct pipeline tests may leave this `None`.
    pub session_handoff: Option<SessionHandoff>,
    /// Legacy direct-pipeline lifecycle fallback. Production transfers the
    /// already-owned lease above; tests that exercise deletion can still let
    /// `send_chain` acquire one here.
    pub lifecycle: Option<Arc<SessionLifecycle>>,
    /// The detached title sidecar — spawned on success only, after the
    /// memory hook. `None` skips naming entirely (tests that don't
    /// exercise it).
    pub title: Option<TitleSidecar>,
}

/// Re-read compaction state after history/regenerate processing. The token
/// captured before history is the source-incarnation snapshot; the digest and
/// watermark come only from this post-history read so regenerate clearing is
/// reflected in the live prompt and detached plan.
pub(super) fn compaction_after_history(
    db: &Db,
    session_id: Option<i64>,
    pre_history_token: Option<String>,
    history_rows: &[crate::storage::Message],
) -> (Option<String>, Option<CompactionPlan>) {
    let (Some(sid), Some(pre_history_token)) = (session_id, pre_history_token) else {
        return (None, None);
    };
    match db.session_compaction(sid) {
        Ok(Some((post_history_token, digest, through)))
            if post_history_token == pre_history_token =>
        {
            let plan = compaction_plan(
                sid,
                pre_history_token,
                history_rows,
                digest.clone(),
                through,
            );
            (digest, plan)
        }
        Ok(Some(_)) => {
            // The history snapshot belongs to a different incarnation. Do
            // not expose the new row's digest or schedule old source rows.
            (None, None)
        }
        Ok(None) => (None, None),
        Err(error) => {
            log::warn!("ask: post-history session compaction load failed: {error}");
            (None, None)
        }
    }
}

fn session_token_matches(db: &Db, session_id: Option<i64>, expected_token: Option<&str>) -> bool {
    let (Some(sid), Some(expected_token)) = (session_id, expected_token) else {
        return session_id.is_none();
    };
    match db.session_compaction(sid) {
        Ok(Some((actual_token, _, _))) => actual_token == expected_token,
        Ok(None) => false,
        Err(error) => {
            log::warn!("ask: session incarnation revalidation failed: {error}");
            false
        }
    }
}

fn incarnation_error(
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
) -> LlmError {
    let message = "The Ask session changed while the request was running";
    emit(EV_ERROR, json!({"message": message}));
    emit(EV_STATE, json!({"state": "idle"}));
    LlmError::Http {
        status: 0,
        message: message.to_string(),
    }
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
///    `ask:state{idle}`, then the detached title sidecar
///    ([`TitleSidecar::schedule`]) on a still-untitled session;
///    `Failed` → warn-log, re-emit
///    `loading` (the card resets its buffer — a dead provider's partial
///    chunks must not bleed into the next attempt), and try the NEXT
///    candidate; `Cancelled` → `ask:state{idle}` and stop immediately —
///    the user asked to stop, so no failover may start a new request.
/// 4. Every candidate failed → `ask:error{message}` (the LAST failure's
///    message — the most actionable one) + `ask:state{idle}`.
///
/// Ordinary Db failures are `log::warn`ed and ignored — a storage hiccup must
/// never block the stream. An incarnation mismatch is different: the run is
/// retired before stale history or output can cross into a recreated id.
///
/// Resolves to the full assistant text. Cancellation before an answer returns a
/// `status:0`/`"cancelled"` [`LlmError::Http`] sentinel — the events, not
/// the return value, drive the UI.
/// Exhausting the chain returns the last provider error, or
/// [`LlmError::NoModel`] for an empty chain. Success returns right after
/// `idle` — the title attempt runs detached, so its failure can't touch
/// the answer and `stop` can't recall it; title usage is excluded from
/// the reported turn usage.
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
        session_id,
        fresh_session,
        regenerate,
        listen_id,
        language,
        instruction,
        preset_id,
        attachments,
        attachments_root,
        memory,
        compact,
        session_handoff,
        lifecycle,
        title,
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
    // The run's session — `kick` resolves it up-front (the busy gate
    // registers the run under it before spawn); `None` resolves here
    // instead, the direct-test path: `fresh_session` ends the open
    // conversation, a linked send lands in the listen doc's own ask
    // session, a regenerate keeps the active session — its question IS
    // that session's last user row.
    let session_id =
        session_id.or_else(|| resolve_session(db, fresh_session, regenerate, listen_id));
    // Every emit past session resolution carries the run's session —
    // the webview drops packets bound to a conversation it isn't
    // showing: New Chat / resume leave the run streaming into ITS
    // session, so only a view on that session renders them.
    let emit = &|name: &str, mut payload: serde_json::Value| {
        if let Some(sid) = session_id {
            payload["session_id"] = sid.into();
        }
        emit(name, payload);
    };
    // Capture the session incarnation before history/regenerate work. In
    // production `kick` transfers an already-owned lease and immutable token;
    // this is what makes a queued Ask retain the original boundary even when
    // its pipeline task is delayed. Direct pipeline tests can use the legacy
    // lifecycle fallback below.
    let (pre_history_token, _session_lease) = match session_id {
        Some(sid) => match session_handoff {
            Some(handoff) if handoff.session_id == sid => {
                (Some(handoff.session_token), Some(handoff.lease))
            }
            Some(handoff) => {
                drop(handoff);
                return Err(incarnation_error(emit));
            }
            None => {
                let lease = match lifecycle.as_ref() {
                    Some(lifecycle) => match lifecycle.acquire(sid) {
                        Some(lease) => Some(lease),
                        None => return Err(incarnation_error(emit)),
                    },
                    None => None,
                };
                let token = match db.session_compaction(sid) {
                    Ok(Some((token, _, _))) => Some(token),
                    Ok(None) => return Err(incarnation_error(emit)),
                    Err(error) => {
                        log::warn!("ask: session compaction incarnation load failed: {error}");
                        return Err(incarnation_error(emit));
                    }
                };
                (token, lease)
            }
        },
        None => {
            drop(session_handoff);
            (None, None)
        }
    };
    // Order matters: history is read BEFORE the new user row persists —
    // the new turn is appended separately so it can carry the frame. A
    // regenerate skips the write: its question is the session's last
    // user row, and history ends BEFORE it (a missing tail — shouldn't
    // happen, `retry` resolved the question from that row — degrades
    // to a normal send).
    let (history_rows, re_asked, regen_row_id, regen_atts) = if regenerate {
        match regenerate_tail_rows(db, session_id, pre_history_token.as_deref()) {
            Some((rows, mid, atts)) => (rows, true, Some(mid), atts),
            None => (message_rows(db, session_id), false, None, Vec::new()),
        }
    } else {
        (message_rows(db, session_id), false, None, Vec::new())
    };
    // Read the digest after history/regenerate processing. The helper keeps
    // this post-history state authoritative for prompt injection while only
    // planning when its token still matches the pre-history snapshot.
    let (compaction, compact_plan) = compaction_after_history(
        db,
        session_id,
        pre_history_token.clone(),
        &history_rows,
    );
    // The digest check alone is not enough: the rows themselves came from the
    // earlier snapshot. Retire the run before converting or sending them when
    // the session was deleted/recreated during history work.
    if !session_token_matches(db, session_id, pre_history_token.as_deref()) {
        return Err(incarnation_error(emit));
    }
    // Keep the raw post-history digest in `compaction` for prompt bounding;
    // the detached plan retains its own raw CAS snapshot.
    let compact_prompt = bounded_compaction(compaction.as_deref());
    // Prior history degrades unreadable images to markers. Current-turn
    // attachments are loaded strictly after the fresh screenshot is resolved.
    let history = {
        let root = attachments_root.clone();
        match tokio::task::spawn_blocking(move || rows_to_history_at(&history_rows, &root)).await {
            Ok(Ok(history)) => history,
            Ok(Err(message)) => return Err(attachment_error(emit, message)),
            Err(e) => {
                return Err(attachment_error(
                    emit,
                    format!("Couldn't load attachments: {e}"),
                ))
            }
        }
    };
    // The effective doc link: the send's explicit one, else the resolved
    // session's stored link — a continued doc chat (or its retry) keeps
    // the doc's context even though the caller passed none.
    let listen_id =
        listen_id.or_else(|| session_id.and_then(|sid| db.session_listen_id(sid).ok().flatten()));
    let listen_history = load_listen_context(db, listen_id);
    // The memory profile snapshot for this run — loaded once, shared by
    // every candidate/retry. A storage hiccup degrades to `None` (the
    // plain prompt); it must never fail the ask.
    let memory_profile = match db.memory_profile() {
        Ok(rows) => crate::memory::profile_prompt(&rows),
        Err(error) => {
            log::warn!("ask: memory profile load failed: {error}");
            None
        }
    };
    let mut user_images: Vec<Vec<u8>> = Vec::new();
    // The run's user-turn attachments — the `loading` payload advertises
    // them so the card/resync renders the persisted message correctly.
    let mut run_attachments: Vec<MessageAttachment> = regen_atts;
    // The user row's id — persisted fresh for sends, resolved for
    // regens (the re-asked row's). `None` when storage hiccuped.
    let message_id = if re_asked {
        regen_row_id
    } else {
        persist_user_message(
            db,
            session_id,
            pre_history_token.as_deref(),
            text,
            preset_id,
        )
    };
    if !session_token_matches(db, session_id, pre_history_token.as_deref()) {
        return Err(incarnation_error(emit));
    }
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
        match persist_attachments(
            db,
            &attachments_root,
            session_id,
            pre_history_token.as_deref(),
            mid,
            pending,
        )
        .await
        {
            Ok((images, meta)) => {
                user_images = images;
                run_attachments = meta;
            }
            Err(message) => {
                // The row claims images it never got — remove it so
                // history never silently loses the attachments. The cleanup
                // is incarnation-bound too; a recreated id is untouchable.
                if let (Some(sid), Some(token)) =
                    (session_id, pre_history_token.as_deref())
                {
                    match db.message_delete_for_session(sid, token, &[mid]) {
                        Ok(Some(_)) | Ok(None) => {}
                        Err(e) => log::warn!("ask: attachment-failure row cleanup failed: {e}"),
                    }
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
            emit(EV_STATE, make_loading(text, preset_id, &run_attachments, 0, re_asked));
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
    if re_asked {
        if shot.is_some() {
            run_attachments.retain(|attachment| attachment.name != "screenshot.jpg");
        }
        let root = attachments_root.clone();
        let attachments = run_attachments.clone();
        user_images = match tokio::task::spawn_blocking(move || {
            attachments
                .iter()
                .map(|a| read_attachment(&root, a))
                .collect::<Result<Vec<_>, _>>()
        })
        .await
        {
            Ok(Ok(images)) => images,
            Ok(Err(message)) => return Err(attachment_error(emit, message)),
            Err(e) => {
                return Err(attachment_error(
                    emit,
                    format!("Couldn't load attachments: {e}"),
                ))
            }
        };
    }
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
                    session_id,
                    pre_history_token.as_deref(),
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
    // Revalidate after all current-turn persistence and immediately before the
    // provider path. This closes the history-capture → provider window; the
    // assistant write below has the same token guard for a later deletion.
    if !session_token_matches(db, session_id, pre_history_token.as_deref()) {
        return Err(incarnation_error(emit));
    }
    // `loading` announces the run's full attachment list — composer
    // picks plus any just-persisted screenshot.
    let loading = make_loading(text, preset_id, &run_attachments, 0, re_asked);
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
        // the failed attempt's partial chunks before the next stream —
        // `attempt: i` marks it as a retry of THIS run, not a new turn.
        if i > 0 {
            let mut retry = loading.clone();
            retry["attempt"] = json!(i);
            emit(EV_STATE, retry);
        }
        // Check once per candidate so a delete/recreate between failover
        // attempts cannot send the old snapshot to the next provider.
        if !session_token_matches(db, session_id, pre_history_token.as_deref()) {
            return Err(incarnation_error(emit));
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
            memory_profile.as_deref(),
            compact_prompt.as_deref(),
        )
        .await
        {
            CandidateOutcome::Done(reply) => {
                // The provider may have completed after deletion/recreation.
                // Do not emit or persist its answer as the new incarnation.
                if !session_token_matches(db, session_id, pre_history_token.as_deref()) {
                    return Err(incarnation_error(emit));
                }
                if let Some(u) = reply.usage {
                    usage.add(&u);
                }
                let usage = (!usage.is_empty()).then_some(usage);
                if !persist_assistant_message(
                    db,
                    session_id,
                    pre_history_token.as_deref(),
                    &reply.full,
                    &cand.id,
                    &cand.model,
                    usage,
                ) {
                    return Err(incarnation_error(emit));
                }
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
                // Memory extraction runs on success only — the raw user
                // text (never the reply, screen, or attachments) is the
                // whole source, and the scheduled job owns the ids of
                // the turn it came from. Fire-and-forget: the answer
                // never waits on it and its failure only warns.
                if let Some(hook) = memory {
                    hook.schedule(text.to_string(), session_id, message_id);
                }
                // The title sidecar is detached like the memory hook —
                // `idle` already went out, the run never waits on it,
                // and a `stop` after this point can't recall it (an
                // ended session still gets named).
                if let Some(sidecar) = title {
                    sidecar.schedule(
                        Arc::clone(&cand.provider),
                        session_id,
                        pre_history_token.clone(),
                        text.to_string(),
                    );
                }
                if let (Some(hook), Some(plan)) = (compact, compact_plan) {
                    hook.maybe_schedule(Arc::clone(&cand.provider), plan);
                }
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

