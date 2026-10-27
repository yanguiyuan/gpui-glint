//! Request configuration tabs, body editors, and panel sizing.

use super::*;

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
            ])
            .id("request-body-format")
            .selected(usize::from(self.body_format == BodyFormat::UrlEncoded))
            .theme(self.theme)
            .on_change(move |index, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.body_format = if index == 1 {
                        BodyFormat::UrlEncoded
                    } else {
                        BodyFormat::Raw
                    };
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
                .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("响应后写入所属集合在发送时环境下的变量，不修改全局变量；集合内同名变量覆盖。例：api_token → $.data.token。多个匹配保存为 JSON 数组；任一规则失败则不写入。"))
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
                .when(self.requests.get(self.active).is_some_and(|request| request.draft.method == "WS"), |settings| settings
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
