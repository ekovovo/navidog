//! Context menus of the tree's rows and of its blank area.

use super::*;

impl NavigatorPanel {
    pub(crate) fn context_menu(node: Option<&Node>, cx: &App) -> NativeMenu {
        let new_conn =
            navidog_db::DriverRegistry::all(cx)
                .into_iter()
                .fold(NativeMenu::new(), |m, d| {
                    let info = d.info();
                    m.menu(
                        info.name.clone(),
                        Box::new(NewConnectionOf {
                            driver: info.id.clone(),
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
                let mysql = Sessions::global(cx).read(cx).dialect(id) == Some(Dialect::MySql);
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
                                n == c.profile,
                                Box::new(SwitchProfile {
                                    name: n.to_string().into(),
                                }),
                            )
                        })
                        .separator()
                        .menu(t!("nav.new_profile").to_string(), Box::new(NewProfile))
                        .menu_with_disabled(
                            t!("nav.delete_profile").to_string(),
                            c.profiles.is_empty(),
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
            Some(Node::Database(..)) => refresh(
                NativeMenu::new()
                    .menu(t!("nav.open_database").to_string(), Box::new(OpenDatabase))
                    .menu(
                        t!("nav.close_database").to_string(),
                        Box::new(CloseDatabase),
                    )
                    .separator()
                    .menu_with_disabled(t!("nav.new_database").to_string(), true, Box::new(Refresh))
                    .menu_with_disabled(
                        t!("nav.edit_database").to_string(),
                        true,
                        Box::new(Refresh),
                    )
                    .menu_with_disabled(
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
            ),
            Some(Node::Schema(_)) => {
                refresh(NativeMenu::new().menu(t!("nav.new_query").to_string(), Box::new(NewQuery)))
            }
            Some(Node::Category(_, k)) => refresh(
                NativeMenu::new()
                    .menu_with_disabled(
                        t!("obj.new_object", kind = kind_label(*k)).to_string(),
                        !matches!(*k, ObjectKind::Query | ObjectKind::Table),
                        if *k == ObjectKind::Query {
                            Box::new(NewQuery)
                        } else {
                            Box::new(NewTable)
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
            .menu_with_disabled(
                t!("obj.design_kind", kind = kind).to_string(),
                o.kind != ObjectKind::Table,
                Box::new(DesignTable {
                    object: Some(o.clone()),
                }),
            )
            .menu_with_disabled(
                t!("obj.new_object", kind = kind).to_string(),
                o.kind != ObjectKind::Table,
                Box::new(NewTable),
            )
            .menu_with_disabled(
                t!("obj.delete_kind", kind = kind).to_string(),
                !droppable,
                Box::new(DeleteObjects),
            );
        if o.kind == ObjectKind::Table {
            menu = menu.menu(t!("obj.empty_table").to_string(), Box::new(EmptyTables));
            if d != Some(Dialect::Sqlite) {
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
