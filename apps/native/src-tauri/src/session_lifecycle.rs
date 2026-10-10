use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::{watch, Notify};

use crate::storage::Db;

struct DeletionGeneration {
    completion: watch::Sender<Option<Result<(), String>>>,
}

impl DeletionGeneration {
    fn new() -> Arc<Self> {
        let (completion, _) = watch::channel(None);
        Arc::new(Self { completion })
    }

    fn subscribe(&self) -> watch::Receiver<Option<Result<(), String>>> {
        self.completion.subscribe()
    }

    fn complete(&self, result: &anyhow::Result<()>) {
        let outcome = match result {
            Ok(()) => Ok(()),
            Err(error) => Err(error.to_string()),
        };
        let _ = self.completion.send(Some(outcome));
    }

    async fn wait(&self) -> anyhow::Result<()> {
        let mut completion = self.subscribe();
        loop {
            if let Some(result) = completion.borrow().clone() {
                return result.map_err(anyhow::Error::msg);
            }
            if completion.changed().await.is_err() {
                return Err(anyhow::anyhow!("session deletion completion was dropped"));
            }
        }
    }
}

struct LifecycleEntry {
    active: usize,
    deletion: Option<Arc<DeletionGeneration>>,
    changed: Arc<Notify>,
}

/// Coordinates destructive session deletion with work that may hand a
/// session's history to a provider. The registry lock is held only while
/// changing the bookkeeping counters; [`SessionLease`] owns the lifetime
/// across provider awaits without retaining a mutex guard.
pub(crate) struct SessionLifecycle {
    entries: Mutex<HashMap<i64, LifecycleEntry>>,
}

/// An immutable session incarnation plus an owned lease captured before an Ask
/// task is queued. The lease keeps the original row alive until the pipeline
/// has finished its handoff, while the token prevents a direct or stale storage
/// boundary from treating a recreated integer id as the original run.
pub(crate) struct SessionHandoff {
    pub(crate) session_id: i64,
    pub(crate) session_token: String,
    pub(crate) lease: SessionLease,
}

/// An owned per-session lease. Dropping it releases one active handoff and
/// wakes a deletion waiting for the last lease to leave.
#[must_use]
pub(crate) struct SessionLease {
    lifecycle: Arc<SessionLifecycle>,
    session_id: i64,
}

impl SessionLifecycle {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(HashMap::new()),
        })
    }

    /// Acquire a lease unless deletion has already been marked for the
    /// session. The increment and deletion check are one short critical
    /// section, so a successful lease prevents the delete path from reaching
    /// the database row until this lease is dropped.
    pub(crate) fn acquire(self: &Arc<Self>, session_id: i64) -> Option<SessionLease> {
        let mut entries = self.entries.lock();
        let entry = entries.entry(session_id).or_insert_with(|| LifecycleEntry {
            active: 0,
            deletion: None,
            changed: Arc::new(Notify::new()),
        });
        if entry.deletion.is_some() {
            return None;
        }
        entry.active += 1;
        Some(SessionLease {
            lifecycle: Arc::clone(self),
            session_id,
        })
    }

    /// Capture the immutable session token and its owned lease synchronously.
    /// The lease is acquired before the identity read so a production queued
    /// Ask cannot observe one incarnation and start on another after deletion
    /// has been marked.
    pub(crate) fn capture_handoff(
        self: &Arc<Self>,
        db: &Db,
        session_id: i64,
    ) -> anyhow::Result<SessionHandoff> {
        let lease = self
            .acquire(session_id)
            .ok_or_else(|| anyhow::anyhow!("session deletion is already in progress"))?;
        let Some((session_token, _, _)) = db.session_compaction(session_id)? else {
            return Err(anyhow::anyhow!("session no longer exists"));
        };
        Ok(SessionHandoff {
            session_id,
            session_token,
            lease,
        })
    }

    /// Mark the session deleting, wait for all owned leases, remove the row,
    /// and always release the registry state before returning the DB result.
    /// Duplicate callers subscribe to this exact deletion generation and
    /// receive the owner's result, including an error. A later incarnation
    /// creates a fresh generation after the entry is released.
    pub(crate) async fn delete(&self, db: &Db, session_id: i64) -> anyhow::Result<()> {
        self.delete_inner(session_id, || db.session_delete(session_id))
            .await
    }

    async fn delete_inner<F>(&self, session_id: i64, delete: F) -> anyhow::Result<()>
    where
        F: FnOnce() -> anyhow::Result<()>,
    {
        let (generation, owner) = {
            let mut entries = self.entries.lock();
            let entry = entries.entry(session_id).or_insert_with(|| LifecycleEntry {
                active: 0,
                deletion: None,
                changed: Arc::new(Notify::new()),
            });
            if let Some(generation) = &entry.deletion {
                (Arc::clone(generation), false)
            } else {
                let generation = DeletionGeneration::new();
                entry.deletion = Some(Arc::clone(&generation));
                (generation, true)
            }
        };

        if !owner {
            return generation.wait().await;
        }

        loop {
            let notified = {
                let entries = self.entries.lock();
                let Some(entry) = entries.get(&session_id) else {
                    break;
                };
                if entry.active == 0 {
                    break;
                }
                Arc::clone(&entry.changed)
            };
            notified.notified().await;
        }

        let result = delete();
        generation.complete(&result);
        {
            let mut entries = self.entries.lock();
            let remove = entries.get(&session_id).is_some_and(|entry| {
                entry.active == 0
                    && entry
                        .deletion
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &generation))
            });
            if remove {
                entries.remove(&session_id);
            }
        }
        result
    }

    #[cfg(test)]
    pub(crate) async fn delete_with_result_for_test(
        &self,
        session_id: i64,
        result: anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        self.delete_inner(session_id, || result).await
    }

    #[cfg(test)]
    pub(crate) fn is_deleting(&self, session_id: i64) -> bool {
        self.entries
            .lock()
            .get(&session_id)
            .is_some_and(|entry| entry.deletion.is_some())
    }
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        let changed = {
            let mut entries = self.lifecycle.entries.lock();
            let Some(entry) = entries.get_mut(&self.session_id) else {
                return;
            };
            debug_assert!(entry.active > 0);
            entry.active = entry.active.saturating_sub(1);
            Arc::clone(&entry.changed)
        };
        changed.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn wait_for_delete_mark(lifecycle: &SessionLifecycle, session_id: i64) {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if lifecycle.is_deleting(session_id) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session deletion did not reach its lifecycle boundary");
    }

    fn test_db() -> (std::path::PathBuf, Arc<Db>) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "marvis-session-lifecycle-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let db = Arc::new(Db::at(dir.join("marvis.db")).expect("test database"));
        (dir, db)
    }

    #[test]
    fn lease_acquisition_and_drop_keep_registry_usable() {
        let lifecycle = SessionLifecycle::new();
        let lease = lifecycle.acquire(7).expect("initial lease should acquire");
        assert!(!lifecycle.is_deleting(7));
        drop(lease);
        assert!(lifecycle.acquire(7).is_some());
    }

    #[tokio::test]
    async fn duplicate_delete_waiters_receive_the_same_generation_result() {
        let (dir, db) = test_db();
        let session_id = db.session_get_or_create_active("ask").unwrap();
        let lifecycle = SessionLifecycle::new();
        let blocker = lifecycle.acquire(session_id).unwrap();

        let owner = {
            let lifecycle = Arc::clone(&lifecycle);
            let db = Arc::clone(&db);
            tokio::spawn(async move { lifecycle.delete(db.as_ref(), session_id).await })
        };
        wait_for_delete_mark(&lifecycle, session_id).await;

        let waiter_one = {
            let lifecycle = Arc::clone(&lifecycle);
            let db = Arc::clone(&db);
            tokio::spawn(async move { lifecycle.delete(db.as_ref(), session_id).await })
        };
        let waiter_two = {
            let lifecycle = Arc::clone(&lifecycle);
            let db = Arc::clone(&db);
            tokio::spawn(async move { lifecycle.delete(db.as_ref(), session_id).await })
        };
        drop(blocker);

        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), owner)
                .await
                .expect("owner deletion hung")
                .unwrap()
                .is_ok()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), waiter_one)
                .await
                .expect("first duplicate deletion hung")
                .unwrap()
                .is_ok()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), waiter_two)
                .await
                .expect("second duplicate deletion hung")
                .unwrap()
                .is_ok()
        );
        assert!(db.session_compaction(session_id).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn duplicate_waiter_keeps_its_generation_result_across_recreation() {
        let (dir, db) = test_db();
        let session_id = db.session_get_or_create_active("ask").unwrap();
        let lifecycle = SessionLifecycle::new();
        let blocker = lifecycle.acquire(session_id).unwrap();

        let owner = {
            let lifecycle = Arc::clone(&lifecycle);
            let db = Arc::clone(&db);
            tokio::spawn(async move { lifecycle.delete(db.as_ref(), session_id).await })
        };
        wait_for_delete_mark(&lifecycle, session_id).await;
        let waiter = {
            let lifecycle = Arc::clone(&lifecycle);
            let db = Arc::clone(&db);
            tokio::spawn(async move { lifecycle.delete(db.as_ref(), session_id).await })
        };
        drop(blocker);

        owner
            .await
            .expect("owner deletion task should join")
            .unwrap();
        assert_eq!(db.session_get_or_create_active("ask").unwrap(), session_id);
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("duplicate waiter hung after recreation")
            .unwrap()
            .unwrap();

        lifecycle.delete(db.as_ref(), session_id).await.unwrap();
        assert!(db.session_compaction(session_id).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn duplicate_delete_waiters_receive_owner_failure_and_registry_releases() {
        let (dir, db) = test_db();
        let session_id = db.session_get_or_create_active("ask").unwrap();
        let lifecycle = SessionLifecycle::new();
        let blocker = lifecycle.acquire(session_id).unwrap();

        let owner = {
            let lifecycle = Arc::clone(&lifecycle);
            tokio::spawn(async move {
                lifecycle
                    .delete_with_result_for_test(
                        session_id,
                        Err(anyhow::anyhow!("injected delete failure")),
                    )
                    .await
            })
        };
        wait_for_delete_mark(&lifecycle, session_id).await;
        let waiter = {
            let lifecycle = Arc::clone(&lifecycle);
            tokio::spawn(async move {
                lifecycle
                    .delete_with_result_for_test(session_id, Ok(()))
                    .await
            })
        };
        drop(blocker);

        let owner_error = owner
            .await
            .expect("owner failure task should join")
            .expect_err("owner deletion should fail");
        assert_eq!(owner_error.to_string(), "injected delete failure");
        let waiter_error = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("duplicate failure waiter hung")
            .unwrap()
            .expect_err("duplicate waiter should receive the owner failure");
        assert_eq!(waiter_error.to_string(), "injected delete failure");
        assert!(!lifecycle.is_deleting(session_id));
        let lease = lifecycle
            .acquire(session_id)
            .expect("registry should release");
        drop(lease);
        assert!(db.session_compaction(session_id).unwrap().is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
