use super::*;
use super::types::*;

impl Db {
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
    /// this way before streaming the replacement. Its managed
    /// attachment files unlink best-effort after the cascade.
    pub fn message_delete(&self, id: i64) -> anyhow::Result<()> {
        let paths = {
            let conn = self.conn.lock();
            let mut stmt =
                conn.prepare("SELECT path FROM message_attachments WHERE message_id = ?1")?;
            let paths = stmt
                .query_map([id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            conn.execute("DELETE FROM messages WHERE id = ?1", [id])?;
            paths
        };
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    /// Persist attachment metadata for a message; returns the inserted
    /// rows (with ids) in `position` order — the `loading` payload and
    /// provider history both read that order back.
    pub fn attachments_add(
        &self,
        message_id: i64,
        rows: &[NewAttachment],
    ) -> anyhow::Result<Vec<MessageAttachment>> {
        let conn = self.conn.lock();
        for row in rows {
            conn.execute(
                "INSERT INTO message_attachments (message_id, name, path, mime, bytes, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    message_id,
                    row.name,
                    row.path,
                    row.mime,
                    row.bytes,
                    row.position
                ],
            )?;
        }
        let mut stmt = conn.prepare(
            "SELECT id, message_id, name, path, mime, bytes, position
             FROM message_attachments
             WHERE message_id = ?1 ORDER BY position ASC, id ASC",
        )?;
        let rows = stmt.query_map([message_id], |row| {
            Ok(MessageAttachment {
                id: row.get(0)?,
                message_id: row.get(1)?,
                name: row.get(2)?,
                path: row.get(3)?,
                mime: row.get(4)?,
                bytes: row.get(5)?,
                position: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// A message's attachments in `position` order.
    #[allow(dead_code)] // test convenience — prod reads them on `Message`
    pub fn attachments_for(&self, message_id: i64) -> anyhow::Result<Vec<MessageAttachment>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, message_id, name, path, mime, bytes, position
             FROM message_attachments
             WHERE message_id = ?1 ORDER BY position ASC, id ASC",
        )?;
        let rows = stmt.query_map([message_id], |row| {
            Ok(MessageAttachment {
                id: row.get(0)?,
                message_id: row.get(1)?,
                name: row.get(2)?,
                path: row.get(3)?,
                mime: row.get(4)?,
                bytes: row.get(5)?,
                position: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// A session's messages, oldest first; `id` breaks same-second ties.
    /// Each row carries its attachments (one follow-up query, grouped by
    /// `message_id` — the chat UI and provider history read them here).
    pub fn messages_for(&self, session_id: i64) -> anyhow::Result<Vec<Message>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, provider, model, tokens_in, tokens_out, preset, ts
             FROM messages
             WHERE session_id = ?1 ORDER BY ts ASC, id ASC",
        )?;
        let mut rows: Vec<Message> = stmt
            .query_map([session_id], |row| {
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
                    attachments: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            return Ok(rows);
        }
        let mut by_id: std::collections::HashMap<i64, &mut Message> =
            rows.iter_mut().map(|m| (m.id, m)).collect();
        let mut stmt = conn.prepare(
            "SELECT a.id, a.message_id, a.name, a.path, a.mime, a.bytes, a.position
             FROM message_attachments a
             JOIN messages m ON m.id = a.message_id
             WHERE m.session_id = ?1
             ORDER BY a.position ASC, a.id ASC",
        )?;
        let attachments = stmt.query_map([session_id], |row| {
            Ok(MessageAttachment {
                id: row.get(0)?,
                message_id: row.get(1)?,
                name: row.get(2)?,
                path: row.get(3)?,
                mime: row.get(4)?,
                bytes: row.get(5)?,
                position: row.get(6)?,
            })
        })?;
        for attachment in attachments.collect::<rusqlite::Result<Vec<_>>>()? {
            if let Some(msg) = by_id.get_mut(&attachment.message_id) {
                msg.attachments.push(attachment);
            }
        }
        Ok(rows)
    }

}
