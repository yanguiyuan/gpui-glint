use super::*;
use chrono::Timelike;
use gpui_glint::components::{InputField, Switch};

fn memory_controls(
    time: Entity<TextEntry>,
    theme: ThemePreset,
    save: impl IntoElement,
    run: impl IntoElement,
) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .flex_wrap()
        .items_end()
        .gap_3()
        .child(
            InputField::new("HH:MM")
                .label("每日整理时间（本地）")
                .width(180.)
                .control(time)
                .theme(theme)
                .render()
                .flex_shrink_0()
                .debug_selector(|| "memory-time-field".into()),
        )
        .child(
            div()
                .flex_shrink_0()
                .debug_selector(|| "memory-save-control".into())
                .child(save),
        )
        .child(
            div()
                .flex_shrink_0()
                .debug_selector(|| "memory-run-control".into())
                .child(run),
        )
}

fn memory_scope(card: &memory::Card) -> &'static str {
    if card.project.is_some() {
        "项目"
    } else {
        "全局"
    }
}

fn matches_memory(card: &memory::Card, query: &str, scope: usize) -> bool {
    let in_scope = match scope {
        1 => card.project.is_some(),
        2 => card.project.is_none(),
        _ => true,
    };
    in_scope
        && (query.is_empty()
            || [&card.title, &card.summary, &card.category]
                .iter()
                .any(|text| text.to_lowercase().contains(query)))
}

pub struct State {
    pub settings: memory::Settings,
    pub cards: Vec<memory::Card>,
    pub selected: Option<memory::Record>,
    pub notice: Option<String>,
    pub last_archive: String,
    pub time: Entity<TextEntry>,
    pub confirming: Option<Uuid>,
    search: Entity<TextEntry>,
    scope: usize,
    show_settings: bool,
    detail_scroll: ScrollHandle,
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
        let search = cx.new(|cx| TextEntry::new(cx, "搜索记忆名称、摘要或分类…", ""));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        Self {
            search,
            scope: 0,
            show_settings: false,
            detail_scroll: ScrollHandle::new(),
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
            Ok(cards) => {
                self.memory_ui.cards = cards;
                if let Some(id) = self
                    .memory_ui
                    .selected
                    .as_ref()
                    .map(|record| record.card.id)
                {
                    self.memory_ui.selected = self
                        .memory
                        .read(id, self.history.active().project.as_deref())
                        .ok();
                    self.memory_ui.confirming = None;
                }
            }
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
    fn select_memory(&mut self, id: Uuid, cx: &mut Context<Self>) {
        match self
            .memory
            .read(id, self.history.active().project.as_deref())
        {
            Ok(record) => {
                self.memory_ui.selected = Some(record);
                self.memory_ui.confirming = None;
                self.memory_ui.detail_scroll = ScrollHandle::new();
            }
            Err(error) => self.memory_ui.notice = Some(error),
        }
        cx.notify();
    }

    fn memory_preferences(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let state = &self.memory_ui;
        let recall_weak = cx.entity().downgrade();
        let nightly_weak = cx.entity().downgrade();
        div()
            .id("memory-preferences")
            .max_h(px(280.))
            .overflow_y_scroll()
            .p_4()
            .bg(rgb(p.surface_subtle))
            .border_b_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                Switch::new("记忆主动匹配")
                    .checked(state.settings.automatic_recall)
                    .theme(self.theme)
                    .on_change(move |checked, _, cx| {
                        let _ = recall_weak.update(cx, |this, cx| {
                            this.memory_ui.settings.automatic_recall = checked;
                            this.save_memory_settings(cx);
                            if !checked {
                                this.recall_runner.update(cx, |runner, _| runner.cancel_all());
                            }
                        });
                    })
                    .render(),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("发送消息后在后台匹配记忆，不等待匹配即可开始执行；结果在后续模型轮次加入上下文。关闭后仍可按需搜索和读取记忆。"),
            )
            .child(
                Switch::new("每天整理对话")
                    .checked(state.settings.nightly_enabled)
                    .theme(self.theme)
                    .on_change(move |checked, _, cx| {
                        let _ = nightly_weak.update(cx, |this, cx| {
                            this.memory_ui.settings.nightly_enabled = checked;
                            this.save_memory_settings(cx);
                        });
                    })
                    .render(),
            )
            .child(memory_controls(
                state.time.clone(),
                self.theme,
                Button::new("保存记忆设置")
                    .kind(ButtonKind::Gray)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| this.save_memory_settings(cx))),
                Button::new(if self.memory_task.is_some() {
                    "正在整理…"
                } else {
                    "立即整理"
                })
                .theme(self.theme)
                .when(self.memory_task.is_some(), |button| button.disabled())
                .render()
                .on_click(cx.listener(|this, _, _, cx| this.tick_memory(true, cx))),
            ))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(if state.settings.nightly_enabled {
                        format!(
                            "下次整理：{} · 应用运行且对话空闲时执行；退出后在下次启动补处理。",
                            state.settings.next_run.format("%m-%d %H:%M")
                        )
                    } else {
                        "自动整理已关闭，仍可随时手动整理。".into()
                    }),
            )
            .when(!state.last_archive.is_empty(), |body| {
                body.child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child(state.last_archive.clone()),
                )
            })
    }

    pub fn memory_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let state = &self.memory_ui;
        // Both entries also follow the active theme when this settings page is opened.
        state
            .time
            .update(cx, |entry, cx| entry.set_theme(self.theme, cx));
        state
            .search
            .update(cx, |entry, cx| entry.set_theme(self.theme, cx));
        let query = state.search.read(cx).value().trim().to_lowercase();
        let filtered: Vec<_> = state
            .cards
            .iter()
            .filter(|card| matches_memory(card, &query, state.scope))
            .collect();
        let status = format!(
            "记忆主动匹配{} · {}",
            if state.settings.automatic_recall {
                "已开启"
            } else {
                "已关闭"
            },
            if state.settings.nightly_enabled {
                format!(
                    "每天 {:02}:{:02} 整理",
                    state.settings.hour, state.settings.minute
                )
            } else {
                "自动整理已关闭".into()
            }
        );
        let header = div()
            .p_6()
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("记忆"),
                    )
                    .child(
                        Button::new("刷新记忆")
                            .kind(ButtonKind::Gray)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.memory_ui.notice = None;
                                this.reload_memories(cx);
                            })),
                    )
                    .child(
                        Button::new(if state.show_settings {
                            "收起设置"
                        } else {
                            "记忆设置"
                        })
                        .kind(ButtonKind::Gray)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.memory_ui.show_settings = !this.memory_ui.show_settings;
                            cx.notify();
                        })),
                    ),
            )
            .child(div().text_color(rgb(p.text_muted)).child(status))
            .when_some(state.notice.clone(), |header, notice| {
                header.child(
                    div()
                        .p_3()
                        .rounded(px(8.))
                        .bg(rgb(p.surface_subtle))
                        .child(notice),
                )
            });
        let weak = cx.entity().downgrade();
        let mut list = div()
            .id("memory-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .px_2()
                    .py_2()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(format!(
                        "显示 {} / {} 条 · 最近 100 条",
                        filtered.len(),
                        state.cards.len()
                    )),
            );
        if filtered.is_empty() {
            list = list.child(div().p_3().text_color(rgb(p.text_muted)).child(
                if state.cards.is_empty() {
                    "尚无记忆。在对话中说“记住我的偏好”，或在记忆设置中整理已有对话。"
                } else {
                    "没有匹配的记忆，试试其他关键词或切换范围。"
                },
            ));
        }
        for card in filtered {
            let id = card.id;
            let selected = state
                .selected
                .as_ref()
                .is_some_and(|record| record.card.id == id);
            list = list.child(
                div()
                    .id(SharedString::from(format!("memory:{id}")))
                    .p_3()
                    .flex_shrink_0()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(if selected { p.accent } else { p.border }))
                    .bg(rgb(if selected { p.accent_tint } else { p.panel }))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.surface_subtle)))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(card.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.text_muted))
                                    .child(memory_scope(card)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(preview_line(&card.summary, 85)),
                    )
                    .when(!card.category.is_empty(), |row| {
                        row.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child(card.category.clone()),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.select_memory(id, cx))),
            );
        }
        let sidebar = div()
            .w(gpui::relative(0.36))
            .min_w(px(240.))
            .max_w(px(380.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .min_h_0()
            .border_r_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .p_3()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        SearchInput::new("搜索记忆")
                            .control(state.search.clone())
                            .theme(self.theme)
                            .render()
                            .w_full(),
                    )
                    .child(
                        SegmentedControl::new([
                            SegmentOption::new("全部"),
                            SegmentOption::new("项目"),
                            SegmentOption::new("全局"),
                        ])
                        .id("memory-scope")
                        .selected(state.scope)
                        .theme(self.theme)
                        .on_change(move |index, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.memory_ui.scope = index;
                                cx.notify();
                            });
                        })
                        .render()
                        .w_full(),
                    ),
            )
            .child(list);
        let mut detail = div()
            .id("memory-detail")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(p.panel));
        if let Some(record) = &state.selected {
            let id = record.card.id;
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
                            match result {
                                Ok(()) => {
                                    this.memory_ui.selected = None;
                                    this.memory_ui.notice =
                                        Some("记忆已删除，原始会话已保留。".into());
                                }
                                Err(error) => this.memory_ui.notice = Some(error),
                            }
                            this.reload_memories(cx);
                        })),
                );
            let weak = cx.entity().downgrade();
            detail = detail.child(
                div()
                    .p_4()
                    .flex_shrink_0()
                    .border_b_1()
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
                                    .min_w_0()
                                    .text_size(px(18.))
                                    .text_ellipsis()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(record.card.title.clone()),
                            )
                            .child(
                                Popover::new(
                                    SharedString::from(format!("delete-memory:{id}")),
                                    IconButton::new(Icon::Trash, "删除记忆")
                                        .plain()
                                        .theme(self.theme)
                                        .render(),
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
                            )
                            .child(
                                IconButton::new(Icon::Close, "关闭记忆详情")
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.memory_ui.selected = None;
                                        this.memory_ui.confirming = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(format!(
                                "{} · {}",
                                memory_scope(&record.card),
                                record.card.category
                            )),
                    ),
            );
            detail = detail.child(
                div()
                    .id("memory-detail-scroll")
                    .track_scroll(&state.detail_scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_6()
                    .child(
                        div()
                            .mb_3()
                            .text_color(rgb(p.text_muted))
                            .child(record.card.summary.clone()),
                    )
                    .child(
                        div()
                            .mb_4()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!("更新于 {}", record.card.updated_at)),
                    )
                    .child(self.markdown_text(
                        "memory-settings-preview".into(),
                        &record.content,
                        cx,
                    ))
                    .when(!record.keywords.is_empty(), |body| {
                        body.child(
                            div()
                                .mt_4()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(format!("关键词：{}", record.keywords.join("、"))),
                        )
                    })
                    .child(
                        div()
                            .mt_6()
                            .pt_4()
                            .border_t_1()
                            .border_color(rgb(p.border))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(div().font_weight(FontWeight::MEDIUM).child("来源记录"))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child(
                                        "记忆来源于历史对话，可能随时间变化；原始对话保留在本机。",
                                    ),
                            )
                            .child(
                                div()
                                    .p_3()
                                    .rounded(px(8.))
                                    .bg(rgb(p.surface_subtle))
                                    .text_size(px(11.))
                                    .font_family("monospace")
                                    .child(
                                        serde_json::to_string_pretty(&record.source)
                                            .unwrap_or_default(),
                                    ),
                            ),
                    ),
            );
        } else {
            detail = detail
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(Icon::Database.render(28.).text_color(rgb(p.text_muted)))
                .child(div().font_weight(FontWeight::MEDIUM).child("选择一条记忆"))
                .child(
                    div()
                        .text_color(rgb(p.text_muted))
                        .child("在左侧选择记忆，在这里查看正文和来源。"),
                );
        }
        div()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(header)
            .when(state.show_settings, |page| {
                page.child(self.memory_preferences(cx).flex_shrink_0())
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .child(sidebar)
                    .child(detail),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_filters_combine_scope_with_title_summary_and_category() {
        let mut card = memory::Card {
            id: Uuid::new_v4(),
            title: "GPUI layout".into(),
            summary: "自动滚动".into(),
            category: "交互".into(),
            project: None,
            updated_at: String::new(),
            reason: String::new(),
        };
        assert!(matches_memory(&card, "gpui", 0));
        assert!(matches_memory(&card, "滚动", 2));
        assert!(matches_memory(&card, "交互", 0));
        assert!(!matches_memory(&card, "", 1));
        assert!(!matches_memory(&card, "missing", 0));
        card.project = Some(PathBuf::from("/project"));
        assert!(matches_memory(&card, "gpui", 1));
        assert!(!matches_memory(&card, "gpui", 2));
    }

    struct ControlsPreview {
        time: Entity<TextEntry>,
        width: f32,
    }
    impl Render for ControlsPreview {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(self.width)).child(memory_controls(
                self.time.clone(),
                ThemePreset::default(),
                Button::new("保存记忆设置").kind(ButtonKind::Gray).render(),
                Button::new("正在整理…").disabled().render(),
            ))
        }
    }

    #[gpui::test]
    fn time_and_buttons_do_not_overlap_and_wrap_in_narrow_layout(cx: &mut gpui::TestAppContext) {
        let (preview, window) = cx.add_window_view(|_, cx| ControlsPreview {
            time: cx.new(|cx| TextEntry::new(cx, "HH:MM", "23:00")),
            width: 640.,
        });
        let field = window.debug_bounds("memory-time-field").unwrap();
        let save = window.debug_bounds("memory-save-control").unwrap();
        let run = window.debug_bounds("memory-run-control").unwrap();
        assert_eq!(field.size.width, px(180.));
        assert!(field.right() < save.left());
        assert!(save.right() < run.left());
        assert_eq!(field.bottom(), save.bottom());
        preview.update(window, |preview, cx| {
            preview.width = 240.;
            cx.notify();
        });
        let field = window.debug_bounds("memory-time-field").unwrap();
        let save = window.debug_bounds("memory-save-control").unwrap();
        let run = window.debug_bounds("memory-run-control").unwrap();
        assert_eq!(field.size.width, px(180.));
        assert!(field.bottom() < save.top());
        assert!(save.right() < run.left() || save.bottom() < run.top());
    }
}
