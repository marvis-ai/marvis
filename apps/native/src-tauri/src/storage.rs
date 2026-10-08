//! `marvis.db` — SQLite persistence for sessions and AI messages.
//!
//! One [`Db`] owns one `Mutex<Connection>`; every method locks it per call,
//! so `&self` suffices and the whole handle is `Send + Sync`. `Db::at`
//! creates the parent directory, applies `PRAGMA journal_mode=WAL` and
//! `PRAGMA foreign_keys=ON` (the latter is per-connection — it must run on
//! every open or `ON DELETE CASCADE` silently stops working), creates the
//! tables if missing, and chmods the file `0600` like `keys.json`.
//!
//! Schema (spec §Persistence):
//!
//! ```sql
//! sessions(id PK, type 'ask'|'listen', title?, audio_file?, stt?, started_at, ended_at?, last_active_at)
//! messages(id PK, session_id FK → sessions.id ON DELETE CASCADE, role, content,
//!          provider?, model?, tokens_in?, tokens_out?, preset?, ts)
//! transcripts(id PK, session_id FK → sessions.id ON DELETE CASCADE, speaker, speaker_idx?, content, ts)
//! summaries(id PK, session_id FK → sessions.id ON DELETE CASCADE UNIQUE, tldr, bullets, follow_ups, topic?, created_at, updated_at)
//! ```
//!
//! `summaries` holds one row per session — the live summary is an upsert,
//! not a version history: `created_at` stamps the first write,
//! `updated_at` the latest. `sessions.title` is persisted (an ask
//! session's generated title / newest listen summary topic) rather than
//! computed at read — a NULL ask title reads as the first user message
//! via `session_list`'s COALESCE until the generated one lands.
//!
//! All timestamps are unix-epoch seconds (`i64`).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};

use crate::paths;

type SummaryRow = (i64, i64, String, String, String, Option<String>, i64, i64);

const SCHEMA: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA foreign_keys = ON;

    CREATE TABLE IF NOT EXISTS sessions (
        id             INTEGER PRIMARY KEY,
        type           TEXT NOT NULL,
        title          TEXT,
        audio_file     TEXT,
        stt            TEXT,
        listen_id      INTEGER,
        started_at     INTEGER NOT NULL,
        ended_at       INTEGER,
        last_active_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS messages (
        id         INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL,
        role       TEXT NOT NULL,
        content    TEXT NOT NULL,
        provider   TEXT,
        model      TEXT,
        tokens_in  INTEGER,
        tokens_out INTEGER,
        preset     TEXT,
        ts         INTEGER NOT NULL,
        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS transcripts (
        id          INTEGER PRIMARY KEY,
        session_id  INTEGER NOT NULL,
        speaker     TEXT NOT NULL,
        speaker_idx INTEGER,
        content     TEXT NOT NULL,
        ts          INTEGER NOT NULL,
        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS summaries (
        id         INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL UNIQUE,
        tldr       TEXT NOT NULL,
        bullets    TEXT NOT NULL,
        follow_ups TEXT NOT NULL,
        topic      TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
    );
";

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

/// A row of `messages`. `Serialize` for the `session_get` command.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Message {
    pub id: i64,
    pub session_id: i64,
    pub role: String,
    pub content: String,
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

/// The database handle. Cheap to share: all state lives behind the mutex.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// Database bound to the real `~/.marvis/marvis.db`.
    pub fn open() -> anyhow::Result<Self> {
        Self::at(paths::db_file())
    }

    /// Database bound to an explicit path (tests inject a tmp file).
    /// Creates parent dirs, applies pragmas + schema, chmods the file 0600.
    pub fn at(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        migrate(&conn)?;
        conn.execute_batch(SCHEMA)?;
        // Fresh databases have no legacy titles to backfill on restart.
        conn.execute(
            "INSERT OR IGNORE INTO migrations (name) VALUES ('ask_title_backfill')",
            [],
        )?;
        // After open so an existing file's mode is corrected too.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// The most recently active open (`ended_at IS NULL`) session of
    /// `kind`, or `None` — the read half of
    /// `session_get_or_create_active` for callers that must not create
    /// (e.g. `session_end_active`).
    pub fn session_active_id(&self, kind: &str) -> anyhow::Result<Option<i64>> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT id FROM sessions
                 WHERE type = ?1 AND ended_at IS NULL
                 ORDER BY last_active_at DESC, id DESC
                 LIMIT 1",
                [kind],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The most recently active open (`ended_at IS NULL`) session of `kind`,
    /// or a fresh row when none exists.
    pub fn session_get_or_create_active(&self, kind: &str) -> anyhow::Result<i64> {
        if let Some(id) = self.session_active_id(kind)? {
            return Ok(id);
        }
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO sessions (type, title, started_at, ended_at, last_active_at)
             VALUES (?1, NULL, ?2, NULL, ?2)",
            params![kind, now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The ask session bound to a listen doc — one chat per doc. The
    /// most recent linked row is reopened (`session_reopen` ends other
    /// open asks and clears `ended_at`, so returning to a doc resumes
    /// its thread); a fresh linked row is minted when none exists.
    pub fn ask_session_for_listen(&self, listen_id: i64) -> anyhow::Result<i64> {
        let existing: Option<i64> = self
            .conn
            .lock()
            .query_row(
                "SELECT id FROM sessions
                 WHERE type = 'ask' AND listen_id = ?1
                 ORDER BY last_active_at DESC, id DESC LIMIT 1",
                [listen_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            self.session_reopen(id, "ask")?;
            return Ok(id);
        }
        self.session_end_open("ask")?;
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO sessions (type, title, listen_id, started_at, ended_at, last_active_at)
             VALUES ('ask', NULL, ?1, ?2, NULL, ?2)",
            params![listen_id, now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The session's `listen_id` link — `None` for plain sessions and
    /// rows written before the column existed.
    pub fn session_listen_id(&self, id: i64) -> anyhow::Result<Option<i64>> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT listen_id FROM sessions WHERE id = ?1",
                [id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Bump `last_active_at` to now — keeps the session on top of the list.
    #[allow(dead_code)] // Phase 2 session lifecycle
    pub fn session_touch(&self, id: i64) -> anyhow::Result<()> {
        self.conn.lock().execute(
            "UPDATE sessions SET last_active_at = ?1 WHERE id = ?2",
            params![now(), id],
        )?;
        Ok(())
    }

    /// Mark the session ended (`ended_at = now`); a later
    /// `session_get_or_create_active` for the same kind starts a new one.
    /// Open rows only — a second end is a no-op, so the first `ended_at`
    /// (e.g. the error path's, before `stop()`) wins.
    pub fn session_end(&self, id: i64) -> anyhow::Result<()> {
        self.conn.lock().execute(
            "UPDATE sessions SET ended_at = ?1 WHERE id = ?2 AND ended_at IS NULL",
            params![now(), id],
        )?;
        Ok(())
    }

    /// End every still-open session of `kind` — returns how many rows were
    /// closed. Start paths that must mint a fresh session sweep with this
    /// first, since `session_get_or_create_active` would otherwise resume
    /// a row left open by a killed run or a failed start.
    pub fn session_end_open(&self, kind: &str) -> anyhow::Result<usize> {
        Ok(self.conn.lock().execute(
            "UPDATE sessions SET ended_at = ?1 WHERE type = ?2 AND ended_at IS NULL",
            params![now(), kind],
        )?)
    }

    /// Reopen `id` when it is a `kind` session: every OTHER open `kind`
    /// session ends and the target's `ended_at` clears, atomically. Reopening
    /// does not bump activity; a subsequent message owns that timestamp.
    /// Returns false (no mutation) when `id` isn't a `kind` session.
    pub fn session_reopen(&self, id: i64, kind: &str) -> anyhow::Result<bool> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let matches: bool = tx
            .query_row(
                "SELECT type = ?2 FROM sessions WHERE id = ?1",
                params![id, kind],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false);
        if !matches {
            return Ok(false);
        }
        tx.execute(
            "UPDATE sessions SET ended_at = ?1 WHERE type = ?2 AND ended_at IS NULL AND id != ?3",
            params![now(), kind, id],
        )?;
        tx.execute("UPDATE sessions SET ended_at = NULL WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Session start time — the elapsed-timer epoch for Listen.
    pub fn session_started_at(&self, id: i64) -> anyhow::Result<Option<i64>> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT started_at FROM sessions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Every session, most recently active first. `title` is
    /// COALESCE(stored title, first ask user message, latest listen summary
    /// topic) — the fallback covers untitled rows: pre-title writes and
    /// ask sessions whose generated name hasn't landed yet.
    pub fn session_list(&self) -> anyhow::Result<Vec<Session>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.type,
                    COALESCE(s.title,
                      CASE s.type
                        WHEN 'ask' THEN (
                          SELECT substr(m.content, 1, 60) FROM messages m
                          WHERE m.session_id = s.id AND m.role = 'user'
                          ORDER BY m.ts ASC, m.id ASC LIMIT 1)
                        WHEN 'listen' THEN (
                          SELECT sm.topic FROM summaries sm
                          WHERE sm.session_id = s.id AND sm.topic IS NOT NULL
                          ORDER BY sm.updated_at DESC, sm.id DESC LIMIT 1)
                      END),
                    s.audio_file, s.stt, s.started_at, s.ended_at, s.last_active_at
             FROM sessions s ORDER BY s.last_active_at DESC, s.id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                kind: row.get(1)?,
                title: row.get(2)?,
                audio_file: row.get(3)?,
                stt: row.get(4)?,
                started_at: row.get(5)?,
                ended_at: row.get(6)?,
                last_active_at: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Record the session's retained recording path (written once the WAV
    /// exists, so a crashed run still points at the partial file).
    pub fn session_set_audio_file(&self, id: i64, path: &str) -> anyhow::Result<()> {
        self.conn.lock().execute(
            "UPDATE sessions SET audio_file = ?1 WHERE id = ?2",
            params![path, id],
        )?;
        Ok(())
    }

    /// The retained recording path for a session, or `None` when the session
    /// does not exist or has no recording.
    pub fn session_audio_file(&self, id: i64) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT audio_file FROM sessions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Record the STT engine label at listen start — the finished doc's
    /// header reads it back via `session_list`; pre-column rows show none.
    pub fn session_set_stt(&self, id: i64, stt: &str) -> anyhow::Result<()> {
        self.conn.lock().execute(
            "UPDATE sessions SET stt = ?1 WHERE id = ?2",
            params![stt, id],
        )?;
        Ok(())
    }

    /// The stored title — `None` while the session's only label is the
    /// `session_list` read-time fallback (ask: first user message).
    /// Also returns `None` when `id` does not exist. Database query and
    /// value-conversion errors propagate to the caller.
    pub fn session_title(&self, id: i64) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .lock()
            .query_row("SELECT title FROM sessions WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?
            .flatten())
    }

    /// First-write-wins title setter — the ask pipeline's generated name
    /// lands only on a still-NULL title, so a title that already landed
    /// (or a future rename) is never clobbered. Returns whether the
    /// write happened; `false` also covers a missing session. Stores
    /// `title` verbatim, including an empty string, without updating activity.
    /// Database update errors propagate to the caller.
    pub fn session_set_title(&self, id: i64, title: &str) -> anyhow::Result<bool> {
        Ok(self.conn.lock().execute(
            "UPDATE sessions SET title = ?2 WHERE id = ?1 AND title IS NULL",
            params![id, title],
        )? > 0)
    }

    /// Delete a session; its child records cascade away via their FKs and
    /// the retained recording (if any) is unlinked best-effort.
    pub fn session_delete(&self, id: i64) -> anyhow::Result<()> {
        let audio_file = {
            let conn = self.conn.lock();
            let audio_file: Option<String> = conn
                .query_row(
                    "SELECT audio_file FROM sessions WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            conn.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
            audio_file
        };
        if let Some(path) = audio_file {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    /// Append a message to a session; returns the new row id.
    #[allow(dead_code)] // test convenience — prod writes carry `MessageMeta`
    pub fn message_add(&self, session_id: i64, role: &str, content: &str) -> anyhow::Result<i64> {
        self.message_add_meta(session_id, role, content, &MessageMeta::default())
    }

    /// `message_add` + the assistant-row provenance/spend columns.
    /// Returns the inserted row id and updates the session's last-active
    /// time without setting its title. Database errors propagate, including
    /// a foreign-key error for a missing session; if the activity update
    /// fails, the inserted message remains stored.
    pub fn message_add_meta(
        &self,
        session_id: i64,
        role: &str,
        content: &str,
        meta: &MessageMeta,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.lock();
        let ts = now();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, provider, model, tokens_in, tokens_out, preset, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                session_id,
                role,
                content,
                meta.provider,
                meta.model,
                meta.tokens_in,
                meta.tokens_out,
                meta.preset,
                ts
            ],
        )?;
        // Activity bumps the session to the top of `session_list` — done
        // here rather than via `session_touch` so writers can't forget it.
        conn.execute(
            "UPDATE sessions SET last_active_at = ?1 WHERE id = ?2",
            params![ts, session_id],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Remove one message row — `ask_retry` drops the rejected reply
    /// this way before streaming the replacement.
    pub fn message_delete(&self, id: i64) -> anyhow::Result<()> {
        self.conn
            .lock()
            .execute("DELETE FROM messages WHERE id = ?1", [id])?;
        Ok(())
    }

    /// A session's messages, oldest first; `id` breaks same-second ties.
    pub fn messages_for(&self, session_id: i64) -> anyhow::Result<Vec<Message>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, provider, model, tokens_in, tokens_out, preset, ts
             FROM messages
             WHERE session_id = ?1 ORDER BY ts ASC, id ASC",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            Ok(Message {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                provider: row.get(4)?,
                model: row.get(5)?,
                tokens_in: row.get(6)?,
                tokens_out: row.get(7)?,
                preset: row.get(8)?,
                ts: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Append a finalized listen turn; returns the new row id.
    pub fn transcript_add(
        &self,
        session_id: i64,
        speaker: &str,
        content: &str,
        speaker_idx: Option<u32>,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.lock();
        let ts = now();
        conn.execute(
            "INSERT INTO transcripts (session_id, speaker, speaker_idx, content, ts)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, speaker, speaker_idx.map(i64::from), content, ts],
        )?;
        conn.execute(
            "UPDATE sessions SET last_active_at = ?1 WHERE id = ?2",
            params![ts, session_id],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// A session's listen turns, oldest first; `id` breaks same-second ties.
    pub fn transcripts_for(
        &self,
        session_id: i64,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<Transcript>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, speaker, speaker_idx, content, ts FROM transcripts
             WHERE session_id = ?1 ORDER BY ts ASC, id ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![session_id, limit.map(|value| value as i64).unwrap_or(-1)],
            |row| {
                Ok(Transcript {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    speaker: row.get(2)?,
                    speaker_idx: row.get(3)?,
                    content: row.get(4)?,
                    ts: row.get(5)?,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The newest `limit` listen turns, returned in the same oldest-first order
    /// as [`transcripts_for`].
    pub fn transcripts_tail(
        &self,
        session_id: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<Transcript>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, speaker, speaker_idx, content, ts FROM transcripts
             WHERE session_id = ?1 ORDER BY ts DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], |row| {
            Ok(Transcript {
                id: row.get(0)?,
                session_id: row.get(1)?,
                speaker: row.get(2)?,
                speaker_idx: row.get(3)?,
                content: row.get(4)?,
                ts: row.get(5)?,
            })
        })?;
        let mut rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.reverse();
        Ok(rows)
    }

    /// Persist a structured listen summary — one row per session: the
    /// first write inserts (`created_at`/`updated_at` stamped), later
    /// writes update in place so the live summary evolves instead of
    /// accreting versions. A `topic` also refreshes `sessions.title`
    /// (the newest topic is the best label). Returns the row id.
    pub fn summary_upsert(
        &self,
        session_id: i64,
        tldr: &str,
        bullets: &[String],
        follow_ups: &[String],
        topic: Option<&str>,
    ) -> anyhow::Result<i64> {
        let bullets = serde_json::to_string(bullets)?;
        let follow_ups = serde_json::to_string(follow_ups)?;
        let conn = self.conn.lock();
        let ts = now();
        conn.execute(
            "INSERT INTO summaries (session_id, tldr, bullets, follow_ups, topic, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT (session_id) DO UPDATE SET
                tldr = excluded.tldr,
                bullets = excluded.bullets,
                follow_ups = excluded.follow_ups,
                topic = excluded.topic,
                updated_at = excluded.updated_at",
            params![session_id, tldr, bullets, follow_ups, topic, ts],
        )?;
        if let Some(topic) = topic {
            conn.execute(
                "UPDATE sessions SET title = ?1 WHERE id = ?2",
                params![topic, session_id],
            )?;
        }
        conn.execute(
            "UPDATE sessions SET last_active_at = ?1 WHERE id = ?2",
            params![ts, session_id],
        )?;
        Ok(conn.query_row(
            "SELECT id FROM summaries WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// The session's summary, if one exists (unique per session).
    pub fn summary_latest(&self, session_id: i64) -> anyhow::Result<Option<Summary>> {
        let conn = self.conn.lock();
        let row: Option<SummaryRow> = conn
            .query_row(
                "SELECT id, session_id, tldr, bullets, follow_ups, topic, created_at, updated_at
                 FROM summaries WHERE session_id = ?1
                 ORDER BY updated_at DESC, id DESC LIMIT 1",
                [session_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .optional()?;

        row.map(
            |(id, session_id, tldr, bullets, follow_ups, topic, created_at, updated_at)| {
                Ok(Summary {
                    id,
                    session_id,
                    tldr,
                    bullets: serde_json::from_str(&bullets)?,
                    follow_ups: serde_json::from_str(&follow_ups)?,
                    topic,
                    created_at,
                    updated_at,
                })
            },
        )
        .transpose()
    }
}

/// Upgrade databases created before the current schema. Runs before
/// `SCHEMA`: `CREATE TABLE IF NOT EXISTS` never re-creates an existing
/// table, so renames, added columns, and the summaries single-row
/// rebuild all have to happen here first or every query against the
/// new shape would fail.
fn migrate(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS migrations (name TEXT PRIMARY KEY)")?;
    let table_exists = |name: &str| -> rusqlite::Result<bool> {
        conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master
             WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
    };
    let columns = |name: &str| -> rusqlite::Result<Vec<String>> {
        conn.prepare(&format!("PRAGMA table_info({name})"))?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect()
    };
    if table_exists("ai_messages")? {
        if table_exists("messages")? {
            // A build with the new schema already ran once: `messages`
            // exists alongside the legacy table — merge rather than
            // rename, then drop the old one.
            conn.execute_batch(
                "INSERT INTO messages (session_id, role, content, ts)
                 SELECT session_id, role, content, ts FROM ai_messages;
                 DROP TABLE ai_messages;",
            )?;
        } else {
            conn.execute_batch("ALTER TABLE ai_messages RENAME TO messages")?;
        }
    }
    if table_exists("messages")? {
        let columns = columns("messages")?;
        if !columns.iter().any(|c| c == "preset") {
            // The armed prompt preset on a user turn (presets.rs) —
            // NULL on every row written before presets existed.
            conn.execute_batch("ALTER TABLE messages ADD COLUMN preset TEXT")?;
        }
    }
    if table_exists("transcripts")? {
        let columns = columns("transcripts")?;
        let has = |name: &str| columns.iter().any(|c| c == name);
        if has("text") && !has("content") {
            conn.execute_batch("ALTER TABLE transcripts RENAME COLUMN text TO content")?;
        }
        // Per-turn audio retention moved to sessions.audio_file — this
        // column was never populated.
        if has("audio_file") {
            conn.execute_batch("ALTER TABLE transcripts DROP COLUMN audio_file")?;
        }
        if !has("speaker_idx") {
            conn.execute_batch("ALTER TABLE transcripts ADD COLUMN speaker_idx INTEGER")?;
        }
    }
    if table_exists("sessions")? {
        let columns = columns("sessions")?;
        if !columns.iter().any(|c| c == "audio_file") {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN audio_file TEXT")?;
        }
        if !columns.iter().any(|c| c == "stt") {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN stt TEXT")?;
        }
        // The listen session an ask chat is bound to — plain INTEGER,
        // not a self-FK (a deleted doc leaves the chat readable).
        if !columns.iter().any(|c| c == "listen_id") {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN listen_id INTEGER")?;
        }
    }
    if table_exists("summaries")? {
        let columns = columns("summaries")?;
        let has = |name: &str| columns.iter().any(|c| c == name);
        // The multi-version layout (`ts`) became one row per session
        // (`created_at`/`updated_at`): rename, dedupe to the newest row,
        // then enforce uniqueness so the upsert has a conflict target.
        if has("ts") && !has("created_at") {
            conn.execute_batch("ALTER TABLE summaries RENAME COLUMN ts TO created_at")?;
        }
        if !has("updated_at") {
            conn.execute_batch(
                "ALTER TABLE summaries ADD COLUMN updated_at INTEGER;
                 UPDATE summaries SET updated_at = created_at WHERE updated_at IS NULL;",
            )?;
        }
        conn.execute_batch(
            "DELETE FROM summaries WHERE id NOT IN
               (SELECT MAX(id) FROM summaries GROUP BY session_id);",
        )?;
        // Fresh tables get uniqueness from the column's UNIQUE constraint
        // (an autoindex); migrated ones need an explicit index — either
        // gives the upsert its conflict target.
        let indexed: bool = conn
            .prepare("PRAGMA index_list(summaries)")?
            .query_map([], |row| row.get::<_, i64>(2))?
            .any(|unique| unique.is_ok_and(|u| u != 0));
        if !indexed {
            conn.execute_batch(
                "CREATE UNIQUE INDEX summaries_session_id ON summaries(session_id)",
            )?;
        }
        // Titles are now persisted — backfill listen sessions from the
        // surviving summary's topic.
        conn.execute_batch(
            "UPDATE sessions SET title = (
               SELECT sm.topic FROM summaries sm
               WHERE sm.session_id = sessions.id AND sm.topic IS NOT NULL
               ORDER BY sm.updated_at DESC, sm.id DESC LIMIT 1)
             WHERE sessions.type = 'listen' AND sessions.title IS NULL;",
        )?;
    }
    if table_exists("messages")? {
        let columns = columns("messages")?;
        // Assistant-row provenance + spend (the card's ⋯ menu) — added
        // after the fact, so pre-meta rows keep NULLs.
        for (col, ty) in [
            ("provider", "TEXT"),
            ("model", "TEXT"),
            ("tokens_in", "INTEGER"),
            ("tokens_out", "INTEGER"),
        ] {
            if !columns.iter().any(|c| c == col) {
                conn.execute_batch(&format!("ALTER TABLE messages ADD COLUMN {col} {ty}"))?;
            }
        }
        // Backfill legacy asks once. New NULL titles must survive reopen
        // so a failed generated-title request can be retried.
        let tx = conn.unchecked_transaction()?;
        let backfilled: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM migrations WHERE name = 'ask_title_backfill')",
            [],
            |row| row.get(0),
        )?;
        if !backfilled {
            tx.execute_batch(
                "UPDATE sessions SET title = (
                   SELECT substr(m.content, 1, 60) FROM messages m
                   WHERE m.session_id = sessions.id AND m.role = 'user'
                   ORDER BY m.ts ASC, m.id ASC LIMIT 1)
                 WHERE sessions.type = 'ask' AND sessions.title IS NULL;
                 INSERT INTO migrations (name) VALUES ('ask_title_backfill');",
            )?;
        }
        tx.commit()?;
    }
    Ok(())
}

/// Unix-epoch seconds now.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Unique temp dir per test; `Db::at` creates it (parent-dirs path).
    fn tmp_dir() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "marvis-storage-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn get_or_create_active_reuses_then_recreates() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();

        let ask1 = db.session_get_or_create_active("ask").unwrap();
        // Same kind, still open → the same session is reused, not duplicated.
        assert_eq!(db.session_get_or_create_active("ask").unwrap(), ask1);
        // A different kind always gets its own session.
        let listen1 = db.session_get_or_create_active("listen").unwrap();
        assert_ne!(listen1, ask1);
        // Ending a session makes the next call for that kind insert a new row…
        db.session_end(ask1).unwrap();
        let ask2 = db.session_get_or_create_active("ask").unwrap();
        assert_ne!(ask2, ask1);
        // …while the other kind's open session is untouched.
        assert_eq!(db.session_get_or_create_active("listen").unwrap(), listen1);
        // The ended session is marked, not deleted.
        let ended = db
            .session_list()
            .unwrap()
            .into_iter()
            .find(|s| s.id == ask1)
            .unwrap();
        assert!(ended.ended_at.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn message_roundtrip_is_ordered() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("ask").unwrap();

        db.message_add(sid, "user", "hello").unwrap();
        db.message_add(sid, "assistant", "hi there").unwrap();
        db.message_add(sid, "user", "follow-up").unwrap();

        let msgs = db.messages_for(sid).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "hi there");
        assert_eq!(msgs[2].role, "user");
        assert_eq!(msgs[2].content, "follow-up");
        // Same-second inserts must keep insertion order (ts ASC, id ASC).
        assert!(msgs[0].id < msgs[1].id && msgs[1].id < msgs[2].id);
        for m in &msgs {
            assert_eq!(m.session_id, sid);
            assert!(m.ts > 0);
        }
        // Messages are scoped to their session.
        let other = db.session_get_or_create_active("listen").unwrap();
        assert!(db.messages_for(other).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_audio_file_reads_recording_path() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("listen").unwrap();
        let wav = dir.join("recording_test.wav");
        std::fs::write(&wav, b"RIFF/WAVE test").unwrap();

        db.session_set_audio_file(sid, wav.to_str().unwrap())
            .unwrap();
        assert_eq!(
            db.session_audio_file(sid).unwrap(),
            Some(wav.to_string_lossy().into_owned())
        );
        assert_eq!(db.session_audio_file(i64::MAX).unwrap(), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_delete_cascades_messages() {
        // foreign_keys is a per-connection pragma — if it weren't applied,
        // this test would leave orphan rows instead of cascading.
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        {
            let conn = db.conn.lock();
            let fk: i64 = conn
                .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
                .unwrap();
            assert_eq!(fk, 1);
        }

        let sid = db.session_get_or_create_active("ask").unwrap();
        db.message_add(sid, "user", "one").unwrap();
        db.message_add(sid, "assistant", "two").unwrap();

        db.session_delete(sid).unwrap();
        assert!(db.messages_for(sid).unwrap().is_empty());
        assert!(db.session_list().unwrap().iter().all(|s| s.id != sid));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transcript_roundtrip_is_oldest_first_with_limit() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("listen").unwrap();

        db.transcript_add(sid, "me", "hello", None).unwrap();
        db.transcript_add(sid, "them", "hi there", None).unwrap();
        db.transcript_add(sid, "me", "follow-up", None).unwrap();

        let transcripts = db.transcripts_for(sid, None).unwrap();
        assert_eq!(transcripts.len(), 3);
        assert_eq!(transcripts[0].speaker, "me");
        assert_eq!(transcripts[0].content, "hello");
        assert_eq!(transcripts[1].speaker, "them");
        assert_eq!(transcripts[1].content, "hi there");
        assert_eq!(transcripts[2].content, "follow-up");
        assert!(transcripts[0].id < transcripts[1].id && transcripts[1].id < transcripts[2].id);
        assert!(transcripts.iter().all(|transcript| transcript.ts > 0));

        let limited = db.transcripts_for(sid, Some(2)).unwrap();
        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].content, "hello");
        assert_eq!(limited[1].content, "hi there");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transcript_tail_returns_latest_rows_oldest_first() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("listen").unwrap();
        for id in 0..25 {
            db.transcript_add(sid, "me", &format!("turn-{id}"), None)
                .unwrap();
        }

        let tail = db.transcripts_tail(sid, 20).unwrap();
        assert_eq!(tail.len(), 20);
        assert_eq!(tail.first().unwrap().content, "turn-5");
        assert_eq!(tail.last().unwrap().content, "turn-24");
        assert!(tail.windows(2).all(|rows| rows[0].id < rows[1].id));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summary_roundtrip_deserializes_json_fields() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("listen").unwrap();

        db.summary_upsert(
            sid,
            "A useful summary",
            &["First point".to_owned(), "Second point".to_owned()],
            &["Follow up".to_owned()],
            Some("Planning"),
        )
        .unwrap();

        let summary = db.summary_latest(sid).unwrap().unwrap();
        assert_eq!(summary.session_id, sid);
        assert_eq!(summary.tldr, "A useful summary");
        assert_eq!(summary.bullets, vec!["First point", "Second point"]);
        assert_eq!(summary.follow_ups, vec!["Follow up"]);
        assert_eq!(summary.topic.as_deref(), Some("Planning"));
        assert!(summary.created_at > 0);
        assert!(summary.updated_at >= summary.created_at);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summary_malformed_json_returns_error() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("listen").unwrap();
        {
            let conn = db.conn.lock();
            conn.execute(
                "INSERT INTO summaries (session_id, tldr, bullets, follow_ups, topic, created_at, updated_at)
                 VALUES (?1, 'Summary', 'not-json', '[]', NULL, 1, 1)",
                [sid],
            )
            .unwrap();
        }

        assert!(db.summary_latest(sid).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn listen_session_delete_cascades_transcripts_and_summaries() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("listen").unwrap();
        db.transcript_add(sid, "me", "hello", None).unwrap();
        db.summary_upsert(sid, "Summary", &[], &[], None).unwrap();

        db.session_delete(sid).unwrap();
        assert!(db.transcripts_for(sid, None).unwrap().is_empty());
        assert!(db.summary_latest(sid).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_list_orders_by_last_active_desc() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let s1 = db.session_get_or_create_active("ask").unwrap();
        let s2 = db.session_get_or_create_active("listen").unwrap();
        // Timestamps are second-resolution and can tie, so pin
        // last_active_at directly to test the ORDER BY, not the clock.
        {
            let conn = db.conn.lock();
            conn.execute(
                "UPDATE sessions SET last_active_at = 100 WHERE id = ?1",
                [s1],
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions SET last_active_at = 200 WHERE id = ?1",
                [s2],
            )
            .unwrap();
        }

        let list = db.session_list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, s2);
        assert_eq!(list[1].id, s1);
        assert_eq!(list[0].kind, "listen");
        assert_eq!(list[1].kind, "ask");
        assert_eq!(list[0].last_active_at, 200);
        assert!(list[0].title.is_none());
        assert!(list[0].ended_at.is_none());
        assert!(list[0].started_at > 0);

        // session_touch bumps last_active_at to now, moving s1 to the top.
        db.session_touch(s1).unwrap();
        let list = db.session_list().unwrap();
        assert_eq!(list[0].id, s1);
        assert!(list[0].last_active_at > 200);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn db_uses_wal_journal_mode() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let mode: String = db
            .conn
            .lock()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn db_file_has_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp_dir();
        let path = dir.join("marvis.db");
        let _db = Db::at(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_active_id_reads_without_creating() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        // None before any session — and must not create one.
        assert_eq!(db.session_active_id("ask").unwrap(), None);
        assert!(db.session_list().unwrap().is_empty());
        let sid = db.session_get_or_create_active("ask").unwrap();
        assert_eq!(db.session_active_id("ask").unwrap(), Some(sid));
        db.session_end(sid).unwrap();
        assert_eq!(db.session_active_id("ask").unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One chat per listen doc: mint on first use, end the open ask,
    /// then REOPEN the linked row on a later send — ending whatever ask
    /// took its place in between.
    #[test]
    fn ask_session_for_listen_mints_then_resumes_per_doc() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let doc_a = db.session_get_or_create_active("listen").unwrap();
        db.session_end(doc_a).unwrap();
        let doc_b = db.session_get_or_create_active("listen").unwrap();
        db.session_end(doc_b).unwrap();

        // First send about doc A: ends the open generic ask, mints a
        // linked row.
        let generic = db.session_get_or_create_active("ask").unwrap();
        let chat_a = db.ask_session_for_listen(doc_a).unwrap();
        assert_ne!(chat_a, generic);
        assert_eq!(db.session_listen_id(chat_a).unwrap(), Some(doc_a));
        assert_eq!(db.session_listen_id(generic).unwrap(), None);
        assert_eq!(db.session_active_id("ask").unwrap(), Some(chat_a));

        // Chatting about doc B ends A's thread and mints its own.
        let chat_b = db.ask_session_for_listen(doc_b).unwrap();
        assert_ne!(chat_b, chat_a);
        assert_eq!(db.session_active_id("ask").unwrap(), Some(chat_b));

        // Back to A: the ENDED linked row is resumed, not duplicated —
        // B's open row ends in the swap.
        assert_eq!(db.ask_session_for_listen(doc_a).unwrap(), chat_a);
        assert_eq!(db.session_active_id("ask").unwrap(), Some(chat_a));
        assert_eq!(
            db.session_list()
                .unwrap()
                .iter()
                .filter(|s| s.kind == "ask")
                .count(),
            3 // generic + chat_a + chat_b — B closed, A reopened
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The schema used before the `ai_messages`/`transcripts.text`
    /// renames — kept verbatim so the migration test builds a real v1
    /// file instead of the current `SCHEMA`.
    const V1_SCHEMA: &str = "
        CREATE TABLE sessions (
            id             INTEGER PRIMARY KEY,
            type           TEXT NOT NULL,
            title          TEXT,
            started_at     INTEGER NOT NULL,
            ended_at       INTEGER,
            last_active_at INTEGER NOT NULL
        );
        CREATE TABLE ai_messages (
            id         INTEGER PRIMARY KEY,
            session_id INTEGER NOT NULL,
            role       TEXT NOT NULL,
            content    TEXT NOT NULL,
            ts         INTEGER NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );
        CREATE TABLE transcripts (
            id         INTEGER PRIMARY KEY,
            session_id INTEGER NOT NULL,
            speaker    TEXT NOT NULL,
            text       TEXT NOT NULL,
            ts         INTEGER NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );
    ";

    #[test]
    fn migrate_upgrades_pre_rename_database() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("marvis.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(V1_SCHEMA).unwrap();
            conn.execute_batch(
                "INSERT INTO sessions (type, started_at, last_active_at)
                 VALUES ('ask', 1, 1);
                 INSERT INTO ai_messages (session_id, role, content, ts)
                 VALUES (1, 'user', 'old question', 1);
                 INSERT INTO transcripts (session_id, speaker, text, ts)
                 VALUES (1, 'me', 'old turn', 1);",
            )
            .unwrap();
        }
        let db = Db::at(&path).unwrap();
        // The legacy table is gone, its rows live in `messages` — and the
        // ask session's title backfilled from the first user message.
        let messages = db.messages_for(1).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "old question");
        assert_eq!(
            db.session_list().unwrap()[0].title.as_deref(),
            Some("old question")
        );
        // `text` became `content`; the unused `audio_file` was dropped.
        let transcripts = db.transcripts_for(1, None).unwrap();
        assert_eq!(transcripts.len(), 1);
        assert_eq!(transcripts[0].content, "old turn");
        db.transcript_add(1, "them", "new turn", Some(1)).unwrap();
        let transcripts = db.transcripts_for(1, None).unwrap();
        assert_eq!(transcripts[1].speaker_idx, Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_title_backfill_runs_once_on_existing_databases() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("marvis.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            conn.execute_batch(
                "INSERT INTO sessions (type, started_at, last_active_at)
                 VALUES ('ask', 1, 1), ('ask', 2, 2);
                 INSERT INTO messages (session_id, role, content, ts)
                 VALUES (1, 'user', 'legacy question', 1);",
            )
            .unwrap();
        }
        {
            let db = Db::at(&path).unwrap();
            assert_eq!(
                db.session_title(1).unwrap().as_deref(),
                Some("legacy question")
            );
            // This session was empty during migration, then title generation failed.
            db.message_add(2, "user", "retry this title").unwrap();
        }
        {
            let db = Db::at(&path).unwrap();
            assert_eq!(
                db.session_title(1).unwrap().as_deref(),
                Some("legacy question")
            );
            assert_eq!(db.session_title(2).unwrap(), None);
            assert!(db.session_set_title(2, "Retried title").unwrap());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fresh_database_skips_ask_title_backfill_on_restart() {
        let dir = tmp_dir();
        let path = dir.join("marvis.db");
        let sid;
        {
            let db = Db::at(&path).unwrap();
            sid = db.session_get_or_create_active("ask").unwrap();
            db.message_add(sid, "user", "retry after restart").unwrap();
        }
        {
            let db = Db::at(&path).unwrap();
            assert_eq!(db.session_title(sid).unwrap(), None);
            assert_eq!(
                db.session_list().unwrap()[0].title.as_deref(),
                Some("retry after restart")
            );
            assert!(db.session_set_title(sid, "Retried title").unwrap());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_merges_when_messages_table_already_exists() {
        // A build with the new schema ran before the migration shipped:
        // `messages` exists (possibly holding new rows) next to the
        // legacy `ai_messages` — the old rows must be copied in, not
        // dropped or blocked by the rename.
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("marvis.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(V1_SCHEMA).unwrap();
            conn.execute_batch(
                "INSERT INTO sessions (type, started_at, last_active_at)
                 VALUES ('ask', 1, 1);
                 INSERT INTO ai_messages (session_id, role, content, ts)
                 VALUES (1, 'user', 'old question', 1);
                 CREATE TABLE messages (
                    id         INTEGER PRIMARY KEY,
                    session_id INTEGER NOT NULL,
                    role       TEXT NOT NULL,
                    content    TEXT NOT NULL,
                    ts         INTEGER NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                 );
                 INSERT INTO messages (session_id, role, content, ts)
                 VALUES (1, 'assistant', 'new answer', 2);",
            )
            .unwrap();
        }
        let db = Db::at(&path).unwrap();
        let messages = db.messages_for(1).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "old question");
        assert_eq!(messages[1].content, "new answer");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_list_computes_titles() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let ask = db.session_get_or_create_active("ask").unwrap();
        db.message_add(ask, "user", "how do I fix the capsule width")
            .unwrap();
        let listen = db.session_get_or_create_active("listen").unwrap();
        db.summary_upsert(
            listen,
            "tldr",
            &["b".to_string()],
            &[],
            Some("release review"),
        )
        .unwrap();
        let bare = db.session_get_or_create_active("ask").unwrap_or_else(|_| {
            db.session_end(ask).unwrap();
            db.session_get_or_create_active("ask").unwrap()
        });
        let list = db.session_list().unwrap();
        let a = list.iter().find(|s| s.id == ask).unwrap();
        assert_eq!(a.title.as_deref(), Some("how do I fix the capsule width"));
        let l = list.iter().find(|s| s.id == listen).unwrap();
        assert_eq!(l.title.as_deref(), Some("release review"));
        let _ = bare; // an empty session keeps title NULL — UI falls back
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_reopen_swaps_the_active_ask_session() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let a = db.session_get_or_create_active("ask").unwrap();
        let b = {
            db.session_end(a).unwrap();
            db.session_get_or_create_active("ask").unwrap()
        };
        {
            let conn = db.conn.lock();
            conn.execute(
                "UPDATE sessions SET last_active_at = 100 WHERE id = ?1",
                [a],
            )
            .unwrap();
        }
        // Reopening a listen id is rejected and ends nothing.
        let l = db.session_get_or_create_active("listen").unwrap();
        assert!(!db.session_reopen(l, "ask").unwrap());
        assert_eq!(db.session_active_id("ask").unwrap(), Some(b));
        // Real resume: b ends, a reopens and becomes the active ask session.
        assert!(db.session_reopen(a, "ask").unwrap());
        assert_eq!(db.session_active_id("ask").unwrap(), Some(a));
        assert_eq!(db.session_active_id("listen").unwrap(), Some(l));
        let reopened = db
            .session_list()
            .unwrap()
            .into_iter()
            .find(|session| session.id == a)
            .unwrap();
        assert_eq!(reopened.last_active_at, 100);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The live summary is one evolving row, not a version history:
    /// repeated upserts update in place and the newest topic wins the
    /// session title.
    #[test]
    fn summary_upsert_keeps_one_row_and_tracks_title() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("listen").unwrap();

        db.summary_upsert(sid, "v1", &["a".to_string()], &[], Some("early"))
            .unwrap();
        let first = db.summary_latest(sid).unwrap().unwrap();
        db.summary_upsert(sid, "v2", &["b".to_string()], &[], Some("final"))
            .unwrap();
        let second = db.summary_latest(sid).unwrap().unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.tldr, "v2");
        assert_eq!(second.bullets, vec!["b"]);
        assert_eq!(second.created_at, first.created_at);
        assert!(second.updated_at >= first.updated_at);
        {
            let conn = db.conn.lock();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM summaries WHERE session_id = ?1",
                    [sid],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }
        let session = db
            .session_list()
            .unwrap()
            .into_iter()
            .find(|s| s.id == sid)
            .unwrap();
        assert_eq!(session.title.as_deref(), Some("final"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Messages no longer write titles — an ask session's stored title
    /// stays NULL until the pipeline's generated name lands, and
    /// `session_list` falls back to the first user message meanwhile.
    /// `session_set_title` is first-write-wins.
    #[test]
    fn ask_title_falls_back_until_a_generated_one_lands() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("ask").unwrap();

        db.message_add(sid, "assistant", "hi").unwrap();
        db.message_add(sid, "user", "first question").unwrap();
        db.message_add(sid, "user", "second question").unwrap();

        assert_eq!(db.session_title(sid).unwrap(), None);
        let session = db
            .session_list()
            .unwrap()
            .into_iter()
            .find(|s| s.id == sid)
            .unwrap();
        assert_eq!(session.title.as_deref(), Some("first question"));

        // The generated title lands once — later writes don't clobber.
        assert!(db.session_set_title(sid, "Capsule fix").unwrap());
        assert!(!db.session_set_title(sid, "later write").unwrap());
        assert_eq!(
            db.session_title(sid).unwrap().as_deref(),
            Some("Capsule fix")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `session_set_audio_file` links the recording; `session_delete`
    /// unlinks the file with the row.
    #[test]
    fn session_audio_file_is_stored_and_removed_on_delete() {
        let dir = tmp_dir();
        let wav = dir.join("recording_1.wav");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&wav, b"RIFF").unwrap();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("listen").unwrap();

        db.session_set_audio_file(sid, wav.to_str().unwrap())
            .unwrap();
        let session = db
            .session_list()
            .unwrap()
            .into_iter()
            .find(|s| s.id == sid)
            .unwrap();
        assert_eq!(session.audio_file.as_deref(), wav.to_str());

        db.session_delete(sid).unwrap();
        assert!(!wav.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A migrated multi-version summaries table collapses to one row per
    /// session — the newest — and the upsert conflict target exists.
    #[test]
    fn migrate_dedupes_summaries_to_the_newest_row() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("marvis.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE sessions (
                    id INTEGER PRIMARY KEY, type TEXT NOT NULL, title TEXT,
                    started_at INTEGER NOT NULL, ended_at INTEGER,
                    last_active_at INTEGER NOT NULL);
                 CREATE TABLE summaries (
                    id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL,
                    tldr TEXT NOT NULL, bullets TEXT NOT NULL,
                    follow_ups TEXT NOT NULL, topic TEXT, ts INTEGER NOT NULL);
                 INSERT INTO sessions (id, type, started_at, last_active_at)
                    VALUES (1, 'listen', 1, 1);
                 INSERT INTO summaries (session_id, tldr, bullets, follow_ups, topic, ts)
                    VALUES (1, 'old', '[]', '[]', 'early topic', 10),
                           (1, 'new', '[]', '[]', 'final topic', 20);",
            )
            .unwrap();
        }
        let db = Db::at(&path).unwrap();
        let summary = db.summary_latest(1).unwrap().unwrap();
        assert_eq!(summary.tldr, "new");
        assert_eq!(summary.created_at, 20);
        assert_eq!(summary.updated_at, 20);
        // The write path is now an upsert — no second row appears.
        db.summary_upsert(1, "v3", &[], &[], Some("newest"))
            .unwrap();
        assert_eq!(db.summary_latest(1).unwrap().unwrap().tldr, "v3");
        // The surviving topic backfilled the session title.
        assert_eq!(
            db.session_list().unwrap()[0].title.as_deref(),
            Some("newest")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_started_at_reads_the_start_epoch() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("ask").unwrap();
        let expected = db
            .session_list()
            .unwrap()
            .into_iter()
            .find(|s| s.id == sid)
            .unwrap()
            .started_at;
        assert_eq!(db.session_started_at(sid).unwrap(), Some(expected));
        // Unknown ids read as None instead of erroring.
        assert_eq!(db.session_started_at(sid + 1000).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn message_preset_roundtrips_and_migrates() {
        // Fresh schema: the column writes and reads.
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("ask").unwrap();
        db.message_add_meta(
            sid,
            "user",
            "hi",
            &MessageMeta {
                preset: Some("b:concise".to_string()),
                ..MessageMeta::default()
            },
        )
        .unwrap();
        db.message_add(sid, "user", "plain").unwrap();
        let rows = db.messages_for(sid).unwrap();
        assert_eq!(rows[0].preset.as_deref(), Some("b:concise"));
        assert_eq!(rows[1].preset, None);

        // A pre-column database gains `preset` via migrate().
        let dir2 = tmp_dir();
        std::fs::create_dir_all(&dir2).unwrap();
        let path = dir2.join("marvis.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(V1_SCHEMA).unwrap();
            conn.execute_batch(
                "INSERT INTO sessions (type, started_at, last_active_at)
                 VALUES ('ask', 1, 1);
                 CREATE TABLE messages (
                    id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL,
                    role TEXT NOT NULL, content TEXT NOT NULL, ts INTEGER NOT NULL);
                 INSERT INTO messages (session_id, role, content, ts)
                 VALUES (1, 'user', 'old', 1);",
            )
            .unwrap();
        }
        let db = Db::at(&path).unwrap();
        assert_eq!(db.messages_for(1).unwrap()[0].preset, None);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }
}
