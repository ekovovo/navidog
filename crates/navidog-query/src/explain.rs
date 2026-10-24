//! Explain tabs (31-query-results.md §5): plans fetched without running the statement, shown as a
//! grid, text and JSON.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::{Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div,
};
use navidog_db::dialect::Dialect;
use navidog_db::{Column, DbResult, Relational, ResultSet};
use navidog_grid::{GridColumn, ResultGrid};
use navidog_ui::skin::truncated;
use navidog_ui::{palette, tokens};
use navidog_util::explain::{self as plan, PlanNode};
use rust_i18n::t;

use crate::editor::QueryEditor;
use crate::results::{Body, RunInfo};
use crate::run::{Outcome, Planned};

/// What the server said about one statement's plan.
#[derive(Clone, Debug, Default)]
pub(crate) struct Plan {
    /// Classic `EXPLAIN` rows (MySQL, MariaDB, SQLite)
    pub grid: Option<ResultSet>,
    /// MySQL `FORMAT=TREE`, PostgreSQL text, SQLite's indented tree
    pub text: Option<String>,
    /// `FORMAT=JSON` / `FORMAT JSON`
    pub json: Option<String>,
    pub tree: Option<PlanNode>,
}

fn first_text(sets: &[ResultSet]) -> String {
    sets.iter()
        .flat_map(|s| &s.rows)
        .filter_map(|r| r.first().cloned().flatten())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Fetches a plan the way each dialect offers it (31 §5); only the first query may fail.
pub(crate) async fn fetch_plan(
    rel: &mut dyn Relational,
    dialect: Dialect,
    mariadb: bool,
    sql: &str,
) -> DbResult<Plan> {
    let mut p = Plan::default();
    match dialect {
        Dialect::MySql => {
            let grid = rel.execute(&format!("EXPLAIN {sql}")).await?;
            p.grid = grid.into_iter().find(|s| !s.columns.is_empty());
            p.json = rel
                .execute(&format!("EXPLAIN FORMAT=JSON {sql}"))
                .await
                .ok()
                .map(|s| first_text(&s));
            p.text = if mariadb {
                p.grid.as_ref().map(|g| {
                    let names: Vec<String> = g.columns.iter().map(|c| c.name.clone()).collect();
                    plan::text_table(&names, &g.rows)
                })
            } else {
                rel.execute(&format!("EXPLAIN FORMAT=TREE {sql}"))
                    .await
                    .ok()
                    .map(|s| first_text(&s))
            };
            p.tree = p.json.as_deref().and_then(plan::parse_mysql);
        }
        Dialect::Postgres => {
            let json = rel.execute(&format!("EXPLAIN (FORMAT JSON) {sql}")).await?;
            p.json = Some(first_text(&json));
            p.text = rel
                .execute(&format!("EXPLAIN {sql}"))
                .await
                .ok()
                .map(|s| first_text(&s));
            p.tree = p.json.as_deref().and_then(plan::parse_postgres);
            p.grid = p.tree.as_ref().map(postgres_grid);
        }
        Dialect::Sqlite => {
            let rows = rel.execute(&format!("EXPLAIN QUERY PLAN {sql}")).await?;
            let set = rows
                .into_iter()
                .find(|s| !s.columns.is_empty())
                .unwrap_or_default();
            let items: Vec<(i64, i64, String)> = set
                .rows
                .iter()
                .map(|r| {
                    let num = |i: usize| {
                        r.get(i)
                            .cloned()
                            .flatten()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0)
                    };
                    (
                        num(0),
                        num(1),
                        r.get(3).cloned().flatten().unwrap_or_default(),
                    )
                })
                .collect();
            p.text = Some(plan::sqlite_text(&items));
            p.tree = Some(plan::parse_sqlite(&items));
            p.grid = Some(set);
        }
    }
    Ok(p)
}

/// PostgreSQL plan nodes as grid rows, indented by depth.
fn postgres_grid(root: &PlanNode) -> ResultSet {
    let names = [
        "Node Type",
        "Relation",
        "Startup Cost",
        "Total Cost",
        "Rows",
        "Width",
    ];
    let prop = |n: &PlanNode, k: &str| n.prop(k).map(String::from);
    ResultSet {
        columns: names
            .iter()
            .map(|n| Column {
                name: n.to_string(),
                ..Column::default()
            })
            .collect(),
        rows: plan::flatten(root)
            .into_iter()
            .map(|(depth, n)| {
                vec![
                    Some(format!("{}{}", "    ".repeat(depth), n.op)),
                    n.relation.clone(),
                    prop(n, "Startup Cost"),
                    prop(n, "Total Cost"),
                    prop(n, "Plan Rows"),
                    prop(n, "Plan Width"),
                ]
            })
            .collect(),
        ..ResultSet::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum View {
    Visual,
    Grid,
    Text,
    Json,
    Stats,
    Info,
}

pub(crate) struct ExplainView {
    plan: Plan,
    sql: String,
    info: RunInfo,
    view: View,
    grid: Option<Entity<ResultGrid>>,
    text: Entity<EditorState>,
    json: Entity<EditorState>,
    pub(crate) visual: crate::explain_visual::Visual,
    _subscriptions: Vec<gpui_kit::Subscription>,
}

impl ExplainView {
    pub fn new(
        plan: Plan,
        sql: String,
        info: RunInfo,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let grid = plan.grid.as_ref().map(|set| {
            let grid = cx.new(|cx| ResultGrid::new(window, cx));
            let columns = set
                .columns
                .iter()
                .map(|c| GridColumn {
                    name: c.name.clone().into(),
                    type_name: c.type_name.clone().into(),
                    ..GridColumn::default()
                })
                .collect();
            let rows = set
                .rows
                .iter()
                .map(|r| r.iter().map(|v| v.clone().map(Into::into)).collect())
                .collect();
            grid.update(cx, |g, cx| {
                g.set_quick_filter(false);
                g.set_data(columns, rows, cx);
                g.set_locked(true, cx);
            });
            grid
        });
        let editor = |language: &'static str, text: String, window: &mut Window, cx: &mut App| {
            let e = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language(language)
                    .line_number(false)
            });
            e.update(cx, |e, cx| {
                e.set_value(text, window, cx);
                e.set_readonly(true, cx);
            });
            e
        };
        let text = editor("text", plan.text.clone().unwrap_or_default(), window, cx);
        let json = editor(
            "json",
            plan.json
                .as_deref()
                .and_then(navidog_util::format::pretty_json)
                .or(plan.json.clone())
                .unwrap_or_default(),
            window,
            cx,
        );
        let visual = crate::explain_visual::Visual::new(plan.tree.clone(), window, cx);
        let view = if plan.tree.is_some() && plan.json.is_some() {
            View::Visual
        } else {
            View::Grid
        };
        cx.new(|cx| {
            let subs = vec![
                cx.observe(&visual.threshold, |_, _, cx| cx.notify()),
                cx.observe(&visual.search, |_, _, cx| cx.notify()),
            ];
            Self {
                plan,
                sql,
                info,
                view,
                grid,
                text,
                json,
                visual,
                _subscriptions: subs,
            }
        })
    }

    fn views(&self) -> Vec<View> {
        let mut out = Vec::new();
        let has_json = self.plan.json.is_some();
        if self.plan.tree.is_some() && has_json {
            out.push(View::Visual);
        }
        if self.grid.is_some() {
            out.push(View::Grid);
        }
        if self.plan.text.is_some() {
            out.push(View::Text);
        }
        if has_json {
            out.push(View::Json);
            if self.plan.tree.is_some() {
                out.push(View::Stats);
            }
        }
        out.push(View::Info);
        out
    }

    fn render_info(&self, cx: &App) -> AnyElement {
        let p = palette(cx);
        v_flex()
            .p(tokens::info::INSET.rems())
            .gap(tokens::info::ROW_GAP.rems())
            .children(
                self.info
                    .lines(&self.sql)
                    .into_iter()
                    .enumerate()
                    .map(|(ix, (k, v))| {
                        v_flex()
                            .min_w_0()
                            .child(
                                truncated(("explain-info-key", ix), k)
                                    .text_size(tokens::font::SIZE_SMALL.rems())
                                    .text_color(p.label_secondary),
                            )
                            .child(truncated(("explain-info-value", ix), v).text_color(p.label))
                    }),
            )
            .into_any_element()
    }
}

impl Render for ExplainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let views = self.views();
        if !views.contains(&self.view) {
            self.view = views[0];
        }
        let label = |v: View| {
            match v {
                View::Visual => t!("query.explain_visual"),
                View::Grid => t!("query.explain_grid"),
                View::Text => t!("query.explain_text"),
                View::Json => t!("query.explain_json"),
                View::Stats => t!("query.explain_stats"),
                View::Info => t!("query.explain_info"),
            }
            .to_string()
        };
        let body: AnyElement = match self.view {
            View::Visual => self.render_visual(window, cx),
            View::Grid => self
                .grid
                .clone()
                .map(|g| g.into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            View::Text => Editor::new(&self.text)
                .bordered(false)
                .size_full()
                .into_any_element(),
            View::Json => Editor::new(&self.json)
                .bordered(false)
                .size_full()
                .into_any_element(),
            View::Stats => self.render_stats(cx),
            View::Info => self.render_info(cx),
        };
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .h(tokens::explain::VIEW_SWITCH_HEIGHT.rems())
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(p.separator)
                    .children(views.into_iter().map(|v| {
                        Button::new(SharedString::from(format!("explain-view-{v:?}")))
                            .ghost()
                            .small()
                            .label(label(v))
                            .selected(self.view == v)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.view = v;
                                cx.notify();
                            }))
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(body),
            )
    }
}

impl QueryEditor {
    pub(crate) fn record_plan(
        &mut self,
        planned: Planned,
        result: DbResult<Plan>,
        info: RunInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let started = info.at;
        let elapsed = info.query_time;
        match result {
            Ok(plan) => {
                let name = self.results.next_explain_name();
                let view = ExplainView::new(plan, planned.sql.clone(), info.clone(), window, cx);
                self.results
                    .add_tab(name, Body::Explain(view), info, planned.sql.clone());
                let outcome = Outcome {
                    planned,
                    result: Ok(Vec::new()),
                    started,
                    elapsed,
                };
                self.results.note(outcome, window, cx);
            }
            Err(e) => {
                self.mark_error(planned.range.clone(), &crate::errors::error_message(&e), cx);
                let outcome = Outcome {
                    planned,
                    result: Err(e),
                    started,
                    elapsed,
                };
                self.results.note(outcome, window, cx);
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use navidog_db::mysql::MySqlDriver;
    use navidog_db::postgres::PostgresDriver;
    use navidog_db::{ConnectParams, Driver, connect, runtime};

    use super::*;

    /// Real plans from the docker-compose servers parse into trees with costs (31 §5).
    /// `docker compose up -d --wait && NAVIDOG_LIVE=1 cargo test -p navidog-query live_plans`
    #[test]
    fn live_plans_parse_on_every_server() {
        if std::env::var_os("NAVIDOG_LIVE").is_none() {
            return;
        }
        let sql = "SELECT a.first_name FROM actor a JOIN film_actor fa ON fa.actor_id = a.actor_id WHERE a.actor_id < 5";
        runtime::runtime().block_on(async {
            for (driver, port, mariadb) in [
                (MySqlDriver::mysql(), 13306, false),
                (MySqlDriver::mariadb(), 13307, true),
            ] {
                let params = ConnectParams {
                    host: "127.0.0.1".into(),
                    port,
                    user: "root".into(),
                    password: Some("navidog".into()),
                    database: Some("sakila".into()),
                    ..Default::default()
                };
                let driver: Arc<dyn Driver> = Arc::new(driver);
                let mut s = connect(driver, params).await.unwrap();
                let p = fetch_plan(s.relational().unwrap(), Dialect::MySql, mariadb, sql)
                    .await
                    .unwrap();
                assert!(
                    p.grid.is_some() && p.json.is_some() && p.text.is_some(),
                    "{p:?}"
                );
                let tree = p.tree.expect("the JSON plan parses");
                assert!(
                    plan::flatten(&tree)
                        .iter()
                        .any(|(_, n)| n.relation.as_deref() == Some("a")),
                    "{tree:?}"
                );
                assert!(plan::root_cost(&tree).is_some(), "{mariadb}: {tree:?}");
                s.close().await.unwrap();
            }
            let params = ConnectParams {
                host: "127.0.0.1".into(),
                port: 15432,
                user: "postgres".into(),
                password: Some("navidog".into()),
                database: Some("pagila".into()),
                ..Default::default()
            };
            let mut s = connect(Arc::new(PostgresDriver::default()), params)
                .await
                .unwrap();
            let p = fetch_plan(
                s.relational().unwrap(),
                Dialect::Postgres,
                false,
                "SELECT * FROM actor a JOIN film f ON f.film_id = a.actor_id WHERE a.actor_id < 5",
            )
            .await
            .unwrap();
            let tree = p.tree.expect("the JSON plan parses");
            let total = plan::root_cost(&tree).unwrap();
            let sum: f64 = plan::stats_by_type(&tree).iter().map(|s| s.cost).sum();
            assert!((sum - total).abs() < 1e-6, "{sum} vs {total}");
            assert!(p.grid.unwrap().rows.len() >= 2 && p.text.is_some());
            s.close().await.unwrap();
        });
    }
}
