//! Workbench shell, navigation, request tabs, layout, and root rendering.

use super::*;

impl Workbench {
    pub(super) fn topbar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let request_controls = !self.show_vault
            && !self.show_proxies
            && !self.show_mcp
            && !self.show_plugins
            && !self.show_environments;
        div()
            .size_full()
            .flex()
            .items_center()
            .gap_3()
            .pl(px(if cfg!(target_os = "macos") { 80. } else { 16. }))
            .pr_3()
            .text_size(px(12.))
            .child(img(app_icon::image()).size(px(22.)).flex_shrink_0())
            .child(div().font_weight(FontWeight::SEMIBOLD).child(
                if self.host_navigation.is_some() {
                    "Glint Workbench"
                } else {
                    "Glint API"
                },
            ))
            .when(!self.show_vault, |bar| {
                bar.child(self.workspace_trigger(cx))
            })
            .child(
                div()
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child(if self.show_vault {
                        "›  密码箱".into()
                    } else if self.show_proxies {
                        "›  代理管理".into()
                    } else if self.show_mcp {
                        "›  MCP".into()
                    } else if self.show_plugins {
                        "›  插件管理".into()
                    } else if self.show_environments {
                        format!(
                            "›  环境管理  ›  {}",
                            self.environment_name(self.editing_environment)
                        )
                    } else {
                        self.requests
                            .get(self.active)
                            .map(|request| {
                                if request.group.is_empty() {
                                    format!("›  {}", request.collection)
                                } else {
                                    format!(
                                        "›  {}  ›  {}",
                                        request.collection,
                                        request.group.replace('/', "  ›  ")
                                    )
                                }
                            })
                            .unwrap_or_else(|| "›  暂无请求".into())
                    }),
            )
            .child(gpui_glint::components::window_drag_region())
            .when(request_controls, |bar| {
                bar.child(
                    div()
                        .id("open-command-search")
                        .w(px(240.))
                        .h(px(30.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(self.theme.palette().border))
                        .bg(rgb(self.theme.palette().panel))
                        .text_color(rgb(self.theme.palette().text_muted))
                        .text_size(px(11.))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(self.theme.palette().surface)))
                        .child(Icon::Search.render(15.).flex_shrink_0())
                        .child(div().flex_1().child("搜索命令与请求"))
                        .child(div().flex_shrink_0().text_size(px(10.)).child(
                            if cfg!(target_os = "macos") {
                                "⌘ K"
                            } else {
                                "Ctrl K"
                            },
                        ))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_commands(window, cx)),
                        ),
                )
            })
            .when(request_controls, |bar| {
                bar.child(
                    div()
                        .flex_shrink_0()
                        .debug_selector(|| "global-environment-selector".into())
                        .child(self.environment.clone()),
                )
            })
            .when(request_controls, |bar| {
                bar.child(self.background_tasks_menu(cx))
            })
            .when(request_controls, |bar| {
                bar.child(
                    IconButton::new(Icon::Sidebar, "展开或收起记录")
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(editor) = this.shared_request_editor(this.active) {
                                editor.update(cx, |editor, cx| {
                                    editor.show_activity = !editor.show_activity;
                                    cx.notify();
                                });
                            } else {
                                this.show_activity = !this.show_activity;
                            }
                            cx.notify();
                        })),
                )
            })
    }

    pub(super) fn rail(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .py_3()
            .child(
                IconButton::new(Icon::Folders, "请求集合")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_vault = false;
                        this.show_proxies = false;
                        this.show_mcp = false;
                        this.show_environments = false;
                        this.show_plugins = false;
                        this.show_collection_menu = false;
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new(Icon::Globe, "环境变量")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_vault = false;
                        this.show_proxies = false;
                        this.show_mcp = false;
                        this.show_plugins = false;
                        this.show_environments = true;
                        this.editing_environment = this.selected_environment(cx);
                        this.revealed_variables.clear();
                        this.variable_delete = None;
                        this.variable_error = None;
                        this.variable_search
                            .update(cx, |entry, cx| entry.set_value("", cx));
                        this.show_collection_menu = false;
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new(Icon::Shuffle, "代理管理")
                    .theme(self.theme)
                    .render()
                    .when(self.show_proxies, |button| {
                        button.bg(rgb(self.theme.palette().accent_tint))
                    })
                    .debug_selector(|| "open-proxy-manager".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_vault = false;
                        this.show_proxies = true;
                        this.show_environments = false;
                        this.show_plugins = false;
                        this.show_mcp = false;
                        this.variable_error = None;
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new(Icon::Script, "插件管理")
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "open-plugin-manager".into())
                    .on_click(cx.listener(|this, _, _, cx| this.open_plugin_manager(cx))),
            )
            .child(
                IconButton::new(Icon::Clock, "请求历史")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_vault = false;
                        this.show_proxies = false;
                        this.show_mcp = false;
                        this.show_plugins = false;
                        this.show_environments = false;
                        this.activity = 1;
                        this.show_activity = true;
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new(Icon::Workflow, "MCP 调用历史")
                    .theme(self.theme)
                    .render()
                    .when(self.show_mcp, |button| {
                        button.bg(rgb(self.theme.palette().accent_tint))
                    })
                    .debug_selector(|| "open-mcp-history".into())
                    .on_click(cx.listener(|this, _, _, cx| this.open_mcp_history(cx))),
            )
            .child(
                IconButton::new(Icon::Key, "密码箱")
                    .theme(self.theme)
                    .render()
                    .when(self.show_vault, |button| {
                        button.bg(rgb(self.theme.palette().accent_tint))
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_vault = true;
                        this.show_proxies = false;
                        this.show_mcp = false;
                        this.show_plugins = false;
                        this.show_environments = false;
                        this.directory_settings = None;
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(self.update_button(cx))
            .child(
                IconButton::new(Icon::Settings, "切换明暗主题")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx))),
            )
    }

    pub(super) fn workspace(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        if self
            .request_binding
            .as_ref()
            .is_some_and(|binding| binding.detached)
        {
            return div()
                .size_full()
                .p_5()
                .child("请求已删除、工作区已切换或主窗口已关闭。此窗口已停止编辑和执行。");
        }
        if self.requests.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .bg(rgb(p.surface))
                .text_color(rgb(p.text_muted))
                .child("暂无请求")
                .child(
                    Button::new(if self.collections.is_empty() {
                        "创建集合"
                    } else {
                        "新建请求"
                    })
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| this.add_request(cx))),
                );
        }
        let draft = self.snapshot(cx);
        let dirty = self.version_modified(self.active, &draft);
        let select_owner = cx.entity();
        let close_owner = select_owner.clone();
        let context_owner = select_owner.clone();
        let add_owner = select_owner.clone();
        let tabs = ClosableTabs::new(self.tabs.iter().map(|&i| {
            let r = &self.requests[i];
            let method = if i == self.active {
                &draft.method
            } else {
                &r.draft.method
            };
            ClosableTab::new(i as u64, format!("{method}  {}", r.name))
                .modified(i == self.active && dirty)
        }))
        .selected(Some(self.active as u64))
        .theme(self.theme)
        .fill_height()
        .content_padding(0.)
        .on_select(move |id, _, app| {
            select_owner.update(app, |this, cx| this.choose(id as usize, cx));
        })
        .on_close(move |id, _, app| {
            close_owner.update(app, |this, cx| {
                let id = id as usize;
                if this.tabs.len() > 1 {
                    this.tabs.retain(|&i| i != id);
                    if this.active == id {
                        this.choose(this.tabs[0], cx);
                    }
                    cx.notify();
                }
            });
        })
        .on_context(move |id, position, window, app| {
            context_owner.update(app, |this, cx| {
                this.tab_menu = Some((id as usize, position));
                this.show_collection_menu = false;
                this.tree_menu = None;
                window.focus(&this.focus, cx);
                cx.notify();
            });
        })
        .on_add(move |window, app| {
            add_owner.update(app, |this, cx| this.open_commands(window, cx));
        });
        if let Some(editor) = self.shared_request_editor(self.active) {
            return tabs
                .content(self.active as u64, div().size_full().child(editor))
                .render();
        }
        let top = div()
            .px_5()
            .py_4()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.request_name_view(cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_shrink_0()
                            .gap(px(8.))
                            .child(
                                Button::new("保存")
                                    .icon(Icon::Save)
                                    .kind(ButtonKind::Plain)
                                    .size(ButtonSize::Regular)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "request-save".into())
                                    .text_size(px(12.))
                                    .px(px(10.))
                                    .bg(rgb(p.panel))
                                    .border_color(rgb(p.panel))
                                    .text_color(rgb(p.text_muted))
                                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .debug_selector(|| "request-endpoint".into())
                            .flex_1()
                            .min_w_0()
                            .child(self.endpoint.clone()),
                    )
                    .child(
                        Button::new(if self.ws_commands.is_some() {
                            "断开"
                        } else if self.sending.is_some() {
                            "取消  ×"
                        } else if draft.method == "WS" {
                            "连接"
                        } else {
                            "发送  ↗"
                        })
                        .when(self.ws_commands.is_some(), |button| {
                            button.kind(ButtonKind::Danger)
                        })
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "request-send".into())
                        .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                    ),
            );
        let mut config_tabs = div()
            .h(px(37.))
            .flex_shrink_0()
            .px_5()
            .flex()
            .items_center()
            .gap_5()
            .border_b_1()
            .border_color(rgb(p.border));
        for (i, label) in [
            "参数",
            "身份验证",
            "请求头",
            "请求体",
            "请求设置",
            "前置动作",
            "后置动作",
        ]
        .iter()
        .enumerate()
        {
            config_tabs = config_tabs.child(
                tab_button(label, self.config == i, self.theme)
                    .debug_selector(move || format!("request-config-{i}").into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.config = i;
                        if i == 4 {
                            this.ws_refresh_decoder_select(cx);
                        }
                        this.pair_delete = None;
                        this.sync_pair_tables(cx);
                        cx.notify();
                    })),
            );
        }
        let content = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(p.surface))
            .child(top)
            .child(config_tabs)
            .child(self.configuration(cx))
            .child(
                div()
                    .id("request-response-resize-handle")
                    .debug_selector(|| "request-response-resize-handle".into())
                    .w_full()
                    .h(px(8.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .cursor(gpui::CursorStyle::ResizeUpDown)
                    .hover(move |style| style.bg(rgb(p.surface_subtle)))
                    .child(div().w_full().h(px(1.)).bg(rgb(p.border)))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.request_resize_start = Some((
                                event.position.y,
                                this.request_config_height
                                    .unwrap_or_else(|| this.default_configuration_height(cx)),
                            ));
                            cx.notify();
                        }),
                    ),
            )
            .child(self.response_panel(cx))
            .child(
                div()
                    .h(px(25.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .overflow_hidden()
                    .child(self.notice.clone()),
            );
        if self.request_binding.is_some() {
            content
        } else {
            tabs.content(self.active as u64, content).render()
        }
    }
    /// 按方向批量关闭标签页，被点击的标签始终保留；若当前标签被关闭则切到被点击的标签。

    pub(super) fn close_tabs_around(
        &mut self,
        keep: usize,
        mode: TabCloseMode,
        cx: &mut Context<Self>,
    ) {
        self.tab_menu = None;
        let Some(position) = self.tabs.iter().position(|&i| i == keep) else {
            cx.notify();
            return;
        };
        let before = self.tabs.len();
        self.tabs = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(index, _)| match mode {
                TabCloseMode::Others => *index == position,
                TabCloseMode::Left => *index >= position,
                TabCloseMode::Right => *index <= position,
            })
            .map(|(_, &i)| i)
            .collect();
        let closed = before - self.tabs.len();
        if closed == 0 {
            cx.notify();
            return;
        }
        if !self.tabs.contains(&self.active) {
            self.choose(keep, cx);
        }
        self.notice = format!("已关闭 {closed} 个标签页");
        cx.notify();
    }
    /// 标签页右键菜单。

    pub(super) fn tab_context_menu(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let (index, position) = self.tab_menu?;
        let tab_position = self.tabs.iter().position(|&i| i == index)?;
        let p = self.theme.palette();
        let menu = div()
            .id("request-tab-context-menu")
            .w(px(200.))
            .p_1()
            .rounded(px(7.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(2.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.tab_menu.take().is_some() {
                    cx.notify();
                }
            }))
            .child(
                MenuItem::new("tab-open-window", "在独立窗口中打开")
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "tab-open-window".into())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_request_window(index, window, cx);
                    })),
            )
            .child(
                MenuItem::new("tab-open-snapshot", "打开只读快照")
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "tab-open-snapshot".into())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_request_preview(index, window, cx)
                    })),
            )
            .child(div().h(px(1.)).my_1().bg(rgb(p.border)))
            .child(
                MenuItem::new("tab-close-others", "关闭其他标签页")
                    .disabled(self.tabs.len() <= 1)
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "tab-close-others".into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_tabs_around(index, TabCloseMode::Others, cx);
                    })),
            )
            .child(
                MenuItem::new("tab-close-left", "关闭左侧标签页")
                    .disabled(tab_position == 0)
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "tab-close-left".into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_tabs_around(index, TabCloseMode::Left, cx);
                    })),
            )
            .child(
                MenuItem::new("tab-close-right", "关闭右侧标签页")
                    .disabled(tab_position + 1 == self.tabs.len())
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "tab-close-right".into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_tabs_around(index, TabCloseMode::Right, cx);
                    })),
            );
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(position)
                    .snap_to_window_with_margin(px(8.))
                    .child(menu),
            )
            .with_priority(50)
            .into_any_element(),
        )
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.show_proxies || self.show_mcp || self.show_plugins || self.show_environments {
            self.show_vault = false;
        }
        self.vault.update(cx, |vault, cx| {
            vault.set_host_state(self.show_vault, self.theme, cx)
        });
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        {
            self.activate_html_window(window, cx);
            if self.html_preview
                && self.response_data.as_ref().is_some_and(|response| {
                    response.language() == "html"
                        && self.html_webview_source.as_deref() != Some(response.text.as_str())
                })
            {
                self.preview_html(window, cx);
            }
        }
        if let Some(pending) = &self.duplicate_version {
            if !pending.focus.is_focused(window) {
                window.focus(&pending.focus, cx);
            }
        }
        if self.group_edit.is_some() && !self.group_dialog_open {
            window.focus(&self.group_name.read(cx).focus_handle(cx), cx);
        }
        self.group_dialog_open = self.group_edit.is_some();
        if self.curl_import_needs_focus && self.show_curl_import {
            window.focus(&self.curl_input.read(cx).focus_handle(cx), cx);
            self.curl_import_needs_focus = false;
        }
        if !self.show_registry_dialog
            && [&self.registry_name, &self.registry_url, &self.registry_key]
                .iter()
                .any(|e| e.read(cx).focus_handle(cx).is_focused(window))
        {
            window.focus(&self.focus, cx);
        }
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        if let Some(view) = &self.html_webview {
            let visible = self.html_preview
                && self.sending.is_none()
                && self.error.is_none()
                && self.response_data.is_some()
                && !self.show_proxies
                && !self.show_environments
                && !self.show_plugins
                && !self.show_mcp
                && !self.show_curl_import
                && !self.show_workspaces
                && self.workspace_transfer.is_none()
                && self.group_edit.is_none()
                && self.duplicate_version.is_none()
                && self.directory_settings.is_none()
                && self.response_tab == 0
                && !self.commands.read(cx).is_open();
            if let Err(error) = view.update(cx, |view, cx| view.set_visible(visible, cx)) {
                self.notice = format!("WebView 显隐失败：{error}");
            }
        }
        let _ = &self.subscriptions;
        let content = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .child(if self.show_vault {
                div().size_full().child(self.vault.clone())
            } else if self.show_proxies {
                self.proxy_workspace(cx)
            } else if self.show_mcp {
                self.mcp_history_workspace(cx)
            } else if self.show_plugins {
                self.plugin_workspace(cx)
            } else if self.show_environments {
                self.environment_workspace(cx)
            } else if let Some(editor) = &self.directory_settings {
                div().size_full().child(editor.clone())
            } else {
                self.workspace(cx)
            })
            .when(
                self.show_activity
                    && !self.show_vault
                    && self.shared_request_editor(self.active).is_none()
                    && !self.show_proxies
                    && !self.show_environments
                    && !self.show_plugins
                    && !self.show_mcp
                    && self.directory_settings.is_none(),
                |s| s.child(self.activity_panel(cx)),
            );
        let content = div()
            .size_full()
            .flex()
            .flex_col()
            .when_some(
                self.request_binding
                    .as_ref()
                    .and_then(|binding| binding.storage_error.clone())
                    .or_else(|| self.storage_error.clone()),
                |el, error| {
                    el.child(
                        div()
                            .p_2()
                            .text_color(rgb(0xd9342b))
                            .child(format!("SQLite 保存失败：{error}。修改尚未写入磁盘。")),
                    )
                },
            )
            .child(content);
        let maximum_sidebar_width =
            (f32::from(window.viewport_size().width) - 46. - 640.).clamp(180., 480.);
        let maximum_request_config_height =
            (f32::from(window.viewport_size().height) - 360.).max(110.);
        self.sidebar_width = self.sidebar_width.clamp(180., maximum_sidebar_width);
        let resize_accent = self.theme.palette().accent;
        let resize_handle = div()
            .id("sidebar-resize-handle")
            .debug_selector(|| "sidebar-resize-handle".into())
            .size_full()
            .cursor(gpui::CursorStyle::ResizeLeftRight)
            .hover(move |style| style.bg(rgb(resize_accent)))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.sidebar_resize_start = Some((event.position.x, this.sidebar_width));
                    cx.notify();
                }),
            );
        let shell = if self.request_binding.is_some() {
            content.into_any_element()
        } else {
            AppLayout::new(self.topbar(cx), self.sidebar(cx), content)
                .navigation_rail(
                    self.host_navigation
                        .clone()
                        .map(|rail| rail.into_any_element())
                        .unwrap_or_else(|| self.rail(cx).into_any_element()),
                )
                .navigation_rail_width(46.)
                .sidebar_visible(!self.show_vault)
                .sidebar_width(self.sidebar_width)
                .sidebar_resize_handle(resize_handle)
                .theme(self.theme)
                .into_any_element()
        };
        let overlay = self
            .commands
            .update(cx, |commands, cx| commands.overlay(cx));
        div()
            .key_context("ApiWorkbench")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .text_size(px(12.))
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    if let Some((start, width)) = this.sidebar_resize_start {
                        if event.dragging() {
                            this.sidebar_width = (width + f32::from(event.position.x - start))
                                .clamp(180., maximum_sidebar_width);
                        } else {
                            this.sidebar_resize_start = None;
                        }
                        cx.stop_propagation();
                        cx.notify();
                    } else if let Some((start, height)) = this.request_resize_start {
                        if event.dragging() {
                            this.request_config_height = Some(
                                (height + f32::from(event.position.y - start))
                                    .clamp(110., maximum_request_config_height),
                            );
                        } else {
                            this.request_resize_start = None;
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.sidebar_resize_start.take().is_some() {
                        cx.notify();
                    }
                    if this.request_resize_start.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.sidebar_resize_start.take().is_some() {
                        cx.notify();
                    }
                    if this.request_resize_start.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &SendRequest, _, cx| {
                if this.show_vault {
                    cx.stop_propagation();
                    return;
                }
                if this.request_binding.is_some() {
                    cx.stop_propagation();
                }
                if this.show_workspaces
                    || this.workspace_transfer.is_some()
                    || this.group_edit.is_some()
                    || this.duplicate_version.is_some()
                {
                    return;
                }
                if !this.show_mcp
                    && !this.show_plugins
                    && !this.show_environments
                    && this.directory_settings.is_none()
                {
                    if this.ws_connected {
                        this.send_websocket_message(cx);
                    } else {
                        this.send(cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SaveRequest, _, cx| {
                if this.show_vault {
                    cx.stop_propagation();
                    return;
                }
                if this.request_binding.is_some() {
                    cx.stop_propagation();
                }
                if this.show_workspaces
                    || this.workspace_transfer.is_some()
                    || this.group_edit.is_some()
                    || this.duplicate_version.is_some()
                {
                    return;
                }
                if this.show_plugins || this.show_mcp {
                    return;
                } else if this.show_environments {
                    this.save_environment_variables(cx);
                } else if let Some(editor) = this.directory_settings.clone() {
                    editor.update(cx, |editor, cx| editor.save(cx));
                } else {
                    this.save(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SearchResponse, window, cx| {
                if this.show_vault {
                    cx.stop_propagation();
                    return;
                }
                if this.request_binding.is_some() {
                    cx.stop_propagation();
                }
                if this.group_edit.is_none()
                    && !this.show_workspaces
                    && this.workspace_transfer.is_none()
                    && !this.show_mcp
                    && !this.show_plugins
                    && !this.show_environments
                    && this.response_data.is_some()
                {
                    this.response_tab = 0;
                    if !this.show_response_search {
                        this.toggle_response_search(window, cx);
                    } else {
                        window.focus(&this.response_search.focus_handle(cx), cx);
                    }
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &SearchCommands, w, cx| {
                if this.request_binding.is_some() {
                    // An embedded shared editor uses the main window's workspace
                    // commands; standalone request windows keep their local palette.
                    if w.window_handle()
                        .downcast::<request_window::RequestWindow>()
                        .is_none()
                    {
                        cx.propagate();
                        return;
                    }
                    cx.stop_propagation();
                }
                if this.group_edit.is_none()
                    && !this.show_workspaces
                    && this.workspace_transfer.is_none()
                    && !this.show_registry_dialog
                    && !this.show_curl_import
                {
                    this.open_commands(w, cx);
                }
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.workspace_transfer.is_some() {
                    if event.keystroke.key == "escape" && this.workspace_transfer_task.is_none() {
                        this.workspace_transfer = None;
                        window.focus(&this.focus, cx);
                        cx.notify();
                        cx.stop_propagation();
                    }
                    return;
                }
                if this.show_workspaces {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            this.show_workspaces = false;
                            window.focus(&this.focus, cx);
                            cx.notify();
                            cx.stop_propagation();
                        }
                        "enter"
                            if this
                                .workspace_name_input
                                .read(cx)
                                .focus_handle(cx)
                                .is_focused(window) =>
                        {
                            this.submit_workspace(window, cx);
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                    return;
                }
                if this.show_curl_import {
                    if event.keystroke.key == "escape" {
                        this.show_curl_import = false;
                        window.focus(&this.focus, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    return;
                }
                if this.show_registry_dialog {
                    if event.keystroke.key == "tab" {
                        let fields = [&this.registry_name, &this.registry_url, &this.registry_key];
                        let current = fields
                            .iter()
                            .position(|e| e.read(cx).focus_handle(cx).is_focused(window))
                            .unwrap_or(0);
                        let next = if event.keystroke.modifiers.shift {
                            (current + 2) % 3
                        } else {
                            (current + 1) % 3
                        };
                        window.focus(&fields[next].read(cx).focus_handle(cx), cx);
                        cx.stop_propagation();
                    }
                    if event.keystroke.key == "escape" {
                        if !this.registry_adding_source {
                            this.show_registry_dialog = false;
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }
                    return;
                }
                if event.keystroke.key == "escape" && this.tab_menu.is_some() {
                    this.tab_menu = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if event.keystroke.key == "escape" && this.pair_delete.is_some() {
                    this.pair_delete = None;
                    this.sync_pair_tables(cx);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if event.keystroke.key == "escape" && this.history_delete.is_some() {
                    this.history_delete = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if event.keystroke.key == "escape"
                    && (this.json_pick.is_some() || this.json_menu.is_some())
                {
                    this.cancel_json_pick(cx);
                    this.json_menu = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if event.keystroke.key == "escape" && this.show_update_details {
                    this.show_update_details = false;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if event.keystroke.key == "escape" && this.request_menu.is_some() {
                    this.request_menu = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if event.keystroke.key == "escape" && this.tree_delete.is_some() {
                    this.tree_delete = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if event.keystroke.key == "escape" && this.tree_menu.is_some() {
                    this.tree_menu = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if this.show_collection_menu {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            this.show_collection_menu = false;
                            cx.stop_propagation();
                            cx.notify();
                        }
                        "enter" => {
                            this.edit_collection(window, cx);
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                }
            }))
            .child(shell)
            .child(self.toasts.clone())
            .children(overlay)
            .children(self.tab_context_menu(cx))
            .children(self.registry_dialog(cx))
            .children(self.curl_import_dialog(cx))
            .children(self.workspace_dialog(cx))
            .children(self.workspace_transfer_dialog(cx))
            .children(self.group_dialog(cx))
            .children(self.duplicate_version_dialog(cx))
    }
}
