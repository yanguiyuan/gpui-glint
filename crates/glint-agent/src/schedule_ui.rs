use super::*;
use chrono::{Datelike, Local, TimeZone};
use gpui_glint::components::{
    DatePicker, InputField, PromptEditor, SegmentOption, SegmentedControl,
};

pub struct ScheduleEditor {
    pub open: bool,
    pub editing: Option<Uuid>,
    pub title: Entity<TextEntry>,
    pub prompt: Entity<PromptEditor>,
    pub date: Entity<DatePicker>,
    pub time: Entity<TextEntry>,
    pub repeat: usize,
    pub wake_mode: usize,
    pub notice: Option<String>,
}

impl ScheduleEditor {
    pub fn new(cx: &mut Context<AgentWorkspace>) -> Self {
        Self {
            open: false,
            editing: None,
            title: cx.new(|cx| TextEntry::new(cx, "任务标题", "")),
            prompt: cx.new(|cx| PromptEditor::new(cx, "到时间后让 Agent 执行什么？").min_rows(3)),
            date: cx.new(|cx| DatePicker::new(cx).selected(Local::now().date_naive())),
            time: cx.new(|cx| TextEntry::new(cx, "HH:MM", "09:00")),
            repeat: 0,
            wake_mode: 0,
            notice: None,
        }
    }
}

impl AgentWorkspace {
    fn partner_repo_name(&self, id: Uuid) -> Option<String> {
        partners::Repository::new(self.path.clone())
            .list()
            .ok()?
            .into_iter()
            .find(|p| p.id == id)
            .map(|p| p.name)
    }
    pub fn refresh_calendar(&self, cx: &mut Context<Self>) {
        let mut markers = std::collections::HashMap::new();
        if let Ok(mut store) = self.schedules.lock() {
            let _ = store.refresh();
            for task in &store.tasks {
                *markers.entry(task.run_at.date_naive()).or_insert(0) += 1;
                if let Some(last) = task
                    .last_run
                    .filter(|last| last.date_naive() != task.run_at.date_naive())
                {
                    *markers.entry(last.date_naive()).or_insert(0) += 1;
                }
            }
        }
        self.calendar.update(cx, |calendar, cx| {
            calendar.set_markers(markers, cx);
            calendar.set_theme(self.theme, cx);
        });
    }

    fn new_schedule(&mut self, cx: &mut Context<Self>) {
        let date = self
            .calendar
            .read(cx)
            .value()
            .unwrap_or_else(|| Local::now().date_naive());
        self.schedule_editor.open = true;
        self.schedule_editor.editing = None;
        self.schedule_editor.repeat = 0;
        self.schedule_editor.wake_mode = 0;
        self.schedule_editor.notice = None;
        self.schedule_editor
            .title
            .update(cx, |entry, cx| entry.set_value("", cx));
        self.schedule_editor
            .prompt
            .update(cx, |entry, cx| entry.set_value("", cx));
        self.schedule_editor
            .date
            .update(cx, |entry, cx| entry.set_selected(Some(date), cx));
        cx.notify();
    }

    pub(crate) fn edit_schedule(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let task = self
            .schedules
            .lock()
            .ok()
            .and_then(|store| store.tasks.iter().find(|task| task.id == id).cloned());
        if let Some(task) = task {
            self.schedule_editor.open = true;
            self.schedule_editor.editing = Some(id);
            self.schedule_editor.wake_mode =
                usize::from(task.wake_mode == schedules::WakeMode::Reflect);
            self.schedule_editor.notice = None;
            self.schedule_editor.repeat = match task.repeat {
                schedules::Repeat::Once => 0,
                schedules::Repeat::Daily => 1,
                schedules::Repeat::Weekly => 2,
            };
            self.schedule_editor
                .title
                .update(cx, |entry, cx| entry.set_value(task.title, cx));
            self.schedule_editor
                .prompt
                .update(cx, |entry, cx| entry.set_value(task.prompt, cx));
            self.schedule_editor.date.update(cx, |entry, cx| {
                entry.set_selected(Some(task.run_at.date_naive()), cx)
            });
            self.schedule_editor.time.update(cx, |entry, cx| {
                entry.set_value(task.run_at.format("%H:%M").to_string(), cx)
            });
        }
        cx.notify();
    }

    fn save_schedule(&mut self, cx: &mut Context<Self>) {
        let result = (|| {
            let date = self
                .schedule_editor
                .date
                .read(cx)
                .value()
                .ok_or("请选择日期")?;
            let time = chrono::NaiveTime::parse_from_str(
                self.schedule_editor.time.read(cx).value().trim(),
                "%H:%M",
            )
            .map_err(|_| "时间格式应为 HH:MM")?;
            let run_at = Local
                .from_local_datetime(&date.and_time(time))
                .single()
                .ok_or("该本地时间无效或有歧义")?;
            let mut store = self.schedules.lock().map_err(|_| "日程存储不可用")?;
            let existing = self
                .schedule_editor
                .editing
                .and_then(|id| store.tasks.iter().find(|task| task.id == id).cloned());
            if existing.as_ref().is_some_and(|task| task.running) {
                return Err("请等待任务执行完成".into());
            }
            let mut task = existing.unwrap_or_else(|| schedules::Schedule {
                wake_mode: schedules::WakeMode::Work,
                self_wake: false,
                partner_id: self.history.active().partner_id,
                partner_goal: None,
                id: Uuid::new_v4(),
                title: String::new(),
                prompt: String::new(),
                run_at,
                repeat: schedules::Repeat::Once,
                enabled: true,
                project: self.history.active().project.clone(),
                provider_id: self.catalog.active,
                model: self.catalog.active_model.clone(),
                running: false,
                last_run: None,
                last_error: None,
            });
            task.title = self
                .schedule_editor
                .title
                .read(cx)
                .value()
                .trim()
                .to_owned();
            task.prompt = self
                .schedule_editor
                .prompt
                .read(cx)
                .value()
                .trim()
                .to_owned();
            task.run_at = run_at;
            task.enabled = true;
            task.wake_mode = if self.schedule_editor.wake_mode == 1 {
                schedules::WakeMode::Reflect
            } else {
                schedules::WakeMode::Work
            };
            task.repeat = match self.schedule_editor.repeat {
                1 => schedules::Repeat::Daily,
                2 => schedules::Repeat::Weekly,
                _ => schedules::Repeat::Once,
            };
            store.upsert(task)?;
            Ok::<_, String>(date)
        })();
        match result {
            Ok(date) => {
                self.schedule_editor.open = false;
                self.calendar
                    .update(cx, |calendar, cx| calendar.set_selected(date, cx));
                self.refresh_calendar(cx);
            }
            Err(error) => self.schedule_editor.notice = Some(error),
        }
        cx.notify();
    }

    pub fn calendar_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let date = self
            .calendar
            .read(cx)
            .value()
            .unwrap_or_else(|| Local::now().date_naive());
        let mut list = div()
            .id("scheduled-tasks")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_3();
        let tasks = self
            .schedules
            .lock()
            .map(|store| {
                store
                    .tasks
                    .iter()
                    .filter(|task| {
                        task.run_at.date_naive() == date
                            || task.last_run.is_some_and(|last| last.date_naive() == date)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if tasks.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_color(rgb(p.text_muted))
                    .child("这一天没有定时任务"),
            );
        }
        for task in tasks {
            let id = task.id;
            list = list.child(
                div()
                    .id(gpui::SharedString::from(format!("schedule-{id}")))
                    .px_3()
                    .py_3()
                    .mb_2()
                    .rounded(px(8.))
                    .bg(rgb(p.surface_subtle))
                    .cursor_pointer()
                    .child(div().font_weight(FontWeight::MEDIUM).child(task.title))
                    .child(
                        div()
                            .pt_1()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!(
                                "{} · {}",
                                task.run_at.format("%H:%M"),
                                if task.running {
                                    "执行中"
                                } else if task.last_error.is_some() {
                                    "执行失败"
                                } else if !task.enabled && task.last_run.is_some() {
                                    "已执行"
                                } else if !task.enabled {
                                    "已暂停"
                                } else {
                                    "已启用"
                                }
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!(
                                "{} · {}",
                                task.wake_mode.label(),
                                task.partner_id
                                    .and_then(|id| self.partner_repo_name(id))
                                    .unwrap_or_else(|| "普通 Agent".into())
                            )),
                    )
                    .when_some(task.project.as_ref(), |row, path| {
                        row.child(
                            div()
                                .pt_1()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child(
                                    path.file_name()
                                        .unwrap_or(path.as_os_str())
                                        .to_string_lossy()
                                        .into_owned(),
                                ),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.edit_schedule(id, cx))),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .child(
                div()
                    .px_4()
                    .py_4()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{} 月 {} 日", date.month(), date.day())),
                    )
                    .child(
                        IconButton::new(Icon::Plus, "新建定时任务")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.new_schedule(cx))),
                    ),
            )
            .child(list)
            .child(
                div()
                    .p_3()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("应用运行时执行 · 自动保存到本机"),
            )
    }

    pub fn calendar_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let mut page = div()
            .w_full()
            .max_w(px(980.))
            .mx_auto()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(26.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("日程"),
                    )
                    .child(
                        Button::new(if self.schedule_editor.open {
                            "返回日历"
                        } else {
                            "新建任务"
                        })
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.schedule_editor.open {
                                this.schedule_editor.open = false;
                                cx.notify();
                            } else {
                                this.new_schedule(cx);
                            }
                        })),
                    ),
            );
        if self.schedule_editor.open {
            let editor = &self.schedule_editor;
            for entry in [&editor.title, &editor.time] {
                entry.update(cx, |entry, cx| entry.set_theme(self.theme, cx));
            }
            editor
                .prompt
                .update(cx, |entry, cx| entry.set_theme(self.theme, cx));
            editor
                .date
                .update(cx, |entry, cx| entry.set_theme(self.theme, cx));
            let weak = cx.entity().downgrade();
            page=page.child(InputField::new("").label("任务名称").control(editor.title.clone()).theme(self.theme).render().w_full())
                .child(div().rounded(px(8.)).border_1().border_color(rgb(p.border)).p_3().child(editor.prompt.clone()))
                .child(div().flex().items_end().gap_3().child(editor.date.clone()).child(InputField::new("").label("本地时间").control(editor.time.clone()).theme(self.theme).render()))
                .child(SegmentedControl::new([SegmentOption::new("执行任务"),SegmentOption::new("回顾整理")]).id("schedule-wake-mode").selected(editor.wake_mode).theme(self.theme).on_change({let weak=weak.clone(); move |index,_,cx|{let _=weak.update(cx,|this,cx|{this.schedule_editor.wake_mode=index;cx.notify();});}}).render())
                .child(SegmentedControl::new([SegmentOption::new("一次"),SegmentOption::new("每天"),SegmentOption::new("每周")]).id("schedule-repeat").selected(editor.repeat).theme(self.theme)
                    .on_change(move |index,_,cx|{let _=weak.update(cx,|this,cx|{this.schedule_editor.repeat=index;cx.notify();});}).render())
                .when_some(editor.notice.clone(),|page,message|page.child(div().text_color(rgb(0xd9342b)).child(message)))
                .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("使用创建时的项目、供应商与模型。应用关闭期间不会执行；重新打开后补执行到期任务一次。"))
                .child(div().flex().gap_3().child(Button::new("保存任务").theme(self.theme).render().on_click(cx.listener(|this,_,_,cx|this.save_schedule(cx))))
                    .when_some(editor.editing,|row,id|row.child(Button::new("删除任务").kind(ButtonKind::DangerTinted).theme(self.theme).render().on_click(cx.listener(move |this,_,_,cx|{
                        let result=this.schedules.lock().map_err(|_|"日程存储不可用".to_owned()).and_then(|mut store|store.remove(id));
                        match result {Ok(())=>{this.schedule_editor.open=false;this.refresh_calendar(cx);},Err(error)=>this.schedule_editor.notice=Some(error)} cx.notify();
                    })))));
        } else {
            page = page.child(self.calendar.clone());
        }
        div().size_full().child(
            div()
                .id("calendar-page")
                .size_full()
                .overflow_y_scroll()
                .p_6()
                .child(page),
        )
    }

    pub fn start_scheduler(&mut self, cx: &mut Context<Self>) {
        self.scheduler_task = Some(cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                if weak
                    .update(cx, |this, cx| {
                        if this.pending.is_some() || this.preparing.is_some() {
                            return;
                        }
                        let due = this
                            .schedules
                            .lock()
                            .map_err(|_| "日程存储不可用".to_owned())
                            .and_then(|mut store| {
                                store.refresh()?;
                                store.claim_due(Local::now())
                            });
                        match due {
                            Ok(Some(task)) => {
                                let config = match task.provider_id {
                                    Some(id) => this
                                        .catalog
                                        .providers
                                        .iter()
                                        .find(|provider| provider.id == id)
                                        .cloned(),
                                    None => this.catalog.active.map(|_| this.provider.clone()),
                                };
                                if let Some(mut config) = config {
                                    if !task.model.is_empty() {
                                        config.model = task.model.clone();
                                    }
                                    let mut session = model::Session::new();
                                    session.project = task.project.clone();
                                    session.partner_id = task.partner_id;
                                    session.partner_goal = task.partner_goal;
                                    session.wake_mode = Some(task.wake_mode);
                                    session.title = format!("定时任务 · {}", task.title);
                                    let id = session.id;
                                    this.history.sessions.insert(0, session);
                                    this.running_schedule = Some(task.id);
                                    this.send_to(
                                        id,
                                        ComposerSubmission::text(format!("本次是日程唤醒，模式：{}。唤醒目的：{}\n先回顾自己的常驻记忆、近期经历和未完成目标，再判断现在是否仍需推进。完成后记录结果；若有必要，可用 schedule_create 自己选择下一次唤醒的时间、目的与 wake_mode，避免无进展的重复唤醒。{}", task.wake_mode.label(),task.prompt,if task.wake_mode==schedules::WakeMode::Reflect {"本次只允许阅读、回顾和更新自身记忆，以及安排自己的后续唤醒。"}else{"沿用当前工具权限执行实际工作。"})),
                                        config,
                                        false,
                                        cx,
                                    );
                                    if let Some(session) = this
                                        .history
                                        .sessions
                                        .iter_mut()
                                        .find(|session| session.id == id)
                                    {
                                        session.title = format!("定时任务 · {}", task.title);
                                    }
                                    this.save(cx);
                                    if this.pending.is_none() && this.preparing.is_none() {
                                        let error = this
                                            .provider_error
                                            .clone()
                                            .unwrap_or_else(|| "无法启动任务".into());
                                        let _ = this
                                            .schedules
                                            .lock()
                                            .map(|mut store| store.finish(task.id, Some(error)));
                                        this.running_schedule = None;
                                    }
                                } else {
                                    let _ = this.schedules.lock().map(|mut store| {
                                        store.finish(task.id, Some("未配置可用的供应商".into()))
                                    });
                                }
                                this.refresh_calendar(cx);
                                this.save(cx);
                                cx.notify();
                            }
                            Err(error) => {
                                this.schedule_editor.notice = Some(error);
                            }
                            Ok(None) => this.tick_memory(false, cx),
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
}
