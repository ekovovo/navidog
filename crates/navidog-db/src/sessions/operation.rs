//! Cancellation and draining belong to one live connection generation.

use std::future::poll_fn;
use std::task::Poll;

use futures::future::BoxFuture;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::*;

/// Each operation owns a receiver until cancellation and session cleanup have finished. Dropping
/// that receiver deregisters it automatically, including when its task panics or is aborted.
#[derive(Clone)]
pub(super) struct OperationScope {
    revoked: watch::Sender<bool>,
}

impl Default for OperationScope {
    fn default() -> Self {
        let (revoked, _) = watch::channel(false);
        Self { revoked }
    }
}

impl OperationScope {
    pub(super) fn revoke(&self) {
        self.revoked.send_replace(true);
    }

    pub(super) fn is_drained(&self) -> bool {
        self.revoked.receiver_count() == 0
    }

    pub(super) async fn drained(&self) {
        self.revoked.closed().await;
    }

    pub(super) fn run<T: Send + 'static>(
        &self,
        driver: Arc<dyn Driver>,
        params: ConnectParams,
        f: impl for<'a> FnOnce(&'a mut dyn Relational) -> BoxFuture<'a, DbResult<T>> + Send + 'static,
    ) -> JoinHandle<DbResult<T>> {
        let mut revoked = self.revoked.subscribe();
        runtime::spawn(async move {
            let initialization_may_write = !params.initial_queries.trim().is_empty();
            let mut connecting = crate::connect(driver, params);
            let mut connecting_started = false;
            let connected = {
                let state = revoked.clone();
                tokio::select! {
                    biased;
                    _ = until_revoked(&mut revoked) => None,
                    result = poll_fn(|cx| {
                        let current = state.borrow();
                        if *current {
                            Poll::Ready(None)
                        } else {
                            connecting_started = true;
                            connecting.as_mut().poll(cx).map(Some)
                        }
                    }) => result,
                }
            };
            let mut session = match connected {
                Some(result) => result?,
                None if !connecting_started => return Err(interrupted(false)),
                None => {
                    // Driver initialization can execute arbitrary SQL, and SQLite runs it in
                    // spawn_blocking. Dropping the connect future would detach that work and
                    // release the retirement fence before its writes or cleanup had finished.
                    return match connecting.await {
                        Ok(session) => {
                            if let Err(error) = session.close().await {
                                tracing::warn!(
                                    "Closing a retired initializing session failed: {error}"
                                );
                            }
                            Err(interrupted(initialization_may_write))
                        }
                        Err(error) => Err(error.with_session_loss(initialization_may_write)),
                    };
                }
            };
            let cancel = session.cancel_handle();
            let mut started = false;
            let result = {
                let state = revoked.clone();
                let mut operation = Box::pin(async {
                    match session.relational() {
                        Some(rel) => {
                            started = true;
                            f(rel).await
                        }
                        None => Err(not_relational()),
                    }
                });
                tokio::select! {
                    biased;
                    _ = until_revoked(&mut revoked) => None,
                    result = poll_fn(|cx| {
                        // Revocation and starting/polling the business closure have a definite
                        // order. Never begin another closure poll after this domain is revoked.
                        let current = state.borrow();
                        if *current {
                            Poll::Ready(None)
                        } else {
                            operation.as_mut().poll(cx).map(Some)
                        }
                    }) => result,
                }
            };
            if result.is_none()
                && started
                && let Some(cancel) = cancel
                && let Err(error) = cancel().await
            {
                tracing::warn!("Cancelling a retired operation session failed: {error}");
            }
            if let Err(error) = session.close().await {
                tracing::warn!("Closing an operation session failed: {error}");
            }
            // A result already observed before cleanup is known, even if the connection was
            // retired while closing. UI owners reject stale results using Sessions::generation.
            result.unwrap_or_else(|| Err(interrupted(started)))
        })
    }
}

pub(super) async fn connect_after_drain(
    retiring: Vec<OperationScope>,
    driver: Arc<dyn Driver>,
    params: ConnectParams,
) -> DbResult<Box<dyn Session>> {
    for scope in retiring {
        scope.drained().await;
    }
    crate::connect(driver, params).await
}

async fn until_revoked(revoked: &mut watch::Receiver<bool>) {
    if !*revoked.borrow() {
        let _ = revoked.changed().await;
    }
}

fn interrupted(outcome_unknown: bool) -> DbError {
    DbError::msg(rust_i18n::t!("database.operation_connection_retired").to_string())
        .with_session_loss(outcome_unknown)
}

#[cfg(test)]
mod tests {
    #[test]
    fn retirement_diagnostics_have_english_and_chinese_translations() {
        for (key, english, chinese) in [
            (
                "database.operation_connection_retired",
                "The operation's connection was closed or reopened.",
                "操作所属的连接已关闭或重新打开。",
            ),
            (
                "database.sqlite_queued_session_closed",
                "The SQLite session was closed before queued work started.",
                "排队的操作开始前，SQLite 会话已关闭。",
            ),
        ] {
            assert_eq!(rust_i18n::t!(key, locale = "en"), english);
            assert_eq!(rust_i18n::t!(key, locale = "zh-CN"), chinese);
        }
    }
}
