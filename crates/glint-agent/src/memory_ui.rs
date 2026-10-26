use super::*;
use chrono::Timelike;
use gpui_glint::components::{InputField, Switch};

pub struct State {
    pub settings: memory::Settings,
    pub cards: Vec<memory::Card>,
    pub selected: Option<memory::Record>,
    pub notice: Option<String>,
    pub last_archive: String,
    pub time: Entity<TextEntry>,
    pub confirming: Option<Uuid>,
}
impl State {
    pub fn new(
        repository: &memory::Repository,
        project: Option<&std::path::Path>,
        cx: &mut Context<AgentWorkspace>,
    ) -> Self {
        let result = repository.settings();
        let notice = result.as_ref().err().cloned();
        let settings = result.unwrap_or_default();
        Self {
            time: cx.new(|cx| {
                TextEntry::new(
                    cx,
                    "23:00",
                    format!("{:02}:{:02}", settings.hour, settings.minute),
                )
            }),
            settings,
            cards: repository.list(project, 100).unwrap_or_default(),
            selected: None,
            notice,
            last_archive: repository.last_archive().unwrap_or_default(),
            confirming: None,
        }
    }
}
impl AgentWorkspace {
    pub fn reload_memories(&mut self, cx: &mut Context<Self>) {
        match self.memory.settings() {
            Ok(settings) => {
                self.memory_ui.time.update(cx, |entry, cx| {
                    entry.set_value(format!("{:02}:{:02}", settings.hour, settings.minute), cx)
                });
                self.memory_ui.settings = settings;
            }
            Err(error) => self.memory_ui.notice = Some(error),
        }
        match self
            .memory
            .list(self.history.active().project.as_deref(), 100)
        {
            Ok(cards) => self.memory_ui.cards = cards,
            Err(error) => self.memory_ui.notice = Some(error),
        }
        self.memory_ui.last_archive = self.memory.last_archive().unwrap_or_default();
        cx.notify();
    }
    pub fn tick_memory(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.memory_task.is_some() || self.pending.is_some() || self.preparing.is_some() {
            if manual {
                self.memory_ui.notice = Some("请等待当前对话或整理任务完成".into());
                cx.notify();
            }
            return;
        }
        if !manual
            && self
                .memory_last_probe
                .is_some_and(|last| last.elapsed() < std::time::Duration::from_secs(60))
        {
            return;
        }
        self.memory_last_probe = Some(std::time::Instant::now());
        if !self.provider.enabled {
            if manual {
                self.memory_ui.notice = Some("请先启用供应商，整理使用当前选中的模型".into());
                cx.notify();
            }
            return;
        }
        let repository = self.memory.clone();
        let config = self.provider.clone();
        let work = cx.background_executor().spawn(async move {
            let Some(run) = repository.claim_archive(chrono::Local::now(), manual)? else {
                return Ok(None);
            };
            let count = repository.archive(config, run).await?;
            Ok::<_, String>(Some(count))
        });
        self.memory_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.memory_task.take() {
                    task.detach();
                }
                match result {
                    Ok(Some(count)) => {
                        this.memory_ui.notice =
                            Some(format!("本轮已整理 {count} 条长期记忆，原始对话已保留。"));
                        this.reload_memories(cx);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        this.memory_ui.notice =
                            Some(format!("记忆整理失败：{error}；进度已保留，稍后重试。"))
                    }
                }
                cx.notify();
            });
        }));
        if manual {
            self.memory_ui.notice = Some("正在后台整理未归档对话…".into());
        }
        cx.notify();
    }
    fn save_memory_settings(&mut self, cx: &mut Context<Self>) {
        let result = (|| {
            let time = chrono::NaiveTime::parse_from_str(
                self.memory_ui.time.read(cx).value().trim(),
                "%H:%M",
            )
            .map_err(|_| "请输入有效时间，例如 23:00")?;
            let mut settings = self.memory.settings()?;
            settings.automatic_recall = self.memory_ui.settings.automatic_recall;
            settings.nightly_enabled = self.memory_ui.settings.nightly_enabled;
            settings.hour = time.hour();
            settings.minute = time.minute();
            self.memory.save_settings(settings)
        })();
        self.memory_ui.notice = Some(match result {
            Ok(()) => "记忆设置已保存".into(),
            Err(error) => error,
        });
        self.reload_memories(cx);
    }
    pub fn memory_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let state = &self.memory_ui;
        let recall_weak = cx.entity().downgrade();
        let nightly_weak = cx.entity().downgrade();
        let mut page=div().w_full().max_w(px(820.)).mx_auto().flex().flex_col().gap_4()
            .child(div().text_size(px(28.)).font_weight(FontWeight::SEMIBOLD).child("记忆"))
            .child(div().text_color(rgb(p.text_muted)).child("根据当前意图回忆相关信息；原始对话保留在本机。"))
            .child(div().p_4().rounded(px(12.)).border_1().border_color(rgb(p.border)).flex().flex_col().gap_4()
                .child(div().flex().items_center().justify_between().child(Switch::new("自动语义回忆").checked(state.settings.automatic_recall).theme(self.theme).on_change(move |checked,_,cx|{let _=recall_weak.update(cx,|this,cx|{this.memory_ui.settings.automatic_recall=checked;this.save_memory_settings(cx);});}).render()))
                .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("先理解问题意图，再筛选记忆摘要；模型按需读取全文。使用当前模型进行语义判断。"))
                .child(div().flex().items_center().justify_between().child(Switch::new("每天整理对话").checked(state.settings.nightly_enabled).theme(self.theme).on_change(move |checked,_,cx|{let _=nightly_weak.update(cx,|this,cx|{this.memory_ui.settings.nightly_enabled=checked;this.save_memory_settings(cx);});}).render()))
                .child(div().flex().items_end().gap_3().child(div().w(px(180.)).child(InputField::new("每日整理时间（本地）").control(state.time.clone()).theme(self.theme).render()))
                    .child(Button::new("保存记忆设置").kind(ButtonKind::Gray).theme(self.theme).render().on_click(cx.listener(|this,_,_,cx|this.save_memory_settings(cx))))
                    .child(Button::new(if self.memory_task.is_some(){"正在整理…"}else{"立即整理"}).theme(self.theme).when(self.memory_task.is_some(),|button|button.disabled()).render().on_click(cx.listener(|this,_,_,cx|this.tick_memory(true,cx)))))
                .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child(format!("下次：{} · 应用运行且对话空闲时执行；退出后在下次启动补处理。",state.settings.next_run.format("%m-%d %H:%M"))))
                .child(div().text_size(px(10.)).text_color(rgb(p.text_muted)).child(state.last_archive.clone())))
            .when_some(state.notice.clone(),|page,notice|page.child(div().p_3().rounded(px(8.)).bg(rgb(p.surface_subtle)).child(notice)))
            .child(div().font_weight(FontWeight::MEDIUM).child(format!("当前项目与全局记忆 · {} 条（最近 100 条）",state.cards.len())));
        if state.cards.is_empty() {
            page = page.child(
                div()
                    .p_5()
                    .text_color(rgb(p.text_muted))
                    .child("尚无记忆。可以在对话中说“记住我的偏好”，或立即整理已有对话。"),
            );
        }
        for card in &state.cards {
            let id = card.id;
            let read = id;
            let weak = cx.entity().downgrade();
            let confirm = div()
                .flex()
                .flex_col()
                .gap_3()
                .child("删除这条记忆？原始会话不会删除。")
                .child(
                    Button::new("删除记忆")
                        .kind(ButtonKind::Danger)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let result = this
                                .memory
                                .delete(id, this.history.active().project.as_deref());
                            this.memory_ui.confirming = None;
                            this.memory_ui.notice = result.err();
                            if this
                                .memory_ui
                                .selected
                                .as_ref()
                                .is_some_and(|record| record.card.id == id)
                            {
                                this.memory_ui.selected = None;
                            }
                            this.reload_memories(cx);
                        })),
                );
            let trigger = IconButton::new(Icon::Trash, format!("删除记忆 {}", card.title))
                .plain()
                .theme(self.theme)
                .render();
            page = page.child(
                div()
                    .id(SharedString::from(format!("memory:{id}")))
                    .p_4()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(card.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.text_muted))
                                    .child(if card.project.is_some() {
                                        "项目"
                                    } else {
                                        "全局记忆"
                                    }),
                            )
                            .child(
                                Popover::new(
                                    SharedString::from(format!("delete-memory:{id}")),
                                    trigger,
                                    confirm,
                                )
                                .open(state.confirming == Some(id))
                                .theme(self.theme)
                                .on_open_change(
                                    move |open, _, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.memory_ui.confirming = open.then_some(id);
                                            cx.notify();
                                        });
                                    },
                                ),
                            ),
                    )
                    .child(
                        div()
                            .text_color(rgb(p.text_muted))
                            .child(card.summary.clone()),
                    )
                    .child(
                        Button::new("查看记忆和来源")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Small)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                match this
                                    .memory
                                    .read(read, this.history.active().project.as_deref())
                                {
                                    Ok(record) => this.memory_ui.selected = Some(record),
                                    Err(error) => this.memory_ui.notice = Some(error),
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(record) = &state.selected {
            page = page.child(
                div()
                    .p_4()
                    .rounded(px(10.))
                    .bg(rgb(p.surface_subtle))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(record.card.title.clone()),
                    )
                    .child(record.content.clone())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!("来源：{}", record.source)),
                    ),
            );
        }
        div().size_full().child(
            div()
                .id("memory-settings")
                .size_full()
                .overflow_y_scroll()
                .p_6()
                .child(page),
        )
    }
}
