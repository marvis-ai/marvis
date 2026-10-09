use super::*;

/// What the ask chain attaches: the truth table's outcomes.
pub(crate) enum ScreenMaterial {
    /// The reader's cached description → `<screen_context>` text.
    Text(String),
    /// Raw JPEG frame — an explicit ask's becomes a persisted user
    /// attachment (same pipeline as composer picks); an ambient one
    /// rides the turn directly when no vision reader is configured.
    Frame(Frame),
}

/// Everything `resolve_screen` needs — bundled so `send_chain` keeps
/// one param instead of seven. No `AppHandle`: side-effects are injected
/// seams so the truth table is unit-testable.
pub(crate) struct ScreenInput<'a> {
    pub reader: &'a screen_read::ScreenReader,
    pub ring: &'a Mutex<RingBuffer>,
    /// `state.capture` is live — ring frames are fresh.
    pub capture_running: bool,
    /// `with_screen`/`screen_required`/intent — the user asked about
    /// the screen: while recording this attaches the newest ring frame
    /// (never just the cache), and no material at all errors rather
    /// than answering blind.
    pub explicit: bool,
    /// explicit || capture_running — the OFF branch's shot gate.
    pub needs_screen: bool,
    pub read_interval_secs: u64,
    /// `crate::permissions::screen_status` in prod; stubbed in tests.
    pub screen_permission: fn() -> bool,
    /// `crate::capture::shot_fullscreen` in prod; stubbed in tests.
    pub shot: fn() -> anyhow::Result<Option<Frame>>,
}

/// Unix seconds — same `SystemTime` pattern as `encode_frame`/
/// `seed_context`; the cached-context age annotation needs it.
pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The screen-material truth table (spec §Ask flow):
/// recording ON + intent → the newest ring frame (a user attachment);
/// recording ON ambient → cached context (vision) / ring frame;
/// OFF + intent → one-shot frame (a user attachment);
/// OFF + no intent → None. `Err` = required capture failed → ask:error.
/// `emit` is the ask task's gen-guarded sender — reused for the
/// `capture:permission-needed` broadcast (it targets the bar window,
/// which is the toast's only consumer).
pub(crate) async fn resolve_screen(
    input: &ScreenInput<'_>,
    vision: Option<&ProviderCandidate>,
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
) -> Result<Option<ScreenMaterial>, String> {
    if input.capture_running {
        let material = if vision.is_some() {
            // The reader's cached context — annotate age when it's
            // older than two read ticks.
            let cached = || {
                input.reader.context().map(|c| {
                    let age = unix_now() - c.ts;
                    let text = if age > (input.read_interval_secs * 2) as i64 {
                        format!("{}\n(captured ~{}s ago)", c.text, age)
                    } else {
                        c.text
                    };
                    ScreenMaterial::Text(text)
                })
            };
            if input.explicit {
                // Intent / Cmd+Enter means "the screen NOW" — attach
                // the newest ring frame (the ring respects the picked
                // scope; a fullscreen one-shot would not). An empty
                // ring or revoked permission falls back to the cache.
                match input.ring.lock().latest() {
                    Some(f) if (input.screen_permission)() => {
                        Some(ScreenMaterial::Frame(f))
                    }
                    Some(_) => {
                        emit(
                            "capture:permission-needed",
                            json!({ "permission": "screen" }),
                        );
                        cached()
                    }
                    None => cached(),
                }
            } else {
                cached()
            }
        } else {
            // No vision reader: the freshest ring frame carries the
            // screen — attached for explicit asks, ambient context
            // otherwise. Permission revoked mid-session → drop the
            // stale frame and warn the UI.
            let frame = input.ring.lock().latest();
            if frame.is_some() && !(input.screen_permission)() {
                emit(
                    "capture:permission-needed",
                    json!({ "permission": "screen" }),
                );
                None
            } else {
                frame.map(ScreenMaterial::Frame)
            }
        };
        // An explicit screen ask must never answer blind — no material
        // is the pre-redesign "No frame captured" → ask:error. Ambient
        // asks (explicit=false) still get Ok(None) → plain text.
        if material.is_none() && input.explicit {
            return Err("No screen material captured — check screen permission".into());
        }
        return Ok(material);
    }
    if !input.needs_screen {
        return Ok(None);
    }
    // One-shot: SCScreenshotManager is a sync Cocoa call — keep it off
    // the async executor.
    let frame = match tokio::task::spawn_blocking(input.shot).await {
        Ok(Ok(Some(f))) => f,
        Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
            // A failed shot with revoked permission is what the
            // pre-redesign pre-flight check surfaced — keep the toast
            // emit ahead of the error.
            if !(input.screen_permission)() {
                emit(
                    "capture:permission-needed",
                    json!({ "permission": "screen" }),
                );
            }
            // The ask explicitly wanted the screen — a text-only
            // fallback would answer blind (the confabulation failure
            // this redesign exists to kill).
            return Err("Screenshot failed — check screen permission".into());
        }
    };
    log::info!(
        "screen_read: one-shot screenshot — {}x{}, {}B jpeg",
        frame.width,
        frame.height,
        frame.jpeg.len()
    );
    // Always explicit here (`needs_screen && !capture_running`):
    // send_chain persists the frame as a user attachment.
    Ok(Some(ScreenMaterial::Frame(frame)))
}

