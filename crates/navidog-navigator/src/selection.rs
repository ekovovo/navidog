//! Click selection in the navigator, with Finder semantics: a plain click selects one node,
//! ⌘-click toggles a node, ⇧-click selects the visible range from the anchor.

use gpui_kit::SharedString;

pub(crate) struct Click<'a> {
    /// Currently selected node ids, the primary one first
    pub selected: &'a [SharedString],
    pub anchor: Option<&'a SharedString>,
    /// Ids of the visible rows, top to bottom
    pub visible: &'a [SharedString],
    pub cmd: bool,
    pub shift: bool,
}

/// Returns the new selection (primary first) and anchor after clicking `clicked`.
pub(crate) fn click(c: Click, clicked: &SharedString) -> (Vec<SharedString>, Option<SharedString>) {
    let position = |id: &SharedString| c.visible.iter().position(|v| v == id);
    if c.shift
        && let (Some(a), Some(b)) = (c.anchor.and_then(position), position(clicked))
    {
        let range = c.visible[a.min(b)..=a.max(b)]
            .iter()
            .filter(|id| *id != clicked)
            .cloned();
        return (
            std::iter::once(clicked.clone()).chain(range).collect(),
            c.anchor.cloned(),
        );
    }
    if c.cmd {
        let mut selected: Vec<_> = c
            .selected
            .iter()
            .filter(|id| *id != clicked)
            .cloned()
            .collect();
        if selected.len() == c.selected.len() {
            selected.insert(0, clicked.clone());
        }
        return (selected, Some(clicked.clone()));
    }
    (vec![clicked.clone()], Some(clicked.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<SharedString> {
        v.iter()
            .map(|s| SharedString::from(s.to_string()))
            .collect()
    }

    #[test]
    fn plain_cmd_and_shift_clicks() {
        let visible = ids(&["a", "b", "c", "d"]);
        let run = |selected: &[SharedString], anchor: Option<&str>, cmd, shift, clicked: &str| {
            let anchor = anchor.map(|a| SharedString::from(a.to_string()));
            let c = Click {
                selected,
                anchor: anchor.as_ref(),
                visible: &visible,
                cmd,
                shift,
            };
            click(c, &clicked.to_string().into())
        };
        assert_eq!(
            run(&ids(&["a"]), Some("a"), false, false, "c"),
            (ids(&["c"]), Some("c".into()))
        );
        // ⌘ adds the node as the new primary, ⌘ again removes it
        let (sel, anchor) = run(&ids(&["a"]), Some("a"), true, false, "c");
        assert_eq!((sel.clone(), anchor), (ids(&["c", "a"]), Some("c".into())));
        assert_eq!(run(&sel, Some("c"), true, false, "c").0, ids(&["a"]));
        // ⇧ selects the range between the anchor and the clicked row; the anchor stays
        assert_eq!(
            run(&ids(&["b"]), Some("b"), false, true, "d"),
            (ids(&["d", "b", "c"]), Some("b".into()))
        );
        assert_eq!(
            run(&ids(&["c"]), Some("c"), false, true, "a").0,
            ids(&["a", "b", "c"])
        );
        // Without a visible anchor ⇧ acts as a plain click
        assert_eq!(
            run(&ids(&["x"]), Some("x"), false, true, "b").0,
            ids(&["b"])
        );
    }
}
