//! Find and replace on the loaded page (F-DATA-007).

use super::*;

impl TableViewer {
    pub(super) fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find = true;
        self.replace_mode = replace;
        self.find_input.update(cx, |s, cx| s.focus(window, cx));
        self.sync_highlight(cx);
        cx.notify();
    }

    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find = false;
        self.sync_highlight(cx);
        let handle = self.grid.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    pub(super) fn sync_highlight(&mut self, cx: &mut Context<Self>) {
        let query = self.find_input.read(cx).value().to_string();
        let on = self.show_find && self.find_highlight && !self.find_columns && !query.is_empty();
        let case = self.find_case;
        self.grid.update(cx, |g, cx| {
            g.set_find(on.then(|| (query.into(), case)), cx);
        });
    }

    pub(super) fn find_step(&mut self, after: bool, cx: &mut Context<Self>) {
        let query = self.find_input.read(cx).value().to_string();
        if query.is_empty() {
            self.sync_highlight(cx);
            return;
        }
        let case = self.find_case;
        let head = self.grid.read(cx).model(cx).head();
        if self.find_columns {
            let hit = self
                .grid
                .read(cx)
                .model(cx)
                .find_column_name(&query, head.1, after, case);
            if let Some(c) = hit {
                self.grid.update(cx, |g, cx| {
                    g.update_model(cx, |m| m.select_column(c, false))
                });
            }
        } else if let Some(pos) = self
            .grid
            .read(cx)
            .model(cx)
            .find_cell(&query, head, after, case)
        {
            self.grid.update(cx, |g, cx| g.go_to(pos, cx));
        }
        self.sync_highlight(cx);
    }

    pub(super) fn replace_current(&mut self, cx: &mut Context<Self>) {
        if self.find_columns {
            return;
        }
        let query = self.find_input.read(cx).value().to_string();
        let replacement = self.replace_input.read(cx).value().to_string();
        let case = self.find_case;
        let head = self.grid.read(cx).model(cx).head();
        let replaced = self.grid.update(cx, |g, cx| {
            let mut ok = false;
            g.update_model(cx, |m| {
                ok = m.replace_cell(head, &query, &replacement, case)
            });
            ok
        });
        self.find_step(replaced, cx);
    }

    pub(super) fn replace_every(&mut self, cx: &mut Context<Self>) {
        if self.find_columns {
            return;
        }
        let query = self.find_input.read(cx).value().to_string();
        let replacement = self.replace_input.read(cx).value().to_string();
        let case = self.find_case;
        self.grid.update(cx, |g, cx| {
            g.update_model(cx, |m| {
                m.replace_all(&query, &replacement, case);
            });
        });
        self.sync_highlight(cx);
    }

    fn find_mode_label(&self) -> String {
        if self.find_columns {
            t!("viewer.find_columns").to_string()
        } else if self.replace_mode {
            t!("viewer.replace").to_string()
        } else {
            t!("viewer.find").to_string()
        }
    }

    fn find_menu(&self) -> NativeMenu {
        NativeMenu::new()
            .menu_with_check(
                t!("viewer.find").to_string(),
                !self.replace_mode && !self.find_columns,
                Box::new(FindData),
            )
            .menu_with_check(
                t!("viewer.replace").to_string(),
                self.replace_mode && !self.find_columns,
                Box::new(FindReplaceMode),
            )
            .menu_with_check(
                t!("viewer.find_columns").to_string(),
                self.find_columns,
                Box::new(FindColumnsMode),
            )
            .separator()
            .menu_with_check(
                t!("viewer.find_case").to_string(),
                self.find_case,
                Box::new(ToggleMatchCase),
            )
            .menu_with_check(
                t!("viewer.find_highlight").to_string(),
                self.find_highlight,
                Box::new(ToggleHighlight),
            )
            .menu_with_check(
                t!("viewer.find_incremental").to_string(),
                self.find_incremental,
                Box::new(ToggleIncremental),
            )
    }

    pub(super) fn render_find(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let replace = (self.replace_mode && !self.find_columns).then(|| {
            navidog_ui::find_bar::ReplaceRow::new(
                "replace-one",
                "replace-all",
                Input::new(&self.replace_input).small().flex_1(),
                t!("viewer.replace").to_string(),
                t!("viewer.replace_all").to_string(),
            )
            .on_one(cx.listener(|this, _, _, cx| this.replace_current(cx)))
            .on_all(cx.listener(|this, _, _, cx| this.replace_every(cx)))
        });
        navidog_ui::find_bar::FindBar::new(
            self.find_mode_label(),
            Input::new(&self.find_input).small().flex_1(),
            t!("viewer.find_done").to_string(),
        )
        .on_mode(cx.listener(|this, e: &ClickEvent, window, cx| {
            this.focus_for_menu(window, cx);
            this.find_menu().show(e.position(), window, cx);
        }))
        .on_prev(cx.listener(|this, _, _, cx| this.find_step(false, cx)))
        .on_next(cx.listener(|this, _, _, cx| this.find_step(true, cx)))
        .on_done(cx.listener(|this, _, window, cx| this.close_find(window, cx)))
        .replace(replace)
    }
}
