use super::*;
use gpui_glint::components::Switch;
use std::sync::Arc;

// Metadata is shown above the instructions. Keep YAML descriptions from
// becoming accidental Markdown headings in the reading pane.
fn skill_instructions(source: &str) -> &str {
    let source = source.trim_start_matches('\u{feff}');
    let mut lines = source.split_inclusive('\n');
    let Some(first) = lines.next().filter(|line| line.trim() == "---") else {
        return source;
    };
    let mut offset = first.len();
    for line in lines {
        offset += line.len();
        if line.trim() == "---" {
            return &source[offset..];
        }
    }
    source
}

fn context_sections(catalog: Option<&skills::Catalog>) -> Vec<ComposerContextSection> {
    let mut entries: Vec<_> = catalog
        .into_iter()
        .flat_map(|catalog| catalog.skills.iter())
        .filter(|skill| skill.enabled)
        .map(|skill| {
            ComposerContextEntry::new(format!("skill:{}", skill.name), skill.name.clone())
                .icon(Icon::Workflow)
                .description(skill.description.clone())
                .insert_mention(false)
        })
        .collect();
    entries.push(
        ComposerContextEntry::new("manage-skills", "管理技能")
            .icon(Icon::Settings)
            .description("配置技能目录和启停")
            .insert_mention(false),
    );
    vec![
        ComposerContextSection::untitled([
            ComposerContextEntry::new("file", "文件")
                .icon(Icon::Text)
                .description("选择文件作为上下文")
                .insert_mention(false),
            ComposerContextEntry::new("directory", "目录")
                .icon(Icon::Folder)
                .description("选择目录作为上下文")
                .insert_mention(false),
        ]),
        ComposerContextSection::new("技能", entries),
    ]
}

pub struct State {
    catalog: Option<Arc<skills::Catalog>>,
    task: Option<Task<()>>,
    search: Entity<TextEntry>,
    notice: Option<String>,
    selected: Option<(String, Result<String, String>)>,
    detail_scroll: ScrollHandle,
}
impl State {
    pub fn set_theme(&self, theme: ThemePreset, cx: &mut Context<AgentWorkspace>) {
        self.search
            .update(cx, |entry, cx| entry.set_theme(theme, cx));
    }
    pub fn new(theme: ThemePreset, cx: &mut Context<AgentWorkspace>) -> Self {
        let search = cx.new(|cx| TextEntry::new(cx, "搜索技能名称或描述…", "").theme(theme));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        Self {
            catalog: None,
            task: None,
            search,
            notice: None,
            selected: None,
            detail_scroll: ScrollHandle::new(),
        }
    }
}
impl AgentWorkspace {
    pub fn sync_skill_context(&mut self, cx: &mut Context<Self>) {
        let mut sections = context_sections(self.skills_ui.catalog.as_deref());
        if let Some(files) = self.project_context.section() {
            sections.insert(1, files);
        }
        let color = self.theme.palette().accent;
        let tokens: Vec<_> = self
            .skills_ui
            .catalog
            .as_ref()
            .into_iter()
            .flat_map(|catalog| catalog.skills.iter())
            .filter(|skill| skill.enabled)
            .map(|skill| (SharedString::from(format!("${}", skill.name)), color))
            .collect();
        self.composer.update(cx, |composer, cx| {
            composer.set_context_sections(sections, cx);
            composer.register_context_tokens(tokens, cx);
        });
    }
    pub fn reset_skill_context(&mut self, cx: &mut Context<Self>) {
        self.skills_ui.catalog = None;
        self.skills_ui.selected = None;
        self.sync_skill_context(cx);
        self.reload_skills(cx);
    }
    pub fn select_skill_context(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.skills_ui.catalog.as_ref().is_some_and(|catalog| {
            catalog
                .skills
                .iter()
                .any(|skill| skill.name == name && skill.enabled)
        }) {
            self.provider_error = Some("此技能已不可用，请刷新技能列表".into());
            cx.notify();
            return;
        }
        let token = format!("${name}");
        let color = self.theme.palette().accent;
        self.composer.update(cx, |composer, cx| {
            composer.add_context_token(token, color, cx)
        });
        self.capture_draft(cx);
        self.save(cx);
    }
    pub fn reload_skills(&mut self, cx: &mut Context<Self>) {
        if let Some(task) = self.skills_ui.task.take() {
            drop(task);
        }
        let manager = skills::Manager::new(self.path.clone());
        let project = self.history.active().project.clone();
        let work = cx
            .background_executor()
            .spawn(async move { manager.catalog(project.as_deref()) });
        self.skills_ui.task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.skills_ui.task.take() {
                    task.detach();
                }
                match result {
                    Ok(catalog) => {
                        this.skills_ui.selected = this
                            .skills_ui
                            .selected
                            .as_ref()
                            .filter(|(id, _)| catalog.skills.iter().any(|skill| skill.id == *id))
                            .map(|(id, _)| (id.clone(), catalog.preview(id)));
                        this.skills_ui.catalog = Some(Arc::new(catalog));
                        this.skills_ui.notice = None;
                        this.sync_skill_context(cx);
                    }
                    Err(error) => this.skills_ui.notice = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn choose_skills_root(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("添加技能目录".into()),
        });
        cx.spawn(async move |weak, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = weak.update(cx, |this, cx| {
                match skills::Manager::new(this.path.clone()).add_root(&path) {
                    Ok(()) => this.reload_skills(cx),
                    Err(error) => {
                        this.skills_ui.notice = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    fn open_skill_preview(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(catalog) = &self.skills_ui.catalog {
            let content = catalog.preview(&id);
            self.skills_ui.selected = Some((id, content));
            self.skills_ui.detail_scroll = ScrollHandle::new();
            cx.notify();
        }
    }

    pub fn skills_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let state = &self.skills_ui;
        let query = state.search.read(cx).value().trim().to_lowercase();
        let header = div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_6()
            .border_b_1()
            .border_color(rgb(p.border))
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
                            .child("技能"),
                    )
                    .child(
                        Button::new(if state.task.is_some() {
                            "正在扫描…"
                        } else {
                            "刷新技能"
                        })
                        .kind(ButtonKind::Gray)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| this.reload_skills(cx))),
                    )
                    .child(
                        Button::new("添加技能目录")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.choose_skills_root(cx))),
                    ),
            )
            .child(
                div()
                    .text_color(rgb(p.text_muted))
                    .child("点击技能查看指令，在提问中用 $技能名 指定使用。"),
            )
            .when_some(state.notice.clone(), |page, notice| {
                page.child(
                    div()
                        .p_3()
                        .rounded(px(8.))
                        .bg(rgb(p.surface_subtle))
                        .child(notice),
                )
            });

        let mut list = div()
            .id("skills-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_2();
        if let Some(catalog) = &state.catalog {
            let filtered: Vec<_> = catalog
                .skills
                .iter()
                .filter(|skill| {
                    query.is_empty()
                        || skill.name.to_lowercase().contains(&query)
                        || skill.description.to_lowercase().contains(&query)
                })
                .collect();
            list = list.child(
                div()
                    .px_2()
                    .py_2()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(format!(
                        "{} 项技能 · 已启用 {} 项",
                        catalog.skills.len(),
                        catalog.skills.iter().filter(|skill| skill.enabled).count()
                    )),
            );
            if filtered.is_empty() {
                list = list.child(div().p_3().text_color(rgb(p.text_muted)).child(
                    if catalog.skills.is_empty() {
                        "尚未发现技能，请添加包含 SKILL.md 的目录。"
                    } else {
                        "没有匹配的技能，试试其他关键词。"
                    },
                ));
            }
            for skill in filtered {
                let id = skill.id.clone();
                let selected = state
                    .selected
                    .as_ref()
                    .is_some_and(|(id, _)| id == &skill.id);
                list = list.child(
                    div()
                        .id(SharedString::from(format!("skill:{}", skill.id)))
                        .debug_selector(move || format!("skill:{id}"))
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
                                        .child(skill.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(p.text_muted))
                                        .child(if skill.enabled {
                                            skill.source.clone()
                                        } else {
                                            format!("{} · 已停用", skill.source)
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(preview_line(&skill.description, 85)),
                        )
                        .on_click({
                            let id = skill.id.clone();
                            cx.listener(move |this, _, _, cx| {
                                this.open_skill_preview(id.clone(), cx);
                            })
                        }),
                );
            }
            list = list.child(
                div()
                    .mt_4()
                    .px_2()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .child("扫描目录"),
            );
            list = list.child(
                div()
                    .px_2()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("同名技能优先使用项目版本"),
            );
            for (path, source) in &catalog.roots {
                let remove = path.clone();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("skill-root:{}", path.display())))
                        .p_2()
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child(format!("{source} · {}", path.display())),
                        )
                        .when(source == "自定义", |row| {
                            row.child(
                                Button::new("移除目录")
                                    .kind(ButtonKind::Plain)
                                    .size(ButtonSize::Mini)
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match skills::Manager::new(this.path.clone())
                                            .remove_root(&remove)
                                        {
                                            Ok(()) => this.reload_skills(cx),
                                            Err(error) => {
                                                this.skills_ui.notice = Some(error);
                                                cx.notify();
                                            }
                                        }
                                    })),
                            )
                        }),
                );
            }
            for warning in &catalog.warnings {
                list = list.child(
                    div()
                        .p_2()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child(warning.clone()),
                );
            }
        } else {
            list = list.child(div().p_3().text_color(rgb(p.text_muted)).child(
                if state.task.is_some() {
                    "正在扫描技能…"
                } else {
                    "点击刷新技能加载列表。"
                },
            ));
        }
        let sidebar = div()
            .w(gpui::relative(0.36))
            .min_w(px(240.))
            .max_w(px(380.))
            .h_full()
            .flex_shrink_0()
            .min_h_0()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(rgb(p.border))
            .child(
                div().p_3().flex_shrink_0().child(
                    SearchInput::new("搜索技能")
                        .control(state.search.clone())
                        .theme(self.theme)
                        .render()
                        .w_full(),
                ),
            )
            .child(list);
        let mut detail = div()
            .id("skill-detail")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(p.panel));
        if let Some(((id, content), skill)) = state.selected.as_ref().and_then(|selection| {
            state
                .catalog
                .as_ref()?
                .skills
                .iter()
                .find(|skill| skill.id == selection.0)
                .map(|skill| (selection, skill))
        }) {
            let weak = cx.entity().downgrade();
            let toggle_id = id.clone();
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
                                    .child(skill.name.clone()),
                            )
                            .child(
                                Switch::new("启用")
                                    .checked(skill.enabled)
                                    .theme(self.theme)
                                    .on_change(move |enabled, _, cx| {
                                        let _ =
                                            weak.update(cx, |this, cx| match skills::Manager::new(
                                                this.path.clone(),
                                            )
                                            .set_enabled(&toggle_id, enabled)
                                            {
                                                Ok(()) => this.reload_skills(cx),
                                                Err(error) => {
                                                    this.skills_ui.notice = Some(error);
                                                    cx.notify();
                                                }
                                            });
                                    })
                                    .render(),
                            )
                            .child(
                                IconButton::new(Icon::Close, "关闭技能详情")
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.skills_ui.selected = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(format!("{} · SKILL.md", skill.source)),
                    ),
            );
            let mut body = div()
                .id("skill-detail-scroll")
                .debug_selector(|| "skill-detail-scroll".into())
                .track_scroll(&state.detail_scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_6()
                .child(
                    div()
                        .mb_4()
                        .text_color(rgb(p.text_muted))
                        .child(skill.description.clone()),
                )
                .when_some(skill.compatibility.clone(), |body, text| {
                    body.child(
                        div()
                            .mb_3()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(format!("兼容性：{text}")),
                    )
                })
                .child(
                    div()
                        .mb_4()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child(skill.root.join("SKILL.md").display().to_string()),
                );
            body = match content {
                Ok(text) => body.child(self.markdown_text(
                    "skill-settings-preview".into(),
                    skill_instructions(text),
                    cx,
                )),
                Err(error) => body.child(
                    div()
                        .p_3()
                        .rounded(px(8.))
                        .bg(rgb(p.surface_subtle))
                        .child(error.clone()),
                ),
            };
            detail = detail.child(body);
        } else {
            detail = detail
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(Icon::Workflow.render(28.).text_color(rgb(p.text_muted)))
                .child(div().font_weight(FontWeight::MEDIUM).child("选择一个技能"))
                .child(
                    div()
                        .text_color(rgb(p.text_muted))
                        .child("在左侧选择技能，在这里查看完整指令和启用状态。"),
                );
        }
        div()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(header)
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
    fn preview_separates_metadata_without_changing_markdown_instructions() {
        assert_eq!(
            skill_instructions("\u{feff}---\r\nname: review\r\n---\r\n# Review\r\n\r\n---\r\nBody"),
            "# Review\r\n\r\n---\r\nBody"
        );
        assert_eq!(skill_instructions("# Plain"), "# Plain");
        assert_eq!(
            skill_instructions("---\nname: incomplete"),
            "---\nname: incomplete"
        );
    }
    struct Preview {
        composer: Entity<Composer>,
        _subscription: Subscription,
    }
    impl Render for Preview {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(650.)).pt(px(320.)).child(self.composer.clone())
        }
    }
    #[gpui::test]
    fn context_panel_selects_enabled_skill_replaces_at_and_live_refresh_removes_disabled(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let anchor = directory.path().join("sessions.json");
        let manager = skills::Manager::new(anchor);
        let root = manager.user_root().join("review");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("SKILL.md"),
            "---\nname: review\ndescription: Review code changes\n---\nRead the diff first.\n",
        )
        .unwrap();
        let catalog = manager.catalog_with_home(None, None).unwrap();
        let id = catalog.skills[0].id.clone();
        let (preview, window) = cx.add_window_view(|_, cx| {
            let composer = cx.new(|cx| {
                Composer::new("Message", cx)
                    .context_panel("attach", "添加上下文", context_sections(Some(&catalog)))
                    .context_on_at(true)
            });
            let subscription = cx.subscribe(
                &composer,
                |this: &mut Preview, _, event: &ComposerEvent, cx| {
                    if let ComposerEvent::ContextSelected { id, .. } = event
                        && let Some(name) = id.strip_prefix("skill:")
                    {
                        this.composer.update(cx, |composer, cx| {
                            composer.add_context_token(format!("${name}"), 0x007aff, cx)
                        });
                    }
                },
            );
            Preview {
                composer,
                _subscription: subscription,
            }
        });
        let composer = preview.read_with(window, |this, _| this.composer.clone());
        composer.update(window, |composer, cx| composer.set_value("检查 @", cx));
        let row = window
            .debug_bounds("composer-context:skill:review")
            .unwrap();
        window.simulate_click(row.center(), gpui::Modifiers::default());
        composer.read_with(window, |composer, cx| {
            assert_eq!(composer.value(cx).as_ref(), "检查 $review ");
            assert!(!composer.context_panel_open());
        });
        composer.update(window, |composer, cx| composer.open_context_panel(cx));
        manager.set_enabled(&id, false).unwrap();
        let disabled = manager.catalog_with_home(None, None).unwrap();
        composer.update(window, |composer, cx| {
            composer.set_context_sections(context_sections(Some(&disabled)), cx)
        });
        assert!(composer.read_with(window, |composer, _| composer.context_panel_open()));
        assert!(
            window
                .debug_bounds("composer-context:skill:review")
                .is_none()
        );
        assert!(
            window
                .debug_bounds("composer-context:manage-skills")
                .is_some()
        );
    }
}
