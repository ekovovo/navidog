//! Query plans (31-query-results.md §5): one tree shape for PostgreSQL `FORMAT JSON`, MySQL and
//! MariaDB `FORMAT=JSON` and SQLite `EXPLAIN QUERY PLAN`, plus cost shares, statistics and the
//! layout of the visual tree.

use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanNode {
    /// Operation ("Seq Scan", "nested_loop", MySQL access type such as "ALL")
    pub op: String,
    pub relation: Option<String>,
    pub rows: Option<f64>,
    /// The node's own cost, without its children; `None` when the plan has no cost
    pub cost: Option<f64>,
    /// Every scalar property, for the node info box
    pub props: Vec<(String, String)>,
    pub children: Vec<PlanNode>,
}

impl PlanNode {
    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Cost of the whole tree: the sum of every node's own cost.
    pub fn total_cost(&self) -> Option<f64> {
        let own = self.cost;
        let kids: Vec<f64> = self
            .children
            .iter()
            .filter_map(|c| c.total_cost())
            .collect();
        if own.is_none() && kids.is_empty() {
            return None;
        }
        Some(own.unwrap_or(0.) + kids.iter().sum::<f64>())
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn scalar_props(obj: &serde_json::Map<String, Value>, out: &mut Vec<(String, String)>) {
    for (k, v) in obj {
        match v {
            Value::String(s) => out.push((k.clone(), s.clone())),
            Value::Number(n) => out.push((k.clone(), n.to_string())),
            Value::Bool(b) => out.push((k.clone(), b.to_string())),
            Value::Array(items) if items.iter().all(|i| !i.is_object()) => out.push((
                k.clone(),
                items
                    .iter()
                    .map(|i| {
                        i.as_str()
                            .map(String::from)
                            .unwrap_or_else(|| i.to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            )),
            _ => {}
        }
    }
}

/// PostgreSQL `EXPLAIN (FORMAT JSON)`. A node's own cost is its `Total Cost` minus its children's.
pub fn parse_postgres(json: &str) -> Option<PlanNode> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    let plan = v.get(0)?.get("Plan")?;
    Some(pg_node(plan))
}

fn pg_node(v: &Value) -> PlanNode {
    let obj = v.as_object().cloned().unwrap_or_default();
    let mut props = Vec::new();
    scalar_props(&obj, &mut props);
    let children: Vec<PlanNode> = obj
        .get("Plans")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(pg_node).collect())
        .unwrap_or_default();
    let total = obj.get("Total Cost").and_then(num);
    let kids_total: f64 = children
        .iter()
        .filter_map(|c| c.prop("Total Cost").and_then(|s| s.parse::<f64>().ok()))
        .sum();
    PlanNode {
        op: obj
            .get("Node Type")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string(),
        relation: obj
            .get("Relation Name")
            .and_then(Value::as_str)
            .map(String::from),
        rows: obj.get("Plan Rows").and_then(num),
        cost: total.map(|t| (t - kids_total).max(0.)),
        props,
        children,
    }
}

/// MySQL 8 and MariaDB `EXPLAIN FORMAT=JSON`. Tables carry the cost: MySQL as
/// `cost_info.read_cost + eval_cost`, MariaDB as `cost`.
pub fn parse_mysql(json: &str) -> Option<PlanNode> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    let block = v.get("query_block")?;
    Some(mysql_block(block))
}

fn mysql_block(v: &Value) -> PlanNode {
    let obj = v.as_object().cloned().unwrap_or_default();
    let mut props = Vec::new();
    scalar_props(&obj, &mut props);
    if let Some(Value::Object(ci)) = obj.get("cost_info") {
        scalar_props(ci, &mut props);
    }
    let id = obj
        .get("select_id")
        .map(|v| v.to_string())
        .unwrap_or_default();
    PlanNode {
        op: format!("query_block #{id}")
            .trim_end_matches(" #")
            .to_string(),
        relation: None,
        rows: None,
        cost: None,
        props,
        children: mysql_children(&obj),
    }
}

/// Structural keys that become nodes, in the order MySQL nests them.
const MYSQL_OPS: &[&str] = &[
    "ordering_operation",
    "grouping_operation",
    "duplicates_removal",
    "windowing",
    "buffer_result",
    "read_sorted_file",
    "filesort",
    "temporary_table",
];

fn mysql_children(obj: &serde_json::Map<String, Value>) -> Vec<PlanNode> {
    let mut out = Vec::new();
    for op in MYSQL_OPS {
        if let Some(Value::Object(inner)) = obj.get(*op) {
            let mut props = Vec::new();
            scalar_props(inner, &mut props);
            out.push(PlanNode {
                op: op.to_string(),
                props,
                children: mysql_children(inner),
                ..PlanNode::default()
            });
        }
    }
    if let Some(Value::Object(t)) = obj.get("table") {
        out.push(mysql_table(t));
    }
    if let Some(Value::Array(items)) = obj.get("nested_loop") {
        out.push(PlanNode {
            op: "nested_loop".into(),
            children: items
                .iter()
                .filter_map(|i| i.get("table").and_then(Value::as_object).map(mysql_table))
                .collect(),
            ..PlanNode::default()
        });
    }
    if let Some(Value::Object(u)) = obj.get("union_result") {
        let mut props = Vec::new();
        scalar_props(u, &mut props);
        let specs = u
            .get("query_specifications")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.get("query_block"))
                    .map(mysql_block)
                    .collect()
            })
            .unwrap_or_default();
        out.push(PlanNode {
            op: "union_result".into(),
            props,
            children: specs,
            ..PlanNode::default()
        });
    }
    for key in [
        "attached_subqueries",
        "optimized_away_subqueries",
        "subqueries",
    ] {
        if let Some(Value::Array(items)) = obj.get(key) {
            out.extend(
                items
                    .iter()
                    .filter_map(|s| s.get("query_block"))
                    .map(mysql_block),
            );
        }
    }
    out
}

fn mysql_table(t: &serde_json::Map<String, Value>) -> PlanNode {
    let mut props = Vec::new();
    scalar_props(t, &mut props);
    let cost = match t.get("cost_info") {
        Some(Value::Object(ci)) => {
            scalar_props(ci, &mut props);
            let read = ci.get("read_cost").and_then(num);
            let eval = ci.get("eval_cost").and_then(num);
            match (read, eval) {
                (None, None) => None,
                (r, e) => Some(r.unwrap_or(0.) + e.unwrap_or(0.)),
            }
        }
        _ => t.get("cost").and_then(num),
    };
    let mut children = Vec::new();
    if let Some(sub) = t
        .get("materialized_from_subquery")
        .and_then(|m| m.get("query_block"))
    {
        children.push(mysql_block(sub));
    }
    PlanNode {
        op: t
            .get("access_type")
            .and_then(Value::as_str)
            .unwrap_or("table")
            .to_string(),
        relation: t
            .get("table_name")
            .and_then(Value::as_str)
            .map(String::from),
        rows: t
            .get("rows_examined_per_scan")
            .or_else(|| t.get("rows"))
            .and_then(num),
        cost,
        props,
        children,
    }
}

/// The plan's total for shares: PostgreSQL's root `Total Cost`, MySQL's `query_cost`, MariaDB's
/// block `cost`; else the sum of the nodes.
pub fn root_cost(root: &PlanNode) -> Option<f64> {
    ["Total Cost", "query_cost", "cost"]
        .iter()
        .find_map(|k| root.prop(k).and_then(|v| v.parse::<f64>().ok()))
        .or_else(|| root.total_cost())
        .filter(|c| *c > 0.)
}

/// A node's own cost as a percentage of the plan.
pub fn share(node: &PlanNode, root_total: Option<f64>) -> Option<f64> {
    Some(node.cost? / root_total? * 100.)
}

/// SQLite `EXPLAIN QUERY PLAN` rows (id, parent, detail) as a tree under a "QUERY PLAN" root.
pub fn parse_sqlite(rows: &[(i64, i64, String)]) -> PlanNode {
    fn kids(rows: &[(i64, i64, String)], parent: i64) -> Vec<PlanNode> {
        rows.iter()
            .filter(|(_, p, _)| *p == parent)
            .map(|(id, _, detail)| {
                let mut words = detail.split_whitespace();
                let op = words.next().unwrap_or("").to_string();
                let relation = matches!(op.as_str(), "SCAN" | "SEARCH")
                    .then(|| words.next().map(String::from))
                    .flatten();
                PlanNode {
                    op,
                    relation,
                    props: vec![("detail".into(), detail.clone())],
                    children: kids(rows, *id),
                    ..PlanNode::default()
                }
            })
            .collect()
    }
    PlanNode {
        op: "QUERY PLAN".into(),
        children: kids(rows, 0),
        ..PlanNode::default()
    }
}

/// SQLite plan rows as the indented tree the sqlite3 shell prints.
pub fn sqlite_text(rows: &[(i64, i64, String)]) -> String {
    fn walk(rows: &[(i64, i64, String)], parent: i64, depth: usize, out: &mut String) {
        let level: Vec<_> = rows.iter().filter(|(_, p, _)| *p == parent).collect();
        for (i, (id, _, detail)) in level.iter().enumerate() {
            let branch = if i + 1 == level.len() { "`--" } else { "|--" };
            out.push_str(&format!("{}{branch}{detail}\n", "   ".repeat(depth)));
            walk(rows, *id, depth + 1, out);
        }
    }
    let mut out = String::from("QUERY PLAN\n");
    walk(rows, 0, 0, &mut out);
    out
}

/// A result set as the text table MariaDB's plan has no other form of.
pub fn text_table(columns: &[String], rows: &[Vec<Option<String>>]) -> String {
    crate::console::mysql_table(columns, rows, &vec![false; columns.len()])
}

/// Nodes depth first with their depth, for the grid view.
pub fn flatten(root: &PlanNode) -> Vec<(usize, &PlanNode)> {
    fn walk<'a>(n: &'a PlanNode, depth: usize, out: &mut Vec<(usize, &'a PlanNode)>) {
        out.push((depth, n));
        for c in &n.children {
            walk(c, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeStat {
    pub node_type: String,
    pub count: usize,
    pub cost: f64,
    pub percent: f64,
}

/// "Per node type" statistics: costs add up to the root's total.
pub fn stats_by_type(root: &PlanNode) -> Vec<TypeStat> {
    let total = root_cost(root).unwrap_or(0.);
    let mut out: Vec<TypeStat> = Vec::new();
    for (_, n) in flatten(root) {
        let cost = n.cost.unwrap_or(0.);
        match out.iter_mut().find(|s| s.node_type == n.op) {
            Some(s) => {
                s.count += 1;
                s.cost += cost;
            }
            None => out.push(TypeStat {
                node_type: n.op.clone(),
                count: 1,
                cost,
                percent: 0.,
            }),
        }
    }
    for s in &mut out {
        s.percent = if total > 0. {
            s.cost / total * 100.
        } else {
            0.
        };
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelationStat {
    pub relation: String,
    /// Nodes reading this relation
    pub scans: usize,
    pub node_type: String,
    pub count: usize,
    pub cost: f64,
    /// Share of this relation's cost
    pub percent: f64,
}

/// "Per relation" statistics: one row per relation and node type.
pub fn stats_by_relation(root: &PlanNode) -> Vec<RelationStat> {
    let mut out: Vec<RelationStat> = Vec::new();
    for (_, n) in flatten(root) {
        let Some(rel) = &n.relation else { continue };
        let cost = n.cost.unwrap_or(0.);
        match out
            .iter_mut()
            .find(|s| &s.relation == rel && s.node_type == n.op)
        {
            Some(s) => {
                s.count += 1;
                s.cost += cost;
            }
            None => out.push(RelationStat {
                relation: rel.clone(),
                scans: 0,
                node_type: n.op.clone(),
                count: 1,
                cost,
                percent: 0.,
            }),
        }
    }
    let totals: Vec<(String, usize, f64)> = out
        .iter()
        .map(|s| s.relation.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|r| {
            let rows = out.iter().filter(|s| s.relation == r);
            let (n, c) = rows.fold((0, 0.), |(n, c), s| (n + s.count, c + s.cost));
            (r, n, c)
        })
        .collect();
    for s in &mut out {
        if let Some((_, scans, cost)) = totals.iter().find(|(r, _, _)| *r == s.relation) {
            s.scans = *scans;
            s.percent = if *cost > 0. { s.cost / cost * 100. } else { 0. };
        }
    }
    out
}

/// Visual tree layout in node slots: leaves on the left, the root on the right (31 §5). Each entry
/// is (column, row) for the node at the same index in [`flatten`]; columns count from the left,
/// rows are fractional so a parent sits level with the middle of its children.
pub fn layout(root: &PlanNode) -> Vec<(usize, f32)> {
    fn depth(n: &PlanNode) -> usize {
        1 + n.children.iter().map(depth).max().unwrap_or(0)
    }
    fn walk(
        n: &PlanNode,
        d: usize,
        max: usize,
        next_leaf: &mut f32,
        out: &mut Vec<(usize, f32)>,
    ) -> f32 {
        let ix = out.len();
        out.push((max - 1 - d, 0.));
        let row = if n.children.is_empty() {
            let r = *next_leaf;
            *next_leaf += 1.;
            r
        } else {
            let rows: Vec<f32> = n
                .children
                .iter()
                .map(|c| walk(c, d + 1, max, next_leaf, out))
                .collect();
            (rows[0] + rows[rows.len() - 1]) / 2.
        };
        out[ix].1 = row;
        row
    }
    let max = depth(root);
    let mut out = Vec::new();
    walk(root, 0, max, &mut 0., &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PG: &str = r#"[{"Plan": {"Node Type": "Hash Join", "Startup Cost": 1.5, "Total Cost": 10.0, "Plan Rows": 5, "Plan Width": 8,
        "Plans": [
          {"Node Type": "Seq Scan", "Relation Name": "film", "Startup Cost": 0, "Total Cost": 6.0, "Plan Rows": 100, "Plan Width": 4},
          {"Node Type": "Hash", "Startup Cost": 0, "Total Cost": 3.0, "Plan Rows": 10, "Plan Width": 4,
           "Plans": [{"Node Type": "Seq Scan", "Relation Name": "actor", "Startup Cost": 0, "Total Cost": 2.5, "Plan Rows": 10, "Plan Width": 4}]}
        ]}}]"#;

    #[test]
    fn postgres_costs_add_up_to_the_root() {
        let root = parse_postgres(PG).unwrap();
        assert_eq!(root.op, "Hash Join");
        assert_eq!(root.cost, Some(1.0));
        assert_eq!(root.children[1].cost, Some(0.5));
        let total = root_cost(&root);
        assert_eq!(total, Some(10.0));
        let sum: f64 = stats_by_type(&root).iter().map(|s| s.cost).sum();
        assert!((sum - 10.0).abs() < 1e-9);
        let seq = stats_by_type(&root)
            .into_iter()
            .find(|s| s.node_type == "Seq Scan")
            .unwrap();
        assert_eq!((seq.count, seq.cost), (2, 8.5));
        assert_eq!(share(&root.children[0], total), Some(60.0));
        let rel = stats_by_relation(&root);
        assert_eq!(rel.len(), 2);
        assert!(
            rel.iter()
                .all(|r| r.scans == 1 && (r.percent - 100.).abs() < 1e-9)
        );
    }

    #[test]
    fn mysql_and_mariadb_tables_carry_the_cost() {
        let my = r#"{"query_block": {"select_id": 1, "cost_info": {"query_cost": "20.25"},
            "ordering_operation": {"using_filesort": true, "nested_loop": [
              {"table": {"table_name": "a", "access_type": "ALL", "rows_examined_per_scan": 200,
                         "cost_info": {"read_cost": "0.25", "eval_cost": "20.00"}}},
              {"table": {"table_name": "b", "access_type": "eq_ref", "rows_examined_per_scan": 1,
                         "cost_info": {"read_cost": "0", "eval_cost": "0"}}}]}}}"#;
        let root = parse_mysql(my).unwrap();
        assert_eq!(root.op, "query_block #1");
        assert_eq!(root_cost(&root), Some(20.25));
        let sort = &root.children[0];
        assert_eq!(sort.op, "ordering_operation");
        let a = &sort.children[0].children[0];
        assert_eq!(
            (a.relation.as_deref(), a.rows, a.cost),
            (Some("a"), Some(200.), Some(20.25))
        );
        assert_eq!(share(a, root_cost(&root)), Some(100.));

        let maria = r#"{"query_block": {"select_id": 1, "cost": 0.05,
            "nested_loop": [{"table": {"table_name": "actor", "access_type": "ALL", "rows": 200, "cost": 0.04}}]}}"#;
        let root = parse_mysql(maria).unwrap();
        let t = &root.children[0].children[0];
        assert_eq!((t.op.as_str(), t.cost), ("ALL", Some(0.04)));
        assert!((share(t, root_cost(&root)).unwrap() - 80.).abs() < 1e-9);
    }

    #[test]
    fn sqlite_rows_form_a_tree() {
        let rows = vec![
            (2, 0, "SCAN a".to_string()),
            (
                5,
                0,
                "SEARCH b USING INTEGER PRIMARY KEY (rowid=?)".to_string(),
            ),
            (9, 5, "CORRELATED SCALAR SUBQUERY 1".to_string()),
        ];
        let root = parse_sqlite(&rows);
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[1].relation.as_deref(), Some("b"));
        assert_eq!(root.children[1].children.len(), 1);
        assert_eq!(
            sqlite_text(&rows),
            "QUERY PLAN\n|--SCAN a\n`--SEARCH b USING INTEGER PRIMARY KEY (rowid=?)\n   `--CORRELATED SCALAR SUBQUERY 1\n"
        );
        assert!(root_cost(&root).is_none());
    }

    #[test]
    fn layout_puts_leaves_left_and_centres_parents() {
        let root = parse_postgres(PG).unwrap();
        let slots = layout(&root);
        // Depth 3: root in column 2, leaves further left
        assert_eq!(slots[0].0, 2);
        let flat = flatten(&root);
        let film = flat
            .iter()
            .position(|(_, n)| n.relation.as_deref() == Some("film"))
            .unwrap();
        let actor = flat
            .iter()
            .position(|(_, n)| n.relation.as_deref() == Some("actor"))
            .unwrap();
        assert_eq!(slots[film], (1, 0.));
        assert_eq!(slots[actor], (0, 1.));
        assert_eq!(slots[0].1, 0.5);
    }
}
