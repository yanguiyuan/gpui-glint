use super::*;
use model::{DefaultSection as Section, DirectoryDefaults};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct DirectorySettings {
    pub(super) collection: usize,
    path: String,
    title: String,
    theme: ThemePreset,
    sections: BTreeSet<Section>,
    inherited: Draft,
    sources: BTreeMap<Section, String>,
    selected: Section,
    endpoint: Entity<SelectField>,
    authentication: Entity<SelectField>,
    oauth: Option<oauth::Config>,
    oauth_source: Option<oauth::Config>,
    body: Entity<CodeCard>,
    body_format: BodyFormat,
    rows: BTreeMap<Section, Vec<PairInput>>,
    form: Vec<PairInput>,
    actions: Vec<(String, model::BeforeAction)>,
    tls: bool,
    completions: Vec<TextCompletion>,
}
pub(super) struct SaveDefaults {
    collection: usize,
    path: String,
    defaults: DirectoryDefaults,
}
impl gpui::EventEmitter<SaveDefaults> for DirectorySettings {}
impl DirectorySettings {
    fn row(pair: &Pair, theme: ThemePreset, cx: &mut Context<Self>) -> PairInput {
        let mut entry = |value: &str, hint: &str| {
            cx.new(|cx| {
                TextEntry::new(cx, hint.to_owned(), value.to_owned())
                    .font_size(12.)
                    .theme(theme)
            })
        };
        PairInput {
            enabled: pair.enabled,
            key: entry(&pair.key, "键"),
            value: entry(&pair.value, "值 / {{变量名}}"),
            note: entry(&pair.note, "说明"),
        }
    }
    fn new(
        collection: usize,
        path: String,
        groups: &RequestGroups,
        catalog: &[gpui_glint::plugins::Plugin],
        theme: ThemePreset,
        completions: Vec<TextCompletion>,
        cx: &mut Context<Self>,
    ) -> Self {
        let own = groups.defaults.get(&path).cloned().unwrap_or_default();
        let inherited = if path.is_empty() {
            Draft::default()
        } else {
            groups.request_defaults(RequestGroups::parent(&path))
        };
        let mut sources = BTreeMap::new();
        let mut ancestor = if path.is_empty() {
            None
        } else {
            Some(RequestGroups::parent(&path))
        };
        while let Some(p) = ancestor {
            if let Some(defaults) = groups.defaults.get(p) {
                for section in &defaults.sections {
                    sources.entry(*section).or_insert_with(|| {
                        if p.is_empty() {
                            groups.collection.clone()
                        } else {
                            p.to_owned()
                        }
                    });
                }
            }
            ancestor = if p.is_empty() {
                None
            } else {
                Some(RequestGroups::parent(p))
            };
        }
        let draft = groups.request_defaults(&path);
        let endpoint = cx.new(|cx| {
            let mut field = SelectField::new(
                [
                    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "WS",
                ]
                .map(|s| SelectOption::new(s, s)),
                "{{baseurl}}/path",
                cx,
            );
            field.set_selected(Some(draft.method.clone().into()), cx);
            field.set_text(draft.url.clone(), cx);
            field.set_theme(theme, cx);
            field.set_completions(completions.clone(), cx);
            field
        });
        let authentication = cx.new(|cx| {
            let mut field = SelectField::new(
                [
                    SelectOption::new("none", "No Auth"),
                    SelectOption::new("bearer", "Bearer"),
                    SelectOption::new("oauth2", "OAuth 2.0").disabled(draft.oauth.is_none()),
                ],
                "Token / {{api_token}}",
                cx,
            );
            field.set_selected(
                Some(
                    if draft.oauth.is_some() {
                        "oauth2"
                    } else if draft.auth {
                        "bearer"
                    } else {
                        "none"
                    }
                    .into(),
                ),
                cx,
            );
            field.set_text(draft.token.clone(), cx);
            field.set_theme(theme, cx);
            field.set_completions(completions.clone(), cx);
            field
        });
        let body = cx.new(|cx| {
            let mut card = CodeCard::new(cx, "json", draft.body.clone()).theme(theme);
            card.set_completions(completions.clone(), cx);
            card
        });
        let mut rows = BTreeMap::new();
        for (section, pairs) in [
            (Section::Params, &draft.params),
            (Section::Headers, &draft.headers),
            (Section::Cookies, &draft.cookies),
            (Section::Extractors, &draft.extractors),
        ] {
            let inputs: Vec<_> = pairs.iter().map(|p| Self::row(p, theme, cx)).collect();
            rows.insert(section, inputs);
        }
        let form: Vec<_> = draft.form.iter().map(|p| Self::row(p, theme, cx)).collect();
        for row in rows.values().flatten().chain(form.iter()) {
            if !rows
                .get(&Section::Extractors)
                .is_some_and(|r| r.iter().any(|p| p.key == row.key))
            {
                for entry in [&row.key, &row.value] {
                    entry.update(cx, |e, cx| e.set_completions(completions.clone(), cx));
                }
            }
        }
        let mut actions: Vec<_> = draft
            .plugins
            .iter()
            .map(|a| (format!("{} / {}", a.id, a.action_id), a.clone()))
            .collect();
        for plugin in catalog {
            let chosen = draft.plugins.iter().find(|a| {
                catalog.iter().any(|p| {
                    p.identity() == a.id && p.family_identity() == plugin.family_identity()
                })
            });
            if chosen.is_some_and(|a| a.id != plugin.identity()) {
                continue;
            }
            if chosen.is_none()
                && catalog.iter().any(|p| {
                    p.family_identity() == plugin.family_identity()
                        && semver::Version::parse(&p.manifest.version).ok()
                            > semver::Version::parse(&plugin.manifest.version).ok()
                })
            {
                continue;
            }
            let contributions = if plugin.manifest.contributions.is_empty() {
                vec![(String::new(), plugin.manifest.name.clone())]
            } else {
                plugin
                    .manifest
                    .contributions
                    .iter()
                    .filter(|c| c.kind == plugins::HOOK)
                    .map(|c| (c.id.clone(), c.title.clone()))
                    .collect()
            };
            for (id, title) in contributions {
                if let Some((label, binding)) = actions.iter_mut().find(|(_, a)| {
                    a.id == plugin.identity() && (a.action_id == id || a.action_id.is_empty())
                }) {
                    *label = title;
                    binding.action_id = id;
                    if !binding.matches(plugin) {
                        binding.enabled = false;
                        binding.approval = gpui_glint::plugins::Approval::for_plugin(plugin);
                    }
                } else {
                    actions.push((
                        title,
                        model::BeforeAction {
                            approval: gpui_glint::plugins::Approval::for_plugin(plugin),
                            action_id: id,
                            enabled: false,
                        },
                    ));
                }
            }
        }
        Self {
            collection,
            path: path.clone(),
            title: if path.is_empty() {
                groups.collection.clone()
            } else {
                format!("{} / {path}", groups.collection)
            },
            theme,
            sections: own.sections,
            inherited,
            sources,
            selected: Section::Endpoint,
            endpoint,
            authentication,
            oauth: draft.oauth.clone(),
            oauth_source: None,
            body,
            body_format: draft.body_format,
            rows,
            form,
            actions,
            tls: draft.skip_tls_verification,
            completions,
        }
    }
    pub(super) fn set_context(
        &mut self,
        theme: ThemePreset,
        completions: Vec<TextCompletion>,
        cx: &mut Context<Self>,
    ) {
        self.theme = theme;
        self.completions = completions.clone();
        for field in [&self.endpoint, &self.authentication] {
            field.update(cx, |field, cx| {
                field.set_theme(theme, cx);
                field.set_completions(completions.clone(), cx);
            });
        }
        for (section, rows) in &self.rows {
            for row in rows {
                for entry in [&row.key, &row.value, &row.note] {
                    entry.update(cx, |e, cx| {
                        e.set_theme(theme, cx);
                        if *section != Section::Extractors && entry != &row.note {
                            e.set_completions(completions.clone(), cx);
                        }
                    });
                }
            }
        }
        for row in &self.form {
            for entry in [&row.key, &row.value, &row.note] {
                entry.update(cx, |e, cx| {
                    e.set_theme(theme, cx);
                    if entry != &row.note {
                        e.set_completions(completions.clone(), cx);
                    }
                });
            }
        }
        self.body.update(cx, |card, cx| {
            card.set_theme(theme, cx);
            card.set_completions(completions, cx);
        });
        cx.notify();
    }
    fn snapshot(&self, cx: &App) -> DirectoryDefaults {
        let pairs = |rows: &[PairInput]| {
            rows.iter()
                .map(|r| Pair {
                    enabled: r.enabled,
                    key: r.key.read(cx).value().into(),
                    value: r.value.read(cx).value().into(),
                    note: r.note.read(cx).value().into(),
                })
                .collect()
        };
        let draft = Draft {
            method: self
                .endpoint
                .read(cx)
                .value()
                .selected
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or("GET".into()),
            url: self.endpoint.read(cx).value().text.to_string(),
            oauth: (self.authentication.read(cx).value().selected.as_deref() == Some("oauth2"))
                .then(|| self.oauth.clone())
                .flatten(),
            auth: self.authentication.read(cx).value().selected.as_deref() == Some("bearer"),
            token: self.authentication.read(cx).value().text.to_string(),
            params: pairs(&self.rows[&Section::Params]),
            headers: pairs(&self.rows[&Section::Headers]),
            cookies: pairs(&self.rows[&Section::Cookies]),
            extractors: pairs(&self.rows[&Section::Extractors]),
            body: self.body.read(cx).content().into(),
            body_format: self.body_format,
            form: pairs(&self.form),
            plugins: self.actions.iter().map(|(_, a)| a.clone()).collect(),
            skip_tls_verification: self.tls,
        };
        let mut active = Draft::default();
        for section in &self.sections {
            section.apply(&draft, &mut active);
        }
        DirectoryDefaults {
            sections: self.sections.clone(),
            draft: active,
        }
    }
    pub(super) fn save(&mut self, cx: &mut Context<Self>) {
        cx.emit(SaveDefaults {
            collection: self.collection,
            path: self.path.clone(),
            defaults: self.snapshot(cx),
        });
        cx.notify();
    }
    fn pairs_view(&self, section: Section, form: bool, cx: &mut Context<Self>) -> gpui::Div {
        let rows = if form {
            &self.form
        } else {
            &self.rows[&section]
        };
        let labels = if section == Section::Extractors {
            ["环境变量名", "JSONPath", "说明"]
        } else {
            ["键", "值", "说明"]
        };
        let mut view = div().flex().flex_col().gap_2().child(
            div()
                .flex()
                .gap_2()
                .pl(px(30.))
                .pr(px(34.))
                .text_color(rgb(self.theme.palette().text_muted))
                .children(
                    labels
                        .into_iter()
                        .map(|label| div().flex_1().min_w_0().child(label)),
                ),
        );
        for (i, row) in rows.iter().enumerate() {
            let owner = cx.entity();
            view = view.child(
                div()
                    .id(("directory-pair", i))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Checkbox::new("")
                            .checked(row.enabled)
                            .theme(self.theme)
                            .on_change(move |checked, _, cx| {
                                owner.update(cx, |this, cx| {
                                    let rows = if form {
                                        &mut this.form
                                    } else {
                                        this.rows.get_mut(&section).unwrap()
                                    };
                                    rows[i].enabled = checked;
                                    cx.notify();
                                });
                            })
                            .render(),
                    )
                    .child(div().flex_1().min_w_0().child(row.key.clone()))
                    .child(div().flex_1().min_w_0().child(row.value.clone()))
                    .child(div().flex_1().min_w_0().child(row.note.clone()))
                    .child(
                        IconButton::new(Icon::Close, "删除规则")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let rows = if form {
                                    &mut this.form
                                } else {
                                    this.rows.get_mut(&section).unwrap()
                                };
                                rows.remove(i);
                                cx.notify();
                            })),
                    ),
            );
        }
        view.child(
            Button::new("添加一行")
                .kind(ButtonKind::Plain)
                .theme(self.theme)
                .render()
                .on_click(cx.listener(move |this, _, _, cx| {
                    let row = Self::row(&Pair::new("", "", ""), this.theme, cx);
                    if section != Section::Extractors {
                        for e in [&row.key, &row.value] {
                            e.update(cx, |e, cx| e.set_completions(this.completions.clone(), cx));
                        }
                    }
                    if form {
                        this.form.push(row);
                    } else {
                        this.rows.get_mut(&section).unwrap().push(row);
                    }
                    cx.notify();
                })),
        )
    }
    fn inherited_summary(&self, section: Section) -> String {
        let d = &self.inherited;
        match section {
            Section::Endpoint => format!(
                "{}  {}",
                d.method,
                if d.url.is_empty() {
                    "未设置地址"
                } else {
                    &d.url
                }
            ),
            Section::Auth => {
                if d.auth {
                    "Bearer · 已配置 Token 模板".into()
                } else {
                    "No Auth".into()
                }
            }
            Section::Params => format!("{} 个参数", d.params.len()),
            Section::Headers => format!("{} 个请求头", d.headers.len()),
            Section::Cookies => format!("{} 个 Cookie", d.cookies.len()),
            Section::Body => {
                if d.body_format == BodyFormat::UrlEncoded {
                    format!("表单 · {} 个字段", d.form.len())
                } else {
                    format!("原始请求体 · {} 字节", d.body.len())
                }
            }
            Section::Plugins => format!(
                "{} 个启用的前置动作",
                d.plugins.iter().filter(|a| a.enabled).count()
            ),
            Section::Extractors => format!("{} 条提取规则", d.extractors.len()),
            Section::Settings => format!(
                "跳过 TLS 校验：{}",
                if d.skip_tls_verification {
                    "是"
                } else {
                    "否"
                }
            ),
        }
    }
}
impl Render for DirectorySettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let section = self.selected;
        let overridden = self.sections.contains(&section);
        let owner = cx.entity();
        let mut tabs = div().flex().flex_wrap().gap_3();
        for s in Section::ALL {
            tabs = tabs.child(
                tab_button(s.label(), s == section, self.theme)
                    .debug_selector(move || format!("directory-section-{s:?}").into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = s;
                        cx.notify();
                    })),
            );
        }
        let mut panel = div().flex().flex_col().gap_3().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(format!(
                    "{} · {}",
                    section.label(),
                    if overridden {
                        "当前目录覆盖".to_owned()
                    } else {
                        format!(
                            "继承自 {}",
                            self.sources
                                .get(&section)
                                .map_or("应用默认值", String::as_str)
                        )
                    }
                ))
                .child(
                    Switch::new("覆盖父目录")
                        .checked(overridden)
                        .theme(self.theme)
                        .on_change(move |checked, _, cx| {
                            owner.update(cx, |this, cx| {
                                if checked {
                                    this.sections.insert(section);
                                } else {
                                    this.sections.remove(&section);
                                }
                                cx.notify();
                            });
                        })
                        .render()
                        .debug_selector(|| "directory-override".into()),
                ),
        );
        if !overridden {
            panel = panel.child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(p.panel))
                    .overflow_hidden()
                    .child(self.inherited_summary(section)),
            );
        } else {
            panel = panel.child(match section {
                Section::Endpoint => div().child(self.endpoint.clone()),
                Section::Auth => div().flex().flex_col().gap_2().child(self.authentication.clone())
                    .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("OAuth 配置可从当前请求复制；目录与新建请求共享连接，同一环境复用系统凭据。"))
                    .when(self.oauth_source.is_some(), |view| view.child(Button::new("使用当前请求的 OAuth 配置").kind(ButtonKind::Plain).theme(self.theme).render().on_click(cx.listener(|this, _, _, cx| {
                        this.oauth = this.oauth_source.clone();
                        this.authentication.update(cx, |field, cx| {
                            field.set_options([SelectOption::new("none", "No Auth"), SelectOption::new("bearer", "Bearer"), SelectOption::new("oauth2", "OAuth 2.0")], cx);
                            field.set_selected(Some("oauth2".into()), cx); field.set_text("", cx);
                        }); cx.notify();
                    })))),
                Section::Params | Section::Headers | Section::Cookies | Section::Extractors => {
                    self.pairs_view(section, false, cx)
                }
                Section::Body => {
                    let owner = cx.entity();
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            SegmentedControl::new([
                                SegmentOption::new("原始 / JSON"),
                                SegmentOption::new("x-www-form-urlencoded"),
                            ])
                            .id("directory-body-format")
                            .selected(if self.body_format == BodyFormat::Raw {
                                0
                            } else {
                                1
                            })
                            .theme(self.theme)
                            .on_change(move |value, _, cx| {
                                owner.update(cx, |this, cx| {
                                    this.body_format = if value == 1 {
                                        BodyFormat::UrlEncoded
                                    } else {
                                        BodyFormat::Raw
                                    };
                                    cx.notify();
                                });
                            })
                            .render(),
                        )
                        .child(if self.body_format == BodyFormat::UrlEncoded {
                            self.pairs_view(section, true, cx)
                        } else {
                            div().h(px(300.)).child(self.body.clone())
                        })
                }
                Section::Settings => {
                    let owner = cx.entity();
                    div().child(
                        Switch::new("跳过 TLS 证书校验")
                            .theme(self.theme)
                            .checked(self.tls)
                            .on_change(move |checked, _, cx| {
                                owner.update(cx, |this, cx| {
                                    this.tls = checked;
                                    cx.notify();
                                });
                            })
                            .render(),
                    )
                }
                Section::Plugins => {
                    let mut list = div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child("按列表顺序执行，使用箭头调整顺序。");
                    for (i, (title, binding)) in self.actions.iter().enumerate() {
                        let owner = cx.entity();
                        list = list.child(
                            div()
                                .id(("directory-action", i))
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().flex_1().min_w_0().truncate().child(title.clone()))
                                .child(
                                    Button::new("↑")
                                        .kind(ButtonKind::Plain)
                                        .theme(self.theme)
                                        .render()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if i > 0 {
                                                this.actions.swap(i, i - 1);
                                                cx.notify();
                                            }
                                        })),
                                )
                                .child(
                                    Button::new("↓")
                                        .kind(ButtonKind::Plain)
                                        .theme(self.theme)
                                        .render()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if i + 1 < this.actions.len() {
                                                this.actions.swap(i, i + 1);
                                                cx.notify();
                                            }
                                        })),
                                )
                                .child(
                                    Switch::new("")
                                        .checked(binding.enabled)
                                        .theme(self.theme)
                                        .on_change(move |checked, _, cx| {
                                            owner.update(cx, |this, cx| {
                                                this.actions[i].1.enabled = checked;
                                                cx.notify();
                                            });
                                        })
                                        .render(),
                                ),
                        );
                    }
                    list
                }
            });
        }
        div().debug_selector(|| "directory-settings".into()).size_full().flex().flex_col().bg(rgb(p.surface)).p_5().gap_4()
            .child(div().flex().items_center().gap_3().child(div().flex_1().min_w_0().truncate().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(format!("{} · 目录设置",self.title)))
                .child(Button::new("保存默认配置").theme(self.theme).render().debug_selector(|| "save-directory-defaults".into()).on_click(cx.listener(|this,_,_,cx| this.save(cx)))))
            .child(div().text_color(rgb(p.text_muted)).child("保存后，仅在此目录及子目录新建请求时使用。各项独立继承；开启覆盖后，空配置会清除父目录的对应默认值。"))
            .child(tabs)
            .child(div().id("directory-settings-scroll").flex_1().min_h_0().overflow_y_scroll().child(panel))
    }
}
impl Workbench {
    pub(super) fn open_directory_settings(
        &mut self,
        collection: usize,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.capture_draft(cx);
        self.cancel_request_name(cx);
        self.tree_menu = None;
        self.tree_delete = None;
        self.current_collection = collection;
        self.current_directory = path.clone();
        self.show_environments = false;
        self.show_plugins = false;
        let groups = self.collections[collection].clone();
        let catalog = self.plugin_catalog.clone();
        let theme = self.theme;
        let completions = self.environment_completions_for(cx, Some(&groups.collection));
        let oauth_source = self.snapshot(cx).oauth;
        let editor = cx.new(|cx| {
            let mut editor =
                DirectorySettings::new(collection, path, &groups, &catalog, theme, completions, cx);
            editor.oauth_source = oauth_source;
            editor
        });
        self.directory_subscription =
            Some(cx.subscribe(&editor, |this, _, event: &SaveDefaults, cx| {
                if let Some(groups) = this
                    .collections
                    .get_mut(event.collection)
                    .filter(|g| g.contains(&event.path))
                {
                    groups
                        .defaults
                        .insert(event.path.clone(), event.defaults.clone());
                    this.storage_dirty = true;
                    this.persist(cx);
                    let toast = match &this.storage_error {
                        Some(error) => Toast::new("目录默认配置保存失败")
                            .description(error.clone())
                            .kind(ToastKind::Error),
                        None => Toast::new("目录默认配置已保存").kind(ToastKind::Success),
                    };
                    this.toasts.update(cx, |host, cx| host.push(toast, cx));
                }
            }));
        self.directory_settings = Some(editor);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults(sections: &[Section], draft: Draft) -> DirectoryDefaults {
        DirectoryDefaults {
            sections: sections.iter().copied().collect(),
            draft,
        }
    }
    #[test]
    fn nested_defaults_replace_sections_and_follow_tree_changes() {
        let mut group = RequestGroups::create_collection("API", &[]).unwrap();
        group.add_child("", "parent").unwrap();
        group.add_child("parent", "child").unwrap();
        group.add_child("parent/child", "leaf").unwrap();
        group.defaults.insert(
            "".into(),
            defaults(
                &[Section::Endpoint, Section::Auth, Section::Headers],
                Draft {
                    url: "{{baseurl}}/v1".into(),
                    auth: true,
                    token: "{{token}}".into(),
                    headers: vec![Pair::new("X-Root", "root", "")],
                    ..Draft::default()
                },
            ),
        );
        let action = model::BeforeAction {
            approval: gpui_glint::plugins::Approval {
                id: "test".into(),
                fingerprint: "fingerprint".into(),
                capabilities: Default::default(),
            },
            action_id: "sign".into(),
            enabled: true,
        };
        group.defaults.insert(
            "parent".into(),
            defaults(
                &[Section::Plugins, Section::Params],
                Draft {
                    plugins: vec![action.clone()],
                    params: vec![Pair::new("limit", "10", "")],
                    ..Draft::default()
                },
            ),
        );
        group.defaults.insert(
            "parent/child".into(),
            defaults(&[Section::Auth, Section::Headers], Draft::default()),
        );
        group.defaults.insert(
            "parent/child/leaf".into(),
            defaults(
                &[Section::Endpoint],
                Draft {
                    url: "{{other_baseurl}}/v2".into(),
                    ..Draft::default()
                },
            ),
        );
        let child = group.request_defaults("parent/child");
        assert_eq!(child.url, "{{baseurl}}/v1");
        assert!(!child.auth);
        assert!(child.token.is_empty() && child.headers.is_empty());
        assert_eq!(child.plugins, vec![action]);
        assert_eq!(child.params[0].key, "limit");
        assert!(group.request_defaults("parent").auth);
        assert_eq!(
            group.request_defaults("parent/child/leaf").url,
            "{{other_baseurl}}/v2"
        );
        group.rename("parent", "renamed", &mut []).unwrap();
        assert!(!group.defaults.contains_key("parent/child"));
        assert_eq!(group.request_defaults("renamed/child"), child);
        let mut groups = vec![
            group,
            RequestGroups::create_collection("Other", &[]).unwrap(),
        ];
        model::move_node(
            &mut groups,
            &mut [],
            &model::TreeNode::Directory {
                collection: 0,
                path: "renamed/child".into(),
            },
            &model::TreeNode::Directory {
                collection: 1,
                path: String::new(),
            },
            model::Placement::Inside,
        )
        .unwrap();
        assert!(!groups[0].defaults.contains_key("renamed/child"));
        assert!(groups[1].defaults.contains_key("child"));
        assert_eq!(
            groups[1].request_defaults("child/leaf").url,
            "{{other_baseurl}}/v2"
        );
        assert!(groups[1].request_defaults("child").plugins.is_empty());
        groups[1].remove("child", &mut []);
        assert!(groups[1].defaults.is_empty());
        let mut legacy = serde_json::to_value(&groups[0]).unwrap();
        legacy.as_object_mut().unwrap().remove("defaults");
        assert!(
            serde_json::from_value::<RequestGroups>(legacy)
                .unwrap()
                .defaults
                .is_empty()
        );
    }

    #[gpui::test]
    fn directory_double_click_save_and_new_requests_use_persisted_defaults(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("API", cx));
            host.save_group(cx);
            host.collections[0].add_child("", "Parent").unwrap();
            host.collections[0].add_child("Parent", "Child").unwrap();
            host.collections[0].defaults.insert(
                "Parent".into(),
                defaults(
                    &[Section::Endpoint, Section::Auth, Section::Headers],
                    Draft {
                        url: "{{baseurl}}/v1".into(),
                        auth: true,
                        token: "{{token}}".into(),
                        headers: vec![Pair::new("X-Default", "yes", "")],
                        ..Draft::default()
                    },
                ),
            );
            let plugin = &host.plugin_catalog[0];
            let binding = model::BeforeAction {
                approval: gpui_glint::plugins::Approval::for_plugin(plugin),
                action_id: plugin
                    .manifest
                    .contributions
                    .iter()
                    .find(|c| c.kind == plugins::HOOK)
                    .map(|c| c.id.clone())
                    .unwrap_or_default(),
                enabled: true,
            };
            let parent = host.collections[0].defaults.get_mut("Parent").unwrap();
            parent.sections.insert(Section::Plugins);
            parent.draft.plugins = vec![binding];
            host.add_request_to("Parent/Child".into(), cx);
            host.show_activity = false;
            host
        });
        let original = host.read_with(window, |h, _| h.requests[0].draft.clone());
        let bounds = window
            .debug_bounds("request-directory:0:Parent/Child")
            .unwrap();
        for down in [true, false] {
            if down {
                window.simulate_event(gpui::MouseDownEvent {
                    button: gpui::MouseButton::Left,
                    position: bounds.center(),
                    click_count: 2,
                    ..Default::default()
                });
            } else {
                window.simulate_event(gpui::MouseUpEvent {
                    button: gpui::MouseButton::Left,
                    position: bounds.center(),
                    click_count: 2,
                    ..Default::default()
                });
            }
        }
        assert!(window.debug_bounds("directory-settings").is_some());
        let editor = host.read_with(window, |h, _| h.directory_settings.clone().unwrap());
        let switch = window.debug_bounds("directory-override").unwrap();
        window.simulate_click(switch.center(), gpui::Modifiers::default());
        editor.update(window, |e, cx| {
            assert!(e.sections.contains(&Section::Endpoint));
            assert!(!e.sections.contains(&Section::Auth));
            assert_eq!(e.inherited.url, "{{baseurl}}/v1");
            e.endpoint
                .update(cx, |f, cx| f.set_text("{{child_baseurl}}/v2", cx));
        });
        let auth_tab = window.debug_bounds("directory-section-Auth").unwrap();
        window.simulate_click(auth_tab.center(), gpui::Modifiers::default());
        let switch = window.debug_bounds("directory-override").unwrap();
        window.simulate_click(switch.center(), gpui::Modifiers::default());
        editor.update(window, |e, cx| {
            assert!(e.sections.contains(&Section::Auth));
            e.authentication.update(cx, |f, cx| {
                f.set_selected(Some("none".into()), cx);
                f.set_text("", cx);
            });
        });
        let save = window.debug_bounds("save-directory-defaults").unwrap();
        window.simulate_click(save.center(), gpui::Modifiers::default());
        host.update(window, |h, cx| {
            assert_eq!(h.requests[0].draft, original);
            assert!(
                h.collections[0].defaults["Parent/Child"]
                    .sections
                    .contains(&Section::Auth)
            );
            h.add_request_to("Parent/Child".into(), cx);
            assert!(h.directory_settings.is_none());
            let child = h.snapshot(cx);
            assert_eq!(child.url, "{{child_baseurl}}/v2");
            assert!(!child.auth);
            assert!(child.token.is_empty());
            assert_eq!(child.headers, original.headers);
            assert_eq!(child.plugins, original.plugins);
            assert_eq!(h.requests[1].versions[0].draft, child);
            h.endpoint
                .update(cx, |f, cx| f.set_text("{{child_baseurl}}/custom", cx));
            h.persist(cx);
        });
        let restored = storage::Store::open(&path).unwrap().load().unwrap();
        assert!(
            restored.collections[0].defaults["Parent/Child"]
                .draft
                .plugins
                .is_empty()
        );
        assert_eq!(restored.requests[0].draft, original);
        assert_eq!(restored.requests[1].draft.url, "{{child_baseurl}}/custom");
        assert_eq!(
            restored.collections[0].request_defaults("Parent/Child").url,
            "{{child_baseurl}}/v2"
        );
        assert!(
            !restored.collections[0]
                .request_defaults("Parent/Child")
                .auth
        );
        host.update(window, |h, cx| {
            h.open_directory_settings(0, "Parent/Child".into(), cx)
        });
        let editor = host.read_with(window, |h, _| h.directory_settings.clone().unwrap());
        for section in Section::ALL {
            editor.update(window, |e, cx| {
                e.selected = section;
                e.sections.insert(section);
                cx.notify();
            });
            assert!(window.debug_bounds("directory-settings").is_some());
        }
        editor.update(window, |e, cx| {
            e.selected = Section::Auth;
            e.sections = [Section::Auth, Section::Endpoint].into_iter().collect();
            cx.notify();
        });
        let switch = window.debug_bounds("directory-override").unwrap();
        window.simulate_click(switch.center(), gpui::Modifiers::default());
        let save = window.debug_bounds("save-directory-defaults").unwrap();
        window.simulate_click(save.center(), gpui::Modifiers::default());
        host.update(window, |h, cx| {
            h.add_request_to("Parent/Child".into(), cx);
            assert!(h.snapshot(cx).auth);
            assert_eq!(h.snapshot(cx).token, "{{token}}");
            assert_eq!(h.requests[1].draft.url, "{{child_baseurl}}/custom");
        });
    }
}
