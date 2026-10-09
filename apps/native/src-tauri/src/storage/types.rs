pub(super) type SummaryRow = (i64, i64, String, String, String, Option<String>, i64, i64);

/// A row of `sessions`. `kind` maps to the `type` column (`type` is a Rust
/// keyword); `title`/`ended_at`/`audio_file` are `NULL` until set.
/// `Serialize` so the `session_list` command can return rows to the webview.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Session {
    pub id: i64,
    pub kind: String,
    pub title: Option<String>,
    /// The retained session recording (mixed mic+system WAV under
    /// `~/.marvis/audios`) — listen sessions only.
    pub audio_file: Option<String>,
    /// The STT engine label that captured the session
    /// (`"whisper large-v3"`-shaped) — listen only; NULL on ask rows
    /// and sessions written before the column existed.
    pub stt: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub last_active_at: i64,
}

/// A row of `message_attachments` — metadata for one managed image.
/// `path` points at the normalized JPEG under `~/.marvis/attachments`
/// (a generated name — the original `name` is display metadata only).
/// `Serialize` — it rides `Message` rows out of `session_get` and the
/// `ask:state{loading}` payload.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MessageAttachment {
    pub id: i64,
    pub message_id: i64,
    /// The original filename — alt text/tooltip; never a path.
    pub name: String,
    pub path: String,
    pub mime: String,
    pub bytes: i64,
    /// Position within the message — the provider image-part order.
    pub position: i64,
}

/// The not-yet-persisted half of a [`MessageAttachment`] — what
/// [`Db::attachments_add`] writes. `position` is the caller's pick
/// order (the composer's chip order).
#[derive(Debug, Clone, PartialEq)]
pub struct NewAttachment {
    pub name: String,
    pub path: String,
    pub mime: String,
    pub bytes: i64,
    pub position: i64,
}

/// A row of `messages`. `Serialize` for the `session_get` command.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Message {
    pub id: i64,
    pub session_id: i64,
    pub role: String,
    pub content: String,
    /// Attached images — user rows only; empty when the turn is
    /// text-only or predates the column.
    pub attachments: Vec<MessageAttachment>,
    /// Assistant-row provenance + spend — NULL on user turns and on rows
    /// written before the columns existed.
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    /// The `instruct` preset armed on this user send (presets.rs id) —
    /// `ask_retry` re-resolves it. NULL on assistant rows and rows
    /// written before presets existed.
    pub preset: Option<String>,
    pub ts: i64,
}

/// Optional provenance columns: the armed preset rides user rows;
/// provider/model/tokens ride assistant replies — the card's ⋯ menu
/// reads them back. `Default` leaves the columns NULL, which is what
/// every non-reply write wants.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageMeta {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    /// The armed preset id — user rows only.
    pub preset: Option<String>,
}

/// A persisted speaker turn from a listen session.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Transcript {
    pub id: i64,
    pub session_id: i64,
    /// The capture channel (`me` = mic, `them` = system audio) — not a
    /// person. The display name derives from `speaker` + `speaker_idx`.
    pub speaker: String,
    /// Diarized voice cluster within `speaker`'s channel — `NULL` when the
    /// session ran without diarization or the turn was unlabelable.
    pub speaker_idx: Option<i64>,
    pub audio_start_ms: Option<i64>,
    pub content: String,
    pub ts: i64,
}

/// A persisted structured summary from a listen session — unique per
/// session: `created_at` stamps the first write, `updated_at` the latest.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Summary {
    pub id: i64,
    pub session_id: i64,
    pub tldr: String,
    pub bullets: Vec<String>,
    pub follow_ups: Vec<String>,
    pub topic: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A row of `memories` — one stored identity/preference fact.
/// `Serialize` so the `memory_list` command hands rows to the settings
/// UI verbatim. `category`/`attribute`/`value` are the display columns;
/// `basis` (`"explicit"`/`"inferred"`) and `confidence` are provenance.
/// `source` is `"automatic"` (extracted by the Memory LLM) or `"manual"`
/// (user-edited — automatic writes never overwrite it).
/// `source_*` ids point at the ask turn a fact came from; `ON DELETE
/// SET NULL` clears them when history is wiped.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Memory {
    pub id: i64,
    pub category: String,
    pub attribute: String,
    pub value: String,
    pub confidence: f64,
    pub basis: String,
    pub source: String,
    pub source_session_id: Option<i64>,
    pub source_message_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A parsed extraction candidate — the not-yet-persisted half of a
/// [`Memory`]. `memory_apply` upserts these under `source = "auto"`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MemoryCandidate {
    pub category: String,
    pub attribute: String,
    pub value: String,
    pub confidence: f64,
    pub basis: String,
}
