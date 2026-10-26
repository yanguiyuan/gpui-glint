use gpui::StatefulInteractiveElement;
use gpui::{
    AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Subscription, Window, div, px, rgb,
};
use gpui_glint::components::{
    Button, ButtonKind, Calendar, CalendarEvent, Checkbox, Composer, ComposerChip,
    ComposerContextEntry, ComposerContextSection, ComposerEvent, Date, DatePicker, DatePickerEvent,
    DateRangePicker, DateTimePicker, DateTimeRangePicker, GroupedDropdown, Icon, InputField,
    RadioOption, SearchInput, SegmentOption, SegmentedControl, Select, SelectEvent, SelectField,
    SelectFieldEvent, SelectOption, Slider, Stepper, Switch, TextEntry, ThemeSelector,
    ThemeSelectorEvent, Time, TimePicker, TimeRangePicker, Toast, ToastHost, ToastKind, TokenInput,
};
use gpui_glint::theme::ThemePreset;

use super::common::{page, panel};

pub struct InputsPage {
    pub theme: ThemePreset,
    sort_dropdown: Entity<GroupedDropdown>,
    slider: Entity<Slider>,
    search_input: Entity<TextEntry>,
    name_input: Entity<TextEntry>,
    error_input: Entity<TextEntry>,
    selects: Vec<Entity<Select>>,
    compound: Entity<SelectField>,
    composer: Entity<Composer>,
    calendar: Entity<Calendar>,
    theme_selector: Entity<ThemeSelector>,
    dates: Vec<Entity<DatePicker>>,
    time: Entity<TimePicker>,
    datetime: Entity<DateTimePicker>,
    date_range: Entity<DateRangePicker>,
    time_range: Entity<TimeRangePicker>,
    datetime_range: Entity<DateTimeRangePicker>,
    toasts: Entity<ToastHost>,
    _select_subscriptions: Vec<Subscription>,
    media_type: usize,
    view_mode: usize,
    show_seen: bool,
    partial_selected: bool,
    only_unfinished: bool,
    continuous: bool,
    auto_locate: bool,
    per_page: i32,
    tags: Vec<&'static str>,
}

impl EventEmitter<ThemeSelectorEvent> for InputsPage {}

impl InputsPage {
    pub fn new(
        theme: ThemePreset,
        sort_dropdown: Entity<GroupedDropdown>,
        slider: Entity<Slider>,
        search_input: Entity<TextEntry>,
        name_input: Entity<TextEntry>,
        error_input: Entity<TextEntry>,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = vec![
            SelectOption::new("dev", "开发环境"),
            SelectOption::new("test", "测试环境"),
            SelectOption::new("staging", "预发布环境").disabled(true),
            SelectOption::new("prod", "生产环境"),
        ];
        let selects = vec![
            cx.new(|cx| {
                Select::new(options.clone(), cx)
                    .selected("dev")
                    .theme(theme)
            }),
            cx.new(|cx| {
                Select::new(options.clone(), cx)
                    .selected("prod")
                    .searchable(true)
                    .theme(theme)
            }),
            cx.new(|cx| {
                Select::new(options, cx)
                    .placeholder("暂无权限")
                    .disabled(true)
                    .theme(theme)
            }),
        ];
        let mut subscriptions: Vec<Subscription> = selects
            .iter()
            .map(|select| {
                cx.subscribe(select, |_, select, event: &SelectEvent, cx| {
                    let SelectEvent::Changed(value) = event;
                    select.update(cx, |select, cx| {
                        select.set_selected(Some(value.clone()), cx)
                    });
                })
            })
            .collect();
        let compound = cx.new(|cx| {
            let mut field = SelectField::new(
                ["GET", "POST", "PUT", "PATCH", "DELETE"].map(|method| {
                    SelectOption::new(method, method).color(match method {
                        "GET" => 0x24864c,
                        "POST" => 0xb37716,
                        "DELETE" => 0xd9342b,
                        "PATCH" => 0x876ab4,
                        _ => 0x007aff,
                    })
                }),
                "输入请求 URL 或路径",
                cx,
            );
            field.set_text("{{base_url}}/v1/resources", cx);
            field.set_theme(theme, cx);
            field
        });
        subscriptions.push(cx.subscribe(&compound, |_, _, _: &SelectFieldEvent, _| {}));
        let composer = cx.new(|cx| {
            Composer::new("提出后续修改要求…", cx)
                .theme(theme)
                .chip(ComposerChip::new("attach").icon(Icon::Plus))
                .chip(
                    ComposerChip::new("access")
                        .icon(Icon::Shield)
                        .option("normal", "普通访问")
                        .option("full", "完全访问")
                        .default_selected("full")
                        .tint(0xb37716)
                        .chevron(),
                )
                .menu_chip(
                    ComposerChip::new("model")
                        .option("glm-5.3-flash", "GLM-5.3-Flash")
                        .option("glm-5", "GLM-5")
                        .option("glm-4.6", "GLM-4.6")
                        .default_selected("glm-5.3-flash")
                        .chevron(),
                )
                .menu_chip(
                    ComposerChip::new("effort")
                        .icon(Icon::Pulse)
                        .option("low", "低")
                        .option("high", "高")
                        .option("max", "最高")
                        .default_selected("max")
                        .chevron(),
                )
                .context_panel(
                    "attach",
                    "添加",
                    [
                        ComposerContextSection::untitled([
                            ComposerContextEntry::new("attachment", "附件").icon(Icon::Link),
                            ComposerContextEntry::new("workflow", "工作流")
                                .icon(Icon::Workflow)
                                .color(0x24864c),
                        ]),
                        ComposerContextSection::new(
                            "插件",
                            [
                                ComposerContextEntry::new("plugin-creator", "插件创建器")
                                    .icon(Icon::Settings)
                                    .color(0x007aff)
                                    .description(
                                        "开发、校验 ZCode 插件，完成本地 dev 市场安装、试用与更新。",
                                    ),
                                ComposerContextEntry::new("skill-creator", "技能创建器")
                                    .icon(Icon::Script)
                                    .color(0xb37716)
                                    .description("创建、编辑和验证可复用的 ZCode 技能。"),
                                ComposerContextEntry::new("guide", "ZCode 使用指南")
                                    .icon(Icon::ApiDoc)
                                    .description("提供 ZCode 配置指南与插件、技能、MCP、命令和钩子诊断。"),
                                ComposerContextEntry::new("pdf", "PDF")
                                    .icon(Icon::ApiDoc)
                                    .color(0xd9342b)
                                    .description("创建、编辑与审阅 PDF。"),
                            ],
                        ),
                    ],
                )
                .context_hints([
                    ("@", "添加上下文"),
                    ("/", "选择能力"),
                    ("$", "选择技能"),
                ])
        });
        subscriptions.push(cx.subscribe(
            &composer,
            |this, _, event: &ComposerEvent, cx| match event {
                ComposerEvent::Submitted(text) => {
                    this.composer
                        .update(cx, |composer, cx| composer.set_busy(true, cx));
                    this.toasts.update(cx, |host, cx| {
                        host.push(
                            Toast::new("已提交提示词")
                                .kind(ToastKind::Success)
                                .description(text.to_string()),
                            cx,
                        );
                    });
                    let timer = cx
                        .background_executor()
                        .timer(std::time::Duration::from_millis(1200));
                    cx.spawn(async move |this, cx| {
                        timer.await;
                        let _ = this.update(cx, |this, cx| {
                            this.composer
                                .update(cx, |composer, cx| composer.set_busy(false, cx));
                        });
                    })
                    .detach();
                }
                ComposerEvent::Queued(text) => {
                    this.toasts.update(cx, |host, cx| {
                        host.push(
                            Toast::new("已加入队列")
                                .kind(ToastKind::Info)
                                .description(text.to_string()),
                            cx,
                        );
                    });
                }
                ComposerEvent::ChipSelected { chip, value } => {
                    this.toasts.update(cx, |host, cx| {
                        host.push(
                            Toast::new(format!("{chip} = {value}")).kind(ToastKind::Info),
                            cx,
                        );
                    });
                }
                ComposerEvent::ContextSelected { id, label } => {
                    this.toasts.update(cx, |host, cx| {
                        host.push(
                            Toast::new(format!("已插入 @{label}"))
                                .kind(ToastKind::Info)
                                .description(id.to_string()),
                            cx,
                        );
                    });
                }
                ComposerEvent::ChipClicked(id) => {
                    this.toasts.update(cx, |host, cx| {
                        host.push(
                            Toast::new(format!("点击了 {}", id)).kind(ToastKind::Info),
                            cx,
                        );
                    });
                }
                ComposerEvent::Changed(_) => {}
            },
        ));
        let theme_selector = cx.new(|cx| ThemeSelector::new(cx).selected(theme).theme(theme));
        subscriptions.push(cx.subscribe(
            &theme_selector,
            |_, _, event: &ThemeSelectorEvent, cx| {
                cx.emit(event.clone());
            },
        ));
        let dates = vec![
            cx.new(|cx| DatePicker::new(cx).theme(theme)),
            cx.new(|cx| {
                DatePicker::new(cx)
                    .theme(theme)
                    .range(
                        Date::from_ymd_opt(2026, 10, 1),
                        Date::from_ymd_opt(2026, 10, 31),
                    )
                    .selected(Date::from_ymd_opt(2026, 10, 4).unwrap())
            }),
            cx.new(|cx| DatePicker::new(cx).theme(theme).disabled(true)),
        ];
        let toasts = cx.new(|_| ToastHost::new().theme(theme));
        for date in &dates {
            subscriptions.push(cx.subscribe(date, |this, _, event: &DatePickerEvent, cx| {
                let DatePickerEvent::Changed(value) = event;
                let message = value
                    .map(|d| format!("已选择 {d}"))
                    .unwrap_or_else(|| "已清空日期".into());
                this.toasts.update(cx, |host, cx| {
                    host.push(Toast::new(message).kind(ToastKind::Success), cx);
                });
            }));
        }
        let time = cx.new(|cx| {
            TimePicker::new(cx)
                .selected(Time::from_hms_opt(9, 30, 0).unwrap())
                .theme(theme)
        });
        let calendar = cx.new(|cx| Calendar::new(cx).theme(theme).day_height(56.));
        subscriptions.push(
            cx.subscribe(&calendar, |this, _, event: &CalendarEvent, cx| {
                if let CalendarEvent::DateSelected(date) = event {
                    this.toasts.update(cx, |host, cx| {
                        host.push(Toast::new(format!("已选择 {date}")), cx)
                    });
                }
            }),
        );
        let datetime = cx.new(|cx| DateTimePicker::new(cx).theme(theme, cx));
        let date_range = cx.new(|cx| DateRangePicker::new(cx).theme(theme));
        let time_range = cx.new(|cx| TimeRangePicker::new(cx).theme(theme));
        let datetime_range = cx.new(|cx| DateTimeRangePicker::new(cx).theme(theme));
        Self {
            calendar,
            time,
            datetime,
            date_range,
            time_range,
            datetime_range,
            theme,
            sort_dropdown,
            slider,
            search_input,
            name_input,
            error_input,
            selects,
            compound,
            composer,
            theme_selector,
            dates,
            toasts,
            _select_subscriptions: subscriptions,
            media_type: 1,
            view_mode: 1,
            show_seen: true,
            partial_selected: false,
            only_unfinished: false,
            continuous: false,
            auto_locate: true,
            per_page: 24,
            tags: vec!["夜景", "长曝光"],
        }
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.calendar
            .update(cx, |calendar, cx| calendar.set_theme(theme, cx));
        self.compound
            .update(cx, |field, cx| field.set_theme(theme, cx));
        self.composer
            .update(cx, |composer, cx| composer.set_theme(theme, cx));
        self.theme = theme;
        self.sort_dropdown
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.slider
            .update(cx, |input, cx| input.set_theme(theme, cx));
        for input in [&self.search_input, &self.name_input, &self.error_input] {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
        self.theme_selector.update(cx, |selector, cx| {
            selector.set_selected(theme, cx);
            selector.set_theme(theme, cx);
        });
        cx.notify();
    }

    pub fn close_menu(&mut self, cx: &mut Context<Self>) {
        for date in &self.dates {
            date.update(cx, |date, cx| date.close(cx));
        }
        for select in &self.selects {
            select.update(cx, |select, cx| select.close(cx));
        }
        self.theme_selector
            .update(cx, |selector, cx| selector.close(cx));
        self.sort_dropdown.update(cx, |dropdown, cx| {
            dropdown.close();
            cx.notify();
        });
    }
}

impl Render for InputsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sort_dropdown
            .update(cx, |input, cx| input.set_theme(self.theme, cx));
        self.slider
            .update(cx, |input, cx| input.set_theme(self.theme, cx));
        for input in [&self.search_input, &self.name_input, &self.error_input] {
            input.update(cx, |input, cx| input.set_theme(self.theme, cx));
        }
        for select in &self.selects {
            if select.read(cx).theme_preset() != self.theme {
                select.update(cx, |select, cx| select.set_theme(self.theme, cx));
            }
        }
        self.theme_selector
            .update(cx, |selector, cx| selector.set_theme(self.theme, cx));
        for date in &self.dates {
            date.update(cx, |date, cx| date.set_theme(self.theme, cx));
        }
        self.toasts
            .update(cx, |host, cx| host.set_theme(self.theme, cx));
        self.time.update(cx, |p, cx| p.set_theme(self.theme, cx));
        self.datetime
            .update(cx, |p, cx| p.set_theme(self.theme, cx));
        self.date_range
            .update(cx, |p, cx| p.set_theme(self.theme, cx));
        self.time_range
            .update(cx, |p, cx| p.set_theme(self.theme, cx));
        self.datetime_range
            .update(cx, |p, cx| p.set_theme(self.theme, cx));
        let temporal_examples = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(labeled("TimePicker · 时分秒", self.time.clone()))
            .child(labeled("DateTimePicker · 日期时间", self.datetime.clone()))
            .child(labeled(
                "DateRangePicker · 日期范围",
                self.date_range.clone(),
            ))
            .child(labeled(
                "TimeRangePicker · 时间范围",
                self.time_range.clone(),
            ))
            .child(labeled(
                "DateTimeRangePicker · 日期时间范围",
                self.datetime_range.clone(),
            ));
        let date_examples = div()
            .flex()
            .flex_wrap()
            .gap_5()
            .child(labeled("DatePicker · 默认", self.dates[0].clone()))
            .child(labeled("DatePicker · 限定十月", self.dates[1].clone()))
            .child(labeled("DatePicker · 禁用", self.dates[2].clone()));
        let mut toast_examples = div().flex().flex_wrap().gap_3();
        for (label, kind) in [
            ("信息提示", ToastKind::Info),
            ("保存成功", ToastKind::Success),
            ("修改未保存", ToastKind::Warning),
            ("导出失败", ToastKind::Error),
        ] {
            toast_examples = toast_examples.child(
                Button::new(label)
                    .theme(self.theme)
                    .kind(ButtonKind::Glass)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toasts.update(cx, |host, cx| {
                            host.push(
                                Toast::new(label)
                                    .kind(kind)
                                    .description("悬停暂停计时，可手动关闭"),
                                cx,
                            );
                        });
                    })),
            );
        }
        let select_examples = div()
            .flex()
            .flex_wrap()
            .gap_5()
            .child(labeled("Select · 默认", self.selects[0].clone()))
            .child(labeled("Select · 可搜索", self.selects[1].clone()))
            .child(labeled("Select · 禁用", self.selects[2].clone()))
            .child(labeled(
                "ThemeSelector · 主题预览",
                self.theme_selector.clone(),
            ));
        let compound = self.compound.clone();
        let select_examples = select_examples.child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(labeled("选择输入框 · 方法 + 路径", self.compound.clone())),
                )
                .child(
                    Button::new("发送")
                        .theme(self.theme)
                        .kind(ButtonKind::Primary)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let value = compound.read(cx).value().clone();
                            if value.text.trim().is_empty() {
                                compound.update(cx, |field, cx| {
                                    field.set_error(Some("请输入请求 URL 或路径。".into()), cx)
                                });
                                return;
                            }
                            compound.update(cx, |field, cx| field.set_error(None, cx));
                            this.toasts.update(cx, |host, cx| {
                                host.push(
                                    Toast::new(format!(
                                        "演示：{} {}",
                                        value
                                            .selected
                                            .as_ref()
                                            .map(|value| value.to_string())
                                            .unwrap_or_default(),
                                        value.text
                                    )),
                                    cx,
                                );
                            });
                        })),
                ),
        );
        let page_entity = cx.entity();
        let search = SearchInput::new("搜索集合或资源")
            .theme(self.theme)
            .shortcut("⌘K")
            .width(300.)
            .control(self.search_input.clone())
            .render();
        let name = InputField::new("输入以筛选…")
            .theme(self.theme)
            .label("名称")
            .width(300.)
            .control(self.name_input.clone())
            .render();
        let error = InputField::new("")
            .theme(self.theme)
            .label("错误态")
            .value("不存在的集合 ID")
            .error("找不到该集合，请检查链接")
            .width(300.)
            .control(self.error_input.clone())
            .render();
        let media_page = page_entity.clone();
        let media_type = SegmentedControl::new([
            SegmentOption::new("全部"),
            SegmentOption::new("视频").count("109"),
            SegmentOption::new("图集").count("125"),
            SegmentOption::new("文本").count("19"),
        ])
        .theme(self.theme)
        .selected(self.media_type)
        .on_change(move |index, _, app| {
            media_page.update(app, |this, cx| {
                this.media_type = index;
                cx.notify();
            });
        })
        .render();
        let view_page = page_entity.clone();
        let view_mode =
            SegmentedControl::new([SegmentOption::new("网格"), SegmentOption::new("列表")])
                .theme(self.theme)
                .selected(self.view_mode)
                .on_change(move |index, _, app| {
                    view_page.update(app, |this, cx| {
                        this.view_mode = index;
                        cx.notify();
                    });
                })
                .render();
        let seen_page = page_entity.clone();
        let partial_page = page_entity.clone();
        let unfinished_page = page_entity.clone();
        let single_page = page_entity.clone();
        let continuous_page = page_entity.clone();
        let locate_page = page_entity.clone();
        let per_page_page = page_entity.clone();
        let sort_dropdown = self.sort_dropdown.clone();

        let fields = div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap_5()
            .child(search)
            .child(name)
            .child(error);
        let selectors = div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap_5()
            .child(labeled("类型 · 分段控件", media_type))
            .child(labeled("强调分段", view_mode))
            .child(labeled("排序 · 自定义下拉", sort_dropdown));
        let toggles = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_5()
            .child(
                Checkbox::new("显示已看条目")
                    .theme(self.theme)
                    .checked(self.show_seen)
                    .on_change(move |value, _, app| {
                        seen_page.update(app, |this, cx| {
                            this.show_seen = value;
                            cx.notify();
                        });
                    })
                    .render(),
            )
            .child(
                Checkbox::new("部分选中")
                    .theme(self.theme)
                    .checked(self.partial_selected)
                    .indeterminate(!self.partial_selected)
                    .on_change(move |value, _, app| {
                        partial_page.update(app, |this, cx| {
                            this.partial_selected = value;
                            cx.notify();
                        });
                    })
                    .render(),
            )
            .child(
                Checkbox::new("仅看未完成")
                    .theme(self.theme)
                    .checked(self.only_unfinished)
                    .on_change(move |value, _, app| {
                        unfinished_page.update(app, |this, cx| {
                            this.only_unfinished = value;
                            cx.notify();
                        });
                    })
                    .render(),
            )
            .child(
                RadioOption::new("单页")
                    .theme(self.theme)
                    .selected(!self.continuous)
                    .on_select(move |_, _, app| {
                        single_page.update(app, |this, cx| {
                            this.continuous = false;
                            cx.notify();
                        });
                    })
                    .render(),
            )
            .child(
                RadioOption::new("连续")
                    .theme(self.theme)
                    .selected(self.continuous)
                    .on_select(move |_, _, app| {
                        continuous_page.update(app, |this, cx| {
                            this.continuous = true;
                            cx.notify();
                        });
                    })
                    .render(),
            )
            .child(
                Switch::new("自动定位到上次位置")
                    .theme(self.theme)
                    .checked(self.auto_locate)
                    .on_change(move |value, _, app| {
                        locate_page.update(app, |this, cx| {
                            this.auto_locate = value;
                            cx.notify();
                        });
                    })
                    .render(),
            );
        let per_page = labeled(
            "每页条数",
            Stepper::new(self.per_page)
                .theme(self.theme)
                .range(1, 100)
                .on_change(move |value, _, app| {
                    per_page_page.update(app, |this, cx| {
                        this.per_page = value;
                        cx.notify();
                    });
                })
                .render(),
        );
        let tags_page = page_entity.clone();
        let tags = labeled(
            "标签",
            TokenInput::new(self.tags.iter().copied(), "添加标签…")
                .theme(self.theme)
                .width(340.)
                .on_remove(move |index, _, app| {
                    tags_page.update(app, |this, cx| {
                        if index < this.tags.len() {
                            this.tags.remove(index);
                            cx.notify();
                        }
                    });
                })
                .render(),
        );
        let lower = div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap_8()
            .child(labeled("缩放", self.slider.clone()))
            .child(per_page)
            .child(tags);
        let controls = div()
            .flex()
            .flex_col()
            .gap_6()
            .child(fields)
            .child(selectors)
            .child(toggles)
            .child(lower);
        let spec = div()
            .flex()
            .flex_wrap()
            .gap_5()
            .text_size(px(12.))
            .text_color(rgb(self.theme.palette().text_muted))
            .child("输入高度：32px（表单）/ 30px（搜索与下拉）")
            .child("圆角：7px · 聚焦 3.5px 柔和光晕")
            .child("分段控件：凹槽 26px + 白色滑块 24px")
            .child("开关：40 × 24 · 系统绿");
        let composer_example = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(labeled("AI 输入框 · Composer", self.composer.clone()));
        let content = page(
            "04",
            "输入与选择",
            "输入框采用薄玻璃和内高光；聚焦时以蓝色描边与柔和光晕提示。分段控件使用凹槽和白色滑块，开关保留系统绿。",
            div()
                .flex()
                .flex_col()
                .gap_5()
                .child(panel(controls, self.theme))
                .child(spec)
                .child(panel(select_examples, self.theme))
                .child(panel(composer_example, self.theme))
                .child(panel(date_examples, self.theme))
                .child(panel(labeled("Calendar · 月历", self.calendar.clone()), self.theme))
                .child(panel(temporal_examples, self.theme))
                .child(panel(toast_examples, self.theme)),
            self.theme,
        )
        .on_any_mouse_down(cx.listener(|this, _, _, cx| this.close_menu(cx)));
        div()
            .relative()
            .size_full()
            .child(content)
            .child(self.toasts.clone())
    }
}

fn labeled(label: &'static str, control: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap_2()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(label),
        )
        .child(control)
}
