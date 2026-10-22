//! Operations on the selected objects (F-OBJ-008): delete, empty, truncate and duplicate.
//! The navigator tree, the objects tab and the menu bar dispatch the same actions; renaming is
//! edited in place in the navigator (`NavigatorPanel::start_rename_object`).

use std::collections::HashSet;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, InteractiveElement, Window};
use navidog_core::actions::{
    DeleteObjects, DuplicateQuery, DuplicateTable, EmptyTables, OpenQueryExternally, QueryRenamed,
    RevealQuery, TruncateTables,
};
use navidog_core::object::{ObjectKind, ObjectRef};
use navidog_core::queries;
use navidog_core::workspace::{Selection, Workspace};
use navidog_db::DbError;
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{Sessions, prepare_database};
use navidog_ui::assets::lucide;
use rust_i18n::t;

use crate::catalog::{Catalog, Load};

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

fn dialect(obj: &ObjectRef, cx: &App) -> Option<Dialect> {
    Sessions::global(cx).read(cx).dialect(&obj.scope.connection)
}

pub(crate) fn bind<E: InteractiveElement>(el: E) -> E {
    el.on_action(|_: &DeleteObjects, window, cx| delete(window, cx))
        .on_action(|_: &EmptyTables, window, cx| empty(false, window, cx))
        .on_action(|_: &TruncateTables, window, cx| empty(true, window, cx))
        .on_action(|a: &DuplicateTable, window, cx| duplicate(a.with_data, window, cx))
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

fn kind_label(kind: ObjectKind) -> String {
    t!(format!("kind.{}", kind.key())).to_string()
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
            let sql = format!(
                "DROP {} {}",
                d.drop_keyword(o.kind)?,
                d.qualified(&o.scope, &o.name)
            );
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
            match (truncate, d) {
                (true, Dialect::Sqlite) => None,
                (true, _) => Some((o, format!("TRUNCATE TABLE {target}"))),
                (false, _) => Some((o, format!("DELETE FROM {target}"))),
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

/// Runs the statements in order, each on a session opened for that statement, then reloads the
/// affected object lists and selects `then` on success. The first error is shown as the database
/// reported it. The shared metadata session is left untouched.
pub(crate) fn run(
    stmts: Vec<(ObjectRef, String)>,
    then: Option<Selection>,
    window: &mut Window,
    cx: &mut App,
) {
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
                        Sessions::global(cx).read(cx).run_once(
                            &obj.scope.connection,
                            Some(&obj.scope.database),
                            move |r| {
                                Box::pin(async move {
                                    if let Some(dialect) = dialect {
                                        prepare_database(r, dialect, &database).await?;
                                    }
                                    r.execute(&sql).await.map(|_| ())
                                })
                            },
                        )
                    })
                    .ok()
                    .flatten();
                let Some(job) = job else { continue };
                if let Err(e) = job
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| crate::db_text(&e)))
                {
                    error = Some(e);
                    break;
                }
            }
            let _ = cx.update(|window, cx| {
                refresh(stmts.iter().map(|(o, _)| o), cx);
                match error {
                    Some(e) => show_error(e, window, cx),
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

fn refresh<'a>(objects: impl Iterator<Item = &'a ObjectRef>, cx: &mut App) {
    let lists: HashSet<_> = objects.map(|o| (o.scope.clone(), o.kind)).collect();
    Catalog::global(cx).update(cx, |c, cx| {
        for (scope, kind) in lists {
            c.ensure_objects(&scope, kind, true, cx);
        }
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
    let Some(job) = Sessions::global(cx).read(cx).run_once(
        &o.scope.connection,
        Some(&o.scope.database),
        move |r| {
            Box::pin(async move {
                prepare_database(r, d, &src.scope.database).await?;
                let ddl = match d {
                    Dialect::Sqlite => Some(r.object_ddl(&src.scope, src.kind, &src.name).await?),
                    _ => None,
                };
                let stmts = d
                    .duplicate_table(&src.scope, &src.name, &target, with_data, ddl.as_deref())
                    .ok_or_else(|| DbError::msg("cannot read the table definition"))?;
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
            let result = job
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| crate::db_text(&e)));
            let _ = cx.update(|window, cx| {
                refresh(std::iter::once(&copy), cx);
                match result {
                    Ok(()) => Workspace::global(cx)
                        .update(cx, |w, cx| w.select(Selection::Object(copy.clone()), cx)),
                    Err(e) => show_error(e, window, cx),
                }
            });
        })
        .detach();
}
