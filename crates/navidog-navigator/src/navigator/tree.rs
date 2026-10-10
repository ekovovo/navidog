//! Builds the tree items from the connection store, sessions, catalog and virtual groups.

use super::*;

/// Builds the tree items (read-only access to the globals).
pub(super) struct TreeBuilder<'a> {
    pub(super) panel: &'a NavigatorPanel,
    pub(super) store: &'a ConnectionStore,
    pub(super) sessions: &'a Sessions,
    pub(super) catalog: &'a Catalog,
    pub(super) vgroups: &'a VirtualGroups,
    pub(super) query: &'a str,
    pub(super) filtering: bool,
    pub(super) show_children: bool,
    pub(super) show_objects: bool,
}

impl TreeBuilder<'_> {
    pub(super) fn matches(&self, label: &str) -> bool {
        !self.filtering || label.to_lowercase().contains(self.query)
    }

    pub(super) fn object_item(&self, scope: &Scope, kind: ObjectKind, name: &str) -> TreeItem {
        let obj = ObjectRef {
            scope: scope.clone(),
            kind,
            name: name.to_string(),
        };
        if kind == ObjectKind::Table && self.show_children {
            let children = self.table_children(&obj);
            self.item(Node::Object(obj), name.to_string(), children)
        } else {
            TreeItem::new(Node::Object(obj).id(), name.to_string())
        }
    }

    /// Fields, indexes, foreign keys and triggers folders under a table.
    pub(super) fn table_children(&self, table: &ObjectRef) -> Vec<TreeItem> {
        Child::ALL
            .into_iter()
            .map(|child| {
                let folder = Node::TableFolder(table.clone(), child);
                let loaded = match child {
                    Child::Fields => match self.catalog.columns(&table.scope, &table.name) {
                        Some(Load::Ready(cols)) => {
                            Ok(cols.iter().map(|c| c.name.clone()).collect())
                        }
                        Some(Load::Failed(e)) => Err(e.to_string()),
                        _ => Err(t!("nav.loading").to_string()),
                    },
                    _ => match self.catalog.table_children(&table.scope, &table.name) {
                        Some(Load::Ready(c)) => Ok(match child {
                            Child::Indexes => c.indexes.clone(),
                            Child::ForeignKeys => c.foreign_keys.clone(),
                            _ => c.triggers.clone(),
                        }),
                        Some(Load::Failed(e)) => Err(e.to_string()),
                        _ => Err(t!("nav.loading").to_string()),
                    },
                };
                let items = match loaded {
                    Ok(names) => names
                        .into_iter()
                        .map(|n: String| {
                            TreeItem::new(Node::TableItem(table.clone(), child, n.clone()).id(), n)
                        })
                        .collect(),
                    Err(message) => Self::placeholder(&folder, message),
                };
                self.item(
                    folder,
                    t!(format!("nav.child_{}", child.key())).to_string(),
                    items,
                )
            })
            .collect()
    }

    /// Whether the search or a quick filter is hiding nodes (empty groups are hidden then).
    pub(super) fn narrowing(&self) -> bool {
        let p = self.panel;
        self.filtering || p.view.only_active || p.only_starred || p.type_filter.is_some()
    }

    pub(super) fn shows_connection(&self, c: &Connection) -> bool {
        let p = self.panel;
        !c.hidden
            && (!p.only_starred || c.starred)
            && (!p.view.only_active || self.sessions.state(&c.id).is_open())
            && p.type_filter
                .as_ref()
                .is_none_or(|t| t.as_ref() == c.driver)
    }

    pub(super) fn expanded(&self, node: &Node) -> bool {
        self.filtering || self.panel.expanded.contains(&node.id())
    }

    pub(super) fn item(
        &self,
        node: Node,
        label: impl Into<SharedString>,
        children: Vec<TreeItem>,
    ) -> TreeItem {
        let expanded = self.expanded(&node);
        TreeItem::new(node.id(), label)
            .children(children)
            .expanded(expanded)
    }

    pub(super) fn placeholder(parent: &Node, label: String) -> Vec<TreeItem> {
        vec![TreeItem::new(
            Node::Placeholder(parent.id().to_string()).id(),
            label,
        )]
    }

    pub(super) fn groups(&self, parent: Option<&str>) -> Vec<TreeItem> {
        let mut items = Vec::new();
        for g in self
            .store
            .groups()
            .iter()
            .filter(|g| g.parent.as_deref() == parent)
        {
            let children = self.groups(Some(&g.id));
            if children.is_empty()
                && (!self.matches(&g.name) || (self.narrowing() && !self.filtering))
            {
                continue;
            }
            items.push(self.item(Node::Group(g.id.clone()), g.name.clone(), children));
        }
        items.extend(
            self.store
                .connections()
                .iter()
                .filter(|c| c.group.as_deref() == parent)
                .filter_map(|c| self.connection(c)),
        );
        items
    }

    pub(super) fn connection(&self, c: &Connection) -> Option<TreeItem> {
        if !self.shows_connection(c) {
            return None;
        }
        let dbs: Vec<TreeItem> = match self.sessions.state(&c.id) {
            ConnState::Open { databases, .. } => {
                let dialect = self.sessions.dialect(&c.id);
                databases
                    .iter()
                    .filter_map(|d| self.database(&c.id, d, dialect, &c.name))
                    .collect()
            }
            _ => Vec::new(),
        };
        if !self.matches(&c.name) && dbs.is_empty() {
            return None;
        }
        Some(self.item(Node::Connection(c.id.clone()), c.name.clone(), dbs))
    }

    pub(super) fn database(
        &self,
        conn: &str,
        db: &str,
        dialect: Option<Dialect>,
        conn_name: &str,
    ) -> Option<TreeItem> {
        let node = Node::Database(conn.to_string(), db.to_string());
        let Some(dialect) = dialect else {
            return Some(self.item(node, db.to_string(), Vec::new()));
        };
        let view = &self.panel.view;
        if (!view.show_hidden && dialect.is_system_database(db))
            || (view.only_active && !self.panel.expanded.contains(&node.id()))
        {
            return None;
        }
        let children = if dialect.has_schemas() {
            match self.catalog.schemas(conn, db) {
                Some(Load::Ready(list)) => list
                    .iter()
                    .filter(|s| view.show_hidden || !dialect.is_system_schema(s))
                    .map(|s| {
                        let scope = Scope::new(conn, db, Some(s.clone()));
                        self.item(
                            Node::Schema(scope.clone()),
                            s.clone(),
                            self.categories(&scope, dialect),
                        )
                    })
                    .collect(),
                Some(Load::Failed(e)) => Self::placeholder(&node, e.to_string()),
                _ => Self::placeholder(&node, t!("nav.loading").to_string()),
            }
        } else {
            self.categories(&Scope::new(conn, db, None), dialect)
        };
        let any_match = self.matches(db)
            || self.matches(conn_name)
            || children.iter().any(|c| !c.children.is_empty());
        any_match.then(|| self.item(node, db.to_string(), children))
    }

    pub(super) fn categories(&self, scope: &Scope, dialect: Dialect) -> Vec<TreeItem> {
        if !self.show_objects {
            return Vec::new();
        }
        dialect
            .object_kinds()
            .iter()
            .filter_map(|&kind| {
                let node = Node::Category(scope.clone(), kind);
                let children = match self.catalog.objects(scope, kind) {
                    Some(Load::Ready(list)) => {
                        let names: Vec<&str> = list
                            .iter()
                            .map(|o| o.name.as_str())
                            .filter(|n| self.matches(n))
                            .collect();
                        let mut items = Vec::new();
                        let mut grouped = HashSet::new();
                        if !self.panel.view.hide_object_groups {
                            for g in self.vgroups.groups_for(scope, kind) {
                                let members: Vec<TreeItem> = g
                                    .members
                                    .iter()
                                    .filter(|m| names.contains(&m.as_str()))
                                    .map(|m| {
                                        grouped.insert(m.as_str());
                                        self.object_item(scope, kind, m)
                                    })
                                    .collect();
                                if self.filtering && members.is_empty() && !self.matches(&g.name) {
                                    continue;
                                }
                                let vg = Node::VGroup(scope.clone(), kind, g.id.clone());
                                items.push(self.item(vg, g.name.clone(), members));
                            }
                        }
                        items.extend(
                            names
                                .iter()
                                .filter(|n| !grouped.contains(**n))
                                .map(|n| self.object_item(scope, kind, n)),
                        );
                        items
                    }
                    Some(Load::Failed(e)) => Self::placeholder(&node, e.to_string()),
                    _ if self.filtering => return None,
                    _ => Self::placeholder(&node, t!("nav.loading").to_string()),
                };
                if self.filtering && children.is_empty() {
                    return None;
                }
                Some(self.item(node, kind_label(kind), children))
            })
            .collect()
    }
}
