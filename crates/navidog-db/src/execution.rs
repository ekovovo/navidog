//! Conservative physical-session affinity and cancellation across safe reconnects.

use std::sync::{Arc, Mutex};

use futures::FutureExt as _;

use crate::{CancelHandle, DbError, dialect::Dialect};

pub(crate) struct RecoveryState {
    reconnectable: bool,
}

impl RecoveryState {
    pub(crate) fn new(initial_queries: &str, dialect: Dialect) -> Self {
        Self {
            reconnectable: crate::sql::split_statements(initial_queries, dialect)
                .iter()
                .all(|sql| crate::sql::session_setup(sql).is_some()),
        }
    }

    pub(crate) fn observe(&mut self, sql: &str) {
        // Arbitrary SQL can establish transactions, locks, temporary objects or session state.
        // Never infer that it is stateless from a SELECT prefix, nor reset affinity on COMMIT.
        self.reconnectable &= crate::sql::session_setup(sql).is_some();
    }

    pub(crate) fn can_reconnect(&self) -> bool {
        self.reconnectable
    }
}

pub(crate) fn interrupted(mut error: DbError, outcome_unknown: bool) -> DbError {
    error.disconnected = true;
    error.outcome_unknown = outcome_unknown;
    error
}

/// Only the driver owns this value; returned handles share the current target, not its snapshot.
#[derive(Default)]
pub(crate) struct Cancellation {
    target: Arc<Mutex<Option<CancelHandle>>>,
}

impl Cancellation {
    pub(crate) fn replace(&self, target: CancelHandle) {
        *self.target.lock().unwrap_or_else(|p| p.into_inner()) = Some(target);
    }

    pub(crate) fn clear(&self) {
        *self.target.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    pub(crate) fn handle(&self) -> CancelHandle {
        let target = self.target.clone();
        Arc::new(move || {
            let current = target.lock().unwrap_or_else(|p| p.into_inner()).clone();
            async move {
                match current {
                    Some(cancel) => cancel().await,
                    None => Err(DbError::msg("The database session is no longer available.")),
                }
            }
            .boxed()
        })
    }
}

impl Drop for Cancellation {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn arbitrary_sql_and_initial_queries_pin_the_physical_session() {
        let mut state = RecoveryState::new("", Dialect::Postgres);
        assert!(state.can_reconnect());
        state.observe("SET search_path TO public");
        assert!(state.can_reconnect());
        state.observe("SELECT side_effect()");
        assert!(!state.can_reconnect());
        state.observe("COMMIT");
        state.observe("SET search_path TO public");
        assert!(!state.can_reconnect());
        assert!(!RecoveryState::new("BEGIN", Dialect::Postgres).can_reconnect());
        assert!(RecoveryState::new("USE sakila", Dialect::MySql).can_reconnect());
    }

    #[test]
    fn old_cancellation_handles_follow_replacement_and_stop_after_close() {
        futures::executor::block_on(async {
            let calls = Arc::new(AtomicUsize::new(0));
            let cancellation = Cancellation::default();
            let handle = cancellation.handle();
            for n in [1, 10] {
                let calls = calls.clone();
                cancellation.replace(Arc::new(move || {
                    calls.fetch_add(n, Ordering::SeqCst);
                    async { Ok(()) }.boxed()
                }));
                handle().await.unwrap();
            }
            assert_eq!(calls.load(Ordering::SeqCst), 11);
            cancellation.clear();
            assert!(handle().await.is_err());
            assert_eq!(calls.load(Ordering::SeqCst), 11);
            drop(cancellation);
            assert!(handle().await.is_err());
        });
    }

    #[test]
    fn uncertain_outcomes_keep_the_original_database_error_code() {
        let error = interrupted(DbError::coded("08006", "connection lost"), true);
        assert_eq!(error.code.as_deref(), Some("08006"));
        assert!(error.disconnected);
        assert!(error.outcome_unknown);
        assert_eq!(error.message, "connection lost");
    }
}
