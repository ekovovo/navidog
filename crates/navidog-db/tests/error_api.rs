use navidog_db::DbError;

use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use navidog_db::{CancelHandle, DbResult, Session, sessions};
use tokio::sync::Mutex;

#[test]
#[allow(deprecated)]
fn legacy_operation_entry_point_remains_callable_from_another_crate() {
    let sessions = navidog_db::sessions::Sessions::default();
    assert!(
        sessions
            .run_once("closed", None, |_| Box::pin(async { Ok(()) }))
            .is_none()
    );
}

#[test]
fn error_record_supports_external_literals_and_exhaustive_destructuring() {
    let error = DbError {
        code: Some("08006".into()),
        message: "connection lost".into(),
        disconnected: true,
        outcome_unknown: true,
    };
    assert!(error.is_session_lost());
    assert!(error.is_outcome_unknown());
    assert_eq!(error.to_string(), "08006 - connection lost");
    let DbError {
        code,
        message,
        disconnected,
        outcome_unknown,
    } = error;
    assert_eq!(code.as_deref(), Some("08006"));
    assert_eq!(message, "connection lost");
    assert!(disconnected);
    assert!(outcome_unknown);
}

struct NonRelationalSession;

impl Session for NonRelationalSession {
    fn server_version(&self) -> &str {
        "test"
    }

    fn ping(&mut self) -> BoxFuture<'_, DbResult<()>> {
        async { Ok(()) }.boxed()
    }

    fn cancel_handle(&self) -> Option<CancelHandle> {
        None
    }

    fn session_id(&self) -> Option<u64> {
        None
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>> {
        async { Ok(()) }.boxed()
    }
}

#[test]
fn non_relational_session_error_uses_the_interface_language() {
    let previous = rust_i18n::locale().to_string();
    navidog_db::runtime::runtime().block_on(async {
        for (locale, expected) in [
            ("en", "This connection is not a relational database."),
            ("zh-CN", "该连接不是关系型数据库。"),
        ] {
            rust_i18n::set_locale(locale);
            let session: sessions::SharedSession =
                Arc::new(Mutex::new(Some(Box::new(NonRelationalSession))));
            let error = sessions::run_on::<()>(session, |_| {
                async { panic!("a non-relational session must not invoke the operation") }.boxed()
            })
            .await
            .unwrap()
            .unwrap_err();
            assert_eq!(error.message, expected);
            assert_eq!(error.code, None);
            assert!(!error.is_session_lost());
            assert!(!error.is_outcome_unknown());
        }
    });
    rust_i18n::set_locale(&previous);
}
