use super::*;
use serde_json::{Value, json};

pub struct State {
    config: Value,
    selected: Option<String>,
    name: Entity<TextEntry>,
    endpoint: Entity<TextEntry>,
    args: Entity<TextEntry>,
    env: Entity<TextEntry>,
    http: bool,
    enabled: bool,
    notice: Option<String>,
    tools: Vec<(String, String)>,
    testing: Option<Task<()>>,
    generation: u64,
    confirm_delete: bool,
    advanced: bool,
    show_path: bool,
}
impl State {
    pub fn new(theme: ThemePreset, cx: &mut Context<AgentWorkspace>) -> Self {
        let mut entry = |hint: &str, value: &str| {
            let input =
                cx.new(|cx| TextEntry::new(cx, hint.to_owned(), value.to_owned()).theme(theme));
            cx.observe(&input, |this, _, cx| {
                this.mcp_ui.invalidate();
                cx.notify();
            })
            .detach();
            input
        };
        Self {
            config: json!({"mcpServers":{}}),
            selected: None,
            name: entry("服务器名称", ""),
            endpoint: entry("程序绝对路径或 MCP URL", ""),
            args: entry("JSON 数组，例如 [\"--port\",\"8000\"]", "[]"),
            env: entry("JSON 对象，例如 {\"SETTING\":\"value\"}", "{}"),
            http: false,
            enabled: true,
            notice: None,
            tools: vec![],
            testing: None,
            generation: 0,
            confirm_delete: false,
            advanced: false,
            show_path: false,
        }
    }
    pub fn set_theme(&self, theme: ThemePreset, cx: &mut Context<AgentWorkspace>) {
        for input in [&self.name, &self.endpoint, &self.args, &self.env] {
            input.update(cx, |entry, cx| entry.set_theme(theme, cx));
        }
    }
    fn invalidate(&mut self) {
        self.generation += 1;
        self.testing = None;
        self.tools.clear();
        self.confirm_delete = false;
    }
    fn server(&self, cx: &App) -> Result<(String, Value), String> {
        parse_server(
            self.name.read(cx).value(),
            self.endpoint.read(cx).value(),
            self.args.read(cx).value(),
            self.env.read(cx).value(),
            self.http,
            self.enabled,
        )
    }
}
fn parse_server(
    name: &str,
    endpoint: &str,
    args: &str,
    env: &str,
    http: bool,
    enabled: bool,
) -> Result<(String, Value), String> {
    let name = name.trim().to_owned();
    let endpoint = endpoint.trim();
    if name.is_empty() || endpoint.is_empty() {
        return Err("请填写服务器名称和连接地址。".into());
    }
    if http {
        let url = reqwest::Url::parse(endpoint).map_err(|_| "请输入有效的 HTTP / HTTPS 地址")?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("远程连接仅支持 HTTP / HTTPS。".into());
        }
        Ok((name, json!({"url":endpoint,"enabled":enabled})))
    } else {
        let args: Vec<String> =
            serde_json::from_str(args).map_err(|_| "参数必须是 JSON 字符串数组。")?;
        let env: std::collections::BTreeMap<String, String> =
            serde_json::from_str(env).map_err(|_| "环境变量必须是 JSON 字符串对象。")?;
        Ok((
            name,
            json!({"command":endpoint,"args":args,"env":env,"enabled":enabled}),
        ))
    }
}
impl AgentWorkspace {
    pub fn reload_mcp(&mut self, cx: &mut Context<Self>) {
        self.mcp_ui.invalidate();
        match mcp::Client::new(self.path.with_file_name("mcp.json"))
            .read_config()
            .and_then(|source| serde_json::from_str::<Value>(&source).map_err(|e| e.to_string()))
        {
            Ok(mut config)
                if config.is_object()
                    && (config["mcpServers"].is_null() || config["mcpServers"].is_object()) =>
            {
                if config["mcpServers"].is_null() {
                    config["mcpServers"] = json!({});
                }
                self.mcp_ui.config = config;
                self.select_mcp(None, cx);
            }
            Ok(_) => self.mcp_ui.notice = Some("配置格式无效：mcpServers 必须是对象。".into()),
            Err(error) => self.mcp_ui.notice = Some(error),
        }
        cx.notify();
    }
    fn select_mcp(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.mcp_ui.invalidate();
        self.mcp_ui.notice = None;
        let server = name
            .as_ref()
            .map(|n| self.mcp_ui.config["mcpServers"][n].clone())
            .unwrap_or(json!({}));
        let state = &mut self.mcp_ui;
        state.http = server["url"].is_string();
        state.enabled = server["enabled"].as_bool().unwrap_or(true);
        state.name.update(cx, |entry, cx| {
            entry.set_value(name.clone().unwrap_or_default(), cx)
        });
        state.endpoint.update(cx, |entry, cx| {
            entry.set_value(
                server[if state.http { "url" } else { "command" }]
                    .as_str()
                    .unwrap_or(""),
                cx,
            )
        });
        state.args.update(cx, |entry, cx| {
            entry.set_value(server.get("args").unwrap_or(&json!([])).to_string(), cx)
        });
        state.env.update(cx, |entry, cx| {
            entry.set_value(server.get("env").unwrap_or(&json!({})).to_string(), cx)
        });
        state.selected = name;
        cx.notify();
    }
    fn save_mcp(&mut self, remove: bool, cx: &mut Context<Self>) {
        let result = (|| {
            let client = mcp::Client::new(self.path.with_file_name("mcp.json"));
            // Read current disk content to preserve changes made by the agent.
            let mut config: Value =
                serde_json::from_str(&client.read_config()?).map_err(|e| e.to_string())?;
            if config["mcpServers"].is_null() {
                config["mcpServers"] = json!({});
            }
            let servers = config["mcpServers"]
                .as_object_mut()
                .ok_or("mcpServers 必须是对象")?;
            let mut saved_name = None;
            if remove {
                servers.remove(self.mcp_ui.selected.as_deref().ok_or("请先选择服务器")?);
            } else {
                let (name, server) = self.mcp_ui.server(cx)?;
                if self.mcp_ui.selected.as_deref() != Some(&name) && servers.contains_key(&name) {
                    return Err("同名服务器已存在。".into());
                }
                if let Some(old) = &self.mcp_ui.selected {
                    servers.remove(old);
                }
                servers.insert(name.clone(), server);
                saved_name = Some(name);
            }
            client.write_config(&config)?;
            self.mcp_ui.config = config;
            self.mcp_ui.selected = saved_name;
            Ok::<_, String>(())
        })();
        self.mcp_ui.notice = Some(match result {
            Ok(()) => {
                self.mcp_ui.invalidate();
                if remove {
                    self.select_mcp(None, cx);
                }
                "已保存，下次对话请求生效。".into()
            }
            Err(error) => error,
        });
        cx.notify();
    }
    fn test_mcp(&mut self, cx: &mut Context<Self>) {
        let result = self.mcp_ui.server(cx).and_then(|(name, server)| {
            let root = self.history.active().project.clone();
            provider::Job::start(mcp::Client::probe(name, server, root))
        });
        match result {
            Err(error) => self.mcp_ui.notice = Some(error),
            Ok(task) => {
                self.mcp_ui.invalidate();
                let generation = self.mcp_ui.generation;
                self.mcp_ui.notice = Some("正在连接并读取工具…".into());
                self.mcp_ui.testing = Some(cx.spawn(async move |weak, cx| {
                    let result = task.result().await;
                    let _ = weak.update(cx, |this, cx| {
                        if this.mcp_ui.generation != generation {
                            return;
                        }
                        this.mcp_ui.testing = None;
                        match result {
                            Ok(tools) => {
                                this.mcp_ui.notice = Some(format!(
                                    "连接成功 · {} 个工具（本次检查结果）",
                                    tools.len()
                                ));
                                this.mcp_ui.tools = tools;
                            }
                            Err(e) => this.mcp_ui.notice = Some(e),
                        }
                        cx.notify();
                    });
                }));
            }
        }
        cx.notify();
    }
    pub fn mcp_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        use gpui_glint::components::{SegmentOption, SegmentedControl, Switch};
        let p = self.theme.palette();
        let theme = self.theme;
        let state = &self.mcp_ui;
        let servers = state.config["mcpServers"].as_object();
        let count = servers.map_or(0, |s| s.len());
        let quiet = |label: &str| {
            Button::new(label.to_owned())
                .kind(ButtonKind::Plain)
                .size(ButtonSize::Small)
                .theme(theme)
                .render()
        };
        let caption = |text: &str| {
            div()
                .text_size(px(12.))
                .text_color(rgb(p.text_muted))
                .child(text.to_owned())
        };
        let field = |label: &str, hint: &str, input: &Entity<TextEntry>| {
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(label.to_owned()),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .h(px(38.))
                        .px_3()
                        .flex()
                        .items_center()
                        .bg(rgb(p.surface_subtle))
                        .rounded(px(7.))
                        .border_1()
                        .border_color(rgb(p.border))
                        .child(input.clone()),
                )
                .when(!hint.is_empty(), |s| s.child(caption(hint)))
        };
        let mut list = div()
            .w(px(196.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .h(px(36.))
                    .mb_3()
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(p.text_muted))
                            .child(format!("服务器 · {count}")),
                    )
                    .child(
                        quiet("刷新").on_click(cx.listener(|this, _, _, cx| this.reload_mcp(cx))),
                    ),
            );
        for (index, (name, server)) in servers.into_iter().flatten().enumerate() {
            let selected = state.selected.as_ref() == Some(name);
            let name_click = name.clone();
            let enabled = server["enabled"].as_bool().unwrap_or(true);
            list = list.child(
                div()
                    .id(("mcp-server", index))
                    .w_full()
                    .p_3()
                    .flex()
                    .gap_3()
                    .items_center()
                    .rounded(px(9.))
                    .cursor_pointer()
                    .when(selected, |s| s.bg(rgb(p.surface_subtle)))
                    .hover(|s| s.bg(rgb(p.surface_subtle)))
                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(if enabled {
                        p.accent
                    } else {
                        p.text_muted
                    })))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(name.clone()),
                            )
                            .child(caption(if server["url"].is_string() {
                                "远程 · HTTP"
                            } else {
                                "本地 · stdio"
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_mcp(Some(name_click.clone()), cx)
                    })),
            );
        }
        if count == 0 {
            list = list.child(
                div()
                    .px_3()
                    .py_6()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_size(px(13.)).child("还没有服务器"))
                    .child(caption("连接 MCP，为 Agent 添加外部工具。")),
            );
        }
        list = list.child(
            div().mt_3().child(
                quiet("＋ 添加服务器")
                    .w_full()
                    .on_click(cx.listener(|this, _, _, cx| this.select_mcp(None, cx))),
            ),
        );
        let transport_weak = cx.entity().downgrade();
        let switch_weak = cx.entity().downgrade();
        let mut form = div()
            .p_5()
            .flex()
            .flex_col()
            .gap_5()
            .child(field("服务器名称", "", &state.name))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(caption("连接方式"))
                    .child(
                        SegmentedControl::new([
                            SegmentOption::new("本地程序"),
                            SegmentOption::new("远程服务"),
                        ])
                        .id("mcp-transport")
                        .selected(usize::from(state.http))
                        .theme(theme)
                        .on_change(move |index, _, cx| {
                            let _ = transport_weak.update(cx, |this, cx| {
                                this.mcp_ui.http = index == 1;
                                this.mcp_ui.invalidate();
                                cx.notify();
                            });
                        })
                        .render()
                        .w_full(),
                    ),
            )
            .child(field(
                if state.http {
                    "服务器地址"
                } else {
                    "启动程序"
                },
                if state.http {
                    "Streamable HTTP 端点，例如 https://example.com/mcp"
                } else {
                    "使用程序的绝对路径，避免应用启动时找不到命令。"
                },
                &state.endpoint,
            ));
        if !state.http {
            form = form.child(
                div()
                    .id("mcp-advanced")
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(p.text_muted))
                    .cursor_pointer()
                    .child(if state.advanced { "⌄" } else { "›" })
                    .child("参数与环境变量")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.mcp_ui.advanced = !this.mcp_ui.advanced;
                        cx.notify();
                    })),
            );
            if state.advanced {
                form = form.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(field(
                            "启动参数",
                            "JSON 数组，例如 [\"--port\", \"8000\"]",
                            &state.args,
                        ))
                        .child(field(
                            "环境变量",
                            "JSON 对象；内容以明文保存在本机。",
                            &state.env,
                        )),
                );
            }
        }
        let mut card = div()
            .flex_1()
            .min_w_0()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.panel))
            .overflow_hidden()
            .child(
                div()
                    .px_5()
                    .py_4()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if state.selected.is_some() {
                                        "编辑服务器"
                                    } else {
                                        "新建连接"
                                    }),
                            )
                            .child(caption(if state.selected.is_some() {
                                "更新连接配置与可用状态"
                            } else {
                                "选择连接方式，添加 Agent 可调用的工具"
                            })),
                    )
                    .child(Icon::Workflow.render(20.).text_color(rgb(p.text_muted))),
            )
            .child(form)
            .child(
                div()
                    .mx_5()
                    .py_4()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_size(px(13.)).child("在对话中使用"))
                            .child(caption("保存后，从下一次请求开始生效")),
                    )
                    .child(
                        Switch::new("")
                            .checked(state.enabled)
                            .theme(theme)
                            .on_change(move |enabled, _, cx| {
                                let _ = switch_weak.update(cx, |this, cx| {
                                    this.mcp_ui.enabled = enabled;
                                    cx.notify();
                                });
                            })
                            .render(),
                    ),
            )
            .child(
                div()
                    .px_5()
                    .py_4()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .when(state.selected.is_some(), |s| {
                                s.child(
                                    Button::new(if state.confirm_delete {
                                        "确认删除"
                                    } else {
                                        "删除服务器"
                                    })
                                    .kind(ButtonKind::DangerTinted)
                                    .size(ButtonSize::Small)
                                    .theme(theme)
                                    .render()
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            if this.mcp_ui.confirm_delete {
                                                this.save_mcp(true, cx);
                                            } else {
                                                this.mcp_ui.confirm_delete = true;
                                                cx.notify();
                                            }
                                        },
                                    )),
                                )
                            })
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new(if state.testing.is_some() {
                                            "检查中…"
                                        } else {
                                            "检查连接"
                                        })
                                        .kind(ButtonKind::Plain)
                                        .theme(theme)
                                        .when(state.testing.is_some(), |b| b.disabled())
                                        .render()
                                        .on_click(cx.listener(|this, _, _, cx| this.test_mcp(cx))),
                                    )
                                    .child(Button::new("保存连接").theme(theme).render().on_click(
                                        cx.listener(|this, _, _, cx| this.save_mcp(false, cx)),
                                    )),
                            ),
                    ),
            );
        if let Some(notice) = &state.notice {
            card = card.child(
                div()
                    .px_5()
                    .py_3()
                    .bg(rgb(p.surface_subtle))
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .text_size(px(12.))
                    .child(notice.clone()),
            );
        }
        let mut detail = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_4()
            .child(card)
            .child(caption(
                "检查连接会运行程序或访问远程服务，请只连接可信的服务器。",
            ));
        if !state.tools.is_empty() {
            let mut tools = div()
                .rounded(px(12.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.panel))
                .p_5()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("可用工具"),
                        )
                        .child(caption(&format!("{} 个 · 最近检查", state.tools.len()))),
                );
            for (name, description) in &state.tools {
                tools = tools.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(name.clone()),
                        )
                        .child(caption(description)),
                );
            }
            detail = detail.child(tools);
        }
        self.settings_container(
            div()
                .flex()
                .flex_col()
                .gap_6()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(24.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("MCP"),
                        )
                        .child(caption("连接外部工具，扩展 Agent 的能力。")),
                )
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_start()
                        .gap_6()
                        .child(list)
                        .child(detail),
                )
                .child(
                    div()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgb(p.border))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .id("mcp-config-location")
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .cursor_pointer()
                                .child(if state.show_path {
                                    "⌄ 配置文件位置"
                                } else {
                                    "› 配置文件位置"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.mcp_ui.show_path = !this.mcp_ui.show_path;
                                    cx.notify();
                                })),
                        )
                        .when(state.show_path, |s| {
                            s.child(caption(
                                &self.path.with_file_name("mcp.json").display().to_string(),
                            ))
                        }),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_forms_validate_transport_and_preserve_arguments() {
        let (_, local) = parse_server(
            "local",
            "/bin/server",
            r#"["argument with spaces"]"#,
            r#"{"SETTING":"hello"}"#,
            false,
            true,
        )
        .unwrap();
        assert_eq!(local["args"][0], "argument with spaces");
        assert_eq!(local["env"]["SETTING"], "hello");
        assert!(local.get("url").is_none());
        let (_, remote) = parse_server(
            "remote",
            "https://example.com/mcp",
            "invalid ignored",
            "",
            true,
            false,
        )
        .unwrap();
        assert_eq!(remote["enabled"], false);
        assert!(remote.get("command").is_none());
        for (name, endpoint, args, env, http) in [
            ("", "x", "[]", "{}", false),
            ("x", "", "[]", "{}", false),
            ("x", "file:///tmp/mcp", "[]", "{}", true),
            ("x", "command", "[1]", "{}", false),
            ("x", "command", "[]", "{invalid}", false),
        ] {
            assert!(parse_server(name, endpoint, args, env, http, true).is_err());
        }
    }
}
