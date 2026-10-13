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
        let check = window.debug_bounds("check-app-update").unwrap();
        assert!(check.size.width > px(0.));
        host.update(window, |host, _| assert!(host.update_task.is_none()));
        let dismiss = window.debug_bounds("dismiss-app-update").unwrap();
        window.simulate_click(dismiss.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(!host.show_update_details);
            assert!(host.update_task.is_none());
        });
    }
}
impl UpdateState {
    fn busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Downloading { .. })
    }
}

impl Workbench {
    pub(super) fn check_for_updates(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.update_state.busy() || matches!(self.update_state, UpdateState::Ready { .. }) {
            return;
        }
        self.show_update_details |= manual;
        self.update_error = None;
        self.update_state = UpdateState::Checking;
        self.update_task = Some(cx.spawn(async move |this, cx| {
            let result = updater::check().await;
            let _ = this.update(cx, |this, cx| {
                this.update_state = match result {
                    Ok(Some(update)) => {
                        this.show_update_details = true;
                        UpdateState::Available(update)
                    }
                    Ok(None) => UpdateState::Current,
                    Err(error) => UpdateState::Failed(error),
                };
                this.update_task = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn update_can_restart(&mut self, cx: &mut Context<Self>) -> bool {
        let reason = if self.sending.is_some() {
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

    fn download_update(&mut self, update: updater::Update, cx: &mut Context<Self>) {
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

    fn install_update(&mut self, cx: &mut Context<Self>) {
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
        IconButton::new(
            if matches!(
                self.update_state,
                UpdateState::Available(_) | UpdateState::Ready { .. }
            ) {
                Icon::Download
            } else {
                Icon::Refresh
            },
            format!("Glint API {} · 检查更新", updater::VERSION),
        )
        .theme(self.theme)
        .render()
        .id("check-app-update")
        .debug_selector(|| "check-app-update".into())
        .on_click(cx.listener(|this, _, _, cx| {
            this.show_update_details = true;
            if matches!(
                this.update_state,
                UpdateState::Idle | UpdateState::Current | UpdateState::Failed(_)
            ) {
                this.check_for_updates(true, cx);
            }
            cx.notify();
        }))
        .into_any_element()
    }

    pub(super) fn update_banner(&self, cx: &mut Context<Self>) -> gpui::Div {
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
        let mut banner = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .bg(rgb(p.accent_tint))
            .border_b_1()
            .border_color(rgb(p.border))
            .text_size(px(11.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(text)
                    .when_some(self.update_error.clone(), |el, error| {
                        el.child(div().text_color(rgb(p.text_muted)).child(error))
                    }),
            );
        match &self.update_state {
            UpdateState::Available(update) => {
                let update = update.clone();
                banner = banner.child(
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
                banner = banner.child(
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
                banner = banner.child(
                    Button::new("重试")
                        .size(ButtonSize::Mini)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| this.check_for_updates(true, cx))),
                );
            }
            UpdateState::Downloading { .. } => {
                banner = banner.child(
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
            _ => {}
        }
        banner.child(
            IconButton::new(Icon::Close, "收起更新信息")
                .theme(self.theme)
                .render()
                .debug_selector(|| "dismiss-app-update".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_update_details = false;
                    cx.notify();
                })),
        )
    }
}
