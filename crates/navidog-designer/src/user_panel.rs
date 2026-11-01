// User list and the Navicat user designer. Included into `tools.rs` so it can
// reach the panel fields. SQL stays in `navidog-sql`.

const PLUGINS: &[&str] = &[
    "",
    "caching_sha2_password",
    "mysql_native_password",
    "sha256_password",
];
const SSL_TYPES: &[&str] = &["", "NONE", "ANY", "X509", "SPECIFIED"];

impl ToolPanel {
    fn start(&mut self, cx: &mut Context<Self>) {
        if self.tool != Tool::Users
            || matches!(self.dialect.account_form(), AccountForm::Unavailable(_))
        {
            return;
        }
        match self.target.as_deref() {
            None => self.list_accounts(cx),
            Some("new") => self.loaded_draft = self.account_draft(cx),
            Some(_) => self.load_detail(cx),
        }
    }

    fn editing(&self) -> Option<(String, String)> {
        let target = self.target.as_deref()?;
        let rest = target.strip_prefix("edit\u{1f}")?;
        let (name, host) = rest.split_once('\u{1f}')?;
        Some((name.to_string(), host.to_string()))
    }

    fn user_tab_title(&self, cx: &App) -> String {
        match self.target.as_deref() {
            None => t!("designer.tool_users").to_string(),
            Some("new") => {
                let name = Self::text(&self.name, cx);
                if name.trim().is_empty() {
                    t!("designer.tool_user_new").to_string()
                } else {
                    self.account_label(cx)
                }
            }
            Some(_) => self.account_label(cx),
        }
    }

    fn account_label(&self, cx: &App) -> String {
        let name = Self::text(&self.name, cx);
        let host = Self::text(&self.host, cx);
        let (name, host) = if name.trim().is_empty() {
            self.editing().unwrap_or_default()
        } else {
            (name, host)
        };
        if host.is_empty() {
            name
        } else {
            format!("{name}@{host}")
        }
    }

    fn user_dirty(&self, cx: &App) -> bool {
        if self.target.is_none() {
            return false;
        }
        let draft = self.account_draft(cx);
        if !draft.password.trim().is_empty() {
            return true;
        }
        comparable(draft) != comparable(self.loaded_draft.clone())
            || self.current_grants() != (self.loaded_grants.clone(), self.loaded_members.clone())
    }

    fn current_grants(&self) -> (GrantSet, BTreeSet<(String, String)>) {
        let mut grants = self.loaded_grants.clone();
        overlay_privs(
            &mut grants.server,
            &mut grants.server_grant_option,
            &self.server_on,
            SERVER_PRIVILEGES,
        );
        for row in &self.objects {
            let key = (row.schema.clone(), row.name.clone());
            let existed = grants.objects.contains_key(&key);
            let mut grant = grants.objects.get(&key).cloned().unwrap_or_default();
            overlay_privs(
                &mut grant.privileges,
                &mut grant.grant_option,
                &row.on,
                OBJECT_PRIVILEGES,
            );
            let any = grant.grant_option || !grant.privileges.is_empty();
            if any || existed {
                grants.objects.insert(key, grant);
            }
        }
        grants.roles = self
            .roles
            .iter()
            .filter(|row| row.member_of)
            .map(|row| (row.name.clone(), row.host.clone()))
            .collect();
        grants.defaults = self
            .roles
            .iter()
            .filter(|row| row.default_role)
            .map(|row| (row.name.clone(), row.host.clone()))
            .collect();
        let members = self
            .roles
            .iter()
            .filter(|row| row.member)
            .map(|row| (row.name.clone(), row.host.clone()))
            .collect();
        (grants, members)
    }

    fn apply_fill(&mut self, fill: ProfileFill, window: &mut Window, cx: &mut Context<Self>) {
        let mut set = |input: &Entity<InputState>, value: &str, cx: &mut Context<Self>| {
            let value = value.to_string();
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        };
        set(&self.name, &fill.name, cx);
        set(&self.host, if fill.host.is_empty() { "%" } else { &fill.host }, cx);
        set(&self.max_queries, &fill.max_queries, cx);
        set(&self.max_updates, &fill.max_updates, cx);
        set(&self.max_connections, &fill.max_connections, cx);
        set(&self.max_user_connections, &fill.max_user_connections, cx);
        set(&self.ssl_cipher, &fill.ssl_cipher, cx);
        set(&self.ssl_issuer, &fill.ssl_issuer, cx);
        set(&self.ssl_subject, &fill.ssl_subject, cx);
        if let Some(attrs) = &fill.postgres {
            self.login = attrs.login;
            self.superuser = attrs.superuser;
            self.createdb = attrs.createdb;
            self.createrole = attrs.createrole;
            self.inherit = attrs.inherit;
            self.replication = attrs.replication;
            set(&self.connection_limit, &attrs.connection_limit, cx);
            set(&self.valid_until, &attrs.valid_until, cx);
        }
        self.locked = fill.locked;
        select_value(&self.plugin_choice, &fill.plugin, PLUGINS, window, cx);
        select_value(&self.ssl_choice, &fill.ssl_type, SSL_TYPES, window, cx);
        self.loaded_draft = self.account_draft(cx);
        self.loaded_draft.previous_name = Some(fill.name.clone());
        self.loaded_draft.previous_host = Some(fill.host);
        self.loaded_draft.previous_plugin = Some(fill.plugin);
        self.loaded_draft.previous_ssl = Some(fill.ssl_type);
        self.loaded_draft.previous_cipher = Some(fill.ssl_cipher);
        self.loaded_draft.previous_issuer = Some(fill.ssl_issuer);
        self.loaded_draft.previous_subject = Some(fill.ssl_subject);
        self.loaded_draft.previous_locked = Some(fill.locked);
        self.loaded_draft.password.clear();
    }

    fn apply_fetch(&mut self, sets: Vec<ResultSet>, cx: &mut Context<Self>) {
        match self.fetch_kind {
            FetchKind::Output => {
                self.output = format_sets(sets);
                self.preview.clear();
                self.error = None;
                if self.tool == Tool::Users && self.target.is_some() {
                    self.adopt_saved(cx);
                }
            }
            FetchKind::Accounts => {
                if let Some(set) = sets.iter().find(|set| !set.columns.is_empty()) {
                    self.store_accounts(set);
                }
                self.error = None;
            }
            FetchKind::Detail => self.apply_detail(sets, cx),
            FetchKind::Defaults => self.apply_defaults(sets, cx),
            FetchKind::Members => self.apply_members(&sets),
        }
    }

    fn finish_optional_fetch(&mut self, cx: &mut Context<Self>) {
        if self.fetch_kind == FetchKind::Defaults {
            self.load_members(cx);
        }
    }

    fn load_detail(&mut self, cx: &mut Context<Self>) {
        let Some((name, host)) = self.editing() else {
            return;
        };
        if self.dialect.account_form() == AccountForm::Postgres {
            match security::role_profile_sql(self.dialect.as_ref(), &name) {
                Ok(sql) => self.spawn_query(vec![sql], true, FetchKind::Detail, cx),
                Err(error) => self.error = Some(error),
            }
            return;
        }
        if self.dialect.account_form() != AccountForm::Mysql {
            self.pending_fill = Some(ProfileFill::named(name, host));
            self.loaded_draft = AccountDraft::default();
            return;
        }
        match security::user_load_sql(
            self.dialect.as_ref(),
            self.dialect.definition_family(),
            &name,
            &host,
            &self.scope.database,
        ) {
            Ok(sql) => {
                self.expect_tables = sql.tables.is_some();
                let mut queries = vec![sql.profile, sql.grants, sql.accounts];
                if let Some(tables) = sql.tables {
                    queries.push(tables);
                }
                self.spawn_query(queries, true, FetchKind::Detail, cx);
            }
            Err(error) => {
                self.pending_fill = Some(ProfileFill::named(name, host));
                self.error = Some(error);
            }
        }
    }

    fn load_defaults(&mut self, cx: &mut Context<Self>) {
        let Some((name, host)) = self.editing() else {
            return;
        };
        let Ok(sql) = security::user_load_sql(
            self.dialect.as_ref(),
            self.dialect.definition_family(),
            &name,
            &host,
            &self.scope.database,
        ) else {
            return;
        };
        self.spawn_query(vec![sql.defaults], true, FetchKind::Defaults, cx);
    }

    fn load_members(&mut self, cx: &mut Context<Self>) {
        let Some((name, host)) = self.editing() else {
            return;
        };
        let Ok(sql) = security::user_load_sql(
            self.dialect.as_ref(),
            self.dialect.definition_family(),
            &name,
            &host,
            &self.scope.database,
        ) else {
            return;
        };
        self.spawn_query(vec![sql.members], true, FetchKind::Members, cx);
    }

    fn adopt_saved(&mut self, cx: &mut Context<Self>) {
        let Some(running) = self.running_account.take() else {
            return;
        };
        let snapshot = match self.account_preview.clone() {
            Some(later) => AccountPreview {
                account: adopted_account(running.account, later.account),
                ..running
            },
            None => running,
        };
        let clear_password = Self::text(&self.password, cx).trim() == snapshot.password;
        self.loaded_grants = snapshot.grants;
        self.loaded_members = snapshot.members;
        self.grants_touched = false;
        self.loaded_draft = snapshot.account;
        self.clear_password = clear_password;
    }

    fn apply_detail(&mut self, sets: Vec<ResultSet>, cx: &mut Context<Self>) {
        let (name, host) = self.editing().unwrap_or_default();
        if self.dialect.account_form() == AccountForm::Postgres {
            self.pending_fill = Some(match sets.first().and_then(|set| set.rows.first()) {
                Some(row) => postgres_profile(name, host, row),
                None => ProfileFill::named(name, host),
            });
            self.error = None;
            return;
        }
        if let Some(row) = sets.first().and_then(|set| set.rows.first()) {
            let cell = |index: usize| {
                row.get(index)
                    .and_then(|value| value.clone())
                    .unwrap_or_default()
            };
            self.pending_fill = Some(ProfileFill {
                name: name.clone(),
                host: host.clone(),
                plugin: cell(0),
                locked: cell(1) == "1",
                max_queries: cell(3),
                max_updates: cell(4),
                max_connections: cell(5),
                max_user_connections: cell(6),
                ssl_type: cell(7),
                ssl_cipher: cell(8),
                ssl_issuer: cell(9),
                ssl_subject: cell(10),
                postgres: None,
            });
        } else {
            self.pending_fill = Some(ProfileFill::named(name, host));
        }
        if self.grants_touched {
            return;
        }
        let lines = sets.get(1).map(grant_lines).unwrap_or_default();
        let mut grants = security::parse_show_grants(&lines);
        expand_known(&mut grants.server, SERVER_PRIVILEGES);
        for grant in grants.objects.values_mut() {
            expand_known(&mut grant.privileges, OBJECT_PRIVILEGES);
        }
        self.server_on = checks_from(&grants.server, grants.server_grant_option, SERVER_PRIVILEGES);
        if let Some(accounts) = sets.get(2) {
            self.roles = roles_from(accounts, &grants, &self.editing());
        }
        let mut objects = Vec::new();
        if self.expect_tables
            && let Some(tables) = sets.get(3)
        {
            for row in &tables.rows {
                let schema = cell_at(row, 0);
                let name = cell_at(row, 1);
                if !schema.is_empty() {
                    objects.push(object_row(schema, name, &grants));
                }
            }
        }
        for key in grants.objects.keys() {
            if !objects
                .iter()
                .any(|row| row.schema == key.0 && row.name == key.1)
            {
                objects.push(object_row(key.0.clone(), key.1.clone(), &grants));
            }
        }
        self.objects = objects;
        self.loaded_grants = grants;
        self.error = None;
        self.load_defaults(cx);
    }

    fn apply_defaults(&mut self, sets: Vec<ResultSet>, cx: &mut Context<Self>) {
        if !self.grants_touched
            && let Some(set) = sets.first()
        {
            for row in &set.rows {
                let name = cell_at(row, 0);
                let host = cell_at(row, 1);
                if let Some(role) = self
                    .roles
                    .iter_mut()
                    .find(|role| role.name == name && role.host == host)
                {
                    role.default_role = true;
                }
                self.loaded_grants.defaults.insert((name, host));
            }
        }
        self.load_members(cx);
    }

    fn apply_members(&mut self, sets: &[ResultSet]) {
        if self.grants_touched {
            return;
        }
        let Some(set) = sets.first() else {
            return;
        };
        for row in &set.rows {
            let name = cell_at(row, 0);
            let host = cell_at(row, 1);
            if let Some(role) = self
                .roles
                .iter_mut()
                .find(|role| role.name == name && role.host == host)
            {
                role.member = true;
            }
            self.loaded_members.insert((name, host));
        }
    }

    fn store_accounts(&mut self, set: &ResultSet) {
        self.account_columns = set.columns.iter().map(|column| column.name.clone()).collect();
        self.account_rows = set
            .rows
            .iter()
            .map(|row| row.iter().map(|cell| cell.clone().unwrap_or_default()).collect())
            .collect();
    }

    fn open_account(&self, name: String, host: String, window: &mut Window, cx: &mut App) {
        window.dispatch_action(
            Box::new(OpenTool {
                scope: Some(self.scope.clone()),
                tool: "users".into(),
                target: Some(format!("edit\u{1f}{name}\u{1f}{host}").into()),
            }),
            cx,
        );
    }

    fn render_users(
        &self,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        if let AccountForm::Unavailable(reason) = self.dialect.account_form() {
            return v_flex()
                .id("tool-users")
                .flex_1()
                .min_h_0()
                .p_3()
                .child(reason.to_string())
                .into_any_element();
        }
        if self.target.is_none() {
            self.render_user_list(disabled, window, cx)
        } else {
            self.render_user_designer(disabled, window, cx)
        }
    }

    fn render_user_list(
        &self,
        disabled: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let p = palette(cx);
        let filter = Self::text(&self.user_filter, cx).to_lowercase();
        let rows: Vec<(usize, Vec<String>)> = self
            .account_rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                filter.is_empty()
                    || row.iter().any(|cell| cell.to_lowercase().contains(&filter))
            })
            .map(|(ix, row)| (ix, row.clone()))
            .collect();
        let count = rows.len();
        v_flex()
            .id("tool-users")
            .flex_1()
            .min_h_0()
            .size_full()
            .child(
                h_flex()
                    .h(tokens::object_toolbar::HEIGHT.rems())
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(p.separator)
                    .child(
                        Button::new("user-new")
                            .ghost()
                            .small()
                            .icon(lucide("plus"))
                            .label(t!("designer.tool_user_new").to_string())
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                window.dispatch_action(
                                    Box::new(OpenTool {
                                        scope: Some(this.scope.clone()),
                                        tool: "users".into(),
                                        target: Some("new".into()),
                                    }),
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("user-design")
                            .ghost()
                            .small()
                            .icon(lucide("pencil"))
                            .label(t!("designer.tool_design").to_string())
                            .disabled(disabled || self.selected_account.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                let Some(ix) = this.selected_account else {
                                    return;
                                };
                                let Some(row) = this.account_rows.get(ix) else {
                                    return;
                                };
                                let (name, host) = row_identity(&this.account_columns, row);
                                this.open_account(name, host, window, cx);
                            })),
                    )
                    .child(
                        Button::new("user-refresh")
                            .ghost()
                            .small()
                            .icon(lucide("refresh-cw"))
                            .label(t!("designer.tool_refresh").to_string())
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, _, cx| this.list_accounts(cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Input::new(&self.user_filter)
                            .id("user-list-filter")
                            .small()
                            .w(tokens::form::POPUP_WIDTH_SHORT.rems()),
                    ),
            )
            .child(
                h_flex()
                    .h(tokens::list::HEADER_HEIGHT.rems())
                    .border_b_1()
                    .border_color(p.separator)
                    .text_color(p.label_secondary)
                    .children(self.account_columns.iter().map(|column| {
                        div()
                            .w(column_width(column).rems())
                            .flex_none()
                            .px_2()
                            .min_w_0()
                            .child(truncated(
                                SharedString::from(format!("user-th-{column}")),
                                column_label(column),
                            ))
                    })),
            )
            .child(
                v_flex()
                    .id("user-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows.into_iter().map(|(ix, row)| {
                        let selected = self.selected_account == Some(ix);
                        let (name, host) = row_identity(&self.account_columns, &row);
                        h_flex()
                            .id(SharedString::from(format!("user-row-{ix}")))
                            .h(tokens::list::ROW_HEIGHT.rems())
                            .when(selected, |line| {
                                line.bg(p.selection).text_color(p.selection_text)
                            })
                            .children(self.account_columns.iter().zip(row.iter()).map(
                                |(column, cell)| {
                                    div()
                                        .w(column_width(column).rems())
                                        .flex_none()
                                        .px_2()
                                        .min_w_0()
                                        .child(truncated(
                                            SharedString::from(format!("user-cell-{ix}-{column}")),
                                            cell.clone(),
                                        ))
                                },
                            ))
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                this.selected_account = Some(ix);
                                if event.click_count() >= 2 {
                                    this.open_account(name.clone(), host.clone(), window, cx);
                                }
                                cx.notify();
                            }))
                    })),
            )
            .child(
                div()
                    .h(tokens::list::HEADER_HEIGHT.rems())
                    .px_2()
                    .border_t_1()
                    .border_color(p.separator)
                    .text_color(p.label_secondary)
                    .child(t!("designer.tool_users_count", n = count).to_string()),
            )
            .into_any_element()
    }

    fn render_user_designer(
        &self,
        disabled: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let mysql = self.dialect.account_form() == AccountForm::Mysql;
        let mut pages = vec![(UserPage::General, "general")];
        if mysql && !self.role {
            pages.push((UserPage::Advanced, "advanced"));
            pages.push((UserPage::MemberOf, "member-of"));
        }
        if mysql {
            pages.push((UserPage::Members, "members"));
            pages.push((UserPage::Server, "server"));
            pages.push((UserPage::Objects, "objects"));
        }
        pages.push((UserPage::Preview, "preview"));
        if !pages.iter().any(|(page, _)| *page == self.user_page) {
            // The page is a field on self; the render borrow is shared. Leave a
            // stale page to fall through as general by picking the first match below.
        }
        let selected = pages
            .iter()
            .position(|(page, _)| *page == self.user_page)
            .unwrap_or(0);
        let page = pages.get(selected).map(|(page, _)| *page).unwrap_or(UserPage::General);
        let pages_for_bar = pages.clone();
        v_flex()
            .id("tool-user-designer")
            .flex_1()
            .min_h_0()
            .size_full()
            .child(
                TabBar::new("user-pages")
                    .underline()
                    .small()
                    .w_full()
                    .selected_index(selected)
                    .on_click(cx.listener(move |this, ix: &usize, window, cx| {
                        if let Some((page, _)) = pages_for_bar.get(*ix) {
                            this.user_page = *page;
                            if *page == UserPage::Preview {
                                this.show_account(window, cx);
                            }
                            cx.notify();
                        }
                    }))
                    .children(pages.into_iter().map(|(page, key)| {
                        Tab::new().child(
                            div()
                                .id(SharedString::from(format!("user-page-{key}")))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.user_page = page;
                                    if page == UserPage::Preview {
                                        this.show_account(window, cx);
                                    }
                                    cx.notify();
                                }))
                                .child(user_page_label(page))
                                .test_support(),
                        )
                    })),
            )
            .child(match page {
                UserPage::General => self.render_user_general(disabled, mysql, cx),
                UserPage::Advanced => self.render_user_advanced(disabled, cx),
                UserPage::MemberOf => self.render_roles(disabled, true, cx),
                UserPage::Members => self.render_roles(disabled, false, cx),
                UserPage::Server => self.render_server_privs(disabled, cx),
                UserPage::Objects => self.render_object_privs(disabled, cx),
                UserPage::Preview => Editor::new(&self.sql_view).size_full().into_any_element(),
            })
            .into_any_element()
    }

    fn render_user_general(
        &self,
        disabled: bool,
        mysql: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        v_flex()
            .id("user-general")
            .flex_1()
            .min_h_0()
            .p_3()
            .gap_2()
            .overflow_y_scroll()
            .child(self.field_row(
                "user-name",
                t!("designer.name").to_string(),
                &self.name,
                false,
                disabled,
            ))
            .when(mysql && !self.role, |col| {
                col.child(self.field_row(
                    "user-host",
                    t!("designer.tool_host").to_string(),
                    &self.host,
                    false,
                    disabled,
                ))
                .child(
                    FormRow::new(
                        t!("designer.tool_plugin").to_string(),
                        Select::new(&self.plugin_choice)
                            .id("user-plugin")
                            .small()
                            .w(tokens::form::FIELD_WIDTH.rems()),
                    )
                    .hug_label(),
                )
            })
            .when(!self.role, |col| {
                col.child(self.field_row(
                    "user-password",
                    t!("designer.tool_password").to_string(),
                    &self.password,
                    true,
                    disabled,
                ))
                .child(self.field_row(
                    "user-confirm",
                    t!("designer.tool_confirm").to_string(),
                    &self.confirm,
                    true,
                    disabled,
                ))
            })
            .when(mysql, |col| {
                col.child(check(
                    "user-role",
                    "tool_role",
                    self.role,
                    disabled,
                    cx,
                    |this, on| this.role = on,
                ))
            })
            .when(!mysql, |col| {
                col.child(check(
                    "user-login",
                    "tool_login",
                    self.login,
                    disabled,
                    cx,
                    |this, on| this.login = on,
                ))
                .child(check(
                    "user-superuser",
                    "tool_superuser",
                    self.superuser,
                    disabled,
                    cx,
                    |this, on| this.superuser = on,
                ))
                .child(check(
                    "user-createdb",
                    "tool_createdb",
                    self.createdb,
                    disabled,
                    cx,
                    |this, on| this.createdb = on,
                ))
                .child(check(
                    "user-createrole",
                    "tool_createrole",
                    self.createrole,
                    disabled,
                    cx,
                    |this, on| this.createrole = on,
                ))
                .child(check(
                    "user-inherit",
                    "tool_inherit",
                    self.inherit,
                    disabled,
                    cx,
                    |this, on| this.inherit = on,
                ))
                .child(check(
                    "user-replication",
                    "tool_replication",
                    self.replication,
                    disabled,
                    cx,
                    |this, on| this.replication = on,
                ))
                .child(self.field_row(
                    "user-connection-limit",
                    t!("designer.tool_connection_limit").to_string(),
                    &self.connection_limit,
                    false,
                    disabled,
                ))
                .child(self.field_row(
                    "user-valid-until",
                    t!("designer.tool_valid_until").to_string(),
                    &self.valid_until,
                    false,
                    disabled,
                ))
            })
            .into_any_element()
    }

    fn render_user_advanced(
        &self,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let specified = choice_text(&self.ssl_choice, cx).eq_ignore_ascii_case("SPECIFIED");
        v_flex()
            .id("user-advanced")
            .flex_1()
            .min_h_0()
            .p_3()
            .gap_2()
            .overflow_y_scroll()
            .child(self.field_row(
                "user-max-queries",
                t!("designer.tool_max_queries").to_string(),
                &self.max_queries,
                false,
                disabled,
            ))
            .child(self.field_row(
                "user-max-updates",
                t!("designer.tool_max_updates").to_string(),
                &self.max_updates,
                false,
                disabled,
            ))
            .child(self.field_row(
                "user-max-connections",
                t!("designer.tool_max_connections").to_string(),
                &self.max_connections,
                false,
                disabled,
            ))
            .child(self.field_row(
                "user-max-user-connections",
                t!("designer.tool_max_user_connections").to_string(),
                &self.max_user_connections,
                false,
                disabled,
            ))
            .child(
                div()
                    .text_color(palette(cx).label_secondary)
                    .child(t!("designer.tool_unlimited").to_string()),
            )
            .child(check(
                "user-locked",
                "tool_locked",
                self.locked,
                disabled,
                cx,
                |this, on| this.locked = on,
            ))
            .child(
                FormRow::new(
                    t!("designer.tool_ssl").to_string(),
                    Select::new(&self.ssl_choice)
                        .id("user-ssl")
                        .small()
                        .w(tokens::form::FIELD_WIDTH.rems()),
                )
                .hug_label(),
            )
            .when(specified, |col| {
                col.child(self.field_row(
                    "user-cipher",
                    t!("designer.tool_cipher").to_string(),
                    &self.ssl_cipher,
                    false,
                    disabled,
                ))
                .child(self.field_row(
                    "user-issuer",
                    t!("designer.tool_issuer").to_string(),
                    &self.ssl_issuer,
                    false,
                    disabled,
                ))
                .child(self.field_row(
                    "user-subject",
                    t!("designer.tool_subject").to_string(),
                    &self.ssl_subject,
                    false,
                    disabled,
                ))
            })
            .into_any_element()
    }

    fn field_row(
        &self,
        id: &'static str,
        label: String,
        input: &Entity<InputState>,
        masked: bool,
        disabled: bool,
    ) -> gpui_kit::AnyElement {
        let field = Input::new(input)
            .id(id)
            .small()
            .w(tokens::form::FIELD_WIDTH.rems())
            .disabled(disabled);
        let field = if masked { field.mask_toggle() } else { field };
        FormRow::new(label, field).hug_label().into_any_element()
    }

    fn render_roles(
        &self,
        disabled: bool,
        member_of: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let p = palette(cx);
        v_flex()
            .id(if member_of { "user-member-of" } else { "user-members" })
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .h(tokens::list::HEADER_HEIGHT.rems())
                    .px_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(p.separator)
                    .text_color(p.label_secondary)
                    .child(
                        div()
                            .flex_1()
                            .child(t!("designer.user_col_name").to_string()),
                    )
                    .child(
                        div()
                            .w(tokens::designer::FLAG_WIDTH.rems())
                            .child(t!("designer.tool_grant").to_string()),
                    )
                    .when(member_of, |row| {
                        row.child(
                            div()
                                .w(tokens::designer::FLAG_WIDTH.rems())
                                .child(t!("designer.tool_default_role").to_string()),
                        )
                    }),
            )
            .child(
                v_flex()
                    .id("user-role-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(self.roles.iter().enumerate().map(|(ix, role)| {
                        let label = if role.host.is_empty() {
                            role.name.clone()
                        } else {
                            format!("{}@{}", role.name, role.host)
                        };
                        h_flex()
                            .h(tokens::list::ROW_HEIGHT.rems())
                            .px_2()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div().flex_1().min_w_0().child(truncated(
                                    SharedString::from(format!("role-name-{member_of}-{ix}")),
                                    label,
                                )),
                            )
                            .child(
                                div().w(tokens::designer::FLAG_WIDTH.rems()).child(
                                    Checkbox::new(SharedString::from(format!(
                                        "role-{}-{ix}",
                                        if member_of { "of" } else { "has" }
                                    )))
                                    .checked(if member_of { role.member_of } else { role.member })
                                    .disabled(disabled)
                                    .on_click(cx.listener(move |this, value, _, cx| {
                                        if let Some(role) = this.roles.get_mut(ix) {
                                            if member_of {
                                                role.member_of = *value;
                                            } else {
                                                role.member = *value;
                                            }
                                            this.grants_touched = true;
                                        }
                                        cx.notify();
                                    })),
                                ),
                            )
                            .when(member_of, |row| {
                                row.child(div().w(tokens::designer::FLAG_WIDTH.rems()).child(
                                    Checkbox::new(SharedString::from(format!("role-default-{ix}")))
                                        .checked(role.default_role)
                                        .disabled(disabled)
                                        .on_click(cx.listener(move |this, value, _, cx| {
                                            if let Some(role) = this.roles.get_mut(ix) {
                                                role.default_role = *value;
                                                this.grants_touched = true;
                                            }
                                            cx.notify();
                                        })),
                                ))
                            })
                    })),
            )
            .into_any_element()
    }

    fn render_server_privs(
        &self,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let p = palette(cx);
        v_flex()
            .id("user-server")
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .h(tokens::list::HEADER_HEIGHT.rems())
                    .px_2()
                    .border_b_1()
                    .border_color(p.separator)
                    .text_color(p.label_secondary)
                    .child(div().flex_1().child(t!("designer.tool_privilege").to_string()))
                    .child(
                        div()
                            .w(tokens::designer::FLAG_WIDTH.rems())
                            .child(t!("designer.tool_grant").to_string()),
                    ),
            )
            .child(
                v_flex()
                    .id("user-server-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(SERVER_PRIVILEGES.iter().enumerate().map(|(ix, name)| {
                        h_flex()
                            .h(tokens::list::ROW_HEIGHT.rems())
                            .px_2()
                            .child(div().flex_1().child(priv_label(name)))
                            .child(
                                Checkbox::new(SharedString::from(format!("server-priv-{ix}")))
                                    .checked(self.server_on.get(ix).copied().unwrap_or(false))
                                    .disabled(disabled)
                                    .on_click(cx.listener(move |this, value, _, cx| {
                                        if let Some(on) = this.server_on.get_mut(ix) {
                                            *on = *value;
                                            this.grants_touched = true;
                                        }
                                        cx.notify();
                                    })),
                            )
                    })),
            )
            .into_any_element()
    }

    fn render_object_privs(
        &self,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let p = palette(cx);
        let filter = Self::text(&self.user_filter, cx).to_lowercase();
        v_flex()
            .id("user-objects")
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .h(tokens::object_toolbar::HEIGHT.rems())
                    .px_2()
                    .border_b_1()
                    .border_color(p.separator)
                    .justify_end()
                    .child(
                        Input::new(&self.user_filter)
                            .id("user-priv-filter")
                            .small()
                            .w(tokens::form::POPUP_WIDTH_SHORT.rems()),
                    ),
            )
            .child(
                v_flex()
                    .id("user-object-grid")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .child(
                        h_flex()
                            .h(tokens::list::HEADER_HEIGHT.rems())
                            .border_b_1()
                            .border_color(p.separator)
                            .text_color(p.label_secondary)
                            .child(object_head("schema", t!("designer.tool_schema").to_string()))
                            .child(object_head("name", t!("designer.user_col_name").to_string()))
                            .children(OBJECT_PRIVILEGES.iter().map(|name| {
                                object_head(
                                    name,
                                    if *name == "CREATE TEMPORARY TABLES" {
                                        "Create Temporary Tables".to_string()
                                    } else {
                                        priv_label(name).to_string()
                                    },
                                )
                            })),
                    )
                    .children(self.objects.iter().enumerate().filter_map(|(ix, row)| {
                        let haystack = format!("{}.{}", row.schema, row.name).to_lowercase();
                        if !filter.is_empty() && !haystack.contains(&filter) {
                            return None;
                        }
                        let schema = row.schema.clone();
                        let name = row.name.clone();
                        Some(
                            h_flex()
                                .h(tokens::list::ROW_HEIGHT.rems())
                                .child(object_cell(format!("obj-schema-{ix}"), schema))
                                .child(object_cell(format!("obj-name-{ix}"), name))
                                .children(row.on.iter().enumerate().map(|(priv_ix, on)| {
                                    div()
                                        .w(tokens::list::DETAIL_COLUMN_WIDTH.rems())
                                        .flex_none()
                                        .px_2()
                                        .child(
                                            Checkbox::new(SharedString::from(format!(
                                                "obj-priv-{ix}-{priv_ix}"
                                            )))
                                            .checked(*on)
                                            .disabled(disabled)
                                            .on_click(cx.listener(move |this, value, _, cx| {
                                                if let Some(row) = this.objects.get_mut(ix)
                                                    && let Some(flag) = row.on.get_mut(priv_ix)
                                                {
                                                    *flag = *value;
                                                    this.grants_touched = true;
                                                }
                                                cx.notify();
                                            })),
                                        )
                                })),
                        )
                    })),
            )
            .into_any_element()
    }
}

impl ProfileFill {
    fn named(name: String, host: String) -> Self {
        Self {
            name,
            host,
            plugin: String::new(),
            locked: false,
            max_queries: "0".into(),
            max_updates: "0".into(),
            max_connections: "0".into(),
            max_user_connections: "0".into(),
            ssl_type: String::new(),
            ssl_cipher: String::new(),
            ssl_issuer: String::new(),
            ssl_subject: String::new(),
            postgres: None,
        }
    }
}

/// Baseline after a successful save. The next preview alters this account
/// instead of creating it again, and an empty password field is not dirty.
/// The baseline is the snapshot captured when execution started.
fn adopted_account(running: AccountDraft, _later: AccountDraft) -> AccountDraft {
    running
}

fn saved_account(mut draft: AccountDraft) -> AccountDraft {
    let name = draft.name.trim().to_string();
    let host = draft.host.trim().to_string();
    draft.password.clear();
    draft.previous_name = Some(name);
    draft.previous_host = Some(host);
    draft.previous_plugin = Some(draft.plugin.clone());
    draft.previous_ssl = Some(draft.ssl_type.clone());
    draft.previous_cipher = Some(draft.ssl_cipher.clone());
    draft.previous_issuer = Some(draft.ssl_issuer.clone());
    draft.previous_subject = Some(draft.ssl_subject.clone());
    draft.previous_locked = Some(draft.locked);
    draft
}

fn postgres_profile(name: String, host: String, row: &[Option<String>]) -> ProfileFill {
    let cell = |index: usize| {
        row.get(index)
            .and_then(|value| value.clone())
            .unwrap_or_default()
    };
    let limit = cell(6);
    let limit = if limit.trim().is_empty() || limit.trim() == "-1" {
        String::new()
    } else {
        limit
    };
    let mut fill = ProfileFill::named(name, host);
    fill.postgres = Some(PostgresAttrs {
        login: pg_flag(&cell(0)),
        superuser: pg_flag(&cell(1)),
        createdb: pg_flag(&cell(2)),
        createrole: pg_flag(&cell(3)),
        inherit: pg_flag(&cell(4)),
        replication: pg_flag(&cell(5)),
        connection_limit: limit,
        valid_until: cell(7),
    });
    fill
}

fn pg_flag(value: &str) -> bool {
    matches!(value.trim(), "t" | "T" | "true" | "TRUE" | "1")
}

fn comparable(mut draft: AccountDraft) -> AccountDraft {
    draft.password.clear();
    draft.previous_name = None;
    draft.previous_host = None;
    draft.previous_plugin = None;
    draft.previous_ssl = None;
    draft.previous_cipher = None;
    draft.previous_issuer = None;
    draft.previous_subject = None;
    draft.previous_locked = None;
    draft
}

fn choice_text(choice: &Entity<SelectState<Vec<SharedString>>>, cx: &App) -> String {
    choice
        .read(cx)
        .selected_value()
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn select_value(
    choice: &Entity<SelectState<Vec<SharedString>>>,
    value: &str,
    presets: &[&str],
    window: &mut Window,
    cx: &mut Context<ToolPanel>,
) {
    let mut items: Vec<SharedString> = presets.iter().map(|item| (*item).into()).collect();
    if !value.is_empty() && !presets.contains(&value) {
        items.push(value.to_string().into());
    }
    let row = items.iter().position(|item| item.as_ref() == value);
    choice.update(cx, |state, cx| {
        state.set_items(items, window, cx);
        state.set_selected_index(row.map(|row| IndexPath::default().row(row)), window, cx);
    });
}

fn overlay_privs(set: &mut BTreeSet<String>, option: &mut bool, checks: &[bool], names: &[&str]) {
    for (on, name) in checks.iter().zip(names.iter()) {
        if *name == "GRANT OPTION" {
            *option = *on;
        } else if *on {
            set.insert((*name).to_string());
        } else {
            set.remove(*name);
        }
    }
}

fn expand_known(set: &mut BTreeSet<String>, names: &[&str]) {
    if !(set.contains("ALL") || set.contains("ALL PRIVILEGES")) {
        return;
    }
    set.remove("ALL");
    set.remove("ALL PRIVILEGES");
    for name in names {
        if *name != "GRANT OPTION" {
            set.insert((*name).to_string());
        }
    }
}

fn checks_from(set: &BTreeSet<String>, option: bool, names: &[&str]) -> Vec<bool> {
    names
        .iter()
        .map(|name| {
            if *name == "GRANT OPTION" {
                option
            } else {
                set.contains(*name)
            }
        })
        .collect()
}

fn object_row(schema: String, name: String, grants: &GrantSet) -> ObjectRow {
    let found = grants.objects.get(&(schema.clone(), name.clone()));
    ObjectRow {
        schema,
        name,
        on: checks_from(
            found.map(|grant| &grant.privileges).unwrap_or(&BTreeSet::new()),
            found.is_some_and(|grant| grant.grant_option),
            OBJECT_PRIVILEGES,
        ),
    }
}

fn roles_from(
    set: &ResultSet,
    grants: &GrantSet,
    self_id: &Option<(String, String)>,
) -> Vec<RoleRow> {
    let columns: Vec<String> = set.columns.iter().map(|column| column.name.clone()).collect();
    let mut rows = Vec::new();
    for raw in &set.rows {
        let cells: Vec<String> = raw.iter().map(|cell| cell.clone().unwrap_or_default()).collect();
        let (name, host) = row_identity(&columns, &cells);
        if name.is_empty() || self_id.as_ref().is_some_and(|(n, h)| n == &name && h == &host) {
            continue;
        }
        let key = (name.clone(), host.clone());
        rows.push(RoleRow {
            name,
            host,
            member_of: grants.roles.contains(&key),
            default_role: grants.defaults.contains(&key),
            member: false,
        });
    }
    for role in &grants.roles {
        if rows.iter().any(|row| row.name == role.0 && row.host == role.1) {
            continue;
        }
        rows.push(RoleRow {
            name: role.0.clone(),
            host: role.1.clone(),
            member_of: true,
            default_role: grants.defaults.contains(role),
            member: false,
        });
    }
    rows
}

fn grant_lines(set: &ResultSet) -> Vec<String> {
    set.rows
        .iter()
        .filter_map(|row| row.first().and_then(|cell| cell.clone()))
        .collect()
}

fn cell_at(row: &[Option<String>], index: usize) -> String {
    row.get(index)
        .and_then(|cell| cell.clone())
        .unwrap_or_default()
}

fn row_identity(columns: &[String], row: &[String]) -> (String, String) {
    let cell = |key: &str| {
        columns
            .iter()
            .position(|column| column == key)
            .and_then(|index| row.get(index))
            .map(|value| value.as_str())
            .unwrap_or("")
    };
    if columns.iter().any(|column| column == "user_name") {
        (cell("user_name").to_string(), cell("host").to_string())
    } else {
        (cell("name").to_string(), String::new())
    }
}

fn column_width(name: &str) -> tokens::Pt {
    if name == "name" {
        tokens::list::NAME_COLUMN_WIDTH
    } else {
        tokens::list::DETAIL_COLUMN_WIDTH
    }
}

fn column_label(name: &str) -> String {
    match name {
        "name" => t!("designer.user_col_name").to_string(),
        "user_name" => t!("designer.user_col_user").to_string(),
        "host" => t!("designer.user_col_host").to_string(),
        "plugin" => t!("designer.user_col_plugin").to_string(),
        "expire" => t!("designer.user_col_expire").to_string(),
        "max_queries" => t!("designer.user_col_queries").to_string(),
        "max_updates" => t!("designer.user_col_updates").to_string(),
        "max_connections" => t!("designer.user_col_connections").to_string(),
        "max_user_connections" => t!("designer.user_col_user_connections").to_string(),
        "kind" => t!("designer.user_col_kind").to_string(),
        "superuser" => t!("designer.tool_superuser").to_string(),
        "createdb" => t!("designer.tool_createdb").to_string(),
        "createrole" => t!("designer.tool_createrole").to_string(),
        "connection_limit" => t!("designer.tool_connection_limit").to_string(),
        other => other.to_string(),
    }
}

fn user_page_label(page: UserPage) -> String {
    match page {
        UserPage::General => t!("designer.user_general").to_string(),
        UserPage::Advanced => t!("designer.user_advanced").to_string(),
        UserPage::MemberOf => t!("designer.user_member_of").to_string(),
        UserPage::Members => t!("designer.user_members").to_string(),
        UserPage::Server => t!("designer.user_server").to_string(),
        UserPage::Objects => t!("designer.user_objects").to_string(),
        UserPage::Preview => t!("designer.user_preview").to_string(),
    }
}

fn priv_label(sql: &str) -> &'static str {
    match sql {
        "ALTER" => "Alter",
        "ALTER ROUTINE" => "Alter Routine",
        "CREATE" => "Create",
        "CREATE ROUTINE" => "Create Routine",
        "CREATE TEMPORARY TABLES" => "Create Temporary",
        "CREATE USER" => "Create User",
        "CREATE VIEW" => "Create View",
        "DELETE" => "Delete",
        "DROP" => "Drop",
        "EVENT" => "Event",
        "EXECUTE" => "Execute",
        "FILE" => "File",
        "GRANT OPTION" => "Grant Option",
        "INDEX" => "Index",
        "INSERT" => "Insert",
        "LOCK TABLES" => "Lock Tables",
        "PROCESS" => "Process",
        "REFERENCES" => "References",
        "RELOAD" => "Reload",
        "REPLICATION CLIENT" => "Replication Client",
        "REPLICATION SLAVE" => "Replication Slave",
        "SELECT" => "Select",
        "SHOW DATABASES" => "Show Databases",
        "SHOW VIEW" => "Show View",
        "SHUTDOWN" => "Shutdown",
        "SUPER" => "Super",
        "SYSTEM_USER" => "System User",
        "TRIGGER" => "Trigger",
        "UPDATE" => "Update",
        _ => "Privilege",
    }
}

fn object_head(id: &str, label: String) -> gpui_kit::Div {
    div()
        .w(if id == "name" || id == "schema" {
            tokens::list::NAME_COLUMN_WIDTH.rems()
        } else {
            tokens::list::DETAIL_COLUMN_WIDTH.rems()
        })
        .flex_none()
        .px_2()
        .min_w_0()
        .child(truncated(SharedString::from(format!("obj-th-{id}")), label))
}

fn object_cell(id: String, text: String) -> gpui_kit::Div {
    div()
        .w(tokens::list::NAME_COLUMN_WIDTH.rems())
        .flex_none()
        .px_2()
        .min_w_0()
        .child(truncated(SharedString::from(id), text))
}
