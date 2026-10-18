use super::*;
use glint_plugin_registry::{Client as RegistryClient, Release, Source};
impl Workbench {
    pub(super) fn registry_operation(
        &mut self,
        work: impl FnOnce() -> Result<Vec<(Source, Release)>, String> + Send + 'static,
        refresh_plugins: bool,
        cx: &mut Context<Self>,
    ) {
        if self.registry_task.is_some() {
            return;
        }
        self.registry_message = Some("正在连接插件源…".into());
        self.registry_task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { work() }).await;
            let _ = this.update(cx, |this, cx| {
                this.registry_task = None;
                this.registry_sources = plugins::registry_store()
                    .and_then(|s| s.sources())
                    .unwrap_or_default();
                let adding_source = this.registry_adding_source;
                this.registry_adding_source = false;
                match result {
                    Ok(releases) => {
                        if adding_source {
                            this.show_registry_dialog = false;
                            for entry in [&this.registry_name, &this.registry_url, &this.registry_key] { entry.update(cx, |entry,cx| entry.set_value("",cx)); }
                        }
                        this.registry_releases = releases;
                        this.registry_sources = plugins::registry_store()
                            .and_then(|s| s.sources())
                            .unwrap_or_default();
                        this.registry_message = Some(
                            if refresh_plugins {
                                "插件资源已更新。在请求的“前置动作”中选择并授权版本；已运行请求不受影响。"
                            } else {
                                "已验证插件源签名并更新列表。"
                            }
                            .into(),
                        );
                        if refresh_plugins {
                            (this.plugin_catalog, this.plugin_errors) = plugins::catalog();
                        }
                    }
                    Err(error) => this.registry_message = Some(format!("插件源操作失败：{error}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(super) fn refresh_registry(&mut self, cx: &mut Context<Self>) {
        self.registry_operation(
            || {
                let store = plugins::registry_store()?;
                let client = RegistryClient::new()?;
                let mut releases = vec![];
                for source in store.sources()? {
                    let index = store.refresh(&client, &source)?;
                    for release in index.releases {
                        if release.supports("glint-api", updater::VERSION)
                            && plugins::validate_manifest(&release.manifest).is_ok()
                        {
                            releases.push((source.clone(), release));
                        }
                    }
                }
                Ok(releases)
            },
            false,
            cx,
        );
    }
    pub(super) fn registry_configuration(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div().flex().items_center().gap_2().child("插件源").child(
                            gpui_glint::components::CountBadge::new(
                                self.registry_sources.len().to_string(),
                            )
                            .kind(gpui_glint::components::CountBadgeKind::Muted)
                            .theme(self.theme),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new(if self.registry_task.is_some() {
                                    "处理中…"
                                } else {
                                    "获取插件列表"
                                })
                                .kind(ButtonKind::Plain)
                                .size(ButtonSize::Small)
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| this.refresh_registry(cx))),
                            )
                            .child(
                                Button::new("添加插件源")
                                    .size(ButtonSize::Small)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "open-registry-dialog".into())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.registry_task.is_some() {
                                            return;
                                        }
                                        this.show_registry_dialog = true;
                                        this.registry_message = None;
                                        window.focus(
                                            &this.registry_name.read(cx).focus_handle(cx),
                                            cx,
                                        );
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("从已添加的源获取插件，安装后可在请求中启用动作。"),
            );
        if self.registry_sources.is_empty() {
            panel = panel.child(
                div()
                    .p_4()
                    .rounded(px(8.))
                    .bg(rgb(p.panel))
                    .text_color(rgb(p.text_muted))
                    .child("还没有插件源，添加后即可浏览可用插件。"),
            );
        }
        for source in &self.registry_sources {
            let name = source.name.clone();
            panel = panel.child(
                div()
                    .id(gpui::SharedString::from(format!("registry-source-{name}")))
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .child(Icon::External.render(18.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(source.name.clone())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.text_muted))
                                    .child(source.url.clone()),
                            ),
                    )
                    .child(
                        Button::new("删除源")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.registry_task.is_some() {
                                    return;
                                }
                                match plugins::registry_store()
                                    .and_then(|store| store.remove_source(&name))
                                {
                                    Ok(()) => {
                                        this.registry_sources.retain(|s| s.name != name);
                                        this.registry_releases.retain(|(s, _)| s.name != name);
                                        this.registry_message =
                                            Some("已删除源；已安装和授权的版本保留。".into());
                                    }
                                    Err(error) => this.registry_message = Some(error),
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        let filter = self.registry_filter.read(cx).value().trim().to_lowercase();
        panel = panel.child(self.registry_filter.clone());
        let matches: Vec<_> = self
            .registry_releases
            .iter()
            .filter(|(source, release)| {
                format!(
                    "{} {} {} {}",
                    source.name,
                    release.manifest.name,
                    release.manifest.id,
                    release.manifest.version
                )
                .to_lowercase()
                .contains(&filter)
            })
            .collect();
        if matches.len() > 50 {
            panel = panel.child(div().text_size(px(10.)).child(format!(
                "匹配 {} 个版本，显示前 50 个；请细化筛选。",
                matches.len()
            )));
        }
        for (source, release) in matches.into_iter().take(50) {
            let source = source.clone();
            let release = release.clone();
            let title = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(release.manifest.name.clone())
                .child(
                    gpui_glint::components::CountBadge::new(format!(
                        "v{}",
                        release.manifest.version
                    ))
                    .kind(gpui_glint::components::CountBadgeKind::Muted)
                    .theme(self.theme),
                )
                .child(
                    gpui_glint::components::TagBadge::new(source.name.clone())
                        .kind(gpui_glint::components::TagBadgeKind::Synced)
                        .theme(self.theme),
                );
            let description = release.changelog.clone();
            let permissions = release
                .manifest
                .capabilities
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(" · ");
            panel = panel.child(
                div()
                    .id(gpui::SharedString::from(format!(
                        "remote-{}-{}-{}",
                        source.name, release.manifest.id, release.manifest.version
                    )))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .border_1()
                    .border_color(rgb(p.border))
                    .rounded(px(7.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(title)
                            .child(
                                Button::new("安装此版本")
                                    .kind(ButtonKind::Plain)
                                    .size(ButtonSize::Mini)
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let source = source.clone();
                                        let release = release.clone();
                                        this.registry_operation(
                                            move || {
                                                let store = plugins::registry_store()?;
                                                let client = RegistryClient::new()?;
                                                store.install(
                                                    &client,
                                                    &source,
                                                    &release.manifest.id,
                                                    &release.manifest.version,
                                                    "glint-api",
                                                    updater::VERSION,
                                                )?;
                                                Ok(vec![])
                                            },
                                            true,
                                            cx,
                                        );
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!("申请权限：{permissions}")),
                    )
                    .child(div().text_size(px(11.)).child(description)),
            );
        }
        if let Some(message) = &self.registry_message {
            panel = panel.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(message.clone()),
            );
        }
        panel
    }
}

impl Workbench {
    pub(super) fn registry_dialog(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.show_registry_dialog {
            return None;
        }
        let p = self.theme.palette();
        let busy = self.registry_adding_source;
        let mut form = div()
            .id("registry-source-dialog")
            .debug_selector(|| "registry-source-dialog".into())
            .flex()
            .flex_col()
            .gap_4()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .child("添加插件源"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("填写源地址和可信公钥，验证签名后添加。"),
            );
        for (label, entry) in [
            ("名称", &self.registry_name),
            ("地址", &self.registry_url),
            ("公钥", &self.registry_key),
        ] {
            form = form.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(label)
                    .child(entry.clone()),
            );
        }
        form = form.child(
            div()
                .text_size(px(11.))
                .text_color(rgb(p.text_muted))
                .child("公钥请向插件源管理员确认。发布令牌仅供 CLI 使用，无需填写。"),
        );
        if let Some(message) = self
            .registry_message
            .as_ref()
            .filter(|m| m.starts_with("插件源操作失败"))
        {
            form = form.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(message.clone()),
            );
        }
        form = form.child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("取消")
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "cancel-registry-dialog".into())
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !this.registry_adding_source {
                                this.show_registry_dialog = false;
                                window.focus(&this.focus, cx);
                                cx.notify();
                            }
                        })),
                )
                .child(
                    Button::new(if busy {
                        "验证中…"
                    } else {
                        "验证并添加"
                    })
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "submit-registry-dialog".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.registry_task.is_some() {
                            return;
                        }
                        let source = Source {
                            name: this.registry_name.read(cx).value().trim().into(),
                            url: this.registry_url.read(cx).value().trim().into(),
                            public_key: this.registry_key.read(cx).value().trim().into(),
                            allow_http: false,
                        };
                        if let Err(error) = source.validate() {
                            this.registry_message = Some(format!("插件源操作失败：{error}"));
                            cx.notify();
                            return;
                        }
                        this.registry_adding_source = true;
                        this.registry_operation(
                            move || {
                                let client = RegistryClient::new()?;
                                let signed = client.fetch(&source)?;
                                glint_plugin_registry::verify(
                                    &signed,
                                    &source.public_key,
                                    0,
                                    false,
                                )?;
                                let store = plugins::registry_store()?;
                                store.add_source(source.clone())?;
                                let index = store.refresh(&client, &source)?;
                                Ok(index
                                    .releases
                                    .into_iter()
                                    .filter(|r| r.supports("glint-api", updater::VERSION))
                                    .map(|r| (source.clone(), r))
                                    .collect())
                            },
                            false,
                            cx,
                        );
                    })),
                ),
        );
        Some(
            Dialog::new("registry_ui-modal", form)
                .focus_handles(
                    [&self.registry_name, &self.registry_url, &self.registry_key]
                        .map(|e| e.read(cx).focus_handle(cx)),
                )
                .theme(self.theme)
                .width(480.)
                .priority(110)
                .dismissible(!self.registry_adding_source)
                .on_close(cx.listener(|this, _, window, cx| {
                    this.show_registry_dialog = false;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }))
                .render(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn source_form_is_modal_keeps_drafts_and_validates_before_network(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&dir.path().join("workspace.sqlite3")).unwrap(),
                cx,
            );
            host.show_plugins = true;
            host.plugin_manager_tab = 1;
            host
        });
        assert!(window.debug_bounds("registry-source-dialog").is_none());
        let open = window.debug_bounds("open-registry-dialog").unwrap();
        window.simulate_click(open.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("registry-source-dialog").is_some());
        host.update(window, |host, cx| {
            host.registry_url
                .update(cx, |entry, cx| entry.set_value("draft-url", cx));
        });
        let submit = window.debug_bounds("submit-registry-dialog").unwrap();
        window.simulate_click(submit.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(host.registry_task.is_none());
            assert!(
                host.registry_message
                    .as_ref()
                    .unwrap()
                    .starts_with("插件源操作失败")
            );
            assert!(host.show_registry_dialog);
        });
        let cancel = window.debug_bounds("cancel-registry-dialog").unwrap();
        window.simulate_click(cancel.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("registry-source-dialog").is_none());
        window.simulate_click(open.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.registry_url.read(cx).value(), "draft-url")
        });
        window.simulate_keystrokes("tab");
        window.simulate_keystrokes("x");
        host.update(window, |host, cx| {
            assert!(host.registry_url.read(cx).value().contains('x'))
        });
        window.simulate_keystrokes("escape");
        assert!(window.debug_bounds("registry-source-dialog").is_none());
    }
}
