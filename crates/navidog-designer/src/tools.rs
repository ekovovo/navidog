//! Users, privileges, server monitor, and find. Statement text comes from
//! `navidog-sql`; this panel edits the draft and shows what the server returns.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::TestSupportExt;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dock::{
    BasePanel, Panel, PanelEvent, PanelId, PanelInfo, PanelState, TabGroup, panel_handle,
    register_panel,
};
use gpui_kit::component::input::{Editor, EditorState, Input, InputState};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    Disableable as _, IndexPath, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, KeyBinding, Render, SharedString, Task, WeakEntity, Window, div, prelude::*,
};
use navidog_app_state::actions::OpenTool;
use navidog_db::{DbError, DbResult, ResultSet};
use navidog_model::object::{ObjectKind, Scope};
use navidog_session::sessions::Sessions;
use navidog_sql::dialect::Dialect;
use navidog_sql::security::{
    self, AccountDraft, AccountForm, FindRequest, GrantDraft, GrantSet, MatchMode,
    OBJECT_PRIVILEGES, Probe, SERVER_PRIVILEGES,
};
use navidog_ui::assets::lucide;
use navidog_ui::skin::{FormRow, truncated};
use navidog_ui::{palette, tabs, tokens};
use rust_i18n::t;
use serde::{Deserialize, Serialize};

use crate::panel::{self, Save};

const PANEL: &str = "ToolPanel";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Tool {
    Users,
    Privileges,
    Monitor,
    Find,
}

impl Tool {
    fn parse(key: &str) -> Option<Self> {
        match key {
            "users" => Some(Self::Users),
            "privileges" => Some(Self::Privileges),
            "monitor" => Some(Self::Monitor),
            "find" => Some(Self::Find),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Users => "users",
            Self::Privileges => "privileges",
            Self::Monitor => "monitor",
            Self::Find => "find",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MonitorPage {
    Processes,
    Variables,
    Status,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UserPage {
    General,
    Advanced,
    MemberOf,
    Members,
    Server,
    Objects,
    Preview,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FetchKind {
    Output,
    Accounts,
    Detail,
    Defaults,
    Members,
}

#[derive(Clone)]
struct RoleRow {
    name: String,
    host: String,
    member_of: bool,
    default_role: bool,
    member: bool,
}

#[derive(Clone)]
struct ObjectRow {
    schema: String,
    name: String,
    on: Vec<bool>,
}

struct PostgresAttrs {
    login: bool,
    superuser: bool,
    createdb: bool,
    createrole: bool,
    inherit: bool,
    replication: bool,
    connection_limit: String,
    valid_until: String,
}

struct ProfileFill {
    name: String,
    host: String,
    plugin: String,
    locked: bool,
    max_queries: String,
    max_updates: String,
    max_connections: String,
    max_user_connections: String,
    ssl_type: String,
    ssl_cipher: String,
    ssl_issuer: String,
    ssl_subject: String,
    postgres: Option<PostgresAttrs>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    scope: Scope,
    tool: String,
    target: Option<String>,
}

/// The account state that the current preview will execute.
#[derive(Clone)]
struct AccountPreview {
    account: AccountDraft,
    grants: GrantSet,
    members: BTreeSet<(String, String)>,
    password: String,
}

pub struct ToolPanel {
    scope: Scope,
    tool: Tool,
    target: Option<String>,
    dialect: Arc<dyn Dialect>,
    focus: FocusHandle,
    busy: bool,
    close_approved: bool,
    error: Option<String>,
    output: String,
    preview: Vec<String>,
    /// Draft and grants that produced `preview`. Success adopts this, not the form.
    account_preview: Option<AccountPreview>,
    /// Copy of `account_preview` taken when execution starts. A later preview must not replace it.
    running_account: Option<AccountPreview>,
    name: Entity<InputState>,
    host: Entity<InputState>,
    password: Entity<InputState>,
    role: bool,
    locked: bool,
    login: bool,
    superuser: bool,
    createdb: bool,
    createrole: bool,
    inherit: bool,
    replication: bool,
    expire: Entity<InputState>,
    max_queries: Entity<InputState>,
    max_updates: Entity<InputState>,
    max_connections: Entity<InputState>,
    max_user_connections: Entity<InputState>,
    connection_limit: Entity<InputState>,
    valid_until: Entity<InputState>,
    privilege: Entity<InputState>,
    object_name: Entity<InputState>,
    grantee: Entity<InputState>,
    grant_option: bool,
    auto: bool,
    monitor: MonitorPage,
    interval: Entity<InputState>,
    kill_id: Entity<InputState>,
    var_name: Entity<InputState>,
    var_value: Entity<InputState>,
    needle: Entity<InputState>,
    mode: MatchMode,
    case_sensitive: bool,
    search_data: bool,
    kinds: Vec<(ObjectKind, bool)>,
    task: Option<Task<()>>,
    watch: Option<Task<()>>,
    user_page: UserPage,
    plugin_choice: Entity<SelectState<Vec<SharedString>>>,
    ssl_choice: Entity<SelectState<Vec<SharedString>>>,
    confirm: Entity<InputState>,
    ssl_cipher: Entity<InputState>,
    ssl_issuer: Entity<InputState>,
    ssl_subject: Entity<InputState>,
    user_filter: Entity<InputState>,
    sql_view: Entity<EditorState>,
    account_columns: Vec<String>,
    account_rows: Vec<Vec<String>>,
    selected_account: Option<usize>,
    server_on: Vec<bool>,
    roles: Vec<RoleRow>,
    objects: Vec<ObjectRow>,
    loaded_draft: AccountDraft,
    loaded_grants: GrantSet,
    loaded_members: BTreeSet<(String, String)>,
    grants_touched: bool,
    expect_tables: bool,
    fetch_kind: FetchKind,
    fetch_gen: u64,
    pending_fill: Option<ProfileFill>,
    clear_password: bool,
}

impl ToolPanel {
    pub fn open(
        scope: Scope,
        tool: &str,
        target: Option<String>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Entity<Self>> {
        let tool = Tool::parse(tool)?;
        let dialect = Sessions::global(cx).read(cx).dialect(&scope.connection)?;
        let panel = cx.new(|cx| Self::new(scope, tool, target, dialect, window, cx));
        panel.update(cx, |this, cx| this.start(cx));
        Some(panel)
    }

    fn new(
        scope: Scope,
        tool: Tool,
        target: Option<String>,
        dialect: Arc<dyn Dialect>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let kinds = dialect
            .object_kinds()
            .iter()
            .copied()
            .filter(|kind| *kind != ObjectKind::Query)
            .map(|kind| (kind, true))
            .collect();
        let input = |value: &str, masked: bool, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .masked(masked)
            })
        };
        let choice = |items: &[&str], window: &mut Window, cx: &mut Context<Self>| {
            let items: Vec<SharedString> = items.iter().map(|item| (*item).into()).collect();
            cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(0)), window, cx))
        };
        let sql_view = cx.new(|cx| EditorState::new(window, cx).language("sql"));
        sql_view.update(cx, |state, cx| state.set_readonly(true, cx));
        let user_filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("designer.tool_filter").to_string())
        });
        Self {
            scope,
            tool,
            target,
            dialect,
            focus: cx.focus_handle(),
            busy: false,
            close_approved: false,
            error: None,
            output: String::new(),
            preview: Vec::new(),
            account_preview: None,
            running_account: None,
            name: input("", false, window, cx),
            host: input("%", false, window, cx),
            password: input("", true, window, cx),
            monitor: MonitorPage::Processes,
            role: false,
            locked: false,
            login: true,
            superuser: false,
            createdb: false,
            createrole: false,
            inherit: true,
            replication: false,
            expire: input("", false, window, cx),
            max_queries: input("0", false, window, cx),
            max_updates: input("0", false, window, cx),
            max_connections: input("0", false, window, cx),
            max_user_connections: input("0", false, window, cx),
            connection_limit: input("", false, window, cx),
            valid_until: input("", false, window, cx),
            privilege: input("SELECT", false, window, cx),
            object_name: input("", false, window, cx),
            grantee: input("", false, window, cx),
            grant_option: false,
            auto: false,
            interval: input("5", false, window, cx),
            kill_id: input("", false, window, cx),
            var_name: input("", false, window, cx),
            var_value: input("", false, window, cx),
            needle: input("", false, window, cx),
            mode: MatchMode::Contains,
            case_sensitive: false,
            search_data: false,
            kinds,
            task: None,
            watch: None,
            user_page: UserPage::General,
            plugin_choice: choice(
                &[
                    "",
                    "caching_sha2_password",
                    "mysql_native_password",
                    "sha256_password",
                ],
                window,
                cx,
            ),
            ssl_choice: choice(&["", "NONE", "ANY", "X509", "SPECIFIED"], window, cx),
            confirm: input("", true, window, cx),
            ssl_cipher: input("", false, window, cx),
            ssl_issuer: input("", false, window, cx),
            ssl_subject: input("", false, window, cx),
            user_filter,
            sql_view,
            account_columns: Vec::new(),
            account_rows: Vec::new(),
            selected_account: None,
            server_on: vec![false; SERVER_PRIVILEGES.len()],
            roles: Vec::new(),
            objects: Vec::new(),
            loaded_draft: AccountDraft::default(),
            loaded_grants: GrantSet::default(),
            loaded_members: BTreeSet::new(),
            grants_touched: false,
            expect_tables: false,
            fetch_kind: FetchKind::Output,
            fetch_gen: 0,
            pending_fill: None,
            clear_password: false,
        }
    }

    fn text(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().to_string()
    }

    fn account_draft(&self, cx: &App) -> AccountDraft {
        AccountDraft {
            name: Self::text(&self.name, cx),
            host: Self::text(&self.host, cx),
            password: Self::text(&self.password, cx),
            role: self.role,
            locked: self.locked,
            expire: Self::text(&self.expire, cx),
            max_queries: Self::text(&self.max_queries, cx),
            max_updates: Self::text(&self.max_updates, cx),
            max_connections: Self::text(&self.max_connections, cx),
            max_user_connections: Self::text(&self.max_user_connections, cx),
            login: self.login,
            superuser: self.superuser,
            createdb: self.createdb,
            createrole: self.createrole,
            inherit: self.inherit,
            replication: self.replication,
            connection_limit: Self::text(&self.connection_limit, cx),
            valid_until: Self::text(&self.valid_until, cx),
            plugin: choice_text(&self.plugin_choice, cx),
            ssl_type: choice_text(&self.ssl_choice, cx),
            ssl_cipher: Self::text(&self.ssl_cipher, cx),
            ssl_issuer: Self::text(&self.ssl_issuer, cx),
            ssl_subject: Self::text(&self.ssl_subject, cx),
            ..AccountDraft::default()
        }
    }

    fn show_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if matches!(self.dialect.account_form(), AccountForm::Unavailable(_)) {
            self.preview.clear();
            self.error = Some(security::SQLITE_AUTH_REASON.into());
            return;
        }
        let password = Self::text(&self.password, cx);
        let confirm = Self::text(&self.confirm, cx);
        if !confirm.is_empty() && confirm != password {
            self.preview.clear();
            self.account_preview = None;
            self.error = Some(t!("designer.tool_password_mismatch").to_string());
            cx.notify();
            return;
        }
        let mut draft = self.account_draft(cx);
        draft.previous_name = self.loaded_draft.previous_name.clone();
        draft.previous_host = self.loaded_draft.previous_host.clone();
        draft.previous_plugin = self.loaded_draft.previous_plugin.clone();
        draft.previous_ssl = self.loaded_draft.previous_ssl.clone();
        draft.previous_cipher = self.loaded_draft.previous_cipher.clone();
        draft.previous_issuer = self.loaded_draft.previous_issuer.clone();
        draft.previous_subject = self.loaded_draft.previous_subject.clone();
        draft.previous_locked = self.loaded_draft.previous_locked;
        let (grants, members) = self.current_grants();
        let privileges = if self.dialect.account_form() == AccountForm::Mysql {
            security::privilege_statements(
                self.dialect.as_ref(),
                self.dialect.definition_family(),
                security::PrivilegeEdit {
                    grantee: draft.name.trim(),
                    host: draft.host.trim(),
                    before: &self.loaded_grants,
                    after: &grants,
                    before_members: &self.loaded_members,
                    after_members: &members,
                },
            )
        } else {
            Ok(Vec::new())
        };
        match security::account_statements(
            self.dialect.as_ref(),
            self.dialect.definition_family(),
            &draft,
        )
        .and_then(|mut sql| {
            sql.extend(privileges?);
            Ok(sql)
        }) {
            Ok(sql) => {
                let literal = (!password.trim().is_empty())
                    .then(|| self.dialect.string_literal(password.trim()));
                // The preview keeps the real statement. The pane must not show the password.
                let redacted = redact_password_literal(&sql.join(";\n"), literal.as_deref());
                self.sql_view.update(cx, |state, cx| {
                    state.set_value(redacted, window, cx);
                });
                self.output.clear();
                self.preview = sql;
                self.account_preview = Some(AccountPreview {
                    account: saved_account(draft),
                    grants,
                    members,
                    password: password.trim().to_string(),
                });
                self.error = None;
            }
            Err(error) => {
                self.preview.clear();
                self.account_preview = None;
                self.error = Some(error);
            }
        }
        cx.notify();
    }

    fn show_grant(&mut self, revoke: bool, cx: &mut Context<Self>) {
        if !self.dialect.manages_privileges() {
            self.preview.clear();
            self.error = Some(t!("designer.tool_no_privileges").to_string());
            return;
        }
        let draft = GrantDraft {
            privilege: Self::text(&self.privilege, cx),
            object: Self::text(&self.object_name, cx),
            grantee: Self::text(&self.grantee, cx),
            host: Self::text(&self.host, cx),
            grant_option: self.grant_option,
            revoke,
        };
        match security::grant_statements(
            self.dialect.as_ref(),
            self.dialect.definition_family(),
            &draft,
        ) {
            Ok(sql) => {
                self.output = sql.join(";\n");
                self.preview = sql;
                self.error = None;
            }
            Err(error) => {
                self.preview.clear();
                self.error = Some(error);
            }
        }
        cx.notify();
    }

    fn execute_preview(&mut self, cx: &mut Context<Self>) {
        if self.preview.is_empty() || self.busy {
            return;
        }
        let sql = self.preview.clone();
        self.running_account = self.account_preview.clone();
        self.run(sql, false, cx);
    }

    fn run(&mut self, sqls: Vec<String>, read: bool, cx: &mut Context<Self>) {
        self.spawn_query(sqls, read, FetchKind::Output, cx);
    }

    fn spawn_query(
        &mut self,
        sqls: Vec<String>,
        read: bool,
        kind: FetchKind,
        cx: &mut Context<Self>,
    ) {
        if sqls.is_empty() {
            return;
        }
        let database = if self.scope.database.is_empty() {
            None
        } else {
            Some(self.scope.database.clone())
        };
        let Some(job) = Sessions::global(cx)
            .read(cx)
            .new_session(&self.scope.connection, database.as_deref())
        else {
            self.error = Some(t!("designer.tool_not_connected").to_string());
            return;
        };
        self.busy = true;
        self.fetch_kind = kind;
        self.fetch_gen += 1;
        let generation = self.fetch_gen;
        if !matches!(kind, FetchKind::Defaults | FetchKind::Members) {
            self.error = None;
        }
        self.task = Some(cx.spawn(async move |this, cx| {
            let result: DbResult<Vec<ResultSet>> = async {
                let mut session = job.await.map_err(|e| DbError::msg(e.to_string()))??;
                let job = navidog_db::runtime::spawn(async move {
                    let result = async {
                        let r = session.relational().ok_or_else(|| {
                            DbError::msg("This connection is not a relational database.")
                        })?;
                        if let Some(database) = database.as_deref()
                            && let Some(sql) = r.dialect().prepare_database_sql(database)
                        {
                            r.execute(&sql).await?;
                        }
                        let mut sets = Vec::new();
                        for sql in &sqls {
                            let mut got = if read {
                                r.execute_read(sql).await?
                            } else {
                                r.execute(sql).await?
                            };
                            sets.append(&mut got);
                        }
                        Ok(sets)
                    }
                    .await;
                    let _ = session.close().await;
                    result
                });
                job.await.map_err(|e| DbError::msg(e.to_string()))?
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                if this.fetch_gen != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(sets) => this.apply_fetch(sets, cx),
                    Err(error) => {
                        if !matches!(this.fetch_kind, FetchKind::Defaults | FetchKind::Members) {
                            this.error = Some(error.to_string());
                        }
                        if this.fetch_kind == FetchKind::Output {
                            this.running_account = None;
                        }
                        this.finish_optional_fetch(cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn list_accounts(&mut self, cx: &mut Context<Self>) {
        let Some(sql) = self.dialect.account_list_sql() else {
            self.error = Some(security::SQLITE_AUTH_REASON.into());
            cx.notify();
            return;
        };
        self.spawn_query(vec![sql.into()], true, FetchKind::Accounts, cx);
    }

    fn load_monitor(&mut self, page: MonitorPage, cx: &mut Context<Self>) {
        let Some(sql) = self.monitor_sql(page) else {
            self.error = Some(t!("designer.tool_no_monitor").to_string());
            cx.notify();
            return;
        };
        self.monitor = page;
        self.run(vec![sql], true, cx);
    }

    fn monitor_sql(&self, page: MonitorPage) -> Option<String> {
        match page {
            MonitorPage::Processes => self.dialect.process_sql().map(str::to_string),
            MonitorPage::Variables => self.dialect.variables_sql().map(str::to_string),
            MonitorPage::Status => self.dialect.status_sql().map(str::to_string),
        }
    }

    fn kill(&mut self, cx: &mut Context<Self>) {
        match self.dialect.kill_sql(&Self::text(&self.kill_id, cx)) {
            Ok(Some(sql)) => self.run(vec![sql], false, cx),
            Ok(None) => {
                self.error = Some(t!("designer.tool_no_monitor").to_string());
                cx.notify();
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn set_variable(&mut self, cx: &mut Context<Self>) {
        match security::set_variable_sql(
            self.dialect.as_ref(),
            self.dialect.definition_family(),
            &Self::text(&self.var_name, cx),
            &Self::text(&self.var_value, cx),
        ) {
            Ok(sql) if !sql.is_empty() => self.run(sql, false, cx),
            Ok(_) => {
                self.error = Some(t!("designer.tool_no_monitor").to_string());
                cx.notify();
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn toggle_auto(&mut self, on: bool, cx: &mut Context<Self>) {
        self.auto = on;
        self.watch.take();
        if !on || !self.dialect.monitors_server() {
            return;
        }
        let secs = Self::text(&self.interval, cx)
            .trim()
            .parse::<u64>()
            .unwrap_or(5)
            .clamp(1, 3600);
        self.watch = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(secs))
                    .await;
                let go = this
                    .update(cx, |this, cx| {
                        if !this.auto {
                            return false;
                        }
                        if let Some(sql) = this.monitor_sql(this.monitor) {
                            this.run(vec![sql], true, cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if !go {
                    break;
                }
            }
        }));
    }

    fn find(&mut self, cx: &mut Context<Self>) {
        let kinds: Vec<ObjectKind> = self
            .kinds
            .iter()
            .filter(|(_, on)| *on)
            .map(|(kind, _)| *kind)
            .collect();
        let needle = Self::text(&self.needle, cx);
        let plan = match security::find_plan(
            self.dialect.as_ref(),
            self.dialect.definition_family(),
            &self.scope,
            FindRequest {
                needle: &needle,
                mode: self.mode,
                case_sensitive: self.case_sensitive,
                kinds: &kinds,
                data: self.search_data,
            },
        ) {
            Ok(plan) => plan,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let database = if self.scope.database.is_empty() {
            None
        } else {
            Some(self.scope.database.clone())
        };
        let Some(job) = Sessions::global(cx)
            .read(cx)
            .new_session(&self.scope.connection, database.as_deref())
        else {
            self.error = Some(t!("designer.tool_not_connected").to_string());
            cx.notify();
            return;
        };
        self.busy = true;
        self.error = None;
        let scope = self.scope.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result: DbResult<String> = async {
                let mut session = job.await.map_err(|e| DbError::msg(e.to_string()))??;
                let job = navidog_db::runtime::spawn(async move {
                    let result = async {
                        let r = session.relational().ok_or_else(|| {
                            DbError::msg("This connection is not a relational database.")
                        })?;
                        if let Some(database) = database.as_deref()
                            && let Some(sql) = r.dialect().prepare_database_sql(database)
                        {
                            r.execute(&sql).await?;
                        }
                        let mut out = String::new();
                        for sql in &plan.structure {
                            out.push_str(&format_sets(r.execute_read(sql).await?));
                        }
                        if let Some(columns_sql) = &plan.text_columns {
                            let sets = r.execute_read(columns_sql).await?;
                            let mut by_table: BTreeMap<String, Vec<String>> = BTreeMap::new();
                            for set in sets {
                                for row in set.rows {
                                    let table = row.first().cloned().flatten().unwrap_or_default();
                                    let column = row.get(1).cloned().flatten().unwrap_or_default();
                                    if !table.is_empty() && !column.is_empty() {
                                        by_table.entry(table).or_default().push(column);
                                    }
                                }
                            }
                            let dialect = r.dialect();
                            for (table, columns) in by_table {
                                match security::data_probe(
                                    dialect.as_ref(),
                                    dialect.definition_family(),
                                    &scope,
                                    Probe {
                                        table: &table,
                                        columns: &columns,
                                        needle: &plan.needle,
                                        mode: plan.mode,
                                        case_sensitive: plan.case_sensitive,
                                    },
                                ) {
                                    Ok(sql) => match r.execute_read(&sql).await {
                                        Ok(sets) => {
                                            let body = format_sets(sets);
                                            if !body.trim().is_empty() {
                                                out.push_str(&table);
                                                out.push('\n');
                                                out.push_str(&body);
                                            }
                                        }
                                        Err(error) => {
                                            out.push_str(&table);
                                            out.push('\n');
                                            out.push_str(&error.to_string());
                                            out.push('\n');
                                        }
                                    },
                                    Err(error) => {
                                        out.push_str(&error);
                                        out.push('\n');
                                    }
                                }
                            }
                        }
                        Ok(out)
                    }
                    .await;
                    let _ = session.close().await;
                    result
                });
                job.await.map_err(|e| DbError::msg(e.to_string()))?
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(text) => {
                        this.output = if text.trim().is_empty() {
                            t!("designer.tool_no_hits").to_string()
                        } else {
                            text
                        };
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn is_dirty(&self, cx: &App) -> bool {
        if self.tool == Tool::Users {
            return self.user_dirty(cx);
        }
        !self.preview.is_empty()
    }

    fn ask_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.is_dirty(cx) {
            return;
        }
        let weak = cx.entity().downgrade();
        panel::confirm_discard(
            PanelId::from(cx.entity_id()),
            t!("designer.tool_discard").to_string(),
            t!("designer.tool_discard_body").to_string(),
            window,
            cx,
            move |window, cx| {
                if let Ok(id) = weak.update(cx, |this, cx| {
                    this.close_approved = true;
                    PanelId::from(cx.entity_id())
                }) {
                    window.dispatch_action(
                        Box::new(tabs::CloseTabs {
                            panel: Some(id),
                            which: tabs::Close::This,
                        }),
                        cx,
                    );
                }
            },
        );
    }
}

impl EventEmitter<PanelEvent> for ToolPanel {}

impl Focusable for ToolPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BasePanel for ToolPanel {
    fn panel_name(&self) -> &'static str {
        PANEL
    }
    fn closable(&self, cx: &App) -> bool {
        !self.busy && (self.close_approved || !self.is_dirty(cx))
    }
    fn dump(&self, _: &App) -> PanelState {
        PanelState {
            panel_name: PANEL.into(),
            children: Vec::new(),
            info: PanelInfo::panel(
                serde_json::to_value(Saved {
                    scope: self.scope.clone(),
                    tool: self.tool.key().into(),
                    target: self.target.clone(),
                })
                .unwrap_or_default(),
            ),
        }
    }
    fn on_added_to(
        &mut self,
        group: WeakEntity<TabGroup>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = PanelId::from(cx.entity_id());
        tabs::added(id, group, cx);
        let weak = cx.entity().downgrade();
        let home = window.window_handle();
        tabs::set_close_guard(
            id,
            Rc::new(move |cx| {
                let weak = weak.clone();
                let _ = home.update(cx, |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| this.ask_close(window, cx));
                });
            }),
            cx,
        );
    }
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        tabs::set_active(PanelId::from(cx.entity_id()), active, cx);
    }
    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        // Moving the tab to another window reports removed as well. Keep the refresh running.
        if tabs::removed(PanelId::from(cx.entity_id()), cx) {
            return;
        }
        self.auto = false;
        self.watch.take();
        self.task.take();
    }
}

impl Panel for ToolPanel {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = if self.tool == Tool::Users {
            self.user_tab_title(cx)
        } else {
            t!(format!("designer.tool_{}", self.tool.key())).to_string()
        };
        tabs::title(
            PanelId::from(cx.entity_id()),
            Some(lucide(match self.tool {
                Tool::Users => "user",
                Tool::Privileges => "key",
                Tool::Monitor => "server",
                Tool::Find => "search",
            })),
            label,
            None,
            self.is_dirty(cx),
        )
    }
}

impl Render for ToolPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(fill) = self.pending_fill.take() {
            self.apply_fill(fill, window, cx);
        }
        if self.clear_password {
            self.clear_password = false;
            self.password
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
            self.confirm
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        }
        let p = palette(cx);
        let disabled = self.busy;
        let users = self.tool == Tool::Users;
        let body = match self.tool {
            Tool::Users => self.render_users(disabled, window, cx),
            Tool::Privileges => self.render_privileges(disabled, cx),
            Tool::Monitor => self.render_monitor(disabled, cx),
            Tool::Find => self.render_find(disabled, cx),
        };
        v_flex()
            .id("tool-panel")
            .key_context(PANEL)
            .track_focus(&self.focus)
            .size_full()
            .bg(p.window_bg)
            .on_action(cx.listener(|this, _: &Save, window, cx| match this.tool {
                Tool::Users => this.show_account(window, cx),
                Tool::Privileges => this.show_grant(false, cx),
                _ => {}
            }))
            .when(!users || !self.preview.is_empty(), |col| {
                col.child(
                    h_flex()
                        .min_h(tokens::object_toolbar::HEIGHT.rems())
                        .px_2()
                        .gap_1()
                        .border_b_1()
                        .border_color(p.separator)
                        .when(!self.preview.is_empty(), |bar| {
                            bar.child(
                                Button::new("tool-execute")
                                    .small()
                                    .icon(lucide("play"))
                                    .label(t!("designer.execute").to_string())
                                    .disabled(disabled)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.execute_preview(cx)),
                                    ),
                            )
                        }),
                )
            })
            .when_some(self.error.clone(), |col, error| {
                col.child(div().px_2().text_color(p.danger).child(error))
            })
            .child(body)
            .when(!users, |col| col.child(self.render_output()))
            .when(users && !self.output.is_empty(), |col| {
                col.child(
                    div()
                        .px_2()
                        .text_color(p.label_secondary)
                        .child(self.output.clone()),
                )
            })
    }
}

impl ToolPanel {
    fn render_output(&self) -> gpui_kit::AnyElement {
        v_flex()
            .id("tool-output")
            .flex_1()
            .min_h_0()
            .p_2()
            .gap_1()
            .overflow_y_scroll()
            .children(
                self.output
                    .lines()
                    .map(|line| div().child(line.to_string())),
            )
            .into_any_element()
    }

    fn render_privileges(&self, disabled: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if !self.dialect.manages_privileges() {
            return v_flex()
                .id("tool-privileges")
                .p_3()
                .child(t!("designer.tool_no_privileges").to_string())
                .into_any_element();
        }
        let mysql = self.dialect.account_form() == AccountForm::Mysql;
        v_flex()
            .id("tool-privileges")
            .p_3()
            .gap_2()
            .child(FormRow::new(
                t!("designer.tool_privilege").to_string(),
                Input::new(&self.privilege)
                    .id("tool-privilege")
                    .small()
                    .disabled(disabled),
            ))
            .child(FormRow::new(
                t!("designer.tool_object").to_string(),
                Input::new(&self.object_name)
                    .id("tool-object")
                    .small()
                    .disabled(disabled),
            ))
            .child(FormRow::new(
                t!("designer.tool_grantee").to_string(),
                Input::new(&self.grantee)
                    .id("tool-grantee")
                    .small()
                    .disabled(disabled),
            ))
            .when(mysql, |col| {
                col.child(FormRow::new(
                    t!("designer.tool_host").to_string(),
                    Input::new(&self.host)
                        .id("tool-grant-host")
                        .small()
                        .disabled(disabled),
                ))
            })
            .child(check(
                "tool-grant-option",
                "tool_grant_option",
                self.grant_option,
                disabled,
                cx,
                |this, on| this.grant_option = on,
            ))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("tool-grant")
                            .small()
                            .label(t!("designer.tool_grant").to_string())
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, _, cx| this.show_grant(false, cx))),
                    )
                    .child(
                        Button::new("tool-revoke")
                            .small()
                            .label(t!("designer.tool_revoke").to_string())
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, _, cx| this.show_grant(true, cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_monitor(&self, disabled: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if !self.dialect.monitors_server() {
            return v_flex()
                .id("tool-monitor")
                .p_3()
                .child(t!("designer.tool_no_monitor").to_string())
                .into_any_element();
        }
        let writable = self.dialect.definition_family()
            == Some(navidog_sql::definition::DefinitionFamily::Mysql);
        v_flex()
            .id("tool-monitor")
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("tool-processes")
                            .small()
                            .label(t!("designer.tool_processes").to_string())
                            .disabled(disabled)
                            .when(self.monitor == MonitorPage::Processes, |b| b.selected(true))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load_monitor(MonitorPage::Processes, cx);
                            })),
                    )
                    .child(
                        Button::new("tool-variables")
                            .small()
                            .label(t!("designer.tool_variables").to_string())
                            .disabled(disabled)
                            .when(self.monitor == MonitorPage::Variables, |b| b.selected(true))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load_monitor(MonitorPage::Variables, cx);
                            })),
                    )
                    .child(
                        Button::new("tool-status")
                            .small()
                            .label(t!("designer.tool_status").to_string())
                            .disabled(disabled)
                            .when(self.monitor == MonitorPage::Status, |b| b.selected(true))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load_monitor(MonitorPage::Status, cx);
                            })),
                    ),
            )
            .child(
                Checkbox::new("tool-auto")
                    .label(t!("designer.tool_auto").to_string())
                    .checked(self.auto)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, value, _, cx| this.toggle_auto(*value, cx))),
            )
            .child(FormRow::new(
                t!("designer.tool_interval").to_string(),
                Input::new(&self.interval)
                    .id("tool-interval")
                    .small()
                    .disabled(disabled),
            ))
            .child(FormRow::new(
                t!("designer.tool_process_id").to_string(),
                Input::new(&self.kill_id)
                    .id("tool-kill-id")
                    .small()
                    .disabled(disabled),
            ))
            .child(
                Button::new("tool-kill")
                    .small()
                    .label(t!("designer.tool_kill").to_string())
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| this.kill(cx))),
            )
            .when(writable, |page| {
                page.child(FormRow::new(
                    t!("designer.tool_variable").to_string(),
                    Input::new(&self.var_name)
                        .id("tool-var-name")
                        .small()
                        .disabled(disabled),
                ))
                .child(FormRow::new(
                    t!("designer.tool_value").to_string(),
                    Input::new(&self.var_value)
                        .id("tool-var-value")
                        .small()
                        .disabled(disabled),
                ))
                .child(
                    Button::new("tool-set-var")
                        .small()
                        .label(t!("designer.tool_set").to_string())
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, _, cx| this.set_variable(cx))),
                )
            })
            .into_any_element()
    }

    fn render_find(&self, disabled: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        v_flex()
            .id("tool-find")
            .p_3()
            .gap_2()
            .child(FormRow::new(
                t!("designer.tool_find_text").to_string(),
                Input::new(&self.needle)
                    .id("tool-needle")
                    .small()
                    .disabled(disabled),
            ))
            .child(
                h_flex()
                    .gap_1()
                    .child(mode_button("contains", MatchMode::Contains, self.mode, cx))
                    .child(mode_button("word", MatchMode::WholeWord, self.mode, cx))
                    .child(mode_button("prefix", MatchMode::Prefix, self.mode, cx))
                    .child(mode_button("regex", MatchMode::Regex, self.mode, cx)),
            )
            .child(check(
                "tool-case",
                "tool_case",
                self.case_sensitive,
                disabled,
                cx,
                |this, on| this.case_sensitive = on,
            ))
            .child(check(
                "tool-data",
                "tool_data",
                self.search_data,
                disabled,
                cx,
                |this, on| this.search_data = on,
            ))
            .child(h_flex().gap_1().children(self.kinds.iter().enumerate().map(
                |(ix, (kind, on))| {
                    let kind_key = kind.key();
                    Checkbox::new(SharedString::from(format!("tool-kind-{kind_key}")))
                        .label(t!(format!("designer.kind_{kind_key}")).to_string())
                        .checked(*on)
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, value, _, cx| {
                            if let Some(item) = this.kinds.get_mut(ix) {
                                item.1 = *value;
                            }
                            cx.notify();
                        }))
                },
            )))
            .child(
                Button::new("tool-find-run")
                    .small()
                    .icon(lucide("search"))
                    .label(t!("designer.tool_find").to_string())
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| this.find(cx))),
            )
            .into_any_element()
    }
}

fn mode_button(
    key: &'static str,
    mode: MatchMode,
    current: MatchMode,
    cx: &mut Context<ToolPanel>,
) -> Button {
    Button::new(SharedString::from(format!("tool-mode-{key}")))
        .small()
        .selected(current == mode)
        .label(t!(format!("designer.tool_mode_{key}")).to_string())
        .on_click(cx.listener(move |this, _, _, cx| {
            this.mode = mode;
            cx.notify();
        }))
}

fn check(
    id: &'static str,
    label: &'static str,
    on: bool,
    disabled: bool,
    cx: &mut Context<ToolPanel>,
    set: impl Fn(&mut ToolPanel, bool) + 'static,
) -> gpui_kit::AnyElement {
    Checkbox::new(id)
        .label(t!(format!("designer.{label}")).to_string())
        .checked(on)
        .disabled(disabled)
        .on_click(cx.listener(move |this, value, _, cx| {
            set(this, *value);
            cx.notify();
        }))
        .into_any_element()
}

fn format_sets(sets: Vec<ResultSet>) -> String {
    let mut out = String::new();
    if sets.is_empty() {
        out.push_str("(ok)\n");
        return out;
    }
    for set in sets {
        if set.rows.is_empty() {
            if let Some(rows) = set.affected_rows {
                out.push_str(&format!("({rows})\n"));
            }
            continue;
        }
        for row in set.rows {
            for (i, cell) in row.iter().enumerate() {
                if i > 0 {
                    out.push('\t');
                }
                out.push_str(cell.as_deref().unwrap_or("NULL"));
            }
            out.push('\n');
        }
    }
    out
}

/// Hides a password literal in the text the user sees. `IDENTIFIED BY` and `PASSWORD`
/// are the only clauses that carry it; a matching literal used as a name or a date stays.
fn redact_password_literal(sql: &str, literal: Option<&str>) -> String {
    let Some(literal) = literal.filter(|literal| !literal.is_empty()) else {
        return sql.to_string();
    };
    const MARKS: &[&str] = &["IDENTIFIED BY ", "PASSWORD "];
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(index) = rest.find(literal) {
        let (head, tail) = rest.split_at(index);
        out.push_str(head);
        if MARKS.iter().any(|mark| head.ends_with(mark)) {
            out.push_str("'***'");
        } else {
            out.push_str(literal);
        }
        rest = &tail[literal.len()..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", Save, Some(PANEL)),
        KeyBinding::new("cmd-s", Save, Some("ToolPanel > Input")),
    ]);
    register_panel(cx, PANEL, |ctx, window, cx| {
        let saved = match ctx.info() {
            PanelInfo::Panel(value) => serde_json::from_value::<Saved>(value.clone()).ok(),
            _ => None,
        };
        let Some(saved) = saved else {
            return panel_handle(panel::MissingConnection::open(ctx.state().clone(), cx));
        };
        match ToolPanel::open(saved.scope, &saved.tool, saved.target, window, cx) {
            Some(panel) => panel_handle(panel),
            None => panel_handle(panel::MissingConnection::open(ctx.state().clone(), cx)),
        }
    });
}

include!("user_panel.rs");

#[cfg(test)]
mod tests {
    use super::redact_password_literal;

    #[test]
    fn account_preview_hides_the_password_and_leaves_the_same_text_elsewhere() {
        let sql = "CREATE USER `ada`@`%` IDENTIFIED BY 's''ecret' PASSWORD EXPIRE;\n\
                   ALTER ROLE \"ada\" PASSWORD 's''ecret' VALID UNTIL 's''ecret'";
        let shown = redact_password_literal(sql, Some("'s''ecret'"));
        assert!(shown.contains("IDENTIFIED BY '***'"), "{shown}");
        assert!(shown.contains("PASSWORD '***'"), "{shown}");
        assert!(shown.contains("VALID UNTIL 's''ecret'"), "{shown}");
        assert!(!shown.contains("IDENTIFIED BY 's''ecret'"), "{shown}");
    }

    #[test]
    fn a_saved_account_is_altered_instead_of_created_again() {
        let draft = navidog_sql::security::AccountDraft {
            name: " ada ".into(),
            host: "%".into(),
            password: "secret".into(),
            plugin: "caching_sha2_password".into(),
            ..navidog_sql::security::AccountDraft::default()
        };
        let saved = super::saved_account(draft);
        assert!(saved.password.is_empty());
        assert_eq!(saved.previous_name.as_deref(), Some("ada"));
        assert_eq!(saved.previous_host.as_deref(), Some("%"));
        assert_eq!(
            saved.previous_plugin.as_deref(),
            Some("caching_sha2_password")
        );
    }

    #[test]
    fn a_later_edit_stays_dirty_after_the_previewed_account_is_saved() {
        let previewed = navidog_sql::security::AccountDraft {
            name: "ada".into(),
            host: "%".into(),
            password: "secret".into(),
            ssl_type: "ANY".into(),
            ..navidog_sql::security::AccountDraft::default()
        };
        let baseline = super::saved_account(previewed);
        let moved = navidog_sql::security::AccountDraft {
            name: "ada".into(),
            host: "%".into(),
            ssl_type: "NONE".into(),
            ..navidog_sql::security::AccountDraft::default()
        };
        assert_eq!(baseline.ssl_type, "ANY");
        assert_eq!(baseline.previous_ssl.as_deref(), Some("ANY"));
        assert_ne!(
            super::comparable(moved.clone()),
            super::comparable(baseline)
        );
        assert_eq!(
            super::comparable(super::saved_account(moved.clone())),
            super::comparable(moved)
        );
    }

    #[test]
    fn a_running_save_keeps_the_snapshot_from_when_it_started() {
        let running = super::saved_account(navidog_sql::security::AccountDraft {
            name: "ada".into(),
            host: "%".into(),
            ssl_type: "ANY".into(),
            ..navidog_sql::security::AccountDraft::default()
        });
        let later = super::saved_account(navidog_sql::security::AccountDraft {
            name: "ada".into(),
            host: "%".into(),
            ssl_type: "NONE".into(),
            ..navidog_sql::security::AccountDraft::default()
        });
        let adopted = super::adopted_account(running, later);
        assert_eq!(adopted.ssl_type, "ANY");
        assert_eq!(adopted.previous_ssl.as_deref(), Some("ANY"));
    }

    #[test]
    fn a_loaded_postgres_role_keeps_nologin_and_superuser() {
        let row = [
            Some("f".into()),
            Some("t".into()),
            Some("f".into()),
            Some("f".into()),
            Some("t".into()),
            Some("f".into()),
            Some("-1".into()),
            Some(String::new()),
        ];
        let fill = super::postgres_profile("app".into(), String::new(), &row);
        let attrs = fill.postgres.expect("role attributes");
        assert!(!attrs.login);
        assert!(attrs.superuser);
        assert!(attrs.inherit);
        assert!(attrs.connection_limit.is_empty());
    }
}
