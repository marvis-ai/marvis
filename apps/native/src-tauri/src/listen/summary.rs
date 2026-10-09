use super::*;
use super::turns::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListenSummary {
    pub tldr: String,
    pub bullets: Vec<String>,
    pub follow_ups: Vec<String>,
    pub topic: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListenSummaryEvent {
    pub session_id: i64,
    #[serde(flatten)]
    pub summary: ListenSummary,
}

pub fn parse_summary(raw: &str) -> anyhow::Result<ListenSummary> {
    let value: serde_json::Value = serde_json::from_str(raw.trim())?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("summary is not an object"))?;
    let tldr = object
        .get("tldr")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if tldr.is_empty() {
        anyhow::bail!("summary has no tldr");
    }
    let strings = |key: &str, max: usize| -> Vec<String> {
        object
            .get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .take(max)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let topic = object
        .get("topic")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    Ok(ListenSummary {
        tldr,
        bullets: strings("bullets", 5),
        follow_ups: strings("follow_ups", 3),
        topic,
    })
}

/// Shared terminal-failure path for STT-provider and audio-source errors:
/// cancel the run, mark status `error`, end the open session row (so
/// History doesn't show it Live until the next `stop()`/`start()` sweep —
/// `session_end` only writes open rows, so `stop()` settling after this
/// can't double-write), and emit one sanitized `listen:error`.
pub(super) fn report_terminal_error(context: &SessionContext, message: &str) {
    context.cancel.store(true, Ordering::Release);
    let mut status = context.status.lock();
    status.state = "error".into();
    status.error = Some(ListenError {
        message: sanitize_provider_error(message),
        needs_setup: false,
    });
    let error = status.error.clone().expect("just stored");
    drop(status);
    if let Err(db_error) = context.db.session_end(context.session_id) {
        log::warn!("listen: session_end on runtime error failed: {db_error}");
    }
    (context.emit)(ListenEvent::Error {
        message: error.message,
        needs_setup: error.needs_setup,
    });
}

pub(super) fn persist_turn(context: &Arc<SessionContext>, turn: ClosedTurn) {
    let inserted = context.db.transcript_add(
        context.session_id,
        speaker_name(turn.speaker),
        &turn.text,
        turn.speaker_idx,
        turn.audio_start_ms,
    );
    if let Err(error) = &inserted {
        log::warn!("listen transcript persistence failed: {error}");
    }

    let count = if inserted.is_ok() {
        context.persisted_turns.fetch_add(1, Ordering::AcqRel) + 1
    } else {
        // A database failure must not hide a valid transcript event.
        {
            context.status.lock().turns += 1;
        }
        (context.emit)(ListenEvent::Turn(ListenTurn {
            audio_start_ms: turn.audio_start_ms,
            speaker: turn.speaker,
            speaker_idx: turn.speaker_idx,
            text: turn.text,
            ts: turn.ts,
            session_id: context.session_id,
            finality: true,
        }));
        return;
    };
    {
        context.status.lock().turns += 1;
    }
    (context.emit)(ListenEvent::Turn(ListenTurn {
        audio_start_ms: turn.audio_start_ms,
        speaker: turn.speaker,
        speaker_idx: turn.speaker_idx,
        text: turn.text,
        ts: turn.ts,
        session_id: context.session_id,
        finality: true,
    }));

    if count % SUMMARY_EVERY == 0 {
        schedule_summary(context);
    }
}

#[derive(Default)]
pub(super) struct SummarySchedule {
    running: bool,
    pending: bool,
}

impl SummarySchedule {
    fn request(&mut self) -> bool {
        if self.running {
            self.pending = true;
            false
        } else {
            self.running = true;
            true
        }
    }

    fn finish(&mut self) -> bool {
        if std::mem::take(&mut self.pending) {
            true
        } else {
            self.running = false;
            false
        }
    }

    /// A worker that died mid-run left `running` set — clear both flags so
    /// the next boundary can schedule again. A fresh run summarizes the
    /// full transcript, so a dropped `pending` mark is covered by it.
    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Resets the schedule if the worker exits without `finish()` — a panic
/// mid-run would otherwise leave `running` set forever, silently ending
/// summaries for the rest of the session. Disarmed on normal exit.
struct SummaryResetGuard<'a> {
    schedule: &'a Mutex<SummarySchedule>,
    armed: bool,
}

impl Drop for SummaryResetGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.schedule.lock().reset();
        }
    }
}

fn schedule_summary(context: &Arc<SessionContext>) {
    if !context.summary_schedule.lock().request() {
        return;
    }
    let context = context.clone();
    std::thread::spawn(move || {
        let mut reset = SummaryResetGuard {
            schedule: &context.summary_schedule,
            armed: true,
        };
        loop {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()
                .and_then(|runtime| {
                    runtime
                        .block_on(async {
                            tokio::time::timeout(
                                SUMMARY_TIMEOUT,
                                generate_summary(
                                    &context.db,
                                    context.session_id,
                                    &context.config,
                                    &context.keystore,
                                ),
                            )
                            .await
                        })
                        .ok()
                        .and_then(Result::ok)
                });
            if let Some(summary) = result {
                (context.emit)(ListenEvent::Summary(ListenSummaryEvent {
                    session_id: context.session_id,
                    summary,
                }));
            }
            // Finish and claim any pending work under one lock, so a new
            // boundary cannot start a second worker in between these steps.
            if !context.summary_schedule.lock().finish() {
                break;
            }
        }
        reset.armed = false;
    });
}

pub(super) fn preserve_previous_summary(
    result: anyhow::Result<ListenSummary>,
    previous: Option<ListenSummary>,
) -> anyhow::Result<ListenSummary> {
    result.or_else(|error| previous.ok_or(error))
}

pub(super) fn format_previous_summary(summary: &ListenSummary) -> String {
    format!(
        "TLDR: {}\nTopic: {}\nBullets: {}\nSuggested questions: {}",
        summary.tldr,
        summary.topic.as_deref().unwrap_or("none"),
        summary.bullets.join("; "),
        summary.follow_ups.join("; ")
    )
}

pub(super) fn build_summary_messages(
    history: &str,
    previous: Option<&str>,
    language: &str,
    focus: &str,
) -> [ChatMessage; 2] {
    [
        ChatMessage::text(Role::System, summary_system_prompt_for(language, focus)),
        ChatMessage::text(Role::User, summary_context(history, previous)),
    ]
}

/// The summary's transcript input — the session's full persisted
/// transcript, oldest first, as `speaker: text` lines.
pub(super) fn summary_transcript(db: &Db, session_id: i64) -> anyhow::Result<String> {
    let rows = db.transcripts_for(session_id, None)?;
    Ok(rows
        .iter()
        .map(|t| format!("{}: {}", t.speaker, t.content))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Generate and persist a bounded structured summary. Provider failures are deliberately non-fatal.
pub async fn generate_summary(
    db: &Db,
    session_id: i64,
    config: &Config,
    keystore: &Keystore,
) -> anyhow::Result<ListenSummary> {
    let candidates = crate::provider_candidates(config, keystore);
    let previous = match db.summary_latest(session_id) {
        Ok(summary) => summary.map(|summary| ListenSummary {
            tldr: summary.tldr,
            bullets: summary.bullets,
            follow_ups: summary.follow_ups,
            topic: summary.topic,
        }),
        Err(error) => {
            log::warn!("listen summary history lookup failed: {error}");
            None
        }
    };
    let history = match summary_transcript(db, session_id) {
        Ok(history) => history,
        Err(error) => {
            log::warn!("listen summary transcript load failed: {error}");
            return preserve_previous_summary(Err(error), previous);
        }
    };
    let previous_text = previous.as_ref().map(format_previous_summary);
    let messages = build_summary_messages(
        &history,
        previous_text.as_deref(),
        &config.app.main_language,
        &config.recording.summary_prompt,
    );
    for candidate in candidates {
        let mut sink = |_token: &str| {};
        match candidate.provider.stream_chat(&messages, &mut sink).await {
            Ok(reply) => match parse_summary(&reply.full) {
                Ok(summary) => {
                    if let Err(error) = db.summary_upsert(
                        session_id,
                        &summary.tldr,
                        &summary.bullets,
                        &summary.follow_ups,
                        summary.topic.as_deref(),
                    ) {
                        log::warn!("listen summary persistence failed: {error}");
                    }
                    return Ok(summary);
                }
                Err(error) => log::warn!("listen summary parse failed: {error}"),
            },
            Err(error) => log::warn!("listen summary provider failed: {error}"),
        }
    }
    preserve_previous_summary(
        Err(anyhow::anyhow!("no provider produced a valid summary")),
        previous,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_during_a_run_coalesce_and_are_drained_before_idle() {
        let mut schedule = SummarySchedule::default();
        assert!(schedule.request());
        assert!(!schedule.request());
        assert!(!schedule.request());
        assert!(schedule.finish()); // the same worker must refresh the summary
        assert!(!schedule.request()); // it still owns the next pass
        assert!(schedule.finish());
        assert!(!schedule.finish());
        assert!(schedule.request()); // idle can start a new worker
    }

    /// A worker that panics before `finish()` drops its reset guard — the
    /// stale `running` flag must not wedge the scheduler, and the pending
    /// mark is covered by the fresh run that the next boundary starts.
    #[test]
    fn crashed_worker_resets_so_the_next_boundary_reschedules() {
        let mut schedule = SummarySchedule::default();
        assert!(schedule.request());
        assert!(!schedule.request()); // pending work while it ran
        schedule.reset();
        assert!(schedule.request()); // a fresh worker can start
        assert!(!schedule.finish());
    }
}
