//! Reload (F-CONN-019, MySQL / MariaDB): run the chosen FLUSH options on the connection.
//! Needs the RELOAD privilege; the server's error is shown as is.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _, Window,
};
use navidog_db::{DbError, sessions::Sessions};
use navidog_ui::skin::{FormRow, confirm_with, heading};
use navidog_ui::tokens;
use rust_i18n::t;

use crate::secrets::footer;

/// FLUSH options both MySQL 8.4 and MariaDB 11 accept (MySQL 8.4 dropped HOSTS). Each runs as
/// its own statement because FLUSH TABLES can't share a statement with other options.
const OPTIONS: [&str; 5] = ["PRIVILEGES", "LOGS", "STATUS", "TABLES", "USER_RESOURCES"];

pub fn open(id: String, window: &mut Window, cx: &mut App) {
    let view = cx.new(|_| Reload {
        id,
        checked: [true, false, false, false, false],
    });
    window.open_dialog(cx, move |dialog, window, _| {
        confirm_with(dialog, &view, Reload::run)
            .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
            .close_button(false)
            .child(view.clone())
    });
}

struct Reload {
    id: String,
    checked: [bool; OPTIONS.len()],
}

/// One `FLUSH x` per checked option.
fn statements(checked: &[bool]) -> Vec<String> {
    OPTIONS
        .iter()
        .zip(checked)
        .filter(|(_, c)| **c)
        .map(|(o, _)| format!("FLUSH {o}"))
        .collect()
}

impl Reload {
    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stmts = statements(&self.checked);
        if stmts.is_empty() {
            return;
        }
        window.close_dialog(cx);
        let id = self.id.clone();
        let generation = Sessions::global(cx).read(cx).generation(&id);
        let job = Sessions::global(cx)
            .read(cx)
            .run_operation_rel(&self.id, None, |r| {
                Box::pin(async move {
                    for sql in &stmts {
                        r.execute(sql).await?;
                    }
                    Ok(())
                })
            });
        let Some(job) = job else { return };
        window
            .spawn(cx, async move |cx| {
                let result = job
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| error_message(&e)));
                cx.update(|window, cx| {
                    if Sessions::global(cx).read(cx).generation(&id) != generation {
                        return;
                    }
                    match result {
                        Ok(()) => window.push_notification(t!("conn.reload_done").to_string(), cx),
                        Err(e) => {
                            crate::show_error(t!("conn.reload_failed").into(), e.into(), window, cx)
                        }
                    }
                })
            })
            .detach();
    }
}

fn error_message(error: &DbError) -> String {
    recovery_message(error, &rust_i18n::locale())
}

fn recovery_message(error: &DbError, locale: &str) -> String {
    navidog_ui::db_error_text_with(
        error.code.as_deref(),
        &error.message,
        error.is_session_lost(),
        error.is_outcome_unknown(),
        |detail| t!("conn.reload_session_lost", locale = locale, detail = detail).to_string(),
        |detail| {
            t!(
                "conn.reload_outcome_unknown",
                locale = locale,
                detail = detail
            )
            .to_string()
        },
    )
}

impl Render for Reload {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(heading(t!("conn.reload_title").to_string()))
            .children(OPTIONS.iter().enumerate().map(|(ix, o)| {
                FormRow::bare(
                    Checkbox::new(("flush", ix))
                        .label(*o)
                        .checked(self.checked[ix])
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            this.checked[ix] = *checked;
                            cx.notify();
                        })),
                )
            }))
            .child(footer(
                t!("conn.reload").to_string(),
                cx.listener(|this, _, window, cx| this.run(window, cx)),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::{recovery_message, statements};

    #[test]
    fn one_flush_per_checked_option() {
        assert_eq!(
            statements(&[true, false, false, true, false]),
            ["FLUSH PRIVILEGES", "FLUSH TABLES"]
        );
        assert!(statements(&[false; 5]).is_empty());
    }

    #[test]
    fn flush_recovery_guidance_keeps_diagnostics_and_uses_the_interface_language() {
        let mut error = navidog_db::DbError::coded("08006", "original FLUSH diagnostic");
        let detail = error.to_string();
        for locale in ["en", "zh-CN"] {
            assert_eq!(recovery_message(&error, locale), detail);
            error.disconnected = true;
            for unknown in [false, true] {
                error.outcome_unknown = unknown;
                let message = recovery_message(&error, locale);
                assert!(message.starts_with(&detail));
                assert!(!message.contains('\n'));
                if locale == "en" {
                    assert!(message.to_ascii_lowercase().contains("reopen"));
                } else {
                    assert!(message.contains("重新打开"));
                }
                if unknown {
                    assert!(message.contains(if locale == "en" {
                        "outcome may be unknown"
                    } else {
                        "执行结果可能未知"
                    }));
                }
                if locale == "zh-CN" {
                    assert!(!message.contains("Reopen"));
                }
            }
            error.disconnected = false;
            error.outcome_unknown = false;
        }
        assert_eq!(error.to_string(), detail);
    }
}
