//! Native companion wake view and interactions.
use super::*;

impl AgentWorkspace {
    pub(super) fn partner_wakes(
        &self,
        partner: &partners::Partner,
        tasks: &[schedules::Schedule],
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.theme.palette();
        let mut body = div()
            .flex()
            .flex_col()
            .gap_5()
            .debug_selector(|| "partner-wakes-page".into())
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(self.partner_section("唤醒计划"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child("自己安排，也可以随时暂停。"),
                    ),
            )
            .child(
                div()
                    .max_w(px(560.))
                    .child(self.partner_wake_summary(partner, tasks, cx)),
            );
        if tasks.is_empty() {
            body = body.child(
                div()
                    .py_6()
                    .text_size(px(12.))
                    .text_color(rgb(p.text_muted))
                    .child("还没有唤醒计划。开启自主唤醒，或在会话中请伙伴安排一个日程。"),
            );
        }
        for task in tasks {
            let id = task.id;
            let running = task.running;
            let enable = !task.enabled;
            let can_toggle = !running
                && !(task.repeat == schedules::Repeat::Once
                    && task.last_run.is_some()
                    && !task.enabled);
            let mut row = div()
                .py_4()
                .border_b_1()
                .border_color(rgb(p.border))
                .flex()
                .flex_wrap()
                .items_center()
                .gap_4()
                .child(
                    div()
                        .w(px(76.))
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(20.))
                                .text_color(rgb(p.accent))
                                .child(task.run_at.format("%H:%M").to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child(task.run_at.format("%m-%d").to_string()),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(150.))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(task.title.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(preview_line(&task.prompt, 140)),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child(format!(
                                    "{} · {} · {}",
                                    if task.self_wake {
                                        format!("由{}安排", partner.name)
                                    } else {
                                        "手动安排".into()
                                    },
                                    schedule_status(task),
                                    task.project
                                        .as_ref()
                                        .and_then(|p| p.file_name())
                                        .map(|s| s.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| "无项目".into())
                                )),
                        ),
                )
                .child(self.partner_badge(task.wake_mode.label(), false));
            if can_toggle {
                let selector = format!("partner-wake-toggle-{id}");
                row = row.child(
                    Button::new(if task.enabled { "暂停" } else { "恢复" })
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .id(SharedString::from(selector.clone()))
                        .debug_selector(move || selector.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let result = this
                                .schedules
                                .lock()
                                .map_err(|_| "日程存储不可用".to_string())
                                .and_then(|mut store| {
                                    store.refresh()?;
                                    let mut task = store
                                        .tasks
                                        .iter()
                                        .find(|t| t.id == id)
                                        .cloned()
                                        .ok_or("日程不存在")?;
                                    if task.running {
                                        return Err("任务正在执行，请先在会话中停止".into());
                                    }
                                    task.enabled = enable;
                                    store.upsert(task)
                                });
                            this.partners_ui.notice = result.err();
                            this.refresh_calendar(cx);
                            cx.notify();
                        })),
                );
            }
            if !running {
                row = row.child(
                    Button::new("编辑")
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.page = crate::Page::Calendar;
                            this.show_auxiliary_sidebar = true;
                            this.edit_schedule(id, cx);
                            this.refresh_calendar(cx);
                            cx.notify();
                        })),
                );
            }
            if let Some(error) = &task.last_error {
                row = row.child(
                    div()
                        .w_full()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child(format!("上次执行：{error}")),
                );
            }
            body = body.child(row);
        }
        body.into_any_element()
    }
}
