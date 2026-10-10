//! Result grid technical check (21-table-viewer.md §1): 100,000 rows × 20 columns, one frame timed
//! per scroll step. Needs Metal: `cargo run -p navidog-grid --example spike [--release]`.

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
    /// Real type names, so the header draws its type row and numbers align right as in a table
    const TYPES: [&str; 5] = ["int", "varchar(45)", "datetime", "decimal(10,2)", "text"];

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
                    cx.new(|cx| ResultGrid::new(window, cx))
                })
            })
            .unwrap();

        let t = Instant::now();
        let columns = (0..COLS)
            .map(|c| GridColumn {
                name: format!("column_{c}").into(),
                type_name: TYPES[c % TYPES.len()].into(),
                primary_key: c == 0,
                comment: None,
                read_only: false,
            })
            .collect();
        let rows = (0..ROWS)
            .map(|r| {
                (0..COLS)
                    .map(|c| {
                        (c != 3).then(|| {
                            match TYPES[c % TYPES.len()] {
                                "int" => format!("{}", r * COLS + c),
                                "datetime" => format!("2026-10-{:02} 12:{:02}:00", r % 28 + 1, c),
                                "decimal(10,2)" => format!("{r}.{c:02}"),
                                _ => format!("value {r}-{c}"),
                            }
                            .into()
                        })
                    })
                    .collect()
            })
            .collect();
        cx.update(|cx| grid.update(cx, |g, cx| g.set_data(columns, rows, cx)));
        println!("loaded {ROWS} rows × {COLS} columns: {:?}", t.elapsed());

        let frame = |cx: &mut HeadlessAppContext| {
            let t = Instant::now();
            cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
            t.elapsed()
        };
        for _ in 0..5 {
            frame(&mut cx);
        }
        for (name, step) in [("row by row", 3usize), ("jumping", 997)] {
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
                "{name}: avg {avg:?}, p95 {p95:?}, max {:?}, frames over 8.3ms {over}/300",
                times.last().unwrap()
            );
        }
    }
}
