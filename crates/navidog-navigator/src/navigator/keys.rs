//! Selection in the tree: clicks, arrow keys, folding, following the workspace, and scrolling at the edge during a drag.

use super::*;

impl NavigatorPanel {
    pub(super) fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let selection = Workspace::global(cx).read(cx).selection().clone();
        let target = Node::from_selection(&selection).map(|n| n.id());
        self.tree.update(cx, |t, cx| {
            let current = t.selected_item().map(|i| i.id.clone());
            if current == target {
                return;
            }
            // A field or folder row under a table selects that table. Keep the focused row
            // there, or arrow keys and clicks could never move below the table row.
            let stands_for = current
                .as_ref()
                .and_then(|id| Node::parse(id))
                .and_then(|n| n.selection());
            if selection != Selection::None && stands_for == Some(selection) {
                return;
            }
            let ix = target.as_ref().and_then(|id| t.index_of(id));
            t.set_selected_index(ix, cx);
        });
    }

    /// When the selection comes from another panel (objects tab, toolbar), expands and loads the
    /// ancestors so the selection is visible in the tree.
    pub(super) fn reveal_selection(&mut self, cx: &mut Context<Self>) {
        let Some(node) = Node::from_selection(Workspace::global(cx).read(cx).selection()) else {
            self.sync_selection(cx);
            return;
        };
        let dialect = Workspace::global(cx)
            .read(cx)
            .selection()
            .connection_id()
            .and_then(|c| Sessions::global(cx).read(cx).dialect(c));
        let mut changed = false;
        for a in node.ancestors(dialect) {
            if self.expanded.insert(a.id()) {
                changed = true;
                Self::ensure_children(&a, false, cx);
            }
        }
        if changed {
            self.rebuild(cx);
        } else {
            self.sync_selection(cx);
        }
    }

    /// A navigator selection that is not the open editor tab's table goes back to the
    /// objects tab, so the info pane can follow the tree. A field under that table still
    /// counts as the table.
    pub(super) fn show_objects_tab_unless_same(
        &self,
        clicked: &Selection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = Workspace::global(cx).read(cx).selection().clone();
        if &current == clicked {
            return;
        }
        let panel = ObjectsPanel::shared(window, cx);
        let id = PanelId::from(panel.entity_id());
        let Some(group) = tabs::group_of(id, cx) else {
            return;
        };
        let ix = group
            .read(cx)
            .panels()
            .iter()
            .position(|p| p.panel_id(cx) == id);
        if let Some(ix) = ix {
            group.read(cx).context(cx).select_tab(ix, window, cx);
            // `select_tab` focuses the objects pane; arrow keys and Return stay in the tree
            window.focus(&self.focus_handle, cx);
        }
    }

    pub(super) fn select(&mut self, node: &Node, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(sel) = node.selection() {
            self.show_objects_tab_unless_same(&sel, window, cx);
            Workspace::global(cx).update(cx, |w, cx| w.select(sel, cx));
            let id = node.id();
            self.tree.update(cx, |t, cx| {
                let ix = t.index_of(&id);
                t.set_selected_index(ix, cx);
            });
        }
    }

    pub(super) fn selected_ids(cx: &App) -> Vec<SharedString> {
        Workspace::global(cx)
            .read(cx)
            .selections()
            .filter_map(Node::from_selection)
            .map(|n| n.id())
            .collect()
    }

    pub(super) fn click_select(
        &mut self,
        id: &SharedString,
        cmd: bool,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = Self::selected_ids(cx);
        let tree = self.tree.read(cx);
        let visible: Vec<SharedString> = (0..)
            .map_while(|ix| tree.entry(ix).map(|e| e.item().id.clone()))
            .collect();
        let click = Click {
            selected: &selected,
            anchor: self.anchor.as_ref(),
            visible: &visible,
            cmd,
            shift,
        };
        let (ids, anchor) = selection::click(click, id);
        self.anchor = anchor;
        // Rows under one table all stand for it; a range across them names it once
        let mut selections: Vec<Selection> = Vec::new();
        for sel in ids.iter().filter_map(|id| Node::parse(id)?.selection()) {
            if !selections.contains(&sel) {
                selections.push(sel);
            }
        }
        let mut selections = selections.into_iter();
        let primary = selections.next().unwrap_or_default();
        let also = selections.collect();
        self.show_objects_tab_unless_same(&primary, window, cx);
        Workspace::global(cx).update(cx, |w, cx| w.select_many(primary, also, cx));
        // The clicked row is the focused one, even when it stands for its table
        self.tree.update(cx, |t, cx| {
            let ix = t.index_of(id);
            t.set_selected_index(ix, cx);
        });
    }

    /// Moves the focused row. `extend` keeps the anchor and selects every visible row between them.
    pub(super) fn step_selection(
        &mut self,
        down: bool,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (len, ix) = {
            let tree = self.tree.read(cx);
            let len = (0..).position(|i| tree.entry(i).is_none()).unwrap_or(0);
            (len, tree.selected_index())
        };
        if len == 0 {
            return;
        }
        let next = match ix {
            None => Some(0),
            Some(ix) if down => (ix + 1 < len).then_some(ix + 1),
            Some(ix) => ix.checked_sub(1),
        };
        let Some(next) = next else { return };
        let id = self
            .tree
            .read(cx)
            .entry(next)
            .expect("next is in range")
            .item()
            .id
            .clone();
        if extend && self.anchor.is_none() {
            self.anchor = self.tree.read(cx).selected_item().map(|i| i.id.clone());
        }
        self.click_select(&id, false, extend, window, cx);
        let strategy = if down {
            gpui_kit::ScrollStrategy::Bottom
        } else {
            gpui_kit::ScrollStrategy::Top
        };
        self.tree.update(cx, |t, _| {
            if let Some(i) = t.selected_index() {
                t.scroll_to_item(i, strategy);
            }
        });
    }

    /// While a drag is inside the tree and within one row of its top or bottom, step one row that way.
    /// Holding still keeps stepping: each step asks for another frame. `scroll_edge` is the decision.
    /// Steps the offset itself: `top_item` / `bottom_item` stay 0 for a `uniform_list`, which does
    /// not record child bounds.
    pub(super) fn autoscroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !cx.has_active_drag() {
            return;
        }
        let handle = self
            .tree
            .read(cx)
            .scroll_handle()
            .0
            .borrow()
            .base_handle
            .clone();
        let (bounds, offset, max) = (handle.bounds(), handle.offset(), handle.max_offset());
        let margin = tokens::navigator::ROW_HEIGHT.to_pixels(window.rem_size());
        let pos = window.mouse_position();
        let Some(down) = scroll_edge(
            pos.x.as_f32(),
            pos.y.as_f32(),
            bounds.left().as_f32(),
            bounds.right().as_f32(),
            bounds.top().as_f32(),
            bounds.bottom().as_f32(),
            margin.as_f32(),
        ) else {
            return;
        };
        let Some(y) = step_offset(offset.y.as_f32(), max.y.as_f32(), margin.as_f32(), down) else {
            return;
        };
        let now = std::time::Instant::now();
        if self.autoscroll_at.is_some_and(|at| {
            now.saturating_duration_since(at) < std::time::Duration::from_millis(80)
        }) {
            window.request_animation_frame();
            return;
        }
        self.autoscroll_at = Some(now);
        // raw-px: a scroll offset computed at runtime
        handle.set_offset(gpui_kit::point(offset.x, gpui_kit::px(y)));
        self.tree.update(cx, |_, cx| cx.notify());
        window.request_animation_frame();
    }

    /// Right expands the focused folder (a closed connection opens). Left collapses it.
    pub(super) fn fold(&mut self, expand: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (id, folder) = {
            let tree = self.tree.read(cx);
            let Some(entry) = tree.selected_entry() else {
                return;
            };
            (entry.item().id.clone(), entry.is_folder())
        };
        let Some(node) = Node::parse(&id) else { return };
        // A closed connection has no children yet, so it is not a folder; → opens it
        if expand
            && let Node::Connection(c) = &node
            && !Sessions::global(cx).read(cx).state(c).is_open()
        {
            window.dispatch_action(Box::new(OpenConnection), cx);
            return;
        }
        if !folder {
            return;
        }
        if expand {
            if self.expanded.insert(id) {
                Self::ensure_children(&node, false, cx);
                self.rebuild(cx);
            }
        } else if self.expanded.remove(&id) {
            self.rebuild(cx);
        }
    }
}
