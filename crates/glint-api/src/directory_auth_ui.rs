use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn directory_authorization_presets_follow_plugins_and_keep_copied_schemes(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join("presets.sqlite3")).unwrap(),
                cx,
            );
            let mut plugin = host
                .plugin_catalog
                .iter()
                .find(|p| {
                    p.manifest
                        .contributions
                        .iter()
                        .any(|c| c.kind == plugins::OAUTH_PRESET)
                })
                .unwrap()
                .clone();
            plugin.manifest.id = "example.oauth".into();
            plugin.manifest.name = "Example SSO".into();
            let contribution = plugin
                .manifest
                .contributions
                .iter_mut()
                .find(|c| c.kind == plugins::OAUTH_PRESET)
                .unwrap();
            contribution.title = "Example 用户授权".into();
            let data = contribution.data.as_mut().unwrap();
            data["authorization_url"] = serde_json::json!("https://sso.example.test/authorize");
            data["token_url"] = serde_json::json!("https://sso.example.test/token");
            host.plugin_catalog = vec![plugin];
            host.workspace
                .collections
                .push(RequestGroups::create_collection("API", &[]).unwrap());
            host.add_request(cx);
            host.open_directory_settings(0, String::new(), cx);
            host
        });
        let editor = host.read_with(window, |h, _| h.directory_settings.clone().unwrap());
        let (key, add, expected) = editor.update(window, |e, cx| {
            e.selected = Section::Auth;
            cx.notify();
            assert_eq!(e.auth_presets.len(), 1);
            let preset = &e.auth_presets[0];
            (
                format!("preset:{}", preset.key),
                e.auth_add.clone(),
                preset.preset.config().unwrap(),
            )
        });
        assert!(window.debug_bounds("directory-auth-add").is_some());
        add.update(window, |_, cx| {
            cx.emit(SelectEvent::Changed(key.clone().into()))
        });
        editor.read_with(window, |e, cx| {
            assert_eq!(e.auth_profiles.len(), 1);
            let actual = e.auth_profiles[0].snapshot(cx);
            assert_eq!(actual.name, "Example 用户授权");
            let actual = actual.oauth.unwrap();
            let mut expected = expected.clone();
            expected.id = actual.id.clone();
            assert_eq!(actual, expected);
            assert!(e.auth_add.read(cx).value().is_none());
        });
        add.update(window, |_, cx| {
            cx.emit(SelectEvent::Changed(key.clone().into()))
        });
        let copied = editor.read_with(window, |e, cx| {
            assert_eq!(e.auth_profiles[1].snapshot(cx).name, "Example 用户授权 (2)");
            e.auth_profiles[0].snapshot(cx)
        });
        host.update(window, |h, cx| {
            let id = h.plugin_catalog[0].identity();
            h.set_plugin_enabled(&id, false, cx).unwrap();
        });
        window.run_until_parked();
        editor.update(window, |e, cx| {
            assert!(e.auth_presets.is_empty());
            assert_eq!(e.auth_profiles[0].snapshot(cx), copied);
            e.add_auth_profile(&key, cx);
            assert!(e.auth_error.is_some());
            assert_eq!(e.auth_profiles.len(), 2);
            e.add_auth_profile("blank:bearer", cx);
            assert!(e.auth_profiles[2].snapshot(cx).oauth.is_none());
            assert!(e.auth_error.is_none());
        });
    }

    #[gpui::test]
    fn auth_profile_compact_layout_switches_and_collapses_without_losing_values(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join("layout.sqlite3")).unwrap(),
                cx,
            );
            host.workspace
                .collections
                .push(RequestGroups::create_collection("API", &[]).unwrap());
            host.add_request(cx);
            host.open_directory_settings(0, String::new(), cx);
            host
        });
        let editor = host.read_with(window, |h, _| h.directory_settings.clone().unwrap());
        editor.update(window, |e, cx| {
            e.selected = Section::Auth;
            e.add_auth_profile("blank:oauth2", cx);
            e.auth_profiles[0]
                .secret
                .update(cx, |field, cx| field.set_value("keep-secret", cx));
        });
        let client = window.debug_bounds("auth-profile-client-0").unwrap();
        let secret = window.debug_bounds("auth-profile-secret-0").unwrap();
        assert_eq!(client.top(), secret.top());
        assert!(client.right() < secret.left());
        assert!(window.debug_bounds("auth-profile-token-0").is_none());
        assert!(window.debug_bounds("auth-profile-options-0").is_none());
        let advanced = window.debug_bounds("auth-profile-advanced-0").unwrap();
        window.simulate_click(advanced.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("auth-profile-options-0").is_some());
        let toggle = window.debug_bounds("auth-profile-toggle-0").unwrap();
        window.simulate_click(toggle.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("auth-profile-form-0").is_none());
        let toggle = window.debug_bounds("auth-profile-toggle-0").unwrap();
        window.simulate_click(toggle.center(), gpui::Modifiers::default());
        let mode = window.debug_bounds("auth-profile-mode-control-0").unwrap();
        window.simulate_click(
            point(mode.left() + px(20.), mode.center().y),
            gpui::Modifiers::default(),
        );
        assert!(window.debug_bounds("auth-profile-token-0").is_some());
        assert!(window.debug_bounds("auth-profile-client-0").is_none());
        let mode = window.debug_bounds("auth-profile-mode-control-0").unwrap();
        window.simulate_click(
            point(mode.right() - px(20.), mode.center().y),
            gpui::Modifiers::default(),
        );
        editor.read_with(window, |e, cx| {
            assert_eq!(
                e.auth_profiles[0].snapshot(cx).oauth.unwrap().client_secret,
                "keep-secret"
            )
        });
    }

    #[gpui::test]
    fn auth_profile_directory_saves_multiple_schemes_and_new_request_default(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("directory-auth.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.workspace
                .collections
                .push(RequestGroups::create_collection("API", &[]).unwrap());
            host.add_request(cx);
            host.open_directory_settings(0, String::new(), cx);
            host
        });
        let editor = host.read_with(window, |host, _| host.directory_settings.clone().unwrap());
        let id = editor.update(window, |editor, cx| {
            editor.selected = Section::Auth;
            editor.add_auth_profile("blank:oauth2", cx);
            editor.auth_profiles[0].fields[2].update(cx, |e, cx| e.set_value("client", cx));
            editor.auth_profiles[0]
                .secret
                .update(cx, |e, cx| e.set_value("saved-secret", cx));
            editor.add_auth_profile("blank:bearer", cx);
            editor.auth_profiles[1].mode.update(cx, |f, cx| {
                f.set_selected(Some("bearer".into()), cx);
                f.set_text("{{app_token}}", cx);
            });
            let id = editor.auth_profiles[0].id.clone();
            editor.sections.insert(Section::Auth);
            editor.sync_auth_options(cx);
            editor.authentication.update(cx, |f, cx| {
                f.set_selected(Some(format!("profile:{id}").into()), cx)
            });
            editor.save(cx);
            id
        });
        window.run_until_parked();
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        let profiles = &saved.collections[0].defaults[""].auth_profiles;
        assert_eq!(profiles.len(), 2);
        assert_eq!(
            profiles[0].oauth.as_ref().unwrap().client_secret,
            "saved-secret"
        );
        assert!(profiles[1].oauth.is_none());
        host.update(window, |host, cx| {
            host.add_request(cx);
            assert_eq!(host.snapshot(cx).auth_profile.as_deref(), Some(id.as_str()));
            assert_eq!(host.oauth.secret.read(cx).value(), "saved-secret");
        });
    }
}

pub(super) struct ProfileInput {
    pub(super) expanded: bool,
    advanced: bool,
    id: String,
    name: Entity<TextEntry>,
    secret: Entity<TextEntry>,
    mode: Entity<SelectField>,
    config: oauth::Config,
    fields: [Entity<TextEntry>; 5],
}
impl ProfileInput {
    pub(super) fn new(
        profile: &model::AuthProfile,
        theme: ThemePreset,
        completions: &[TextCompletion],
        cx: &mut Context<DirectorySettings>,
    ) -> Self {
        let config = profile.oauth.clone().unwrap_or_default();
        let mut entry = |value: &str, hint: &str| {
            cx.new(|cx| {
                let mut entry = TextEntry::new(cx, hint.to_owned(), value.to_owned()).theme(theme);
                entry.set_completions(completions.to_vec(), cx);
                entry
            })
        };
        let name = entry(&profile.name, "授权名称，例如：用户授权 / 应用授权");
        let secret = entry(&config.client_secret, "Client Secret / {{client_secret}}");
        let fields = [
            (&config.authorization_url, "授权地址"),
            (&config.token_url, "Token 地址"),
            (&config.client_id, "Client ID / {{client_id}}"),
            (&config.redirect_uri, "回调地址"),
            (&config.scope, "Scope"),
        ]
        .map(|(v, hint)| entry(v, hint));
        let mode = cx.new(|cx| {
            let mut field = SelectField::new(
                [
                    SelectOption::new("bearer", "Bearer"),
                    SelectOption::new("oauth2", "OAuth 2.0"),
                ],
                "Token / {{api_token}}",
                cx,
            );
            field.set_selected(
                Some(
                    if profile.oauth.is_some() {
                        "oauth2"
                    } else {
                        "bearer"
                    }
                    .into(),
                ),
                cx,
            );
            field.set_text(profile.token.clone(), cx);
            field.set_theme(theme, cx);
            field.set_completions(completions.to_vec(), cx);
            field
        });
        cx.observe(&mode, |_, _, cx| cx.notify()).detach();
        cx.observe(&name, |_, _, cx| cx.notify()).detach();
        Self {
            expanded: true,
            advanced: false,
            id: profile.id.clone(),
            name,
            secret,
            mode,
            config,
            fields,
        }
    }
    pub(super) fn snapshot(&self, cx: &App) -> model::AuthProfile {
        let mut config = self.config.clone();
        config.authorization_url = self.fields[0].read(cx).value().trim().into();
        config.token_url = self.fields[1].read(cx).value().trim().into();
        config.client_id = self.fields[2].read(cx).value().trim().into();
        config.redirect_uri = self.fields[3].read(cx).value().trim().into();
        config.scope = self.fields[4].read(cx).value().trim().into();
        config.client_secret = self.secret.read(cx).value().into();
        model::AuthProfile {
            id: self.id.clone(),
            name: self.name.read(cx).value().trim().into(),
            token: self.mode.read(cx).value().text.to_string(),
            oauth: (self.mode.read(cx).value().selected.as_deref() == Some("oauth2"))
                .then_some(config),
        }
    }
    pub(super) fn set_context(
        &self,
        theme: ThemePreset,
        completions: &[TextCompletion],
        cx: &mut Context<DirectorySettings>,
    ) {
        for field in [&self.name, &self.secret]
            .into_iter()
            .chain(self.fields.iter())
        {
            field.update(cx, |e, cx| {
                e.set_theme(theme, cx);
                e.set_completions(completions.to_vec(), cx);
            });
        }
        self.mode.update(cx, |f, cx| {
            f.set_theme(theme, cx);
            f.set_completions(completions.to_vec(), cx);
        });
    }
}
impl DirectorySettings {
    fn auth_profile_input(&self, entry: &Entity<TextEntry>) -> gpui::Div {
        div()
            .h(px(32.))
            .px_3()
            .border_1()
            .border_color(rgb(self.theme.palette().border))
            .rounded_md()
            .child(entry.clone())
    }
    pub(super) fn sync_auth_options(&mut self, cx: &mut Context<Self>) {
        let mut options = vec![
            SelectOption::new("none", "No Auth"),
            SelectOption::new("bearer", "Bearer"),
            SelectOption::new("oauth2", "OAuth 2.0").disabled(self.oauth.is_none()),
        ];
        options.extend(
            self.auth_profiles
                .iter()
                .map(|p| p.snapshot(cx))
                .chain(self.inherited_profiles.clone())
                .map(|p| {
                    SelectOption::new(
                        format!("profile:{}", p.id),
                        format!("默认使用 · {}", p.name),
                    )
                }),
        );
        self.authentication
            .update(cx, |f, cx| f.set_options(options, cx));
    }
    pub(crate) fn set_auth_presets(
        &mut self,
        presets: Vec<plugins::PresetEntry>,
        cx: &mut Context<Self>,
    ) {
        if self.auth_presets == presets {
            return;
        }
        let mut options = vec![
            SelectOption::new("blank:bearer", "空白 Bearer 方案"),
            SelectOption::new("blank:oauth2", "空白 OAuth 2.0 方案"),
        ];
        options.extend(presets.iter().map(|preset| {
            SelectOption::new(
                format!("preset:{}", preset.key),
                format!("预设 · {}", preset.title),
            )
        }));
        self.auth_presets = presets;
        self.auth_add
            .update(cx, |select, cx| select.set_options(options, cx));
        cx.notify();
    }
    pub(super) fn add_auth_profile(&mut self, choice: &str, cx: &mut Context<Self>) {
        self.auth_add
            .update(cx, |select, cx| select.set_selected(None, cx));
        let selected = match choice {
            "blank:bearer" => Ok(("Bearer 授权".to_owned(), None)),
            "blank:oauth2" => Ok(("OAuth 2.0 授权".to_owned(), Some(oauth::Config::default()))),
            _ => choice
                .strip_prefix("preset:")
                .and_then(|key| self.auth_presets.iter().find(|p| p.key == key))
                .ok_or_else(|| "授权预设已移除或停用，请重新选择".to_owned())
                .and_then(|preset| {
                    preset
                        .preset
                        .config()
                        .map(|config| (preset.title.clone(), Some(config)))
                }),
        };
        let (title, config) = match selected {
            Ok(profile) => profile,
            Err(error) => {
                self.auth_error = Some(error);
                cx.notify();
                return;
            }
        };
        let mut name = title.clone();
        let mut suffix = 2;
        while self
            .auth_profiles
            .iter()
            .any(|p| p.name.read(cx).value().trim() == name)
        {
            name = format!("{title} ({suffix})");
            suffix += 1;
        }
        let profile = model::AuthProfile {
            id: model::new_id(),
            name,
            token: String::new(),
            oauth: config,
        };
        self.auth_profiles.push(ProfileInput::new(
            &profile,
            self.theme,
            &self.completions,
            cx,
        ));
        self.auth_error = None;
        self.sync_auth_options(cx);
        cx.notify();
    }
    fn auth_profile_field(&self, label: &str, entry: &Entity<TextEntry>) -> gpui::Div {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child(label.to_owned()),
            )
            .child(self.auth_profile_input(entry))
    }
    fn set_auth_default(&mut self, value: String, cx: &mut Context<Self>) {
        self.sync_auth_options(cx);
        self.sections.insert(Section::Auth);
        self.authentication
            .update(cx, |field, cx| field.set_selected(Some(value.into()), cx));
        cx.notify();
    }
    pub(super) fn auth_default_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let selected = self
            .authentication
            .read(cx)
            .value()
            .selected
            .as_deref()
            .unwrap_or("none");
        let label = if let Some(id) = selected.strip_prefix("profile:") {
            self.auth_profiles
                .iter()
                .map(|p| p.snapshot(cx))
                .chain(self.inherited_profiles.clone())
                .find(|p| p.id == id)
                .map(|p| p.name)
                .unwrap_or_else(|| "方案已删除，请重新选择".into())
        } else {
            match selected {
                "bearer" => "Bearer（原有配置）",
                "oauth2" => "OAuth 2.0（原有配置）",
                _ => "No Auth",
            }
            .into()
        };
        let mut view = div()
            .p_3()
            .rounded_md()
            .bg(rgb(p.panel))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .child(format!("新请求默认 · {label}")),
                    )
                    .child(
                        Button::new("不使用授权")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .when(selected == "none", |b| b.disabled())
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_auth_default("none".into(), cx)
                            })),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("在方案卡片上设为默认，仅影响此后新建的请求。"),
            );
        if selected == "bearer" {
            view = view.child(
                self.auth_profile_field("Token", &self.authentication.read(cx).text_input()),
            );
        }
        for profile in &self.inherited_profiles {
            let id = profile.id.clone();
            view = view.child(
                Button::new(format!("使用父目录方案 · {}", profile.name))
                    .kind(ButtonKind::Plain)
                    .size(ButtonSize::Mini)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_auth_default(format!("profile:{id}"), cx)
                    })),
            );
        }
        view
    }
    pub(super) fn auth_profiles_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let selected = self.authentication.read(cx).value().selected.clone();
        let mut view = div()
            .flex()
            .flex_col()
            .gap_3()
            .children(
                self.auth_error
                    .as_ref()
                    .map(|error| div().text_color(rgb(0xd9342b)).child(error.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("授权方案 · {}", self.auth_profiles.len())),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child("配置一次，当前目录和子目录的请求均可继承。"),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .debug_selector(|| "directory-auth-add".into())
                            .child(self.auth_add.clone()),
                    ),
            );
        if self.auth_profiles.is_empty() {
            view = view.child(
                div()
                    .p_5()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(p.border))
                    .text_size(px(12.))
                    .text_color(rgb(p.text_muted))
                    .child("还没有授权方案，添加 Bearer 或 OAuth 2.0 后，请求即可选择继承。"),
            );
        }
        for (index, profile) in self.auth_profiles.iter().enumerate() {
            let oauth = profile.mode.read(cx).value().selected.as_deref() == Some("oauth2");
            let is_default = self.sections.contains(&Section::Auth)
                && selected.as_deref() == Some(format!("profile:{}", profile.id).as_str());
            let name = profile.name.read(cx).value();
            let mut card = div()
                .id(("directory-auth-profile", index))
                .rounded_md()
                .border_1()
                .border_color(rgb(if is_default { p.accent } else { p.border }))
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .bg(rgb(p.panel))
                        .rounded_t_md()
                        .child(
                            div()
                                .id(("auth-profile-toggle", index))
                                .debug_selector(move || format!("auth-profile-toggle-{index}"))
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap_2()
                                .cursor_pointer()
                                .child(Icon::disclosure(14., profile.expanded))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(
                                            if name.is_empty() {
                                                "未命名方案"
                                            } else {
                                                name
                                            }
                                            .to_owned(),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(p.text_muted))
                                        .child(if oauth { "OAuth 2.0" } else { "Bearer" }),
                                )
                                .when(is_default, |row| {
                                    row.child(
                                        div()
                                            .px_2()
                                            .py(px(2.))
                                            .rounded_md()
                                            .bg(rgb(p.accent_tint))
                                            .text_color(rgb(p.accent))
                                            .text_size(px(10.))
                                            .child("新请求默认"),
                                    )
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.auth_profiles[index].expanded =
                                        !this.auth_profiles[index].expanded;
                                    cx.notify();
                                })),
                        )
                        .when(!is_default, |row| {
                            row.child(
                                Button::new("设为默认")
                                    .kind(ButtonKind::Plain)
                                    .size(ButtonSize::Mini)
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let id = this.auth_profiles[index].id.clone();
                                        this.set_auth_default(format!("profile:{id}"), cx);
                                    })),
                            )
                        })
                        .child(
                            IconButton::new(Icon::Trash, "删除授权方案")
                                .plain()
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.auth_profiles.remove(index);
                                    this.sync_auth_options(cx);
                                    cx.notify();
                                })),
                        ),
                );
            if profile.expanded {
                let owner = cx.entity();
                let mut form = div()
                    .id(("auth-profile-form", index))
                    .debug_selector(move || format!("auth-profile-form-{index}"))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap_4()
                            .child(self.auth_profile_field("方案名称", &profile.name))
                            .child(
                                SegmentedControl::new([
                                    SegmentOption::new("Bearer"),
                                    SegmentOption::new("OAuth 2.0"),
                                ])
                                .id(format!("auth-profile-mode-{index}"))
                                .selected(usize::from(oauth))
                                .theme(self.theme)
                                .on_change(move |mode, _, cx| {
                                    owner.update(cx, |this, cx| {
                                        this.auth_profiles[index].mode.update(cx, |field, cx| {
                                            field.set_selected(
                                                Some(
                                                    if mode == 1 { "oauth2" } else { "bearer" }
                                                        .into(),
                                                ),
                                                cx,
                                            )
                                        });
                                        cx.notify();
                                    });
                                })
                                .render()
                                .debug_selector(move || {
                                    format!("auth-profile-mode-control-{index}")
                                }),
                            ),
                    );
                if oauth {
                    form = form.child(
                        div()
                            .flex()
                            .gap_4()
                            .child(
                                self.auth_profile_field("Client ID", &profile.fields[2])
                                    .debug_selector(move || format!("auth-profile-client-{index}")),
                            )
                            .child(
                                self.auth_profile_field("Client Secret · 选填", &profile.secret)
                                    .debug_selector(move || format!("auth-profile-secret-{index}")),
                            ),
                    );
                    for (i, label) in [(0, "授权地址"), (1, "Token 地址")] {
                        form = form.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .w(px(76.))
                                        .flex_shrink_0()
                                        .text_size(px(11.))
                                        .text_color(rgb(p.text_muted))
                                        .child(label),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(self.auth_profile_input(&profile.fields[i])),
                                ),
                        );
                    }
                    form = form.child(
                        div()
                            .flex()
                            .gap_4()
                            .child(self.auth_profile_field("回调地址", &profile.fields[3]))
                            .child(self.auth_profile_field("Scope", &profile.fields[4])),
                    );
                    let summary = format!(
                        "{} · {} · {}",
                        if profile.config.pkce {
                            "PKCE S256"
                        } else {
                            "无 PKCE"
                        },
                        if profile.config.basic_auth {
                            "Basic 认证"
                        } else {
                            "表单认证"
                        },
                        if profile.config.manual_callback {
                            "手动回调"
                        } else {
                            "自动回调"
                        }
                    );
                    form = form.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new("高级设置")
                                    .disclosure(profile.advanced)
                                    .kind(ButtonKind::Plain)
                                    .size(ButtonSize::Mini)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(move || {
                                        format!("auth-profile-advanced-{index}")
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.auth_profiles[index].advanced =
                                            !this.auth_profiles[index].advanced;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.text_muted))
                                    .child(summary),
                            ),
                    );
                    if profile.advanced {
                        let mut options = div()
                            .id(("auth-profile-options", index))
                            .debug_selector(move || format!("auth-profile-options-{index}"))
                            .p_3()
                            .rounded_md()
                            .bg(rgb(p.panel))
                            .flex()
                            .flex_wrap()
                            .gap_4();
                        for (flag, label, checked) in [
                            (0, "PKCE S256", profile.config.pkce),
                            (1, "Basic 客户端认证", profile.config.basic_auth),
                            (2, "手动接收回调", profile.config.manual_callback),
                        ] {
                            let owner = cx.entity();
                            options = options.child(
                                Checkbox::new(label)
                                    .checked(checked)
                                    .theme(self.theme)
                                    .on_change(move |value, _, cx| {
                                        owner.update(cx, |this, cx| {
                                            let config = &mut this.auth_profiles[index].config;
                                            match flag {
                                                0 => config.pkce = value,
                                                1 => config.basic_auth = value,
                                                _ => config.manual_callback = value,
                                            };
                                            cx.notify();
                                        });
                                    })
                                    .render(),
                            );
                        }
                        form = form.child(options);
                    }
                } else {
                    form =
                        form.child(
                            div()
                                .debug_selector(move || format!("auth-profile-token-{index}"))
                                .child(self.auth_profile_field(
                                    "Token",
                                    &profile.mode.read(cx).text_input(),
                                )),
                        );
                }
                card = card.child(form);
            }
            view = view.child(card);
        }
        view
    }
}
