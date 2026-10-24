//! Connection form (11-connection.md §1.2, §2, §3).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{IndexPath, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Render, SharedString, Styled as _, Task,
    TestSupportExt as _, Window, div,
};
use navidog_core::connection::{
    AttachedDatabase, Connection, DatabaseItem, HostPreference, Member, SecretField, SshAuth,
    SslMode,
};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace};
use navidog_db::{ConnectParams, DriverRegistry, Secrets, runtime};
use navidog_ui::skin::{FormRow, confirm_with, form_note};
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use navidog_db::sessions::Sessions;

pub fn open_new(driver: SharedString, window: &mut Window, cx: &mut App) {
    let mut conn = Connection::new(&driver);
    if let Selection::Group(g) = Workspace::global(cx).read(cx).selection() {
        conn.group = Some(g.clone());
    }
    open(conn, None, window, cx);
}

/// New connection prefilled from a URI (F-CONN-016).
pub fn open_from_uri(uri: navidog_core::uri::DatabaseUri, window: &mut Window, cx: &mut App) {
    let mut conn = Connection::new(&uri.driver);
    conn.name = match &uri.database {
        Some(db) if !uri.host.is_empty() => format!("{}/{db}", uri.host),
        _ => uri.host.clone(),
    };
    if !uri.host.is_empty() {
        conn.general.host = uri.host;
    }
    if uri.port.is_some() {
        conn.general.port = uri.port;
    }
    if !uri.user.is_empty() {
        conn.general.user = uri.user;
    }
    if uri.database.is_some() {
        conn.general.database = uri.database;
    }
    conn.general.save_password |= uri.password.is_some();
    open(conn, Some([uri.password, None, None]), window, cx);
}

pub fn open_edit(id: String, window: &mut Window, cx: &mut App) {
    let store = ConnectionStore::global(cx);
    let Some(conn) = store.read(cx).get(&id).cloned() else {
        return;
    };
    // Editing fills in the saved secrets; when locked they stay empty and saving keeps the old values
    let secrets =
        SecretField::ALL.map(|f| store.update(cx, |s, _| s.reveal(&id, f)).ok().flatten());
    open(conn, Some(secrets), window, cx);
}

fn open(conn: Connection, secrets: Option<[Option<String>; 3]>, window: &mut Window, cx: &mut App) {
    let form = cx.new(|cx| ConnectionForm::new(conn, secrets, window, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        // Enter in a single-line field saves, like OK, instead of closing and losing the input
        confirm_with(dialog, &form, ConnectionForm::save)
            .w(tokens::conn_form::WIDTH.to_pixels(window.rem_size()))
            .close_button(false)
            .child(form.clone())
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab_ {
    General,
    Advanced,
    Databases,
    Ssl,
    Ssh,
    Remark,
}

impl Tab_ {
    fn label(self) -> String {
        match self {
            Self::General => t!("conn.tab_general"),
            Self::Advanced => t!("conn.tab_advanced"),
            Self::Databases => t!("conn.tab_databases"),
            Self::Ssl => t!("conn.tab_ssl"),
            Self::Ssh => t!("conn.tab_ssh"),
            Self::Remark => t!("conn.tab_remark"),
        }
        .to_string()
    }
}

type Text = Entity<InputState>;
type Choice = Entity<SelectState<Vec<SharedString>>>;

struct ConnectionForm {
    draft: Connection,
    is_new: bool,
    /// The secrets were filled in for editing (not locked), so saving writes them all back
    secrets_loaded: bool,
    tab: Tab_,
    name: Text,
    /// PostgreSQL node type: single or multiple
    node_type: Choice,
    was_multi: bool,
    /// PostgreSQL multiple nodes, one `host:port` per line, the first one is `host`
    members: Entity<TextareaState>,
    host_preference: Choice,
    host: Text,
    port: Text,
    database: Text,
    file: Text,
    user: Text,
    password: Text,
    group: Choice,
    group_ids: Vec<Option<String>>,
    keepalive: Text,
    connect_timeout: Text,
    read_timeout: Text,
    write_timeout: Text,
    socket: Text,
    session_limit: Text,
    params: Text,
    initial_queries: Text,
    keepalive_on: bool,
    read_timeout_on: bool,
    write_timeout_on: bool,
    socket_on: bool,
    session_limit_on: bool,
    new_db: Text,
    attach_name: Text,
    attach_file: Text,
    ssl_mode: Choice,
    client_key: Text,
    client_cert: Text,
    ca_cert: Text,
    cipher: Text,
    crl: Text,
    ssh_host: Text,
    ssh_port: Text,
    ssh_user: Text,
    ssh_auth: Choice,
    ssh_password: Text,
    ssh_key: Text,
    ssh_passphrase: Text,
    remark: Entity<TextareaState>,
    testing: Option<Task<()>>,
    error: Option<SharedString>,
}

fn text(
    value: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut Context<ConnectionForm>,
) -> Text {
    let value = value.into();
    cx.new(|cx| InputState::new(window, cx).default_value(value))
}

fn secret(value: Option<String>, window: &mut Window, cx: &mut Context<ConnectionForm>) -> Text {
    cx.new(|cx| {
        InputState::new(window, cx)
            .masked(true)
            .default_value(value.unwrap_or_default())
    })
}

fn choice(
    items: Vec<SharedString>,
    selected: usize,
    window: &mut Window,
    cx: &mut Context<ConnectionForm>,
) -> Choice {
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
}

fn opt_num(v: Option<u32>) -> String {
    v.map(|n| n.to_string()).unwrap_or_default()
}

const SSH_AUTHS: [SshAuth; 3] = [SshAuth::Password, SshAuth::PublicKey, SshAuth::Both];

impl ConnectionForm {
    fn new(
        conn: Connection,
        secrets: Option<[Option<String>; 3]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let is_new = secrets.is_none();
        let secrets_loaded = secrets
            .as_ref()
            .is_some_and(|s| s.iter().any(Option::is_some))
            || is_new;
        let [pw, ssh_pw, passphrase] = secrets.unwrap_or_default();

        let store = ConnectionStore::global(cx);
        let groups = store.read(cx).groups().to_vec();
        let mut group_ids = vec![None];
        let mut group_labels: Vec<SharedString> =
            vec![t!("conn.my_connections").to_string().into()];
        for g in &groups {
            group_ids.push(Some(g.id.clone()));
            group_labels.push(format!("{}{}", "    ".repeat(depth(&groups, &g.id)), g.name).into());
        }
        let group_ix = group_ids.iter().position(|g| *g == conn.group).unwrap_or(0);

        let a = &conn.advanced;
        let ssl_ix = SslMode::ALL
            .iter()
            .position(|m| *m == conn.ssl.mode)
            .unwrap_or(1);
        let auth_ix = SSH_AUTHS
            .iter()
            .position(|m| *m == conn.ssh.auth)
            .unwrap_or(0);
        let auth_labels = vec![
            t!("conn.auth_password").to_string().into(),
            t!("conn.auth_public_key").to_string().into(),
            t!("conn.auth_both").to_string().into(),
        ];
        let name = text(conn.name.clone(), window, cx);
        name.update(cx, |s, cx| s.focus(window, cx));
        let g = &conn.general;
        let node_type = choice(
            vec![
                t!("conn.node_single").to_string().into(),
                t!("conn.node_multiple").to_string().into(),
            ],
            usize::from(!g.members.is_empty()),
            window,
            cx,
        );
        cx.observe_in(&node_type, window, |this: &mut Self, _, window, cx| {
            let multi = this.multi_node(cx);
            if multi != this.was_multi {
                this.was_multi = multi;
                this.sync_members(multi, window, cx);
            }
            cx.notify();
        })
        .detach();
        let members = std::iter::once(Member {
            host: g.host.clone(),
            port: g.port,
        })
        .chain(g.members.iter().cloned())
        .map(|m| m.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        let pref_ix = HostPreference::ALL
            .iter()
            .position(|p| *p == g.host_preference)
            .unwrap_or(0);
        let host_preference = choice(
            vec![
                t!("conn.pref_any").to_string().into(),
                t!("conn.pref_read_write").to_string().into(),
                t!("conn.pref_read_only").to_string().into(),
            ],
            pref_ix,
            window,
            cx,
        );
        Self {
            is_new,
            secrets_loaded,
            tab: Tab_::General,
            name,
            was_multi: !g.members.is_empty(),
            node_type,
            members: cx.new(|cx| TextareaState::new(window, cx).default_value(members)),
            host_preference,
            host: text(conn.general.host.clone(), window, cx),
            port: text(
                conn.general.port.map(|p| p.to_string()).unwrap_or_default(),
                window,
                cx,
            ),
            database: text(
                conn.general.database.clone().unwrap_or_default(),
                window,
                cx,
            ),
            file: text(conn.general.file.clone().unwrap_or_default(), window, cx),
            user: text(conn.general.user.clone(), window, cx),
            password: secret(pw, window, cx),
            group: choice(group_labels, group_ix, window, cx),
            group_ids,
            keepalive: text(
                a.keepalive_secs.map_or("240".into(), |s| s.to_string()),
                window,
                cx,
            ),
            connect_timeout: text(opt_num(a.connect_timeout_secs), window, cx),
            read_timeout: text(opt_num(a.read_timeout_secs), window, cx),
            write_timeout: text(opt_num(a.write_timeout_secs), window, cx),
            socket: text(a.socket.clone().unwrap_or_default(), window, cx),
            session_limit: text(
                a.session_limit.map_or("10".into(), |s| s.to_string()),
                window,
                cx,
            ),
            params: text(a.params.clone(), window, cx),
            initial_queries: text(a.initial_queries.clone(), window, cx),
            keepalive_on: a.keepalive_secs.is_some(),
            read_timeout_on: a.read_timeout_secs.is_some(),
            write_timeout_on: a.write_timeout_secs.is_some(),
            socket_on: a.socket.is_some(),
            session_limit_on: a.session_limit.is_some(),
            new_db: text("", window, cx),
            attach_name: text("", window, cx),
            attach_file: text("", window, cx),
            ssl_mode: choice(
                SslMode::ALL
                    .iter()
                    .map(|m| SharedString::from(m.as_str()))
                    .collect(),
                ssl_ix,
                window,
                cx,
            ),
            client_key: text(conn.ssl.client_key.clone().unwrap_or_default(), window, cx),
            client_cert: text(conn.ssl.client_cert.clone().unwrap_or_default(), window, cx),
            ca_cert: text(conn.ssl.ca_cert.clone().unwrap_or_default(), window, cx),
            cipher: text(conn.ssl.cipher.clone().unwrap_or_default(), window, cx),
            crl: text(conn.ssl.crl.clone().unwrap_or_default(), window, cx),
            ssh_host: text(conn.ssh.host.clone(), window, cx),
            ssh_port: text(conn.ssh.port.to_string(), window, cx),
            ssh_user: text(conn.ssh.user.clone(), window, cx),
            ssh_auth: choice(auth_labels, auth_ix, window, cx),
            ssh_password: secret(ssh_pw, window, cx),
            ssh_key: text(conn.ssh.private_key.clone().unwrap_or_default(), window, cx),
            ssh_passphrase: secret(passphrase, window, cx),
            remark: {
                let value = conn.remark.clone();
                cx.new(|cx| TextareaState::new(window, cx).default_value(value))
            },
            testing: None,
            error: None,
            draft: conn,
        }
    }

    fn tabs(&self) -> Vec<Tab_> {
        if self.draft.supports_network() {
            vec![
                Tab_::General,
                Tab_::Advanced,
                Tab_::Databases,
                Tab_::Ssl,
                Tab_::Ssh,
                Tab_::Remark,
            ]
        } else {
            vec![Tab_::General, Tab_::Advanced, Tab_::Databases, Tab_::Remark]
        }
    }

    fn value(input: &Text, cx: &App) -> String {
        input.read(cx).value().trim().to_string()
    }

    fn opt(input: &Text, cx: &App) -> Option<String> {
        Some(Self::value(input, cx)).filter(|v| !v.is_empty())
    }

    fn selected(choice: &Choice, cx: &App) -> usize {
        choice
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .unwrap_or(0)
    }

    fn multi_node(&self, cx: &App) -> bool {
        self.draft.driver == "postgres" && Self::selected(&self.node_type, cx) == 1
    }

    /// Carries the host between the host/port fields and the first member line on a node type switch.
    fn sync_members(&mut self, to_multi: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.members.read(cx).value().to_string();
        let mut lines: Vec<String> = text.lines().map(String::from).collect();
        if to_multi {
            let first = Member {
                host: Self::value(&self.host, cx),
                port: Self::value(&self.port, cx).parse().ok(),
            };
            match lines.first_mut() {
                Some(l) => *l = first.to_string(),
                None => lines.push(first.to_string()),
            }
            let text = lines.join("\n");
            self.members
                .update(cx, |s, cx| s.set_value(text, window, cx));
        } else if let Some(first) = lines.first().and_then(|l| Member::parse(l)) {
            let port = first.port.map(|p| p.to_string()).unwrap_or_default();
            self.host
                .update(cx, |s, cx| s.set_value(first.host, window, cx));
            self.port.update(cx, |s, cx| s.set_value(port, window, cx));
        }
    }

    fn num(input: &Text, on: bool, cx: &App) -> Option<u32> {
        if !on {
            return None;
        }
        Self::value(input, cx).parse().ok()
    }

    /// Collects the full connection settings from the form controls.
    fn collect(&self, cx: &App) -> Result<Connection, SharedString> {
        let mut c = self.draft.clone();
        c.name = Self::value(&self.name, cx);
        if c.name.is_empty() {
            return Err(t!("conn.required_name").to_string().into());
        }
        let group_ix = self
            .group
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .unwrap_or(0);
        c.group = self.group_ids.get(group_ix).cloned().flatten();
        let g = &mut c.general;
        g.host = Self::value(&self.host, cx);
        g.port = Self::value(&self.port, cx).parse().ok();
        g.database = Self::opt(&self.database, cx);
        g.file = Self::opt(&self.file, cx);
        g.user = Self::value(&self.user, cx);
        if self.multi_node(cx) {
            let text = self.members.read(cx).value().to_string();
            let mut members: Vec<Member> = text.lines().filter_map(Member::parse).collect();
            if members.is_empty() {
                return Err(t!("conn.members_required").to_string().into());
            }
            let first = members.remove(0);
            (g.host, g.port, g.members) = (first.host, first.port, members);
            let ix = Self::selected(&self.host_preference, cx);
            g.host_preference = HostPreference::ALL[ix.min(2)];
        } else {
            g.members.clear();
            g.host_preference = HostPreference::Any;
        }
        let a = &mut c.advanced;
        a.keepalive_secs = Self::num(&self.keepalive, self.keepalive_on, cx);
        a.connect_timeout_secs = Self::value(&self.connect_timeout, cx).parse().ok();
        a.read_timeout_secs = Self::num(&self.read_timeout, self.read_timeout_on, cx);
        a.write_timeout_secs = Self::num(&self.write_timeout, self.write_timeout_on, cx);
        a.socket = self
            .socket_on
            .then(|| Self::value(&self.socket, cx))
            .filter(|s| !s.is_empty());
        a.session_limit = Self::num(&self.session_limit, self.session_limit_on, cx);
        a.params = Self::value(&self.params, cx);
        a.initial_queries = Self::value(&self.initial_queries, cx);
        let ssl_ix = self
            .ssl_mode
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .unwrap_or(1);
        c.ssl.mode = SslMode::ALL[ssl_ix.min(4)];
        c.ssl.client_key = Self::opt(&self.client_key, cx);
        c.ssl.client_cert = Self::opt(&self.client_cert, cx);
        c.ssl.ca_cert = Self::opt(&self.ca_cert, cx);
        c.ssl.cipher = Self::opt(&self.cipher, cx);
        c.ssl.crl = Self::opt(&self.crl, cx);
        c.ssh.host = Self::value(&self.ssh_host, cx);
        c.ssh.port = Self::value(&self.ssh_port, cx).parse().unwrap_or(22);
        c.ssh.user = Self::value(&self.ssh_user, cx);
        let auth_ix = self
            .ssh_auth
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .unwrap_or(0);
        c.ssh.auth = SSH_AUTHS[auth_ix.min(2)];
        c.ssh.private_key = Self::opt(&self.ssh_key, cx);
        c.remark = self.remark.read(cx).value().to_string();
        Ok(c)
    }

    fn secrets(&self, cx: &App) -> Secrets {
        Secrets {
            password: Self::opt(&self.password, cx),
            ssh_password: Self::opt(&self.ssh_password, cx),
            ssh_passphrase: Self::opt(&self.ssh_passphrase, cx),
        }
    }

    fn test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.testing.take().is_some() {
            cx.notify();
            return;
        }
        let conn = match self.collect(cx) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        let Some(driver) = DriverRegistry::get(cx, &conn.driver) else {
            return;
        };
        let params = ConnectParams::from_connection(&conn, self.secrets(cx));
        let job = runtime::spawn(navidog_db::test_connection(driver, params));
        self.testing = Some(cx.spawn_in(window, async move |this, cx| {
            let result = job
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| crate::db_text(&e)));
            let _ = this.update_in(cx, |this, window, cx| {
                this.testing = None;
                cx.notify();
                let (title, body): (SharedString, SharedString) = match result {
                    Ok(version) => (t!("conn.test_ok").to_string().into(), version.into()),
                    Err(e) => (t!("conn.test_failed").to_string().into(), e.into()),
                };
                window.open_alert_dialog(cx, move |alert, _, _| {
                    alert
                        .title(title.clone())
                        .description(body.clone())
                        .ok_text(t!("conn.ok").to_string())
                });
            });
        }));
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let conn = match self.collect(cx) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(e);
                self.tab = Tab_::General;
                cx.notify();
                return;
            }
        };
        // When editing without the secrets filled in (master password locked), empty means keep
        let keep_if_empty = !self.is_new && !self.secrets_loaded;
        let pick = |input: &Text, cx: &App| {
            let v = input.read(cx).value().to_string();
            if v.is_empty() && keep_if_empty {
                None
            } else {
                Some(v)
            }
        };
        let secrets = vec![
            (SecretField::Password, pick(&self.password, cx)),
            (SecretField::SshPassword, pick(&self.ssh_password, cx)),
            (SecretField::SshPassphrase, pick(&self.ssh_passphrase, cx)),
        ];
        let id = conn.id.clone();
        let was_open = Sessions::global(cx).read(cx).state(&id).is_open();
        let store = ConnectionStore::global(cx);
        if let Err(e) = store.update(cx, |s, cx| s.upsert(conn, &secrets, cx)) {
            self.error = Some(e.localized_message().into());
            cx.notify();
            return;
        }
        window.close_dialog(cx);
        Workspace::global(cx).update(cx, |w, cx| w.select(Selection::Connection(id.clone()), cx));
        if was_open {
            window.open_alert_dialog(cx, move |alert, _, _| {
                let id = id.clone();
                alert
                    .title(t!("conn.reopen_required").to_string())
                    .show_cancel(true)
                    .ok_text(t!("conn.reopen_now").to_string())
                    .cancel_text(t!("conn.later").to_string())
                    .on_ok(move |_, window, cx| {
                        crate::reopen_connection(id.clone(), window, cx);
                        true
                    })
            });
        }
    }

    fn browse(
        &mut self,
        target: Text,
        directories: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: !directories,
            directories,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.first()
            {
                let path = path.to_string_lossy().to_string();
                let _ = target.update_in(cx, |s, window, cx| s.set_value(path, window, cx));
            }
        })
        .detach();
    }

    fn check(
        &self,
        id: &'static str,
        label: String,
        checked: bool,
        f: fn(&mut Self, bool),
        cx: &mut Context<Self>,
    ) -> Checkbox {
        Checkbox::new(id)
            .label(label)
            .checked(checked)
            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                f(this, *checked);
                cx.notify();
            }))
    }

    fn path_field(
        &self,
        id: &'static str,
        input: &Text,
        wide: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let width = if wide {
            tokens::form::FIELD_WIDTH_WIDE
        } else {
            tokens::form::FIELD_WIDTH
        };
        let target = input.clone();
        h_flex()
            .gap_1()
            .child(
                Input::new(input)
                    .id(SharedString::from(format!("{id}-input")))
                    .w(width.rems()),
            )
            .child(
                Button::new(id)
                    .label(t!("conn.choose").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.browse(target.clone(), false, window, cx)
                    })),
            )
    }

    fn render_general(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let w = tokens::form::FIELD_WIDTH.rems();
        let mut rows = vec![
            FormRow::new(
                t!("conn.name").to_string(),
                Input::new(&self.name).id("conn-name").w(w),
            )
            .into_any_element(),
            FormRow::new(
                t!("conn.add_to").to_string(),
                Select::new(&self.group).w(tokens::form::POPUP_WIDTH.rems()),
            )
            .into_any_element(),
        ];
        if self.draft.supports_network() {
            let postgres = self.draft.driver == "postgres";
            if postgres {
                rows.push(
                    FormRow::new(
                        t!("conn.node_type").to_string(),
                        Select::new(&self.node_type).w(tokens::form::POPUP_WIDTH_SHORT.rems()),
                    )
                    .into_any_element(),
                );
            }
            if self.multi_node(cx) {
                rows.push(
                    FormRow::new(
                        t!("conn.members").to_string(),
                        Textarea::new(&self.members)
                            .w(w)
                            .h(tokens::form::LIST_HEIGHT.rems()),
                    )
                    .into_any_element(),
                );
                rows.push(
                    FormRow::new(
                        t!("conn.host_preference").to_string(),
                        Select::new(&self.host_preference)
                            .w(tokens::form::POPUP_WIDTH_MEDIUM.rems()),
                    )
                    .into_any_element(),
                );
            } else {
                rows.push(
                    FormRow::new(t!("conn.host").to_string(), Input::new(&self.host).w(w))
                        .into_any_element(),
                );
                rows.push(
                    FormRow::new(
                        t!("conn.port").to_string(),
                        Input::new(&self.port).w(tokens::form::FIELD_WIDTH_SHORT.rems()),
                    )
                    .into_any_element(),
                );
            }
            if postgres {
                rows.push(
                    FormRow::new(
                        t!("conn.initial_database").to_string(),
                        Input::new(&self.database).w(w),
                    )
                    .into_any_element(),
                );
            }
        } else {
            let create_new = self.draft.general.create_new;
            rows.push(
                FormRow::new(
                    t!("conn.sqlite_type").to_string(),
                    RadioGroup::vertical("sqlite-type")
                        .selected_index(Some(usize::from(create_new)))
                        .child(Radio::new("existing").label(t!("conn.sqlite_existing").to_string()))
                        // The group stretches to the column, so the row's center is
                        // empty space past the label. The label hugs its text.
                        .child(
                            Radio::new("new")
                                .accessibility_label(t!("conn.sqlite_new").to_string())
                                .child(
                                    div()
                                        .id("sqlite-new")
                                        .self_start()
                                        .child(t!("conn.sqlite_new").to_string())
                                        .test_support(),
                                ),
                        )
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.draft.general.create_new = *ix == 1;
                            cx.notify();
                        })),
                )
                .into_any_element(),
            );
            rows.push(
                FormRow::new(
                    t!("conn.database_file").to_string(),
                    self.path_field("file-browse", &self.file, false, cx),
                )
                .into_any_element(),
            );
        }
        rows.push(
            FormRow::new(t!("conn.user").to_string(), Input::new(&self.user).w(w))
                .into_any_element(),
        );
        rows.push(
            FormRow::new(
                t!("conn.password").to_string(),
                Input::new(&self.password).w(w),
            )
            .into_any_element(),
        );
        rows.push(
            FormRow::bare(self.check(
                "save-password",
                t!("conn.save_password").to_string(),
                self.draft.general.save_password,
                |t, v| t.draft.general.save_password = v,
                cx,
            ))
            .into_any_element(),
        );
        rows.push(
            FormRow::bare(form_note(t!("conn.password_note").to_string(), cx)).into_any_element(),
        );
        rows
    }

    fn toggled_number(
        &self,
        id: &'static str,
        label: String,
        on: bool,
        input: &Text,
        f: fn(&mut Self, bool),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        FormRow::bare(
            h_flex()
                .gap_2()
                .child(self.check(id, label, on, f, cx))
                .child(
                    Input::new(input)
                        .w(tokens::form::FIELD_WIDTH_SHORT.rems())
                        .disabled(!on),
                ),
        )
        .compact()
        .into_any_element()
    }

    fn render_advanced(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = vec![
            FormRow::new(
                t!("conn.settings_location").to_string(),
                div().text_color(palette(cx).label_secondary).child(
                    navidog_core::paths::profile_dir(&self.draft.id)
                        .display()
                        .to_string(),
                ),
            )
            .compact()
            .into_any_element(),
        ];
        let mysql = matches!(self.draft.driver.as_str(), "mysql" | "mariadb");
        let pg = self.draft.driver == "postgres";
        if mysql || pg {
            rows.push(self.toggled_number(
                "keepalive",
                t!("conn.keepalive").to_string(),
                self.keepalive_on,
                &self.keepalive,
                |t, v| t.keepalive_on = v,
                cx,
            ));
        }
        if mysql {
            rows.push(
                FormRow::new(
                    t!("conn.connect_timeout")
                        .to_string()
                        .trim_end_matches(':')
                        .trim_end_matches('：')
                        .to_string(),
                    Input::new(&self.connect_timeout).w(tokens::form::FIELD_WIDTH_SHORT.rems()),
                )
                .compact()
                .into_any_element(),
            );
            rows.push(self.toggled_number(
                "read-timeout",
                t!("conn.read_timeout").to_string(),
                self.read_timeout_on,
                &self.read_timeout,
                |t, v| t.read_timeout_on = v,
                cx,
            ));
            rows.push(self.toggled_number(
                "write-timeout",
                t!("conn.write_timeout").to_string(),
                self.write_timeout_on,
                &self.write_timeout,
                |t, v| t.write_timeout_on = v,
                cx,
            ));
            rows.push(
                FormRow::bare(self.check(
                    "compression",
                    t!("conn.compression").to_string(),
                    self.draft.advanced.compression,
                    |t, v| t.draft.advanced.compression = v,
                    cx,
                ))
                .compact()
                .into_any_element(),
            );
        }
        rows.push(
            FormRow::bare(self.check(
                "auto-connect",
                t!("conn.auto_connect").to_string(),
                self.draft.advanced.auto_connect,
                |t, v| t.draft.advanced.auto_connect = v,
                cx,
            ))
            .compact()
            .into_any_element(),
        );
        if mysql {
            rows.push(
                FormRow::bare(
                    h_flex()
                        .gap_2()
                        .child(self.check(
                            "socket",
                            t!("conn.socket").to_string(),
                            self.socket_on,
                            |t, v| t.socket_on = v,
                            cx,
                        ))
                        .child(
                            Input::new(&self.socket)
                                .w(tokens::form::POPUP_WIDTH_SHORT.rems())
                                .disabled(!self.socket_on),
                        ),
                )
                .compact()
                .into_any_element(),
            );
        }
        if mysql || pg {
            rows.push(self.toggled_number(
                "session-limit",
                t!("conn.session_limit").to_string(),
                self.session_limit_on,
                &self.session_limit,
                |t, v| t.session_limit_on = v,
                cx,
            ));
        }
        if pg {
            rows.push(
                FormRow::new(
                    t!("conn.params").to_string(),
                    Input::new(&self.params).w(tokens::form::FIELD_WIDTH.rems()),
                )
                .compact()
                .into_any_element(),
            );
        }
        rows.push(
            FormRow::new(
                t!("conn.initial_queries").to_string(),
                Input::new(&self.initial_queries).w(tokens::form::FIELD_WIDTH.rems()),
            )
            .compact()
            .into_any_element(),
        );
        rows
    }

    fn render_databases(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = palette(cx);
        let list = |rows: Vec<AnyElement>| {
            v_flex()
                .w(tokens::form::LIST_WIDTH.rems())
                .h(tokens::form::LIST_HEIGHT.rems())
                .border_1()
                .border_color(p.separator)
                .bg(p.control_bg)
                .children(rows)
        };
        let header = |cols: Vec<String>| {
            h_flex()
                .h(tokens::form::LIST_HEADER_HEIGHT.rems())
                .px_2()
                .gap_4()
                .border_b_1()
                .border_color(p.separator)
                .text_color(p.label)
                .children(cols.into_iter().map(|c| div().child(c)))
        };
        if self.draft.supports_network() {
            let items = self.draft.databases.items.clone();
            let mut rows = vec![
                header(vec![
                    t!("conn.col_auto_open").to_string(),
                    t!("conn.col_name").to_string(),
                ])
                .into_any_element(),
            ];
            for (ix, item) in items.iter().enumerate() {
                rows.push(
                    h_flex()
                        .h(tokens::form::LIST_ROW_HEIGHT.rems())
                        .px_2()
                        .gap_4()
                        .child(
                            Checkbox::new(("auto-open", ix))
                                .checked(item.auto_open)
                                .on_click(cx.listener(move |this, v: &bool, _, cx| {
                                    this.draft.databases.items[ix].auto_open = *v;
                                    cx.notify();
                                })),
                        )
                        .child(item.name.clone())
                        .into_any_element(),
                );
            }
            vec![
                FormRow::bare(self.check(
                    "custom-list",
                    t!("conn.use_custom_db_list").to_string(),
                    self.draft.databases.custom_list,
                    |t, v| t.draft.databases.custom_list = v,
                    cx,
                ))
                .into_any_element(),
                FormRow::bare(list(rows)).into_any_element(),
                FormRow::bare(
                    h_flex()
                        .gap_1()
                        .child(Input::new(&self.new_db).w(tokens::form::FIELD_WIDTH.rems()))
                        .child(Button::new("db-add").label("+").on_click(cx.listener(
                            |this, _, window, cx| {
                                let name = Self::value(&this.new_db, cx);
                                if !name.is_empty()
                                    && !this.draft.databases.items.iter().any(|i| i.name == name)
                                {
                                    this.draft.databases.items.push(DatabaseItem {
                                        name,
                                        auto_open: false,
                                    });
                                    this.new_db.update(cx, |s, cx| s.set_value("", window, cx));
                                }
                                cx.notify();
                            },
                        )))
                        .child(Button::new("db-remove").label("−").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.draft.databases.items.pop();
                                cx.notify();
                            },
                        ))),
                )
                .into_any_element(),
            ]
        } else {
            let mut rows = vec![
                header(vec![
                    t!("conn.col_name").to_string(),
                    t!("conn.col_file").to_string(),
                ])
                .into_any_element(),
            ];
            for item in &self.draft.databases.attached {
                rows.push(
                    h_flex()
                        .h(tokens::form::LIST_ROW_HEIGHT.rems())
                        .px_2()
                        .gap_4()
                        .child(item.name.clone())
                        .child(item.file.clone())
                        .into_any_element(),
                );
            }
            vec![
                FormRow::bare(list(rows)).into_any_element(),
                FormRow::new(
                    t!("conn.new_database_name").to_string(),
                    Input::new(&self.attach_name).w(tokens::form::FIELD_WIDTH.rems()),
                )
                .into_any_element(),
                FormRow::new(
                    t!("conn.database_file").to_string(),
                    self.path_field("attach-browse", &self.attach_file, false, cx),
                )
                .into_any_element(),
                FormRow::bare(
                    h_flex()
                        .gap_1()
                        .child(Button::new("attach-add").label("+").on_click(cx.listener(
                            |this, _, window, cx| {
                                let name = Self::value(&this.attach_name, cx);
                                let file = Self::value(&this.attach_file, cx);
                                if !name.is_empty() && !file.is_empty() {
                                    this.draft
                                        .databases
                                        .attached
                                        .push(AttachedDatabase { name, file });
                                    this.attach_name
                                        .update(cx, |s, cx| s.set_value("", window, cx));
                                    this.attach_file
                                        .update(cx, |s, cx| s.set_value("", window, cx));
                                }
                                cx.notify();
                            },
                        )))
                        .child(
                            Button::new("attach-remove")
                                .label("−")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.draft.databases.attached.pop();
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element(),
            ]
        }
    }

    fn render_ssl(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let pg = self.draft.driver == "postgres";
        let mut rows = vec![
            FormRow::bare(self.check(
                "use-ssl",
                t!("conn.use_ssl").to_string(),
                self.draft.ssl.enabled,
                |t, v| t.draft.ssl.enabled = v,
                cx,
            ))
            .into_any_element(),
        ];
        if !self.draft.ssl.enabled {
            return rows;
        }
        if pg {
            rows.push(
                FormRow::new(
                    t!("conn.ssl_mode").to_string(),
                    Select::new(&self.ssl_mode).w(tokens::form::POPUP_WIDTH_MEDIUM.rems()),
                )
                .into_any_element(),
            );
        }
        rows.push(
            FormRow::bare(self.check(
                "use-auth",
                t!("conn.use_auth").to_string(),
                self.draft.ssl.use_auth,
                |t, v| t.draft.ssl.use_auth = v,
                cx,
            ))
            .into_any_element(),
        );
        if self.draft.ssl.use_auth {
            rows.push(
                FormRow::new(
                    t!("conn.client_key").to_string(),
                    self.path_field("key-browse", &self.client_key, false, cx),
                )
                .into_any_element(),
            );
            rows.push(
                FormRow::new(
                    t!("conn.client_cert").to_string(),
                    self.path_field("cert-browse", &self.client_cert, false, cx),
                )
                .into_any_element(),
            );
            let ca_label = if pg {
                t!("conn.root_cert")
            } else {
                t!("conn.ca_cert")
            };
            rows.push(
                FormRow::new(
                    ca_label.to_string(),
                    self.path_field("ca-browse", &self.ca_cert, false, cx),
                )
                .into_any_element(),
            );
            if pg {
                rows.push(
                    FormRow::new(
                        t!("conn.crl").to_string(),
                        self.path_field("crl-browse", &self.crl, false, cx),
                    )
                    .into_any_element(),
                );
            } else {
                rows.push(
                    FormRow::bare(self.check(
                        "verify-ca",
                        t!("conn.verify_ca").to_string(),
                        self.draft.ssl.verify_server_cert,
                        |t, v| t.draft.ssl.verify_server_cert = v,
                        cx,
                    ))
                    .into_any_element(),
                );
                rows.push(
                    FormRow::new(
                        t!("conn.cipher").to_string(),
                        Input::new(&self.cipher).w(tokens::form::FIELD_WIDTH.rems()),
                    )
                    .into_any_element(),
                );
            }
        }
        rows
    }

    fn render_ssh(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = &self.draft.ssh;
        let w = tokens::form::FIELD_WIDTH.rems();
        let mut rows = vec![
            FormRow::bare(self.check(
                "use-ssh",
                t!("conn.use_ssh").to_string(),
                s.enabled,
                |t, v| t.draft.ssh.enabled = v,
                cx,
            ))
            .into_any_element(),
        ];
        if !s.enabled {
            return rows;
        }
        let auth_ix = self
            .ssh_auth
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .unwrap_or(0);
        let auth = SSH_AUTHS[auth_ix.min(2)];
        rows.push(
            FormRow::new(t!("conn.host").to_string(), Input::new(&self.ssh_host).w(w))
                .into_any_element(),
        );
        rows.push(
            FormRow::new(
                t!("conn.port").to_string(),
                Input::new(&self.ssh_port).w(tokens::form::FIELD_WIDTH_SHORT.rems()),
            )
            .into_any_element(),
        );
        rows.push(
            FormRow::new(t!("conn.user").to_string(), Input::new(&self.ssh_user).w(w))
                .into_any_element(),
        );
        rows.push(
            FormRow::new(
                t!("conn.auth_method").to_string(),
                Select::new(&self.ssh_auth).w(tokens::form::POPUP_WIDTH_SHORT.rems()),
            )
            .into_any_element(),
        );
        if auth.uses_password() {
            rows.push(
                FormRow::new(
                    t!("conn.password").to_string(),
                    Input::new(&self.ssh_password).w(w),
                )
                .into_any_element(),
            );
            rows.push(
                FormRow::bare(self.check(
                    "ssh-save-password",
                    t!("conn.save_password").to_string(),
                    s.save_password,
                    |t, v| t.draft.ssh.save_password = v,
                    cx,
                ))
                .into_any_element(),
            );
        }
        if auth.uses_key() {
            rows.push(
                FormRow::new(
                    t!("conn.private_key").to_string(),
                    self.path_field("ssh-key-browse", &self.ssh_key, false, cx),
                )
                .into_any_element(),
            );
            rows.push(
                FormRow::new(
                    t!("conn.passphrase").to_string(),
                    Input::new(&self.ssh_passphrase).w(w),
                )
                .into_any_element(),
            );
            rows.push(
                FormRow::bare(self.check(
                    "ssh-save-passphrase",
                    t!("conn.save_passphrase").to_string(),
                    s.save_passphrase,
                    |t, v| t.draft.ssh.save_passphrase = v,
                    cx,
                ))
                .into_any_element(),
            );
        }
        rows.push(
            FormRow::bare(form_note(t!("conn.password_note").to_string(), cx)).into_any_element(),
        );
        rows
    }
}

fn depth(groups: &[navidog_core::connection::Group], id: &str) -> usize {
    let mut d = 0;
    let mut cur = groups
        .iter()
        .find(|g| g.id == id)
        .and_then(|g| g.parent.clone());
    while let Some(p) = cur {
        d += 1;
        cur = groups
            .iter()
            .find(|g| g.id == p)
            .and_then(|g| g.parent.clone());
    }
    d
}

impl Render for ConnectionForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = window;
        let tabs = self.tabs();
        let selected = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
        let title = if self.is_new {
            t!(
                "conn.new_connection_of",
                driver = crate::driver_name(&self.draft.driver, cx)
            )
            .to_string()
        } else {
            t!("conn.edit_connection_title", name = self.draft.name).to_string()
        };
        let body: Vec<AnyElement> = match self.tab {
            Tab_::General => self.render_general(cx),
            Tab_::Advanced => self.render_advanced(cx),
            Tab_::Databases => self.render_databases(cx),
            Tab_::Ssl => self.render_ssl(cx),
            Tab_::Ssh => self.render_ssh(cx),
            Tab_::Remark => vec![
                div()
                    .w_full()
                    .child(
                        Textarea::new(&self.remark)
                            .w_full()
                            .h(tokens::form::LIST_HEIGHT.rems()),
                    )
                    .into_any_element(),
            ],
        };
        let testing = self.testing.is_some();
        v_flex()
            .gap_3()
            .child(navidog_ui::skin::heading(title))
            .child(
                h_flex().justify_center().child(
                    TabBar::new("conn-tabs")
                        .segmented()
                        .selected_index(selected)
                        .children(tabs.iter().map(|t| Tab::new().label(t.label())))
                        .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                            this.tab = tabs[*ix];
                            cx.notify();
                        })),
                ),
            )
            .child(
                v_flex()
                    .min_h(tokens::conn_form::BODY_HEIGHT.rems())
                    .px(tokens::form::MARGIN.rems())
                    .gap(tokens::form::ROW_GAP.rems())
                    .children(body),
            )
            .when_some(self.error.clone(), |this, e| {
                this.child(div().text_color(palette(cx).danger).child(e))
            })
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        Button::new("test")
                            .label(
                                if testing {
                                    t!("conn.testing")
                                } else {
                                    t!("conn.test")
                                }
                                .to_string(),
                            )
                            .loading(testing)
                            .on_click(cx.listener(|this, _, window, cx| this.test(window, cx))),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("cancel")
                                    .label(t!("conn.cancel").to_string())
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("ok")
                                    .primary()
                                    .label(t!("conn.ok").to_string())
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.save(window, cx)),
                                    ),
                            ),
                    ),
            )
    }
}
