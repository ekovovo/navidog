//! Picks the editor that opens an object in the object pane (the shell only composes feature panels).

use std::sync::Arc;

use gpui_kit::component::dock::{BasePanelView, panel_handle};
use gpui_kit::{App, Window};
use navidog_designer::ObjectDesigner;
use navidog_grid::TableViewer;
use navidog_model::object::{ObjectKind, ObjectRef};
use navidog_query::QueryEditor;

pub fn open(object: ObjectRef, window: &mut Window, cx: &mut App) -> Arc<dyn BasePanelView> {
    match object.kind {
        ObjectKind::Table => panel_handle(TableViewer::new(object, window, cx)),
        ObjectKind::Query => panel_handle(QueryEditor::open_saved(object, window, cx)),
        _ => panel_handle(ObjectDesigner::open(object, window, cx)),
    }
}
