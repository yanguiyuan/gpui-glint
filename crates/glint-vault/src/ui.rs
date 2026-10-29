use crate::browser::{self, Bridge, Pending, Reply};
use crate::store::{Entry, Vault, VaultStore, generate_password};
use gpui::{
    App, ClipboardItem, Context, Entity, FocusHandle, FontWeight, Render, SharedString,
    Subscription, Task, Window, div, prelude::*, px, rgb,
};
use gpui_glint::{
    components::{
        Button, ButtonKind, ButtonSize, Dialog, Icon, IconButton, InputField, Popover, SearchInput,
        Switch, TextEntry, Toast, ToastHost, ToastKind,
    },
    theme::ThemePreset,
};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

/// Navigation event with no credential data. Hosts should show this page to confirm.
pub enum VaultEvent {
    BrowserFillRequested,
}

/// Embeddable page. The host owns navigation; this entity owns all decrypted data.
/// Unlock lasts until explicit locking or application exit; hiding preserves it.
pub struct VaultPage {
    store: VaultStore,
    toasts: Entity<ToastHost>,
    vault: Option<Vault>,
    initialized: bool,
    storage_ok: bool,
    visible: bool,
    browser_focus: FocusHandle,
    theme: ThemePreset,
    master: Entity<TextEntry>,
    confirmation: Entity<TextEntry>,
    search: Entity<TextEntry>,
    fields: Vec<Entity<TextEntry>>,
    selected: Option<String>,
    editing: bool,
    revealed: bool,
    deleting: bool,
    notice: Option<String>,
    clipboard: Option<([u8; 32], Instant)>,
    bridge: Option<Bridge>,
    browser_pending: Option<(Pending, Instant)>,
    browser_auto_approve: bool,
    browser_match_website: bool,
    last_refresh: Instant,
    unlock_task: Option<Task<()>>,
    _timer: Task<()>,
    _subscriptions: Vec<Subscription>,
}
impl gpui::EventEmitter<VaultEvent> for VaultPage {}
impl VaultPage {
    pub fn new(path: PathBuf, theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let store = VaultStore::new(path);
        let initialized = store.initialized();
        let browser_auto_approve = store.browser_auto_approve().unwrap_or(false);
        let browser_match_website = store.browser_match_website().unwrap_or(true);
        let master = cx.new(|cx| TextEntry::new(cx, "主密码", "").masked(true));
        let confirmation = cx.new(|cx| TextEntry::new(cx, "再次输入主密码", "").masked(true));
        let search = cx.new(|cx| TextEntry::new(cx, "搜索名称、账号或网站", ""));
        let fields: Vec<_> = [
            "例如：工作邮箱",
            "账号或邮箱",
            "密码",
            "https://example.com",
            "备注",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, placeholder)| {
            cx.new(|cx| TextEntry::new(cx, placeholder, "").masked(index == 2))
        })
        .collect();
        let mut subscriptions = Vec::new();
        for field in fields.iter().chain([&master, &confirmation, &search]) {
            subscriptions.push(cx.observe(field, |_, _, cx| cx.notify()));
        }
        subscriptions.push(cx.on_release(|this, cx| {
            this.clear_clipboard(cx);
            this.vault = None;
            this.bridge = None;
            this.browser_pending = None;
        }));
        subscriptions.push(cx.on_app_quit(|this, cx| {
            this.lock(cx);
            async {}
        }));
        let timer = cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if weak
                    .update(cx, |this, cx| {
                        this.browser_tick(cx);
                        this.refresh_shared(cx);
                        if this
                            .clipboard
                            .as_ref()
                            .is_some_and(|(_, at)| at.elapsed() >= Duration::from_secs(30))
                        {
                            this.clear_clipboard(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut page = Self {
            toasts: cx.new(|_| ToastHost::new().theme(theme)),
            store,
            vault: None,
            initialized: initialized.as_ref().copied().unwrap_or(false),
            storage_ok: initialized.is_ok(),
            visible: true,
            browser_focus: cx.focus_handle(),
            theme,
            master,
            confirmation,
            search,
            fields,
            selected: None,
            editing: false,
            revealed: false,
            deleting: false,
            notice: initialized.err(),
            clipboard: None,
            bridge: None,
            browser_pending: None,
            browser_auto_approve,
            browser_match_website,
            last_refresh: Instant::now(),
            unlock_task: None,
            _timer: timer,
            _subscriptions: subscriptions,
        };
        page.apply_theme(cx);
        page
    }
    /// Call when host navigation/theme changes; navigation preserves this unlock session.
    pub fn set_host_state(&mut self, visible: bool, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.visible != visible {
            self.visible = visible;
            if !visible {
                self.reply_browser(false, cx);
                self.revealed = false;
                self.fields[2].update(cx, |field, cx| field.set_masked(true, cx));
            } else {
                match self.store.initialized() {
                    Ok(initialized) => self.initialized = initialized,
                    Err(error) => {
                        self.storage_ok = false;
                        self.notice = Some(error);
                    }
                }
                if self.vault.is_some() {
                    self.connect_browser(cx);
                }
                cx.notify();
            }
        }
        if self.theme != theme {
            self.theme = theme;
            self.apply_theme(cx);
            cx.notify();
        }
    }
    fn apply_theme(&mut self, cx: &mut Context<Self>) {
        self.toasts
            .update(cx, |host, cx| host.set_theme(self.theme, cx));
        for field in self
            .fields
            .iter()
            .chain([&self.master, &self.confirmation, &self.search])
        {
            field.update(cx, |field, cx| field.set_theme(self.theme, cx));
        }
    }
    fn clear_clipboard(&mut self, cx: &mut App) {
        if let Some((hash, _)) = self.clipboard.take() {
            if cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .is_some_and(|text| <[u8; 32]>::from(Sha256::digest(text.as_bytes())) == hash)
            {
                cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
            }
        }
    }
    pub fn lock(&mut self, cx: &mut Context<Self>) {
        self.unlock_task = None;
        self.reply_browser(false, cx);
        self.bridge = None;
        self.vault = None;
        self.clear_clipboard(cx);
        self.toasts.update(cx, |host, cx| host.clear(cx));
        self.selected = None;
        self.editing = false;
        self.revealed = false;
        self.deleting = false;
        self.notice = None;
        for field in self
            .fields
            .iter()
            .chain([&self.master, &self.confirmation, &self.search])
        {
            field.update(cx, |field, cx| field.set_value("", cx));
        }
        self.fields[2].update(cx, |field, cx| field.set_masked(true, cx));
        cx.notify();
    }
    fn unlock(&mut self, cx: &mut Context<Self>) {
        if self.unlock_task.is_some() || !self.storage_ok {
            return;
        }
        let password = Zeroizing::new(self.master.read(cx).value().to_string());
        let create = !self.initialized;
        if create && password.as_str() != self.confirmation.read(cx).value() {
            self.notice = Some("两次输入的主密码不一致".into());
            cx.notify();
            return;
        }
        let store = self.store.clone();
        self.master.update(cx, |entry, cx| entry.set_value("", cx));
        self.confirmation
            .update(cx, |entry, cx| entry.set_value("", cx));
        let work = cx.background_executor().spawn(async move {
            if create {
                store.create(&password)
            } else {
                store.unlock(&password)
            }
        });
        self.unlock_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.unlock_task.take() {
                    task.detach();
                }
                match result {
                    Ok(vault) => {
                        this.vault = Some(vault);
                        this.notice = None;
                        match Bridge::start(this.store.path()) {
                            Ok(bridge) => this.bridge = Some(bridge),
                            Err(error) => this.notice = Some(error),
                        }
                        this.initialized = true;
                    }
                    Err(error) => {
                        this.notice = Some(error);
                        if let Ok(exists) = this.store.initialized() {
                            this.initialized = exists;
                        }
                    }
                }
                cx.notify();
            });
        }));
        self.notice = None;
        cx.notify();
    }
    fn draft(&self, cx: &App) -> Entry {
        Entry {
            id: self.selected.clone().unwrap_or_default(),
            title: self.fields[0].read(cx).value().to_string(),
            username: self.fields[1].read(cx).value().to_string(),
            password: self.fields[2].read(cx).value().to_string(),
            website: self.fields[3].read(cx).value().to_string(),
            notes: self.fields[4].read(cx).value().to_string(),
            updated_at: String::new(),
        }
    }
    fn dirty(&self, cx: &App) -> bool {
        if !self.editing {
            return false;
        }
        let draft = self.draft(cx);
        let previous = self.vault.as_ref().and_then(|vault| {
            vault
                .entries()
                .iter()
                .find(|entry| Some(&entry.id) == self.selected.as_ref())
        });
        if let Some(previous) = previous {
            draft.title != previous.title
                || draft.username != previous.username
                || draft.password != previous.password
                || draft.website != previous.website
                || draft.notes != previous.notes
        } else {
            !draft.title.is_empty()
                || !draft.username.is_empty()
                || !draft.password.is_empty()
                || !draft.website.is_empty()
                || !draft.notes.is_empty()
        }
    }
    fn select(&mut self, id: Option<String>, discard: bool, cx: &mut Context<Self>) {
        if !discard && self.dirty(cx) {
            self.notice = Some("请先保存或取消当前修改".into());
            cx.notify();
            return;
        }
        let entry = id
            .as_ref()
            .and_then(|id| {
                self.vault
                    .as_ref()?
                    .entries()
                    .iter()
                    .find(|entry| &entry.id == id)
            })
            .cloned()
            .unwrap_or_default();
        for (field, value) in self.fields.iter().zip([
            &entry.title,
            &entry.username,
            &entry.password,
            &entry.website,
            &entry.notes,
        ]) {
            field.update(cx, |field, cx| field.set_value(value.clone(), cx));
        }
        self.selected = id;
        self.editing = true;

        self.revealed = false;
        self.deleting = false;
        self.notice = None;
        self.fields[2].update(cx, |field, cx| field.set_masked(true, cx));
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        let draft = self.draft(cx);
        if let Some(vault) = self.vault.as_mut() {
            match vault.upsert(draft) {
                Ok(id) => {
                    self.selected = Some(id);
                    self.notice = None;
                    self.toasts.update(cx, |host, cx| {
                        host.push(
                            Toast::new("已保存")
                                .kind(ToastKind::Success)
                                .duration(Some(Duration::from_secs(3))),
                            cx,
                        )
                    });
                }
                Err(error) => self.notice = Some(error),
            }
        }

        cx.notify();
    }
    fn copy_password(&mut self, cx: &mut Context<Self>) {
        let password = Zeroizing::new(self.fields[2].read(cx).value().to_string());
        if !password.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(password.to_string()));
            self.clipboard = Some((Sha256::digest(password.as_bytes()).into(), Instant::now()));
            self.toasts.update(cx, |host, cx| {
                host.push(
                    Toast::new("已复制密码")
                        .description("30 秒后清除剪贴板")
                        .kind(ToastKind::Success)
                        .duration(Some(Duration::from_secs(3))),
                    cx,
                )
            });

            cx.notify();
        }
    }
    fn refresh_shared(&mut self, cx: &mut Context<Self>) {
        if self.last_refresh.elapsed() < Duration::from_secs(2)
            || self.dirty(cx)
            || self.browser_pending.is_some()
        {
            return;
        }
        self.last_refresh = Instant::now();
        let browser_match_website = self.store.browser_match_website().unwrap_or(true);
        if self.browser_match_website != browser_match_website {
            self.browser_match_website = browser_match_website;
            cx.notify();
        }
        let browser_auto_approve = self.store.browser_auto_approve().unwrap_or(false);
        if self.browser_auto_approve != browser_auto_approve {
            self.browser_auto_approve = browser_auto_approve;
            cx.notify();
        }
        let result = self.vault.as_mut().map(Vault::refresh);
        match result {
            Some(Ok(true)) => {
                let selected = self.selected.clone();
                if selected.as_ref().is_some_and(|id| {
                    self.vault
                        .as_ref()
                        .unwrap()
                        .entries()
                        .iter()
                        .any(|entry| &entry.id == id)
                }) {
                    self.select(selected, true, cx);
                } else if selected.is_some() {
                    self.select(None, true, cx);
                    self.editing = false;
                }

                cx.notify();
            }
            Some(Err(error)) => {
                self.lock(cx);
                self.notice = Some(error);
                cx.notify();
            }
            _ => {}
        }
    }
    fn connect_browser(&mut self, cx: &mut Context<Self>) {
        if self.vault.is_none() {
            return;
        }
        match Bridge::start(self.store.path()) {
            Ok(bridge) => {
                self.bridge = Some(bridge);
            }
            Err(error) => self.notice = Some(error),
        }
        cx.notify();
    }
    fn browser_tick(&mut self, cx: &mut Context<Self>) {
        if self
            .browser_pending
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= Duration::from_secs(55))
        {
            self.reply_browser(false, cx);
        }
        let requests: Vec<_> = self
            .bridge
            .as_ref()
            .map(|bridge| bridge.pending.try_iter().take(8).collect())
            .unwrap_or_default();
        for pending in requests {
            self.handle_browser_request(pending, cx);
        }
    }
    fn handle_browser_request(&mut self, pending: Pending, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.as_ref() else {
            let _ = pending.respond.send(Reply::error("密码箱已锁定"));
            return;
        };
        let match_website = self.store.browser_match_website().unwrap_or(true);
        if pending.request.op == "list" {
            let offset = pending.request.offset.unwrap_or(0).min(10000);
            let entries: Vec<_> = vault
                .entries()
                .iter()
                .filter(|entry| {
                    !match_website || browser::matches(&entry.website, &pending.request.origin)
                })
                .skip(offset)
                .take(51)
                .collect();
            let next_offset = (entries.len() > 50).then_some(offset + 50);
            let matches: Vec<_> = entries.iter().take(50).map(|entry| serde_json::json!({"id":entry.id,"title":entry.title,"username":entry.username})).collect();
            let _ = pending
                .respond
                .send(Reply(serde_json::json!({"ok":true,"entries":matches,"next_offset":next_offset,"match_website":match_website})));
        } else if self.browser_pending.is_some() {
            let _ = pending.respond.send(Reply::error("请先处理当前填充请求"));
        } else if vault.entries().iter().any(|entry| {
            Some(&entry.id) == pending.request.entry_id.as_ref()
                && (!match_website || browser::matches(&entry.website, &pending.request.origin))
        }) {
            self.browser_pending = Some((pending, Instant::now()));
            // Read the shared preference for every request so disabling it
            // in another host takes effect without relying on the UI cache.
            if self.store.browser_auto_approve().unwrap_or(false) {
                self.reply_browser(true, cx);
                return;
            }
            cx.emit(VaultEvent::BrowserFillRequested);
            cx.activate(true);
            cx.notify();
        } else {
            let _ = pending.respond.send(Reply::error("该条目不属于当前网站"));
        }
    }
    fn set_browser_auto_approve(&mut self, enabled: bool, cx: &mut Context<Self>) {
        match self.store.set_browser_auto_approve(enabled) {
            Ok(()) => self.browser_auto_approve = enabled,
            Err(error) => self.notice = Some(error),
        }
        cx.notify();
    }
    fn browser_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let weak = cx.entity().downgrade();
        let match_owner = weak.clone();
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_3()
            .child(
                Switch::new("浏览器操作免二次确认")
                    .checked(self.browser_auto_approve)
                    .theme(self.theme)
                    .on_change(move |enabled, _, cx| {
                        let _ =
                            weak.update(cx, |this, cx| this.set_browser_auto_approve(enabled, cx));
                    })
                    .render()
                    .debug_selector(|| "vault-browser-auto-approve".into()),
            )
            .child(
                Switch::new("按当前网址匹配账号")
                    .checked(self.browser_match_website)
                    .theme(self.theme)
                    .on_change(move |enabled, _, cx| {
                        let _ = match_owner.update(cx, |this, cx| {
                            match this.store.set_browser_match_website(enabled) {
                                Ok(()) => this.browser_match_website = enabled,
                                Err(error) => this.notice = Some(error),
                            }
                            cx.notify();
                        });
                    })
                    .render()
                    .debug_selector(|| "vault-browser-match-website".into()),
            )
    }
    fn reply_browser(&mut self, approved: bool, cx: &mut Context<Self>) {
        let Some((pending, _)) = self.browser_pending.take() else {
            return;
        };
        let reply = if approved {
            let match_website = self.store.browser_match_website().unwrap_or(true);
            match self.vault.as_ref().and_then(|vault| {
                vault.entries().iter().find(|entry| {
                    Some(&entry.id) == pending.request.entry_id.as_ref()
                        && (!match_website
                            || browser::matches(&entry.website, &pending.request.origin))
                })
            }) {
                Some(entry) if pending.request.op == "copy" => {
                    cx.write_to_clipboard(ClipboardItem::new_string(entry.password.clone()));
                    self.clipboard = Some((
                        Sha256::digest(entry.password.as_bytes()).into(),
                        Instant::now(),
                    ));
                    Reply(serde_json::json!({"ok":true,"copied":true}))
                }
                Some(entry) => Reply(
                    serde_json::json!({"ok":true,"origin":pending.request.origin,"username":entry.username,"password":entry.password}),
                ),
                None => Reply::error("密码箱已锁定或条目已改变"),
            }
        } else {
            Reply::error("已取消操作")
        };
        let _ = pending.respond.send(reply);

        cx.notify();
    }
    fn browser_dialog(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let (pending, _) = self.browser_pending.as_ref()?;
        let copying = pending.request.op == "copy";
        let title = self
            .vault
            .as_ref()?
            .entries()
            .iter()
            .find(|entry| Some(&entry.id) == pending.request.entry_id.as_ref())?
            .title
            .clone();
        let weak = cx.entity().downgrade();
        Some(
            Dialog::new(
                "vault-browser-confirm",
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if copying {
                                "允许复制密码？"
                            } else {
                                "允许浏览器填充密码？"
                            }),
                    )
                    .child(pending.request.origin.clone())
                    .child(title)
                    .child(
                        div()
                            .text_color(rgb(self.theme.palette().text_muted))
                            .child(if copying {
                                "将所选账号的密码复制到系统剪贴板，随后可手动粘贴。"
                            } else {
                                "仅将所选账号交给这个网站，不会自动提交登录。"
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                Button::new(if copying {
                                    "允许本次复制"
                                } else {
                                    "允许本次填充"
                                })
                                .theme(self.theme)
                                .render()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.reply_browser(true, cx)),
                                ),
                            )
                            .child(
                                Button::new("取消")
                                    .kind(ButtonKind::Gray)
                                    .theme(self.theme)
                                    .render()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.reply_browser(false, cx)),
                                    ),
                            ),
                    ),
            )
            .theme(self.theme)
            .on_close(move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| this.reply_browser(false, cx));
            })
            .render(),
        )
    }

    fn field(&self, index: usize, label: &str) -> gpui::Div {
        InputField::new("")
            .label(SharedString::from(label.to_owned()))
            .control(self.fields[index].clone())
            .theme(self.theme)
            .render()
            .w_full()
    }
    fn locked_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let busy = self.unlock_task.is_some();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .child(
                div()
                    .w_full()
                    .max_w(px(390.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(Icon::Key.render(32.).text_color(rgb(p.accent)))
                    .child(
                        div()
                            .text_size(px(25.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if self.initialized {
                                "解锁密码箱"
                            } else {
                                "创建你的密码箱"
                            }),
                    )
                    .child(
                        div()
                            .text_color(rgb(p.text_muted))
                            .child(if self.initialized {
                                "输入主密码，查看保存在本机的账号。"
                            } else {
                                "请妥善保管主密码，遗忘后无法恢复。"
                            }),
                    )
                    .child(
                        InputField::new("")
                            .label(if self.initialized {
                                "主密码"
                            } else {
                                "主密码 · 至少 12 个字符"
                            })
                            .control(self.master.clone())
                            .theme(self.theme)
                            .render()
                            .w_full(),
                    )
                    .when(!self.initialized, |page| {
                        page.child(
                            InputField::new("")
                                .label("确认主密码")
                                .control(self.confirmation.clone())
                                .theme(self.theme)
                                .render()
                                .w_full(),
                        )
                    })
                    .when_some(self.notice.clone(), |page, notice| {
                        page.child(div().text_color(rgb(p.text_muted)).child(notice))
                    })
                    .child(
                        Button::new(if busy {
                            "正在解锁…"
                        } else if self.initialized {
                            "解锁"
                        } else {
                            "创建密码箱"
                        })
                        .theme(self.theme)
                        .when(busy || !self.storage_ok, |button| button.disabled())
                        .render()
                        .debug_selector(|| "vault-unlock".into())
                        .on_click(cx.listener(|this, _, _, cx| this.unlock(cx))),
                    ),
            )
    }
}
impl Render for VaultPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.browser_pending.is_some() && !self.browser_focus.is_focused(window) {
            window.focus(&self.browser_focus, cx);
        }
        let p = self.theme.palette();
        let content = if self.vault.is_none() {
            self.locked_page(cx)
        } else {
            let query = self.search.read(cx).value().to_lowercase();
            let entries: Vec<_> = self
                .vault
                .as_ref()
                .unwrap()
                .entries()
                .iter()
                .filter(|entry| {
                    query.is_empty()
                        || entry.title.to_lowercase().contains(&query)
                        || entry.username.to_lowercase().contains(&query)
                        || entry.website.to_lowercase().contains(&query)
                })
                .map(|entry| {
                    (
                        entry.id.clone(),
                        entry.title.clone(),
                        entry.username.clone(),
                    )
                })
                .collect();
            let mut list = div()
                .id("vault-entries")
                .w(px(250.))
                .flex_shrink_0()
                .h_full()
                .overflow_y_scroll()
                .border_r_1()
                .border_color(rgb(p.border))
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child(format!("全部条目 · {}", entries.len())),
                )
                .child(
                    SearchInput::new("搜索密码箱")
                        .control(self.search.clone())
                        .width(224.)
                        .theme(self.theme)
                        .render(),
                );
            for (id, title, username) in entries {
                let active = self.selected.as_ref() == Some(&id);
                let selected = id.clone();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("vault-entry:{id}")))
                        .p_3()
                        .rounded(px(8.))
                        .cursor_pointer()
                        .when(active, |row| row.bg(rgb(p.accent_tint)))
                        .hover(|style| style.bg(rgb(p.surface_subtle)))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().font_weight(FontWeight::MEDIUM).child(title))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(username),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select(Some(selected.clone()), false, cx)
                        })),
                );
            }
            let details = if !self.editing {
                div()
                    .id("vault-empty")
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .items_center()
                            .child(Icon::Key.render(28.).text_color(rgb(p.text_muted)))
                            .child("选择一个条目，或添加第一个密码")
                            .child(
                                Button::new("添加密码")
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "vault-add-empty".into())
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.select(None, false, cx)),
                                    ),
                            ),
                    )
                    .into_any_element()
            } else {
                let weak = cx.entity().downgrade();
                let delete_confirmation = div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child("确定删除这个密码条目？")
                    .child(
                        Button::new("删除条目")
                            .kind(ButtonKind::Danger)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let (Some(vault), Some(id)) =
                                    (this.vault.as_mut(), this.selected.clone())
                                {
                                    match vault.delete(&id) {
                                        Ok(()) => {
                                            this.select(None, true, cx);
                                            this.editing = false;
                                        }
                                        Err(error) => this.notice = Some(error),
                                    }
                                }
                                this.deleting = false;
                                cx.notify();
                            })),
                    );
                div()
                    .id("vault-details")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .p_6()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(650.))
                            .mx_auto()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if self.selected.is_some() {
                                        "账号详情"
                                    } else {
                                        "添加密码"
                                    }),
                            )
                            .child(self.field(0, "名称"))
                            .child(self.field(1, "账号"))
                            .child(self.field(2, "密码"))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(
                                        Button::new(if self.revealed {
                                            "隐藏密码"
                                        } else {
                                            "显示密码"
                                        })
                                        .kind(ButtonKind::Gray)
                                        .size(ButtonSize::Small)
                                        .theme(self.theme)
                                        .render()
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.revealed = !this.revealed;
                                                this.fields[2].update(cx, |field, cx| {
                                                    field.set_masked(!this.revealed, cx)
                                                });
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                    .child(
                                        Button::new("复制密码")
                                            .kind(ButtonKind::Gray)
                                            .size(ButtonSize::Small)
                                            .theme(self.theme)
                                            .render()
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.copy_password(cx)
                                                }),
                                            ),
                                    )
                                    .child(
                                        Button::new("生成密码 · 24 位")
                                            .kind(ButtonKind::Plain)
                                            .size(ButtonSize::Small)
                                            .theme(self.theme)
                                            .render()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                match generate_password(24) {
                                                    Ok(password) => {
                                                        this.fields[2].update(cx, |field, cx| {
                                                            field.set_value(
                                                                password.as_str().to_owned(),
                                                                cx,
                                                            )
                                                        })
                                                    }
                                                    Err(error) => this.notice = Some(error),
                                                }
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(self.field(3, "网站"))
                            .child(self.field(4, "备注"))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_3()
                                    .child(
                                        Button::new("保存条目")
                                            .theme(self.theme)
                                            .render()
                                            .debug_selector(|| "vault-save".into())
                                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                                    )
                                    .child(
                                        Button::new("取消修改")
                                            .kind(ButtonKind::Gray)
                                            .theme(self.theme)
                                            .render()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.select(this.selected.clone(), true, cx)
                                            })),
                                    )
                                    .when(self.selected.is_some(), |row| {
                                        row.child(
                                            Popover::new(
                                                "vault-delete",
                                                Button::new("删除")
                                                    .kind(ButtonKind::Plain)
                                                    .theme(self.theme)
                                                    .render(),
                                                delete_confirmation,
                                            )
                                            .open(self.deleting)
                                            .theme(self.theme)
                                            .on_open_change(move |open, _, cx| {
                                                let _ = weak.update(cx, |this, cx| {
                                                    this.deleting = open;
                                                    cx.notify();
                                                });
                                            }),
                                        )
                                    }),
                            ),
                    )
                    .into_any_element()
            };
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("vault-toolbar")
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .p_4()
                        .border_b_1()
                        .border_color(rgb(p.border))
                        .child(Icon::Key.render(20.).text_color(rgb(p.accent)))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("密码箱"))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child("本机加密 · 本次运行已解锁"),
                        )
                        .child(div().flex_1())
                        .child(self.browser_settings(cx))
                        .child(
                            Button::new("添加密码")
                                .kind(ButtonKind::Gray)
                                .theme(self.theme)
                                .render()
                                .debug_selector(|| "vault-add-toolbar".into())
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.select(None, false, cx)),
                                ),
                        )
                        .child(
                            IconButton::new(Icon::Refresh, "让浏览器连接这个窗口")
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| this.connect_browser(cx))),
                        )
                        .child(
                            IconButton::new(Icon::Shield, "立即锁定")
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| this.lock(cx))),
                        ),
                )
                .when_some(self.notice.clone(), |page, notice| {
                    page.child(div().px_4().py_2().bg(rgb(p.surface_subtle)).child(notice))
                })
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .child(list)
                        .child(div().flex_1().min_w_0().child(details)),
                )
        };
        div()
            .relative()
            .track_focus(&self.browser_focus)
            .size_full()
            .bg(rgb(p.panel))
            .text_color(rgb(p.text))
            .text_size(px(13.))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if this.browser_pending.is_some() && event.keystroke.key == "escape" {
                    this.reply_browser(false, cx);
                    cx.stop_propagation();
                }
            }))
            .child(content)
            .child(self.toasts.clone())
            .children(self.browser_dialog(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn browser_lists_all_entries_by_default_and_matching_is_optional(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.sqlite3");
        let store = VaultStore::new(path.clone());
        let mut vault = store.create("a safe test master password").unwrap();
        assert!(!store.browser_match_website().unwrap());
        let mut first = String::new();
        for index in 0..52 {
            let mut entry = Entry::default();
            entry.title = format!("Entry {index}");
            entry.website = "https://example.com/login".into();
            entry.password = "test-password".into();
            let id = vault.upsert(entry).unwrap();
            if index == 0 {
                first = id;
            }
        }
        store.set_browser_auto_approve(true).unwrap();
        let (page, window) =
            cx.add_window_view(|_, cx| VaultPage::new(path.clone(), ThemePreset::Moonstone, cx));
        page.update(window, |this, cx| {
            this.vault = Some(vault);
            let request = |op: &str, offset: Option<usize>| {
                let (respond, response) = std::sync::mpsc::sync_channel(1);
                (
                    Pending {
                        request: browser::Request {
                            op: op.into(),
                            origin: "https://other.test".into(),
                            entry_id: Some(first.clone()),
                            offset,
                        },
                        respond,
                    },
                    response,
                )
            };
            for (offset, count) in [(None, 50), (Some(50), 2)] {
                let (pending, response) = request("list", offset);
                this.handle_browser_request(pending, cx);
                let reply = response.try_recv().unwrap();
                assert_eq!(reply.0["entries"].as_array().unwrap().len(), count);
                assert_eq!(reply.0["match_website"], false);
                if offset.is_none() {
                    assert_eq!(reply.0["next_offset"], 50);
                } else {
                    assert!(reply.0["next_offset"].is_null());
                }
                assert!(!reply.0.to_string().contains("test-password"));
            }
            for op in ["fill", "copy"] {
                let (pending, response) = request(op, None);
                this.handle_browser_request(pending, cx);
                assert_eq!(response.try_recv().unwrap().0["ok"], true);
            }
            cx.notify();
        });
        let toggle = window.debug_bounds("vault-browser-match-website").unwrap();
        window.simulate_click(toggle.center(), gpui::Modifiers::default());
        assert!(VaultStore::new(path).browser_match_website().unwrap());
        page.update(window, |this, cx| {
            for op in ["list", "fill", "copy"] {
                let (respond, response) = std::sync::mpsc::sync_channel(1);
                this.handle_browser_request(
                    Pending {
                        request: browser::Request {
                            op: op.into(),
                            origin: "https://other.test".into(),
                            entry_id: Some(first.clone()),
                            offset: None,
                        },
                        respond,
                    },
                    cx,
                );
                let reply = response.try_recv().unwrap();
                if op == "list" {
                    assert!(reply.0["entries"].as_array().unwrap().is_empty());
                } else {
                    assert_eq!(reply.0["ok"], false);
                }
            }
        });
    }
    #[gpui::test]
    fn browser_auto_approval_is_persistent_and_respects_lock_origin_and_opt_out(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.sqlite3");
        let store = VaultStore::new(path.clone());
        let mut vault = store.create("a safe test master password").unwrap();
        store.set_browser_match_website(true).unwrap();
        let mut entry = Entry::default();
        entry.title = "Example".into();
        entry.website = "https://example.com/login".into();
        entry.username = "test-user".into();
        entry.password = "test-password".into();
        let id = vault.upsert(entry).unwrap();
        let (page, window) =
            cx.add_window_view(|_, cx| VaultPage::new(path.clone(), ThemePreset::Moonstone, cx));
        page.update(window, |this, cx| {
            this.vault = Some(vault);
            cx.notify();
        });
        assert!(!store.browser_auto_approve().unwrap());
        let toggle = window.debug_bounds("vault-browser-auto-approve").unwrap();
        window.simulate_click(toggle.center(), gpui::Modifiers::default());
        assert!(VaultStore::new(path).browser_auto_approve().unwrap());
        page.update(window, |this, cx| {
            let request = |op: &str, origin: &str| {
                let (respond, response) = std::sync::mpsc::sync_channel(1);
                (
                    Pending {
                        request: browser::Request {
                            op: op.into(),
                            origin: origin.into(),
                            entry_id: Some(id.clone()),
                            offset: None,
                        },
                        respond,
                    },
                    response,
                )
            };
            for op in ["fill", "copy"] {
                let (pending, response) = request(op, "https://example.com");
                this.handle_browser_request(pending, cx);
                let reply = response.try_recv().unwrap();
                assert_eq!(reply.0["ok"], true);
                assert!(this.browser_pending.is_none());
                if op == "fill" {
                    assert_eq!(reply.0["password"], "test-password");
                } else {
                    assert_eq!(reply.0["copied"], true);
                    assert!(reply.0.get("password").is_none());
                    assert_eq!(
                        cx.read_from_clipboard().unwrap().text().unwrap(),
                        "test-password"
                    );
                }
                let (pending, response) = request(op, "https://other.test");
                this.handle_browser_request(pending, cx);
                assert_eq!(response.try_recv().unwrap().0["ok"], false);
            }
            // Another host disables the preference while this view still shows it on.
            assert!(this.browser_auto_approve);
            store.set_browser_auto_approve(false).unwrap();
            for op in ["fill", "copy"] {
                let (pending, response) = request(op, "https://example.com");
                this.handle_browser_request(pending, cx);
                assert!(response.try_recv().is_err());
                assert!(this.browser_pending.is_some());
                this.reply_browser(false, cx);
                assert_eq!(response.try_recv().unwrap().0["ok"], false);
            }
            this.set_browser_auto_approve(true, cx);
            this.lock(cx);
            for op in ["fill", "copy"] {
                let (pending, response) = request(op, "https://example.com");
                this.handle_browser_request(pending, cx);
                assert_eq!(response.try_recv().unwrap().0["ok"], false);
                assert!(this.browser_pending.is_none());
            }
        });
    }
    #[gpui::test]
    fn navigation_retains_unlock_explicit_lock_clears_secrets_and_browser_requires_approval(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.sqlite3");
        let store = VaultStore::new(path.clone());
        let mut vault = store.create("a safe test master password").unwrap();
        store.set_browser_match_website(true).unwrap();
        let mut entry = Entry::default();
        entry.title = "Example".into();
        entry.username = "alice".into();
        entry.password = "test-password".into();
        entry.website = "https://example.com/login".into();
        let id = vault.upsert(entry).unwrap();
        let (page, window) =
            cx.add_window_view(|_, cx| VaultPage::new(path, ThemePreset::Moonstone, cx));
        page.update(window, |this, cx| {
            this.vault = Some(vault);
            cx.notify();
        });
        let add = window.debug_bounds("vault-add-empty").unwrap();
        window.simulate_click(add.center(), gpui::Modifiers::default());
        page.read_with(window, |this, _| {
            assert!(this.editing, "empty state add must open the form")
        });
        assert!(window.debug_bounds("vault-save").is_some());
        page.update(window, |this, cx| {
            this.editing = false;
            cx.notify();
        });
        let add = window.debug_bounds("vault-add-toolbar").unwrap();
        window.simulate_click(add.center(), gpui::Modifiers::default());
        page.read_with(window, |this, _| {
            assert!(this.editing, "toolbar add must open the form")
        });
        page.update(window, |this, cx| {
            this.select(Some(id.clone()), true, cx);
            this.save(cx);
            assert!(
                this.notice.is_none(),
                "successful save must not add an inline banner"
            );
            assert!(this.toasts.read(cx).toasts().any(
                |toast| toast.message.as_ref() == "已保存" && toast.kind == ToastKind::Success
            ));
            let (respond, response) = std::sync::mpsc::sync_channel(1);
            this.browser_pending = Some((
                Pending {
                    request: browser::Request {
                        op: "fill".into(),
                        origin: "https://example.com".into(),
                        entry_id: Some(id.clone()),
                        offset: None,
                    },
                    respond,
                },
                Instant::now(),
            ));
            this.reply_browser(false, cx);
            assert_eq!(response.recv().unwrap().0["ok"], false);
            let (respond, response) = std::sync::mpsc::sync_channel(1);
            this.browser_pending = Some((
                Pending {
                    request: browser::Request {
                        op: "fill".into(),
                        origin: "https://example.com".into(),
                        entry_id: Some(id.clone()),
                        offset: None,
                    },
                    respond,
                },
                Instant::now(),
            ));
            assert!(response.try_recv().is_err());
            this.reply_browser(true, cx);
            let reply = response.recv().unwrap();
            assert_eq!(reply.0["password"], "test-password");
            // Browser copy is approved locally, and only returns a status to
            // the extension. It uses the existing clipboard expiry/lock cleanup.
            cx.write_to_clipboard(ClipboardItem::new_string("unchanged".into()));
            for approved in [false, true] {
                let (respond, response) = std::sync::mpsc::sync_channel(1);
                this.browser_pending = Some((
                    Pending {
                        request: browser::Request {
                            op: "copy".into(),
                            origin: "https://example.com".into(),
                            entry_id: Some(id.clone()),
                            offset: None,
                        },
                        respond,
                    },
                    Instant::now(),
                ));
                assert!(response.try_recv().is_err());
                this.reply_browser(approved, cx);
                let reply = response.recv().unwrap();
                assert_eq!(reply.0["ok"], approved);
                assert!(reply.0.get("password").is_none());
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().unwrap(),
                    if approved {
                        "test-password"
                    } else {
                        "unchanged"
                    }
                );
                if approved {
                    assert_eq!(reply.0["copied"], true);
                    assert!(this.clipboard.is_some());
                }
            }
            let (respond, response) = std::sync::mpsc::sync_channel(1);
            this.browser_pending = Some((
                Pending {
                    request: browser::Request {
                        op: "fill".into(),
                        origin: "https://example.com.evil.test".into(),
                        entry_id: Some(id),
                        offset: None,
                    },
                    respond,
                },
                Instant::now(),
            ));
            this.reply_browser(true, cx);
            assert_eq!(response.recv().unwrap().0["ok"], false);
            this.copy_password(cx);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                "test-password"
            );
            this.set_host_state(false, ThemePreset::Moonstone, cx);
            assert!(
                this.vault.is_some(),
                "navigation must preserve the unlock session"
            );
            assert_eq!(this.fields[2].read(cx).value(), "test-password");
            this.set_host_state(true, ThemePreset::Moonstone, cx);
            assert!(this.vault.is_some());
            assert_eq!(this.fields[2].read(cx).value(), "test-password");
            this.lock(cx);
            assert!(this.vault.is_none());
            assert!(this.bridge.is_none());
            assert!(this.selected.is_none());
            for field in &this.fields {
                assert!(field.read(cx).value().is_empty());
            }
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap_or_default(),
                ""
            );
        });
    }
}
