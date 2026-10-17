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
            .width(340.)
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
        panel
    }
}
