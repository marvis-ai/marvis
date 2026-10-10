use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::storage::Db;

struct LifecycleEntry {
    active: usize,
    deleting: bool,
    changed: Arc<Notify>,
}

/// Coordinates destructive session deletion with work that may hand a
/// session's history to a provider. The registry lock is held only while
/// changing the bookkeeping counters; [`SessionLease`] owns the lifetime
/// across provider awaits without retaining a mutex guard.
pub(crate) struct SessionLifecycle {
    entries: Mutex<HashMap<i64, LifecycleEntry>>,
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
            deleting: false,
            changed: Arc::new(Notify::new()),
        });
        if entry.deleting {
            return None;
        }
        entry.active += 1;
        Some(SessionLease {
            lifecycle: Arc::clone(self),
            session_id,
        })
    }

    /// Mark the session deleting, wait for all owned leases, remove the row,
    /// and always release the registry state before returning the DB result.
    /// A second delete request waits for the first request's registry entry to
    /// disappear and is idempotent for the command's purposes.
    pub(crate) async fn delete(&self, db: &Db, session_id: i64) -> anyhow::Result<()> {
        let (changed, owner) = {
            let mut entries = self.entries.lock();
            let entry = entries.entry(session_id).or_insert_with(|| LifecycleEntry {
                active: 0,
                deleting: false,
                changed: Arc::new(Notify::new()),
            });
            if entry.deleting {
                (Arc::clone(&entry.changed), false)
            } else {
                entry.deleting = true;
                (Arc::clone(&entry.changed), true)
            }
        };

        if !owner {
            loop {
                let notified = changed.notified();
                let still_registered = self.entries.lock().contains_key(&session_id);
                if !still_registered {
                    return Ok(());
                }
                notified.await;
            }
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

        let result = db.session_delete(session_id);
        let removed = {
            let mut entries = self.entries.lock();
            match entries.get(&session_id) {
                Some(entry) if entry.deleting && entry.active == 0 => {
                    entries.remove(&session_id);
                    true
                }
                _ => false,
            }
        };
        if removed {
            // `notify_one` retains a permit when a concurrent delete future
            // has not polled its waiter yet; `notify_waiters` could lose that
            // wake-up in the same race.
            changed.notify_one();
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn is_deleting(&self, session_id: i64) -> bool {
        self.entries
            .lock()
            .get(&session_id)
            .is_some_and(|entry| entry.deleting)
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

    #[test]
    fn lease_acquisition_and_drop_keep_registry_usable() {
        let lifecycle = SessionLifecycle::new();
        let lease = lifecycle.acquire(7).expect("initial lease should acquire");
        assert!(!lifecycle.is_deleting(7));
        drop(lease);
        assert!(lifecycle.acquire(7).is_some());
    }
}
