use super::types::*;
use super::*;

impl Db {
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
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let existing = tx
            .query_row(
                "SELECT id FROM sessions WHERE type = ?1 AND ended_at IS NULL
             ORDER BY last_active_at DESC, id DESC LIMIT 1",
                [kind],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        tx.execute(
            "INSERT INTO sessions
                (type, title, session_token, started_at, ended_at, last_active_at)
             VALUES (?1, NULL, hex(randomblob(16)), ?2, NULL, ?2)",
            params![kind, now()],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// Reopen or create the single chat for a listen document atomically.
    pub fn ask_session_for_listen(&self, listen_id: i64) -> anyhow::Result<i64> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM sessions
                 WHERE type = 'ask' AND listen_id = ?1
                 ORDER BY last_active_at DESC, id DESC LIMIT 1",
                [listen_id],
                |row| row.get(0),
            )
            .optional()?;
        tx.execute(
            "UPDATE sessions SET ended_at = ?1 WHERE type = 'ask' AND ended_at IS NULL
             AND (?2 IS NULL OR id != ?2)",
            params![now(), existing],
        )?;
        let id = if let Some(id) = existing {
            tx.execute("UPDATE sessions SET ended_at = NULL WHERE id = ?1", [id])?;
            id
        } else {
            tx.execute(
                "INSERT INTO sessions
                    (type, title, listen_id, session_token, started_at, ended_at, last_active_at)
                 VALUES ('ask', NULL, ?1, hex(randomblob(16)), ?2, NULL, ?2)",
                params![listen_id, now()],
            )?;
            tx.last_insert_rowid()
        };
        tx.commit()?;
        Ok(id)
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
                    s.audio_file, s.stt, s.compact, s.compact_through,
                    s.started_at, s.ended_at, s.last_active_at
             FROM sessions s ORDER BY s.last_active_at DESC, s.id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                kind: row.get(1)?,
                title: row.get(2)?,
                audio_file: row.get(3)?,
                stt: row.get(4)?,
                compact: row.get(5)?,
                compact_through: row.get(6)?,
                started_at: row.get(7)?,
                ended_at: row.get(8)?,
                last_active_at: row.get(9)?,
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

    /// Read the session incarnation token, detached compaction digest, and
    /// message watermark. `None` means the owning session no longer exists;
    /// an existing session with an uninitialized digest returns
    /// `Some((token, None, None))`.
    pub fn session_compaction(
        &self,
        session_id: i64,
    ) -> anyhow::Result<Option<(String, Option<String>, Option<i64>)>> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT session_token, compact, compact_through
                 FROM sessions WHERE id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?)
    }

    /// Store a detached compaction digest only when the session incarnation
    /// token and watermark still match the caller's expected values. `NULL`
    /// is a valid initial watermark and is matched explicitly for SQLite's
    /// three-valued logic.
    pub fn session_compact_write(
        &self,
        session_id: i64,
        session_token: &str,
        expected_through: Option<i64>,
        compact: &str,
        new_through: i64,
    ) -> anyhow::Result<bool> {
        Ok(self.conn.lock().execute(
            "UPDATE sessions
             SET compact = ?1, compact_through = ?2
             WHERE id = ?3
               AND session_token = ?4
               AND ((compact_through IS NULL AND ?5 IS NULL)
                    OR compact_through = ?5)",
            params![
                compact,
                new_through,
                session_id,
                session_token,
                expected_through
            ],
        )? > 0)
    }

    /// Clear the detached compaction digest and watermark without changing
    /// messages or session activity. Missing sessions are a no-op.
    pub fn session_compact_clear(&self, session_id: i64) -> anyhow::Result<()> {
        self.conn.lock().execute(
            "UPDATE sessions SET compact = NULL, compact_through = NULL WHERE id = ?1",
            [session_id],
        )?;
        Ok(())
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
    /// the managed files (the retained recording, if any, and every
    /// attachment image) are unlinked best-effort. Attachment paths are
    /// collected BEFORE the cascade deletes their rows.
    pub fn session_delete(&self, id: i64) -> anyhow::Result<()> {
        let (audio_file, attachments) = {
            let conn = self.conn.lock();
            let audio_file: Option<String> = conn
                .query_row(
                    "SELECT audio_file FROM sessions WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            let mut stmt = conn.prepare(
                "SELECT a.path FROM message_attachments a
                 JOIN messages m ON m.id = a.message_id
                 WHERE m.session_id = ?1",
            )?;
            let attachments = stmt
                .query_map([id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            conn.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
            (audio_file, attachments)
        };
        if let Some(path) = audio_file {
            let _ = std::fs::remove_file(path);
        }
        for path in attachments {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }
}
