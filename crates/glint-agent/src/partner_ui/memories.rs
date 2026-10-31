//! Native companion memories view and interactions.
use super::*;

impl AgentWorkspace {
    pub(super) fn open_partner_memory(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.partner_tab(Tab::Memories, cx);
        self.partners_ui.filter = MemoryFilter::All;
        self.partners_ui
            .search
            .update(cx, |e, cx| e.set_value("", cx));
        self.partners_ui.expanded = Some(id);
    }
    pub(super) fn partner_memories(
        &self,
        partner: &partners::Partner,
        entries: &[partners::Entry],
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.theme.palette();
        let id = partner.id;
        let mut filters = div().flex().flex_wrap().gap_2();
        for (filter, label, key) in [
            (MemoryFilter::All, "全部记忆", "all"),
            (MemoryFilter::Active, "活跃", "active"),
            (MemoryFilter::Dormant, "沉睡", "dormant"),
            (MemoryFilter::Pinned, "固定保留", "pinned"),
        ] {
            let selector = format!("partner-filter-{key}");
            filters = filters.child(
                Button::new(label)
                    .kind(if self.partners_ui.filter == filter {
                        ButtonKind::Tinted
                    } else {
                        ButtonKind::Plain
                    })
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.partners_ui.filter = filter;
                        this.partners_ui.confirming_delete = None;
                        cx.notify();
                    })),
            );
        }
        let mut body = div()
            .flex()
            .flex_col()
            .gap_5()
            .debug_selector(|| "partner-memories-page".into())
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_4()
                    .child(self.partner_section("记忆与经历"))
                    .child(div().flex_1())
                    .child(
                        div().h(px(32.)).w(px(300.)).max_w_full().child(
                            InputField::new("搜索记忆，包括沉睡的经历")
                                .width(300.)
                                .control(self.partners_ui.search.clone())
                                .theme(self.theme)
                                .render(),
                        ),
                    ),
            )
            .child(filters)
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("当前项目与跨项目记忆。沉睡的记忆仍可搜索，固定的记忆会进入常驻窗口。"),
            );
        let query = self.partners_ui.search.read(cx).value().trim();
        let found = if query.is_empty() {
            Ok(entries.to_vec())
        } else {
            self.partner_repo().search(
                id,
                self.history.active().project.as_deref(),
                query,
                true,
                40,
            )
        };
        match found {
            Err(error) => body = body.child(self.partner_notice(error)),
            Ok(found) => {
                let mut visible: Vec<_> = found
                    .into_iter()
                    .filter(|e| self.partners_ui.filter.matches(e))
                    .collect();
                visible.sort_by_key(|e| self.partners_ui.expanded != Some(e.id));
                if visible.is_empty() {
                    body = body.child(
                        div()
                            .py_7()
                            .text_size(px(12.))
                            .text_color(rgb(p.text_muted))
                            .child(if entries.is_empty() {
                                "还没有记忆。完成一次交流后，经历会自动整理。"
                            } else {
                                "没有找到相关记忆，试试其他关键词或筛选条件。"
                            }),
                    );
                }
                for entry in visible.into_iter().take(100) {
                    body = body.child(self.partner_memory_card(id, &entry, cx));
                }
            }
        }
        body.into_any_element()
    }
    pub(super) fn partner_memory_card(
        &self,
        partner: Uuid,
        entry: &partners::Entry,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.theme.palette();
        let mid = entry.id;
        let expanded = self.partners_ui.expanded == Some(mid);
        let pinned = entry.pinned;
        let selector = format!("partner-memory-{mid}");
        let mut card =
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .pb_5()
                .border_b_1()
                .border_color(rgb(p.border))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .id(SharedString::from(format!("partner-memory-open-{mid}")))
                        .cursor_pointer()
                        .flex()
                        .items_start()
                        .gap_3()
                        .child(
                            div()
                                .pt_1()
                                .text_color(rgb(p.text_muted))
                                .child(entry_icon(&entry.kind).render(16.)),
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
                                        .child(entry.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(p.text_muted))
                                        .child(entry.summary.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(p.text_muted))
                                        .child(format!(
                                            "{} · {}",
                                            entry_scope(entry),
                                            local_date(entry.occurred_at, "%Y-%m-%d %H:%M")
                                        )),
                                ),
                        )
                        .child(self.partner_badge(
                            entry_status(entry),
                            pinned || entry.kind == "goal_running",
                        ))
                        .child(
                            div().text_color(rgb(p.text_muted)).child(
                                if expanded {
                                    Icon::ChevronDown
                                } else {
                                    Icon::ChevronRight
                                }
                                .render(14.),
                            ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.partners_ui.expanded = if this.partners_ui.expanded == Some(mid) {
                                None
                            } else {
                                Some(mid)
                            };
                            this.partners_ui.confirming_delete = None;
                            cx.notify();
                        })),
                );
        if expanded {
            card = card.child(
                div()
                    .p_4()
                    .rounded(px(8.))
                    .bg(rgb(p.panel))
                    .text_size(px(12.))
                    .line_height(px(22.))
                    .child(entry.content.clone()),
            );
            let mut actions = div().flex().flex_wrap().items_center().gap_2();
            if entry.kind == "identity" {
                let name = entry.content.clone();
                actions = actions.child(
                    Button::new("采用这个名字")
                        .kind(ButtonKind::Tinted)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let repo = this.partner_repo();
                            let result = repo.list().and_then(|ps| {
                                let p = ps
                                    .into_iter()
                                    .find(|p| p.id == partner)
                                    .ok_or("伙伴不存在")?;
                                repo.update(partner, &name, &p.avatar, &p.description)
                            });
                            this.partners_ui.notice = result.err();
                            if let Ok(ps) = repo.list() {
                                if let Some(p) = ps.iter().find(|p| p.id == partner) {
                                    this.select_partner(p, cx);
                                }
                            }
                            cx.notify();
                        })),
                );
            }
            if entry.kind == "goal" {
                actions = actions.child(
                    Button::new("开始推进")
                        .kind(ButtonKind::Tinted)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.partners_ui.notice = this
                                .partner_repo()
                                .start_goal(
                                    partner,
                                    this.history.active().project.as_deref(),
                                    mid,
                                    &this.provider,
                                )
                                .err();
                            cx.notify();
                        })),
                );
            }
            if entry.kind.starts_with("goal") && entry.kind != "goal_done" {
                actions = actions.child(
                    Button::new("结束目标")
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.partners_ui.notice = this
                                .partner_repo()
                                .finish_goal(partner, this.history.active().project.as_deref(), mid)
                                .err();
                            cx.notify();
                        })),
                );
            }
            if let Some(source) = entry.source["session_id"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .filter(|id| {
                    self.history
                        .sessions
                        .iter()
                        .any(|s| s.id == *id && s.project == self.history.active().project)
                })
            {
                actions = actions.child(
                    Button::new("查看来源会话")
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| this.switch(source, cx))),
                );
            }
            actions = actions
                .child(
                    Button::new(if pinned {
                        "取消固定"
                    } else {
                        "固定保留"
                    })
                    .kind(ButtonKind::Plain)
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.partners_ui.notice = this
                            .partner_repo()
                            .pin(
                                partner,
                                this.history.active().project.as_deref(),
                                mid,
                                !pinned,
                            )
                            .err();
                        cx.notify();
                    })),
                )
                .child(div().flex_1());
            if self.partners_ui.confirming_delete == Some(mid) {
                actions = actions
                    .child(
                        Button::new("取消")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Small)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.partners_ui.confirming_delete = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("删除本段经历及关联记忆")
                            .kind(ButtonKind::DangerTinted)
                            .size(ButtonSize::Small)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.partners_ui.notice = this
                                    .partner_repo()
                                    .delete(partner, this.history.active().project.as_deref(), mid)
                                    .err();
                                this.partners_ui.confirming_delete = None;
                                cx.notify();
                            })),
                    );
            } else {
                actions = actions.child(
                    Button::new("删除")
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.partners_ui.confirming_delete = Some(mid);
                            cx.notify();
                        })),
                );
            }
            card = card.child(actions);
        }
        card.into_any_element()
    }
}
