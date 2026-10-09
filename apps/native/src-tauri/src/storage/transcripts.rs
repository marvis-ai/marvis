use super::*;
use super::types::*;

impl Db {
    /// Append a finalized listen turn; returns the new row id.
    pub fn transcript_add(
        &self,
        session_id: i64,
        speaker: &str,
        content: &str,
        speaker_idx: Option<u32>,
        audio_start_ms: Option<u64>,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.lock();
        let ts = now();
        conn.execute(
            "INSERT INTO transcripts (session_id, speaker, speaker_idx, content, ts, audio_start_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id, speaker, speaker_idx.map(i64::from), content, ts, audio_start_ms.map(i64::try_from).transpose()?],
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
            "SELECT id, session_id, speaker, speaker_idx, content, ts, audio_start_ms FROM transcripts
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
                    audio_start_ms: row.get(6)?,
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
            "SELECT id, session_id, speaker, speaker_idx, content, ts, audio_start_ms FROM transcripts
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
                audio_start_ms: row.get(6)?,
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


