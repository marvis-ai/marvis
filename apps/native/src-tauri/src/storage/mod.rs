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
//! sessions(id PK, type 'ask'|'listen', title?, audio_file?, stt?, session_token, started_at, ended_at?, last_active_at)
//! messages(id PK, session_id FK → sessions.id ON DELETE CASCADE, role, content,
//!          provider?, model?, tokens_in?, tokens_out?, preset?, ts)
//! transcripts(id PK, session_id FK → sessions.id ON DELETE CASCADE, speaker, speaker_idx?, content, ts)
//! summaries(id PK, session_id FK → sessions.id ON DELETE CASCADE UNIQUE, tldr, bullets, follow_ups, topic?, created_at, updated_at)
//! memories(id PK, category, attribute, value, confidence, basis, source 'automatic'|'manual',
//!          source_session_id? FK → sessions.id ON DELETE SET NULL,
//!          source_message_id? FK → messages.id ON DELETE SET NULL, created_at, updated_at)
//! memory_history(id PK, memory_id, category, attribute, event 'add'|'update'|'delete',
//!                old_value?, new_value?, source, created_at)
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

mod memories;
mod messages;
mod migrate;
mod sessions;
mod transcripts;
mod types;

use self::migrate::*;
pub(crate) use types::*;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};

use crate::paths;

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
        session_token  TEXT,
        compact        TEXT,
        compact_through INTEGER,
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
        audio_start_ms INTEGER,
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

    CREATE TABLE IF NOT EXISTS message_attachments (
        id         INTEGER PRIMARY KEY,
        message_id INTEGER NOT NULL,
        name       TEXT NOT NULL,
        path       TEXT NOT NULL,
        mime       TEXT NOT NULL,
        bytes      INTEGER NOT NULL,
        position   INTEGER NOT NULL,
        FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS memories (
        id               INTEGER PRIMARY KEY,
        category         TEXT NOT NULL,
        attribute        TEXT NOT NULL,
        value            TEXT NOT NULL,
        confidence       REAL NOT NULL,
        basis            TEXT NOT NULL,
        source           TEXT NOT NULL,
        source_session_id INTEGER,
        source_message_id INTEGER,
        created_at       INTEGER NOT NULL,
        updated_at       INTEGER NOT NULL,
        FOREIGN KEY (source_session_id) REFERENCES sessions(id) ON DELETE SET NULL,
        FOREIGN KEY (source_message_id) REFERENCES messages(id) ON DELETE SET NULL
    );
    CREATE UNIQUE INDEX IF NOT EXISTS memories_key
        ON memories(category, attribute);
    CREATE INDEX IF NOT EXISTS memories_updated_at
        ON memories(updated_at DESC, id DESC);

    -- The audit trail: one row per fact write (add/update/delete).
    -- `memory_id` is deliberately NOT a foreign key — a deleted fact's
    -- history is exactly the record worth keeping, and `category`/
    -- `attribute` are denormalized so it still reads without the row.
    CREATE TABLE IF NOT EXISTS memory_history (
        id         INTEGER PRIMARY KEY,
        memory_id  INTEGER NOT NULL,
        category   TEXT NOT NULL,
        attribute  TEXT NOT NULL,
        event      TEXT NOT NULL,
        old_value  TEXT,
        new_value  TEXT,
        source     TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS memory_history_memory
        ON memory_history(memory_id, id);
";

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
}

/// Unix-epoch seconds now.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs() as i64
}
