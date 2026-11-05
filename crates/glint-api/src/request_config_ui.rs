//! Request configuration tabs, body editors, and panel sizing.

use super::*;

pub(super) fn binary_body_description(body: Option<&model::BinaryBody>) -> String {
    body.map_or_else(|| "未选择文件".into(), model::BinaryBody::description)
}

pub(super) fn choose_binary_file<T: 'static>(
    cx: &mut Context<T>,
    set_body: impl FnOnce(&mut T, Result<Option<model::BinaryBody>, String>, &mut Context<T>) + 'static,
) {
    choose_request_file(cx, |path| model::BinaryBody::read(&path), set_body);
}

pub(super) fn choose_request_file<T: 'static, F: Send + 'static>(
    cx: &mut Context<T>,
    read: impl FnOnce(std::path::PathBuf) -> Result<F, String> + Send + 'static,
    set_file: impl FnOnce(&mut T, Result<Option<F>, String>, &mut Context<T>) + 'static,
) {
    let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("选择二进制请求体文件".into()),
    });
    cx.spawn(async move |this, cx| {
        let result = async {
            let paths = picker
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
                return Ok(None);
            };
            cx.background_executor()
                .spawn(async move { read(path) })
                .await
                .map(Some)
        }
        .await;
        let _ = this.update(cx, |this, cx| match result {
            Ok(Some(file)) => set_file(this, Ok(Some(file)), cx),
            Ok(None) => {}
            Err(error) => set_file(this, Err(error), cx),
        });
    })
    .detach();
}

pub(super) fn binary_body_editor<T: 'static>(
    body: Option<&model::BinaryBody>,
    theme: ThemePreset,
    set_body: impl Fn(&mut T, Result<Option<model::BinaryBody>, String>, &mut Context<T>)
    + Clone
    + 'static,
    cx: &mut Context<T>,
) -> gpui::Div {
    let choose = set_body.clone();
    div().flex().flex_col().gap_2().child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("选择文件")
                    .theme(theme)
                    .render()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        choose_binary_file(cx, choose.clone());
                    })),
            )
            .when(body.is_some(), |row| {
                row.child(
                    Button::new("移除")
                        .kind(ButtonKind::Plain)
                        .theme(theme)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| set_body(this, Ok(None), cx))),
                )
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(binary_body_description(body)),
            ),
    )
}

impl Workbench {
    pub(super) fn request_setting_row(
        &self,
        title: &'static str,
        description: &'static str,
        control: impl IntoElement,
    ) -> gpui::Div {
        let p = self.theme.palette();
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_5()
            .py_3()
            .border_b_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(description),
                    ),
            )
            .child(
                div()
                    .w(px(180.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(control),
            )
    }

    pub(super) fn body_configuration(&self, cx: &mut Context<Self>) -> gpui::Div {
        if self.snapshot(cx).method == "WS" {
            return div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .child("待发送的文本消息 · 连接后点击发送消息"),
                )
                .child(
                    div()
                        .debug_selector(|| "request-body-editor".into())
                        .min_w_0()
                        .flex_shrink_0()
                        .child(self.body.clone()),
                );
        }
        let owner = cx.entity().downgrade();
        let mut head = div().flex().items_center().gap_3().child(
            SegmentedControl::new([
                SegmentOption::new("JSON / 原始"),
                SegmentOption::new("x-www-form-urlencoded"),
                SegmentOption::new("二进制文件"),
            ])
            .id("request-body-format")
            .selected(self.body_format.index())
            .theme(self.theme)
            .on_change(move |index, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.body_format = BodyFormat::from_index(index);
                    cx.notify();
                });
            })
            .render()
            .debug_selector(|| "request-body-format".into()),
        );
        if self.body_format == BodyFormat::Raw {
            head = head.child(div().flex_1()).child(
                Button::new("格式化 JSON")
                    .kind(ButtonKind::Plain)
                    .size(ButtonSize::Mini)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        let valid = this.body.update(cx, |editor, cx| editor.format_json(cx));
                        this.notice = if valid {
                            "已格式化 JSON"
                        } else {
                            "JSON 格式有误，请检查后重试"
                        }
                        .into();
                        cx.notify();
                    })),
            );
        }
        // Editable code cards grow with their content. Keep their full height in
        // the configuration scroll area rather than shrinking a wrapper around
        // them, which hides the bottom of the card behind the response panel.
        let content = div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(head.flex_shrink_0());
        if self.body_format == BodyFormat::UrlEncoded {
            content.child(self.form_table.clone()).child(div().text_size(px(10.)).text_color(rgb(self.theme.palette().text_muted))
                .child("仅发送已启用字段，支持同名字段和模板变量；无需手动编码。自动使用 application/x-www-form-urlencoded，显式 Content-Type 优先。"))
        } else if self.body_format == BodyFormat::Binary {
            let request = &self.workspace.requests[self.active];
            let request_id = request.id.clone();
            let mut files = div().flex().flex_wrap().gap_2();
            if let Some(collection) = self
                .workspace
                .collections
                .iter()
                .find(|c| c.collection == request.collection)
            {
                for name in collection.file_variables.keys() {
                    let selected = self.binary_file_variable.as_ref() == Some(name);
                    let name = name.clone();
                    files = files.child(
                        Button::new(name.clone())
                            .kind(if selected {
                                ButtonKind::Primary
                            } else {
                                ButtonKind::Gray
                            })
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.binary_file_variable = Some(name.clone());
                                cx.notify();
                            })),
                    );
                }
            }
            let mut body = content
                .child(
                    div()
                        .text_size(px(10.))
                        .child("集合文件变量 · 在集合变量页添加文件"),
                )
                .child(files);
            if let Some(name) = &self.binary_file_variable {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(format!("已绑定：{name}"))
                        .child(
                            Button::new("解除绑定")
                                .kind(ButtonKind::Plain)
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.binary_file_variable = None;
                                    cx.notify();
                                })),
                        ),
                );
            }
            body.child(binary_body_editor(
                self.binary_body.as_ref(),
                self.theme,
                move |this: &mut Self, body, cx| {
                    if this.workspace.requests[this.active].id == request_id {
                        match body {
                            Ok(body) => {
                                if body.is_some() {
                                    this.binary_file_variable = None;
                                }
                                this.notice = binary_body_description(body.as_ref()).into();
                                this.binary_body = body;
                            }
                            Err(error) => this.notice = error.into(),
                        }
                        cx.notify();
                    }
                },
                cx,
            ))
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child("直接发送文件字节，最大 16 MiB。默认使用 application/octet-stream。"),
            )
        } else {
            content.child(
                div()
                    .debug_selector(|| "request-body-editor".into())
                    .min_w_0()
                    .flex_shrink_0()
                    .child(self.body.clone()),
            )
        }
    }

    pub(super) fn default_configuration_height(&self, cx: &Context<Self>) -> f32 {
        if self.config == 1
            && self.authentication.read(cx).value().selected.as_deref() == Some("oauth2")
        {
            384.
        } else if matches!(self.config, 5 | 6) {
            360.
        } else {
            218.
        }
    }

    pub(super) fn configuration_height(&self, cx: &Context<Self>) -> f32 {
        self.request_config_height
            .unwrap_or_else(|| self.default_configuration_height(cx))
    }

    pub(super) fn configuration(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let owner = cx.entity().downgrade();
        let view=match self.config {
            0=>self.pair_table(false),
            2=>div().flex().flex_col().gap_3()
                .child(self.pair_table(true))
                .child(div().text_size(px(11.)).font_weight(FontWeight::MEDIUM).child("Cookie 覆盖"))
                .child(self.cookies_table.clone())
                .child(div().text_size(px(10.)).text_color(rgb(p.text_muted)).child("将启用的 Cookie 键值合并到请求头 Cookie；同名键以此表为准，其余原始 Cookie 保留。禁用条目不参与覆盖。")),
            1=>self.authentication_configuration(cx),
            5=>self.plugin_configuration(cx),
            6=>div().flex().flex_col().gap_3()
                .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).child("提取环境变量"))
                .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("响应后写入所属集合在发送时环境下的变量，不修改全局变量；集合内同名变量覆盖。例：api_token → $.data.token。多个匹配保存为 JSON 数组；任一规则失败则不写入。有效期填响应中的 JSONPath（如 $.expires_in），单位秒，留空表示不过期。"))
                .child(self.extractors_table.clone()),
            3=>self.body_configuration(cx),
            _=>div().w_full().flex().flex_col()
                .child(self.request_setting_row("跳过 TLS 证书校验",
                    "允许自签名、过期或域名不匹配的证书，仅作用于当前请求。",
                    Switch::new("")
                    .theme(self.theme)
                    .checked(self.skip_tls_verification)
                    .on_change(move |checked, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.skip_tls_verification = checked;
                            cx.notify();
                        });
                    })
                    .render().debug_selector(|| "skip-tls-verification".into())))
                .when(self.workspace.requests.get(self.active).is_some_and(|request| request.draft.method == "WS"), |settings| settings
                    .child(self.request_setting_row("事件解密插件", "用于解密当前请求接收的 WebSocket 事件，配置随请求保存。",
                        div().id("websocket-event-decoder").debug_selector(|| "websocket-event-decoder".into()).child(self.ws_decoder_select.clone())))
                    .child(self.request_setting_row("跟随最新事件", "收到新事件时自动选中并滚动到最新一条；手动查看历史事件时临时暂停。",
                        Switch::new("").checked(self.ws_follow_latest).theme(self.theme)
                            .on_change({ let owner = cx.entity().downgrade(); move |follow, _, cx| { let _ = owner.update(cx, |this, cx| {
                                this.ws_follow_latest = follow;
                                this.ws_follow = follow;
                                if follow {
                                    this.ws_selected = this.ws_log.back().map(|event| event.id);
                                    this.ws_scroll.scroll_to_item(this.ws_log.len().saturating_sub(1));
                                    this.ws_select_sync(cx);
                                }
                                this.capture_draft(cx);
                                cx.notify();
                            }); } }).render().id("websocket-follow-setting").debug_selector(|| "websocket-follow-setting".into())))
                    .child(self.request_setting_row("事件显示格式", "选择事件体的默认显示格式，原始事件和解密结果共用此设置。",
                        div().id("websocket-body-format").debug_selector(|| "websocket-body-format".into()).child(self.ws_format_select.clone()))))
                .child(self.request_setting_row("连接超时", "建立连接时的最长等待时间。", div().text_size(px(12.)).child("10 秒")))
                .child(self.request_setting_row("请求超时", "包含连接和接收响应的总等待时间。", div().text_size(px(12.)).child("30 秒")))
                .child(self.request_setting_row("重定向", "保留服务器返回的原始 3xx 响应。", div().text_size(px(12.)).child("不自动跟随")))
                .child(self.request_setting_row("响应大小上限", "接收响应内容的最大大小。", div().text_size(px(12.)).child("16 MiB"))),
        };
        div()
            .id("configuration")
            .debug_selector(|| "request-config-scroll".into())
            .h(px(self.configuration_height(cx)))
            .flex_shrink_0()
            .when(
                self.config == 1
                    && self.authentication.read(cx).value().selected.as_deref() == Some("oauth2"),
                |view| view.overflow_hidden(),
            )
            .when(
                !(self.config == 1
                    && self.authentication.read(cx).value().selected.as_deref() == Some("oauth2")),
                |view| view.overflow_y_scroll(),
            )
            .px_5()
            .py_3()
            .child(view)
    }
}
