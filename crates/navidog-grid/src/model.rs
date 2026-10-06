//! 网格的数据模型：当前页数据、修改覆盖层、新增与待删除行、矩形选区（21-table-viewer.md 第 2 节）。
//! 不依赖界面，单元测试覆盖修改集合与复制粘贴。

use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;

use gpui_kit::SharedString;
use navidog_db::dialect::{ChangeSet, RowKey, RowUpdate, Values};

/// 单元格的值，`None` 为 NULL。
pub type Cell = Option<SharedString>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridColumn {
    pub name: SharedString,
    pub type_name: SharedString,
    pub primary_key: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    Clean,
    Edited,
    Inserted,
    Deleted,
}

/// 行列坐标都是数据坐标（不含左侧标记列）。
pub type Pos = (usize, usize);

#[derive(Default)]
pub struct GridModel {
    pub columns: Vec<GridColumn>,
    rows: Vec<Vec<Cell>>,
    /// 已有行的修改：(行, 列) → 新值
    edits: BTreeMap<Pos, Cell>,
    /// 新增行；`None` 表示该列未填写，插入时使用默认值
    new_rows: Vec<Vec<Option<Cell>>>,
    deleted: BTreeSet<usize>,
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

    pub fn value(&self, r: usize, c: usize) -> Option<&Cell> {
        match r.checked_sub(self.rows.len()) {
            None => self.edits.get(&(r, c)).or_else(|| self.rows[r].get(c)),
            Some(n) => self.new_rows.get(n)?.get(c)?.as_ref(),
        }
    }

    pub fn set_value(&mut self, r: usize, c: usize, v: Cell) {
        match r.checked_sub(self.rows.len()) {
            None if self.rows[r].get(c) == Some(&v) => {
                self.edits.remove(&(r, c));
            }
            None => {
                self.edits.insert((r, c), v);
            }
            Some(n) => {
                if let Some(cell) = self.new_rows.get_mut(n).and_then(|row| row.get_mut(c)) {
                    *cell = Some(v);
                }
            }
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

    pub fn is_edited(&self, r: usize, c: usize) -> bool {
        self.edits.contains_key(&(r, c)) || (r >= self.rows.len() && self.value(r, c).is_some())
    }

    /// 追加一个空行并选中它，返回行号。
    pub fn insert_row(&mut self) -> usize {
        self.new_rows.push(vec![None; self.columns.len()]);
        let r = self.row_count() - 1;
        self.select((r, 0), false);
        r
    }

    /// 删除选区覆盖的行：新增行直接移除，已有行标记为待删除。
    pub fn delete_selected_rows(&mut self) {
        let rows = self.selection().0;
        for r in rows.clone().rev() {
            match r.checked_sub(self.rows.len()) {
                Some(n) if n < self.new_rows.len() => {
                    self.new_rows.remove(n);
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
        let mut updates: BTreeMap<usize, Values> = BTreeMap::new();
        for (&(r, c), v) in &self.edits {
            if !self.deleted.contains(&r) {
                updates
                    .entry(r)
                    .or_default()
                    .push((name(c), v.as_ref().map(|s| s.to_string())));
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
            // 一列都没填的新增行不插入
            inserts: self
                .new_rows
                .iter()
                .map(|row| {
                    row.iter()
                        .enumerate()
                        .filter_map(|(c, v)| {
                            Some((name(c), v.as_ref()?.as_ref().map(|s| s.to_string())))
                        })
                        .collect::<Values>()
                })
                .filter(|v| !v.is_empty())
                .collect(),
            deletes: self.deleted.iter().map(|&r| self.key(r)).collect(),
        }
    }

    // 选区

    pub fn head(&self) -> Pos {
        self.head
    }

    /// 选区覆盖的行与列（闭区间）。
    pub fn selection(&self) -> (RangeInclusive<usize>, RangeInclusive<usize>) {
        let (a, h) = (self.anchor, self.head);
        (a.0.min(h.0)..=a.0.max(h.0), a.1.min(h.1)..=a.1.max(h.1))
    }

    pub fn is_selected(&self, r: usize, c: usize) -> bool {
        let (rows, cols) = self.selection();
        rows.contains(&r) && cols.contains(&c)
    }

    /// 选中一格；`extend` 时保留锚点，选区扩展到该格。
    pub fn select(&mut self, pos: Pos, extend: bool) {
        self.head = pos;
        if !extend {
            self.anchor = pos;
        }
    }

    /// 选中整行（点击左侧标记列）。
    pub fn select_rows(&mut self, r: usize, extend: bool) {
        let last = self.columns.len().saturating_sub(1);
        if !extend {
            self.anchor = (r, 0);
        } else {
            self.anchor.1 = 0;
        }
        self.head = (r, last);
    }

    pub fn select_all(&mut self) {
        self.anchor = (0, 0);
        self.head = (
            self.row_count().saturating_sub(1),
            self.columns.len().saturating_sub(1),
        );
    }

    /// 选区内的行，每行只含选中的列。
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

    /// 选区的制表符分隔文本；NULL 复制为空串。`header` 时首行为字段名。
    // ponytail: 值内的制表符和换行不转义，粘贴回网格时会错位；需要时改为带引号的 TSV
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

    /// 把制表符分隔的文本从选区左上角开始写入，超出已有行数时追加新行。
    pub fn paste(&mut self, text: &str) {
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
                    self.set_value(r, c0 + j, Some(v.to_string().into()));
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
        m.set_value(0, 1, Some("a".into())); // 改回原值不算修改
        m.select((0, 0), false);
        m.delete_selected_rows();
        let r = m.insert_row();
        m.set_value(r, 1, Some("c".into()));
        m.insert_row(); // 空的新增行不插入
        assert_eq!(m.row_state(0), RowState::Deleted);
        assert_eq!(m.row_state(1), RowState::Edited);
        let cs = m.change_set();
        let v = |c: &str, s: Option<&str>| (c.to_string(), s.map(String::from));
        assert_eq!(
            cs.updates,
            [RowUpdate {
                key: RowKey(vec![v("id", Some("2"))]),
                values: vec![v("name", Some("b"))]
            }]
        );
        assert_eq!(cs.deletes, [RowKey(vec![v("id", Some("1"))])]);
        assert_eq!(cs.inserts, [vec![v("name", Some("c"))]]);

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

        // 从 (1, 1) 粘贴两行：第二行超出已有行数，追加为新增行
        m.select((1, 1), false);
        m.paste("x\ny\n");
        assert_eq!(m.value(1, 1), Some(&Some("x".into())));
        assert_eq!(m.row_count(), 3);
        assert_eq!(m.value(2, 1), Some(&Some("y".into())));
        assert_eq!(m.value(2, 0), None);
    }
}
