use super::*;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::table::{Column, TableDelegate, TableState};

pub(super) struct DesignerGrid {
    pub owner: WeakEntity<TableDesigner>,
    pub headers: Vec<&'static str>,
    pub widths: Vec<tokens::Pt>,
    pub rows: Vec<Vec<String>>,
    pub selected: Option<usize>,
    pub fields: bool,
    pub disabled: bool,
    pub editing: Option<(usize, usize, Entity<InputState>, &'static str)>,
    pub rem: gpui_kit::Pixels,
}
impl DesignerGrid {
    pub fn new(owner: WeakEntity<TableDesigner>, window: &Window) -> Self {
        Self {
            owner,
            headers: vec![],
            widths: vec![],
            rows: vec![],
            selected: None,
            fields: true,
            disabled: false,
            editing: None,
            rem: window.rem_size(),
        }
    }
}
impl TableDelegate for DesignerGrid {
    fn columns_count(&self, _: &App) -> usize {
        self.headers.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }
    fn column(&self, i: usize, _: &App) -> Column {
        Column::new(
            SharedString::from(self.headers[i]),
            t!(format!("designer.{}", self.headers[i])).to_string(),
        )
        .width(self.widths[i].to_pixels(self.rem))
        .movable(false)
        .p_0()
    }
    fn render_th(
        &mut self,
        i: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        gpui_kit::div()
            .size_full()
            .px_2()
            .overflow_hidden()
            .child(truncated(("designer-header", i), self.column(i, cx).name))
    }
    fn render_td(
        &mut self,
        row: usize,
        col: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let p = palette(cx);
        let value = self.rows[row][col].clone();
        let cell = h_flex()
            .size_full()
            .min_w_0()
            .border_r_1()
            .border_color(p.grid_line)
            .px_2()
            .when(self.selected == Some(row), |d| d.bg(p.row_selected));
        if self.fields && (4..=6).contains(&col) {
            let owner = self.owner.clone();
            let disabled = self.disabled;
            return cell
                .justify_center()
                .child(
                    Checkbox::new(("designer-flag", row * 8 + col))
                        .small()
                        .checked(value == "true")
                        .accessibility_label(
                            t!(format!("designer.{}", self.headers[col])).to_string(),
                        )
                        .disabled(disabled)
                        .on_click(move |checked, w, cx| {
                            let _ = owner
                                .update(cx, |this, cx| this.toggle_cell(row, col, *checked, w, cx));
                        }),
                )
                .into_any_element();
        }
        if let Some((r, c, input, id)) = &self.editing
            && (*r, *c) == (row, col)
        {
            return cell
                .p_0()
                .child(
                    Input::new(input)
                        .id(*id)
                        .small()
                        .disabled(
                            self.disabled
                                || (col == 7
                                    && self
                                        .owner
                                        .read_with(cx, |this, _| this.dialect == Dialect::Sqlite)
                                        .unwrap_or(true)),
                        )
                        .flex_1(),
                )
                .when(col == 1, |row| {
                    row.child(super::view::choices(
                        self.owner.clone(),
                        0,
                        vec![
                            "INTEGER",
                            "BIGINT",
                            "SMALLINT",
                            "VARCHAR",
                            "CHAR",
                            "TEXT",
                            "DECIMAL",
                            "NUMERIC",
                            "REAL",
                            "DOUBLE PRECISION",
                            "BOOLEAN",
                            "DATE",
                            "TIME",
                            "TIMESTAMP",
                            if self
                                .owner
                                .read_with(cx, |this, _| this.dialect == Dialect::Postgres)
                                .unwrap_or(false)
                            {
                                "BYTEA"
                            } else {
                                "BLOB"
                            },
                            "JSON",
                        ]
                        .into_iter()
                        .map(String::from)
                        .collect(),
                        self.disabled,
                    ))
                })
                .into_any_element();
        }
        // Grid cells deliberately omit per-cell tooltip hitboxes, as in the data viewer.
        cell.child(
            gpui_kit::div()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(value),
        )
        .into_any_element()
    }
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        gpui_kit::div()
            .p_3()
            .text_color(palette(cx).label_secondary)
            .child(t!("designer.empty").to_string())
    }
}
