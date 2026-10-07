//! Result grid: wraps `DataTable` with rectangular selection, in-place editing, change markers, and
//! copy/paste (21-table-viewer.md §1, §7).

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
    WeakEntity, Window, actions, div,
};
use navidog_core::object::Scope;
use navidog_db::dialect::Dialect;
use navidog_ui::skin::truncated;
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::model::{Cell, GridColumn, GridModel, Pos, RowState};

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
        ToggleRawData
    ]
);

/// Select every cell of one column (F-DATA-005). `index` is a data column.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct SelectColumn {
    pub index: usize,
}

/// Hide or show one column by name (F-DATA-009). The viewer owns the hidden set.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct HideColumn {
    pub name: SharedString,
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
}

pub struct GridDelegate {
    pub model: GridModel,
    editing: Option<(Pos, Entity<InputState>)>,
    sort: Option<(usize, bool)>,
    sort_changed: bool,
    rem: Pixels,
    /// Show the field type or comment in the column header (F-DATA-013)
    show_type: bool,
    show_comment: bool,
    /// The row a plain click or key move settled on. Leaving it while it is dirty commits.
    settled: Option<usize>,
    /// The pointer dragged the selection, so the mouse-up (not SelectCell) settles the row.
    dragged: bool,
    commit_requested: bool,
}

impl GridDelegate {
    /// A plain navigation to another row commits when the row we left is dirty (F-DATA-003).
    /// Shift-extend keeps the settled row, so a range selection does not commit halfway.
    fn finish_navigation(&mut self, shift: bool) {
        if shift {
            return;
        }
        let row = self.model.head().0;
        let prev = self.settled.replace(row);
        if prev.is_some_and(|p| p != row && self.model.row_dirty(p)) {
            self.commit_requested = true;
        }
    }
}

impl GridDelegate {
    fn width(&self, pt: tokens::Pt) -> Pixels {
        pt.to_pixels(self.rem)
    }
}

impl TableDelegate for GridDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.model.columns.len() + 1
    }

    fn rows_count(&self, _: &App) -> usize {
        self.model.row_count()
    }

    fn column(&self, ix: usize, _: &App) -> Column {
        let Some(col) = ix.checked_sub(1).and_then(|c| self.model.columns.get(c)) else {
            return Column::new("#", "")
                .width(self.width(tokens::grid::ROW_NUMBER_WIDTH))
                .fixed_left()
                .resizable(false)
                .movable(false)
                .selectable(false)
                .p_0();
        };
        let sort = match self.sort {
            Some((c, true)) if c + 1 == ix => ColumnSort::Ascending,
            Some((c, false)) if c + 1 == ix => ColumnSort::Descending,
            _ => ColumnSort::Default,
        };
        Column::new(SharedString::from(ix.to_string()), col.name.clone())
            .width(self.width(tokens::grid::COLUMN_WIDTH))
            .sort(sort)
            .movable(false)
            .p_0()
    }

    fn perform_sort(
        &mut self,
        ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        self.sort = match sort {
            ColumnSort::Ascending => Some((ix - 1, true)),
            ColumnSort::Descending => Some((ix - 1, false)),
            ColumnSort::Default => None,
        };
        self.sort_changed = true;
    }

    fn render_th(
        &mut self,
        ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let col = ix.checked_sub(1).and_then(|c| self.model.columns.get(c));
        let (show_type, show_comment) = (self.show_type, self.show_comment);
        let header = col.map(|c| {
            let mut label = c.name.to_string();
            if show_type && !c.type_name.is_empty() {
                label = format!("{label}  {}", c.type_name);
            }
            if show_comment && let Some(comment) = &c.comment {
                label = format!("{label}  {comment}");
            }
            (ix - 1, c.name.clone(), label)
        });
        div()
            .size_full()
            .px_1()
            .flex()
            .items_center()
            .overflow_hidden()
            .text_color(palette(cx).label)
            .when_some(header, |d, (c, name, label)| {
                d.child(truncated(("grid-th", ix), label))
                    // A click selects the column; the sort still runs because this does not stop it
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |t, e: &MouseDownEvent, _, cx| {
                            let shift = e.modifiers.shift;
                            t.delegate_mut().model.select_column(c, shift);
                            t.delegate_mut().finish_navigation(shift);
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |_t, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            let name = name.clone();
                            NativeMenu::new()
                                .menu(
                                    t!("grid.select_column").to_string(),
                                    Box::new(SelectColumn { index: c }),
                                )
                                .menu_with_check(
                                    t!("grid.show_type").to_string(),
                                    show_type,
                                    Box::new(ToggleFieldType),
                                )
                                .menu_with_check(
                                    t!("grid.show_comment").to_string(),
                                    show_comment,
                                    Box::new(ToggleFieldComment),
                                )
                                .separator()
                                .menu(
                                    t!("grid.hide_column").to_string(),
                                    Box::new(HideColumn { name }),
                                )
                                .show(e.position, window, cx);
                        }),
                    )
            })
    }

    fn render_td(
        &mut self,
        r: usize,
        ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let p = palette(cx);
        let state = self.model.row_state(r);
        let Some(c) = ix.checked_sub(1) else {
            let mark = match state {
                RowState::Inserted => "*",
                RowState::Edited => "✎",
                RowState::Deleted => "✕",
                RowState::Clean if self.model.head().0 == r => "▸",
                RowState::Clean => "",
            };
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(p.label_secondary)
                .child(mark)
                .into_any_element();
        };
        if let Some((pos, input)) = &self.editing
            && *pos == (r, c)
        {
            return Input::new(input)
                .xsmall()
                .appearance(false)
                .size_full()
                .into_any_element();
        }
        let selected = self.model.is_selected(r, c);
        let (text, muted): (SharedString, bool) = match self.model.value(r, c) {
            // A cell shows one line, like Navicat; the cell editor shows the full value
            Some(Some(s)) if s.contains('\n') => (s.replace(['\r', '\n'], " ").into(), false),
            Some(Some(s)) => (s.clone(), false),
            Some(None) => ("(NULL)".into(), true),
            None => ("".into(), true),
        };
        div()
            .size_full()
            .px_1()
            .flex()
            .items_center()
            .overflow_hidden()
            .when(state == RowState::Deleted, |d| d.bg(p.grid_deleted))
            .when(self.model.is_edited(r, c), |d| d.bg(p.grid_edited))
            .when(muted, |d| d.text_color(p.label_tertiary))
            .when(selected, |d| d.bg(p.selection).text_color(p.selection_text))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |t, e: &MouseDownEvent, _, cx| {
                    t.delegate_mut().model.select((r, c), e.modifiers.shift);
                    cx.notify();
                }),
            )
            // Dragging extends the selection; DataTable only emits SelectCell when press and release
            // land on the same cell
            .on_mouse_move(cx.listener(move |t, e: &MouseMoveEvent, _, cx| {
                if e.dragging() && t.delegate().model.head() != (r, c) {
                    t.delegate_mut().dragged = true;
                    t.delegate_mut().model.select((r, c), true);
                    cx.notify();
                }
            }))
            // A drag ends here: SelectCell only fires when the press and the release are one cell
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |t, _: &MouseUpEvent, _, cx| {
                    let d = t.delegate_mut();
                    if !std::mem::take(&mut d.dragged) {
                        return;
                    }
                    d.finish_navigation(false);
                    cx.notify();
                }),
            )
            // No hover tooltip per cell: a hitbox per visible cell costs scrolling time
            .child(div().min_w_0().truncate().child(text))
            .into_any_element()
    }

    fn cell_text(&self, r: usize, ix: usize, _: &App) -> String {
        ix.checked_sub(1)
            .and_then(|c| self.model.value(r, c).cloned().flatten())
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
}

pub struct ResultGrid {
    table: Entity<TableState<GridDelegate>>,
    target: Option<GridTarget>,
    edit_sub: Option<Subscription>,
    row_height: tokens::Pt,
    /// Typed cell text is SQL, not a quoted string (F-DATA-010)
    raw: bool,
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
            sort_changed: false,
            rem: window.rem_size(),
            show_type: false,
            show_comment: false,
            settled: None,
            dragged: false,
            commit_requested: false,
        };
        let table = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .cell_selectable(true)
                .row_header(false)
                .row_selectable(false)
                .col_selectable(false)
                .col_movable(false)
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
            _subscriptions: subs,
        }
    }

    pub fn set_target(&mut self, target: Option<GridTarget>) {
        self.target = target;
    }

    /// Raw data mode: later edits are SQL expressions. Values already typed keep how they were written.
    pub fn set_raw_mode(&mut self, raw: bool) {
        self.raw = raw;
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
            d.editing = None;
            d.model.set_data(columns, rows);
            d.settled = Some(d.model.head().0);
            d.dragged = false;
            d.commit_requested = false;
            t.refresh(cx);
            cx.notify();
        });
    }

    pub fn model<'a>(&self, cx: &'a App) -> &'a GridModel {
        &self.table.read(cx).delegate().model
    }

    pub fn update_model(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut GridModel)) {
        self.table.update(cx, |t, cx| {
            f(&mut t.delegate_mut().model);
            cx.notify();
        });
        cx.emit(GridEvent::Changed);
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
        match *e {
            TableEvent::SelectCell(r, ix) => {
                self.commit_edit(window, cx);
                let shift = window.modifiers().shift;
                self.table.update(cx, |t, cx| {
                    let m = &mut t.delegate_mut().model;
                    match ix.checked_sub(1) {
                        None => m.select_rows(r, shift),
                        // Mouse down or Extend* has already updated the selection
                        Some(c) if m.head() == (r, c) => {}
                        Some(c) => m.select((r, c), shift),
                    }
                    t.delegate_mut().finish_navigation(shift);
                    cx.notify();
                });
            }
            TableEvent::DoubleClickedCell(r, ix) if ix > 0 => {
                self.start_edit((r, ix - 1), window, cx)
            }
            TableEvent::RightClickedCell(r, ix) => {
                if !self.model(cx).is_selected(r, ix.saturating_sub(1)) {
                    self.table.update(cx, |t, _| match ix.checked_sub(1) {
                        None => t.delegate_mut().model.select_rows(r, false),
                        Some(c) => t.delegate_mut().model.select((r, c), false),
                    });
                }
                self.context_menu()
                    .show(window.mouse_position(), window, cx);
            }
            _ => {}
        }
    }

    fn context_menu(&self) -> NativeMenu {
        let copy_as = |key: &str| {
            Box::new(CopyAs {
                format: key.to_string().into(),
            })
        };
        let mut sub = NativeMenu::new();
        if self.target.is_some() {
            sub = sub
                .menu(t!("grid.copy_insert").to_string(), copy_as("insert"))
                .menu(t!("grid.copy_update").to_string(), copy_as("update"))
                .separator();
        }
        let sub = sub
            .menu(t!("grid.copy_tsv_data").to_string(), copy_as("tsv"))
            .menu(t!("grid.copy_tsv_names").to_string(), copy_as("names"))
            .menu(t!("grid.copy_tsv_both").to_string(), copy_as("both"));
        NativeMenu::new()
            .menu(t!("grid.set_empty").to_string(), Box::new(SetEmpty))
            .menu(t!("grid.set_null").to_string(), Box::new(SetNull))
            .separator()
            .menu(t!("grid.copy").to_string(), Box::new(Copy))
            .submenu(t!("grid.copy_as").to_string(), sub)
            .menu(t!("grid.paste").to_string(), Box::new(Paste))
            .separator()
            .menu(t!("grid.delete_rows").to_string(), Box::new(DeleteRows))
            .separator()
            .menu(t!("grid.row_height").to_string(), Box::new(SetRowHeight))
            .separator()
            .menu_with_check(
                t!("grid.raw_data").to_string(),
                self.raw,
                Box::new(ToggleRawData),
            )
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

    fn set_row_height(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.row_height.0.to_string();
        let grid = cx.entity().downgrade();
        let view = cx.new(|cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
            input.update(cx, |s, cx| s.focus(window, cx));
            RowHeight { input, grid }
        });
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
                .close_button(false)
                .child(view.clone())
        });
    }

    pub fn start_edit(&mut self, pos: Pos, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_edit(window, cx);
        let text = self
            .model(cx)
            .value(pos.0, pos.1)
            .cloned()
            .flatten()
            .unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
        self.edit_sub = Some(cx.subscribe_in(
            &input,
            window,
            |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::PressEnter { .. } => this.commit_edit(window, cx),
                InputEvent::Blur => this.commit_edit(window, cx),
                _ => {}
            },
        ));
        input.update(cx, |s, cx| s.focus(window, cx));
        self.table.update(cx, |t, cx| {
            t.delegate_mut().editing = Some((pos, input));
            cx.notify();
        });
    }

    /// Ends in-place editing and writes back to the model; an unchanged value is not a change (a NULL
    /// that is still an empty string after editing stays NULL).
    fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(((r, c), input)) = self
            .table
            .update(cx, |t, _| t.delegate_mut().editing.take())
        else {
            return;
        };
        self.edit_sub = None;
        let text = input.read(cx).value().to_string();
        let old = self
            .model(cx)
            .value(r, c)
            .cloned()
            .flatten()
            .unwrap_or_default();
        if text != old.as_ref() {
            let raw = self.raw;
            self.update_model(cx, |m| {
                let cell = Some(text.into());
                if raw {
                    m.set_raw(r, c, cell);
                } else {
                    m.set_value(r, c, cell);
                }
            });
        }
        self.focus_table(window, cx);
    }

    fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .table
            .update(cx, |t, _| t.delegate_mut().editing.take())
            .is_some()
        {
            self.edit_sub = None;
            self.focus_table(window, cx);
        } else {
            cx.propagate();
        }
    }

    fn focus_table(&self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.table.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.table.update(cx, |_, cx| cx.notify());
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
    }

    fn copy_text(&self, format: &str, cx: &App) -> String {
        let m = self.model(cx);
        match (format, &self.target) {
            ("insert", Some(t)) => t.dialect.insert_statements(
                &t.scope,
                &t.table,
                &m.selected_columns(),
                &m.selected_values(),
            ),
            ("update", Some(t)) => {
                let keys: Vec<String> = m
                    .columns
                    .iter()
                    .filter(|c| c.primary_key)
                    .map(|c| c.name.to_string())
                    .collect();
                t.dialect.update_statements(
                    &t.scope,
                    &t.table,
                    &m.selected_columns(),
                    &keys,
                    &m.selected_values(),
                )
            }
            ("names", _) => m.selected_tsv(true, false),
            ("both", _) => m.selected_tsv(true, true),
            _ => m.selected_tsv(false, true),
        }
    }

    fn set_selected(&mut self, v: Cell, cx: &mut Context<Self>) {
        self.update_model(cx, |m| {
            let (rows, cols) = m.selection();
            for r in rows {
                if m.row_state(r) == RowState::Deleted {
                    continue;
                }
                for c in cols.clone() {
                    m.set_value(r, c, v.clone());
                }
            }
        });
    }
}

impl Render for ResultGrid {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = self
            .row_height
            .to_pixels(self.table.read(cx).delegate().rem);
        div()
            .key_context(CONTEXT)
            .size_full()
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
            .on_action(cx.listener(|this, _: &ToggleFieldType, _, cx| {
                this.table.update(cx, |t, cx| {
                    t.delegate_mut().show_type = !t.delegate().show_type;
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|this, _: &ToggleFieldComment, _, cx| {
                this.table.update(cx, |t, cx| {
                    t.delegate_mut().show_comment = !t.delegate().show_comment;
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|this, a: &SelectColumn, _, cx| {
                this.update_model(cx, |m| m.select_column(a.index, false));
            }))
            .on_action(cx.listener(|this, _: &Escape, window, cx| this.cancel_edit(window, cx)))
            .on_action(cx.listener(|this, _: &ExtendUp, _, cx| this.extend(-1, 0, cx)))
            .on_action(cx.listener(|this, _: &ExtendDown, _, cx| this.extend(1, 0, cx)))
            .on_action(cx.listener(|this, _: &ExtendLeft, _, cx| this.extend(0, -1, cx)))
            .on_action(cx.listener(|this, _: &ExtendRight, _, cx| this.extend(0, 1, cx)))
            .child(
                DataTable::new(&self.table)
                    .with_size(Size::Size(row))
                    .stripe(true)
                    .bordered(false),
            )
    }
}

/// Row height dialog (F-DATA-009). The value is in points and applies to this grid only.
struct RowHeight {
    input: Entity<InputState>,
    grid: WeakEntity<ResultGrid>,
}

impl Render for RowHeight {
    fn render(&mut self, _: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let grid = self.grid.clone();
        let input = self.input.clone();
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(t!("grid.row_height").to_string())
            .child(Input::new(&self.input).w(tokens::form::FIELD_WIDTH_SHORT.rems()))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("row-height-cancel")
                            .label(t!("grid.cancel").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("row-height-ok")
                            .primary()
                            .label(t!("grid.ok").to_string())
                            .on_click(move |_, window, cx| {
                                let value = input
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .parse::<f32>()
                                    .ok()
                                    .filter(|n| *n > 0.);
                                if let Some(n) = value {
                                    let _ = grid.update(cx, |g, cx| {
                                        g.row_height = tokens::Pt(n);
                                        cx.notify();
                                    });
                                }
                                window.close_dialog(cx);
                            }),
                    ),
            )
    }
}
