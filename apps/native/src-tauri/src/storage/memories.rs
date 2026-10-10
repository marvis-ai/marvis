//! The `memories` table — stored identity/preference facts extracted by
//! the consent-gated Memory LLM, plus the user's manual edits.
//!
//! Two writers with different authority:
//!
//! - `memory_apply` (automatic extraction) upserts by
//!   `(category, attribute)` under `source = 'automatic'` and never
//!   touches a `"manual"` row — a user edit always wins. Rows whose
//!   stored value already matches don't count as changes (no
//!   `updated_at` churn, no `memory:changed` event).
//! - `memory_update` (settings UI) rewrites the value and stamps the
//!   row `source = 'manual'`, `confidence = 1.0`, `basis = 'explicit'`,
//!   clearing the source ids — a user-typed fact isn't "from" a turn.
//!
//! `source_session_id`/`source_message_id` are pure provenance for the
//! UI; `ON DELETE SET NULL` clears them when ask history is wiped —
//! the fact itself survives history deletion (privacy = the user can
//! delete the fact too, through `memory_delete`).

use super::types::*;
use super::*;

fn read_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        category: row.get(1)?,
        attribute: row.get(2)?,
        value: row.get(3)?,
        confidence: row.get(4)?,
        basis: row.get(5)?,
        source: row.get(6)?,
        source_session_id: row.get(7)?,
        source_message_id: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

const MEMORY_SELECT: &str = "SELECT id, category, attribute, value, confidence, basis, source,
            source_session_id, source_message_id, created_at, updated_at
     FROM memories";

fn read_history(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryHistory> {
    Ok(MemoryHistory {
        id: row.get(0)?,
        memory_id: row.get(1)?,
        category: row.get(2)?,
        attribute: row.get(3)?,
        event: row.get(4)?,
        old_value: row.get(5)?,
        new_value: row.get(6)?,
        source: row.get(7)?,
        created_at: row.get(8)?,
    })
}

/// One audit row — every fact mutation funnels here. Runs inside the
/// caller's transaction (or under the same connection lock) so a
/// history write can't outlive a rolled-back change.
fn record_history(
    conn: &Connection,
    memory_id: i64,
    category: &str,
    attribute: &str,
    event: &str,
    old_value: Option<&str>,
    new_value: Option<&str>,
    source: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO memory_history
         (memory_id, category, attribute, event, old_value, new_value, source, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            memory_id,
            category,
            attribute,
            event,
            old_value,
            new_value,
            source,
            now()
        ],
    )?;
    Ok(())
}

impl Db {
    /// Every stored fact, sorted deterministically for the settings UI
    /// and the `<user_profile>` prompt block.
    pub fn memory_profile(&self) -> anyhow::Result<Vec<Memory>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "{MEMORY_SELECT} ORDER BY category ASC, attribute ASC, id ASC"
        ))?;
        let rows = stmt.query_map([], read_memory)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// A settings edit: rewrite only `value`, stamp manual provenance,
    /// clear the source ids. `Ok(None)` = unknown id (the command layer
    /// turns it into "Memory not found").
    pub fn memory_update(&self, id: i64, value: &str) -> anyhow::Result<Option<Memory>> {
        let value = value.trim();
        if value.is_empty() || value.chars().count() > 500 {
            return Err(anyhow::anyhow!("memory value must be 1..=500 characters"));
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let existing: Option<(String, String, String)> = tx
            .query_row(
                "SELECT category, attribute, value FROM memories WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((category, attribute, old)) = existing else {
            return Ok(None);
        };
        tx.execute(
            "UPDATE memories
             SET value = ?1, confidence = 1.0, basis = 'explicit', source = 'manual',
                 source_session_id = NULL, source_message_id = NULL, updated_at = ?2
             WHERE id = ?3",
            params![value, now(), id],
        )?;
        record_history(
            &tx,
            id,
            &category,
            &attribute,
            "update",
            Some(&old),
            Some(value),
            "manual",
        )?;
        let row = tx.query_row(&format!("{MEMORY_SELECT} WHERE id = ?1"), [id], read_memory)?;
        tx.commit()?;
        Ok(Some(row))
    }

    pub fn memory_delete(&self, id: i64) -> anyhow::Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let existing: Option<(String, String, String, String)> = tx
            .query_row(
                "SELECT category, attribute, value, source FROM memories WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((category, attribute, old, source)) = existing else {
            return Ok(());
        };
        tx.execute("DELETE FROM memories WHERE id = ?1", [id])?;
        record_history(
            &tx,
            id,
            &category,
            &attribute,
            "delete",
            Some(&old),
            None,
            &source,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Apply validated extractor output: insert new facts, update
    /// matching automatic rows, skip `"manual"` rows and unchanged
    /// values. Returns the number of rows actually inserted/changed —
    /// the caller emits `memory:changed` only when this is non-zero.
    /// One transaction so a multi-fact batch can't land partially.
    pub(crate) fn memory_apply(
        &self,
        session_id: Option<i64>,
        message_id: Option<i64>,
        facts: &[MemoryCandidate],
    ) -> anyhow::Result<usize> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let mut changed = 0;
        for fact in facts {
            let existing: Option<(i64, String, String)> = tx
                .query_row(
                    "SELECT id, source, value FROM memories
                     WHERE category = ?1 AND attribute = ?2",
                    params![fact.category, fact.attribute],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            match existing {
                // Manual rows are user authority — never overwrite. An
                // identical value is a no-op either way.
                Some((_id, source, value)) if source == "manual" || value == fact.value => {}
                Some((id, _, old)) => {
                    tx.execute(
                        "UPDATE memories
                         SET value = ?1, confidence = ?2, basis = ?3, source = 'automatic',
                             source_session_id = ?4, source_message_id = ?5, updated_at = ?6
                         WHERE id = ?7",
                        params![
                            fact.value,
                            fact.confidence,
                            fact.basis,
                            session_id,
                            message_id,
                            now(),
                            id
                        ],
                    )?;
                    record_history(
                        &tx,
                        id,
                        &fact.category,
                        &fact.attribute,
                        "update",
                        Some(&old),
                        Some(&fact.value),
                        "automatic",
                    )?;
                    changed += 1;
                }
                None => {
                    let timestamp = now();
                    tx.execute(
                        "INSERT INTO memories
                         (category, attribute, value, confidence, basis, source,
                          source_session_id, source_message_id, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, 'automatic', ?6, ?7, ?8, ?8)",
                        params![
                            fact.category,
                            fact.attribute,
                            fact.value,
                            fact.confidence,
                            fact.basis,
                            session_id,
                            message_id,
                            timestamp
                        ],
                    )?;
                    record_history(
                        &tx,
                        tx.last_insert_rowid(),
                        &fact.category,
                        &fact.attribute,
                        "add",
                        None,
                        Some(&fact.value),
                        "automatic",
                    )?;
                    changed += 1;
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// One fact's audit trail, oldest first — the `memory_history`
    /// command's row set. Entries survive the fact itself being
    /// deleted (the `delete` event is the last row).
    pub fn memory_history(&self, memory_id: i64) -> anyhow::Result<Vec<MemoryHistory>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, memory_id, category, attribute, event, old_value, new_value,
                    source, created_at
             FROM memory_history WHERE memory_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([memory_id], read_history)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
