//! Result grid: wraps `DataTable` with rectangular selection, in-place editing, change markers, and
//! copy/paste (21-table-viewer.md §1, §7).

use std::collections::{BTreeMap, HashSet};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::table::{
    Column, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui_kit::component::{Sizable as _, Size, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, Pixels, Render, SharedString, Styled as _, Subscription,
    TextAlign, WeakEntity, Window, actions, div, px,
};
use navidog_core::object::Scope;
use navidog_core::settings::{RowShade, Settings};
use navidog_db::dialect::Dialect;
use navidog_ui::assets::lucide;
use navidog_ui::skin::{confirm_with, truncated};
use navidog_ui::{palette, tokens};
use navidog_util::sql_type::{TypeKind, type_kind};
use rust_i18n::t;

use crate::model::{Cell, ColumnLayout, GridColumn, GridModel, Pos, RowState};

mod delegate;
mod edit;
mod menu;

use delegate::GridDelegate;

actions!(
    grid,
    [
        Copy,
        Paste,
        SelectAll,
        DeleteRows,
        StartEdit,
        SetNull,
        SetEmpty,
        ExtendUp,
        ExtendDown,
        ExtendLeft,
        ExtendRight,
        ToggleFieldType,
        ToggleFieldComment,
        SetRowHeight,
        ToggleRawData,
        ToggleRowId,
        PickForeignKey
    ]
);

/// Select every cell of one column (F-DATA-005). `index` is a data column.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct SelectColumn {
    pub index: usize,
}

/// Freeze the display prefix through this data column, without moving columns (F-DATA-009).
#[derive(Clone, PartialEq, Eq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct FreezeThrough {
    pub index: usize,
}

/// Move this data column to the end of the frozen prefix and freeze it (F-DATA-009).
#[derive(Clone, PartialEq, Eq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct FreezeColumn {
    pub index: usize,
}

/// Clear the freeze and undo the moves it caused (F-DATA-009).
#[derive(Clone, PartialEq, Eq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct UnfreezeAll;

/// Put columns back in the order the server returned, and clear the freeze.
#[derive(Clone, PartialEq, Eq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct RestoreColumnOrder;

/// Copy one column's name (the column-header "Copy Field Name" item).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct CopyFieldName {
    pub name: SharedString,
}

/// Hide or show one column by name (F-DATA-009). The viewer owns the hidden set.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct HideColumn {
    pub name: SharedString,
}

/// Right-click quick filter (F-DATA-006). `op` is `filter::Op::key`.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct QuickFilter {
    pub column: SharedString,
    pub op: SharedString,
    pub value: SharedString,
}

/// The "Copy As" submenu.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct CopyAs {
    pub format: SharedString,
}

const CONTEXT: &str = "ResultGrid";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-c", Copy, Some(CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(CONTEXT)),
        KeyBinding::new("cmd-a", SelectAll, Some(CONTEXT)),
        KeyBinding::new("cmd-backspace", DeleteRows, Some(CONTEXT)),
        KeyBinding::new("enter", StartEdit, Some(CONTEXT)),
        KeyBinding::new("shift-up", ExtendUp, Some(CONTEXT)),
        KeyBinding::new("shift-down", ExtendDown, Some(CONTEXT)),
        KeyBinding::new("shift-left", ExtendLeft, Some(CONTEXT)),
        KeyBinding::new("shift-right", ExtendRight, Some(CONTEXT)),
    ]);
}

/// The table behind the grid; "Copy As INSERT / UPDATE" is only available when it is set.
#[derive(Clone)]
pub struct GridTarget {
    pub dialect: Dialect,
    pub scope: Scope,
    pub table: String,
}

pub enum GridEvent {
    /// Header sort changed: column name and whether ascending; `None` clears the sort
    SortChanged(Option<(SharedString, bool)>),
    /// The change set changed (edit, insert, delete, paste)
    Changed,
    /// The current row changed and the row we left has uncommitted changes
    CommitRequested,
    /// The column of the focused cell, for the info pane's Fields page
    ColumnFocused(SharedString),
    /// Append a quick-filter rule and reload
    QuickFilter {
        column: SharedString,
        op: SharedString,
        value: SharedString,
    },
    /// Open the foreign-key value picker for the focused column
    PickForeignKey,
}

pub struct ResultGrid {
    table: Entity<TableState<GridDelegate>>,
    target: Option<GridTarget>,
    edit_sub: Option<Subscription>,
    row_height: tokens::Pt,
    /// Typed cell text is SQL, not a quoted string (F-DATA-010)
    raw: bool,
    /// The context menu shows a check while SQLite ROWID is visible
    pub show_rowid: bool,
    /// Local columns that reference another table (F-DATA-012)
    foreign_keys: Vec<String>,
    /// Query results have no WHERE pane, so the context menu skips quick filters.
    quick_filter: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<GridEvent> for ResultGrid {}

impl Focusable for ResultGrid {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.table.read(cx).focus_handle(cx)
    }
}

impl ResultGrid {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let delegate = GridDelegate {
            model: GridModel::default(),
            editing: None,
            sort: None,
            applied_sort: None,
            sort_changed: false,
            rem: window.rem_size(),
            show_type: true,
            show_comment: false,
            raw: false,
            dialect: Dialect::Sqlite,
            find: None,
            layout: ColumnLayout::default(),
            widths: BTreeMap::new(),
            auto: BTreeMap::new(),
            settled: None,
            dragged: false,
            selecting: false,
            arm_click_edit: false,
            commit_requested: false,
        };
        let table = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .cell_selectable(true)
                .row_header(false)
                .row_selectable(false)
                .col_selectable(false)
                .col_movable(true)
                .sortable(true)
        });
        let subs = vec![
            cx.subscribe_in(&table, window, Self::on_table_event),
            cx.observe(&table, |_, table, cx| {
                let (sort, commit) = table.update(cx, |t, _| {
                    let d = t.delegate_mut();
                    let sort = std::mem::take(&mut d.sort_changed).then(|| {
                        d.sort.and_then(|(c, asc)| {
                            d.model.columns.get(c).map(|col| (col.name.clone(), asc))
                        })
                    });
                    (sort, std::mem::take(&mut d.commit_requested))
                });
                if let Some(sort) = sort {
                    cx.emit(GridEvent::SortChanged(sort));
                }
                if commit {
                    cx.emit(GridEvent::CommitRequested);
                }
            }),
        ];
        Self {
            table,
            target: None,
            edit_sub: None,
            row_height: tokens::grid::ROW_HEIGHT,
            raw: false,
            show_rowid: false,
            foreign_keys: Vec::new(),
            quick_filter: true,
            _subscriptions: subs,
        }
    }

    /// Turns the context menu's quick-filter items on or off.
    pub fn set_quick_filter(&mut self, on: bool) {
        self.quick_filter = on;
    }

    /// Column name and ascending flag of the header arrow.
    pub fn header_sort(&self, cx: &App) -> Option<(SharedString, bool)> {
        let table = self.table.read(cx);
        let d = table.delegate();
        d.sort
            .and_then(|(ix, asc)| d.model.columns.get(ix).map(|c| (c.name.clone(), asc)))
    }

    /// Records the current header sort as the one the rows follow.
    pub fn accept_header_sort(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            let d = t.delegate_mut();
            d.applied_sort = d.sort;
            cx.notify();
        });
    }

    /// Puts the header arrow back on the last sort the rows actually follow.
    pub fn revert_header_sort(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            let d = t.delegate_mut();
            d.sort = d.applied_sort;
            // DataTable writes the new arrow onto its column groups before the delegate runs.
            // Rebuilding them from `column()` is what puts the old arrow back.
            t.refresh(cx);
            cx.notify();
        });
    }

    /// Same path as a header click: `None` clears the sort.
    pub fn request_sort(&mut self, column: Option<usize>, asc: bool, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            let d = t.delegate_mut();
            d.sort = column.map(|c| (c, asc));
            d.sort_changed = true;
            cx.notify();
        });
    }

    pub fn set_foreign_keys(&mut self, columns: Vec<String>, cx: &mut Context<Self>) {
        self.foreign_keys = columns;
        cx.notify();
    }

    pub fn set_target(&mut self, target: Option<GridTarget>, cx: &mut Context<Self>) {
        let dialect = target
            .as_ref()
            .map(|t| t.dialect)
            .unwrap_or(Dialect::Sqlite);
        self.target = target;
        let widths = self
            .target
            .as_ref()
            .map(|t| load_widths(t, cx))
            .unwrap_or_default();
        self.table.update(cx, |t, cx| {
            let d = t.delegate_mut();
            d.dialect = dialect;
            d.widths = widths;
            d.layout.clear();
            d.sort = None;
            t.refresh(cx);
        });
    }

    /// See `GridModel::locked`. A locked grid also does not open the in-place editor.
    pub fn set_locked(&mut self, locked: bool, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            t.delegate_mut().model.locked = locked;
            cx.notify();
        });
    }

    /// Raw data mode: string cells gain SQL quotes, and later edits are expressions.
    pub fn set_raw_mode(&mut self, raw: bool, cx: &mut Context<Self>) {
        self.raw = raw;
        self.table.update(cx, |t, cx| {
            t.delegate_mut().raw = raw;
            cx.notify();
        });
    }

    /// Replaces the data with a new page, discarding uncommitted changes.
    pub fn set_data(
        &mut self,
        columns: Vec<GridColumn>,
        rows: Vec<Vec<Cell>>,
        cx: &mut Context<Self>,
    ) {
        self.edit_sub = None;
        self.table.update(cx, |t, cx| {
            let d = t.delegate_mut();
            let sort_name = d
                .sort
                .and_then(|(i, asc)| d.model.columns.get(i).map(|c| (c.name.clone(), asc)));
            let names: Vec<_> = columns.iter().map(|c| c.name.clone()).collect();
            d.layout.remember_server(&names);
            d.layout.sync(&names);
            let desired = d.layout.display_names(&names);
            let order: Vec<usize> = desired
                .iter()
                .filter_map(|n| names.iter().position(|c| c == n))
                .collect();
            let columns: Vec<_> = order.iter().map(|&i| columns[i].clone()).collect();
            let rows: Vec<_> = rows
                .into_iter()
                .map(|row| {
                    order
                        .iter()
                        .map(|&i| row.get(i).cloned().flatten())
                        .collect()
                })
                .collect();
            d.editing = None;
            d.model.set_data(columns, rows);
            d.sort = sort_name.and_then(|(name, asc)| {
                d.model
                    .columns
                    .iter()
                    .position(|c| c.name == name)
                    .map(|i| (i, asc))
            });
            d.applied_sort = d.sort;
            d.settled = Some(d.model.head().0);
            d.dragged = false;
            d.commit_requested = false;
            refit_auto(d, cx);
            t.refresh(cx);
            cx.notify();
        });
    }

    pub fn model<'a>(&self, cx: &'a App) -> &'a GridModel {
        &self.table.read(cx).delegate().model
    }

    /// Changes which columns are shown using the model's current accepted rows.
    /// Returns false when pending writes must be committed or discarded first; nothing changes then.
    pub fn set_hidden_columns(&mut self, hidden: &HashSet<String>, cx: &mut Context<Self>) -> bool {
        let mut applied = false;
        self.table.update(cx, |t, cx| {
            let d = t.delegate_mut();
            // Resolving an editable target can clear the layout after data was loaded.
            // Remember the current order before hiding moves columns into the hidden partition.
            d.layout.sync(&d.names());
            let sort = d
                .sort
                .and_then(|(ix, asc)| d.model.columns.get(ix).map(|c| (c.name.clone(), asc)));
            if !d.model.set_hidden(hidden) {
                return;
            }
            applied = true;
            let names = d.names();
            let order: Vec<_> = d
                .layout
                .display_names(&names)
                .iter()
                .filter_map(|name| names.iter().position(|n| n == name))
                .collect();
            d.model.reorder(&order);
            d.sort = sort.and_then(|(name, asc)| {
                d.model
                    .columns
                    .iter()
                    .position(|c| c.name == name)
                    .map(|ix| (ix, asc))
            });
            d.applied_sort = d.sort;
            d.editing = None;
            d.settled = Some(d.model.head().0);
            refit_auto(d, cx);
            t.refresh(cx);
        });
        if applied {
            cx.notify();
        }
        applied
    }

    pub fn update_model(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut GridModel)) {
        self.table.update(cx, |t, cx| {
            f(&mut t.delegate_mut().model);
            cx.notify();
        });
        cx.emit(GridEvent::Changed);
    }

    /// Highlights cells that contain `query`. `None` clears the highlight.
    pub fn set_find(&mut self, query: Option<(SharedString, bool)>, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            t.delegate_mut().find = query;
            cx.notify();
        });
    }

    /// Selects a cell and scrolls it into view. Leaving a dirty row still commits.
    pub fn go_to(&mut self, pos: Pos, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            t.delegate_mut().model.select(pos, false);
            t.delegate_mut().finish_navigation(false);
            t.set_selected_cell(pos.0, pos.1 + 1, cx);
            cx.notify();
        });
        self.scroll_to_row(pos.0, cx);
    }

    /// Remembers resized columns by name and writes them for the open table.
    fn store_widths(&mut self, reported: &[Pixels], cx: &mut Context<Self>) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let names: Vec<SharedString> = self
            .model(cx)
            .columns
            .iter()
            .map(|c| c.name.clone())
            .collect();
        let mut changed = BTreeMap::new();
        self.table.update(cx, |t, _| {
            let shown: Vec<Pixels> = {
                let d = t.delegate();
                names.iter().map(|name| d.column_width(name)).collect()
            };
            changed = widths_the_drag_changed(&names, &shown, reported);
            let map = &mut t.delegate_mut().widths;
            for (name, width) in &changed {
                // raw-px: persisted column widths are already pixels
                map.insert(SharedString::from(name.as_str()), px(*width));
            }
        });
        if changed.is_empty() {
            return;
        }
        navidog_core::column_widths::store(&target.scope, &target.table, &changed);
    }

    pub fn scroll_to_row(&self, row: usize, cx: &mut App) {
        self.table.update(cx, |t, cx| t.scroll_to_row(row, cx));
    }

    /// Clears the sort shown in the header (e.g. after the ORDER BY text is cleared).
    pub fn clear_sort(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            t.delegate_mut().sort = None;
            t.refresh(cx);
        });
    }

    fn on_table_event(
        &mut self,
        _: &Entity<TableState<GridDelegate>>,
        e: &TableEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let TableEvent::ColumnWidthsChanged(widths) = e {
            self.store_widths(widths, cx);
            return;
        }
        match *e {
            TableEvent::SelectRow(r) if r < self.model(cx).row_count() => {
                // Same path as the marker column: the empty tail is not a cell, so SelectCell
                // never fires. Commit first, then select the whole row.
                self.commit_edit(window, cx);
                let shift = window.modifiers().shift;
                let col = self.table.update(cx, |t, cx| {
                    let d = t.delegate_mut();
                    // A press on a data cell may have armed click-to-edit. This click is the
                    // empty tail, and the SelectCell below must not open the editor.
                    d.arm_click_edit = false;
                    d.model.select_rows(r, shift);
                    d.finish_navigation(shift);
                    let col = d.model.head().1;
                    cx.notify();
                    col
                });
                // Stay in cell mode on the last field. SelectCell sees head already there and
                // leaves the row selection. That column is just left of the click, so the
                // non-strict scroll does not move the viewport.
                if !self.model(cx).columns.is_empty() {
                    self.table
                        .update(cx, |t, cx| t.set_selected_cell(r, col + 1, cx));
                }
            }
            TableEvent::SelectCell(r, ix) => {
                let real = self.model(cx).row_count();
                if r >= real {
                    if real > 0 {
                        self.table
                            .update(cx, |t, cx| t.set_selected_cell(real - 1, ix, cx));
                    }
                    return;
                }
                let editing_here = ix > 0
                    && self
                        .table
                        .read(cx)
                        .delegate()
                        .editing
                        .as_ref()
                        .is_some_and(|(pos, _)| *pos == (r, ix - 1));
                // Clicking the cell being edited moves the caret; it does not commit.
                if editing_here {
                    self.table
                        .update(cx, |t, _| t.delegate_mut().arm_click_edit = false);
                    return;
                }
                self.commit_edit(window, cx);
                let shift = window.modifiers().shift;
                let (arm, committing) = self.table.update(cx, |t, cx| {
                    let m = &mut t.delegate_mut().model;
                    match ix.checked_sub(1) {
                        None => m.select_rows(r, shift),
                        // Mouse down or Extend* has already updated the selection
                        Some(c) if m.head() == (r, c) => {}
                        Some(c) => m.select((r, c), shift),
                    }
                    let d = t.delegate_mut();
                    d.finish_navigation(shift);
                    let arm = std::mem::take(&mut d.arm_click_edit);
                    let committing = d.commit_requested;
                    cx.notify();
                    (arm, committing)
                });
                self.focus_column(cx);
                // A click that leaves a dirty row commits first; editing waits for the reload.
                if arm && ix > 0 && !committing {
                    self.start_edit((r, ix - 1), window, cx);
                }
            }
            TableEvent::DoubleClickedCell(r, ix) if ix > 0 && r < self.model(cx).row_count() => {
                self.start_edit((r, ix - 1), window, cx)
            }
            TableEvent::RightClickedCell(r, ix) if r < self.model(cx).row_count() => {
                if !self.model(cx).is_selected(r, ix.saturating_sub(1)) {
                    self.table.update(cx, |t, _| match ix.checked_sub(1) {
                        None => t.delegate_mut().model.select_rows(r, false),
                        Some(c) => t.delegate_mut().model.select((r, c), false),
                    });
                }
                // Same as the column menu: focus first, or the item is a no-op.
                self.focus_table(window, cx);
                self.context_menu(cx)
                    .show(window.mouse_position(), window, cx);
            }
            _ => {}
        }
    }

    /// Marks the current row as settled without committing, so a just-inserted row is the one
    /// that commits when the user leaves it.
    pub fn settle(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |t, _| {
            t.delegate_mut().settled = Some(t.delegate().model.head().0);
            t.delegate_mut().commit_requested = false;
        });
    }

    /// Moves the current row. Leaving a dirty row asks the viewer to commit.
    pub fn move_to_row(&mut self, row: usize, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            let c = t.delegate().model.head().1;
            t.delegate_mut().model.select((row, c), false);
            t.delegate_mut().finish_navigation(false);
            cx.notify();
        });
    }

    fn extend(&mut self, dr: isize, dc: isize, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            let m = &mut t.delegate_mut().model;
            let (r, c) = m.head();
            let max = (
                m.row_count().saturating_sub(1),
                m.columns.len().saturating_sub(1),
            );
            let next = (
                r.saturating_add_signed(dr).min(max.0),
                c.saturating_add_signed(dc).min(max.1),
            );
            m.select(next, true);
            t.set_selected_cell(next.0, next.1 + 1, cx);
        });
        self.focus_column(cx);
    }

    fn focus_column(&mut self, cx: &mut Context<Self>) {
        let name = {
            let m = self.model(cx);
            let c = m.head().1;
            m.columns.get(c).map(|col| col.name.clone())
        };
        if let Some(name) = name {
            cx.emit(GridEvent::ColumnFocused(name));
        }
    }
}

impl Render for ResultGrid {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rem = self.table.read(cx).delegate().rem;
        let show_type = self.table.read(cx).delegate().show_type;
        let row = self.row_height.to_pixels(rem);
        let font_pt = grid_font_pt(cx);
        let alternate = row_shade(cx) == RowShade::Alternate;
        // The dock clips overflow. Without a zero minimum the grid grows to the
        // sum of its columns, so this scrollbar has nothing to scroll.
        div()
            .key_context(CONTEXT)
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .when_some(font_pt, |d, pt| d.text_size(tokens::Pt(pt).rems()))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(this.copy_text("tsv", cx)))
            }))
            .on_action(cx.listener(|this, a: &CopyAs, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(this.copy_text(&a.format, cx)))
            }))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    let raw = this.raw;
                    this.update_model(cx, |m| m.paste(&text, raw));
                }
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.update_model(cx, GridModel::select_all)
            }))
            .on_action(cx.listener(|this, _: &DeleteRows, _, cx| {
                this.update_model(cx, GridModel::delete_selected_rows)
            }))
            .on_action(cx.listener(|this, _: &SetNull, _, cx| this.set_selected(None, cx)))
            .on_action(
                cx.listener(|this, _: &SetEmpty, _, cx| this.set_selected(Some("".into()), cx)),
            )
            .on_action(cx.listener(|this, _: &StartEdit, window, cx| {
                let head = this.model(cx).head();
                if this.model(cx).row_count() > 0 && !this.model(cx).columns.is_empty() {
                    this.start_edit(head, window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &SetRowHeight, window, cx| this.set_row_height(window, cx)),
            )
            // Both are only sent from the column menu. On macOS that menu calls
            // Window::refresh after dispatch, which re-renders every view, so neither
            // handler notifies. Header height is applied when this view renders; the
            // comment is read when the table renders.
            .on_action(cx.listener(|this, _: &ToggleFieldType, _, cx| {
                this.table.update(cx, |t, cx| {
                    t.delegate_mut().show_type = !t.delegate().show_type;
                    t.refresh(cx);
                });
            }))
            .on_action(cx.listener(|this, _: &ToggleFieldComment, _, cx| {
                this.table.update(cx, |t, _| {
                    t.delegate_mut().show_comment = !t.delegate().show_comment;
                });
            }))
            .on_action(cx.listener(|this, a: &SelectColumn, _, cx| {
                this.update_model(cx, |m| m.select_column(a.index, false));
                this.focus_column(cx);
            }))
            .on_action(cx.listener(|_, a: &CopyFieldName, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(a.name.to_string()));
            }))
            .on_action(cx.listener(|this, a: &FreezeThrough, _, cx| {
                this.table.update(cx, |t, cx| {
                    let d = t.delegate_mut();
                    let present = d.names();
                    d.layout.freeze_through(&present, a.index);
                    t.refresh(cx);
                });
            }))
            .on_action(cx.listener(|this, a: &FreezeColumn, _, cx| {
                this.table.update(cx, |t, cx| {
                    let d = t.delegate_mut();
                    let Some(name) = d.model.columns.get(a.index).map(|c| c.name.clone()) else {
                        return;
                    };
                    let present = d.names();
                    if d.layout.freeze_column(&present, &name) {
                        let desired = d.layout.display_names(&present);
                        d.apply_display(&desired);
                    }
                    t.refresh(cx);
                });
            }))
            .on_action(cx.listener(|this, _: &UnfreezeAll, _, cx| {
                this.table.update(cx, |t, cx| {
                    let d = t.delegate_mut();
                    d.layout.unfreeze_all();
                    let present = d.names();
                    let desired = d.layout.display_names(&present);
                    d.apply_display(&desired);
                    t.refresh(cx);
                });
            }))
            .on_action(cx.listener(|this, _: &RestoreColumnOrder, _, cx| {
                this.table.update(cx, |t, cx| {
                    let d = t.delegate_mut();
                    d.layout.restore_server();
                    let present = d.names();
                    let desired = d.layout.display_names(&present);
                    d.apply_display(&desired);
                    t.refresh(cx);
                });
            }))
            .on_action(cx.listener(|_, _: &PickForeignKey, _, cx| {
                cx.emit(GridEvent::PickForeignKey);
            }))
            .on_action(cx.listener(|_, a: &QuickFilter, _, cx| {
                cx.emit(GridEvent::QuickFilter {
                    column: a.column.clone(),
                    op: a.op.clone(),
                    value: a.value.clone(),
                });
            }))
            .on_action(cx.listener(|this, _: &Escape, window, cx| this.cancel_edit(window, cx)))
            .on_action(cx.listener(|this, _: &ExtendUp, _, cx| this.extend(-1, 0, cx)))
            .on_action(cx.listener(|this, _: &ExtendDown, _, cx| this.extend(1, 0, cx)))
            .on_action(cx.listener(|this, _: &ExtendLeft, _, cx| this.extend(0, -1, cx)))
            .on_action(cx.listener(|this, _: &ExtendRight, _, cx| this.extend(0, 1, cx)))
            .child(
                DataTable::new(&self.table)
                    .with_size(Size::Size(row))
                    .when(show_type, |table| {
                        table.header_row_height(tokens::grid::TYPE_HEADER_HEIGHT.to_pixels(rem))
                    })
                    .stripe(alternate)
                    .bordered(false),
            )
    }
}

fn row_shade(cx: &App) -> RowShade {
    if cx.has_global::<Settings>() {
        Settings::global(cx).row_shade
    } else {
        RowShade::Alternate
    }
}

/// Grid font: the preference, or the interface size when "use default font" is on.
fn grid_text_size(cx: &App) -> gpui_kit::Rems {
    tokens::Pt(grid_font_pt(cx).unwrap_or(tokens::font::SIZE_BODY.value())).rems()
}

/// Point size when the grid is not using the interface font. `None` keeps the default.
fn grid_font_pt(cx: &App) -> Option<f32> {
    if !cx.has_global::<Settings>() {
        return None;
    }
    let s = Settings::global(cx);
    // settings.json is editable by hand; keep a tiny or huge size from breaking the layout
    let range = Settings::GRID_FONT_PT;
    (!s.use_default_grid_font).then_some(s.grid_font_pt.clamp(*range.start(), *range.end()))
}

fn load_widths(target: &GridTarget, _: &App) -> BTreeMap<SharedString, Pixels> {
    navidog_core::column_widths::load(&target.scope, &target.table)
        .into_iter()
        .map(|(name, width)| {
            // raw-px: persisted column widths are already pixels
            (SharedString::from(name), px(width))
        })
        .collect()
}

/// How many leading rows to measure. A table page is already bounded; a query result can be the
/// whole set, so stop here. ponytail: a longer value past this row stays clipped until a resize
/// or the next page; measuring every row is the upgrade.
const FIT_ROWS: usize = 64;
/// Characters of painted cell text that count toward the fit.
const FIT_CHARS: usize = 64;
/// Latin glyphs are about this many em wide. A non-ASCII character counts as one em.
const LATIN_EM: f32 = 0.62;
/// A reported width within this many pixels of the one on screen was not dragged.
const DRAG_EPSILON: f32 = 1.;

fn char_units(c: char) -> f32 {
    if c.is_ascii() { 1. } else { 1. / LATIN_EM }
}

/// Units of the text a cell paints: newlines become spaces, then at most [`FIT_CHARS`].
fn painted_units(s: &str) -> f32 {
    s.chars()
        .take(FIT_CHARS)
        .map(|c| char_units(if c == '\n' || c == '\r' { ' ' } else { c }))
        .sum()
}

/// Width that fits `header` (plus the sort icon) and `values`, in the grid font.
/// `em` is that font size in pixels.
pub fn auto_width<'a>(header: &str, values: impl IntoIterator<Item = &'a str>, em: f32) -> Pixels {
    let mut units = header.chars().map(char_units).sum::<f32>() + 3.;
    for value in values {
        units = units.max(painted_units(value));
    }
    // raw-px: padding, sort icon, floor and ceiling are a character estimate, not a nib
    const PAD: f32 = 26.;
    const MIN: f32 = 56.;
    const MAX: f32 = 460.;
    px((units * em * LATIN_EM + PAD).clamp(MIN, MAX)) // raw-px: same estimate as the constants above
}

fn grid_em(rem: Pixels, cx: &App) -> Pixels {
    let pt = grid_font_pt(cx).unwrap_or(tokens::font::SIZE_BODY.value());
    tokens::Pt(pt).to_pixels(rem)
}

/// Text the cell paints, so the fit matches the grid rather than the stored value.
fn cell_text(model: &GridModel, r: usize, c: usize, raw: bool, dialect: Dialect) -> String {
    match model.value(r, c) {
        Some(Some(_)) => model.shown_text(r, c, raw, dialect).to_string(),
        Some(None) => "(NULL)".to_string(),
        None => String::new(),
    }
}

/// Columns the user has not resized, fitted to the header and the first loaded rows.
fn refit_auto(d: &mut GridDelegate, cx: &App) {
    let em = grid_em(d.rem, cx).as_f32();
    let rows = d.model.row_count().min(FIT_ROWS);
    let raw = d.raw;
    let dialect = d.dialect;
    let columns: Vec<(usize, SharedString)> = d
        .model
        .columns
        .iter()
        .enumerate()
        .map(|(c, col)| (c, col.name.clone()))
        .collect();
    let mut auto = BTreeMap::new();
    for (c, name) in columns {
        if d.widths.contains_key(&name) {
            continue;
        }
        let texts: Vec<String> = (0..rows)
            .map(|r| cell_text(&d.model, r, c, raw, dialect))
            .collect();
        auto.insert(
            name.clone(),
            auto_width(&name, texts.iter().map(String::as_str), em),
        );
    }
    d.auto = auto;
}

/// `reported` includes the marker column at index 0. Only columns that moved are returned.
fn widths_the_drag_changed(
    names: &[SharedString],
    shown: &[Pixels],
    reported: &[Pixels],
) -> BTreeMap<String, f32> {
    names
        .iter()
        .zip(shown)
        .zip(reported.iter().skip(1))
        .filter(|((_, shown), next)| (shown.as_f32() - next.as_f32()).abs() >= DRAG_EPSILON)
        .map(|((name, _), next)| (name.to_string(), next.as_f32()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_width_tracks_the_longest_text_and_stays_bounded() {
        let narrow = auto_width("id", ["1", "22"], 13.);
        let wide = auto_width("email", ["someone.long@example.com"], 13.);
        assert!(narrow < wide);
        assert!(narrow.as_f32() >= 56.);
        let huge = "x".repeat(500);
        assert!(auto_width("t", [huge.as_str()], 13.).as_f32() <= 460.);
        // A CJK character is about one em, not 0.62.
        let ascii = auto_width("n", ["abcd"], 13.);
        let cjk = auto_width("n", ["中文中文"], 13.);
        assert!(cjk > ascii);
    }

    #[test]
    fn a_drag_keeps_columns_that_did_not_move() {
        let names = ["id", "name"].map(SharedString::from);
        // raw-px: arbitrary widths; the first reported width is the marker column
        let shown = [px(80.), px(140.)];
        let reported = [px(20.), px(80.), px(200.)]; // raw-px: marker column plus the two shown widths
        let changed = widths_the_drag_changed(&names, &shown, &reported);
        assert_eq!(changed.get("id"), None);
        assert_eq!(changed.get("name"), Some(&200.));
        assert_eq!(changed.len(), 1);
    }
}
