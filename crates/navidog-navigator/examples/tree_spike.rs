//! Navigator tree spike (20-objects.md §1): 5000 objects under one category, rows carrying the
//! multi-select, drag-and-drop and inline-rename hooks the navigator needs; measures scroll frames.
//! Needs Metal: `cargo run -p navidog-navigator --example tree_spike [--release]`.

fn main() {
    #[cfg(target_os = "macos")]
    macos::run();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui_kit::component::input::{Input, InputState};
    use gpui_kit::component::list::ListItem;
    use gpui_kit::component::tree::{TreeEntry, TreeItem, TreeState, tree};
    use gpui_kit::component::{Sizable as _, h_flex};
    use gpui_kit::prelude::FluentBuilder as _;
    use gpui_kit::{
        AppContext as _, ClickEvent, Context, ElementId, Entity, HeadlessAppContext,
        InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollStrategy,
        SharedString, StatefulInteractiveElement as _, Styled as _, Window, WindowBounds,
        WindowOptions, div, size,
    };
    use navidog_ui::assets::lucide;
    use navidog_ui::{palette, tokens};

    const OBJECTS: usize = 5000;
    const BUDGET: Duration = Duration::from_micros(8_300);

    #[derive(Clone)]
    struct Dragged(SharedString);

    impl Render for Dragged {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().child(self.0.clone())
        }
    }

    struct Spike {
        tree: Entity<TreeState>,
        selected: HashSet<SharedString>,
        rename: Entity<InputState>,
    }

    impl Spike {
        fn row(&self, entry: &TreeEntry, cx: &mut Context<Self>) -> ListItem {
            let p = palette(cx);
            let id = entry.item().id.clone();
            let label: gpui_kit::AnyElement = if id.as_ref() == "o:2" {
                Input::new(&self.rename).xsmall().into_any_element()
            } else {
                div().child(entry.item().label.clone()).into_any_element()
            };
            let (click_id, drop_id) = (id.clone(), id.clone());
            ListItem::new(ElementId::Name(id.clone()))
                .h(tokens::navigator::ROW_HEIGHT.rems())
                .pl(tokens::navigator::ROW_INSET_X.rems()
                    + tokens::navigator::ROW_INDENT.rems() * entry.depth() as f32)
                .when(self.selected.contains(&id), |l| {
                    l.bg(p.selection).text_color(p.selection_text)
                })
                .child(
                    h_flex()
                        .gap_1()
                        .child(lucide("table").size(tokens::navigator::ROW_ICON.rems()))
                        .child(label),
                )
                .on_drag(Dragged(id), |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<Dragged>(move |s, _, _, _| s.bg(p.fill))
                .on_drop(
                    cx.listener(move |_, d: &Dragged, _, _| println!("拖放：{} → {drop_id}", d.0)),
                )
                .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                    if !e.modifiers().platform {
                        this.selected.clear();
                    }
                    if !this.selected.remove(&click_id) {
                        this.selected.insert(click_id.clone());
                    }
                    cx.notify();
                }))
        }
    }

    impl Render for Spike {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let view = cx.entity().downgrade();
            div()
                .size_full()
                .child(tree(&self.tree, move |ix, entry, _, _, cx| {
                    view.upgrade()
                        .map(|v| v.update(cx, |this, cx| this.row(entry, cx)))
                        .unwrap_or_else(|| ListItem::new(ix))
                }))
        }
    }

    pub fn run() {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(navidog_ui::NavidogAssets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            navidog_ui::theme::init(cx);
        });
        let (handle, view) = cx
            .update(|cx| {
                // raw-px: spike window size (navigator pane height of a maximized window)
                let window_size = size(gpui_kit::px(280.), gpui_kit::px(900.));
                let bounds = gpui_kit::Bounds {
                    origin: Default::default(),
                    size: window_size,
                };
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    show: false,
                    focus: false,
                    ..Default::default()
                };
                gpui_kit::open_window(options, cx, |window, cx| {
                    let objects = (0..OBJECTS)
                        .map(|i| TreeItem::new(format!("o:{i}"), format!("table_{i:05}")));
                    let root = TreeItem::new("k:tables", "表")
                        .expanded(true)
                        .children(objects);
                    let db = TreeItem::new("d:main", "main").expanded(true).child(root);
                    let state = cx.new(|cx| TreeState::new(cx).items(vec![db]));
                    let rename =
                        cx.new(|cx| InputState::new(window, cx).default_value("table_00000"));
                    cx.new(|_| Spike {
                        tree: state,
                        selected: HashSet::new(),
                        rename,
                    })
                })
            })
            .unwrap();

        let frame = |cx: &mut HeadlessAppContext| {
            let t = Instant::now();
            cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
            t.elapsed()
        };
        for _ in 0..5 {
            frame(&mut cx);
        }
        let rows = OBJECTS + 2;
        for (name, step) in [("逐行滚动", 3usize), ("跳跃滚动", 397)] {
            let mut times: Vec<Duration> = (0..300)
                .map(|i| {
                    cx.update(|cx| {
                        let tree = view.read(cx).tree.clone();
                        tree.update(cx, |t, _| {
                            t.scroll_to_item((i * step) % rows, ScrollStrategy::Top)
                        });
                    });
                    frame(&mut cx)
                })
                .collect();
            times.sort();
            let avg = times.iter().sum::<Duration>() / times.len() as u32;
            let p95 = times[times.len() * 95 / 100];
            let over = times.iter().filter(|t| **t > BUDGET).count();
            println!(
                "{name}：平均 {avg:?}，p95 {p95:?}，最大 {:?}，超过 8.3ms 的帧 {over}/300",
                times.last().unwrap()
            );
        }
    }
}
