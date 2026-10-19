//! Result tables as the mysql and psql clients print them (31-query-results.md §7).

use unicode_width::UnicodeWidthStr;

fn cell(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("NULL")
}

fn widths(columns: &[String], rows: &[Vec<Option<String>>]) -> Vec<usize> {
    columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            rows.iter()
                .map(|r| r.get(i).map_or(0, |v| cell(v).width()))
                .chain(std::iter::once(c.width()))
                .max()
                .unwrap_or(0)
        })
        .collect()
}

fn pad(text: &str, width: usize, right: bool) -> String {
    let fill = " ".repeat(width.saturating_sub(text.width()));
    if right {
        format!("{fill}{text}")
    } else {
        format!("{text}{fill}")
    }
}

/// The mysql client's boxed table; `numeric[i]` right-aligns column `i`.
pub fn mysql_table(columns: &[String], rows: &[Vec<Option<String>>], numeric: &[bool]) -> String {
    let w = widths(columns, rows);
    let rule = format!(
        "+{}+\n",
        w.iter()
            .map(|n| "-".repeat(n + 2))
            .collect::<Vec<_>>()
            .join("+")
    );
    let line = |cells: Vec<String>| format!("| {} |\n", cells.join(" | "));
    let mut out = rule.clone();
    out.push_str(&line(
        columns
            .iter()
            .zip(&w)
            .map(|(c, n)| pad(c, *n, false))
            .collect(),
    ));
    out.push_str(&rule);
    for r in rows {
        out.push_str(&line(
            w.iter()
                .enumerate()
                .map(|(i, n)| {
                    let v = r.get(i).map_or("", cell);
                    pad(v, *n, numeric.get(i).copied().unwrap_or(false))
                })
                .collect(),
        ));
    }
    out.push_str(&rule);
    out
}

/// psql's aligned table: centred headers, a dashed rule, no outer border.
pub fn psql_table(columns: &[String], rows: &[Vec<Option<String>>], numeric: &[bool]) -> String {
    let w = widths(columns, rows);
    let centred = |c: &str, n: usize| {
        let len = c.width();
        let left = (n.saturating_sub(len)) / 2;
        format!("{}{c}{}", " ".repeat(left), " ".repeat(n - len - left))
    };
    let join = |cells: Vec<String>| format!(" {}\n", cells.join(" | ").trim_end_matches(' '));
    let mut out = join(
        columns
            .iter()
            .zip(&w)
            .map(|(c, n)| centred(c, *n))
            .collect(),
    );
    out.push_str(&format!(
        "{}\n",
        w.iter()
            .map(|n| "-".repeat(n + 2))
            .collect::<Vec<_>>()
            .join("+")
    ));
    for r in rows {
        out.push_str(&join(
            w.iter()
                .enumerate()
                .map(|(i, n)| {
                    pad(
                        r.get(i).map_or("", cell),
                        *n,
                        numeric.get(i).copied().unwrap_or(false),
                    )
                })
                .collect(),
        ));
    }
    out
}

/// `1 row in set (0.00 sec)`, `2 rows in set …`, or `Empty set …`.
pub fn mysql_rows_footer(rows: usize, secs: f64) -> String {
    match rows {
        0 => format!("Empty set ({secs:.2} sec)"),
        1 => format!("1 row in set ({secs:.2} sec)"),
        n => format!("{n} rows in set ({secs:.2} sec)"),
    }
}

/// `Query OK, 1 row affected (0.01 sec)`.
pub fn mysql_ok(affected: u64, secs: f64) -> String {
    let rows = if affected == 1 { "row" } else { "rows" };
    format!("Query OK, {affected} {rows} affected ({secs:.2} sec)")
}

/// psql's `(1 row)` / `(n rows)`.
pub fn psql_rows_footer(rows: usize) -> String {
    if rows == 1 {
        "(1 row)".into()
    } else {
        format!("({rows} rows)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_columns_align_by_display_width() {
        let columns = vec!["名称".into(), "数量".into()];
        let rows = vec![
            vec![Some("中文".into()), Some("１２".into())],
            vec![Some("e\u{301}".into()), Some("1".into())],
            vec![None, Some("42".into())],
        ];
        let mysql = mysql_table(&columns, &rows, &[false, true]);
        assert!(mysql.lines().all(|line| line.width() == 15));
        assert!(mysql.contains("| e\u{301}    |    1 |"));
        let postgres = psql_table(&columns, &rows, &[false, true]);
        let separators: Vec<_> = postgres
            .lines()
            .map(|line| {
                let offset = line.find(['|', '+']).unwrap();
                line[..offset].width()
            })
            .collect();
        assert!(separators.iter().all(|width| *width == 6));
        assert_eq!(pad("中文", 6, false), "中文  ");
        assert_eq!(pad("e\u{301}", 4, true), "   e\u{301}");
    }

    #[test]
    fn footers() {
        assert_eq!(mysql_rows_footer(0, 0.001), "Empty set (0.00 sec)");
        assert_eq!(mysql_rows_footer(1, 0.0), "1 row in set (0.00 sec)");
        assert_eq!(mysql_rows_footer(3, 0.015), "3 rows in set (0.01 sec)");
        assert_eq!(mysql_ok(1, 0.01), "Query OK, 1 row affected (0.01 sec)");
        assert_eq!(psql_rows_footer(2), "(2 rows)");
    }

    fn data() -> (Vec<String>, Vec<Vec<Option<String>>>) {
        (
            vec!["actor_id".into(), "first_name".into()],
            vec![
                vec![Some("1".into()), Some("PENELOPE".into())],
                vec![Some("12".into()), None],
            ],
        )
    }

    #[test]
    fn mysql_style() {
        let (c, r) = data();
        assert_eq!(
            mysql_table(&c, &r, &[true, false]),
            "+----------+------------+\n\
             | actor_id | first_name |\n\
             +----------+------------+\n\
             |        1 | PENELOPE   |\n\
             |       12 | NULL       |\n\
             +----------+------------+\n"
        );
    }

    #[test]
    fn psql_style() {
        let (c, r) = data();
        assert_eq!(
            psql_table(&c, &r, &[true, false]),
            " actor_id | first_name\n\
             ----------+------------\n\
             \x20       1 | PENELOPE\n\
             \x20      12 | NULL\n"
        );
    }
}
