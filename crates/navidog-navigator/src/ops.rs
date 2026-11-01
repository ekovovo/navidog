//! Operations on the selected objects (F-OBJ-008): delete, empty, truncate and duplicate.
//! The navigator tree, the objects tab and the menu bar dispatch the same actions; renaming is
//! edited in place in the navigator (`NavigatorPanel::start_rename_object`).

use std::collections::HashSet;
use std::sync::Arc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, InteractiveElement, Window};
use navidog_app_state::actions::{
    DeleteObjects, DuplicateQuery, DuplicateTable, EmptyTables, OpenQueryExternally, QueryRenamed,
    RefreshMaterializedView, ReopenConnectionId, RevealQuery, RunMaintenance, TruncateTables,
};
use navidog_app_state::workspace::{Selection, Workspace};
use navidog_db::DbError;
use navidog_model::object::{ObjectKind, ObjectRef};
use navidog_model::queries;
use navidog_session::sessions::{Sessions, prepare_database};
use navidog_sql::dialect::Dialect;
use navidog_ui::assets::lucide;
use rust_i18n::t;

use crate::catalog::{Catalog, Load, Requirement};
use crate::navigator::kind_label;

pub(crate) fn selected_objects(cx: &App) -> Vec<ObjectRef> {
    Workspace::global(cx)
        .read(cx)
        .selections()
        .filter_map(|s| match s {
            Selection::Object(o) => Some(o.clone()),
            _ => None,
        })
        .collect()
}

fn dialect(obj: &ObjectRef, cx: &App) -> Option<Arc<dyn Dialect>> {
    Sessions::global(cx).read(cx).dialect(&obj.scope.connection)
}

pub(crate) fn bind<E: InteractiveElement>(el: E) -> E {
    el.on_action(|_: &DeleteObjects, window, cx| delete(window, cx))
        .on_action(|_: &EmptyTables, window, cx| empty(false, window, cx))
        .on_action(|_: &TruncateTables, window, cx| empty(true, window, cx))
        .on_action(|a: &DuplicateTable, window, cx| duplicate(a.with_data, window, cx))
        .on_action(|a: &RunMaintenance, window, cx| run_maintenance(a, window, cx))
        .on_action(|a: &RefreshMaterializedView, window, cx| {
            let sql = Sessions::global(cx)
                .read(cx)
                .dialect(&a.object.scope.connection)
                .and_then(|d| {
                    d.refresh_materialized_view(&a.object.scope, &a.object.name, a.with_data)
                });
            if let Some(sql) = sql {
                run(vec![(a.object.clone(), sql)], None, window, cx);
            }
        })
        .on_action(|_: &DuplicateQuery, window, cx| duplicate_query(window, cx))
        .on_action(|_: &OpenQueryExternally, _, cx| {
            for o in selected_queries(cx) {
                cx.open_with_system(&queries::path(&o.scope, &o.name));
            }
        })
        .on_action(|_: &RevealQuery, _, cx| {
            if let Some(o) = selected_queries(cx).first() {
                cx.reveal_path(&queries::path(&o.scope, &o.name));
            }
        })
}

fn selected_queries(cx: &App) -> Vec<ObjectRef> {
    selected_objects(cx)
        .into_iter()
        .filter(|o| o.kind == ObjectKind::Query)
        .collect()
}

/// Deleting saved queries deletes their files, after confirming.
fn delete_queries(list: Vec<ObjectRef>, window: &mut Window, cx: &mut App) {
    let message = match list.as_slice() {
        [] => return,
        [o] => t!("obj.confirm_drop", kind = kind_label(o.kind), name = o.name),
        _ => t!("obj.confirm_drop_many", n = list.len()),
    }
    .to_string();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let list = list.clone();
        alert
            .icon(lucide("triangle-alert"))
            .title(message.clone())
            .show_cancel(true)
            .ok_text(t!("obj.ok").to_string())
            .cancel_text(t!("obj.cancel").to_string())
            .on_ok(move |_, window, cx| {
                let failed = list
                    .iter()
                    .find_map(|o| queries::delete(&o.scope, &o.name).err());
                Workspace::global(cx).update(cx, |w, cx| w.queries_changed(cx));
                if let Some(e) = failed {
                    show_error(e.to_string(), window, cx);
                }
                true
            })
    });
}

/// Renames a saved query's file; open editors follow through `QueryRenamed`.
pub(crate) fn rename_query(o: ObjectRef, name: String, window: &mut Window, cx: &mut App) {
    if !queries::name_ok(&o.scope, &name, Some(&o.name)) {
        show_error(t!("obj.query_name_invalid").to_string(), window, cx);
        return;
    }
    if let Err(e) = queries::rename(&o.scope, &o.name, &name) {
        show_error(e.to_string(), window, cx);
        return;
    }
    let to = ObjectRef { name, ..o.clone() };
    Workspace::global(cx).update(cx, |w, cx| {
        w.queries_changed(cx);
        w.select(Selection::Object(to.clone()), cx);
    });
    window.dispatch_action(Box::new(QueryRenamed { from: o, to }), cx);
}

fn duplicate_query(window: &mut Window, cx: &mut App) {
    let Some(o) = selected_queries(cx).into_iter().next() else {
        return;
    };
    match queries::duplicate(&o.scope, &o.name) {
        Ok(copy) => {
            let copy = ObjectRef { name: copy, ..o };
            Workspace::global(cx).update(cx, |w, cx| {
                w.queries_changed(cx);
                w.select(Selection::Object(copy), cx);
            });
        }
        Err(e) => show_error(e.to_string(), window, cx),
    }
}

fn delete(window: &mut Window, cx: &mut App) {
    let (files, objects): (Vec<ObjectRef>, Vec<ObjectRef>) = selected_objects(cx)
        .into_iter()
        .partition(|o| o.kind == ObjectKind::Query);
    if !files.is_empty() {
        delete_queries(files, window, cx);
        return;
    }
    let stmts: Vec<(ObjectRef, String)> = objects
        .into_iter()
        .filter_map(|o| {
            let d = dialect(&o, cx)?;
            let target = if o.kind == ObjectKind::Tablespace {
                d.quote_ident(&o.name)
            } else {
                d.qualified(&o.scope, &o.name)
            };
            let sql = format!("DROP {} {target}", d.drop_keyword(o.kind)?);
            Some((o, sql))
        })
        .collect();
    let message = match stmts.as_slice() {
        [] => return,
        [(o, _)] => t!("obj.confirm_drop", kind = kind_label(o.kind), name = o.name),
        _ => t!("obj.confirm_drop_many", n = stmts.len()),
    };
    confirm(message.to_string(), stmts, window, cx);
}

/// Empties (`DELETE FROM`) or truncates the selected tables; SQLite has no `TRUNCATE`.
fn empty(truncate: bool, window: &mut Window, cx: &mut App) {
    let stmts: Vec<(ObjectRef, String)> = selected_objects(cx)
        .into_iter()
        .filter(|o| o.kind == ObjectKind::Table)
        .filter_map(|o| {
            let d = dialect(&o, cx)?;
            let target = d.qualified(&o.scope, &o.name);
            if truncate && d.features().exposes_rowid {
                None
            } else if truncate {
                Some((o, format!("TRUNCATE TABLE {target}")))
            } else {
                Some((o, format!("DELETE FROM {target}")))
            }
        })
        .collect();
    let message = match (stmts.as_slice(), truncate) {
        ([], _) => return,
        ([(o, _)], false) => t!("obj.confirm_empty", name = o.name),
        ([(o, _)], true) => t!("obj.confirm_truncate", name = o.name),
        (_, false) => t!("obj.confirm_empty_many", n = stmts.len()),
        (_, true) => t!("obj.confirm_truncate_many", n = stmts.len()),
    };
    confirm(message.to_string(), stmts, window, cx);
}

fn confirm(message: String, stmts: Vec<(ObjectRef, String)>, window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        let stmts = stmts.clone();
        alert
            .icon(lucide("triangle-alert"))
            .title(message.clone())
            .show_cancel(true)
            .ok_text(t!("obj.ok").to_string())
            .cancel_text(t!("obj.cancel").to_string())
            .on_ok(move |_, window, cx| {
                run(stmts.clone(), None, window, cx);
                true
            })
    });
}

/// Runs each generated statement once on its own operation session, then reloads the affected
/// object lists and selects `then` on success. Metadata sessions remain eligible for recovery.
pub(crate) fn run(
    stmts: Vec<(ObjectRef, String)>,
    then: Option<Selection>,
    window: &mut Window,
    cx: &mut App,
) {
    let generations: Vec<_> = stmts
        .iter()
        .map(|(obj, _)| obj.scope.connection.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|id| {
            let generation = Sessions::global(cx).read(cx).generation(&id);
            (id, generation)
        })
        .collect();
    window
        .spawn(cx, async move |cx| {
            let mut error = None;
            for (obj, sql) in &stmts {
                let sql = sql.clone();
                let database = obj.scope.database.clone();
                let dialect = cx
                    .update(|_, cx| Sessions::global(cx).read(cx).dialect(&obj.scope.connection))
                    .ok()
                    .flatten();
                let job = cx
                    .update(|_, cx| {
                        if !connections_unchanged(&generations, cx) {
                            return None;
                        }
                        Some(Sessions::global(cx).read(cx).run_operation_rel(
                            &obj.scope.connection,
                            Some(&obj.scope.database),
                            move |r| {
                                Box::pin(async move {
                                    if let Some(dialect) = dialect {
                                        prepare_database(r, dialect.as_ref(), &database).await?;
                                    }
                                    r.execute(&sql).await.map(|_| ())
                                })
                            },
                        ))
                    })
                    .ok()
                    .flatten();
                // The command belongs to its original connections, including between statements.
                let Some(job) = job else { return };
                let Some(job) = job else {
                    continue;
                };
                let result = match job.await {
                    Ok(result) => result,
                    Err(error) => Err(DbError::msg(error.to_string())),
                };
                if let Err(e) = result {
                    error = Some((obj.scope.connection.clone(), e));
                    break;
                }
            }
            let _ = cx.update(|window, cx| {
                if !connections_unchanged(&generations, cx) {
                    return;
                }
                refresh(stmts.iter().map(|(o, _)| o), cx);
                match error {
                    Some((connection, error)) => show_database_error(error, connection, window, cx),
                    None => {
                        if let Some(sel) = then {
                            Workspace::global(cx).update(cx, |w, cx| w.select(sel, cx));
                        }
                    }
                }
            });
        })
        .detach();
}

fn connections_unchanged(generations: &[(String, u64)], cx: &App) -> bool {
    let sessions = Sessions::global(cx);
    let sessions = sessions.read(cx);
    generations
        .iter()
        .all(|(id, generation)| sessions.generation(id) == *generation)
}

fn refresh<'a>(objects: impl Iterator<Item = &'a ObjectRef>, cx: &mut App) {
    // One reload per object category: several objects of the same list share one query.
    let mut seen = HashSet::new();
    let requirements: Vec<_> = objects
        .filter(|o| seen.insert((o.scope.clone(), o.kind)))
        .map(|o| Requirement::Objects {
            scope: o.scope.clone(),
            kind: o.kind,
        })
        .collect();
    Catalog::global(cx).update(cx, |c, cx| c.reload_all(requirements, cx));
}

fn run_maintenance(action: &RunMaintenance, window: &mut Window, cx: &mut App) {
    let connection = action.connection.to_string();
    let database = action.database.to_string();
    let opened = database.clone();
    let sql = action.sql.to_string();
    let generation = Sessions::global(cx).read(cx).generation(&connection);
    let dialect = Sessions::global(cx).read(cx).dialect(&connection);
    let Some(job) =
        Sessions::global(cx)
            .read(cx)
            .run_operation_rel(&connection, Some(&opened), move |r| {
                Box::pin(async move {
                    if let Some(dialect) = dialect
                        && let Some(prep) = dialect.prepare_database_sql(&database)
                    {
                        r.execute(&prep).await?;
                    }
                    r.execute(&sql).await
                })
            })
    else {
        show_error(t!("nav.maintain_offline").to_string(), window, cx);
        return;
    };
    window
        .spawn(cx, async move |cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(error) => Err(DbError::msg(error.to_string())),
            };
            let _ = cx.update(|window, cx| {
                if Sessions::global(cx).read(cx).generation(&connection) != generation {
                    return;
                }
                match result {
                    Ok(sets) => show_maintenance(format_maintenance(sets), window, cx),
                    Err(error) => show_database_error(error, connection, window, cx),
                }
            });
        })
        .detach();
}

fn format_maintenance(sets: Vec<navidog_db::ResultSet>) -> String {
    let mut out = String::new();
    for set in sets {
        if set.rows.is_empty() {
            if let Some(rows) = set.affected_rows {
                out.push_str(&format!("({rows})\n"));
            }
            continue;
        }
        for row in set.rows {
            for (i, cell) in row.iter().enumerate() {
                if i > 0 {
                    out.push('\t');
                }
                out.push_str(cell.as_deref().unwrap_or("NULL"));
            }
            out.push('\n');
        }
    }
    if out.trim().is_empty() {
        t!("nav.maintain_done").to_string()
    } else {
        out
    }
}

fn show_maintenance(text: String, window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        alert
            .icon(lucide("check"))
            .title(t!("nav.maintain").to_string())
            .description(text.clone())
    });
}

fn show_error(message: String, window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        alert
            .icon(lucide("circle-x"))
            .title(t!("obj.failed").to_string())
            .description(message.clone())
    });
}

fn show_database_error(error: DbError, connection: String, window: &mut Window, cx: &mut App) {
    let message = crate::errors::database_message(&error);
    if !error.is_session_lost() {
        show_error(message, window, cx);
        return;
    }
    let focus = window.focused(cx);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let connection = connection.clone();
        let focus = focus.clone();
        alert
            .icon(lucide("circle-x"))
            .title(t!("obj.failed").to_string())
            .description(message.clone())
            .show_cancel(true)
            .cancel_text(t!("obj.close").to_string())
            .ok_text(t!("nav.reopen_connection").to_string())
            .on_ok(move |_, window, cx| {
                let id = connection.clone();
                let focus = focus.clone();
                // Let the error dialog close before showing the connection-loss confirmation.
                window.defer(cx, move |window, cx| {
                    if let Some(focus) = focus {
                        window.focus(&focus, cx);
                    }
                    window.dispatch_action(Box::new(ReopenConnectionId::new(id)), cx);
                });
                true
            })
    });
}

/// Copies the selected table to `<name>_copyN`, the first free name, and selects the copy.
fn duplicate(with_data: bool, window: &mut Window, cx: &mut App) {
    let Some(o) = selected_objects(cx)
        .into_iter()
        .find(|o| o.kind == ObjectKind::Table)
    else {
        return;
    };
    let Some(d) = dialect(&o, cx) else { return };
    let taken: HashSet<String> = match Catalog::global(cx)
        .read(cx)
        .objects(&o.scope, ObjectKind::Table)
    {
        Some(Load::Ready(list)) => list.iter().map(|t| t.name.clone()).collect(),
        _ => HashSet::new(),
    };
    let new = (1..)
        .map(|i| format!("{}_copy{i}", o.name))
        .find(|n| !taken.contains(n))
        .unwrap_or_default();
    let copy = ObjectRef {
        name: new.clone(),
        ..o.clone()
    };
    let (src, target) = (o.clone(), new);
    let generation = Sessions::global(cx)
        .read(cx)
        .generation(&o.scope.connection);
    let Some(job) = Sessions::global(cx).read(cx).run_operation_rel(
        &o.scope.connection,
        Some(&o.scope.database),
        move |r| {
            Box::pin(async move {
                prepare_database(r, d.as_ref(), &src.scope.database).await?;
                let ddl = if d.features().exposes_rowid {
                    Some(r.object_ddl(&src.scope, src.kind, &src.name).await?)
                } else {
                    None
                };
                let stmts = d
                    .duplicate_table(&src.scope, &src.name, &target, with_data, ddl.as_deref())
                    .ok_or_else(|| DbError::msg(t!("obj.definition_unavailable").to_string()))?;
                for sql in stmts {
                    r.execute(&sql).await?;
                }
                Ok(())
            })
        },
    ) else {
        return;
    };
    window
        .spawn(cx, async move |cx| {
            let result = match job.await {
                Ok(result) => result,
                Err(error) => Err(DbError::msg(error.to_string())),
            };
            let _ = cx.update(|window, cx| {
                if Sessions::global(cx)
                    .read(cx)
                    .generation(&copy.scope.connection)
                    != generation
                {
                    return;
                }
                refresh(std::iter::once(&copy), cx);
                match result {
                    Ok(()) => Workspace::global(cx)
                        .update(cx, |w, cx| w.select(Selection::Object(copy.clone()), cx)),
                    Err(error) => {
                        show_database_error(error, copy.scope.connection.clone(), window, cx)
                    }
                }
            });
        })
        .detach();
}

#[cfg(test)]
mod recovery_tests {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Context, FocusHandle, IntoElement, Render, Styled as _,
        TestAppContext, WindowBounds, WindowOptions, div, px, size,
    };

    use super::*;

    struct Host {
        focus: FocusHandle,
        reopened: Vec<String>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .track_focus(&self.focus)
                .on_action(cx.listener(|this, action: &ReopenConnectionId, _, _| {
                    this.reopened.push(action.id.to_string());
                }))
        }
    }

    #[gpui_kit::test]
    fn closing_before_a_queued_batch_starts_discards_its_work_and_callbacks(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            navidog_ui::theme::init(cx);
            navidog_session::init(cx);
        });
        let (handle, _) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| Host {
                    focus: cx.focus_handle(),
                    reopened: Vec::new(),
                })
            })
            .unwrap()
        });
        for reopen in [false, true] {
            cx.update_window(handle, |_, window, cx| {
                let object = ObjectRef {
                    scope: navidog_model::object::Scope::new("retired", "main", None),
                    kind: ObjectKind::Table,
                    name: "t".into(),
                };
                run(
                    vec![(object.clone(), "DELETE FROM t".into())],
                    Some(Selection::Object(object)),
                    window,
                    cx,
                );
                Sessions::global(cx).update(cx, |sessions, cx| {
                    if reopen {
                        sessions.close_for_reopen("retired", cx);
                    } else {
                        sessions.close("retired", cx);
                    }
                });
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(!window.has_active_dialog(cx));
                // No Catalog or Workspace is installed: touching either would fail this test.
                // A retired command must neither refresh objects nor publish its old selection.
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn session_loss_error_offers_reopen_for_the_original_connection_after_dismissal(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            navidog_ui::theme::init(cx);
            cx.set_reduce_motion(true);
        });
        let (handle, host) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        // raw-px: space for the localized error and recovery buttons.
                        size(px(900.), px(600.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|cx| Host {
                        focus: cx.focus_handle(),
                        reopened: Vec::new(),
                    })
                },
            )
            .unwrap()
        });
        for (button, expected) in [("cancel", 0), ("ok", 1)] {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.focus(&host.read(cx).focus.clone(), cx);
                let mut error = DbError::coded("08006", "original diagnostic");
                error.disconnected = true;
                show_database_error(error, "original-connection".into(), window, cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.has_active_dialog(cx));
                window.click(button, cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(!window.has_active_dialog(cx));
                let received = &host.read(cx).reopened;
                assert_eq!(received.len(), expected);
                if expected == 1 {
                    assert_eq!(received, &["original-connection"]);
                }
            })
            .unwrap();
        }
    }
}
