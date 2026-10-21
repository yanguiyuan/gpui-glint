use super::*;
use gpui_glint::components::DiffCard;

// Owned snapshots keep this window independent of tab edits and workspace changes.
struct VersionSnapshot {
    label: String,
    draft: Draft,
}

struct VersionDiff {
    name: String,
    versions: Vec<VersionSnapshot>,
    base: usize,
    target: usize,
    base_select: Entity<Select>,
    target_select: Entity<Select>,
    cards: Vec<(bool, Entity<DiffCard>)>,
    show_unchanged: bool,
    theme: ThemePreset,
    _subscriptions: Vec<Subscription>,
}

impl Workbench {
    pub(super) fn open_version_diff(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.requests.get(self.active) else {
            return;
        };
        if index >= request.versions.len() {
            return;
        }
        let name = request.name.clone();
        let mut versions = vec![VersionSnapshot {
            label: "当前请求（打开时快照）".into(),
            draft: self.snapshot(cx),
        }];
        versions.extend(request.versions.iter().map(|record| VersionSnapshot {
            label: if record.tag.is_empty() {
                record.label.clone()
            } else {
                format!("{} · {}", record.label, record.tag)
            },
            draft: record.draft.clone(),
        }));
        let title = format!("{name} · 版本对比 — Glint API");
        let theme = self.theme;
        let display_id = window.display(cx).map(|display| display.id());
        let bounds = Bounds::centered(display_id, size(px(1040.), px(800.)), cx);
        if let Err(error) = cx.open_window(
            WindowOptions {
                display_id,
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.), px(480.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |_, cx| cx.new(|cx| VersionDiff::new(name, versions, index + 1, theme, cx)),
        ) {
            self.notice = format!("无法打开版本对比窗口：{error}");
            cx.notify();
        }
    }
}

impl VersionDiff {
    fn new(
        name: String,
        versions: Vec<VersionSnapshot>,
        base: usize,
        theme: ThemePreset,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = || {
            versions
                .iter()
                .enumerate()
                .map(|(i, v)| SelectOption::new(i.to_string(), v.label.clone()))
                .collect::<Vec<_>>()
        };
        let base_select = cx.new(|cx| {
            Select::new(options(), cx)
                .selected(base.to_string())
                .searchable(true)
                .width(260.)
                .theme(theme)
        });
        let target_select = cx.new(|cx| {
            Select::new(options(), cx)
                .selected("0")
                .searchable(true)
                .width(260.)
                .theme(theme)
        });
        let mut subscriptions = Vec::new();
        for (is_base, select) in [(true, &base_select), (false, &target_select)] {
            subscriptions.push(
                cx.subscribe(select, move |this, _, event: &SelectEvent, cx| {
                    let SelectEvent::Changed(value) = event;
                    if let Ok(index) = value.parse::<usize>() {
                        this.select_version(is_base, index, cx);
                    }
                }),
            );
        }
        let mut this = Self {
            name,
            versions,
            base,
            target: 0,
            base_select,
            target_select,
            cards: vec![],
            show_unchanged: false,
            theme,
            _subscriptions: subscriptions,
        };
        this.rebuild(cx);
        this
    }

    fn select_version(&mut self, base: bool, index: usize, cx: &mut Context<Self>) {
        if index >= self.versions.len() {
            return;
        }
        if base {
            self.base = index;
        } else {
            self.target = index;
        }
        let select = if base {
            &self.base_select
        } else {
            &self.target_select
        };
        select.update(cx, |select, cx| {
            select.set_selected(Some(index.to_string().into()), cx)
        });
        self.rebuild(cx);
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let old = serde_json::to_value(&self.versions[self.base].draft).unwrap();
        let new = serde_json::to_value(&self.versions[self.target].draft).unwrap();
        let mut keys = old
            .as_object()
            .unwrap()
            .keys()
            .chain(new.as_object().unwrap().keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let order = [
            "method",
            "url",
            "params",
            "headers",
            "cookies",
            "auth",
            "token",
            "oauth",
            "body_format",
            "body",
            "form",
            "plugins",
            "extractors",
            "event_decoder",
            "skip_tls_verification",
            "websocket_follow_latest",
            "websocket_raw",
        ];
        keys.sort_by_key(|key| {
            order
                .iter()
                .position(|field| field == key)
                .unwrap_or(order.len())
        });
        self.cards = keys
            .into_iter()
            .map(|key| {
                let before = field_text(old.get(&key));
                let after = field_text(new.get(&key));
                let changed = before != after;
                let language = if matches!(key.as_str(), "body" | "url" | "token") {
                    "text"
                } else {
                    "json"
                };
                let card = cx.new(|cx| {
                    DiffCard::new(cx, language, before, after)
                        .name(field_label(&key))
                        .path(key)
                        .theme(self.theme)
                });
                (changed, card)
            })
            .collect();
        cx.notify();
    }
}

fn field_text(value: Option<&serde_json::Value>) -> String {
    match value {
        None => "（未设置）".into(),
        Some(serde_json::Value::String(value)) => value.clone(),
        Some(value) => {
            let mut value = value.clone();
            value.sort_all_objects();
            serde_json::to_string_pretty(&value).unwrap()
        }
    }
}

fn field_label(key: &str) -> String {
    match key {
        "method" => "请求方法",
        "url" => "请求地址",
        "params" => "查询参数",
        "headers" => "请求头",
        "cookies" => "Cookie",
        "auth" => "Bearer 鉴权开关",
        "token" => "Bearer Token",
        "oauth" => "OAuth 2.0",
        "body" => "请求体",
        "body_format" => "请求体格式",
        "form" => "表单参数",
        "plugins" => "前置动作",
        "extractors" => "后置提取",
        "event_decoder" => "事件解码器",
        "skip_tls_verification" => "跳过 TLS 校验",
        "websocket_follow_latest" => "WebSocket 跟随最新消息",
        "websocket_raw" => "WebSocket 原始消息",
        key => key,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn version_diff_opens_from_history_and_compares_independent_snapshots(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut h = Workbench::with_store(
                storage::Store::open(&directory.path().join("diff.sqlite3")).unwrap(),
                cx,
            );
            h.group_edit = Some(GroupEdit::CreateCollection);
            h.group_name.update(cx, |e, cx| e.set_value("API", cx));
            h.save_group(cx);
            h.add_request(cx);
            h.endpoint
                .update(cx, |e, cx| e.set_text("https://example.test/v2", cx));
            h.body
                .update(cx, |e, cx| e.set_content("first line\nsecond line", cx));
            h.save(cx);
            h.endpoint
                .update(cx, |e, cx| e.set_text("https://example.test/unsaved", cx));
            h
        });
        let bounds = window.debug_bounds("version-diff-1").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        window.run_until_parked();
        let children = window
            .windows()
            .into_iter()
            .filter_map(|w| w.downcast::<VersionDiff>())
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 1);
        let child = children[0];
        child
            .read_with(window, |diff, cx| {
                assert_eq!(diff.base, 2);
                assert_eq!(diff.target, 0);
                assert_eq!(diff.versions[0].draft.url, "https://example.test/unsaved");
                assert_eq!(diff.cards.iter().filter(|(changed, _)| *changed).count(), 2);
                assert!(
                    diff.cards
                        .iter()
                        .any(|(_, card)| card.read(cx).stats().0 == 2)
                );
            })
            .unwrap();
        host.update(window, |h, cx| {
            // Opening the child must not restore the clicked version in the main window.
            assert_eq!(h.current_versions[0], 0);
            assert_eq!(h.snapshot(cx).url, "https://example.test/unsaved");
            h.endpoint
                .update(cx, |e, cx| e.set_text("https://example.test/later", cx));
            h.requests[0].versions.clear();
        });
        child
            .update(window, |diff, _, cx| {
                assert_eq!(diff.versions[0].draft.url, "https://example.test/unsaved");
                diff.select_version(false, 1, cx);
                assert_eq!(diff.cards.iter().filter(|(changed, _)| *changed).count(), 2);
                diff.select_version(true, 1, cx);
                assert_eq!(diff.cards.iter().filter(|(changed, _)| *changed).count(), 0);
                assert_eq!(
                    diff.base_select.read(cx).value().map(|v| v.as_ref()),
                    Some("1")
                );
                diff.select_version(false, 999, cx);
                assert_eq!(diff.target, 1);
            })
            .unwrap();
    }

    #[test]
    fn diff_text_preserves_body_lines_and_orders_nested_json() {
        assert_eq!(field_text(Some(&serde_json::json!("a\nb"))), "a\nb");
        let left = serde_json::from_str(r#"{"z":false,"a":[null,""]}"#).unwrap();
        let right = serde_json::from_str(r#"{"a":[null,""],"z":false}"#).unwrap();
        assert_eq!(field_text(Some(&left)), field_text(Some(&right)));
        assert!(field_text(Some(&left)).contains("false"));
    }
}

impl Render for VersionDiff {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let changed = self.cards.iter().filter(|(changed, _)| *changed).count();
        let before = request_fingerprint::Fingerprint::new(&self.versions[self.base].draft);
        let after = request_fingerprint::Fingerprint::new(&self.versions[self.target].draft);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.surface))
            .text_color(rgb(p.text))
            .text_size(px(12.))
            .child(
                div()
                    .p_4()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{} · 版本对比", self.name)),
                    )
                    .child(
                        div()
                            .text_color(rgb(p.text_muted))
                            .child("只读 · 打开时快照 · 红色为删除，绿色为新增"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child("基准")
                            .child(self.base_select.clone())
                            .child("→")
                            .child(self.target_select.clone()),
                    )
                    .child(
                        div()
                            .font_family("Menlo")
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(format!(
                                "#{} → #{} · {} 项配置变化{}",
                                before.hash,
                                after.hash,
                                changed,
                                if before == after {
                                    " · 有效参数一致"
                                } else {
                                    ""
                                }
                            )),
                    )
                    .child(
                        Button::new(if self.show_unchanged {
                            "隐藏未修改项"
                        } else {
                            "显示未修改项"
                        })
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Mini)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "diff-toggle-unchanged".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_unchanged = !this.show_unchanged;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .id("version-diff-content")
                    .debug_selector(|| "version-diff-content".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .when(changed == 0 && !self.show_unchanged, |s| {
                        s.child(div().p_4().child("两个版本的完整配置一致，没有差异。"))
                    })
                    .children(
                        self.cards
                            .iter()
                            .filter(|(changed, _)| *changed || self.show_unchanged)
                            .map(|(_, card)| div().flex_shrink_0().child(card.clone())),
                    ),
            )
    }
}
