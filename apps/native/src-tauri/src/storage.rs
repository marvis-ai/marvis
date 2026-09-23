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
//! sessions(id PK, type 'ask'|'listen', title?, started_at, ended_at?, last_active_at)
//! ai_messages(id PK, session_id FK → sessions.id ON DELETE CASCADE, role, content, ts)
//! transcripts(id PK, session_id FK → sessions.id ON DELETE CASCADE, speaker, text, ts)
//! summaries(id PK, session_id FK → sessions.id ON DELETE CASCADE, tldr, bullets, follow_ups, topic?, ts)
//! ```
//!
//! All timestamps are unix-epoch seconds (`i64`).

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
        started_at     INTEGER NOT NULL,
        ended_at       INTEGER,
        last_active_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS ai_messages (
        id         INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL,
        role       TEXT NOT NULL,
        content    TEXT NOT NULL,
        ts         INTEGER NOT NULL,
        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS transcripts (
        id         INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL,
        speaker    TEXT NOT NULL,
        text       TEXT NOT NULL,
        ts         INTEGER NOT NULL,
        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS summaries (
        id         INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL,
        tldr       TEXT NOT NULL,
        bullets    TEXT NOT NULL,
        follow_ups TEXT NOT NULL,
        topic      TEXT,
        ts         INTEGER NOT NULL,
        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
    );
";

/// A row of `sessions`. `kind` maps to the `type` column (`type` is a Rust
/// keyword); `title`/`ended_at` are `NULL` until set. `Serialize` so the
/// `session_list` command can return rows to the webview.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Session {
    pub id: i64,
    pub kind: String,
    pub title: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub last_active_at: i64,
}

/// A row of `ai_messages`. `Serialize` for the `session_get` command.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AiMessage {
    pub id: i64,
    pub session_id: i64,
    pub role: String,
    pub content: String,
    pub ts: i64,
}

/// A persisted speaker turn from a listen session.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Transcript {
    pub id: i64,
    pub session_id: i64,
    pub speaker: String,
    pub text: String,
    pub ts: i64,
}

/// A persisted structured summary from a listen session.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Summary {
    pub id: i64,
    pub session_id: i64,
    pub tldr: String,
    pub bullets: Vec<String>,
    pub follow_ups: Vec<String>,
    pub topic: Option<String>,
    pub ts: i64,
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
        conn.execute_batch(SCHEMA)?;
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
    pub fn session_end(&self, id: i64) -> anyhow::Result<()> {
        self.conn.lock().execute(
            "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
            params![now(), id],
        )?;
        Ok(())
    }

    /// Every session, most recently active first.
    pub fn session_list(&self) -> anyhow::Result<Vec<Session>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, type, title, started_at, ended_at, last_active_at
             FROM sessions ORDER BY last_active_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                kind: row.get(1)?,
                title: row.get(2)?,
                started_at: row.get(3)?,
                ended_at: row.get(4)?,
                last_active_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Delete a session; its child records cascade away via their FKs.
    pub fn session_delete(&self, id: i64) -> anyhow::Result<()> {
        self.conn
            .lock()
            .execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Append a message to a session; returns the new row id.
    pub fn ai_message_add(
        &self,
        session_id: i64,
        role: &str,
        content: &str,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.lock();
        let ts = now();
        conn.execute(
            "INSERT INTO ai_messages (session_id, role, content, ts)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, content, ts],
        )?;
        // Activity bumps the session to the top of `session_list` — done
        // here rather than via `session_touch` so writers can't forget it.
        conn.execute(
            "UPDATE sessions SET last_active_at = ?1 WHERE id = ?2",
            params![ts, session_id],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// A session's messages, oldest first; `id` breaks same-second ties.
    pub fn ai_messages_for(&self, session_id: i64) -> anyhow::Result<Vec<AiMessage>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, ts FROM ai_messages
             WHERE session_id = ?1 ORDER BY ts ASC, id ASC",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            Ok(AiMessage {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                ts: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Append a finalized listen turn; returns the new row id.
    pub fn transcript_add(
        &self,
        session_id: i64,
        speaker: &str,
        text: &str,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.lock();
        let ts = now();
        conn.execute(
            "INSERT INTO transcripts (session_id, speaker, text, ts)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, speaker, text, ts],
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
            "SELECT id, session_id, speaker, text, ts FROM transcripts
             WHERE session_id = ?1 ORDER BY ts ASC, id ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![session_id, limit.map(|value| value as i64).unwrap_or(-1)],
            |row| {
                Ok(Transcript {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    speaker: row.get(2)?,
                    text: row.get(3)?,
                    ts: row.get(4)?,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Persist a structured listen summary; returns the new row id.
    pub fn summary_add(
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
            "INSERT INTO summaries (session_id, tldr, bullets, follow_ups, topic, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id, tldr, bullets, follow_ups, topic, ts],
        )?;
        conn.execute(
            "UPDATE sessions SET last_active_at = ?1 WHERE id = ?2",
            params![ts, session_id],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The newest summary for a session, if one exists.
    pub fn summary_latest(&self, session_id: i64) -> anyhow::Result<Option<Summary>> {
        let conn = self.conn.lock();
        let row: Option<(i64, i64, String, String, String, Option<String>, i64)> = conn
            .query_row(
                "SELECT id, session_id, tldr, bullets, follow_ups, topic, ts
                 FROM summaries WHERE session_id = ?1
                 ORDER BY ts DESC, id DESC LIMIT 1",
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
                    ))
                },
            )
            .optional()?;

        row.map(|(id, session_id, tldr, bullets, follow_ups, topic, ts)| {
            Ok(Summary {
                id,
                session_id,
                tldr,
                bullets: serde_json::from_str(&bullets)?,
                follow_ups: serde_json::from_str(&follow_ups)?,
                topic,
                ts,
            })
        })
        .transpose()
    }
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
    fn ai_message_roundtrip_is_ordered() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("ask").unwrap();

        db.ai_message_add(sid, "user", "hello").unwrap();
        db.ai_message_add(sid, "assistant", "hi there").unwrap();
        db.ai_message_add(sid, "user", "follow-up").unwrap();

        let msgs = db.ai_messages_for(sid).unwrap();
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
        assert!(db.ai_messages_for(other).unwrap().is_empty());

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
        db.ai_message_add(sid, "user", "one").unwrap();
        db.ai_message_add(sid, "assistant", "two").unwrap();

        db.session_delete(sid).unwrap();
        assert!(db.ai_messages_for(sid).unwrap().is_empty());
        assert!(db.session_list().unwrap().iter().all(|s| s.id != sid));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transcript_roundtrip_is_oldest_first_with_limit() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("listen").unwrap();

        db.transcript_add(sid, "me", "hello").unwrap();
        db.transcript_add(sid, "them", "hi there").unwrap();
        db.transcript_add(sid, "me", "follow-up").unwrap();

        let transcripts = db.transcripts_for(sid, None).unwrap();
        assert_eq!(transcripts.len(), 3);
        assert_eq!(transcripts[0].speaker, "me");
        assert_eq!(transcripts[0].text, "hello");
        assert_eq!(transcripts[1].speaker, "them");
        assert_eq!(transcripts[1].text, "hi there");
        assert_eq!(transcripts[2].text, "follow-up");
        assert!(transcripts[0].id < transcripts[1].id && transcripts[1].id < transcripts[2].id);
        assert!(transcripts.iter().all(|transcript| transcript.ts > 0));

        let limited = db.transcripts_for(sid, Some(2)).unwrap();
        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].text, "hello");
        assert_eq!(limited[1].text, "hi there");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summary_roundtrip_deserializes_json_fields() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("listen").unwrap();

        db.summary_add(
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
        assert!(summary.ts > 0);

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
                "INSERT INTO summaries (session_id, tldr, bullets, follow_ups, topic, ts)
                 VALUES (?1, 'Summary', 'not-json', '[]', NULL, 1)",
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
        db.transcript_add(sid, "me", "hello").unwrap();
        db.summary_add(sid, "Summary", &[], &[], None).unwrap();

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
}
