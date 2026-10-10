//! Visual explain and statistics (31-query-results.md §5): the plan tree drawn leaves left, root
//! right, with cost shares, a threshold, zoom, a node-type filter, search and a node info box.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::{Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, Entity,
    InteractiveElement as _, IntoElement, ParentElement as _, PathBuilder, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, point, px,
};
use navidog_ui::assets::lucide;
use navidog_ui::skin::truncated;
use navidog_ui::{palette, tokens};
use navidog_util::explain::{self as plan, PlanNode};
use rust_i18n::t;

use crate::explain::ExplainView;

pub(crate) struct Visual {
    pub tree: Option<PlanNode>,
    pub threshold: Entity<InputState>,
    pub search: Entity<InputState>,
    pub zoom: f32,
    /// Show only nodes of this type
    pub filter: Option<String>,
    /// Index into `plan::flatten` of the node whose info box is open
    pub selected: Option<usize>,
    /// Statistics per relation instead of per node type
    pub by_relation: bool,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = query, no_json)]
pub(crate) struct FilterNodeType {
    /// Empty clears the filter
    pub node_type: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = query, no_json)]
pub(crate) struct StatsMode {
    pub by_relation: bool,
}

impl Visual {
    pub fn new(tree: Option<PlanNode>, window: &mut Window, cx: &mut App) -> Self {
        Self {
            tree,
            threshold: cx.new(|cx| InputState::new(window, cx).default_value("50")),
            search: cx.new(|cx| InputState::new(window, cx)),
            zoom: 1.,
            filter: None,
            selected: None,
            by_relation: false,
        }
    }

    fn threshold(&self, cx: &App) -> f64 {
        self.threshold
            .read(cx)
            .value()
            .trim()
            .parse()
            .unwrap_or(50.)
    }
}

/// The parent of each flattened node (the nearest earlier node one level up).
fn parents(flat: &[(usize, &PlanNode)]) -> Vec<Option<usize>> {
    flat.iter()
        .enumerate()
        .map(|(i, (depth, _))| {
            (*depth > 0)
                .then(|| (0..i).rev().find(|j| flat[*j].0 + 1 == *depth))
                .flatten()
        })
        .collect()
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0. && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

impl ExplainView {
    pub(crate) fn render_visual(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let Some(tree) = self.visual.tree.clone() else {
            return div().into_any_element();
        };
        let flat = plan::flatten(&tree);
        let slots = plan::layout(&tree);
        let ups = parents(&flat);
        let total = plan::root_cost(&tree);
        let threshold = self.visual.threshold(cx);
        let search = self.visual.search.read(cx).value().to_lowercase();
        let rem = window.rem_size();
        let z = self.visual.zoom;
        let nw = tokens::explain::NODE_WIDTH.to_pixels(rem) * z;
        let nh = tokens::explain::NODE_HEIGHT.to_pixels(rem) * z;
        let gx = tokens::explain::NODE_GAP_X.to_pixels(rem) * z;
        let gy = tokens::explain::NODE_GAP_Y.to_pixels(rem) * z;
        let margin = gy;
        let at = |slot: (usize, f32)| {
            point(
                margin + (nw + gx) * slot.0 as f32,
                margin + (nh + gy) * slot.1,
            )
        };
        let cols = slots.iter().map(|s| s.0).max().unwrap_or(0) + 1;
        let rows = slots.iter().map(|s| s.1).fold(0., f32::max) + 1.;
        let width = margin * 2. + (nw + gx) * cols as f32 - gx;
        let height = margin * 2. + (nh + gy) * rows - gy;
        // Edges from a child's right edge to its parent's left edge: the data flows rightwards
        let edges: Vec<_> = ups
            .iter()
            .enumerate()
            .filter_map(|(i, up)| {
                let up = (*up)?;
                let (c, pnt) = (at(slots[i]), at(slots[up]));
                Some((
                    point(c.x + nw, c.y + nh / 2.),
                    point(pnt.x, pnt.y + nh / 2.),
                ))
            })
            .collect();
        let line_color = p.label_tertiary;
        let lines = canvas(
            move |_, _, _| {},
            move |bounds, _, window, _| {
                for (a, b) in &edges {
                    // raw-px: hairline edges
                    let mut path = PathBuilder::stroke(px(1.));
                    let (a, b) = (bounds.origin + *a, bounds.origin + *b);
                    let mid = (a.x + b.x) / 2.;
                    path.move_to(a);
                    path.line_to(point(mid, a.y));
                    path.line_to(point(mid, b.y));
                    path.line_to(b);
                    if let Ok(path) = path.build() {
                        window.paint_path(path, line_color);
                    }
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .w(width)
        .h(height);
        let selected = self.visual.selected;
        let filter = self.visual.filter.clone();
        let nodes: Vec<AnyElement> = flat
            .iter()
            .enumerate()
            .map(|(i, (_, n))| {
                let pos = at(slots[i]);
                let share = plan::share(n, total);
                let hot = share.is_some_and(|s| s >= threshold);
                let dim = filter.as_ref().is_some_and(|f| *f != n.op);
                let hit = !search.is_empty()
                    && (n.op.to_lowercase().contains(&search)
                        || n.relation
                            .as_ref()
                            .is_some_and(|r| r.to_lowercase().contains(&search)));
                let mut lines = vec![n.op.clone()];
                if let Some(r) = &n.relation {
                    lines.push(r.clone());
                }
                let mut stats = Vec::new();
                if let Some(s) = share {
                    stats.push(format!("{s:.1}%"));
                }
                if let Some(r) = n.rows {
                    stats.push(format!("{} {}", fmt_num(r), t!("query.rows")));
                }
                div()
                    .id(("plan-node", i))
                    .absolute()
                    .left(pos.x)
                    .top(pos.y)
                    .w(nw)
                    .h(nh)
                    .p_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if selected == Some(i) {
                        p.accent
                    } else {
                        p.separator
                    })
                    .bg(if hit { p.find_highlight } else { p.window_bg })
                    .when(hit, |d| d.text_color(gpui_kit::black()))
                    .when(dim, |d| d.opacity(0.35))
                    .overflow_hidden()
                    .cursor_pointer()
                    // raw-px: text scales with the zoom like the boxes
                    .text_size(px(11. * z))
                    .children(lines.into_iter().enumerate().map(|(k, l)| {
                        truncated(SharedString::from(format!("plan-node-{i}-{k}")), l)
                            .when(k == 0, |d| d.font_weight(gpui_kit::FontWeight::SEMIBOLD))
                    }))
                    .child(
                        div()
                            .when(hot, |d| d.text_color(p.danger))
                            .child(stats.join("  ")),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.visual.selected = Some(i);
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let info = selected.and_then(|i| flat.get(i).map(|(_, n)| (i, (*n).clone())));
        let info_box = info.map(|(i, n)| {
            let pos = at(slots[i]);
            let props = n.props.clone();
            let copy_text = props
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n");
            v_flex()
                .id("plan-node-info")
                .absolute()
                .left(pos.x + nw + gy / 4.)
                .top(pos.y)
                .w(tokens::explain::NODE_INFO_WIDTH.rems())
                .max_h(tokens::explain::NODE_INFO_HEIGHT.rems())
                .rounded_md()
                .border_1()
                .border_color(p.separator)
                .bg(p.window_bg)
                .shadow_md()
                .child(
                    h_flex()
                        .h(tokens::info::HEADER_HEIGHT.rems())
                        .px_2()
                        .gap_1()
                        .border_b_1()
                        .border_color(p.separator)
                        .child(truncated("plan-node-info-title", n.op.clone()).flex_1())
                        .child(
                            Button::new("plan-node-copy")
                                .ghost()
                                .xsmall()
                                .icon(lucide("copy"))
                                .tooltip(t!("query.copy_all").to_string())
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy_text.clone(),
                                    ))
                                }),
                        )
                        .child(
                            Button::new("plan-node-close")
                                .ghost()
                                .xsmall()
                                .icon(lucide("x"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.visual.selected = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    v_flex()
                        .id("plan-node-props")
                        .p_2()
                        .gap_1()
                        .overflow_y_scroll()
                        .text_size(tokens::font::SIZE_SMALL.rems())
                        .children(props.into_iter().enumerate().map(|(k, (key, v))| {
                            v_flex()
                                .min_w_0()
                                .child(
                                    truncated(("plan-prop-key", k), key)
                                        .text_color(p.label_secondary),
                                )
                                .child(truncated(("plan-prop-value", k), v))
                        })),
                )
        });
        let types: Vec<String> =
            flat.iter()
                .map(|(_, n)| n.op.clone())
                .fold(Vec::new(), |mut acc, op| {
                    if !acc.contains(&op) {
                        acc.push(op);
                    }
                    acc
                });
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(tokens::explain::TOOLBAR_HEIGHT.rems())
                    .px_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(p.separator)
                    .text_xs()
                    .child(t!("query.cost_threshold").to_string())
                    .child(
                        div()
                            .w(tokens::explain::THRESHOLD_WIDTH.rems())
                            .child(Input::new(&self.visual.threshold).xsmall()),
                    )
                    .child(
                        Button::new("plan-zoom-in")
                            .ghost()
                            .xsmall()
                            .icon(lucide("zoom-in"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.visual.zoom = (this.visual.zoom * 1.25).min(4.);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("plan-zoom-out")
                            .ghost()
                            .xsmall()
                            .icon(lucide("zoom-out"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.visual.zoom = (this.visual.zoom / 1.25).max(0.25);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("plan-filter")
                            .ghost()
                            .xsmall()
                            .icon(lucide("funnel"))
                            .label(
                                self.visual
                                    .filter
                                    .clone()
                                    .unwrap_or_else(|| t!("query.node_type").to_string()),
                            )
                            .on_click(move |e: &ClickEvent, window, cx| {
                                let menu = types.iter().fold(NativeMenu::new(), |m, ty| {
                                    m.menu(
                                        ty.clone(),
                                        Box::new(FilterNodeType {
                                            node_type: ty.clone().into(),
                                        }),
                                    )
                                });
                                menu.separator()
                                    .menu(
                                        t!("query.clear_filter").to_string(),
                                        Box::new(FilterNodeType {
                                            node_type: SharedString::default(),
                                        }),
                                    )
                                    .show(e.position(), window, cx);
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .w(tokens::explain::SEARCH_WIDTH.rems())
                            .child(Input::new(&self.visual.search).xsmall().cleanable(true)),
                    ),
            )
            .child(
                div()
                    .id("plan-canvas")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .on_action(cx.listener(|this, a: &FilterNodeType, _, cx| {
                        this.visual.filter =
                            (!a.node_type.is_empty()).then(|| a.node_type.to_string());
                        cx.notify();
                    }))
                    .on_scroll_wheel(cx.listener(
                        |this, e: &gpui_kit::ScrollWheelEvent, window, cx| {
                            if !e.modifiers.platform {
                                return;
                            }
                            cx.stop_propagation();
                            let dy = e.delta.pixel_delta(window.line_height()).y;
                            let factor = if dy > gpui_kit::Pixels::ZERO {
                                1.1
                            } else {
                                1. / 1.1
                            };
                            this.visual.zoom = (this.visual.zoom * factor).clamp(0.25, 4.);
                            cx.notify();
                        },
                    ))
                    .child(
                        div()
                            .relative()
                            .w(width)
                            .h(height)
                            .child(lines)
                            .children(nodes)
                            .children(info_box),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_stats(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(tree) = self.visual.tree.as_ref() else {
            return div().into_any_element();
        };
        let by_relation = self.visual.by_relation;
        let (headers, rows): (Vec<String>, Vec<Vec<String>>) = if by_relation {
            (
                vec![
                    t!("query.stat_relation").to_string(),
                    t!("query.stat_scans").to_string(),
                    t!("query.node_type").to_string(),
                    t!("query.stat_count").to_string(),
                    t!("query.stat_cost").to_string(),
                    t!("query.stat_relation_percent").to_string(),
                ],
                plan::stats_by_relation(tree)
                    .into_iter()
                    .map(|s| {
                        vec![
                            s.relation,
                            s.scans.to_string(),
                            s.node_type,
                            s.count.to_string(),
                            fmt_num(s.cost),
                            format!("{:.2}", s.percent),
                        ]
                    })
                    .collect(),
            )
        } else {
            (
                vec![
                    t!("query.node_type").to_string(),
                    t!("query.stat_count").to_string(),
                    t!("query.stat_cost").to_string(),
                    t!("query.stat_percent").to_string(),
                ],
                plan::stats_by_type(tree)
                    .into_iter()
                    .map(|s| {
                        vec![
                            s.node_type,
                            s.count.to_string(),
                            fmt_num(s.cost),
                            format!("{:.2}", s.percent),
                        ]
                    })
                    .collect(),
            )
        };
        v_flex()
            .size_full()
            .on_action(cx.listener(|this, a: &StatsMode, _, cx| {
                this.visual.by_relation = a.by_relation;
                cx.notify();
            }))
            .child(
                h_flex()
                    .h(tokens::explain::TOOLBAR_HEIGHT.rems())
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(p.separator)
                    .child(
                        Button::new("stats-by-type")
                            .ghost()
                            .xsmall()
                            .label(t!("query.stats_by_type").to_string())
                            .selected(!by_relation)
                            .on_click(|_, window, cx| {
                                window
                                    .dispatch_action(Box::new(StatsMode { by_relation: false }), cx)
                            }),
                    )
                    .child(
                        Button::new("stats-by-relation")
                            .ghost()
                            .xsmall()
                            .label(t!("query.stats_by_relation").to_string())
                            .selected(by_relation)
                            .on_click(|_, window, cx| {
                                window
                                    .dispatch_action(Box::new(StatsMode { by_relation: true }), cx)
                            }),
                    ),
            )
            .child(
                h_flex()
                    .h(tokens::list::HEADER_HEIGHT.rems())
                    .border_b_1()
                    .border_color(p.separator)
                    .children(headers.into_iter().map(|h| {
                        div()
                            .flex_1()
                            .px_2()
                            .truncate()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(h)
                    })),
            )
            .child(
                v_flex()
                    .id("stats-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows.into_iter().enumerate().map(|(ix, row)| {
                        h_flex()
                            .h(tokens::list::ROW_HEIGHT.rems())
                            .when(ix % 2 == 1, |d| d.bg(p.row_alt))
                            .children(row.into_iter().enumerate().map(move |(k, v)| {
                                truncated(SharedString::from(format!("stat-{ix}-{k}")), v)
                                    .flex_1()
                                    .px_2()
                            }))
                    })),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parents_follow_depth() {
        let leaf = PlanNode {
            op: "leaf".into(),
            ..PlanNode::default()
        };
        let root = PlanNode {
            op: "root".into(),
            children: vec![
                PlanNode {
                    op: "mid".into(),
                    children: vec![leaf.clone()],
                    ..PlanNode::default()
                },
                leaf,
            ],
            ..PlanNode::default()
        };
        let flat = plan::flatten(&root);
        assert_eq!(parents(&flat), [None, Some(0), Some(1), Some(0)]);
        assert_eq!(fmt_num(3.), "3");
        assert_eq!(fmt_num(2.345), "2.35");
    }
}
