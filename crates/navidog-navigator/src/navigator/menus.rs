//! Context menus of the tree's rows and of its blank area.

use super::*;

impl NavigatorPanel {
    pub(crate) fn context_menu(node: Option<&Node>, cx: &App) -> NativeMenu {
        let new_conn =
            navidog_session::DriverRegistry::all(cx)
                .into_iter()
                .fold(NativeMenu::new(), |m, d| {
                    let info = d.info();
                    m.menu(
                        gpui_kit::SharedString::from(info.name.clone()),
                        Box::new(NewConnectionOf {
                            driver: info.id.clone().into(),
                        }),
                    )
                });
        let refresh = |m: NativeMenu| {
            m.separator()
                .menu(t!("nav.refresh").to_string(), Box::new(Refresh))
        };
        let colors = navidog_ui::tokens::CONNECTION_COLORS.iter().fold(
            NativeMenu::new()
                .menu(
                    t!("nav.color_none").to_string(),
                    Box::new(SetColor { color: None }),
                )
                .separator(),
            |m, c| {
                m.menu(
                    t!(format!("nav.color_{c}")).to_string(),
                    Box::new(SetColor {
                        color: Some((*c).into()),
                    }),
                )
            },
        );
        match node {
            None => refresh(
                NativeMenu::new()
                    .submenu(t!("nav.new_connection").to_string(), new_conn)
                    .menu(t!("nav.new_group").to_string(), Box::new(NewGroup)),
            ),
            Some(Node::Group(_)) => refresh(
                NativeMenu::new()
                    .submenu(t!("nav.new_connection").to_string(), new_conn)
                    .menu(t!("nav.new_group").to_string(), Box::new(NewGroup))
                    .menu(t!("nav.rename_group").to_string(), Box::new(RenameGroup))
                    .menu(t!("nav.delete_group").to_string(), Box::new(DeleteGroup)),
            ),
            Some(Node::Connection(id)) => {
                let open = Sessions::global(cx).read(cx).state(id).is_open();
                let groups = ConnectionStore::global(cx).read(cx).groups().to_vec();
                let move_menu = groups.iter().fold(
                    NativeMenu::new()
                        .menu(
                            t!("nav.top_level").to_string(),
                            Box::new(MoveToGroup { group: None }),
                        )
                        .separator(),
                    |m, g| {
                        m.menu(
                            g.name.clone(),
                            Box::new(MoveToGroup {
                                group: Some(g.id.clone().into()),
                            }),
                        )
                    },
                );
                let menu = if open {
                    NativeMenu::new()
                        .menu(
                            t!("nav.close_connection").to_string(),
                            Box::new(CloseConnection),
                        )
                        .menu(
                            t!("nav.reopen_connection").to_string(),
                            Box::new(ReopenConnectionId::new(id.clone())),
                        )
                } else {
                    NativeMenu::new().menu(
                        t!("nav.open_connection").to_string(),
                        Box::new(OpenConnection),
                    )
                };
                let menu = menu.separator().menu(
                    t!("nav.edit_connection").to_string(),
                    Box::new(EditConnection),
                );
                let mysql = Sessions::global(cx)
                    .read(cx)
                    .dialect(id)
                    .is_some_and(|d| d.features().switches_database);
                let menu = if open && mysql {
                    menu.menu(t!("nav.reload").to_string(), Box::new(ReloadConnection))
                } else {
                    menu
                };
                let menu = if open {
                    menu.menu_with_disabled(
                        t!("nav.new_database").to_string(),
                        true,
                        Box::new(Refresh),
                    )
                    .menu(t!("nav.new_query").to_string(), Box::new(NewQuery))
                    .menu(t!("nav.console").to_string(), Box::new(OpenConsole))
                } else {
                    menu.submenu(t!("nav.new_connection").to_string(), new_conn)
                        .menu(
                            t!("nav.duplicate_connection").to_string(),
                            Box::new(DuplicateConnection),
                        )
                        .menu(
                            t!("nav.delete_connection").to_string(),
                            Box::new(DeleteConnection),
                        )
                };
                let conn = ConnectionStore::global(cx).read(cx).get(id);
                let starred = conn.is_some_and(|c| c.starred);
                let star = if starred {
                    t!("nav.unstar")
                } else {
                    t!("nav.star")
                };
                // F-CONN-021
                let profiles = conn.map_or(NativeMenu::new(), |c| {
                    c.profile_names()
                        .fold(NativeMenu::new(), |m, n| {
                            let label = if n.is_empty() {
                                t!("nav.profile_default").to_string()
                            } else {
                                n.to_string()
                            };
                            m.menu_with_check(
                                label,
                                n == c.active_profile_name(),
                                Box::new(SwitchProfile {
                                    name: n.to_string().into(),
                                }),
                            )
                        })
                        .separator()
                        .menu(t!("nav.new_profile").to_string(), Box::new(NewProfile))
                        .menu_with_disabled(
                            t!("nav.delete_profile").to_string(),
                            c.profiles.len() <= 1,
                            Box::new(DeleteProfile),
                        )
                });
                let mut menu = menu
                    .separator()
                    .submenu(t!("nav.color").to_string(), colors)
                    .menu(star.to_string(), Box::new(ToggleStar))
                    .submenu(t!("nav.switch_profile").to_string(), profiles);
                if !open {
                    menu = menu.submenu(t!("nav.move_to_group").to_string(), move_menu);
                }
                refresh(menu)
            }
            // Items of later milestones (designers M4, queries M3, SQL files M6) are disabled
            Some(Node::Database(conn, db)) => {
                let dialect = Sessions::global(cx).read(cx).dialect(conn);
                let features = dialect.as_ref().map(|d| d.features());
                let mysql = features.is_some_and(|f| f.inline_table_options);
                let postgres = features.is_some_and(|f| f.has_schemas);
                let sqlite = features.is_some_and(|f| f.rebuilds_tables);
                let scope = Scope::new(conn.clone(), db.clone(), None);
                let mut menu = NativeMenu::new()
                    .menu(t!("nav.open_database").to_string(), Box::new(OpenDatabase))
                    .menu(
                        t!("nav.close_database").to_string(),
                        Box::new(CloseDatabase),
                    )
                    .separator()
                    .menu_with_disabled(
                        t!("nav.new_database").to_string(),
                        !(mysql || postgres),
                        Box::new(OpenDefinition {
                            scope: scope.clone(),
                            kind: "database".into(),
                            name: None,
                        }),
                    )
                    .menu_with_disabled(
                        t!("nav.edit_database").to_string(),
                        !(mysql || postgres),
                        Box::new(OpenDefinition {
                            scope: scope.clone(),
                            kind: "database".into(),
                            name: Some(db.clone().into()),
                        }),
                    )
                    .menu_with_disabled(
                        t!("nav.attach_database").to_string(),
                        !sqlite,
                        Box::new(OpenDefinition {
                            scope: scope.clone(),
                            kind: "attach".into(),
                            name: None,
                        }),
                    )
                    .menu_with_disabled(
                        t!("nav.detach_database").to_string(),
                        !sqlite || db.eq_ignore_ascii_case("main"),
                        Box::new(OpenDefinition {
                            scope: scope.clone(),
                            kind: "detach".into(),
                            name: Some(db.clone().into()),
                        }),
                    );
                if let Some(sql) = dialect.as_ref().and_then(|d| d.master_table_sql(db)) {
                    menu = menu.menu(
                        t!("nav.view_master").to_string(),
                        Box::new(NewQueryText {
                            connection: conn.clone().into(),
                            database: db.clone().into(),
                            schema: None,
                            sql: sql.into(),
                        }),
                    );
                }
                if let Some(d) = dialect.as_ref() {
                    menu = with_maintain(menu, d.as_ref(), &scope, None);
                }
                refresh(
                    menu.menu_with_disabled(
                        t!("nav.delete_database").to_string(),
                        true,
                        Box::new(Refresh),
                    )
                    .separator()
                    .menu(t!("nav.new_query").to_string(), Box::new(NewQuery))
                    .menu_with_disabled(t!("nav.run_sql_file").to_string(), true, Box::new(Refresh))
                    .menu_with_disabled(
                        t!("nav.dump_sql_file").to_string(),
                        true,
                        Box::new(Refresh),
                    ),
                )
            }
            Some(Node::Schema(scope)) => refresh(
                NativeMenu::new()
                    .menu(
                        t!("nav.new_schema").to_string(),
                        Box::new(OpenDefinition {
                            scope: scope.clone(),
                            kind: "schema".into(),
                            name: None,
                        }),
                    )
                    .menu(
                        t!("nav.edit_schema").to_string(),
                        Box::new(OpenDefinition {
                            scope: scope.clone(),
                            kind: "schema".into(),
                            name: scope.schema.clone().map(Into::into),
                        }),
                    )
                    .menu(t!("nav.new_query").to_string(), Box::new(NewQuery)),
            ),
            Some(Node::Category(scope, k)) => refresh(
                NativeMenu::new()
                    .menu_with_disabled(
                        t!("obj.new_object", kind = kind_label(*k)).to_string(),
                        !matches!(*k, ObjectKind::Query | ObjectKind::Table)
                            && navidog_sql::definition::DefinitionKind::from_object(*k).is_none(),
                        if *k == ObjectKind::Query {
                            Box::new(NewQuery)
                        } else if *k == ObjectKind::Table {
                            Box::new(NewTable)
                        } else {
                            Box::new(OpenDefinition {
                                scope: scope.clone(),
                                kind: k.key().into(),
                                name: None,
                            })
                        },
                    )
                    .separator()
                    .submenu(
                        t!("obj.manage_groups").to_string(),
                        NativeMenu::new()
                            .menu(t!("obj.new_vgroup").to_string(), Box::new(NewVirtualGroup)),
                    ),
            ),
            Some(Node::VGroup(_, _, g)) => refresh(
                NativeMenu::new()
                    .menu(
                        t!("obj.rename").to_string(),
                        Box::new(RenameVirtualGroup {
                            group: g.clone().into(),
                        }),
                    )
                    .menu(
                        t!("nav.delete_group").to_string(),
                        Box::new(DeleteVirtualGroup {
                            group: g.clone().into(),
                        }),
                    ),
            ),
            Some(Node::Object(o)) => refresh(Self::object_menu(o, cx)),
            Some(Node::TableItem(table, Child::Fields, name)) => refresh(
                NativeMenu::new()
                    .menu(
                        t!("obj.design").to_string(),
                        Box::new(DesignField {
                            object: table.clone(),
                            column: name.clone().into(),
                        }),
                    )
                    .menu(
                        t!("obj.rename").to_string(),
                        Box::new(RenameColumn {
                            object: table.clone(),
                            column: name.clone().into(),
                        }),
                    ),
            ),
            Some(_) => NativeMenu::new().menu(t!("nav.refresh").to_string(), Box::new(Refresh)),
        }
    }

    /// Context menu of a saved query (30 §7).
    fn query_menu(o: &ObjectRef) -> NativeMenu {
        let kind = kind_label(o.kind);
        NativeMenu::new()
            .menu(
                t!("obj.open_kind", kind = kind).to_string(),
                Box::new(RunQuery { object: o.clone() }),
            )
            .menu(
                t!("obj.design_kind", kind = kind).to_string(),
                Box::new(OpenObject { object: o.clone() }),
            )
            .menu(
                t!("obj.new_object", kind = kind).to_string(),
                Box::new(NewQuery),
            )
            .menu(
                t!("obj.delete_kind", kind = kind).to_string(),
                Box::new(DeleteObjects),
            )
            .menu(t!("obj.rename").to_string(), Box::new(RenameObject))
            .menu(
                t!("obj.duplicate_query").to_string(),
                Box::new(DuplicateQuery),
            )
            .separator()
            .menu(
                t!("obj.open_external").to_string(),
                Box::new(OpenQueryExternally),
            )
            .menu(t!("obj.reveal").to_string(), Box::new(RevealQuery))
            .separator()
            .menu(t!("obj.copy_name").to_string(), Box::new(CopyObjectNames))
    }

    /// Context menu of an object, shared with the objects tab.
    pub(crate) fn object_menu(o: &ObjectRef, cx: &App) -> NativeMenu {
        if o.kind == ObjectKind::Query {
            return Self::query_menu(o);
        }
        let d = Sessions::global(cx).read(cx).dialect(&o.scope.connection);
        let d = d.as_ref();
        let kind = kind_label(o.kind);
        let droppable = d.and_then(|d| d.drop_keyword(o.kind)).is_some();
        let renamable = d
            .and_then(|d| d.rename_object(&o.scope, o.kind, &o.name, &o.name))
            .is_some();
        let mut menu = NativeMenu::new()
            .menu(
                t!("obj.open_kind", kind = kind).to_string(),
                Box::new(OpenObject { object: o.clone() }),
            )
            .menu(
                t!("obj.design_kind", kind = kind).to_string(),
                if o.kind == ObjectKind::Table {
                    Box::new(DesignTable {
                        object: Some(o.clone()),
                    })
                } else {
                    Box::new(OpenObject { object: o.clone() })
                },
            )
            .menu(
                t!("obj.new_object", kind = kind).to_string(),
                if o.kind == ObjectKind::Table {
                    Box::new(NewTable)
                } else {
                    Box::new(OpenDefinition {
                        scope: o.scope.clone(),
                        kind: o.kind.key().into(),
                        name: None,
                    })
                },
            )
            .menu_with_disabled(
                t!("obj.delete_kind", kind = kind).to_string(),
                !droppable,
                Box::new(DeleteObjects),
            );
        if d.as_ref()
            .and_then(|d| d.refresh_materialized_view(&o.scope, &o.name, true))
            .is_some()
        {
            menu = menu
                .menu(
                    t!("obj.refresh_mview").to_string(),
                    Box::new(RefreshMaterializedView {
                        object: o.clone(),
                        with_data: true,
                    }),
                )
                .menu(
                    t!("obj.refresh_mview_empty").to_string(),
                    Box::new(RefreshMaterializedView {
                        object: o.clone(),
                        with_data: false,
                    }),
                );
        }
        if o.kind == ObjectKind::Table {
            if let Some(d) = d {
                menu = with_maintain(menu, d.as_ref(), &o.scope, Some(&o.name));
            }
            menu = menu.menu(t!("obj.empty_table").to_string(), Box::new(EmptyTables));
            if d.as_ref().is_none_or(|d| !d.features().exposes_rowid) {
                menu = menu.menu(
                    t!("obj.truncate_table").to_string(),
                    Box::new(TruncateTables),
                );
            }
            menu = menu.submenu(
                t!("obj.duplicate").to_string(),
                NativeMenu::new()
                    .menu(
                        t!("obj.duplicate_data").to_string(),
                        Box::new(DuplicateTable { with_data: true }),
                    )
                    .menu(
                        t!("obj.duplicate_structure").to_string(),
                        Box::new(DuplicateTable { with_data: false }),
                    ),
            );
        }
        let vgroups = VirtualGroups::global(cx).read(cx);
        let add = vgroups
            .groups_for(&o.scope, o.kind)
            .fold(NativeMenu::new(), |m, g| {
                m.menu(
                    g.name.clone(),
                    Box::new(AddToVirtualGroup {
                        group: g.id.clone().into(),
                    }),
                )
            });
        let has_groups = vgroups.groups_for(&o.scope, o.kind).next().is_some();
        let mut groups =
            NativeMenu::new().menu(t!("obj.new_vgroup").to_string(), Box::new(NewVirtualGroup));
        groups = if has_groups {
            groups.submenu(t!("obj.add_to_vgroup").to_string(), add)
        } else {
            groups.menu_with_disabled(t!("obj.add_to_vgroup").to_string(), true, Box::new(Refresh))
        };
        let grouped = vgroups.group_of(o).is_some();
        menu.menu_with_disabled(
            t!("obj.rename").to_string(),
            !renamable,
            Box::new(RenameObject),
        )
        .separator()
        .submenu(
            t!("obj.manage_groups").to_string(),
            groups.menu_with_disabled(
                t!("obj.remove_from_vgroup").to_string(),
                !grouped,
                Box::new(RemoveFromVirtualGroup),
            ),
        )
        .menu(t!("obj.copy_name").to_string(), Box::new(CopyObjectNames))
    }
}

fn with_maintain(
    menu: NativeMenu,
    dialect: &dyn Dialect,
    scope: &Scope,
    target: Option<&str>,
) -> NativeMenu {
    let jobs =
        navidog_sql::security::maintenance(dialect, dialect.definition_family(), scope, target);
    if jobs.is_empty() {
        return menu;
    }
    let sub = jobs.into_iter().fold(NativeMenu::new(), |menu, job| {
        menu.menu(
            maintain_label(job.key),
            Box::new(RunMaintenance {
                connection: scope.connection.clone().into(),
                database: scope.database.clone().into(),
                sql: job.sql.into(),
            }),
        )
    });
    menu.submenu(t!("nav.maintain").to_string(), sub)
}

fn maintain_label(key: &str) -> String {
    match key {
        "analyze" => t!("nav.maintain_analyze").to_string(),
        "check" => t!("nav.maintain_check").to_string(),
        "optimize" => t!("nav.maintain_optimize").to_string(),
        "repair" => t!("nav.maintain_repair").to_string(),
        "vacuum" => t!("nav.maintain_vacuum").to_string(),
        "reindex" => t!("nav.maintain_reindex").to_string(),
        "integrity" => t!("nav.maintain_integrity").to_string(),
        _ => key.to_string(),
    }
}
