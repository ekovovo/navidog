//! Shell-level commands (10-shell.md §9).

use gpui_kit::Action;

gpui_kit::actions!(
    navidog,
    [
        About,
        Preferences,
        CheckForUpdates,
        Hide,
        HideOthers,
        ShowAll,
        Quit,
        CloseWindow,
        ToggleNavigator,
        ToggleInfoPane,
        ToggleHideGroups,
        ToggleFilterOnTop,
        ToggleStatusBar,
        ToggleToolbar,
        ToggleFullScreen,
        Minimize,
        Zoom,
        ResetLayout,
        ShowMainWindow,
        BringAllToFront,
        OpenHelp,
        ReportIssue,
        // Favorites (F-APP-011) and recent objects (F-APP-013)
        AddActiveToFavorites,
        ClearFavorites,
        ClearRecent,
        // Open an object from a navidog URI (F-APP-014)
        OpenUri,
    ]
);

/// ⌥⇧1–9: adds the active tab to the favorites under that ID.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = navidog, no_json)]
pub struct AddFavoriteWithId {
    pub id: u8,
}

/// ⌃⇧1–9: opens the favorite with that ID.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = navidog, no_json)]
pub struct OpenFavoriteId {
    pub id: u8,
}

/// Favorites menu: opens a favorite by its position.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = navidog, no_json)]
pub struct OpenFavorite {
    pub ix: usize,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = navidog, no_json)]
pub struct RemoveFavorite {
    pub ix: usize,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = navidog, no_json)]
pub struct OpenRecent {
    pub ix: usize,
}
