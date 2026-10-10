//! Result tables as the mysql and psql clients print them (31-query-results.md §7).
//! ponytail: widths count characters, so wide CJK text shifts the columns; a display-width table
//! would fix that.

fn cell(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("NULL")
}

fn widths(columns: &[String], rows: &[Vec<Option<String>>]) -> Vec<usize> {
    columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            rows.iter()
                .map(|r| r.get(i).map_or(0, |v| cell(v).chars().count()))
                .chain(std::iter::once(c.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect()
}

fn pad(text: &str, width: usize, right: bool) -> String {
    let fill = " ".repeat(width.saturating_sub(text.chars().count()));
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
        let len = c.chars().count();
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

#[cfg(test)]
mod tests {
    use super::*;

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
