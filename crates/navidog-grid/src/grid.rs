//! Result grid: wraps `DataTable` with rectangular selection, in-place editing, change markers, and
//! copy/paste (21-table-viewer.md §1, §7).

use std::collections::BTreeMap;

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
    WeakEntity, Window, actions, div, px,
};
use navidog_core::object::Scope;
use navidog_core::settings::{RowShade, Settings};
use navidog_db::dialect::Dialect;
use navidog_ui::assets::lucide;
use navidog_ui::skin::truncated;
use navidog_ui::{palette, tokens};
use navidog_util::sql_type::{TypeKind, type_kind};
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

/// Freeze this many data columns on the left (F-DATA-009). Zero clears the freeze.
#[derive(Clone, PartialEq, Eq, Debug, gpui_kit::Action)]
#[action(namespace = grid, no_json)]
pub struct FreezeColumns {
    pub count: usize,
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

fn menu_value(value: &str) -> String {
    let mut chars = value.chars();
    let short: String = chars.by_ref().take(32).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
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

pub struct GridDelegate {
    pub model: GridModel,
    editing: Option<(Pos, Entity<InputState>)>,
    sort: Option<(usize, bool)>,
    sort_changed: bool,
    rem: Pixels,
    /// Show the field type or comment in the column header (F-DATA-013)
    show_type: bool,
    show_comment: bool,
    /// String values are shown as SQL literals (F-DATA-010)
    raw: bool,
    dialect: Dialect,
    /// Highlight cells containing this text (query, case sensitive)
    find: Option<(SharedString, bool)>,
    /// Data columns fixed on the left, after the marker column
    frozen: usize,
    /// Resized widths by column name, so a refresh does not snap back to the default
    widths: BTreeMap<SharedString, Pixels>,
    /// The row a plain click or key move settled on. Leaving it while it is dirty commits.
    settled: Option<usize>,
    /// The pointer dragged the selection, so the mouse-up (not SelectCell) settles the row.
    dragged: bool,
    /// A data-cell click should open the editor when "click to edit" is on.
    arm_click_edit: bool,
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

    /// Width of the pinned columns. DataTable draws a hairline at this edge on every real row.
    fn pinned_span(&self) -> Pixels {
        let mut span = self.width(tokens::grid::ROW_NUMBER_WIDTH);
        for (ix, col) in self.model.columns.iter().enumerate() {
            if ix >= self.frozen {
                break;
            }
            span += self
                .widths
                .get(&col.name)
                .copied()
                .unwrap_or_else(|| self.width(tokens::grid::COLUMN_WIDTH));
        }
        span
    }
}

fn type_tint(kind: TypeKind, cx: &gpui_kit::App) -> gpui_kit::Hsla {
    let p = palette(cx);
    match kind {
        TypeKind::Number => p.icon_blue,
        TypeKind::Text => p.icon_green,
        TypeKind::Time => p.icon_orange,
        TypeKind::Bool => p.icon_teal,
        TypeKind::Binary => p.icon_yellow,
        TypeKind::Json => p.icon_purple,
        TypeKind::Other => p.label_secondary,
    }
}

fn type_glyph(kind: TypeKind) -> gpui_kit::component::Icon {
    match kind {
        TypeKind::Number => lucide("hash"),
        TypeKind::Text => lucide("a-large-small"),
        TypeKind::Time => lucide("calendar"),
        TypeKind::Bool => lucide("check"),
        TypeKind::Binary => lucide("binary"),
        TypeKind::Json => lucide("braces"),
        TypeKind::Other => lucide("square"),
    }
}

fn header_type(col: &GridColumn, show_comment: bool) -> String {
    let mut label = col.type_name.to_string();
    if show_comment && let Some(comment) = &col.comment {
        if label.is_empty() {
            label = comment.to_string();
        } else {
            label = format!("{label}  {comment}");
        }
    }
    label
}

fn column_menu(col: usize, name: SharedString, show_type: bool, show_comment: bool) -> NativeMenu {
    NativeMenu::new()
        .menu(
            t!("grid.select_column").to_string(),
            Box::new(SelectColumn { index: col }),
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
        .menu(
            t!("grid.freeze").to_string(),
            Box::new(FreezeColumns { count: col + 1 }),
        )
        .menu(
            t!("grid.unfreeze").to_string(),
            Box::new(FreezeColumns { count: 0 }),
        )
}

impl TableDelegate for GridDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.model.columns.len() + 1
    }

    fn rows_count(&self, _: &App) -> usize {
        let n = self.model.row_count();
        // A blank row under the records. The horizontal scrollbar is an overlay,
        // so without this the last row sits underneath it.
        if n == 0 { 0 } else { n + 1 }
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
        let width = self
            .widths
            .get(&col.name)
            .copied()
            .unwrap_or_else(|| self.width(tokens::grid::COLUMN_WIDTH));
        let mut column = Column::new(SharedString::from(ix.to_string()), col.name.clone())
            .width(width)
            .sort(sort)
            .movable(false)
            .p_0();
        if ix - 1 < self.frozen {
            column = column.fixed_left();
        }
        column
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

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let mut row = div().id(("row", row_ix));
        // The blank row under the records is still a real row, so DataTable paints the
        // pinned-column hairline through its first cell. Cover that strip after the row paints.
        if self.model.row_count() > 0 && row_ix == self.model.row_count() {
            let p = palette(cx);
            let striped = row_shade(cx) == RowShade::Alternate && row_ix % 2 == 1;
            // DataTable replaces the row background on hover. This layer is the resting color,
            // painted after that background, so the blank row does not change under the pointer.
            row = row.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(p.control_bg)
                    .when(striped, |d| d.child(div().size_full().bg(p.row_alt))),
            );
            row = row.child(gpui_kit::deferred(
                div()
                    .absolute()
                    .left(self.pinned_span() - px(2.)) // raw-px: pinned hairline plus antialiasing
                    .top_0()
                    .bottom_0()
                    .w(px(2.)) // raw-px: same strip
                    .bg(p.control_bg)
                    .when(striped, |d| d.child(div().size_full().bg(p.row_alt))),
            ));
        }
        row
    }

    fn render_th(
        &mut self,
        ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let col_ix = ix.checked_sub(1);
        let col = col_ix.and_then(|c| self.model.columns.get(c));
        let (show_type, show_comment) = (self.show_type, self.show_comment);
        let header = col.zip(col_ix).map(|(c, data_ix)| {
            let mut title = c.name.to_string();
            let type_label = if show_type {
                header_type(c, show_comment)
            } else {
                if show_comment && let Some(comment) = &c.comment {
                    title = format!("{title}  {comment}");
                }
                String::new()
            };
            (
                data_ix,
                c.name.clone(),
                title,
                type_kind(&c.type_name),
                type_label,
            )
        });
        let p = palette(cx);
        // No border here. Header cells are padded, so a right border lands inside the cell:
        // between the name and the sort icon on a data column, and short of the column edge
        // on the row-number column. The column edge is the resize hairline, or the pinned
        // column's own rule for the row-number column.
        div()
            .size_full()
            .min_w_0()
            .px_1()
            .overflow_hidden()
            .when_some(header, |d, (c, name, title, kind, type_label)| {
                let tint = type_tint(kind, cx);
                d.child(
                    v_flex()
                        .size_full()
                        .min_w_0()
                        .justify_center()
                        .child(
                            truncated(("grid-th", ix), title)
                                .text_color(p.label)
                                .line_height(tokens::Pt(15.).rems()),
                        )
                        .when(show_type && !type_label.is_empty(), |d| {
                            d.child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .items_center()
                                    .gap(tokens::Pt(3.).rems())
                                    .text_size(tokens::font::SIZE_SMALL.rems())
                                    .line_height(tokens::Pt(13.).rems())
                                    .text_color(tint)
                                    .child(
                                        type_glyph(kind)
                                            .size(tokens::font::SIZE_SMALL.rems())
                                            .text_color(tint),
                                    )
                                    .child(truncated(("grid-type", ix), type_label)),
                            )
                        }),
                )
                // A click selects the column; the sort icon still runs on its own hit target
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
                        column_menu(c, name.clone(), show_type, show_comment)
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
        // The trailing blank row only reserves space under the horizontal scrollbar.
        if r >= self.model.row_count() {
            // No column rules: this row only keeps the last record above the scrollbar.
            return div()
                .size_full()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .into_any_element();
        }
        let state = self.model.row_state(r);
        // Every third row is painted on the cells. Alternate uses DataTable's stripe;
        // the empty filler rows below the data stay plain. ponytail: filler rows are not tinted.
        let tint = row_shade(cx) == RowShade::EveryThird
            && RowShade::EveryThird.paints(r)
            && state != RowState::Deleted;
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
                .border_r_1()
                .border_color(p.grid_line)
                .text_color(p.label_secondary)
                .when(tint, |d| d.bg(p.row_alt))
                .when(state == RowState::Inserted, |d| d.bg(p.grid_inserted))
                .when(state == RowState::Deleted, |d| d.bg(p.grid_deleted))
                .child(mark)
                .into_any_element();
        };
        if let Some((pos, input)) = &self.editing
            && *pos == (r, c)
        {
            // The cell's click commits the edit. Stop it here so a click that
            // only moves the caret stays inside the editor.
            return div()
                .size_full()
                .border_r_1()
                .border_color(p.grid_line)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(Input::new(input).xsmall().appearance(false).size_full())
                .into_any_element();
        }
        let selected = self.model.is_selected(r, c);
        let hit = self
            .find
            .as_ref()
            .is_some_and(|(query, case)| self.model.cell_matches(r, c, query, *case));
        let shown = self.model.raw_text(r, c, self.raw, self.dialect);
        // Unfilled cells of a new row show Navicat's literal "(Default)" and are not written.
        let (text, tone): (SharedString, Option<gpui_kit::Hsla>) = match self.model.value(r, c) {
            // A cell shows one line, like Navicat; the cell editor shows the full value
            Some(Some(_)) if shown.contains('\n') => {
                (shown.replace(['\r', '\n'], " ").into(), None)
            }
            Some(Some(_)) => (shown.into(), None),
            Some(None) => ("(NULL)".into(), Some(p.label_tertiary)),
            None if state == RowState::Inserted => ("(Default)".into(), Some(p.label_secondary)),
            None => ("".into(), None),
        };
        div()
            .size_full()
            .px_1()
            .flex()
            .items_center()
            .overflow_hidden()
            .border_r_1()
            .border_color(p.grid_line)
            .when(tint, |d| d.bg(p.row_alt))
            .when(state == RowState::Inserted, |d| d.bg(p.grid_inserted))
            .when(state == RowState::Deleted, |d| d.bg(p.grid_deleted))
            .when(
                state != RowState::Inserted && self.model.is_edited(r, c),
                |d| d.bg(p.grid_edited),
            )
            .when(hit && !selected, |d| d.bg(p.selection_inactive))
            .when_some(tone.filter(|_| !selected), |d, color| d.text_color(color))
            .when(selected, |d| d.bg(p.selection).text_color(p.selection_text))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |t, e: &MouseDownEvent, _, cx| {
                    let d = t.delegate_mut();
                    d.model.select((r, c), e.modifiers.shift);
                    d.arm_click_edit = !e.modifiers.shift
                        && cx.has_global::<Settings>()
                        && Settings::global(cx).click_to_edit;
                    cx.notify();
                }),
            )
            // Dragging extends the selection; DataTable only emits SelectCell when press and release
            // land on the same cell
            .on_mouse_move(cx.listener(move |t, e: &MouseMoveEvent, _, cx| {
                if e.dragging() && t.delegate().model.head() != (r, c) {
                    let d = t.delegate_mut();
                    d.dragged = true;
                    d.arm_click_edit = false;
                    d.model.select((r, c), true);
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
                    d.arm_click_edit = false;
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
    /// The context menu shows a check while SQLite ROWID is visible
    pub show_rowid: bool,
    /// Local columns that reference another table (F-DATA-012)
    foreign_keys: Vec<String>,
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
            show_type: true,
            show_comment: false,
            raw: false,
            dialect: Dialect::Sqlite,
            find: None,
            frozen: 0,
            widths: BTreeMap::new(),
            settled: None,
            dragged: false,
            arm_click_edit: false,
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
            show_rowid: false,
            foreign_keys: Vec::new(),
            _subscriptions: subs,
        }
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
            t.refresh(cx);
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

    /// Remembers resized columns by name and writes them to settings for the open table.
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
        let saved = width_pairs(&names, reported);
        self.table.update(cx, |t, _| {
            let map = &mut t.delegate_mut().widths;
            for (name, width) in &saved {
                // raw-px: persisted column widths are already pixels
                map.insert(SharedString::from(name.as_str()), px(*width));
            }
        });
        if cx.has_global::<Settings>() {
            let key = width_key(&target.scope, &target.table);
            Settings::update(cx, |s| {
                s.column_widths.insert(key, saved);
            });
        }
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
                self.context_menu(cx)
                    .show(window.mouse_position(), window, cx);
            }
            _ => {}
        }
    }

    fn filter_target(&self, cx: &App) -> Option<(SharedString, String, bool)> {
        let m = self.model(cx);
        if m.columns.is_empty() || m.row_count() == 0 {
            return None;
        }
        let (r, c) = m.head();
        let name = m.columns.get(c)?.name.clone();
        match m.value(r, c) {
            Some(Some(text)) => Some((name, text.to_string(), false)),
            _ => Some((name, String::new(), true)),
        }
    }

    fn context_menu(&self, cx: &App) -> NativeMenu {
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
        let quick = |column: SharedString, op: &str, value: &str| {
            Box::new(QuickFilter {
                column,
                op: op.to_string().into(),
                value: value.to_string().into(),
            }) as Box<dyn gpui_kit::Action>
        };
        let mut menu = NativeMenu::new()
            .menu(t!("grid.set_empty").to_string(), Box::new(SetEmpty))
            .menu(t!("grid.set_null").to_string(), Box::new(SetNull))
            .separator()
            .menu(t!("grid.copy").to_string(), Box::new(Copy))
            .submenu(t!("grid.copy_as").to_string(), sub)
            .menu(t!("grid.paste").to_string(), Box::new(Paste));
        if let Some((column, value, is_null)) = self.filter_target(cx) {
            let shown = menu_value(&value);
            menu = menu.separator();
            if is_null {
                menu = menu.menu(
                    t!("grid.filter_null", column = column).to_string(),
                    quick(column, "is_null", ""),
                );
            } else {
                menu = menu
                    .menu(
                        t!("grid.filter_eq", column = column, value = shown).to_string(),
                        quick(column.clone(), "eq", &value),
                    )
                    .menu(
                        t!("grid.filter_ne", column = column, value = shown).to_string(),
                        quick(column.clone(), "ne", &value),
                    )
                    .menu(
                        t!("grid.filter_contains", column = column, value = shown).to_string(),
                        quick(column, "contains", &value),
                    );
            }
        }
        if let Some((column, _, _)) = self.filter_target(cx)
            && self.foreign_keys.iter().any(|name| name == column.as_ref())
        {
            menu = menu.menu(t!("grid.fk_pick").to_string(), Box::new(PickForeignKey));
        }
        let mut menu = menu
            .separator()
            .menu(t!("grid.delete_rows").to_string(), Box::new(DeleteRows))
            .separator()
            .menu(t!("grid.row_height").to_string(), Box::new(SetRowHeight));
        if self
            .target
            .as_ref()
            .is_some_and(|t| t.dialect == Dialect::Sqlite)
        {
            menu = menu.menu_with_check(
                t!("grid.rowid").to_string(),
                self.show_rowid,
                Box::new(ToggleRowId),
            );
        }
        menu.separator().menu_with_check(
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
        if self
            .model(cx)
            .columns
            .get(pos.1)
            .is_some_and(|col| col.read_only)
        {
            return;
        }
        let kind = self
            .model(cx)
            .columns
            .get(pos.1)
            .and_then(|col| crate::date::temporal_kind(&col.type_name));
        let (raw, dialect) = {
            let table = self.table.read(cx);
            let d = table.delegate();
            (d.raw, d.dialect)
        };
        let text = self.model(cx).raw_text(pos.0, pos.1, raw, dialect);
        if let Some(kind) = kind {
            crate::date::open(pos, kind, &text, cx.entity().downgrade(), window, cx);
            return;
        }
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
        let (raw, dialect) = {
            let table = self.table.read(cx);
            let d = table.delegate();
            (d.raw, d.dialect)
        };
        let old = self.model(cx).raw_text(r, c, raw, dialect);
        if text != old {
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
        let rem = self.table.read(cx).delegate().rem;
        let row = self.row_height.to_pixels(rem);
        // The header and the body share one row height. Two lines need the taller one.
        let row = if self.table.read(cx).delegate().show_type {
            row.max(tokens::grid::TYPE_HEADER_HEIGHT.to_pixels(rem))
        } else {
            row
        };
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
            .on_action(cx.listener(|this, _: &ToggleFieldType, _, cx| {
                this.table.update(cx, |t, cx| {
                    t.delegate_mut().show_type = !t.delegate().show_type;
                    t.refresh(cx);
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
                this.focus_column(cx);
            }))
            .on_action(cx.listener(|this, a: &FreezeColumns, _, cx| {
                this.table.update(cx, |t, cx| {
                    t.delegate_mut().frozen = a.count;
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
                    .stripe(alternate)
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

fn row_shade(cx: &App) -> RowShade {
    if cx.has_global::<Settings>() {
        Settings::global(cx).row_shade
    } else {
        RowShade::Alternate
    }
}

/// Point size when the grid is not using the interface font. `None` keeps the default.
fn grid_font_pt(cx: &App) -> Option<f32> {
    if !cx.has_global::<Settings>() {
        return None;
    }
    let s = Settings::global(cx);
    (!s.use_default_grid_font).then_some(s.grid_font_pt)
}

fn width_key(scope: &Scope, table: &str) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{table}",
        scope.connection,
        scope.database,
        scope.schema.as_deref().unwrap_or("")
    )
}

fn load_widths(target: &GridTarget, cx: &App) -> BTreeMap<SharedString, Pixels> {
    if !cx.has_global::<Settings>() {
        return BTreeMap::new();
    }
    Settings::global(cx)
        .column_widths
        .get(&width_key(&target.scope, &target.table))
        .map(|cols| {
            cols.iter()
                .map(|(name, width)| {
                    // raw-px: persisted column widths are already pixels
                    (SharedString::from(name.clone()), px(*width))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `reported` is every column including the marker at index 0.
fn width_pairs(names: &[SharedString], reported: &[Pixels]) -> BTreeMap<String, f32> {
    names
        .iter()
        .zip(reported.iter().skip(1))
        .map(|(name, width)| (name.to_string(), width.as_f32()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resized_widths_skip_the_marker_column() {
        let names = ["id", "name"].map(SharedString::from);
        // raw-px: arbitrary widths; the first is the marker column and is dropped
        let saved = width_pairs(&names, &[px(20.), px(80.), px(140.)]);
        assert_eq!(saved.get("id"), Some(&80.));
        assert_eq!(saved.get("name"), Some(&140.));
        assert_eq!(saved.len(), 2);
    }
}
