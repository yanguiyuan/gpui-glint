//! Native companion overview view and interactions.
use super::*;

impl AgentWorkspace {
    pub(super) fn partner_overview(
        &self,
        partner: &partners::Partner,
        entries: &[partners::Entry],
        tasks: &[schedules::Schedule],
        wide: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.theme.palette();
        let id = partner.id;
        let mut main = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_7()
            .debug_selector(|| "partner-overview-main".into());
        if entries.is_empty() {
            main=main.child(div().pt_5().flex().flex_col().gap_4().debug_selector(||"partner-empty-intro".into())
                .child(div().text_size(px(10.)).text_color(rgb(p.text_muted)).child("A NEW BEGINNING"))
                .child(div().text_size(px(27.)).font_weight(FontWeight::MEDIUM).child("还没有故事，\n先一起做点什么。"))
                .child(div().text_size(px(13.)).line_height(px(24.)).text_color(rgb(p.text_muted)).child("不用先决定名字、性格或擅长的事。\n从一次交流开始，经历和习惯会慢慢留下来。"))
                .child(div().mt_2().child(self.partner_chat_button(id,true,cx))));
            let mut steps = div()
                .mt_3()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.partner_section("我们会怎样慢慢熟悉"));
            for (i, title, desc) in [
                (
                    "01",
                    "留下真实的经历",
                    "一起做过的事，会成为伙伴自己的记忆。",
                ),
                (
                    "02",
                    "形成自己的习惯",
                    "多次合作中得到印证的倾向，才会慢慢稳定。",
                ),
                (
                    "03",
                    "找到值得继续的事",
                    "从交流里发现关注点，提出下一步想做的事。",
                ),
            ] {
                steps = steps.child(
                    div()
                        .py_4()
                        .border_b_1()
                        .border_color(rgb(p.border))
                        .flex()
                        .gap_4()
                        .text_color(rgb(p.text_muted))
                        .child(div().text_size(px(10.)).pt_1().child(i))
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(div().text_size(px(12.)).child(title))
                                .child(div().text_size(px(11.)).child(desc)),
                        ),
                );
            }
            main = main.child(steps);
        } else {
            let focus = entries
                .iter()
                .find(|e| is_active_goal(&e.kind))
                .or_else(|| {
                    entries
                        .iter()
                        .find(|e| e.kind == "commitment" || e.kind == "preference")
                });
            if let Some(entry) = focus {
                let mid = entry.id;
                main = main.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .child(self.partner_section("最近的关注"))
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(p.text_muted))
                                        .child(local_date(entry.occurred_at, "%m-%d %H:%M")),
                                ),
                        )
                        .child(
                            div()
                                .p_5()
                                .rounded(px(10.))
                                .bg(rgb(p.panel))
                                .flex()
                                .flex_col()
                                .gap_4()
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .line_height(px(25.))
                                        .child(entry.summary.clone()),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .text_size(px(10.))
                                        .text_color(rgb(p.text_muted))
                                        .child("来自真实合作中的记忆")
                                        .child(div().flex_1())
                                        .child(
                                            Button::new("查看相关记忆 ↗")
                                                .kind(ButtonKind::Plain)
                                                .size(ButtonSize::Mini)
                                                .theme(self.theme)
                                                .render()
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.open_partner_memory(mid, cx);
                                                })),
                                        ),
                                ),
                        ),
                );
            }
            let goals: Vec<_> = entries
                .iter()
                .filter(|e| is_active_goal(&e.kind))
                .take(3)
                .collect();
            let mut goal_section = div().flex().flex_col().gap_3().child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.partner_section("正在推进"))
                    .child(self.partner_badge(goals.len().to_string(), false)),
            );
            if goals.is_empty() {
                goal_section = goal_section.child(
                    div()
                        .py_3()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child("暂时没有待推进的目标。有值得继续的事情时，伙伴会提出建议。"),
                );
            }
            for goal in goals {
                let mid = goal.id;
                goal_section = goal_section.child(
                    div()
                        .p_4()
                        .border_1()
                        .border_color(rgb(p.border))
                        .rounded(px(9.))
                        .flex()
                        .items_start()
                        .gap_3()
                        .child(
                            div()
                                .pt_1()
                                .text_color(rgb(p.accent))
                                .child(Icon::Compass.render(17.)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(goal.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(p.text_muted))
                                        .child(goal.summary.clone()),
                                )
                                .child(div().child(self.partner_badge(entry_status(goal), true))),
                        )
                        .child(
                            IconButton::new(Icon::ChevronRight, "查看目标")
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_partner_memory(mid, cx);
                                })),
                        ),
                );
            }
            main = main.child(goal_section);
            let mut timeline = div().flex().flex_col().gap_3().child(
                div()
                    .flex()
                    .items_center()
                    .child(self.partner_section("最近的经历"))
                    .child(div().flex_1())
                    .child(self.partner_link("全部经历 ↗", Tab::Memories, cx)),
            );
            let experiences = recent_experiences(entries);
            for entry in experiences.into_iter().take(5) {
                let mid = entry.id;
                timeline = timeline.child(
                    div()
                        .flex()
                        .gap_3()
                        .py_2()
                        .child(
                            div()
                                .w(px(26.))
                                .h(px(26.))
                                .flex_shrink_0()
                                .rounded(px(7.))
                                .bg(rgb(p.panel))
                                .text_color(rgb(p.text_muted))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(entry_icon(&entry.kind).render(14.)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .flex()
                                        .items_start()
                                        .gap_3()
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_size(px(12.))
                                                .child(entry.title.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(rgb(p.text_muted))
                                                .child(local_date(
                                                    entry.occurred_at,
                                                    "%m-%d %H:%M",
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(p.text_muted))
                                        .child(entry.summary.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(p.text_muted))
                                        .child(entry_scope(entry)),
                                ),
                        )
                        .child(
                            IconButton::new(Icon::ChevronRight, "查看这段经历")
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_partner_memory(mid, cx);
                                })),
                        ),
                );
            }
            main = main.child(timeline);
        }
        let side = self.partner_overview_side(partner, entries, tasks, wide, cx);
        div()
            .flex()
            .when(!wide, |d| d.flex_col())
            .gap(px(32.))
            .child(main)
            .child(side)
            .into_any_element()
    }
    pub(super) fn partner_overview_side(
        &self,
        partner: &partners::Partner,
        entries: &[partners::Entry],
        tasks: &[schedules::Schedule],
        wide: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let p = self.theme.palette();
        let repo = self.partner_repo();
        let id = partner.id;
        let mut side = div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_7()
            .when(wide, |d| {
                d.w(px(276.))
                    .pl_6()
                    .border_l_1()
                    .border_color(rgb(p.border))
                    .debug_selector(|| "partner-overview-side".into())
            })
            .when(!wide, |d| {
                d.w_full()
                    .pt_6()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .debug_selector(|| "partner-overview-side".into())
            });
        side = side.child(self.partner_wake_summary(partner, tasks, cx));
        let mut habits = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.partner_section("慢慢形成的习惯"));
        match repo.traits(id) {
            Ok(traits) if !traits.is_empty() => {
                for (t, n) in traits.into_iter().take(4) {
                    let mut dots = div().flex().gap_1();
                    for i in 0..5 {
                        dots = dots.child(
                            div().w(px(4.)).h(px(4.)).rounded_full().bg(rgb(if i < n {
                                p.accent
                            } else {
                                p.border
                            })),
                        );
                    }
                    habits = habits.child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                div().pt_1().text_color(rgb(p.text_muted)).child(
                                    if n >= 3 {
                                        Icon::CheckCircle
                                    } else {
                                        Icon::Star
                                    }
                                    .render(14.),
                                ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(div().text_size(px(12.)).child(t))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(p.text_muted))
                                            .child(format!(
                                                "{} · {n} 次不同会话支持",
                                                if n >= 3 {
                                                    "已形成"
                                                } else {
                                                    "还在观察中"
                                                }
                                            )),
                                    )
                                    .child(dots),
                            ),
                    );
                }
            }
            Ok(_) => {
                habits = habits.child(
                    div()
                        .py_5()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_4()
                        .child(self.partner_avatar(partner, 64.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(p.text_muted))
                                .child("留一点空间，慢慢长成。"),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child("还没有形成固定的习惯。"),
                        ),
                )
            }
            Err(error) => habits = habits.child(self.partner_notice(error)),
        }
        side = side.child(habits);
        if !entries.is_empty() {
            let active = entries
                .iter()
                .filter(|e| e.activation(chrono::Utc::now().timestamp()) >= 0.2)
                .count();
            side = side.child(
                div()
                    .pt_5()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.partner_section("记忆的痕迹"))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_2()
                            .child(div().text_size(px(24.)).child(entries.len().to_string()))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child("段记忆"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child("记得重要的，也允许一些事情慢慢沉睡。"),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!(
                                "{active} 活跃   ·   {} 沉睡",
                                entries.len() - active
                            )),
                    ),
            );
        }
        side
    }
    pub(super) fn partner_wake_summary(
        &self,
        partner: &partners::Partner,
        tasks: &[schedules::Schedule],
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let p = self.theme.palette();
        let id = partner.id;
        let proactive = self.partner_repo().proactive(id);
        let enabled = proactive.as_ref().copied().unwrap_or(false);
        let weak = cx.entity().downgrade();
        let mut section = div().flex().flex_col().gap_3().child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_color(rgb(p.text_muted))
                        .child(Icon::Clock.render(15.)),
                )
                .child(self.partner_section("自主唤醒"))
                .child(div().flex_1())
                .child(
                    Switch::new("")
                        .checked(enabled)
                        .theme(self.theme)
                        .on_change(move |on, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.partners_ui.notice = this
                                    .partner_repo()
                                    .set_proactive(
                                        id,
                                        on,
                                        this.history.active().project.as_deref(),
                                        &this.provider,
                                    )
                                    .err();
                                cx.notify();
                            });
                        })
                        .render()
                        .id("partner-wake-toggle")
                        .debug_selector(|| "partner-wake-toggle".into()),
                ),
        );
        if let Err(error) = proactive {
            section = section.child(self.partner_notice(error));
        }
        let next = tasks.iter().find(|t| t.running || t.enabled);
        let mut next_card = div()
            .p_4()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.panel))
            .flex()
            .flex_col()
            .gap_3()
            .debug_selector(|| "partner-next-wake".into());
        if let Some(task) = next {
            next_card = next_card
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child(if task.running {
                            "正在执行".into()
                        } else {
                            format!("下一次 · {}", task.run_at.format("%m 月 %d 日"))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_baseline()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(28.))
                                .text_color(rgb(p.accent))
                                .child(task.run_at.format("%H:%M").to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child(task.wake_mode.label()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(task.title.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child(preview_line(&task.prompt, 120)),
                )
                .child(
                    div()
                        .pt_3()
                        .border_t_1()
                        .border_color(rgb(p.border))
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child(if task.self_wake {
                            format!("由{}安排", partner.name)
                        } else {
                            "手动安排".into()
                        }),
                );
        } else {
            next_card = next_card
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child("还没有唤醒计划"),
                )
                .child(div().text_size(px(16.)).child("等待一次新的安排"))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child("开启后，伙伴可以根据自己的关注点安排回顾或执行任务。"),
                );
        }
        section.child(next_card)
            .child(div().text_size(px(10.)).text_color(rgb(p.text_muted)).child("应用运行时唤醒，操作沿用当前审批设置。自主唤醒每 24 小时最多 3 次，间隔至少 15 分钟。"))
            .child(div().child(self.partner_link("查看唤醒计划 ↗",Tab::Wake,cx)))
    }
}
