use super::*;
use std::path::PathBuf;

pub(super) enum UpdateState {
    Idle,
    Checking,
    Current,
    Available(updater::Update),
    Downloading {
        received: u64,
        total: u64,
    },
    Ready {
        update: updater::Update,
        installer: PathBuf,
    },
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn custom_update_source_is_validated_persisted_and_reset(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let host = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        let source = updater::UpdateSource {
            url: "https://gateway.yanguiyuan.cn/plugins".into(),
            public_key: "x1+OcqKIYuboSgnxFik6ApNpjOU4HeJiD6nMgQSYIeQ=".into(),
            minimum_sequence: 7,
        };
        host.update(cx, |host, cx| {
            assert!(host.apply_update_source(Some(source.clone()), cx));
            let mut same = source.clone();
            same.minimum_sequence = 0;
            assert!(host.apply_update_source(Some(same), cx));
            assert_eq!(host.update_source.as_ref().unwrap().minimum_sequence, 7);
            let mut bad = source.clone();
            bad.url = "http://example.com".into();
            assert!(!host.apply_update_source(Some(bad), cx));
            assert_eq!(host.update_source.as_ref(), Some(&source));
            host.update_state = UpdateState::Checking;
            assert!(!host.apply_update_source(None, cx));
            host.update_state = UpdateState::Idle;
        });
        let reopened = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(cx, |host, cx| {
            assert_eq!(host.update_source.as_ref(), Some(&source));
            assert_eq!(host.update_source_url.read(cx).value(), source.url);
            assert!(host.apply_update_source(None, cx));
        });
        let reset = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reset.update(cx, |host, _| assert!(host.update_source.is_none()));
    }
    #[gpui::test]
    fn restart_guard_checks_unsaved_environment_drafts_and_name_edits(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let host = cx.new(|cx| {
            Workbench::with_store(
                storage::Store::open(&directory.path().join("workspace.sqlite3")).unwrap(),
                cx,
            )
        });
        host.update(cx, |host, cx| {
            assert!(host.update_can_restart(cx));
            host.group_edit = Some(GroupEdit::CreateEnvironment);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("Development", cx));
            host.save_group(cx);
            host.add_variable(cx);
            host.variable_rows[1][0]
                .name
                .update(cx, |entry, cx| entry.set_value("token", cx));
            host.editing_environment = 0;
            assert!(!host.update_can_restart(cx));
            assert!(host.update_error.as_ref().unwrap().contains("未保存"));
            host.variable_rows[1].clear();
            assert!(host.update_can_restart(cx));
            host.editing_request_name = true;
            assert!(!host.update_can_restart(cx));
            host.editing_request_name = false;
            host.sending = Some(cx.spawn(async move |_, _| std::future::pending::<()>().await));
            assert!(!host.update_can_restart(cx));
            host.sending = None;
            assert!(host.update_can_restart(cx));
        });
    }

    #[gpui::test]
    fn update_details_render_and_can_be_dismissed_without_a_network_request(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join("workspace.sqlite3")).unwrap(),
                cx,
            );
            host.update_state = UpdateState::Failed("离线".into());
            host.show_update_details = true;
            host
        });
        let trigger = window.debug_bounds("app-update-trigger").unwrap();
        let details = window.debug_bounds("app-update-details").unwrap();
        assert!(trigger.left() < px(46.));
        assert!(trigger.top() > px(100.));
        assert!(
            details.left() > trigger.right(),
            "popover must open to the right of the rail"
        );
        assert!(
            details.top() < trigger.top(),
            "popover must extend above the bottom trigger"
        );
        host.update(window, |host, _| assert!(host.update_task.is_none()));
        let dismiss = window.debug_bounds("dismiss-app-update").unwrap();
        window.simulate_click(dismiss.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(!host.show_update_details);
            assert!(host.update_task.is_none());
        });
        let trigger = window.debug_bounds("app-update-trigger").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(host.show_update_details);
            assert!(host.update_task.is_none());
        });
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        host.update(window, |host, _| assert!(!host.show_update_details));
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        window.simulate_click(point(px(450.), px(100.)), gpui::Modifiers::default());
        host.update(window, |host, _| assert!(!host.show_update_details));
    }
}
impl UpdateState {
    pub(super) fn busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Downloading { .. })
    }
}

impl Workbench {
    fn apply_update_source(
        &mut self,
        mut source: Option<updater::UpdateSource>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.update_state.busy() || matches!(self.update_state, UpdateState::Ready { .. }) {
            self.update_error = Some("请先完成或取消当前更新操作".into());
            cx.notify();
            return false;
        }
        if let Some(s) = &mut source {
            if let Err(error) = s.validate() {
                self.update_error = Some(format!("更新源无效：{error}"));
                cx.notify();
                return false;
            }
            if let Some(old) = self
                .update_source
                .as_ref()
                .filter(|old| old.url == s.url && old.public_key == s.public_key)
            {
                s.minimum_sequence = old.minimum_sequence;
            }
        }
        let previous = self.update_source.clone();
        self.update_source = source;
        self.storage_dirty = true;
        self.persist(cx);
        if let Some(error) = &self.storage_error {
            self.update_source = previous;
            self.storage_dirty = true;
            self.update_error = Some(format!("保存更新源失败：{error}"));
            cx.notify();
            return false;
        }
        self.update_error = None;
        self.update_state = UpdateState::Idle;
        cx.notify();
        true
    }
    pub(super) fn check_for_updates(&mut self, manual: bool, cx: &mut Context<Self>) {
        // An embedded module cannot install a release for the standalone application.
        if self.host_navigation.is_some() {
            return;
        }
        if self.update_state.busy() || matches!(self.update_state, UpdateState::Ready { .. }) {
            return;
        }
        self.show_update_details |= manual;
        self.update_error = None;
        self.update_state = UpdateState::Checking;
        let source = self.update_source.clone();
        self.update_task = Some(cx.spawn(async move |this, cx| {
            let result = if let Some(source) = source {
                updater::check_custom(source, &semver::Version::parse(updater::VERSION).unwrap())
                    .await
                    .map(|(update, sequence)| (update, Some(sequence)))
            } else {
                updater::check().await.map(|update| (update, None))
            };
            let _ = this.update(cx, |this, cx| {
                this.update_state = match result {
                    Ok((Some(update), sequence)) => {
                        if let (Some(source), Some(sequence)) = (&mut this.update_source, sequence)
                        {
                            source.minimum_sequence = sequence;
                            this.storage_dirty = true;
                            this.persist(cx);
                        }
                        this.show_update_details = true;
                        UpdateState::Available(update)
                    }
                    Ok((None, sequence)) => {
                        if let (Some(source), Some(sequence)) = (&mut this.update_source, sequence)
                        {
                            source.minimum_sequence = sequence;
                            this.storage_dirty = true;
                            this.persist(cx);
                        }
                        UpdateState::Current
                    }
                    Err(error) => UpdateState::Failed(error),
                };
                this.update_task = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn update_can_restart(&mut self, cx: &mut Context<Self>) -> bool {
        self.flush_request_editors(cx);
        let reason = if self.sending.is_some()
            || self.request_editors.values().any(|shared| shared.running)
        {
            Some("请求正在执行，请完成或取消请求后更新")
        } else if self.has_unsaved_environment(cx) {
            Some("环境设置有未保存修改，请保存或放弃修改后更新")
        } else if self.group_edit.is_some() || self.editing_request_name {
            Some("请完成名称编辑后更新")
        } else {
            self.persist(cx);
            self.storage_error
                .as_ref()
                .map(|_| "工作区尚未保存成功，请处理保存错误后更新")
        };
        self.update_error = reason.map(str::to_owned);
        self.update_error.is_none()
    }

    pub(super) fn download_update(&mut self, update: updater::Update, cx: &mut Context<Self>) {
        if !updater::can_install() {
            cx.open_url(&update.release_url());
            return;
        }
        if !self.update_can_restart(cx) {
            cx.notify();
            return;
        }
        self.update_state = UpdateState::Downloading {
            received: 0,
            total: update.size,
        };
        self.update_task = Some(cx.spawn(async move |this, cx| {
            let (sender, receiver) = async_channel::bounded(32);
            let task = match updater::start_download(update.clone(), sender) {
                Ok(task) => task,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.update_state = UpdateState::Failed(error);
                        this.update_task = None;
                        cx.notify();
                    });
                    return;
                }
            };
            while let Ok(event) = receiver.recv().await {
                let finished = matches!(event, updater::DownloadEvent::Finished(_));
                if this
                    .update(cx, |this, cx| {
                        match event {
                            updater::DownloadEvent::Progress(received, total) => {
                                this.update_state = UpdateState::Downloading { received, total };
                            }
                            updater::DownloadEvent::Finished(result) => {
                                this.update_task = None;
                                match result {
                                    Ok(installer) => {
                                        this.update_state = UpdateState::Ready {
                                            update: update.clone(),
                                            installer,
                                        };
                                        this.install_update(cx);
                                    }
                                    Err(error) => this.update_state = UpdateState::Failed(error),
                                }
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                if finished {
                    break;
                }
            }
            drop(task);
        }));
        cx.notify();
    }

    pub(super) fn install_update(&mut self, cx: &mut Context<Self>) {
        if !self.update_can_restart(cx) {
            return;
        }
        let UpdateState::Ready { update, installer } = &self.update_state else {
            return;
        };
        match updater::launch_installer(update, installer) {
            Ok(()) => cx.quit(),
            Err(error) => self.update_error = Some(error),
        }
    }

    pub(super) fn update_button(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let owner = cx.entity().downgrade();
        let trigger = IconButton::new(
            if matches!(
                self.update_state,
                UpdateState::Available(_) | UpdateState::Ready { .. }
            ) {
                Icon::Download
            } else {
                Icon::Refresh
            },
            format!("Glint API {} · 软件更新", updater::VERSION),
        )
        .theme(self.theme)
        .render()
        .id("app-update-trigger")
        .debug_selector(|| "app-update-trigger".into());
        Popover::new("app-update-popover", trigger, self.update_details(cx))
            .theme(self.theme)
            .width(if self.show_update_source_settings {
                420.
            } else {
                340.
            })
            .right_of_trigger(30.)
            .open(self.show_update_details)
            .on_open_change(move |open, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.show_update_details = open;
                    cx.notify();
                });
            })
            .render()
            .into_any_element()
    }

    fn update_details(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let text = match &self.update_state {
            UpdateState::Idle => format!("Glint API {}", updater::VERSION),
            UpdateState::Checking => "正在检查更新…".into(),
            UpdateState::Current => {
                format!("Glint API {} · 当前已是最新稳定版本", updater::VERSION)
            }
            UpdateState::Available(update) => format!(
                "发现新版本 {} · 当前版本 {}",
                update.version,
                updater::VERSION
            ),
            UpdateState::Downloading { received, total } => format!(
                "正在下载更新 · {}%（{:.1} / {:.1} MB）",
                received * 100 / (*total).max(1),
                *received as f64 / 1048576.,
                *total as f64 / 1048576.
            ),
            UpdateState::Ready { update, .. } => {
                format!("版本 {} 已下载并通过校验", update.version)
            }
            UpdateState::Failed(error) => format!("更新失败：{error}"),
        };
        let mut panel = div()
            .id("app-update-details")
            .debug_selector(|| "app-update-details".into())
            .flex()
            .flex_col()
            .gap_3()
            .text_size(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("软件更新"),
                    )
                    .child(
                        IconButton::new(Icon::Close, "收起更新信息")
                            .plain()
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "dismiss-app-update".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_update_details = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().text_color(rgb(p.text_muted)).child(text))
            .when_some(self.update_error.clone(), |el, error| {
                el.child(div().text_color(rgb(p.text_muted)).child(error))
            });
        match &self.update_state {
            UpdateState::Available(update) => {
                let update = update.clone();
                panel = panel.child(
                    Button::new(if updater::can_install() {
                        "更新并重启"
                    } else {
                        "查看发布"
                    })
                    .kind(ButtonKind::Primary)
                    .size(ButtonSize::Mini)
                    .theme(self.theme)
                    .render()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.download_update(update.clone(), cx)),
                    ),
                );
            }
            UpdateState::Ready { .. } => {
                panel = panel.child(
                    Button::new("安装并重启")
                        .kind(ButtonKind::Primary)
                        .size(ButtonSize::Mini)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.install_update(cx);
                            cx.notify();
                        })),
                );
            }
            UpdateState::Failed(_) => {
                panel = panel.child(
                    Button::new("重试")
                        .size(ButtonSize::Mini)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| this.check_for_updates(true, cx))),
                );
            }
            UpdateState::Downloading { .. } => {
                panel = panel.child(
                    Button::new("取消下载")
                        .size(ButtonSize::Mini)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.update_task = None;
                            this.update_state = UpdateState::Idle;
                            this.show_update_details = false;
                            cx.notify();
                        })),
                );
            }
            UpdateState::Idle | UpdateState::Current => {
                panel = panel.child(
                    Button::new("检查更新")
                        .size(ButtonSize::Mini)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "check-app-update".into())
                        .on_click(cx.listener(|this, _, _, cx| this.check_for_updates(true, cx))),
                );
            }
            _ => {}
        }
        panel = panel.child(
            div()
                .border_t_1()
                .border_color(rgb(p.border))
                .pt_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child(format!(
                            "当前更新源：{}",
                            self.update_source
                                .as_ref()
                                .map(|s| s.url.as_str())
                                .unwrap_or("GitHub（默认）")
                        )),
                )
                .child(
                    Button::new(if self.show_update_source_settings {
                        "收起更新源设置"
                    } else {
                        "更新源设置"
                    })
                    .kind(ButtonKind::Plain)
                    .size(ButtonSize::Mini)
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "update-source-settings".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_update_source_settings = !this.show_update_source_settings;
                        cx.notify();
                    })),
                ),
        );
        if self.show_update_source_settings {
            panel = panel.child(div().flex().flex_col().gap_2()
                .child("分发源地址").child(self.update_source_url.clone()).child("可信 Ed25519 公钥").child(self.update_source_key.clone())
                .child(div().text_size(px(10.)).text_color(rgb(p.text_muted)).child("自定义源覆盖 GitHub。请从可信渠道确认公钥；安装前仍会校验签名目录与文件摘要。"))
                .child(Button::new("填入 gateway 分发源").kind(ButtonKind::Plain).size(ButtonSize::Mini).theme(self.theme).render().on_click(cx.listener(|this,_,_,cx|{
                    this.update_source_url.update(cx,|e,cx|e.set_value("https://gateway.yanguiyuan.cn/plugins",cx));this.update_source_key.update(cx,|e,cx|e.set_value("x1+OcqKIYuboSgnxFik6ApNpjOU4HeJiD6nMgQSYIeQ=",cx));
                })))
                .child(div().flex().gap_2()
                    .child(Button::new("保存并检查").size(ButtonSize::Mini).theme(self.theme).render().debug_selector(||"save-update-source".into()).on_click(cx.listener(|this,_,_,cx| {
                        let url=this.update_source_url.read(cx).value().trim().trim_end_matches('/').to_owned();let key=this.update_source_key.read(cx).value().trim().to_owned();
                        let source=(!url.is_empty()).then_some(updater::UpdateSource{url,public_key:key,minimum_sequence:0});if this.apply_update_source(source,cx){this.show_update_source_settings=false;this.check_for_updates(true,cx);}
                    })))
                    .child(Button::new("恢复 GitHub").kind(ButtonKind::Plain).size(ButtonSize::Mini).theme(self.theme).render().debug_selector(||"reset-update-source".into()).on_click(cx.listener(|this,_,_,cx|{if this.apply_update_source(None,cx){this.update_source_url.update(cx,|e,cx|e.set_value("",cx));this.update_source_key.update(cx,|e,cx|e.set_value("",cx));cx.notify();}})))));
        }
        panel
    }
}
