//! The result grid's `DataTable` delegate: the model, column layout and widths, and how the
//! header, rows and cells paint and respond to the pointer.

use super::*;

use gpui_kit::StatefulInteractiveElement as _;

pub struct GridDelegate {
    pub model: GridModel,
    pub(super) editing: Option<(Pos, Entity<InputState>)>,
    pub(super) sort: Option<(usize, bool)>,
    /// The sort the rows were last reordered by. A refused header click restores `sort` to this.
    pub(super) applied_sort: Option<(usize, bool)>,
    pub(super) sort_changed: bool,
    pub(super) rem: Pixels,
    /// Show the field type or comment in the column header (F-DATA-013)
    pub(super) show_type: bool,
    pub(super) show_comment: bool,
    /// String values are shown as SQL literals (F-DATA-010)
    pub(super) raw: bool,
    pub(super) dialect: Dialect,
    /// Highlight cells containing this text (query, case sensitive)
    pub(super) find: Option<(SharedString, bool)>,
    /// Display order and which columns are frozen (F-DATA-009)
    pub(super) layout: ColumnLayout,
    /// Resized widths by column name, so a refresh does not snap back to the fit
    pub(super) widths: BTreeMap<SharedString, Pixels>,
    /// Fitted widths for columns the user has not resized. Not written to settings.
    pub(super) auto: BTreeMap<SharedString, Pixels>,
    /// The row a plain click or key move settled on. Leaving it while it is dirty commits.
    pub(super) settled: Option<usize>,
    /// The pointer dragged the selection, so the mouse-up (not SelectCell) settles the row.
    pub(super) dragged: bool,
    /// A selection drag only starts from a mouse-down on a data cell. A column-header
    /// drag also has the button down, but it must not select the cells it passes over.
    pub(super) selecting: bool,
    /// A data-cell click should open the editor when "click to edit" is on.
    pub(super) arm_click_edit: bool,
    pub(super) commit_requested: bool,
}

impl GridDelegate {
    /// A plain navigation to another row commits when the row we left is dirty (F-DATA-003).
    /// Shift-extend keeps the settled row, so a range selection does not commit halfway.
    pub(super) fn finish_navigation(&mut self, shift: bool) {
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
    pub(super) fn width(&self, pt: tokens::Pt) -> Pixels {
        pt.to_pixels(self.rem)
    }

    pub(super) fn row_selected(&self, r: usize, state: RowState) -> bool {
        self.model.selection().0.contains(&r) && state != RowState::Deleted
    }

    pub(super) fn names(&self) -> Vec<SharedString> {
        self.model.columns.iter().map(|c| c.name.clone()).collect()
    }

    /// Puts the live page into `desired` order. Edits and the selection move with the columns.
    pub(super) fn apply_display(&mut self, desired: &[SharedString]) {
        let current = self.names();
        if current == desired {
            return;
        }
        let sort_name = self
            .sort
            .and_then(|(i, asc)| current.get(i).map(|n| (n.clone(), asc)));
        let edit_name = self
            .editing
            .as_ref()
            .and_then(|(pos, _)| current.get(pos.1).cloned());
        let order: Vec<usize> = desired
            .iter()
            .filter_map(|n| current.iter().position(|c| c == n))
            .collect();
        self.model.reorder(&order);
        self.sort = sort_name.and_then(|(name, asc)| {
            self.model
                .columns
                .iter()
                .position(|c| c.name == name)
                .map(|i| (i, asc))
        });
        if let Some(name) = edit_name
            && let Some((pos, _)) = &mut self.editing
            && let Some(i) = self.model.columns.iter().position(|c| c.name == name)
        {
            pos.1 = i;
        }
    }

    /// Width of the pinned columns. DataTable draws a hairline at this edge on every real row.
    pub(super) fn pinned_span(&self) -> Pixels {
        let mut span = self.width(tokens::grid::ROW_NUMBER_WIDTH);
        let pinned = self.layout.pinned(&self.model.columns);
        for (ix, col) in self.model.columns.iter().enumerate() {
            if ix >= pinned {
                break;
            }
            span += self.column_width(&col.name);
        }
        span
    }

    /// User-resized width, otherwise the width fitted when the page loaded, otherwise 140pt.
    pub(super) fn column_width(&self, name: &SharedString) -> Pixels {
        self.widths
            .get(name)
            .or_else(|| self.auto.get(name))
            .copied()
            .unwrap_or_else(|| self.width(tokens::grid::COLUMN_WIDTH))
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

fn column_menu(
    col: usize,
    name: SharedString,
    show_type: bool,
    show_comment: bool,
    frozen: bool,
    any_frozen: bool,
    at_server: bool,
) -> NativeMenu {
    NativeMenu::new()
        .menu(
            t!("grid.select_column").to_string(),
            Box::new(SelectColumn { index: col }),
        )
        .menu(
            t!("grid.copy_field_name").to_string(),
            Box::new(CopyFieldName { name: name.clone() }),
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
            Box::new(FreezeThrough { index: col }),
        )
        .menu_with_disabled(
            t!("grid.freeze_column").to_string(),
            frozen,
            Box::new(FreezeColumn { index: col }),
        )
        .menu_with_disabled(
            t!("grid.unfreeze_all").to_string(),
            !any_frozen,
            Box::new(UnfreezeAll),
        )
        .menu_with_disabled(
            t!("grid.restore_order").to_string(),
            at_server,
            Box::new(RestoreColumnOrder),
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
        let width = self.column_width(&col.name);
        let mut column = Column::new(SharedString::from(ix.to_string()), col.name.clone())
            .width(width)
            .sort(sort)
            .movable(true)
            .p_0();
        if ix - 1 < self.layout.pinned(&self.model.columns) {
            column = column.fixed_left();
        }
        column
    }

    fn move_column(
        &mut self,
        col_ix: usize,
        to_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        // Index 0 is the marker column. DataTable still swaps its groups after this
        // returns, so a drop onto the marker is undone on the next frame.
        if col_ix == 0 || to_ix == 0 {
            let table = cx.entity();
            cx.defer(move |cx| {
                table.update(cx, |t, cx| t.refresh(cx));
            });
            return;
        }
        let from = col_ix - 1;
        let to = to_ix - 1;
        let present = self.names();
        self.layout.drag(&present, from, to);
        let desired = self.layout.display_names(&present);
        self.apply_display(&desired);
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
        } else if row_ix < self.model.row_count() {
            let state = self.model.row_state(row_ix);
            let p = palette(cx);
            // Applied before DataTable's hover, so a hover replaces the whole row, empty tail included.
            if self.row_selected(row_ix, state) {
                row = row.bg(p.row_selected);
            } else if row_shade(cx) == RowShade::EveryThird
                && RowShade::EveryThird.paints(row_ix)
                && state != RowState::Deleted
            {
                row = row.bg(p.row_alt);
            }
            // Cells stop this click. What reaches the row is the empty tail past the last field.
            row = row.on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(TableEvent::SelectRow(row_ix));
            }));
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
                                .line_height(tokens::grid::HEADER_NAME_LINE.rems()),
                        )
                        .when(show_type && !type_label.is_empty(), |d| {
                            d.child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .items_center()
                                    .gap(tokens::grid::HEADER_TYPE_GAP.rems())
                                    .text_size(tokens::font::SIZE_SMALL.rems())
                                    .line_height(tokens::grid::HEADER_TYPE_LINE.rems())
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
                // Capture, not bubble. The table wraps this header in a context menu whose
                // right-click listener is registered after the header and therefore runs first
                // on the bubble pass. Stopping there is too late: the empty popup takes focus
                // in a defer, and the native menu item never reaches the grid.
                .capture_any_mouse_down(cx.listener(
                    move |t, e: &MouseDownEvent, window, cx| {
                        if e.button != MouseButton::Right {
                            return;
                        }
                        cx.stop_propagation();
                        // Right-click does not focus the table, and the menu
                        // dispatches to whoever is focused.
                        let handle = t.focus_handle(cx);
                        window.focus(&handle, cx);
                        let d = t.delegate();
                        let frozen = d
                            .model
                            .columns
                            .get(c)
                            .is_some_and(|col| d.layout.is_frozen(&col.name));
                        let any_frozen = d.layout.any_frozen();
                        let present = d.names();
                        let at_server = d.layout.is_server_order(&present);
                        column_menu(
                            c,
                            name.clone(),
                            show_type,
                            show_comment,
                            frozen,
                            any_frozen,
                            at_server,
                        )
                        .show(e.position, window, cx);
                    },
                ))
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
                .when(state == RowState::Inserted, |d| d.bg(p.grid_inserted))
                .when(state == RowState::Deleted, |d| d.bg(p.grid_deleted))
                .child(mark)
                .into_any_element();
        };
        let editing = self
            .editing
            .as_ref()
            .filter(|(pos, _)| *pos == (r, c))
            .map(|(_, input)| input.clone());
        let selected = self.model.is_selected(r, c);
        let numeric = self
            .model
            .columns
            .get(c)
            .is_some_and(|col| type_kind(&col.type_name) == TypeKind::Number);
        let hit = self
            .find
            .as_ref()
            .is_some_and(|(query, case)| self.model.cell_matches(r, c, query, *case));
        let shown = self.model.shown_text(r, c, self.raw, self.dialect);
        // Unfilled cells of a new row show Navicat's literal "(Default)" and are not written.
        let (text, tone): (SharedString, Option<gpui_kit::Hsla>) = match self.model.value(r, c) {
            // A cell shows one line, like Navicat; the cell editor shows the full value
            Some(Some(_)) if shown.contains('\n') => {
                (shown.replace(['\r', '\n'], " ").into(), None)
            }
            Some(Some(_)) => (shown, None),
            Some(None) => ("(NULL)".into(), Some(p.label_tertiary)),
            None if state == RowState::Inserted => ("(Default)".into(), Some(p.label_secondary)),
            None => ("".into(), None),
        };
        let cell = div()
            .size_full()
            .px_1()
            .flex()
            .items_center()
            .overflow_hidden()
            .when(numeric, |d| d.justify_end())
            .when(state == RowState::Inserted, |d| d.bg(p.grid_inserted))
            .when(state == RowState::Deleted, |d| d.bg(p.grid_deleted))
            .when(
                state != RowState::Inserted && self.model.is_edited(r, c),
                |d| d.bg(p.grid_edited),
            )
            .when(hit && !selected, |d| d.bg(p.selection_inactive))
            .when_some(tone.filter(|_| !selected), |d, color| d.text_color(color))
            .when(selected, |d| d.bg(p.selection).text_color(p.selection_text));
        if let Some(input) = editing {
            let font = grid_text_size(cx);
            // Transparent, so the selected-row color shows through. The text selection
            // uses the theme color, which is darker than that row.
            // xsmall fits the row; text_size puts the grid font back (it is applied
            // after Input's own size style).
            return div()
                .size_full()
                .px_1()
                .flex()
                .items_center()
                .overflow_hidden()
                .when(numeric, |d| d.justify_end())
                .border_1()
                .border_color(p.accent)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Input::new(&input)
                        .xsmall()
                        .appearance(false)
                        .size_full()
                        .text_size(font),
                )
                .into_any_element();
        }
        cell.border_r_1()
            .border_color(p.grid_line)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |t, e: &MouseDownEvent, _, cx| {
                    let d = t.delegate_mut();
                    d.selecting = true;
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
                // A drag that starts in the editor selects text, not other cells.
                // A column drag never pressed a cell, so it must not extend the selection.
                if t.delegate().editing.is_some() || !t.delegate().selecting {
                    return;
                }
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
                    d.selecting = false;
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
