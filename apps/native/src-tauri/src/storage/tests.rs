use super::*;
use std::path::{Path, PathBuf};
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
fn session_compaction_roundtrips_and_uses_watermark_cas() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let sid = db.session_get_or_create_active("ask").unwrap();

    assert_eq!(db.session_compaction(sid).unwrap(), Some((None, None)));
    assert!(db
        .session_compact_write(sid, None, "first digest", 10)
        .unwrap());
    assert_eq!(
        db.session_compaction(sid).unwrap(),
        Some((Some("first digest".to_string()), Some(10)))
    );
    assert!(!db
        .session_compact_write(sid, None, "stale digest", 20)
        .unwrap());
    assert!(db
        .session_compact_write(sid, Some(10), "second digest", 20)
        .unwrap());
    assert_eq!(
        db.session_compaction(sid).unwrap(),
        Some((Some("second digest".to_string()), Some(20)))
    );

    db.session_compact_clear(sid).unwrap();
    assert_eq!(db.session_compaction(sid).unwrap(), Some((None, None)));
    assert_eq!(db.session_compaction(i64::MAX).unwrap(), None);
    assert!(!db
        .session_compact_write(i64::MAX, None, "missing digest", 1)
        .unwrap());
    db.session_compact_clear(i64::MAX).unwrap();

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

/// Attach a real managed-looking file to `message_id` under `dir`.
fn attach(db: &Db, dir: &Path, message_id: i64, name: &str, position: i64) -> PathBuf {
    let path = dir.join("attachments").join(format!("att-{name}.jpg"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"jpeg").unwrap();
    db.attachments_add(
        message_id,
        &[NewAttachment {
            name: name.to_string(),
            path: path.to_string_lossy().into_owned(),
            mime: "image/jpeg".to_string(),
            bytes: 4,
            position,
        }],
    )
    .unwrap();
    path
}

#[test]
fn attachments_round_trip_ordered_on_the_message() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let sid = db.session_get_or_create_active("ask").unwrap();
    let mid = db.message_add(sid, "user", "see attached").unwrap();

    // Stored out of order — `position`, not insertion, orders them.
    attach(&db, &dir, mid, "b", 1);
    attach(&db, &dir, mid, "a", 0);
    let plain = db.message_add(sid, "assistant", "ok").unwrap();
    assert_eq!(db.attachments_for(plain).unwrap().len(), 0);

    let msgs = db.messages_for(sid).unwrap();
    assert_eq!(msgs[0].attachments.len(), 2);
    assert_eq!(msgs[0].attachments[0].name, "a");
    assert_eq!(msgs[0].attachments[0].position, 0);
    assert_eq!(msgs[0].attachments[0].mime, "image/jpeg");
    assert_eq!(msgs[0].attachments[1].name, "b");
    assert!(msgs[1].attachments.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn message_delete_removes_managed_attachment_files() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let sid = db.session_get_or_create_active("ask").unwrap();
    let mid = db.message_add(sid, "user", "pic").unwrap();
    let path = attach(&db, &dir, mid, "p", 0);

    db.message_delete(mid).unwrap();
    assert!(!path.exists());
    assert!(db.attachments_for(mid).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn session_delete_removes_attachment_files() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let sid = db.session_get_or_create_active("ask").unwrap();
    let mid = db.message_add(sid, "user", "pic").unwrap();
    let path = attach(&db, &dir, mid, "p", 0);

    db.session_delete(sid).unwrap();
    assert!(!path.exists());
    assert!(db.messages_for(sid).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transcript_roundtrip_is_oldest_first_with_limit() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let sid = db.session_get_or_create_active("listen").unwrap();

    db.transcript_add(sid, "me", "hello", None, None).unwrap();
    db.transcript_add(sid, "them", "hi there", None, Some(1250))
        .unwrap();
    db.transcript_add(sid, "me", "follow-up", None, None)
        .unwrap();

    let transcripts = db.transcripts_for(sid, None).unwrap();
    assert_eq!(transcripts.len(), 3);
    assert_eq!(transcripts[0].speaker, "me");
    assert_eq!(transcripts[0].content, "hello");
    assert_eq!(transcripts[1].speaker, "them");
    assert_eq!(transcripts[1].content, "hi there");
    assert_eq!(transcripts[1].audio_start_ms, Some(1250));
    assert_eq!(
        db.transcripts_tail(sid, 3).unwrap()[1].audio_start_ms,
        Some(1250)
    );
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
        db.transcript_add(sid, "me", &format!("turn-{id}"), None, None)
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
    db.transcript_add(sid, "me", "hello", None, None).unwrap();
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
    assert_eq!(transcripts[0].audio_start_ms, None);
    db.transcript_add(1, "them", "new turn", Some(1), None)
        .unwrap();
    let transcripts = db.transcripts_for(1, None).unwrap();
    assert_eq!(transcripts[1].speaker_idx, Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn session_compaction_migrates_legacy_schema() {
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
                 VALUES (1, 'user', 'old question', 1);",
        )
        .unwrap();
    }

    let db = Db::at(&path).unwrap();
    assert_eq!(db.session_compaction(1).unwrap(), Some((None, None)));
    assert!(db.session_compact_write(1, None, "legacy-safe", 1).unwrap());
    assert_eq!(
        db.session_compaction(1).unwrap().unwrap().0.as_deref(),
        Some("legacy-safe")
    );
    assert_eq!(db.messages_for(1).unwrap()[0].content, "old question");

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

#[test]
fn concurrent_session_resolution_creates_one_row() {
    for linked in [false, true] {
        let dir = tmp_dir();
        let db = std::sync::Arc::new(Db::at(dir.join("marvis.db")).unwrap());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let db = db.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    if linked {
                        db.ask_session_for_listen(42).unwrap()
                    } else {
                        db.session_get_or_create_active("ask").unwrap()
                    }
                })
            })
            .collect();
        let ids: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert!(ids.iter().all(|id| *id == ids[0]));
        assert_eq!(db.session_list().unwrap().len(), 1);
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn legacy_message_merge_rolls_back_if_drop_fails() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;
        CREATE TABLE ai_messages (id INTEGER PRIMARY KEY, session_id INTEGER, role TEXT, content TEXT, ts INTEGER);
        CREATE TABLE messages (session_id INTEGER, role TEXT, content TEXT, ts INTEGER);
        CREATE TABLE legacy_child (parent INTEGER REFERENCES ai_messages(id));
        INSERT INTO ai_messages VALUES (1, 1, 'user', 'preserve me', 1);
        INSERT INTO legacy_child VALUES (1);").unwrap();
    assert!(migrate(&conn).is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT content FROM ai_messages", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "preserve me"
    );
}

#[test]
fn memory_profile_inserts_updates_and_manual_edits_win() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let session_id = db.session_get_or_create_active("ask").unwrap();
    let message_id = db
        .message_add(session_id, "user", "My name is Allen.")
        .unwrap();
    let name = MemoryCandidate {
        category: "identity".into(),
        attribute: "name".into(),
        value: "The user's name is Allen.".into(),
        confidence: 0.98,
        basis: "explicit".into(),
    };

    assert_eq!(
        db.memory_apply(Some(session_id), Some(message_id), &[name.clone()])
            .unwrap(),
        1
    );
    let first = db.memory_profile().unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].value, "The user's name is Allen.");
    assert_eq!(first[0].source_session_id, Some(session_id));
    assert_eq!(first[0].source_message_id, Some(message_id));

    let changed = MemoryCandidate {
        value: "The user's name is Chenillen.".into(),
        confidence: 0.99,
        ..name.clone()
    };
    assert_eq!(
        db.memory_apply(Some(session_id), Some(message_id), &[changed.clone()])
            .unwrap(),
        1
    );
    assert_eq!(
        db.memory_profile().unwrap()[0].value,
        "The user's name is Chenillen."
    );

    let id = db.memory_profile().unwrap()[0].id;
    let edited = db
        .memory_update(id, "The user's preferred name is Chenillen.")
        .unwrap()
        .unwrap();
    assert_eq!(edited.source, "manual");
    assert_eq!(edited.basis, "explicit");
    assert_eq!(edited.confidence, 1.0);
    assert_eq!(edited.source_session_id, None);
    assert_eq!(edited.source_message_id, None);

    // Automatic extraction must never overwrite a manual row.
    let ignored = MemoryCandidate {
        value: "The user's name is Different.".into(),
        ..changed
    };
    assert_eq!(
        db.memory_apply(Some(session_id), Some(message_id), &[ignored])
            .unwrap(),
        0
    );
    assert_eq!(
        db.memory_profile().unwrap()[0].value,
        "The user's preferred name is Chenillen."
    );

    db.memory_delete(id).unwrap();
    assert!(db.memory_profile().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Every write to `memories` leaves one `memory_history` row with the
/// old/new values and the writer's authority — and the trail survives
/// the fact's own deletion (the `delete` row is the last entry).
#[test]
fn memory_history_audits_every_write_and_survives_delete() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let session_id = db.session_get_or_create_active("ask").unwrap();
    let message_id = db
        .message_add(session_id, "user", "My name is Allen.")
        .unwrap();
    let fact = MemoryCandidate {
        category: "identity".into(),
        attribute: "name".into(),
        value: "The user's name is Allen.".into(),
        confidence: 0.98,
        basis: "explicit".into(),
    };
    db.memory_apply(Some(session_id), Some(message_id), &[fact.clone()])
        .unwrap();
    let id = db.memory_profile().unwrap()[0].id;
    db.memory_apply(
        Some(session_id),
        Some(message_id),
        &[MemoryCandidate {
            value: "The user's name is Chenillen.".into(),
            ..fact
        }],
    )
    .unwrap();
    db.memory_update(id, "The user's preferred name is Chenillen.")
        .unwrap();
    db.memory_delete(id).unwrap();

    let history = db.memory_history(id).unwrap();
    assert_eq!(
        history.iter().map(|h| h.event.as_str()).collect::<Vec<_>>(),
        ["add", "update", "update", "delete"]
    );
    assert_eq!(history[0].old_value, None);
    assert_eq!(
        history[0].new_value.as_deref(),
        Some("The user's name is Allen.")
    );
    assert_eq!(history[0].source, "automatic");
    assert_eq!(
        history[1].old_value.as_deref(),
        Some("The user's name is Allen.")
    );
    assert_eq!(
        history[1].new_value.as_deref(),
        Some("The user's name is Chenillen.")
    );
    assert_eq!(history[2].source, "manual");
    assert_eq!(history[3].new_value, None);
    assert_eq!(
        history[3].old_value.as_deref(),
        Some("The user's preferred name is Chenillen.")
    );
    assert!(history
        .iter()
        .all(|h| h.memory_id == id && h.attribute == "name"));
    let _ = std::fs::remove_dir_all(dir);
}

/// `<recent_messages>` reads strictly before the source row — the
/// newest `limit` user/assistant turns, oldest first — so the
/// extractor's context never includes the message it's extracting.
#[test]
fn message_tail_reads_only_prior_turns_oldest_first() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let session_id = db.session_get_or_create_active("ask").unwrap();
    db.message_add(session_id, "user", "first").unwrap();
    db.message_add(session_id, "assistant", "answer one")
        .unwrap();
    let source = db.message_add(session_id, "user", "the source").unwrap();

    assert_eq!(
        db.message_tail(session_id, source, 6).unwrap(),
        vec![
            ("user".to_string(), "first".to_string()),
            ("assistant".to_string(), "answer one".to_string()),
        ]
    );
    assert_eq!(
        db.message_tail(session_id, source, 1).unwrap(),
        vec![("assistant".to_string(), "answer one".to_string())]
    );
    assert!(db.message_tail(session_id, 1, 6).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn memory_source_foreign_keys_are_cleared_when_history_is_deleted() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let session_id = db.session_get_or_create_active("ask").unwrap();
    let message_id = db
        .message_add(session_id, "user", "I prefer lists.")
        .unwrap();
    db.memory_apply(
        Some(session_id),
        Some(message_id),
        &[MemoryCandidate {
            category: "preference".into(),
            attribute: "formatting".into(),
            value: "The user prefers lists.".into(),
            confidence: 0.9,
            basis: "explicit".into(),
        }],
    )
    .unwrap();

    db.session_delete(session_id).unwrap();
    let row = &db.memory_profile().unwrap()[0];
    assert_eq!(row.source_session_id, None);
    assert_eq!(row.source_message_id, None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn existing_database_open_creates_memory_table_without_backfill() {
    let dir = tmp_dir();
    let path = dir.join("marvis.db");
    std::fs::create_dir_all(&dir).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE sessions (id INTEGER PRIMARY KEY, type TEXT NOT NULL, started_at INTEGER NOT NULL, last_active_at INTEGER NOT NULL);",
    )
    .unwrap();
    drop(conn);

    let db = Db::at(&path).unwrap();
    assert!(db.memory_profile().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// `(category, attribute)` is structurally unique on a fresh database:
/// a second row under the same key can never land, and the apply path
/// still updates that key in place.
#[test]
fn memory_key_is_unique_and_updates_in_place() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let session_id = db.session_get_or_create_active("ask").unwrap();
    let message_id = db.message_add(session_id, "user", "Call me Al.").unwrap();
    db.memory_apply(
        Some(session_id),
        Some(message_id),
        &[MemoryCandidate {
            category: "identity".into(),
            attribute: "name".into(),
            value: "The user goes by Al.".into(),
            confidence: 0.9,
            basis: "explicit".into(),
        }],
    )
    .unwrap();

    // A duplicate key cannot be inserted by any path — not even a raw
    // write that bypasses `memory_apply`'s select-then-upsert.
    let conn = db.conn.lock();
    let dup = conn.execute(
        "INSERT INTO memories (category, attribute, value, confidence, basis, source, created_at, updated_at)
         VALUES ('identity', 'name', 'duplicate', 1.0, 'explicit', 'automatic', 0, 0)",
        [],
    );
    assert!(dup.is_err());
    drop(conn);

    // Same key through the normal path still updates the row in place.
    db.memory_apply(
        Some(session_id),
        Some(message_id),
        &[MemoryCandidate {
            category: "identity".into(),
            attribute: "name".into(),
            value: "The user goes by Allen.".into(),
            confidence: 0.95,
            basis: "explicit".into(),
        }],
    )
    .unwrap();
    let profile = db.memory_profile().unwrap();
    assert_eq!(profile.len(), 1);
    assert_eq!(profile[0].value, "The user goes by Allen.");
    let _ = std::fs::remove_dir_all(dir);
}

/// A database written before the constraint can hold synonym-slug or
/// duplicate-key rows. Migration collapses each key to one row —
/// keeping the manual row when present (user authority), else the
/// newest — then installs the unique index.
#[test]
fn migration_dedups_memory_keys_and_manual_rows_win() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("marvis.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (
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
                updated_at       INTEGER NOT NULL
            );
            CREATE INDEX memories_category_attribute
                ON memories(category, attribute);
            INSERT INTO memories (category, attribute, value, confidence, basis, source, created_at, updated_at) VALUES
                ('identity', 'name', 'auto name', 0.9, 'explicit', 'automatic', 1, 1),
                ('identity', 'name', 'manual name', 1.0, 'explicit', 'manual', 2, 2),
                ('preference', 'theme', 'old theme', 0.8, 'inferred', 'automatic', 1, 1),
                ('preference', 'theme', 'new theme', 0.9, 'explicit', 'automatic', 2, 3);",
        )
        .unwrap();
    }

    {
        let db = Db::at(&path).unwrap();
        let profile = db.memory_profile().unwrap();
        assert_eq!(profile.len(), 2);
        let name = profile.iter().find(|m| m.attribute == "name").unwrap();
        assert_eq!(name.value, "manual name");
        assert_eq!(name.source, "manual");
        let theme = profile.iter().find(|m| m.attribute == "theme").unwrap();
        assert_eq!(theme.value, "new theme");

        let conn = db.conn.lock();
        let dup = conn.execute(
            "INSERT INTO memories (category, attribute, value, confidence, basis, source, created_at, updated_at)
             VALUES ('identity', 'name', 'dup', 1.0, 'explicit', 'automatic', 0, 0)",
            [],
        );
        assert!(dup.is_err());
    }
    // Reopening is idempotent — the index exists, the dedup is a no-op.
    {
        let db = Db::at(&path).unwrap();
        assert_eq!(db.memory_profile().unwrap().len(), 2);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
