//! New groups and in-place renaming of groups, virtual groups and objects.

use super::*;

impl NavigatorPanel {
    pub(crate) fn new_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = match Workspace::global(cx).read(cx).selection() {
            Selection::Group(g) => Some(g.clone()),
            _ => None,
        };
        let name = t!("nav.new_group_default").to_string();
        let result =
            ConnectionStore::global(cx).update(cx, |s, cx| s.add_group(name, parent.clone(), cx));
        let id = match result {
            Ok(id) => id,
            Err(error) => {
                window.push_notification(error.localized_message(), cx);
                return;
            }
        };
        if let Some(p) = &parent {
            self.expanded.insert(Node::Group(p.clone()).id());
        }
        Workspace::global(cx).update(cx, |w, cx| w.select(Selection::Group(id.clone()), cx));
        self.start_rename(id, window, cx);
    }

    pub(crate) fn start_rename(
        &mut self,
        group: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let store = ConnectionStore::global(cx);
        let Some(name) = store
            .read(cx)
            .groups()
            .iter()
            .find(|g| g.id == group)
            .map(|g| g.name.clone())
        else {
            return;
        };
        self.start_rename_target(RenameTarget::Group(group), name, window, cx);
    }

    pub(super) fn start_rename_vgroup(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = VirtualGroups::global(cx)
            .read(cx)
            .get(&id)
            .map(|g| g.name.clone());
        if let Some(name) = name {
            self.start_rename_target(RenameTarget::VGroup(id), name, window, cx);
        }
    }

    /// Creates a virtual group in the selected category (moving the selected objects into it)
    /// and edits its name in place.
    pub(crate) fn new_virtual_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let objects = crate::ops::selected_objects(cx);
        let (scope, kind) = match Workspace::global(cx).read(cx).selection() {
            Selection::Category(s, k) => (s.clone(), *k),
            Selection::Object(o) => (o.scope.clone(), o.kind),
            _ => return,
        };
        self.expanded
            .insert(Node::Category(scope.clone(), kind).id());
        let name = t!("nav.new_group_default").to_string();
        let id = VirtualGroups::global(cx).update(cx, |g, cx| {
            let id = g.add_group(scope, kind, name, cx);
            g.move_objects(&objects, Some(&id), cx);
            id
        });
        self.start_rename_vgroup(id, window, cx);
    }

    /// Edits an object's name in place; Enter runs the dialect's `RENAME` statement.
    pub(crate) fn start_rename_object(
        &mut self,
        obj: ObjectRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let renamable = obj.kind == ObjectKind::Query
            || Sessions::global(cx)
                .read(cx)
                .dialect(&obj.scope.connection)
                .and_then(|d| d.rename_object(&obj.scope, obj.kind, &obj.name, &obj.name))
                .is_some();
        if renamable {
            let name = obj.name.clone();
            self.start_rename_target(RenameTarget::Object(obj), name, window, cx);
        }
    }

    pub(super) fn start_rename_target(
        &mut self,
        target: RenameTarget,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        let subscription =
            cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.finish_rename(true, window, cx);
                }
            });
        self.renaming = Some(Rename {
            target,
            input,
            _subscription: subscription,
        });
        self.refresh_rows(cx);
    }

    /// Rows are rendered by the tree's own view, which only re-renders when it is notified.
    pub(super) fn refresh_rows(&self, cx: &mut Context<Self>) {
        self.tree.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub(super) fn finish_rename(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(r) = self.renaming.take() else {
            return;
        };
        let name = r.input.read(cx).value().trim().to_string();
        window.focus(&self.focus_handle, cx);
        self.refresh_rows(cx);
        if !commit || name.is_empty() {
            return;
        }
        match r.target.clone() {
            RenameTarget::Group(g) => {
                let result =
                    ConnectionStore::global(cx).update(cx, |s, cx| s.rename_group(&g, name, cx));
                if let Err(error) = result {
                    window.push_notification(error.localized_message(), cx);
                    r.input.update(cx, |input, cx| input.focus(window, cx));
                    self.renaming = Some(r);
                    self.refresh_rows(cx);
                }
            }
            RenameTarget::VGroup(g) => {
                VirtualGroups::global(cx).update(cx, |s, cx| s.rename_group(&g, name, cx))
            }
            RenameTarget::Object(o) if o.kind == ObjectKind::Query && o.name != name => {
                crate::ops::rename_query(o, name, window, cx);
            }
            RenameTarget::Object(o) if o.name != name => {
                let sql = Sessions::global(cx)
                    .read(cx)
                    .dialect(&o.scope.connection)
                    .and_then(|d| d.rename_object(&o.scope, o.kind, &o.name, &name));
                if let Some(sql) = sql {
                    let renamed = Selection::Object(ObjectRef { name, ..o.clone() });
                    crate::ops::run(vec![(o, sql)], Some(renamed), window, cx);
                }
            }
            RenameTarget::Object(_) => {}
        }
    }
}
