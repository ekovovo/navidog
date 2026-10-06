//! Editor tabs of the object pane: tab commands, the tab context menu and the tab title.
//! Panels report their tab group and activation here, so a command can name any tab by its
//! panel id (a right-clicked tab need not be the active one).

use std::collections::{HashMap, HashSet};

use gpui_kit::component::dock::{PanelId, TabGroup};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::{Icon, h_flex};
use gpui_kit::{
    Action, App, Entity, Global, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, SharedString, Styled as _, WeakEntity,
};
use rust_i18n::t;

use crate::tokens;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Close {
    This,
    Others,
    Right,
    All,
}

/// Closes tabs relative to `panel`, or to the active tab when `None`.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = tabs, no_json)]
pub struct CloseTabs {
    pub panel: Option<PanelId>,
    pub which: Close,
}

/// Moves a tab into a window of its own (handled by the shell).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = tabs, no_json)]
pub struct PopOutTab {
    pub panel: PanelId,
}

/// Adds a tab to the favorites (handled by the shell).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = tabs, no_json)]
pub struct AddToFavorites {
    pub panel: PanelId,
}

/// Adds a tab to the startup screen (handled by the shell).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = tabs, no_json)]
pub struct AddToStartup {
    pub panel: PanelId,
}

gpui_kit::actions!(tabs, [CloseTab, NextTab, PrevTab]);

#[derive(Default)]
struct Tabs {
    groups: HashMap<PanelId, WeakEntity<TabGroup>>,
    active: Option<PanelId>,
    /// Panels leaving the dock for a window of their own; they keep their state
    detaching: HashSet<PanelId>,
}

impl Global for Tabs {}

pub fn init(cx: &mut App) {
    cx.set_global(Tabs::default());
    cx.on_action(|_: &CloseTab, cx| close(None, Close::This, cx));
    cx.on_action(|a: &CloseTabs, cx| close(a.panel, a.which, cx));
    cx.on_action(|_: &NextTab, cx| cycle(1, cx));
    cx.on_action(|_: &PrevTab, cx| cycle(-1, cx));
}

/// Call from `Panel::on_added_to`.
pub fn added(panel: PanelId, group: WeakEntity<TabGroup>, cx: &mut App) {
    cx.global_mut::<Tabs>().groups.insert(panel, group);
}

/// Call from `Panel::set_active`.
pub fn set_active(panel: PanelId, active: bool, cx: &mut App) {
    let tabs = cx.global_mut::<Tabs>();
    if active {
        tabs.active = Some(panel);
    } else if tabs.active == Some(panel) {
        tabs.active = None;
    }
}

/// Call from `Panel::on_removed`. Returns `true` when the panel is only moving to another window,
/// in which case it must keep its state (sessions, pending edits).
pub fn removed(panel: PanelId, cx: &mut App) -> bool {
    let tabs = cx.global_mut::<Tabs>();
    tabs.groups.remove(&panel);
    if tabs.active == Some(panel) {
        tabs.active = None;
    }
    tabs.detaching.remove(&panel)
}

/// Removes a tab from its group without ending it, for moving it into another window.
pub fn detach(panel: PanelId, cx: &mut App) {
    let Some(group) = group_of(panel, cx) else {
        return;
    };
    cx.global_mut::<Tabs>().detaching.insert(panel);
    group.update(cx, |g, cx| g.close_panel(panel, cx));
}

pub fn active(cx: &App) -> Option<PanelId> {
    cx.try_global::<Tabs>()?.active
}

pub fn group_of(panel: PanelId, cx: &App) -> Option<Entity<TabGroup>> {
    cx.try_global::<Tabs>()?.groups.get(&panel)?.upgrade()
}

/// Panels to close for `which`, in tab order; the group itself refuses non-closable ones.
fn targets(ids: &[PanelId], panel: PanelId, which: Close) -> Vec<PanelId> {
    let Some(pos) = ids.iter().position(|p| *p == panel) else {
        return Vec::new();
    };
    match which {
        Close::This => vec![panel],
        Close::Others => ids.iter().copied().filter(|p| *p != panel).collect(),
        Close::Right => ids[pos + 1..].to_vec(),
        Close::All => ids.to_vec(),
    }
}

fn close(panel: Option<PanelId>, which: Close, cx: &mut App) {
    let Some(panel) = panel.or_else(|| active(cx)) else {
        return;
    };
    let Some(group) = group_of(panel, cx) else {
        return;
    };
    let ids: Vec<PanelId> = group
        .read(cx)
        .panels()
        .iter()
        .map(|p| p.panel_id(cx))
        .collect();
    let targets = targets(&ids, panel, which);
    group.update(cx, |g, cx| {
        for id in targets {
            g.close_panel(id, cx);
        }
    });
}

fn cycle(step: isize, cx: &mut App) {
    let Some(group) = active(cx).and_then(|p| group_of(p, cx)) else {
        return;
    };
    let (ix, n) = {
        let g = group.read(cx);
        (g.active_ix() as isize, g.panels().len() as isize)
    };
    let Some(window) = cx.active_window() else {
        return;
    };
    let next = (ix + step).rem_euclid(n.max(1)) as usize;
    let _ = window.update(cx, |_, window, cx| {
        group.update(cx, |g, cx| g.select_tab(next, window, cx))
    });
}

pub fn context_menu(panel: PanelId) -> NativeMenu {
    let close = |which| {
        Box::new(CloseTabs {
            panel: Some(panel),
            which,
        })
    };
    NativeMenu::new()
        .menu(t!("tabs.close").to_string(), close(Close::This))
        .menu(t!("tabs.close_others").to_string(), close(Close::Others))
        .menu(t!("tabs.close_right").to_string(), close(Close::Right))
        .menu(t!("tabs.close_all").to_string(), close(Close::All))
        .separator()
        .menu(
            t!("tabs.pop_out").to_string(),
            Box::new(PopOutTab { panel }),
        )
        .separator()
        .menu(
            t!("tabs.add_favorite").to_string(),
            Box::new(AddToFavorites { panel }),
        )
        .menu(
            t!("tabs.add_startup").to_string(),
            Box::new(AddToStartup { panel }),
        )
}

/// A tab's title: optional object icon, then the label truncated with the full text on hover;
/// right-click opens the tab context menu. Panels return it from `Panel::title` (leaving
/// `tab_name` as `None`).
pub fn title(
    panel: PanelId,
    icon: Option<Icon>,
    label: impl Into<SharedString>,
) -> impl IntoElement {
    h_flex()
        .id(SharedString::from(format!("tab-title-{panel:?}")))
        .gap_1()
        .min_w_0()
        .max_w(tokens::editor_tab::TITLE_MAX_WIDTH.rems())
        .children(icon.map(|i| i.size(tokens::navigator::ROW_ICON.rems())))
        .child(crate::skin::truncated(
            SharedString::from(format!("tab-label-{panel:?}")),
            label,
        ))
        .on_mouse_down(MouseButton::Right, move |e, window, cx| {
            cx.stop_propagation();
            context_menu(panel).show(e.position, window, cx);
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_targets_follow_tab_order() {
        let ids: Vec<PanelId> = (1..=4).map(PanelId::from_u64).collect();
        let p = |n| PanelId::from_u64(n);
        assert_eq!(targets(&ids, p(2), Close::This), [p(2)]);
        assert_eq!(targets(&ids, p(2), Close::Others), [p(1), p(3), p(4)]);
        assert_eq!(targets(&ids, p(2), Close::Right), [p(3), p(4)]);
        assert_eq!(targets(&ids, p(4), Close::Right), []);
        assert_eq!(targets(&ids, p(9), Close::All), []);
    }
}
