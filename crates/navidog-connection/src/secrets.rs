//! Resolves secret fields before opening a connection: unlock the master password
//! first when one is set, and ask for any password that was not saved (F-CONN-020).

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Window, div,
};
use navidog_core::connection::{Connection, SecretField};
use navidog_core::store::{ConnectionStore, StoreError};
use navidog_db::Secrets;
use navidog_ui::skin::{FormRow, heading};
use navidog_ui::{palette, tokens};
use rust_i18n::t;

type Then = Rc<RefCell<Option<Box<dyn FnOnce(Secrets, &mut Window, &mut App)>>>>;

/// After this many failed unlocks in one run, stop asking (spec §7).
const MAX_UNLOCK_FAILURES: u8 = 3;

pub fn resolve(
    id: String,
    window: &mut Window,
    cx: &mut App,
    then: impl FnOnce(Secrets, &mut Window, &mut App) + 'static,
) {
    let then: Then = Rc::new(RefCell::new(Some(Box::new(then))));
    resolve_inner(id, then, window, cx);
}

fn resolve_inner(id: String, then: Then, window: &mut Window, cx: &mut App) {
    let store = ConnectionStore::global(cx);
    let Some(conn) = store.read(cx).get(&id).cloned() else {
        return;
    };

    let mut secrets = Secrets::default();
    let mut missing = Vec::new();
    for field in needed_fields(&conn) {
        let saved = saved_flag(&conn, field);
        let value = if saved {
            match store.update(cx, |s, _| s.reveal(&id, field)) {
                Ok(v) => v,
                Err(StoreError::Locked) => {
                    if store.read(cx).unlock_failures < MAX_UNLOCK_FAILURES {
                        open_unlock(id, then, window, cx);
                        return;
                    }
                    missing.push(field);
                    continue;
                }
                Err(e) => {
                    tracing::warn!("秘密字段无法解密，视为未保存: {e}");
                    missing.push(field);
                    continue;
                }
            }
        } else {
            missing.push(field);
            continue;
        };
        set(&mut secrets, field, value);
    }

    if missing.is_empty() {
        if let Some(f) = then.borrow_mut().take() {
            f(secrets, window, cx);
        }
        return;
    }
    let view = cx.new(|cx| AskPassword::new(conn, missing, secrets, then, window, cx));
    let first = view.read(cx).inputs.first().map(|(_, input)| input.clone());
    window.open_dialog(cx, {
        let view = view.clone();
        move |dialog, window, _| {
            dialog
                .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
                .close_button(false)
                .on_ok({
                    let view = view.clone();
                    move |_, window, cx| {
                        view.update(cx, |this, cx| this.confirm(window, cx));
                        false
                    }
                })
                .child(view.clone())
        }
    });
    if let Some(input) = first {
        window.defer(cx, move |window, cx| {
            input.update(cx, |state, cx| state.focus(window, cx));
        });
    }
}

fn needed_fields(conn: &Connection) -> Vec<SecretField> {
    let mut fields = Vec::new();
    // SQLite only needs a password for an encrypted database. M1 does not support
    // that, so the field is read only when one was saved.
    if conn.supports_network() || conn.general.password.is_some() {
        fields.push(SecretField::Password);
    }
    if conn.ssh.enabled && conn.supports_network() {
        if conn.ssh.auth.uses_password() {
            fields.push(SecretField::SshPassword);
        }
        if conn.ssh.auth.uses_key() {
            fields.push(SecretField::SshPassphrase);
        }
    }
    fields
}

fn saved_flag(conn: &Connection, field: SecretField) -> bool {
    match field {
        SecretField::Password => conn.general.save_password || !conn.supports_network(),
        SecretField::SshPassword => conn.ssh.save_password,
        SecretField::SshPassphrase => conn.ssh.save_passphrase,
    }
}

fn set(secrets: &mut Secrets, field: SecretField, value: Option<String>) {
    match field {
        SecretField::Password => secrets.password = value,
        SecretField::SshPassword => secrets.ssh_password = value,
        SecretField::SshPassphrase => secrets.ssh_passphrase = value,
    }
}

struct AskPassword {
    conn: Connection,
    inputs: Vec<(SecretField, Entity<InputState>)>,
    secrets: Secrets,
    save: bool,
    then: Then,
    _subs: Vec<Subscription>,
}

impl AskPassword {
    fn new(
        conn: Connection,
        missing: Vec<SecretField>,
        secrets: Secrets,
        then: Then,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs: Vec<_> = missing
            .into_iter()
            .map(|f| (f, cx.new(|cx| InputState::new(window, cx).masked(true))))
            .collect();
        let _subs = inputs
            .iter()
            .map(|(_, input)| {
                cx.subscribe_in(
                    input,
                    window,
                    |this: &mut Self, _, e: &InputEvent, window, cx| {
                        if matches!(e, InputEvent::PressEnter { shift: false, .. }) {
                            this.confirm(window, cx);
                        }
                    },
                )
            })
            .collect();
        Self {
            conn,
            inputs,
            secrets,
            save: false,
            then,
            _subs,
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut secrets = std::mem::take(&mut self.secrets);
        let mut to_save = Vec::new();
        for (field, input) in &self.inputs {
            let value = input.read(cx).value().to_string();
            to_save.push((*field, Some(value.clone())));
            set(&mut secrets, *field, Some(value).filter(|v| !v.is_empty()));
        }
        if self.save {
            let mut conn = self.conn.clone();
            for (field, _) in &self.inputs {
                match field {
                    SecretField::Password => conn.general.save_password = true,
                    SecretField::SshPassword => conn.ssh.save_password = true,
                    SecretField::SshPassphrase => conn.ssh.save_passphrase = true,
                }
            }
            let store = ConnectionStore::global(cx);
            if let Err(e) = store.update(cx, |s, cx| s.upsert(conn, &to_save, cx)) {
                tracing::warn!("保存密码失败: {e}");
            }
        }
        window.close_dialog(cx);
        if let Some(f) = self.then.borrow_mut().take() {
            f(secrets, window, cx);
        }
    }
}

impl Render for AskPassword {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = t!("conn.ask_password_title", name = self.conn.name).to_string();
        v_flex()
            .w_full()
            .min_w_0()
            .gap(tokens::form::ROW_GAP.rems())
            .child(heading(title))
            .children(
                self.inputs.iter().map(|(f, input)| {
                    FormRow::new(crate::secret_label(*f), field(input)).hug_label()
                }),
            )
            .child(
                FormRow::bare(
                    Checkbox::new("save")
                        .label(t!("conn.save_password").to_string())
                        .checked(self.save)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.save = *checked;
                            cx.notify();
                        })),
                )
                .hug_label(),
            )
            .child(footer(
                t!("conn.ok").to_string(),
                cx.listener(|this, _, window, cx| this.confirm(window, cx)),
            ))
    }
}

/// Dialog footer: Cancel and the primary button, aligned to the right.
pub(crate) fn footer(
    ok: String,
    on_ok: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex()
        .justify_end()
        .gap_2()
        .pt_2()
        .child(
            Button::new("cancel")
                .label(t!("conn.cancel").to_string())
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .child(Button::new("ok").primary().label(ok).on_click(on_ok))
}

struct Unlock {
    id: String,
    input: Entity<InputState>,
    error: Option<SharedString>,
    then: Then,
    _sub: Subscription,
}

fn open_unlock(id: String, then: Then, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| {
        let input = cx.new(|cx| InputState::new(window, cx).masked(true));
        let _sub = cx.subscribe_in(
            &input,
            window,
            |this: &mut Unlock, _, e: &InputEvent, window, cx| {
                // A single-line input propagates Enter. The dialog also binds Enter
                // to Confirm, which closes unless `on_ok` handles it. This covers
                // the key while the field itself is focused.
                if matches!(e, InputEvent::PressEnter { shift: false, .. }) {
                    this.confirm(window, cx);
                }
            },
        );
        Unlock {
            id,
            input,
            error: None,
            then,
            _sub,
        }
    });
    let input = view.read(cx).input.clone();
    window.open_dialog(cx, {
        let view = view.clone();
        move |dialog, window, _| {
            dialog
                .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
                .close_button(false)
                .on_ok({
                    let view = view.clone();
                    move |_, window, cx| {
                        view.update(cx, |this, cx| this.confirm(window, cx));
                        // `confirm` closes the dialog on success. Returning true here
                        // would close it again and could dismiss whatever opened next.
                        false
                    }
                })
                .child(view.clone())
        }
    });
    // `open_dialog` focuses the dialog itself, so a focus call before that does not stick.
    window.defer(cx, move |window, cx| {
        input.update(cx, |state, cx| state.focus(window, cx));
    });
}

/// A field that fills the dialog instead of the connection form's fixed 300pt width.
fn field(input: &Entity<InputState>) -> impl IntoElement {
    Input::new(input).w_full().min_w_0()
}

impl Unlock {
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.input.read(cx).value().to_string();
        let store = ConnectionStore::global(cx);
        match store.update(cx, |s, _| s.unlock(&password)) {
            Ok(()) => {
                window.close_dialog(cx);
                resolve_inner(self.id.clone(), self.then.clone(), window, cx);
            }
            Err(_) => {
                if store.read(cx).unlock_failures >= MAX_UNLOCK_FAILURES {
                    window.close_dialog(cx);
                    resolve_inner(self.id.clone(), self.then.clone(), window, cx);
                    return;
                }
                self.error = Some(t!("conn.unlock_failed").to_string().into());
                self.input.update(cx, |s, cx| s.set_value("", window, cx));
                cx.notify();
            }
        }
    }
}

impl Render for Unlock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        v_flex()
            .w_full()
            .min_w_0()
            .gap(tokens::form::ROW_GAP.rems())
            .child(heading(t!("conn.unlock_title").to_string()))
            .child(
                div()
                    .text_color(p.label_secondary)
                    .child(t!("conn.unlock_prompt").to_string()),
            )
            .child(
                FormRow::new(t!("conn.master_password").to_string(), field(&self.input))
                    .hug_label(),
            )
            .when_some(self.error.clone(), |this, e| {
                this.child(FormRow::bare(div().text_color(p.danger).child(e)).hug_label())
            })
            .child(footer(
                t!("conn.unlock").to_string(),
                cx.listener(|this, _, window, cx| this.confirm(window, cx)),
            ))
    }
}
