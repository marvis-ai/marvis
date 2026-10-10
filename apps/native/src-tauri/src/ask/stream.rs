use super::*;

/// The run's multimodal fallback — the `[vision]` provider describes
/// the user's attachments when a chat candidate can't take image
/// input, the same idea as `resolve_screen` turning a frame into
/// `<screen_context>`. `described` memoizes the `<attached_images>`
/// block: one vision read per run, shared by every candidate's
/// text-only retry.
pub(super) struct ImageFallback<'a> {
    pub(super) vision: Option<&'a ProviderCandidate>,
    pub(super) described: Option<String>,
}

impl ImageFallback<'_> {
    /// Arm the text-only retry by describing `images` through the
    /// vision provider. `Ok(true)` = `described` now holds the block;
    /// `Ok(false)` = cancelled mid-read; `Err` = no usable vision read
    /// (reader unconfigured, failed, or answered empty — the caller
    /// hands the original rejection off to the next candidate).
    async fn describe(
        &mut self,
        images: &[Vec<u8>],
        question: &str,
        usage: &mut TokenUsage,
        cancel: &CancellationToken,
    ) -> Result<bool, LlmError> {
        if self.described.is_some() {
            return Ok(true);
        }
        let Some(vis) = self.vision else {
            return Err(LlmError::NoModel);
        };
        let reply = match crate::screen_read::describe_images(
            &*vis.provider,
            images,
            question,
            cancel,
        )
        .await
        {
            Ok(Some(reply)) => reply,
            Ok(None) => return Ok(false),
            Err(e) => return Err(e),
        };
        if let Some(u) = reply.usage {
            usage.add(&u);
        }
        let described = reply.full.trim().to_string();
        // An empty read is no read — a text-only retry without the
        // block would silently drop the attachments it exists to carry.
        if described.is_empty() {
            return Err(LlmError::NoModel);
        }
        self.described = Some(described);
        Ok(true)
    }
}

/// Every `ImageJpeg` part in `history` becomes a text marker — a
/// text-only retry can't carry bytes the model already rejected, and
/// an explicit marker keeps "an image was attached here" honest
/// without the pixels.
pub(super) fn text_only_history(history: &[ChatMessage]) -> Vec<ChatMessage> {
    history
        .iter()
        .map(|m| ChatMessage {
            role: m.role,
            content: m
                .content
                .iter()
                .map(|part| match part {
                    ContentPart::ImageJpeg(_) => {
                        ContentPart::Text("[image attachment]".to_string())
                    }
                    part => part.clone(),
                })
                .collect(),
        })
        .collect()
}

/// Any image parts in the history tail — a text-only model rejects
/// them the same way it rejects the current turn's attachments.
pub(super) fn history_has_images(history: &[ChatMessage]) -> bool {
    history
        .iter()
        .any(|m| m.content.iter().any(|p| matches!(p, ContentPart::ImageJpeg(_))))
}

/// One candidate's full attempt: stream, and on a
/// `MultimodalUnsupported` rejection retry ONCE text-only — the user's
/// attachments vision-described into `<attached_images>` when the
/// reader is configured, a screen frame dropped, history images
/// marked. Emits `ask:chunk`/`ask:state{streaming}` but never
/// `done`/`error`/`idle` — the chain owns the run's protocol; this
/// owns one provider's messages.
#[allow(clippy::too_many_arguments)]
pub(super) async fn stream_candidate(
    provider: &dyn Provider,
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
    history: &[ChatMessage],
    listen_history: &str,
    text: &str,
    user_images: &[Vec<u8>],
    frame: Option<&Frame>,
    screen: Option<&str>,
    image_fallback: &mut ImageFallback<'_>,
    usage: &mut TokenUsage,
    cancel: &CancellationToken,
    language: &str,
    instruction: Option<&str>,
    profile: Option<&str>,
    compaction: Option<&str>,
) -> CandidateOutcome {
    let mut streaming = false;
    let mut msgs = build_messages(
        history,
        listen_history,
        text,
        user_images,
        frame,
        screen,
        None,
        language,
        instruction,
        profile,
        compaction,
    );
    let mut retried = false;
    loop {
        match stream_once(provider, &msgs, emit, cancel, &mut streaming).await {
            StreamOutcome::Done(reply) => return CandidateOutcome::Done(reply),
            StreamOutcome::Cancelled => return CandidateOutcome::Cancelled,
            StreamOutcome::Failed(e) => {
                // A vision-incapable model may retry ONCE text-only —
                // but the user's own attachments are never silently
                // dropped: the vision reader describes them into an
                // `<attached_images>` block first (one read per run —
                // later candidates reuse it). No reader or a failed
                // read hands off to the next candidate unchanged.
                if !retried && e.is_multimodal() {
                    if !user_images.is_empty() {
                        match image_fallback
                            .describe(user_images, text, usage, cancel)
                            .await
                        {
                            Ok(true) => {
                                retried = true;
                                msgs = build_messages(
                                    &text_only_history(history),
                                    listen_history,
                                    text,
                                    &[],
                                    None,
                                    screen,
                                    image_fallback.described.as_deref(),
                                    language,
                                    instruction,
                                    profile,
                                    compaction,
                                );
                                continue;
                            }
                            Ok(false) => return CandidateOutcome::Cancelled,
                            Err(_) => {}
                        }
                    } else if frame.is_some() || history_has_images(history) {
                        // No new attachments: the droppable material is
                        // the frame, plus any persisted history images a
                        // text-only model would reject — those degrade
                        // to `[image attachment]` markers.
                        retried = true;
                        msgs = build_messages(
                            &text_only_history(history),
                            listen_history,
                            text,
                            &[],
                            None,
                            screen,
                            None,
                            language,
                            instruction,
                            profile,
                            compaction,
                        );
                        continue;
                    }
                }
                return CandidateOutcome::Failed(e);
            }
        }
    }
}

/// One provider's result within the chain — `Failed` hands off to the
/// next candidate; `Cancelled`/`Done` end the run.
pub(super) enum CandidateOutcome {
    Done(StreamReply),
    Cancelled,
    Failed(LlmError),
}

/// One `stream_chat` attempt's result.
pub(super) enum StreamOutcome {
    Done(StreamReply),
    /// `cancel` fired — the request future was dropped mid-flight.
    Cancelled,
    Failed(LlmError),
}

/// Race one `stream_chat` against cancellation. Emits
/// `ask:state{streaming}` on the FIRST token (`streaming` persists across
/// the multimodal retry so it fires at most once per send) and
/// `ask:chunk` per token. `on_token` re-checks `is_cancelled` itself —
/// a token racing the cancellation must never reach the webview.
pub(super) async fn stream_once(
    provider: &dyn Provider,
    msgs: &[ChatMessage],
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
    cancel: &CancellationToken,
    streaming: &mut bool,
) -> StreamOutcome {
    let mut on_token = |token: &str| {
        if cancel.is_cancelled() {
            return;
        }
        if !*streaming {
            *streaming = true;
            emit(EV_STATE, json!({"state": "streaming"}));
        }
        emit(EV_CHUNK, json!({"text": token}));
    };
    tokio::select! {
        _ = cancel.cancelled() => StreamOutcome::Cancelled,
        r = provider.stream_chat(msgs, &mut on_token) => match r {
            Ok(reply) => StreamOutcome::Done(reply),
            Err(e) => StreamOutcome::Failed(e),
        },
    }
}

/// `[system] + history + [user]` — history rows may carry their own
/// persisted images; the new user turn pairs `text` with the composer
/// `images` first and the frame's JPEG after when one was captured,
/// else with the screen reader's `<screen_context>` description when
/// a vision provider read it; text-only otherwise (and on the
/// frame-dropping retry — the description, when present, survives it).
/// On the attachment-describe retry the caller passes `attached` — the
/// vision reader's `<attached_images>` text — and no image parts, so a
/// text-only model sees the images' content without their bytes.
/// The system message is
/// `live_system_prompt_with_profile(language, instruction, profile, compaction)` —
/// the armed `instruct` preset's text after the language directive, followed
/// by the memory profile and session digest as untrusted data.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_messages(
    history: &[ChatMessage],
    listen_history: &str,
    text: &str,
    images: &[Vec<u8>],
    frame: Option<&Frame>,
    screen: Option<&str>,
    attached: Option<&str>,
    language: &str,
    instruction: Option<&str>,
    profile: Option<&str>,
    compaction: Option<&str>,
) -> Vec<ChatMessage> {
    let mut msgs = Vec::with_capacity(history.len() + 2);
    msgs.push(ChatMessage::text(
        Role::System,
        live_system_prompt_with_profile(language, instruction, profile, compaction),
    ));
    msgs.extend(history.iter().cloned());
    let request = live_user_prompt(text, listen_history, screen, attached);
    let mut user = ChatMessage::user_with_images(request, images.to_vec());
    if let Some(frame) = frame {
        user.content.push(ContentPart::ImageJpeg(frame.jpeg.clone()));
    }
    msgs.push(user);
    msgs
}

