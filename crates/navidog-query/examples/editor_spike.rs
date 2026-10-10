//! SQL editor technical check (30-query-editor.md §1): 50,000 lines of SQL, one frame timed per
//! scroll step or keystroke, with and without range decorations.
//! Needs Metal: `cargo run -p navidog-query --example editor_spike [--release]`.

fn main() {
    #[cfg(target_os = "macos")]
    macos::run();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui_kit::component::input::{
        Editor, EditorState, RangeDecoration, RangeDecorationStyle, TabSize,
    };
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::{
        AppContext as _, Context, Entity, HeadlessAppContext, IntoElement, ParentElement as _,
        Pixels, Point, Render, Styled as _, Window, WindowBounds, WindowOptions, div, point, size,
    };

    /// `NAVIDOG_SPIKE_LINES` overrides it, to compare against a document small enough for the
    /// synchronous parse
    static LINES: std::sync::LazyLock<usize> = std::sync::LazyLock::new(|| {
        std::env::var("NAVIDOG_SPIKE_LINES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50_000)
    });
    const BUDGET: Duration = Duration::from_micros(8_300);

    struct Spike {
        editor: Entity<EditorState>,
    }

    impl Render for Spike {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(Editor::new(&self.editor).h_full())
        }
    }

    fn script() -> String {
        (0..*LINES)
            .map(|i| match i % 5 {
                0 => format!("SELECT a.id, a.name_{i}, COUNT(*) AS n FROM t_{i} a"),
                1 => format!("  JOIN other b ON (a.id = b.a_id AND b.flag IN (1, 2, {i}))"),
                2 => format!("  WHERE a.note = 'value {i}' -- trailing comment {i}"),
                3 => "  GROUP BY a.id, a.name ORDER BY n DESC;".to_string(),
                _ => format!("/* block {i} */ UPDATE t SET x = x + {i} WHERE id = {i};"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn report(name: &str, mut times: Vec<Duration>) {
        times.sort();
        let avg = times.iter().sum::<Duration>() / times.len() as u32;
        let p95 = times[times.len() * 95 / 100];
        let over = times.iter().filter(|t| **t > BUDGET).count();
        println!(
            "{name}：平均 {avg:?}，p95 {p95:?}，最大 {:?}，超过 8.3ms 的帧 {over}/{}",
            times.last().unwrap(),
            times.len()
        );
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
        let text = script();
        println!("{} 行，{:.1} MB", *LINES, text.len() as f64 / 1e6);
        let (handle, view) = cx
            .update(|cx| {
                // raw-px: window size of the check
                let window_size = size(gpui_kit::px(1440.), gpui_kit::px(900.));
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
                    let editor = cx.new(|cx| {
                        EditorState::new(window, cx)
                            .language("sql")
                            .line_number(true)
                            .soft_wrap(false)
                            .auto_close(false)
                            .tab_size(TabSize {
                                tab_size: 4,
                                hard_tabs: false,
                            })
                    });
                    editor.update(cx, |e, cx| e.set_value(text.clone(), window, cx));
                    cx.new(|_| Spike { editor })
                })
            })
            .unwrap();
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));

        let frame = |cx: &mut HeadlessAppContext| {
            let t = Instant::now();
            cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
            t.elapsed()
        };
        // A parse that misses the 2 ms synchronous budget moves to the background behind a 150 ms
        // debounce. The headless context runs on a test dispatcher whose timers only fire when
        // the clock is advanced, so advance it, or the document stays unhighlighted.
        let settle = |cx: &mut HeadlessAppContext| {
            for _ in 0..3 {
                frame(cx);
                cx.advance_clock(Duration::from_millis(200));
                cx.run_until_parked();
            }
        };
        let t = Instant::now();
        settle(&mut cx);
        println!("首次解析并上色：{:?}", t.elapsed());
        let editor = cx.update(|cx| view.read(cx).editor.clone());
        let line_height = cx.update(|cx| editor.read(cx).line_height()).unwrap();

        let scroll = |cx: &mut HeadlessAppContext, name: &str, step: usize| {
            let times = (0..300)
                .map(|i| {
                    cx.update(|cx| {
                        editor.update(cx, |e, cx| {
                            let y = line_height
                                * ((i * step) % (*LINES).saturating_sub(60).max(1)) as f32;
                            e.set_scroll_offset(point(Pixels::ZERO, -y), cx)
                        })
                    });
                    frame(cx)
                })
                .collect();
            report(name, times);
        };
        scroll(&mut cx, "逐行滚动（每帧 3 行）", 3);
        scroll(&mut cx, "跳跃滚动（每帧 997 行）", 997);

        // Find-all highlight: one fill per 25 lines, plus a bracket frame
        cx.update(|cx| {
            editor.update(cx, |e, cx| {
                let text = e.value();
                let fills = text
                    .match_indices("a.id")
                    .step_by(10)
                    .map(|(at, m)| {
                        RangeDecoration::new(at..at + m.len())
                            .with_style(RangeDecorationStyle::Fill)
                    })
                    .collect::<Vec<_>>();
                println!("装饰：{} 个填充", fills.len());
                e.create_range_decorations_collection(fills, cx);
                let paren = text.find('(').unwrap();
                e.create_range_decorations_collection(
                    vec![RangeDecoration::new(paren..paren + 1)],
                    cx,
                );
            })
        });
        scroll(&mut cx, "带装饰逐行滚动", 3);

        // A look at highlighting and decorations at the top of the document
        let shot = |cx: &mut HeadlessAppContext, name: &str| {
            settle(cx);
            let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../target/editor-spike-{}-{name}.png", *LINES));
            cx.capture_screenshot(handle)
                .expect("needs the Metal renderer")
                .save(&out)
                .unwrap();
            println!("截图：{}", out.display());
        };
        cx.update(|cx| editor.update(cx, |e, cx| e.set_scroll_offset(Point::default(), cx)));
        shot(&mut cx, "top");

        // Typing in the middle of the document
        cx.update_window(handle, |_, window, cx| {
            editor.update(cx, |e, cx| {
                let mid = e.value().len() / 2;
                e.set_selected_range(mid..mid, cx);
                e.insert("", window, cx);
            })
        })
        .unwrap();
        let times = (0..300)
            .map(|_| {
                cx.update_window(handle, |_, window, cx| {
                    editor.update(cx, |e, cx| e.insert("x", window, cx))
                })
                .unwrap();
                frame(&mut cx)
            })
            .collect();
        report("文档中部连续输入", times);
        cx.update(|cx| editor.update(cx, |e, cx| e.set_scroll_offset(Point::default(), cx)));
        shot(&mut cx, "typed");
    }
}
