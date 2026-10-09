use super::*;

/// Upgrade databases created before the current schema. Runs before
/// `SCHEMA`: `CREATE TABLE IF NOT EXISTS` never re-creates an existing
/// table, so renames, added columns, and the summaries single-row
/// rebuild all have to happen here first or every query against the
/// new shape would fail.
pub(super) fn migrate(conn: &Connection) -> anyhow::Result<()> {
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
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(
                "INSERT INTO messages (session_id, role, content, ts)
                 SELECT session_id, role, content, ts FROM ai_messages;
                 DROP TABLE ai_messages;",
            )?;
            tx.commit()?;
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
        if !has("audio_start_ms") {
            conn.execute_batch("ALTER TABLE transcripts ADD COLUMN audio_start_ms INTEGER")?;
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

