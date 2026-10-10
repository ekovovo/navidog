//! Grid data model: the current page, the edit overlay, inserted and pending-delete rows, and the
//! rectangular selection (21-table-viewer.md §2). No UI here; unit tests cover the change set and copy/paste.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::RangeInclusive;

use gpui_kit::SharedString;
use navidog_db::dialect::{ChangeSet, Dialect, RowKey, RowUpdate, Writes, Written};
use navidog_util::sql_type::is_string_type;

/// A cell value; `None` inside the `Option` is NULL, and a missing cell is an unfilled new row.
pub type Cell = Option<SharedString>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridColumn {
    pub name: SharedString,
    pub type_name: SharedString,
    pub primary_key: bool,
    pub comment: Option<SharedString>,
    /// Shown but not written back (SQLite ROWID)
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    Clean,
    Edited,
    Inserted,
    Deleted,
}

/// Row and column coordinates are data coordinates (they exclude the marker column on the left).
pub type Pos = (usize, usize);

#[derive(Default)]
pub struct GridModel {
    pub columns: Vec<GridColumn>,
    rows: Vec<Vec<Cell>>,
    /// Edits of existing rows: (row, column) to the new value
    edits: BTreeMap<Pos, Cell>,
    /// Inserted rows; `None` means that column was left empty and the insert uses the default
    new_rows: Vec<Vec<Option<Cell>>>,
    deleted: BTreeSet<usize>,
    /// Cells whose text is SQL rather than a string literal (raw data mode)
    raw: BTreeSet<Pos>,
    /// Hidden columns and their values in each loaded row. Rows are still located by them, so
    /// hiding the primary key (or any column of a table without one) does not widen a WHERE.
    hidden_columns: Vec<GridColumn>,
    hidden_rows: Vec<Vec<Cell>>,
    anchor: Pos,
    head: Pos,
}

impl GridModel {
    pub fn set_data(&mut self, columns: Vec<GridColumn>, rows: Vec<Vec<Cell>>) {
        self.columns = columns;
        self.rows = rows;
        self.hidden_columns.clear();
        self.hidden_rows.clear();
        self.discard();
        let max = (
            self.row_count().saturating_sub(1),
            self.columns.len().saturating_sub(1),
        );
        self.anchor = (self.anchor.0.min(max.0), self.anchor.1.min(max.1));
        self.head = self.anchor;
    }

    /// Moves one data column. Cells, edits, the raw-SQL flags and the selection follow it.
    pub fn move_column(&mut self, from: usize, to: usize) {
        let n = self.columns.len();
        if from == to || from >= n || to >= n {
            return;
        }
        let mut order: Vec<usize> = (0..n).collect();
        let ix = order.remove(from);
        order.insert(to, ix);
        self.reorder(&order);
    }

    /// `order[new_index]` is the old column index.
    pub fn reorder(&mut self, order: &[usize]) {
        if order.len() != self.columns.len() || (0..order.len()).any(|i| !order.contains(&i)) {
            return;
        }
        self.columns = order.iter().map(|&i| self.columns[i].clone()).collect();
        for row in &mut self.rows {
            let next = permute_cells(row, order);
            *row = next;
        }
        for row in &mut self.new_rows {
            let next: Vec<Option<Cell>> = order
                .iter()
                .map(|&i| row.get(i).cloned().flatten())
                .collect();
            *row = next;
        }
        let edits = std::mem::take(&mut self.edits);
        self.edits = edits
            .into_iter()
            .filter_map(|((r, c), v)| remap_col(c, order).map(|nc| ((r, nc), v)))
            .collect();
        let raw = std::mem::take(&mut self.raw);
        self.raw = raw
            .into_iter()
            .filter_map(|(r, c)| remap_col(c, order).map(|nc| (r, nc)))
            .collect();
        self.anchor.1 = remap_col(self.anchor.1, order).unwrap_or(0);
        self.head.1 = remap_col(self.head.1, order).unwrap_or(0);
    }

    pub fn row_count(&self) -> usize {
        self.rows.len() + self.new_rows.len()
    }

    /// Text the cell shows and the editor starts from. In raw data mode a string value is a SQL
    /// literal (`'PENELOPE'`), including quotes. A value already typed as an expression is left as
    /// typed. NULL and an unfilled cell are empty; the grid paints NULL itself.
    pub fn raw_text(&self, r: usize, c: usize, raw_mode: bool, dialect: Dialect) -> String {
        if self.raw.contains(&(r, c)) {
            return self
                .value(r, c)
                .cloned()
                .flatten()
                .unwrap_or_default()
                .to_string();
        }
        match self.value(r, c) {
            Some(Some(s))
                if raw_mode
                    && self
                        .columns
                        .get(c)
                        .is_some_and(|col| is_string_type(&col.type_name)) =>
            {
                dialect.string_literal(s)
            }
            Some(Some(s)) => s.to_string(),
            _ => String::new(),
        }
    }

    pub fn value(&self, r: usize, c: usize) -> Option<&Cell> {
        match r.checked_sub(self.rows.len()) {
            None => self.edits.get(&(r, c)).or_else(|| self.rows[r].get(c)),
            Some(n) => self.new_rows.get(n)?.get(c)?.as_ref(),
        }
    }

    pub fn set_value(&mut self, r: usize, c: usize, v: Cell) {
        self.write(r, c, v, false);
    }

    /// Like `set_value`, but a non-empty value is submitted as SQL (raw data mode).
    pub fn set_raw(&mut self, r: usize, c: usize, v: Cell) {
        self.write(r, c, v, true);
    }

    fn write(&mut self, r: usize, c: usize, v: Cell, raw: bool) {
        if self.columns.get(c).is_some_and(|col| col.read_only) {
            return;
        }
        let stored = match r.checked_sub(self.rows.len()) {
            None if self.rows[r].get(c) == Some(&v) => {
                self.edits.remove(&(r, c));
                false
            }
            None => {
                self.edits.insert((r, c), v);
                true
            }
            Some(n) => {
                if let Some(cell) = self.new_rows.get_mut(n).and_then(|row| row.get_mut(c)) {
                    *cell = Some(v);
                    true
                } else {
                    false
                }
            }
        };
        if stored && raw {
            self.raw.insert((r, c));
        } else {
            self.raw.remove(&(r, c));
        }
    }

    pub fn row_state(&self, r: usize) -> RowState {
        // Past the last row is not an insert. An empty grid used to report row 0 as
        // Inserted, so "add row" tried to commit nothing and did not insert.
        if r >= self.row_count() {
            RowState::Clean
        } else if r >= self.rows.len() {
            RowState::Inserted
        } else if self.deleted.contains(&r) {
            RowState::Deleted
        } else if self.edits.range((r, 0)..=(r, usize::MAX)).next().is_some() {
            RowState::Edited
        } else {
            RowState::Clean
        }
    }

    /// The row has edits, is a new row, or is pending deletion, so leaving it commits (F-DATA-003).
    pub fn row_dirty(&self, r: usize) -> bool {
        !matches!(self.row_state(r), RowState::Clean)
    }

    pub fn is_edited(&self, r: usize, c: usize) -> bool {
        self.edits.contains_key(&(r, c)) || (r >= self.rows.len() && self.value(r, c).is_some())
    }

    /// Appends an empty row, selects it and returns its row number.
    pub fn insert_row(&mut self) -> usize {
        self.new_rows.push(vec![None; self.columns.len()]);
        let r = self.row_count() - 1;
        self.select((r, 0), false);
        r
    }

    /// Deletes the rows the selection covers: inserted rows go away, existing rows are marked pending.
    pub fn delete_selected_rows(&mut self) {
        let rows = self.selection().0;
        for r in rows.clone().rev() {
            match r.checked_sub(self.rows.len()) {
                Some(n) if n < self.new_rows.len() => {
                    let abs = self.rows.len() + n;
                    self.new_rows.remove(n);
                    self.raw = self
                        .raw
                        .iter()
                        .filter_map(|&(row, col)| match row.cmp(&abs) {
                            std::cmp::Ordering::Equal => None,
                            std::cmp::Ordering::Greater => Some((row - 1, col)),
                            std::cmp::Ordering::Less => Some((row, col)),
                        })
                        .collect();
                }
                Some(_) => {}
                None => {
                    self.deleted.insert(r);
                }
            }
        }
        let r = (*rows.start()).min(self.row_count().saturating_sub(1));
        self.select((r, self.head.1), false);
    }

    pub fn has_changes(&self) -> bool {
        !self.edits.is_empty() || !self.new_rows.is_empty() || !self.deleted.is_empty()
    }

    pub fn discard(&mut self) {
        self.edits.clear();
        self.new_rows.clear();
        self.deleted.clear();
        self.raw.clear();
    }

    /// Folds a successful commit into the loaded page when "synchronize after modifying" is off.
    /// Unfilled insert columns become NULL; generated values show up only on the next refresh.
    pub fn accept_committed(&mut self) {
        for (&(r, c), v) in &self.edits {
            if let Some(cell) = self.rows.get_mut(r).and_then(|row| row.get_mut(c)) {
                *cell = v.clone();
            }
        }
        let mut gone: Vec<usize> = self.deleted.iter().copied().collect();
        gone.sort_unstable_by(|a, b| b.cmp(a));
        for r in gone {
            if r < self.rows.len() {
                self.rows.remove(r);
            }
            if r < self.hidden_rows.len() {
                self.hidden_rows.remove(r);
            }
        }
        for row in self.new_rows.drain(..) {
            if row.iter().any(Option::is_some) {
                self.rows
                    .push(row.into_iter().map(|cell| cell.flatten()).collect());
                if !self.hidden_columns.is_empty() {
                    self.hidden_rows.push(vec![None; self.hidden_columns.len()]);
                }
            }
        }
        self.edits.clear();
        self.deleted.clear();
        self.raw.clear();
        if self.row_count() == 0 {
            self.head = (0, 0);
            self.anchor = (0, 0);
        } else if self.head.0 >= self.row_count() {
            let r = self.row_count() - 1;
            self.head.0 = r;
            self.anchor.0 = r;
        }
    }

    pub fn has_primary_key(&self) -> bool {
        self.columns
            .iter()
            .chain(&self.hidden_columns)
            .any(|c| c.primary_key)
    }

    fn key(&self, r: usize) -> RowKey {
        let pk = self.has_primary_key();
        let visible = self.columns.iter().zip(&self.rows[r]);
        let hidden = self
            .hidden_columns
            .iter()
            .zip(self.hidden_rows.get(r).into_iter().flatten());
        RowKey(
            visible
                .chain(hidden)
                .filter(|(col, _)| (!pk || col.primary_key) && !col.read_only)
                .map(|(col, v)| (col.name.to_string(), v.as_ref().map(|s| s.to_string())))
                .collect(),
        )
    }

    pub fn change_set(&self) -> ChangeSet {
        let name = |c: usize| self.columns[c].name.to_string();
        let written = |r: usize, c: usize, v: &Cell| {
            (
                name(c),
                Written {
                    value: v.as_ref().map(|s| s.to_string()),
                    raw: self.raw.contains(&(r, c)),
                },
            )
        };
        let mut updates: BTreeMap<usize, Writes> = BTreeMap::new();
        for (&(r, c), v) in &self.edits {
            if !self.deleted.contains(&r) && !self.columns[c].read_only {
                updates.entry(r).or_default().push(written(r, c, v));
            }
        }
        ChangeSet {
            updates: updates
                .into_iter()
                .map(|(r, values)| RowUpdate {
                    key: self.key(r),
                    values,
                })
                .collect(),
            // An inserted row with nothing typed is not inserted
            inserts: self
                .new_rows
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let r = self.rows.len() + i;
                    row.iter()
                        .enumerate()
                        .filter_map(|(c, v)| {
                            if self.columns.get(c).is_some_and(|col| col.read_only) {
                                return None;
                            }
                            Some(written(r, c, v.as_ref()?))
                        })
                        .collect::<Writes>()
                })
                .filter(|v| !v.is_empty())
                .collect(),
            deletes: self.deleted.iter().map(|&r| self.key(r)).collect(),
        }
    }

    // Selection

    pub fn head(&self) -> Pos {
        self.head
    }

    /// Rows and columns the selection covers (inclusive).
    pub fn selection(&self) -> (RangeInclusive<usize>, RangeInclusive<usize>) {
        let (a, h) = (self.anchor, self.head);
        (a.0.min(h.0)..=a.0.max(h.0), a.1.min(h.1)..=a.1.max(h.1))
    }

    pub fn is_selected(&self, r: usize, c: usize) -> bool {
        let (rows, cols) = self.selection();
        rows.contains(&r) && cols.contains(&c)
    }

    /// Selects one cell; with `extend`, the anchor stays and the selection grows to that cell.
    pub fn select(&mut self, pos: Pos, extend: bool) {
        self.head = pos;
        if !extend {
            self.anchor = pos;
        }
    }

    /// Selects the whole row (a click on the marker column).
    pub fn select_rows(&mut self, r: usize, extend: bool) {
        let last = self.columns.len().saturating_sub(1);
        if !extend {
            self.anchor = (r, 0);
        } else {
            self.anchor.1 = 0;
        }
        self.head = (r, last);
    }

    /// Selects every cell of one column. With `extend`, columns from the anchor to this one.
    pub fn select_column(&mut self, c: usize, extend: bool) {
        let last = self.row_count().saturating_sub(1);
        if !extend {
            self.anchor = (0, c);
        } else {
            self.anchor.0 = 0;
        }
        self.head = (last, c);
    }

    pub fn select_all(&mut self) {
        self.anchor = (0, 0);
        self.head = (
            self.row_count().saturating_sub(1),
            self.columns.len().saturating_sub(1),
        );
    }

    /// The selected rows, each holding only its selected columns.
    pub fn selected_values(&self) -> Vec<Vec<Option<String>>> {
        let (rows, cols) = self.selection();
        rows.filter(|&r| r < self.row_count())
            .map(|r| {
                cols.clone()
                    .map(|c| self.value(r, c).cloned().flatten().map(|s| s.to_string()))
                    .collect()
            })
            .collect()
    }

    pub fn selected_columns(&self) -> Vec<String> {
        self.selection()
            .1
            .filter_map(|c| self.columns.get(c))
            .map(|c| c.name.to_string())
            .collect()
    }

    /// The selection as tab-separated text; NULL is copied as an empty string. `header` adds a name row.
    // ponytail: tabs and newlines inside a value are not escaped, so pasting back mis-aligns; quote the
    // TSV if a round trip is needed
    pub fn selected_tsv(&self, header: bool, data: bool) -> String {
        let mut lines = Vec::new();
        if header {
            lines.push(self.selected_columns().join("\t"));
        }
        if data {
            for row in self.selected_values() {
                lines.push(
                    row.into_iter()
                        .map(Option::unwrap_or_default)
                        .collect::<Vec<_>>()
                        .join("\t"),
                );
            }
        }
        lines.join("\n")
    }

    /// Columns and cell values with `hidden` names removed. Hiding every column keeps them all.
    pub fn project(
        columns: &[GridColumn],
        rows: &[Vec<Cell>],
        hidden: &HashSet<String>,
    ) -> (Vec<GridColumn>, Vec<Vec<Cell>>) {
        select_columns(columns, rows, |c| !hides(columns, hidden, c))
    }

    /// Keeps the `hidden` columns of the unprojected page, so rows are still located by them.
    /// Call after `set_data` with the projection of the same page.
    pub fn set_hidden_columns(
        &mut self,
        columns: &[GridColumn],
        rows: &[Vec<Cell>],
        hidden: &HashSet<String>,
    ) {
        (self.hidden_columns, self.hidden_rows) =
            select_columns(columns, rows, |c| hides(columns, hidden, c));
    }

    /// Next cell at or after `from` whose text contains `query`. Wraps around the page.
    /// `after` starts at the following cell, so Find Next does not stay on the current hit.
    pub fn find_cell(
        &self,
        query: &str,
        from: Pos,
        after: bool,
        case_sensitive: bool,
    ) -> Option<Pos> {
        if query.is_empty() || self.columns.is_empty() || self.row_count() == 0 {
            return None;
        }
        let cols = self.columns.len();
        let start = if after {
            let (r, c) = from;
            if c + 1 < cols {
                (r, c + 1)
            } else {
                (r.saturating_add(1), 0)
            }
        } else {
            from
        };
        let total = self.row_count() * cols;
        let start_i = (start.0 * cols + start.1).min(total.saturating_sub(1));
        (0..total).find_map(|step| {
            let i = (start_i + step) % total;
            let pos = (i / cols, i % cols);
            self.cell_matches(pos.0, pos.1, query, case_sensitive)
                .then_some(pos)
        })
    }

    /// Next column name containing `query`, wrapping around.
    pub fn find_column_name(
        &self,
        query: &str,
        from: usize,
        after: bool,
        case_sensitive: bool,
    ) -> Option<usize> {
        let n = self.columns.len();
        if query.is_empty() || n == 0 {
            return None;
        }
        let start = if after {
            from.saturating_add(1) % n
        } else {
            from.min(n - 1)
        };
        (0..n).find_map(|step| {
            let c = (start + step) % n;
            contains(&self.columns[c].name, query, case_sensitive).then_some(c)
        })
    }

    pub fn cell_matches(&self, r: usize, c: usize, query: &str, case_sensitive: bool) -> bool {
        self.value(r, c)
            .and_then(|v| v.as_deref())
            .is_some_and(|s| contains(s, query, case_sensitive))
    }

    /// Replaces `query` in one cell. Returns whether the text changed.
    pub fn replace_cell(
        &mut self,
        pos: Pos,
        query: &str,
        replacement: &str,
        case_sensitive: bool,
    ) -> bool {
        let Some(Some(current)) = self.value(pos.0, pos.1).cloned() else {
            return false;
        };
        let Some(next) = replace_text(&current, query, replacement, case_sensitive) else {
            return false;
        };
        self.set_value(pos.0, pos.1, Some(next.into()));
        true
    }

    /// Replaces `query` in every loaded cell. Returns how many cells changed.
    pub fn replace_all(&mut self, query: &str, replacement: &str, case_sensitive: bool) -> usize {
        let hits: Vec<Pos> = (0..self.row_count())
            .flat_map(|r| (0..self.columns.len()).map(move |c| (r, c)))
            .filter(|&(r, c)| self.cell_matches(r, c, query, case_sensitive))
            .collect();
        hits.into_iter()
            .filter(|&pos| self.replace_cell(pos, query, replacement, case_sensitive))
            .count()
    }

    /// Writes tab-separated text starting at the selection's top left, appending rows past the end.
    pub fn paste(&mut self, text: &str, raw: bool) {
        let (rows, cols) = self.selection();
        let (r0, c0) = (*rows.start(), *cols.start());
        let lines: Vec<&str> = text.trim_end_matches(['\n', '\r']).split('\n').collect();
        for (i, line) in lines.iter().enumerate() {
            let r = r0 + i;
            while r >= self.row_count() {
                self.new_rows.push(vec![None; self.columns.len()]);
            }
            for (j, v) in line.trim_end_matches('\r').split('\t').enumerate() {
                if c0 + j < self.columns.len() {
                    let cell = Some(v.to_string().into());
                    if raw {
                        self.set_raw(r, c0 + j, cell);
                    } else {
                        self.set_value(r, c0 + j, cell);
                    }
                }
            }
        }
    }
}

fn server_order(server: &[SharedString], present: &[SharedString]) -> Vec<SharedString> {
    let mut out = Vec::with_capacity(present.len());
    for name in server {
        if present.iter().any(|p| p == name) && !out.iter().any(|n| n == name) {
            out.push(name.clone());
        }
    }
    for name in present {
        if !out.iter().any(|n| n == name) {
            out.push(name.clone());
        }
    }
    out
}

fn permute_cells(row: &[Cell], order: &[usize]) -> Vec<Cell> {
    order
        .iter()
        .map(|&i| row.get(i).cloned().flatten())
        .collect()
}

fn remap_col(old: usize, order: &[usize]) -> Option<usize> {
    order.iter().position(|&i| i == old)
}

/// Hiding every column hides none, since the grid could not show anything to unhide.
fn hides(columns: &[GridColumn], hidden: &HashSet<String>, col: &GridColumn) -> bool {
    hidden.len() < columns.len() && hidden.contains(col.name.as_ref())
}

fn select_columns(
    columns: &[GridColumn],
    rows: &[Vec<Cell>],
    pick: impl Fn(&GridColumn) -> bool,
) -> (Vec<GridColumn>, Vec<Vec<Cell>>) {
    let ix: Vec<usize> = (0..columns.len()).filter(|&i| pick(&columns[i])).collect();
    let columns = ix.iter().map(|&i| columns[i].clone()).collect();
    let rows = rows.iter().map(|r| permute_cells(r, &ix)).collect();
    (columns, rows)
}

/// Column order for one open grid. `home` is the order with freeze-induced moves undone.
/// `frozen` is the frozen names, left to right. Display order pulls frozen names to the front.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColumnLayout {
    home: Vec<SharedString>,
    frozen: Vec<SharedString>,
    /// Column order returned by the server. Drags and freezes do not change it.
    server: Vec<SharedString>,
}

impl ColumnLayout {
    pub fn clear(&mut self) {
        self.home.clear();
        self.frozen.clear();
        self.server.clear();
    }

    /// The query's column order, before this grid permutes it for display.
    pub fn remember_server(&mut self, present: &[SharedString]) {
        self.server = present.to_vec();
    }

    /// Puts `home` back to the server order and clears the freeze.
    pub fn restore_server(&mut self) {
        self.home = self.server.clone();
        self.frozen.clear();
    }

    /// True when the display order is the server order and nothing is frozen.
    pub fn is_server_order(&self, present: &[SharedString]) -> bool {
        !self.any_frozen() && self.display_names(present) == server_order(&self.server, present)
    }

    pub fn is_frozen(&self, name: &str) -> bool {
        self.frozen.iter().any(|n| n.as_ref() == name)
    }

    pub fn any_frozen(&self) -> bool {
        !self.frozen.is_empty()
    }

    /// How many of `columns` are frozen. They occupy the left of the display order.
    pub fn pinned(&self, columns: &[GridColumn]) -> usize {
        self.frozen
            .iter()
            .filter(|n| columns.iter().any(|c| &c.name == *n))
            .count()
    }

    /// Remembers new names at the end of `home`. Names that are absent this page stay put,
    /// so hiding a column and showing it again puts it back.
    pub fn sync(&mut self, present: &[SharedString]) {
        for name in present {
            if !self.home.iter().any(|n| n == name) {
                self.home.push(name.clone());
            }
        }
    }

    pub fn display_names(&self, present: &[SharedString]) -> Vec<SharedString> {
        let has = |n: &SharedString| present.iter().any(|p| p == n);
        let mut out = Vec::with_capacity(present.len());
        for name in &self.frozen {
            if has(name) && !out.iter().any(|n| n == name) {
                out.push(name.clone());
            }
        }
        for name in &self.home {
            if has(name) && !self.is_frozen(name) && !out.iter().any(|n| n == name) {
                out.push(name.clone());
            }
        }
        for name in present {
            if !out.iter().any(|n| n == name) {
                out.push(name.clone());
            }
        }
        out
    }

    /// `from` and `to` are indexes in [`Self::display_names`]. A frozen column only
    /// reorders `frozen`; any other column only reorders the unfrozen slots of `home`.
    pub fn drag(&mut self, present: &[SharedString], from: usize, to: usize) {
        let mut display = self.display_names(present);
        if from >= display.len() || to >= display.len() || from == to {
            return;
        }
        let name = display.remove(from);
        let frozen = self.is_frozen(&name);
        display.insert(to, name);
        if frozen {
            let prefix: Vec<_> = display.into_iter().filter(|n| self.is_frozen(n)).collect();
            rewrite(&mut self.frozen, &prefix);
        } else {
            let rest: Vec<_> = display.into_iter().filter(|n| !self.is_frozen(n)).collect();
            rewrite(&mut self.home, &rest);
        }
    }

    /// Inserts `name` just after the frozen columns. Already frozen, or an unknown name, is a no-op.
    pub fn freeze_column(&mut self, present: &[SharedString], name: &SharedString) -> bool {
        if self.is_frozen(name) || !present.iter().any(|n| n == name) {
            return false;
        }
        self.frozen.push(name.clone());
        true
    }

    /// Freezes the display prefix through `index` without moving columns.
    pub fn freeze_through(&mut self, present: &[SharedString], index: usize) {
        let display = self.display_names(present);
        if display.is_empty() {
            return;
        }
        let end = index.min(display.len() - 1);
        self.frozen = display[..=end].to_vec();
    }

    pub fn unfreeze_all(&mut self) {
        self.frozen.clear();
    }
}

/// Replaces the slots that appear in `order` with `order`, and leaves the others where they are.
fn rewrite(slots: &mut [SharedString], order: &[SharedString]) {
    let mut it = order.iter();
    for slot in slots.iter_mut() {
        if order.iter().any(|n| n == slot)
            && let Some(next) = it.next()
        {
            *slot = next.clone();
        }
    }
}

fn contains(hay: &str, needle: &str, case_sensitive: bool) -> bool {
    if needle.is_empty() {
        return false;
    }
    if case_sensitive {
        hay.contains(needle)
    } else {
        let n: Vec<char> = needle.chars().collect();
        let h: Vec<char> = hay.chars().collect();
        h.windows(n.len()).any(|w| {
            w.iter()
                .zip(&n)
                .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
        })
    }
}

fn replace_text(
    hay: &str,
    needle: &str,
    replacement: &str,
    case_sensitive: bool,
) -> Option<String> {
    if needle.is_empty() || !contains(hay, needle, case_sensitive) {
        return None;
    }
    if case_sensitive {
        return Some(hay.replace(needle, replacement));
    }
    let h: Vec<char> = hay.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < h.len() {
        let hit = i + n.len() <= h.len()
            && h[i..i + n.len()]
                .iter()
                .zip(&n)
                .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()));
        if hit {
            out.push_str(replacement);
            i += n.len();
        } else {
            out.push(h[i]);
            i += 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(pk: bool) -> GridModel {
        let col = |n: &str, pk| GridColumn {
            name: n.to_string().into(),
            type_name: "".into(),
            primary_key: pk,
            comment: None,
            read_only: false,
        };
        let mut m = GridModel::default();
        let cell = |s: &str| Some(SharedString::from(s.to_string()));
        m.set_data(
            vec![col("id", pk), col("name", false)],
            vec![vec![cell("1"), cell("a")], vec![cell("2"), None]],
        );
        m
    }

    #[test]
    fn change_set_uses_primary_key_or_whole_row() {
        let mut m = model(true);
        m.set_value(1, 1, Some("b".into()));
        m.set_value(0, 1, Some("a".into())); // setting the original value is not a change
        m.select((0, 0), false);
        m.delete_selected_rows();
        let r = m.insert_row();
        m.set_value(r, 1, Some("c".into()));
        m.insert_row(); // an empty inserted row is not inserted
        assert_eq!(m.row_state(0), RowState::Deleted);
        assert_eq!(m.row_state(1), RowState::Edited);
        let cs = m.change_set();
        let v = |c: &str, s: Option<&str>| (c.to_string(), s.map(String::from));
        let w = |c: &str, s: Option<&str>| (c.to_string(), Written::text(s.map(String::from)));
        assert_eq!(
            cs.updates,
            [RowUpdate {
                key: RowKey(vec![v("id", Some("2"))]),
                values: vec![w("name", Some("b"))]
            }]
        );
        assert_eq!(cs.deletes, [RowKey(vec![v("id", Some("1"))])]);
        assert_eq!(cs.inserts, [vec![w("name", Some("c"))]]);

        let mut m = model(false);
        m.set_value(1, 0, Some("3".into()));
        let key = &m.change_set().updates[0].key;
        assert_eq!(key, &RowKey(vec![v("id", Some("2")), v("name", None)]));
    }

    #[test]
    fn past_the_last_row_is_not_an_insert() {
        let m = model(true);
        assert_eq!(m.row_count(), 2);
        assert_eq!(m.row_state(2), RowState::Clean);
        assert!(!m.row_dirty(2));

        let mut empty = GridModel {
            columns: m.columns.clone(),
            ..GridModel::default()
        };
        assert_eq!(empty.row_count(), 0);
        assert!(!empty.row_dirty(0));
        empty.insert_row();
        assert!(empty.row_dirty(0));
        assert!(!empty.row_dirty(1));
    }

    #[test]
    fn accept_committed_keeps_saved_rows_and_drops_empty_inserts() {
        let mut m = model(true);
        m.set_value(1, 1, Some("b".into()));
        m.select((0, 0), false);
        m.delete_selected_rows();
        let r = m.insert_row();
        m.set_value(r, 1, Some("c".into()));
        m.insert_row();
        m.accept_committed();
        assert!(!m.has_changes());
        assert_eq!(m.row_count(), 2);
        let text = |m: &GridModel, r, c| {
            m.value(r, c)
                .and_then(|cell| cell.as_deref())
                .map(|s| s.to_string())
        };
        assert_eq!(text(&m, 0, 0), Some("2".into()));
        assert_eq!(text(&m, 0, 1), Some("b".into()));
        assert_eq!(text(&m, 1, 1), Some("c".into()));
    }

    #[test]
    fn rectangle_selection_copy_and_paste() {
        let mut m = model(true);
        m.select((1, 1), false);
        m.select((0, 0), true);
        assert_eq!(m.selection(), (0..=1, 0..=1));
        assert_eq!(m.selected_tsv(true, true), "id\tname\n1\ta\n2\t");
        m.select_rows(1, false);
        assert_eq!(m.selected_values(), [vec![Some("2".into()), None]]);

        // Pasting two rows at (1, 1): the second is past the end, so it is appended
        m.select((1, 1), false);
        m.paste("x\ny\n", false);
        assert_eq!(m.value(1, 1), Some(&Some("x".into())));
        assert_eq!(m.row_count(), 3);
        assert_eq!(m.value(2, 1), Some(&Some("y".into())));
        assert_eq!(m.value(2, 0), None);
    }

    #[test]
    fn leaving_a_dirty_row_is_detectable_and_a_column_selects() {
        let mut m = model(true);
        assert!(!m.row_dirty(0));
        m.set_value(0, 1, Some("z".into()));
        assert!(m.row_dirty(0));
        assert!(!m.row_dirty(1));
        m.select_column(1, false);
        assert_eq!(m.selection(), (0..=1, 1..=1));
        m.select_column(0, true);
        assert_eq!(m.selection(), (0..=1, 0..=1));
    }

    #[test]
    fn projection_drops_hidden_columns() {
        let m = model(true);
        let rows = vec![vec![Some("1".into()), Some("a".into())]];
        let (columns, rows) = GridModel::project(&m.columns, &rows, &["id".into()].into());
        assert_eq!(columns[0].name.as_ref(), "name");
        assert_eq!(rows, [vec![Some("a".into())]]);
        // Hiding every column keeps them: the grid would have nothing left to unhide
        let (columns, _) =
            GridModel::project(&m.columns, &[], &["id".into(), "name".into()].into());
        assert_eq!(columns.len(), 2);
    }

    #[test]
    fn hidden_columns_still_locate_rows() {
        let v = |c: &str, s: &str| (c.to_string(), Some(s.to_string()));
        let full = model(true);
        let rows = vec![
            vec![Some("1".into()), Some("a".into())],
            vec![Some("2".into()), Some("a".into())],
        ];
        let hide = |pk: bool, rows: &[Vec<Cell>]| {
            let mut columns = full.columns.clone();
            columns[0].primary_key = pk;
            let hidden: HashSet<String> = ["id".into()].into();
            let (shown, shown_rows) = GridModel::project(&columns, rows, &hidden);
            let mut m = GridModel::default();
            m.set_data(shown, shown_rows);
            m.set_hidden_columns(&columns, rows, &hidden);
            m
        };

        // Hidden primary key: still the key, and the table still counts as keyed
        let mut m = hide(true, &rows);
        assert!(m.has_primary_key());
        m.set_value(1, 0, Some("b".into()));
        assert_eq!(m.change_set().updates[0].key, RowKey(vec![v("id", "2")]));

        // No primary key: the hidden column stays in the whole-row key
        let mut m = hide(false, &rows);
        m.set_value(1, 0, Some("b".into()));
        assert_eq!(
            m.change_set().updates[0].key,
            RowKey(vec![v("name", "a"), v("id", "2")])
        );

        // Deleting a row keeps the hidden values aligned with the remaining rows
        let mut m = hide(true, &rows);
        m.select((0, 0), false);
        m.delete_selected_rows();
        m.accept_committed();
        m.select((0, 0), false);
        m.delete_selected_rows();
        assert_eq!(m.change_set().deletes, [RowKey(vec![v("id", "2")])]);
    }

    #[test]
    fn raw_edits_stay_unquoted_and_reverting_clears_the_flag() {
        let mut m = model(true);
        m.set_raw(0, 1, Some("UPPER('a')".into()));
        let cs = m.change_set();
        assert_eq!(cs.updates[0].values[0].1, Written::expr("UPPER('a')"));
        m.set_value(0, 1, Some("a".into()));
        assert!(m.change_set().updates.is_empty());
        let r = m.insert_row();
        m.set_raw(r, 1, Some("NOW()".into()));
        assert_eq!(m.change_set().inserts[0][0].1, Written::expr("NOW()"));
    }

    #[test]
    fn raw_mode_quotes_string_values_only() {
        let mut m = model(true);
        m.columns[1].type_name = "varchar(45)".into();
        assert_eq!(m.raw_text(0, 1, true, Dialect::Sqlite), "'a'");
        assert_eq!(m.raw_text(0, 0, true, Dialect::Sqlite), "1");
        assert_eq!(m.raw_text(1, 1, true, Dialect::MySql), "");
        m.set_value(0, 1, Some("O'Brien".into()));
        assert_eq!(m.raw_text(0, 1, true, Dialect::MySql), "'O''Brien'");
        assert_eq!(m.raw_text(0, 1, false, Dialect::MySql), "O'Brien");
    }

    #[test]
    fn find_wraps_and_replace_keeps_the_change_set() {
        let mut m = model(true);
        assert_eq!(m.find_cell("A", (0, 0), false, false), Some((0, 1)));
        assert_eq!(m.find_cell("nope", (0, 0), false, false), None);
        assert_eq!(m.find_cell("a", (0, 1), true, true), Some((0, 1)));
        assert_eq!(m.find_cell("2", (0, 1), true, true), Some((1, 0)));
        assert_eq!(m.find_column_name("na", 0, false, false), Some(1));
        assert!(m.replace_cell((0, 1), "a", "z", true));
        assert_eq!(m.value(0, 1), Some(&Some("z".into())));
        assert_eq!(m.replace_all("z", "zz", false), 1);
        assert!(m.has_changes());
    }

    #[test]
    fn read_only_columns_are_not_written() {
        let mut m = model(true);
        m.columns[0].read_only = true;
        m.set_value(0, 0, Some("9".into()));
        assert!(m.change_set().updates.is_empty());
        assert_eq!(m.value(0, 0), Some(&Some("1".into())));
    }

    #[test]
    fn move_column_moves_cells_edits_and_selection() {
        let mut m = model(true);
        m.set_value(0, 1, Some("z".into()));
        m.set_raw(1, 1, Some("NOW()".into()));
        m.select((1, 1), false);
        m.move_column(1, 0);
        assert_eq!(m.columns[0].name.as_ref(), "name");
        assert_eq!(m.columns[1].name.as_ref(), "id");
        assert_eq!(m.value(0, 0).and_then(|c| c.as_deref()), Some("z"));
        assert_eq!(m.value(0, 1).and_then(|c| c.as_deref()), Some("1"));
        assert!(m.raw.contains(&(1, 0)));
        assert_eq!(m.head(), (1, 0));
    }

    fn names(list: &[&str]) -> Vec<SharedString> {
        list.iter()
            .map(|s| SharedString::from(s.to_string()))
            .collect()
    }

    #[test]
    fn freeze_column_moves_to_the_end_of_the_frozen_prefix_and_unfreeze_restores_home() {
        let present = names(&["a", "b", "c", "d"]);
        let mut layout = ColumnLayout::default();
        layout.sync(&present);
        assert!(layout.freeze_column(&present, &present[2]));
        assert_eq!(layout.display_names(&present), names(&["c", "a", "b", "d"]));
        assert!(layout.freeze_column(&present, &present[0]));
        assert_eq!(layout.display_names(&present), names(&["c", "a", "b", "d"]));
        assert!(!layout.freeze_column(&present, &present[2]));
        layout.unfreeze_all();
        assert_eq!(layout.display_names(&present), present);
    }

    #[test]
    fn freeze_through_does_not_move_columns() {
        let present = names(&["a", "b", "c", "d"]);
        let mut layout = ColumnLayout::default();
        layout.sync(&present);
        layout.freeze_through(&present, 1);
        assert_eq!(layout.display_names(&present), present);
        assert!(layout.is_frozen("a") && layout.is_frozen("b"));
        assert!(!layout.is_frozen("c"));
        layout.unfreeze_all();
        assert_eq!(layout.display_names(&present), present);
        assert!(!layout.any_frozen());
    }

    #[test]
    fn restore_server_undoes_drags_and_freezes() {
        let present = names(&["a", "b", "c", "d"]);
        let mut layout = ColumnLayout::default();
        layout.remember_server(&present);
        layout.sync(&present);
        layout.drag(&present, 0, 3);
        assert!(layout.freeze_column(&present, &present[1]));
        assert_ne!(layout.display_names(&present), present);
        layout.restore_server();
        assert_eq!(layout.display_names(&present), present);
        assert!(layout.is_server_order(&present));
    }
}
