use super::*;
use gpui::{MouseButton, MouseDownEvent, MouseMoveEvent, WeakEntity};
use serde_json::Value;
use serde_json_path::JsonPath;
use std::ops::Range;

#[derive(Clone, Debug)]
pub(super) struct JsonNode {
    pub path: String,
    pub name: String,
    pub range: Range<usize>,
}

/// Index the actual source bytes, keeping escapes, whitespace and array indices intact.
pub(super) fn index(text: &str) -> Vec<JsonNode> {
    struct Parser<'a> {
        text: &'a str,
        offset: usize,
        nodes: Vec<JsonNode>,
    }
    impl Parser<'_> {
        fn whitespace(&mut self) {
            while self
                .text
                .as_bytes()
                .get(self.offset)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.offset += 1;
            }
        }
        fn string(&mut self) -> Option<String> {
            let start = self.offset;
            self.offset += 1;
            while let Some(&byte) = self.text.as_bytes().get(self.offset) {
                self.offset += 1;
                if byte == b'\\' {
                    self.offset += 1;
                } else if byte == b'"' {
                    return serde_json::from_str(&self.text[start..self.offset]).ok();
                }
            }
            None
        }
        fn value(
            &mut self,
            path: String,
            name: String,
            key_start: Option<usize>,
            depth: usize,
        ) -> Option<()> {
            if depth > 128 || self.nodes.len() >= 50_000 {
                return None;
            }
            self.whitespace();
            let start = key_start.unwrap_or(self.offset);
            match *self.text.as_bytes().get(self.offset)? {
                b'{' => {
                    self.offset += 1;
                    self.whitespace();
                    while *self.text.as_bytes().get(self.offset)? != b'}' {
                        let key_start = self.offset;
                        let key = self.string()?;
                        self.whitespace();
                        if *self.text.as_bytes().get(self.offset)? != b':' {
                            return None;
                        }
                        self.offset += 1;
                        let simple = key
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                        let child = if simple {
                            format!("{path}.{key}")
                        } else {
                            format!("{path}[{}]", serde_json::to_string(&key).ok()?)
                        };
                        self.value(child, key, Some(key_start), depth + 1)?;
                        self.whitespace();
                        if self.text.as_bytes().get(self.offset) == Some(&b',') {
                            self.offset += 1;
                            self.whitespace();
                        } else {
                            break;
                        }
                    }
                    if self.text.as_bytes().get(self.offset) != Some(&b'}') {
                        return None;
                    }
                    self.offset += 1;
                }
                b'[' => {
                    self.offset += 1;
                    self.whitespace();
                    let mut i = 0;
                    while *self.text.as_bytes().get(self.offset)? != b']' {
                        self.value(format!("{path}[{i}]"), name.clone(), None, depth + 1)?;
                        i += 1;
                        self.whitespace();
                        if self.text.as_bytes().get(self.offset) == Some(&b',') {
                            self.offset += 1;
                            self.whitespace();
                        } else {
                            break;
                        }
                    }
                    if self.text.as_bytes().get(self.offset) != Some(&b']') {
                        return None;
                    }
                    self.offset += 1;
                }
                b'"' => {
                    self.string()?;
                }
                _ => {
                    let value_start = self.offset;
                    while self
                        .text
                        .as_bytes()
                        .get(self.offset)
                        .is_some_and(|b| !b.is_ascii_whitespace() && !b",]}".contains(b))
                    {
                        self.offset += 1;
                    }
                    serde_json::from_str::<Value>(&self.text[value_start..self.offset]).ok()?;
                }
            }
            self.nodes.push(JsonNode {
                path,
                name,
                range: start..self.offset,
            });
            Some(())
        }
    }
    let mut parser = Parser {
        text,
        offset: 0,
        nodes: vec![],
    };
    if parser
        .value("$".into(), "response".into(), None, 0)
        .is_none()
    {
        return vec![];
    }
    parser.whitespace();
    if parser.offset != text.len() {
        return vec![];
    }
    parser.nodes
}

fn suggested_name(key: &str) -> String {
    let mut name: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() || name.chars().all(|c| c == '_') {
        name = "value".into();
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name = format!("var_{name}");
    }
    name
}

fn preview(json: Option<&Value>, path: &str, sensitive: bool) -> String {
    if path.trim().is_empty() {
        return "填写 JSONPath 或从响应选取".into();
    }
    let Some(json) = json else {
        return "发送请求获得 JSON 响应后可预览".into();
    };
    let Ok(path) = JsonPath::parse(path) else {
        return "JSONPath 格式无效".into();
    };
    let nodes = path.query(json).all();
    let Some(first) = nodes.first() else {
        return "0 个匹配".into();
    };
    let summary = if sensitive {
        "••••••".into()
    } else {
        match first {
            Value::Object(object) => format!("对象 · {} 个字段", object.len()),
            Value::Array(array) => format!("数组 · {} 项", array.len()),
            _ => {
                let raw = first.to_string();
                let short: String = raw.chars().take(64).collect();
                if short.len() < raw.len() {
                    format!("{short}…")
                } else {
                    short
                }
            }
        }
    };
    format!("{} 个匹配 · {summary}", nodes.len())
}

pub(super) struct ExtractorCell {
    owner: WeakEntity<Workbench>,
    row: usize,
    _subscription: Subscription,
}
impl ExtractorCell {
    pub fn new(owner: WeakEntity<Workbench>, row: usize, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&owner.upgrade().unwrap(), |_, _, cx| cx.notify());
        Self {
            owner,
            row,
            _subscription: subscription,
        }
    }
}
impl Render for ExtractorCell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            return div();
        };
        let host = owner.read(cx);
        let Some(row) = host.extractors.get(self.row) else {
            return div();
        };
        let path = row.value.read(cx).value();
        let name = row.key.read(cx).value();
        let sensitive = ["token", "secret", "password", "authorization", "cookie"]
            .iter()
            .any(|word| format!("{name} {path}").to_lowercase().contains(word))
            || host.environment_inputs[host.selected_environment(cx)]
                .variables
                .iter()
                .chain(
                    host.requests
                        .get(host.active)
                        .and_then(|r| {
                            host.collections
                                .iter()
                                .find(|c| c.collection == r.collection)
                        })
                        .and_then(|c| {
                            c.environments
                                .get(host.environment_name(host.selected_environment(cx)))
                        })
                        .into_iter()
                        .flat_map(|env| env.variables.iter()),
                )
                .any(|v| v.secret && v.name == name);
        let description = preview(host.response_json.as_ref(), path, sensitive);
        let theme = host.theme;
        let row_index = self.row;
        let owner = self.owner.clone();
        div()
            .group("extractor-path")
            .debug_selector(move || format!("extractor-jsonpath-{row_index}").into())
            .w_full()
            .min_w_0()
            .h(px(26.))
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().h_full().child(row.value.clone()))
            .child(
                div()
                    .max_w(px(140.))
                    .min_w_0()
                    .text_size(px(10.))
                    .text_color(rgb(theme.palette().text_muted))
                    .truncate()
                    .child(description),
            )
            .child(
                IconButton::new(Icon::Compass, "从响应选取 JSONPath")
                    .plain()
                    .theme(theme)
                    .render()
                    .size(px(22.))
                    .flex_shrink_0()
                    .debug_selector(move || format!("pick-jsonpath-{row_index}").into())
                    .on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |host, cx| {
                            cx.stop_propagation();
                            host.start_json_pick(row_index, window, cx)
                        });
                    }),
            )
    }
}

impl Workbench {
    pub(super) fn extractor_input(&mut self, pair: &Pair, cx: &mut Context<Self>) -> PairInput {
        let input = self.pair(pair, cx);
        input
            .key
            .update(cx, |entry, cx| entry.set_placeholder("变量名", cx));
        input.value.update(cx, |entry, cx| {
            entry.set_placeholder("JSONPath，例如 $.data.token", cx);
            entry.set_reference_resolver(None, cx);
        });
        input
    }
    pub(super) fn cancel_json_pick(&mut self, cx: &mut Context<Self>) {
        self.json_pick = None;
        self.json_hover = None;
        let folding = self.pretty && self.response_json.is_some();
        self.response.update(cx, |editor, cx| {
            editor.set_source_highlight(None, cx);
            editor.set_json_folding(folding, cx);
        });
    }
    fn start_json_pick(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        if row >= self.extractors.len() {
            return;
        }
        if self.response_json.is_none() {
            self.request_error_toast(
                "无法选取 JSONPath",
                self.environment_name(self.selected_environment(cx))
                    .to_owned()
                    .as_str(),
                "请先发送请求获得 JSON 响应",
                cx,
            );
            return;
        }
        self.response_tab = 0;
        self.pretty = true;
        self.show_response_search = false;
        self.update_response(cx);
        if self.json_nodes.is_empty() {
            let environment = self
                .environment_name(self.selected_environment(cx))
                .to_owned();
            self.request_error_toast(
                "无法选取 JSONPath",
                &environment,
                "响应字段过多，暂不支持交互选取，请手动填写 JSONPath",
                cx,
            );
            return;
        }
        self.response
            .update(cx, |editor, cx| editor.set_json_folding(false, cx));
        self.json_pick = Some(row);
        window.focus(&self.focus, cx);
        cx.notify();
    }
    fn json_node_at(&self, position: gpui::Point<gpui::Pixels>, cx: &App) -> Option<usize> {
        let offset = self.response.read(cx).source_offset_at(position)?;
        self.json_nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.range.contains(&offset))
            .min_by_key(|(_, node)| node.range.len())
            .map(|(index, _)| index)
    }
    fn assign_json_node(
        &mut self,
        node: &JsonNode,
        target: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.requests.is_empty() {
            return;
        }
        let index = target
            .filter(|index| *index < self.extractors.len())
            .or_else(|| {
                self.extractors
                    .iter()
                    .position(|row| row.value.read(cx).value() == node.path)
            })
            .or_else(|| {
                self.extractors.iter().position(|row| {
                    row.key.read(cx).value().trim().is_empty()
                        && row.value.read(cx).value().trim().is_empty()
                })
            })
            .unwrap_or_else(|| {
                let input = self.extractor_input(&Pair::new("", "", ""), cx);
                self.extractors.push(input);
                self.extractors.len() - 1
            });
        let row = &self.extractors[index];
        if row.key.read(cx).value().trim().is_empty() {
            row.key.update(cx, |entry, cx| {
                entry.set_value(suggested_name(&node.name), cx)
            });
        }
        row.value
            .update(cx, |entry, cx| entry.set_value(node.path.clone(), cx));
        let focus = row.key.focus_handle(cx);
        self.cancel_json_pick(cx);
        self.json_menu = None;
        self.config = 6;
        self.storage_dirty = true;
        self.sync_pair_tables(cx);
        window.focus(&focus, cx);
        cx.notify();
    }
    pub(super) fn json_response_view(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let mut view = div()
            .id("json-response-source")
            .debug_selector(|| "json-response-source".into())
            .size_full()
            .relative()
            .child(self.response.clone())
            .on_mouse_move(cx.listener(|host, event: &MouseMoveEvent, _, cx| {
                if host.json_pick.is_none() {
                    return;
                }
                let node = host.json_node_at(event.position, cx);
                if host.json_hover != node {
                    host.json_hover = node;
                    let range = node.map(|index| host.json_nodes[index].range.clone());
                    host.response
                        .update(cx, |editor, cx| editor.set_source_highlight(range, cx));
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|host, event: &MouseDownEvent, window, cx| {
                    if let Some(row) = host.json_pick
                        && let Some(index) = host.json_node_at(event.position, cx)
                    {
                        let node = host.json_nodes[index].clone();
                        host.assign_json_node(&node, Some(row), window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|host, event: &MouseDownEvent, _, cx| {
                    if let Some(index) = host.json_node_at(event.position, cx) {
                        host.json_menu = Some((host.json_nodes[index].clone(), event.position));
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            );
        if let Some((node, position)) = &self.json_menu {
            let node = node.clone();
            let p = self.theme.palette();
            let menu = div()
                .id("json-path-context-menu")
                .w(px(260.))
                .p_2()
                .rounded(px(7.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.surface))
                .shadow_lg()
                .on_mouse_down_out(cx.listener(|host, _, _, cx| {
                    host.json_menu = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .truncate()
                        .child(node.path.clone()),
                )
                .child(
                    Button::new("添加为提取规则")
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "add-json-extractor".into())
                        .on_click(cx.listener(move |host, _, window, cx| {
                            cx.stop_propagation();
                            host.assign_json_node(&node, None, window, cx);
                        })),
                );
            view = view.child(
                gpui::deferred(
                    gpui::anchored()
                        .position(*position)
                        .snap_to_window_with_margin(px(8.))
                        .child(menu),
                )
                .with_priority(40),
            );
        }
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_preserve_unicode_arrays_and_escaped_property_names() {
        let value = json!({"user.name": {"a'b\\c\"": [ {"id": 17}, "中文" ]}, "normal_key": true});
        for text in [
            serde_json::to_string(&value).unwrap(),
            serde_json::to_string_pretty(&value).unwrap(),
        ] {
            let nodes = index(&text);
            assert_eq!(nodes.len(), 7);
            for node in &nodes {
                assert!(
                    text.is_char_boundary(node.range.start)
                        && text.is_char_boundary(node.range.end)
                );
                let result = JsonPath::parse(&node.path)
                    .unwrap()
                    .query(&value)
                    .exactly_one()
                    .unwrap();
                if node.name == "id" {
                    assert_eq!(result, &json!(17));
                    assert!(node.path.ends_with("[0].id"));
                }
                if node.name == "normal_key" {
                    assert_eq!(node.path, "$.normal_key");
                }
            }
        }
        assert_eq!(suggested_name("user.name"), "user_name");
        assert_eq!(suggested_name("123-id"), "var_123_id");
    }

    #[test]
    fn previews_report_counts_errors_and_mask_sensitive_values() {
        let value = json!({"access_token":"private-credential", "items":[{"id":1},{"id":2}]});
        assert_eq!(
            preview(Some(&value), "$.access_token", true),
            "1 个匹配 · ••••••"
        );
        assert!(preview(Some(&value), "$.items[*].id", false).starts_with("2 个匹配"));
        assert_eq!(preview(Some(&value), "$.missing", false), "0 个匹配");
        assert_eq!(preview(Some(&value), "$[", false), "JSONPath 格式无效");
        assert!(preview(None, "$.token", false).contains("JSON 响应"));
    }

    #[gpui::test]
    fn native_response_pick_context_menu_preview_and_persistence(cx: &mut gpui::TestAppContext) {
        struct Panel(Entity<Workbench>);
        impl Render for Panel {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let host = self.0.clone();
                div()
                    .w(px(900.))
                    .h(px(690.))
                    .flex()
                    .flex_col()
                    .child(host.update(cx, |host, cx| host.configuration(cx)))
                    .child(host.update(cx, |host, cx| host.response_panel(cx)))
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let host = cx.new(|cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            let value = json!({"access_token":"private-token", "expires_in":7199});
            host.response_data = Some(HttpResponse {
                status: 200,
                headers: vec![],
                text: value.to_string(),
                bytes: 64,
                json: Some(value.clone()),
            });
            host.response_json = Some(value);
            host.update_response(cx);
            host.config = 6;
            let input = host.extractor_input(&Pair::new("", "", ""), cx);
            host.extractors.push(input);
            host.sync_pair_tables(cx);
            host
        });
        let (_, window) = cx.add_window_view(|_, _| Panel(host.clone()));
        let cell = window.debug_bounds("extractor-jsonpath-0").unwrap();
        window.simulate_event(MouseMoveEvent {
            position: cell.center(),
            ..Default::default()
        });
        let pick = window.debug_bounds("pick-jsonpath-0").unwrap();
        window.simulate_click(pick.center(), gpui::Modifiers::default());
        host.update(window, |host, _| assert_eq!(host.json_pick, Some(0)));
        let source = window.debug_bounds("json-response-source").unwrap();
        let field = point(source.left() + px(110.), source.top() + px(31.));
        window.simulate_event(MouseMoveEvent {
            position: field,
            ..Default::default()
        });
        host.update(window, |host, _| {
            assert!(
                host.json_hover.is_some(),
                "pointer must hit a painted source node"
            )
        });
        window.simulate_click(field, gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert!(host.json_pick.is_none());
            assert_eq!(host.extractors[0].value.read(cx).value(), "$.access_token");
            assert_eq!(host.extractors[0].key.read(cx).value(), "access_token");
            host.extractors[0]
                .key
                .update(cx, |entry, cx| entry.set_value("custom_token", cx));
        });
        // Picking again preserves the custom variable name; cancellation never changes the rule.
        let cell = window.debug_bounds("extractor-jsonpath-0").unwrap();
        window.simulate_event(MouseMoveEvent {
            position: cell.center(),
            ..Default::default()
        });
        let pick = window.debug_bounds("pick-jsonpath-0").unwrap();
        window.simulate_click(pick.center(), gpui::Modifiers::default());
        let cancel = window.debug_bounds("cancel-json-pick").unwrap();
        window.simulate_click(cancel.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert!(host.json_pick.is_none());
            assert_eq!(host.extractors[0].key.read(cx).value(), "custom_token");
        });
        window.simulate_event(MouseMoveEvent {
            position: cell.center(),
            ..Default::default()
        });
        let pick = window.debug_bounds("pick-jsonpath-0").unwrap();
        window.simulate_click(pick.center(), gpui::Modifiers::default());
        window.simulate_click(field, gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.extractors[0].key.read(cx).value(), "custom_token")
        });
        // Right-click on the second field, then add a rule through the actual menu.
        let second = point(source.left() + px(110.), source.top() + px(52.));
        window.simulate_event(MouseDownEvent {
            position: second,
            button: MouseButton::Right,
            ..Default::default()
        });
        let add = window.debug_bounds("add-json-extractor").unwrap();
        window.simulate_click(add.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.extractors.len(), 2);
            assert_eq!(host.extractors[0].key.read(cx).value(), "custom_token");
            assert_eq!(host.extractors[1].value.read(cx).value(), "$.expires_in");
            assert_eq!(host.extractors[1].key.read(cx).value(), "expires_in");
            host.persist(cx);
        });
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert_eq!(saved.requests[0].draft.extractors.len(), 2);
        assert_eq!(saved.requests[0].draft.extractors[0].key, "custom_token");
        assert!(
            saved.variables[0].variables.is_empty(),
            "picking must only configure a rule"
        );
    }
}
