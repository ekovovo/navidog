//! Date, time, and timestamp cell editor (F-DATA-003). The window size matches
//! `NAVDateTimePicker.nib` (317 × 247).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, WeakEntity, Window, div,
};
use navidog_ui::skin::confirm_with;
use navidog_ui::tokens;
use rust_i18n::t;

use crate::grid::ResultGrid;
use crate::model::Pos;

pub(crate) use navidog_util::sql_type::{Temporal, temporal_kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Parts {
    y: i32,
    m: u32,
    d: u32,
    h: u32,
    min: u32,
    s: u32,
}

fn format_parts(kind: Temporal, p: Parts) -> String {
    let date = format!("{:04}-{:02}-{:02}", p.y, p.m, p.d);
    let time = format!("{:02}:{:02}:{:02}", p.h, p.min, p.s);
    match kind {
        Temporal::Date => date,
        Temporal::Time => time,
        Temporal::DateTime => format!("{date} {time}"),
    }
}

fn parse_parts(text: &str, kind: Temporal) -> Parts {
    let mut p = Parts {
        y: 1970,
        m: 1,
        d: 1,
        h: 0,
        min: 0,
        s: 0,
    };
    let (ty, tm, td) = utc_today();
    if kind != Temporal::Time {
        p.y = ty;
        p.m = tm;
        p.d = td;
    }
    let text = text.trim();
    if text.is_empty() {
        return p;
    }
    let (date, time) = text.split_once([' ', 'T']).unwrap_or(match kind {
        Temporal::Time => ("", text),
        Temporal::Date => (text, ""),
        Temporal::DateTime => (text, ""),
    });
    if let Some((y, m, d)) = split3(date, '-') {
        p.y = y;
        p.m = m.clamp(1, 12);
        p.d = d.clamp(1, days_in_month(p.y, p.m));
    }
    if let Some((h, min, s)) = split3(time, ':') {
        p.h = (h as u32).min(23);
        p.min = min.min(59);
        p.s = s.min(59);
    }
    p
}

fn split3(text: &str, sep: char) -> Option<(i32, u32, u32)> {
    let mut it = text.split(sep);
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.split('.').next()?.parse().ok()?;
    Some((a, b, c))
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 30,
    }
}

fn is_leap(y: i32) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

/// Sunday = 0. Sakamoto's method.
fn weekday_sunday(y: i32, m: u32, d: u32) -> u32 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    (y + y / 4 - y / 100 + y / 400 + t[(m as usize) - 1] + d as i32).rem_euclid(7) as u32
}

/// 42 cells, Monday first. `None` is a leading or trailing blank.
fn month_cells(y: i32, m: u32) -> [Option<u32>; 42] {
    // ponytail: weeks start on Monday; a locale-aware first weekday can replace the offset
    let lead = (weekday_sunday(y, m, 1) + 6) % 7;
    let n = days_in_month(y, m);
    let mut cells = [None; 42];
    for day in 1..=n {
        cells[(lead + day - 1) as usize] = Some(day);
    }
    cells
}

fn utc_today() -> (i32, u32, u32) {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    civil_from_days(days)
}

/// Howard Hinnant's `civil_from_days`. Day 0 is 1970-01-01.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

pub(crate) fn open(
    pos: Pos,
    kind: Temporal,
    current: &str,
    grid: WeakEntity<ResultGrid>,
    window: &mut Window,
    cx: &mut App,
) {
    let parts = parse_parts(current, kind);
    let input = cx.new(|cx| InputState::new(window, cx).default_value(format_parts(kind, parts)));
    let view = cx.new(|_| DateDialog {
        kind,
        parts,
        input: input.clone(),
        pos,
        grid,
    });
    window.open_dialog(cx, move |dialog, window, _| {
        confirm_with(dialog, &view, DateDialog::apply)
            .w(tokens::grid::DATE_PICKER_WIDTH.to_pixels(window.rem_size()))
            .close_button(false)
            .child(view.clone())
    });
    // `open_dialog` focuses the dialog itself, so a focus call before that does not stick
    window.defer(cx, move |window, cx| {
        input.update(cx, |s, cx| s.focus(window, cx));
    });
}

struct DateDialog {
    kind: Temporal,
    parts: Parts,
    input: Entity<InputState>,
    pos: Pos,
    grid: WeakEntity<ResultGrid>,
}

impl DateDialog {
    fn write_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let text = format_parts(self.kind, self.parts);
        self.input.update(cx, |s, cx| s.set_value(text, window, cx));
    }

    fn shift_month(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let mut m = self.parts.m as i32 + delta;
        let mut y = self.parts.y;
        while m < 1 {
            m += 12;
            y -= 1;
        }
        while m > 12 {
            m -= 12;
            y += 1;
        }
        self.parts.y = y;
        self.parts.m = m as u32;
        self.parts.d = self.parts.d.min(days_in_month(y, self.parts.m));
        self.write_input(window, cx);
    }

    fn pick_day(&mut self, day: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.parts.d = day;
        self.write_input(window, cx);
        if self.kind == Temporal::Date {
            self.apply(window, cx);
        }
    }

    fn today(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (y, m, d) = utc_today();
        self.parts.y = y;
        self.parts.m = m;
        self.parts.d = d;
        self.write_input(window, cx);
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        let value = (!text.is_empty()).then(|| SharedString::from(text));
        let pos = self.pos;
        let _ = self.grid.update(cx, |g, cx| {
            g.update_model(cx, |m| m.set_value(pos.0, pos.1, value));
        });
        window.close_dialog(cx);
    }
}

impl Render for DateDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let cells = month_cells(self.parts.y, self.parts.m);
        let weeks: Vec<AnyElement> = t!("grid.weekdays")
            .split_whitespace()
            .map(|label| div().flex_1().child(label.to_string()).into_any_element())
            .collect();
        let mut rows = Vec::new();
        for week in 0..6 {
            let mut row = Vec::new();
            for col in 0..7 {
                let ix = week * 7 + col;
                let el = if let Some(day) = cells[ix] {
                    let this = this.clone();
                    Button::new(("date-day", ix))
                        .ghost()
                        .label(day.to_string())
                        .on_click(move |_, window, cx| {
                            let _ = this.update(cx, |d, cx| d.pick_day(day, window, cx));
                        })
                        .into_any_element()
                } else {
                    div().flex_1().into_any_element()
                };
                row.push(el);
            }
            rows.push(h_flex().children(row));
        }
        let show_calendar = self.kind != Temporal::Time;
        let this_prev = this.clone();
        let this_next = this.clone();
        let this_today = this.clone();
        let this_ok = this.clone();
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(Input::new(&self.input))
            .when(show_calendar, |v| {
                v.child(
                    h_flex()
                        .justify_between()
                        .child(Button::new("date-prev").ghost().label("<").on_click(
                            move |_, window, cx| {
                                let _ = this_prev.update(cx, |d, cx| d.shift_month(-1, window, cx));
                            },
                        ))
                        .child(format!("{:04}-{:02}", self.parts.y, self.parts.m))
                        .child(Button::new("date-next").ghost().label(">").on_click(
                            move |_, window, cx| {
                                let _ = this_next.update(cx, |d, cx| d.shift_month(1, window, cx));
                            },
                        )),
                )
                .child(h_flex().children(weeks))
                .children(rows)
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap(tokens::form::ROW_GAP.rems())
                    .when(self.kind != Temporal::Time, |h| {
                        h.child(
                            Button::new("date-today")
                                .label(t!("grid.date_today").to_string())
                                .on_click(move |_, window, cx| {
                                    let _ = this_today.update(cx, |d, cx| d.today(window, cx));
                                }),
                        )
                    })
                    .child(
                        Button::new("date-ok")
                            .primary()
                            .label(t!("grid.ok").to_string())
                            .on_click(move |_, window, cx| {
                                let _ = this_ok.update(cx, |d, cx| d.apply(window, cx));
                            }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_october_2026() {
        let p = parse_parts("2026-10-07 16:05:09.12", Temporal::DateTime);
        assert_eq!(
            p,
            Parts {
                y: 2026,
                m: 10,
                d: 7,
                h: 16,
                min: 5,
                s: 9
            }
        );
        assert_eq!(format_parts(Temporal::DateTime, p), "2026-10-07 16:05:09");
        // 2026-10-01 is a Thursday, so Monday-first has three blanks
        let cells = month_cells(2026, 10);
        assert_eq!(cells[..4], [None, None, None, Some(1)]);
        assert_eq!(cells[33], Some(31));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }
}
