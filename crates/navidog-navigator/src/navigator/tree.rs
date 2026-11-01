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
    pub(super) show_system: bool,
}

/// Groups, connections, databases, schemas, categories and virtual groups.
/// Objects and the rows under a table are matches, not their ancestors.
fn search_opens(node: &Node) -> bool {
    matches!(
        node,
        Node::Group(_)
            | Node::Connection(_)
            | Node::Database(..)
            | Node::Schema(_)
            | Node::Category(..)
            | Node::VGroup(..)
    )
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
                        Some(Load::Failed(e)) => Err(Some(e.to_string())),
                        _ => Err(None),
                    },
                    _ => match self.catalog.table_children(&table.scope, &table.name) {
                        Some(Load::Ready(c)) => Ok(match child {
                            Child::Indexes => c.indexes.clone(),
                            Child::ForeignKeys => c.foreign_keys.clone(),
                            _ => c.triggers.clone(),
                        }),
                        Some(Load::Failed(e)) => Err(Some(e.to_string())),
                        _ => Err(None),
                    },
                };
                let items = match loaded {
                    Ok(names) => names
                        .into_iter()
                        .map(|n: String| {
                            TreeItem::new(Node::TableItem(table.clone(), child, n.clone()).id(), n)
                        })
                        .collect(),
                    Err(error) => Self::placeholder(&folder, error),
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

    /// Search opens the containers above a match. A table and the folders under
    /// it stay as the user left them: forcing those open shows skeleton rows
    /// whose catalog fetch never starts.
    pub(super) fn expanded(&self, node: &Node) -> bool {
        self.panel.expanded.contains(&node.id()) || (self.filtering && search_opens(node))
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

    /// `None` is still loading: three bars, so one row does not look like a glitch.
    /// `Some` is a failure, including an empty driver message, and stays a single row.
    pub(super) fn placeholder(parent: &Node, error: Option<String>) -> Vec<TreeItem> {
        let Some(message) = error else {
            return (0..3)
                .map(|i| {
                    TreeItem::new(
                        Node::Placeholder(format!("{}#{i}", parent.id())).id(),
                        String::new(),
                    )
                })
                .collect();
        };
        let message = if message.is_empty() {
            t!("obj.failed").to_string()
        } else {
            message
        };
        vec![TreeItem::new(
            Node::Placeholder(parent.id().to_string()).id(),
            message,
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
                    .filter_map(|d| self.database(&c.id, d, dialect.clone(), &c.name))
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
        dialect: Option<Arc<dyn Dialect>>,
        conn_name: &str,
    ) -> Option<TreeItem> {
        let node = Node::Database(conn.to_string(), db.to_string());
        let Some(dialect) = dialect else {
            return Some(self.item(node, db.to_string(), Vec::new()));
        };
        let view = &self.panel.view;
        if (!self.show_system && dialect.is_system_database(db))
            || (view.only_active && !self.panel.expanded.contains(&node.id()))
        {
            return None;
        }
        let children = if dialect.features().has_schemas {
            match self.catalog.schemas(conn, db) {
                Some(Load::Ready(list)) => list
                    .iter()
                    .filter(|s| self.show_system || !dialect.is_system_schema(s))
                    .map(|s| {
                        let scope = Scope::new(conn, db, Some(s.clone()));
                        self.item(
                            Node::Schema(scope.clone()),
                            s.clone(),
                            self.categories(&scope, dialect.as_ref()),
                        )
                    })
                    .collect(),
                Some(Load::Failed(e)) => Self::placeholder(&node, Some(e.to_string())),
                _ => Self::placeholder(&node, None),
            }
        } else {
            self.categories(&Scope::new(conn, db, None), dialect.as_ref())
        };
        let any_match = self.matches(db)
            || self.matches(conn_name)
            || children.iter().any(|c| !c.children.is_empty());
        any_match.then(|| self.item(node, db.to_string(), children))
    }

    pub(super) fn categories(&self, scope: &Scope, dialect: &dyn Dialect) -> Vec<TreeItem> {
        if !self.show_objects {
            return Vec::new();
        }
        // Every database (PostgreSQL: every schema) also lists its saved queries.
        // MariaDB sequences are not on the shared MySQL kind list; `lists_kind` adds them.
        let driver = self
            .store
            .get(&scope.connection)
            .map(|c| c.driver.as_str())
            .unwrap_or("");
        let version = match self.sessions.state(&scope.connection) {
            ConnState::Open { version, .. } => version.as_str(),
            _ => "",
        };
        let mut kinds: Vec<ObjectKind> = dialect.object_kinds().to_vec();
        if !kinds.contains(&ObjectKind::Sequence) {
            kinds.push(ObjectKind::Sequence);
        }
        kinds.push(ObjectKind::Query);
        kinds.retain(|kind| {
            *kind == ObjectKind::Query || dialect.lists_kind(*kind, driver, version)
        });
        kinds
            .into_iter()
            .filter_map(|kind| {
                let node = Node::Category(scope.clone(), kind);
                let children = match self.catalog.objects(scope, kind) {
                    Some(Load::Ready(list)) => {
                        let names: Vec<&str> = list
                            .iter()
                            .map(|o| o.name.as_str())
                            .filter(|n| self.show_system || !dialect.is_system_object(n))
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
                                let only_hidden = members.is_empty()
                                    && g.members.iter().any(|m| {
                                        list.iter().any(|o| {
                                            o.name == *m && dialect.is_system_object(&o.name)
                                        })
                                    });
                                if only_hidden && !(self.filtering && self.matches(&g.name)) {
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
                    Some(Load::Failed(e)) => Self::placeholder(&node, Some(e.to_string())),
                    _ if self.filtering => return None,
                    _ => Self::placeholder(&node, None),
                };
                if self.filtering && children.is_empty() {
                    return None;
                }
                Some(self.item(node, kind_label(kind), children))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loading_folder_shows_three_bars_and_an_error_stays_one_row() {
        let parent = Node::Connection("c".into());
        let loading = TreeBuilder::placeholder(&parent, None);
        assert_eq!(
            loading
                .iter()
                .map(|item| item.id.to_string())
                .collect::<Vec<_>>(),
            ["p:c:c#0", "p:c:c#1", "p:c:c#2"]
        );
        assert!(loading.iter().all(|item| item.label.is_empty()));
        let error = TreeBuilder::placeholder(&parent, Some("down".into()));
        assert_eq!(error.len(), 1);
        assert_eq!(error[0].id.as_ref(), "p:c:c");
        assert_eq!(error[0].label.as_ref(), "down");
        let blank = TreeBuilder::placeholder(&parent, Some(String::new()));
        assert_eq!(blank.len(), 1);
        assert!(!blank[0].label.is_empty());
    }
}
