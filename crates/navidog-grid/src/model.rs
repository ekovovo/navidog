//! Grid data model: the current page, the edit overlay, inserted and pending-delete rows, and the
//! rectangular selection (21-table-viewer.md §2). No UI here; unit tests cover the change set and copy/paste.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;

use gpui_kit::SharedString;
use navidog_db::dialect::{ChangeSet, Dialect, RowKey, RowUpdate, Writes, Written};

/// A cell value; `None` inside the `Option` is NULL, and a missing cell is an unfilled new row.
pub type Cell = Option<SharedString>;

/// Character types are quoted in raw data mode. Accepts SQL names (`varchar(45)`,
/// `character varying`) and the MySQL protocol names the driver puts on a result column.
pub fn is_string_type(type_name: &str) -> bool {
    let t = type_name.to_ascii_lowercase();
    if t.contains("mysql_type_string")
        || t.contains("mysql_type_var_string")
        || t.contains("mysql_type_varchar")
        || t.contains("mysql_type_enum")
        || t.contains("mysql_type_set")
        || t.contains("mysql_type_json")
    {
        return true;
    }
    let base = t
        .split(|ch: char| ch == '(' || ch.is_whitespace())
        .next()
        .unwrap_or("");
    matches!(
        base,
        "char"
            | "varchar"
            | "nchar"
            | "nvarchar"
            | "character"
            | "bpchar"
            | "text"
            | "tinytext"
            | "mediumtext"
            | "longtext"
            | "ntext"
            | "clob"
            | "citext"
            | "string"
            | "enum"
            | "set"
            | "json"
            | "jsonb"
            | "xml"
            | "uuid"
            | "name"
    )
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridColumn {
    pub name: SharedString,
    pub type_name: SharedString,
    pub primary_key: bool,
    pub comment: Option<SharedString>,
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
    anchor: Pos,
    head: Pos,
}

impl GridModel {
    pub fn set_data(&mut self, columns: Vec<GridColumn>, rows: Vec<Vec<Cell>>) {
        self.columns = columns;
        self.rows = rows;
        self.discard();
        let max = (
            self.row_count().saturating_sub(1),
            self.columns.len().saturating_sub(1),
        );
        self.anchor = (self.anchor.0.min(max.0), self.anchor.1.min(max.1));
        self.head = self.anchor;
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
        if r >= self.rows.len() {
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

    pub fn has_primary_key(&self) -> bool {
        self.columns.iter().any(|c| c.primary_key)
    }

    fn key(&self, r: usize) -> RowKey {
        let pk = self.has_primary_key();
        RowKey(
            self.columns
                .iter()
                .zip(&self.rows[r])
                .filter(|(col, _)| !pk || col.primary_key)
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
            if !self.deleted.contains(&r) {
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
                        .filter_map(|(c, v)| Some(written(r, c, v.as_ref()?)))
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

    /// Columns and cell values with `hidden` names removed. Hiding every column keeps them all,
    /// since the grid could not show anything to unhide.
    pub fn project(
        columns: &[GridColumn],
        rows: &[Vec<Cell>],
        hidden: &std::collections::HashSet<String>,
    ) -> (Vec<GridColumn>, Vec<Vec<Cell>>) {
        let keep: Vec<usize> = columns
            .iter()
            .enumerate()
            .filter(|(_, c)| hidden.len() >= columns.len() || !hidden.contains(c.name.as_ref()))
            .map(|(i, _)| i)
            .collect();
        let columns = keep.iter().map(|i| columns[*i].clone()).collect();
        let rows = rows
            .iter()
            .map(|r| keep.iter().map(|i| r.get(*i).cloned().flatten()).collect())
            .collect();
        (columns, rows)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn model(pk: bool) -> GridModel {
        let col = |n: &str, pk| GridColumn {
            name: n.to_string().into(),
            type_name: "".into(),
            primary_key: pk,
            comment: None,
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
        assert!(is_string_type("varchar(45)"));
        assert!(is_string_type("character varying(20)"));
        assert!(is_string_type("TEXT"));
        assert!(is_string_type("MYSQL_TYPE_VAR_STRING"));
        assert!(!is_string_type("int"));
        assert!(!is_string_type("timestamp without time zone"));
        assert!(!is_string_type("MYSQL_TYPE_LONG"));
        let mut m = model(true);
        m.columns[1].type_name = "varchar(45)".into();
        assert_eq!(m.raw_text(0, 1, true, Dialect::Sqlite), "'a'");
        assert_eq!(m.raw_text(0, 0, true, Dialect::Sqlite), "1");
        assert_eq!(m.raw_text(1, 1, true, Dialect::MySql), "");
        m.set_value(0, 1, Some("O'Brien".into()));
        assert_eq!(m.raw_text(0, 1, true, Dialect::MySql), "'O''Brien'");
        assert_eq!(m.raw_text(0, 1, false, Dialect::MySql), "O'Brien");
    }
}
