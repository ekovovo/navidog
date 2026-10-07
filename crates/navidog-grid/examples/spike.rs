//! 结果网格技术验证（21-table-viewer.md 第 1 节）：10 万行 × 20 列，滚动时测单帧耗时。
//! 需要 Metal：`cargo run -p navidog-grid --example spike [--release]`。

fn main() {
    #[cfg(target_os = "macos")]
    macos::run();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui_kit::{AppContext as _, HeadlessAppContext, WindowBounds, WindowOptions, size};
    use navidog_grid::ResultGrid;
    use navidog_grid::model::GridColumn;

    const ROWS: usize = 100_000;
    const COLS: usize = 20;
    const BUDGET: Duration = Duration::from_micros(8_300);

    pub fn run() {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(navidog_ui::NavidogAssets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            navidog_ui::theme::init(cx);
            navidog_grid::init(cx);
        });
        let (handle, grid) = cx
            .update(|cx| {
                // raw-px: 验证用窗口尺寸
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
                    cx.new(|cx| ResultGrid::new(window, cx))
                })
            })
            .unwrap();

        let t = Instant::now();
        let columns = (0..COLS)
            .map(|c| GridColumn {
                name: format!("column_{c}").into(),
                type_name: "".into(),
                primary_key: c == 0,
                comment: None,
                read_only: false,
            })
            .collect();
        let rows = (0..ROWS)
            .map(|r| {
                (0..COLS)
                    .map(|c| (c != 3).then(|| format!("value {r}-{c}").into()))
                    .collect()
            })
            .collect();
        cx.update(|cx| grid.update(cx, |g, cx| g.set_data(columns, rows, cx)));
        println!("载入 {ROWS} 行 × {COLS} 列：{:?}", t.elapsed());

        let frame = |cx: &mut HeadlessAppContext| {
            let t = Instant::now();
            cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
            t.elapsed()
        };
        for _ in 0..5 {
            frame(&mut cx);
        }
        for (name, step) in [("逐行滚动", 3usize), ("跳跃滚动", 997)] {
            let mut times: Vec<Duration> = (0..300)
                .map(|i| {
                    cx.update(|cx| grid.update(cx, |g, cx| g.scroll_to_row((i * step) % ROWS, cx)));
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
