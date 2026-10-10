//! History log viewer (31-query-results.md §6, F-QRY-012): every statement sent this session,
//! live. New records follow at the end unless the user scrolled up; "Pause" holds them back.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::{
    BasePanel, Panel, PanelEvent, PanelId, PanelInfo, PanelState, TabGroup, panel_handle,
    register_panel,
};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::{Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, Styled as _, Task,
    WeakEntity, Window, actions, div, point, px,
};
use navidog_core::settings::{HistoryHeader, Settings};
use navidog_db::log::{self, Header, LogEntry};
use navidog_ui::assets::lucide;
use navidog_ui::{palette, tabs, tokens};
use rust_i18n::t;

pub(crate) const PANEL: &str = "HistoryLog";

actions!(
    query,
    [
        ToggleHistoryDate,
        ToggleHistoryTime,
        ToggleHistoryServer,
        ToggleHistorySession,
        ToggleHistoryKind,
        ToggleHistoryElapsed,
    ]
);

pub(crate) fn init(cx: &mut App) {
    register_panel(cx, PANEL, |_, window, cx| {
        panel_handle(HistoryLog::new(window, cx))
    });
}

fn header(h: HistoryHeader) -> Header {
    Header {
        date: h.date,
        time: h.time,
        server: h.server,
        session: h.session,
        kind: h.kind,
        elapsed: h.elapsed,
    }
}

/// The viewer text for some entries.
pub(crate) fn render_entries(entries: &[LogEntry], h: Header, only_errors: bool) -> String {
    entries
        .iter()
        .filter(|e| !only_errors || e.error.is_some())
        .map(|e| e.render(h))
        .collect()
}

pub struct HistoryLog {
    focus_handle: FocusHandle,
    editor: Entity<EditorState>,
    /// Every entry shown so far (and held back while paused); the log keeps 10,000 at most
    entries: Vec<LogEntry>,
    next: u64,
    paused: bool,
    /// Entries that arrived while paused
    held: usize,
    only_errors: bool,
    /// Scroll position after the last jump to the end, to tell whether the user scrolled away
    bottom: Pixels,
    jumped: bool,
    _poll: Task<()>,
}

impl EventEmitter<PanelEvent> for HistoryLog {}

impl Focusable for HistoryLog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for HistoryLog {
    fn panel_name(&self) -> &'static str {
        PANEL
    }
    fn dump(&self, _: &App) -> PanelState {
        PanelState {
            panel_name: PANEL.into(),
            children: Vec::new(),
            info: PanelInfo::panel(serde_json::json!({ "history": true })),
        }
    }
    fn on_added_to(&mut self, group: WeakEntity<TabGroup>, _: &mut Window, cx: &mut Context<Self>) {
        tabs::added(PanelId::from(cx.entity_id()), group, cx);
    }
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        tabs::set_active(PanelId::from(cx.entity_id()), active, cx);
    }
    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let _ = tabs::removed(PanelId::from(cx.entity_id()), cx);
    }
}

impl Panel for HistoryLog {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        tabs::title(
            PanelId::from(cx.entity_id()),
            Some(lucide("file-clock")),
            t!("query.history_log").to_string(),
            None,
            false,
        )
    }
}

impl HistoryLog {
    pub fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .line_number(false)
                .soft_wrap(false)
        });
        let (entries, next) = log::since(0);
        let text = render_entries(&entries, header(Settings::global(cx).history_header), false);
        editor.update(cx, |e, cx| {
            e.set_readonly(true, cx);
            e.set_value(text, window, cx);
        });
        cx.new(|cx: &mut Context<Self>| {
            // ponytail: polls the log four times a second; a channel from the log would avoid it
            let poll = cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(250))
                        .await;
                    if this
                        .update_in(cx, |this, window, cx| this.pull(window, cx))
                        .is_err()
                    {
                        return;
                    }
                }
            });
            Self {
                focus_handle: cx.focus_handle(),
                editor,
                entries,
                next,
                paused: false,
                held: 0,
                only_errors: false,
                bottom: Pixels::ZERO,
                // The first layout scrolls to the end
                jumped: false,
                _poll: poll,
            }
        })
    }

    #[cfg(test)]
    pub(crate) fn text(&self, cx: &App) -> String {
        self.editor.read(cx).value().to_string()
    }

    fn header(cx: &App) -> Header {
        header(Settings::global(cx).history_header)
    }

    /// Takes new entries from the log and appends them, following the end if the view is there.
    pub(crate) fn pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Start at the end once the first layout gives a line height
        if !self.jumped
            && self.bottom == Pixels::ZERO
            && self.editor.read(cx).line_height().is_some()
        {
            self.editor.update(cx, scroll_to_end);
            self.jumped = true;
        }
        if self.next == log::revision() && self.held == 0 {
            self.track_scroll(cx);
            return;
        }
        let (new, next) = log::since(self.next);
        self.next = next;
        if new.is_empty() && self.held == 0 {
            return;
        }
        if self.paused {
            self.held += new.len();
            self.entries.extend(new);
            cx.notify();
            return;
        }
        let following = self.track_scroll(cx);
        let shown = if self.held > 0 {
            self.held = 0;
            self.entries.extend(new);
            render_entries(&self.entries, Self::header(cx), self.only_errors)
        } else {
            let text = render_entries(&new, Self::header(cx), self.only_errors);
            self.entries.extend(new);
            if text.is_empty() {
                return;
            }
            format!("{}{text}", self.editor.read(cx).value())
        };
        self.show(shown, following, window, cx);
    }

    /// Whether the view is at the end (nothing scrolled it since the last jump there).
    fn track_scroll(&mut self, cx: &App) -> bool {
        let y = self.editor.read(cx).scroll_offset().y;
        if self.jumped {
            self.bottom = y;
            self.jumped = false;
        }
        // raw-px: a line's slack before counting as scrolled away, before the first layout
        let line = self.editor.read(cx).line_height().unwrap_or(px(16.));
        y <= self.bottom + line
    }

    fn show(&mut self, text: String, follow: bool, window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.editor.read(cx).scroll_offset();
        self.editor.update(cx, |e, cx| {
            e.set_value(text, window, cx);
            if follow {
                scroll_to_end(e, cx);
            } else {
                e.set_scroll_offset(offset, cx);
            }
        });
        if follow {
            self.jumped = true;
        }
        cx.notify();
    }

    /// Header options or "only errors" changed: everything is rendered again.
    fn rerender(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = render_entries(&self.entries, Self::header(cx), self.only_errors);
        self.show(text, true, window, cx);
    }

    fn toggle_header(
        &mut self,
        f: impl FnOnce(&mut HistoryHeader),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        Settings::update(cx, |s| f(&mut s.history_header));
        self.rerender(window, cx);
    }
}

impl Render for HistoryLog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        v_flex()
            .id("history-log")
            .key_context("HistoryLog")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(p.control_bg)
            .on_action(cx.listener(|this, _: &ToggleHistoryDate, window, cx| {
                this.toggle_header(|h| h.date = !h.date, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleHistoryTime, window, cx| {
                this.toggle_header(|h| h.time = !h.time, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleHistoryServer, window, cx| {
                this.toggle_header(|h| h.server = !h.server, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleHistorySession, window, cx| {
                this.toggle_header(|h| h.session = !h.session, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleHistoryKind, window, cx| {
                this.toggle_header(|h| h.kind = !h.kind, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleHistoryElapsed, window, cx| {
                this.toggle_header(|h| h.elapsed = !h.elapsed, window, cx)
            }))
            .child(
                h_flex()
                    .h(tokens::query::TOOLBAR_HEIGHT.rems())
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(p.separator)
                    .child(
                        Button::new("history-errors")
                            .ghost()
                            .small()
                            .icon(lucide("circle-x"))
                            .label(t!("query.only_errors").to_string())
                            .selected(self.only_errors)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.only_errors = !this.only_errors;
                                this.rerender(window, cx);
                            })),
                    )
                    .child(
                        Button::new("history-pause")
                            .ghost()
                            .small()
                            .icon(lucide(if self.paused { "play" } else { "pause" }))
                            .label(if self.paused {
                                t!("query.resume").to_string()
                            } else {
                                t!("query.pause").to_string()
                            })
                            .selected(self.paused)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.paused = !this.paused;
                                if !this.paused {
                                    this.pull(window, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.label_secondary)
                            .child(t!("query.history_count", n = self.entries.len()).to_string()),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Editor::new(&self.editor).bordered(false).size_full()),
            )
    }
}

/// Scrolls a read-only log view so its last line shows (the editor does not clamp an offset
/// past the end, so it is computed from the line count).
pub(crate) fn scroll_to_end(e: &mut EditorState, cx: &mut Context<EditorState>) {
    use gpui_kit::component::RopeExt as _;
    let Some(line) = e.line_height() else { return };
    let view = e.input_bounds().size.height;
    let content = line * (e.text().lines_len() as f32 + 1.);
    let y = (content - view).max(Pixels::ZERO);
    e.set_scroll_offset(point(Pixels::ZERO, -y), cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_errors_keeps_failed_statements() {
        let at = chrono::TimeZone::with_ymd_and_hms(&chrono::Local, 2026, 1, 2, 3, 4, 5).unwrap();
        let entry = |sql: &str, error: Option<&str>| LogEntry {
            time: at,
            connection: "c".into(),
            session: 1,
            kind: "SQLITE".into(),
            elapsed: Duration::ZERO,
            sql: sql.into(),
            error: error.map(String::from),
        };
        let entries = [
            entry("SELECT 1", None),
            entry("SELECT x", Some("no such column: x")),
        ];
        let all = render_entries(&entries, Header::ALL, false);
        assert!(all.contains("SELECT 1") && all.contains("SELECT x"));
        let errors = render_entries(&entries, Header::ALL, true);
        assert!(!errors.contains("SELECT 1"));
        assert!(errors.contains("SELECT x\n\n") && errors.contains("no such column: x"));
        let bare = render_entries(
            &entries[..1],
            Header {
                date: false,
                time: false,
                server: false,
                session: false,
                kind: false,
                elapsed: false,
            },
            false,
        );
        assert_eq!(bare, "\nSELECT 1\n\n");
    }
}
