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

impl Db {
    /// Every stored fact, sorted deterministically for the settings UI
    /// and the `<user_profile>` prompt block.
    pub fn memory_profile(&self) -> anyhow::Result<Vec<Memory>> {
        let conn = self.conn.lock();
        let mut stmt =
            conn.prepare(&format!("{MEMORY_SELECT} ORDER BY category ASC, attribute ASC, id ASC"))?;
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
        let conn = self.conn.lock();
        let changed = conn.execute(
            "UPDATE memories
             SET value = ?1, confidence = 1.0, basis = 'explicit', source = 'manual',
                 source_session_id = NULL, source_message_id = NULL, updated_at = ?2
             WHERE id = ?3",
            params![value, now(), id],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(Some(conn.query_row(
            &format!("{MEMORY_SELECT} WHERE id = ?1"),
            [id],
            read_memory,
        )?))
    }

    pub fn memory_delete(&self, id: i64) -> anyhow::Result<()> {
        self.conn
            .lock()
            .execute("DELETE FROM memories WHERE id = ?1", [id])?;
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
                Some((id, _, _)) => {
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
                    changed += 1;
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }
}
