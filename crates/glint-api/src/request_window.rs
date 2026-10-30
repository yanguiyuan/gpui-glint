//! A request promoted to its own editor/executor. The main tab and every
//! detached window render the SAME entity; sockets, inputs and responses are
//! never duplicated. The workbench remains the durable workspace owner.
use super::*;

pub(super) struct Binding {
    pub namespace: String,
    pub detached: bool,
    pub storage_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::time::Duration;

    fn wait_for(
        window: &mut gpui::VisualTestContext,
        mut condition: impl FnMut(&mut gpui::VisualTestContext) -> bool,
    ) {
        for _ in 0..500 {
            window.run_until_parked();
            if condition(window) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("shared request did not reach expected state");
    }

    fn setup(path: &std::path::Path, cx: &mut Context<Workbench>) -> Workbench {
        let mut host = Workbench::with_store(storage::Store::open(path).unwrap(), cx);
        host.group_edit = Some(GroupEdit::CreateCollection);
        host.group_name.update(cx, |e, cx| e.set_value("API", cx));
        host.save_group(cx);
        host.add_request(cx);
        host
    }

    fn open(
        host: &Entity<Workbench>,
        index: usize,
        window: &mut gpui::VisualTestContext,
    ) -> Entity<Workbench> {
        window.update(|window, cx| {
            host.update(cx, |host, cx| host.open_request_window(index, window, cx))
        });
        window.run_until_parked();
        host.read_with(window, |host, _| host.shared_request_editor(index).unwrap())
    }

    fn child_for(
        editor: &Entity<Workbench>,
        window: &gpui::VisualTestContext,
    ) -> gpui::WindowHandle<RequestWindow> {
        window
            .windows()
            .into_iter()
            .filter_map(|w| w.downcast::<RequestWindow>())
            .find(|w| {
                w.read_with(window, |view, _| {
                    view.editor.entity_id() == editor.entity_id()
                })
                .unwrap()
            })
            .unwrap()
    }

    #[gpui::test]
    fn rapid_endpoint_edits_paint_before_autosave_and_flush_latest_text(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(bind_text_entry_keys);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rapid-input.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| setup(&path, cx));
        host.update(window, |host, cx| {
            host.endpoint
                .update(cx, |entry, cx| entry.set_text("https://example.test/", cx));
            host.persist(cx);
        });
        let writes = host.read_with(window, |host, _| host.storage.serializations);
        let endpoint = window.debug_bounds("request-endpoint").unwrap();
        window.simulate_click(endpoint.center(), gpui::Modifiers::default());
        window.simulate_keystrokes("end");
        let mut expected = "https://example.test/".to_owned();
        for ch in "abcdefgh{{{{ssss1234567890".chars() {
            window.simulate_input(&ch.to_string());
            expected.push(ch);
            host.read_with(window, |host, cx| {
                assert_eq!(
                    host.endpoint.read(cx).value().text.as_ref(),
                    expected.as_str()
                );
                assert_eq!(
                    host.storage.serializations, writes,
                    "typing must not write SQLite"
                );
            });
        }
        for _ in 0..10 {
            window.simulate_keystrokes("backspace");
            expected.pop();
            host.read_with(window, |host, cx| {
                assert_eq!(
                    host.endpoint.read(cx).value().text.as_ref(),
                    expected.as_str()
                );
                assert_eq!(
                    host.storage.serializations, writes,
                    "backspace must not write SQLite"
                );
            });
        }
        window.executor().advance_clock(Duration::from_millis(200));
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert_eq!(host.storage.serializations, writes + 1)
        });
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert_eq!(saved.requests[0].draft.url, expected);
        // Explicit persistence flushes a pending edit without waiting for the idle timer.
        host.update(window, |host, cx| {
            host.endpoint.update(cx, |entry, cx| {
                entry.set_text("https://example.test/final", cx)
            });
            host.persist(cx);
        });
        assert_eq!(
            storage::Store::open(&path)
                .unwrap()
                .load()
                .unwrap()
                .requests[0]
                .draft
                .url,
            "https://example.test/final"
        );
    }

    #[gpui::test]
    fn detached_request_triggers_refresh_and_persists_expiration(cx: &mut gpui::TestAppContext) {
        cx.background_executor.allow_parking();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut requests = vec![];
            for body in [
                r#"{"access_token":"fresh","expires_in":7199}"#,
                r#"{"ok":true}"#,
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = vec![];
                let mut buf = [0; 4096];
                while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    let count = socket.read(&mut buf).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buf[..count]);
                }
                requests.push(String::from_utf8(bytes).unwrap());
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("triggers.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = setup(&path, cx);
            host.requests[0].id = "f29d4365-00db-4a66-b159-f8ae2d5eae64".into();
            let mut extractor = Pair::new("access_token", "$.access_token", "");
            extractor.expires_in = "$.expires_in".into();
            host.load(
                &Draft {
                    url: format!("{url}/login"),
                    extractors: vec![extractor],
                    ..Default::default()
                },
                cx,
            );
            host.capture_draft(cx);
            host.add_request(cx);
            host.load(
                &Draft {
                    url: format!("{url}/main"),
                    auth: true,
                    token: "{{access_token}}".into(),
                    triggers: vec![model::Trigger {
                        enabled: true,
                        condition: model::TriggerCondition::VariableExpired {
                            variable: "access_token".into(),
                        },
                        action: model::TriggerAction::RunRequest {
                            request_id: "f29d4365-00db-4a66-b159-f8ae2d5eae64".into(),
                        },
                    }],
                    ..Default::default()
                },
                cx,
            );
            host.capture_draft(cx);
            host
        });
        let editor = open(&host, 1, window);
        editor.update(window, |editor, cx| {
            assert_eq!(editor.snapshot(cx).triggers.len(), 1);
            assert_eq!(editor.trigger_requests.as_ref().unwrap().len(), 2);
            editor.send(cx);
        });
        wait_for(window, |window| {
            editor.read_with(window, |editor, _| editor.sending.is_none())
        });
        editor.read_with(window, |editor, _| {
            assert_eq!(editor.status, Some(200), "{:?}", editor.error)
        });
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /login "));
        assert!(requests[1].starts_with("GET /main "));
        assert!(
            requests[1]
                .to_lowercase()
                .contains("authorization: bearer fresh")
        );
        host.update(window, |host, cx| host.persist(cx));
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.read_with(window, |host, _| {
            assert_eq!(host.requests[1].draft.triggers.len(), 1);
            let token = host.collections[0]
                .environments
                .values()
                .flat_map(|e| e.variables.iter())
                .find(|v| v.name == "access_token")
                .unwrap();
            assert_eq!(token.value, "fresh");
            assert!(token.expires_at.unwrap() > chrono::Utc::now().timestamp());
        });
    }

    #[gpui::test]
    fn request_windows_share_editors_inputs_versions_and_mcp_updates(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shared.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| setup(&path, cx));
        let editor = open(&host, 0, window);
        editor.update(window, |editor, cx| {
            editor.config = 3;
            cx.notify();
        });
        let child = child_for(&editor, window);
        assert_eq!(open(&host, 0, window).entity_id(), editor.entity_id());
        let body = editor.read_with(window, |editor, _| editor.body.clone());
        window.update(|window, cx| window.focus(&body.focus_handle(cx), cx));
        window.simulate_input("from main");
        window.run_until_parked();
        assert!(host.read_with(window, |host, cx| {
            host.snapshot(cx).body.contains("from main")
        }));
        window.update(|window, cx| window.dispatch_action(Box::new(SearchCommands), cx));
        window.run_until_parked();
        host.read_with(window, |host, cx| assert!(host.commands.read(cx).is_open()));
        editor.read_with(window, |editor, cx| {
            assert!(!editor.commands.read(cx).is_open())
        });
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.commands
                    .update(cx, |commands, cx| commands.close(window, cx));
            })
        });
        let mut child_context = gpui::VisualTestContext::from_window(child.into(), window);
        child_context.update(|window, cx| window.focus(&body.focus_handle(cx), cx));
        child_context.simulate_input(" from child");
        child_context.run_until_parked();
        assert!(host.read_with(window, |host, cx| {
            host.snapshot(cx).body.contains("from child")
        }));
        let id = host.read_with(window, |host, _| host.requests[0].id.clone());
        host.update(window, |host, cx| {
            let result = host.mcp_call("glint_request_update", serde_json::json!({"request_id":id,"draft":{"url":"https://example.test/shared"}}), cx).unwrap();
            assert_eq!(result["version_created"], true);
            assert_eq!(host.snapshot(cx).url, "https://example.test/shared");
        });
        window.run_until_parked();
        editor.read_with(window, |editor, cx| {
            assert_eq!(
                editor.endpoint.read(cx).value().text.as_ref(),
                "https://example.test/shared"
            );
            assert_eq!(
                editor.requests[0].versions[0].source,
                model::VersionSource::Mcp
            );
        });
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert!(saved.requests[0].draft.body.contains("from child"));
        assert_eq!(saved.requests[0].draft.url, "https://example.test/shared");
        host.update(window, |host, cx| host.delete_request(0, cx));
        window.run_until_parked();
        assert!(editor.read_with(window, |editor, _| {
            editor.request_binding.as_ref().unwrap().detached
        }));
    }

    #[gpui::test]
    fn live_ws_window_receives_events_from_other_request_windows_without_reconnecting(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.background_executor.allow_parking();
        let ws_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let ws_url = format!("ws://{}/events", ws_listener.local_addr().unwrap());
        let http_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let http_url = format!("http://{}/trigger", http_listener.local_addr().unwrap());
        let (trigger, events) = std::sync::mpsc::channel::<usize>();
        let ws_server = std::thread::spawn(move || {
            use tokio_tungstenite::tungstenite::{Message, accept};
            let (socket, _) = ws_listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut socket = accept(socket).unwrap();
            socket.send(Message::Text("ready".into())).unwrap();
            for _ in 0..2 {
                let event = events.recv_timeout(Duration::from_secs(10)).unwrap();
                socket
                    .send(Message::Text(
                        format!("{{\"event\":\"trigger-{event}\"}}").into(),
                    ))
                    .unwrap();
            }
            while let Ok(message) = socket.read() {
                if matches!(message, Message::Close(_)) {
                    let _ = socket.flush();
                    break;
                }
            }
        });
        let http_server = std::thread::spawn(move || {
            for event in 1..=2 {
                let (mut socket, _) = http_listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut buffer = [0; 4096];
                let _ = socket.read(&mut buffer).unwrap();
                trigger.send(event).unwrap();
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .unwrap();
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut h = setup(&directory.path().join("live.sqlite3"), cx);
            h.load(
                &Draft {
                    method: "WS".into(),
                    url: ws_url,
                    ..Draft::default()
                },
                cx,
            );
            h.connect_websocket(cx);
            h
        });
        wait_for(window, |window| {
            host.read_with(window, |h, _| h.ws_connected)
        });
        let ws = open(&host, 0, window);
        let ws_window = child_for(&ws, window);
        host.update(window, |h, cx| {
            h.add_request(cx);
            h.load(
                &Draft {
                    url: http_url,
                    ..Draft::default()
                },
                cx,
            );
            h.capture_draft(cx);
        });
        let http = open(&host, 1, window);
        let http_window = child_for(&http, window);
        host.update(window, |h, cx| {
            h.add_request(cx);
            h.body
                .update(cx, |body, cx| body.set_content("main stays here", cx));
        });
        let mut http_context = gpui::VisualTestContext::from_window(http_window.into(), window);
        for event in 1..=2 {
            let button = http_context.debug_bounds("request-send").unwrap();
            http_context.simulate_click(button.center(), gpui::Modifiers::default());
            wait_for(window, |window| {
                http.read_with(window, |h, _| {
                    h.sending.is_none() && h.response_data.as_ref().is_some_and(|r| r.status == 200)
                }) && ws.read_with(window, |h, _| {
                    h.ws_log
                        .iter()
                        .any(|message| message.log_text().contains(&format!("trigger-{event}")))
                })
            });
            host.read_with(window, |h, cx| {
                assert_eq!(h.active, 2);
                assert_eq!(h.snapshot(cx).body, "main stays here");
                assert_eq!(h.responses[1].as_ref().unwrap().text, "ok");
                assert!(h.response_data.is_none());
            });
            if event == 1 {
                ws_window
                    .update(window, |_, window, _| window.remove_window())
                    .unwrap();
                window.run_until_parked();
                assert!(ws.read_with(window, |h, _| h.ws_connected));
                assert_eq!(open(&host, 0, window).entity_id(), ws.entity_id());
            }
        }
        ws.read_with(window, |h, _| {
            assert_eq!(
                h.ws_log
                    .iter()
                    .filter(|event| event.was_connected())
                    .count(),
                1
            );
            assert_eq!(h.requests[0].history.len(), 1);
        });
        host.update(window, |h, cx| {
            let id = h.requests[0].id.clone();
            h.mcp_call(
                "glint_request_cancel",
                serde_json::json!({"request_id":id}),
                cx,
            )
            .unwrap();
        });
        wait_for(window, |window| {
            ws.read_with(window, |h, _| !h.ws_active && !h.ws_connected)
        });
        http_server.join().unwrap();
        ws_server.join().unwrap();
    }

    #[gpui::test]
    fn opening_an_inflight_http_request_preserves_its_executor_and_response_target(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.background_executor.allow_parking();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/inflight", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buffer = [0; 4096];
            socket.read(&mut buffer).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\npromoted",
                )
                .unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut h = setup(&directory.path().join("inflight.sqlite3"), cx);
            h.load(
                &Draft {
                    url,
                    ..Draft::default()
                },
                cx,
            );
            h
        });
        window.update(|window, cx| {
            host.update(cx, |h, cx| {
                h.send(cx);
                assert!(h.sending.is_some());
                h.open_request_window(0, window, cx);
                h.add_request(cx);
            })
        });
        let editor = host.read_with(window, |h, _| h.shared_request_editor(0).unwrap());
        wait_for(window, |window| {
            editor.read_with(window, |h, _| {
                h.sending.is_none() && h.response_data.is_some()
            })
        });
        host.read_with(window, |h, _| {
            assert_eq!(h.active, 1);
            assert!(h.response_data.is_none());
            assert_eq!(h.responses[0].as_ref().unwrap().text, "promoted");
            assert_eq!(h.requests[0].history.len(), 1);
        });
        server.join().unwrap();
    }
}

pub(super) struct SharedRequest {
    pub editor: Entity<Workbench>,
    pub running: bool,
    pub websocket: bool,
    pub connected: bool,
    signature: String,
    context: String,
    selected_environment: usize,
    collections: Vec<RequestGroups>,
    _subscription: Subscription,
}

pub(super) struct RequestWindow {
    editor: Entity<Workbench>,
    _subscription: Subscription,
}
impl Render for RequestWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.read(cx);
        let theme = editor.theme;
        let p = theme.palette();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.surface))
            .text_color(rgb(p.text))
            .text_size(px(12.))
            .child(
                div()
                    .h(px(40.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child("共享请求 · 修改、发送与响应实时同步")
                    .child(div().flex_1())
                    .child(editor.environment.clone())
                    .child(
                        Button::new("版本 / 历史")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.editor.update(cx, |editor, cx| {
                                    editor.show_activity = !editor.show_activity;
                                    cx.notify();
                                });
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.editor.clone()),
            )
    }
}

impl Workbench {
    pub(super) fn shared_request_editor(&self, index: usize) -> Option<Entity<Self>> {
        self.requests
            .get(index)
            .and_then(|request| self.request_editors.get(&request.id))
            .map(|shared| shared.editor.clone())
    }

    pub(super) fn with_request_executor<R>(
        &mut self,
        request_id: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Context<Self>) -> R,
    ) -> Option<R> {
        if let Some(shared) = self.request_editors.get(request_id) {
            Some(shared.editor.clone().update(cx, |editor, cx| f(editor, cx)))
        } else if self
            .requests
            .get(self.active)
            .is_some_and(|request| request.id == request_id)
        {
            Some(f(self, cx))
        } else {
            None
        }
    }

    pub(super) fn detach_request_window(&mut self, cx: &mut Context<Self>) {
        if let Some(binding) = &mut self.request_binding {
            binding.detached = true;
        }
        if let Some(commands) = self.ws_commands.take() {
            let _ = commands.try_send(websocket::Command::Close);
        }
        self.sending = None;
        self.ws_decode_task = None;
        self.ws_connected = false;
        self.ws_active = false;
        cx.notify();
    }

    pub(super) fn open_request_window(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab_menu = None;
        let Some(request) = self.requests.get(index) else {
            return;
        };
        let name = request.name.clone();
        let editor = if self.request_binding.is_some() {
            cx.entity()
        } else if let Some(editor) = self.shared_request_editor(index) {
            editor
        } else {
            self.capture_draft(cx);
            if index == self.active {
                self.html_preview = false;
            }
            let data = storage::Workspace {
                requests: vec![self.requests[index].clone()],
                collections: self.collections.clone(),
                selected_environment: self.selected_environment(cx),
                environments: self.environment_names.clone(),
                variables: self.environment_inputs.clone(),
                proxies: self.proxies.clone(),
                traces: vec![self.traces[index].clone()],
                responses: vec![if index == self.active {
                    self.response_data.clone()
                } else {
                    self.responses[index].clone()
                }],
                previews: vec![self.sent_requests[index].clone()],
                curls: vec![self.sent_curls[index].clone()],
                update_source: self.update_source.clone(),
            };
            let storage = match storage::Store::memory() {
                Ok(storage) => storage,
                Err(error) => {
                    self.notice = format!("无法创建请求窗口：{error}");
                    cx.notify();
                    return;
                }
            };
            let id = self.requests[index].id.clone();
            let namespace = self.oauth_namespace.clone();
            let owner = cx.entity();
            let theme = self.theme;
            let config = if index == self.active {
                self.config
            } else if matches!(
                self.requests[index].draft.method.as_str(),
                "POST" | "PUT" | "PATCH" | "WS"
            ) {
                3
            } else {
                0
            };
            let disabled_plugins = self.disabled_plugins.clone();
            let plugin_catalog = self.plugin_catalog.clone();
            let mut ws = self.ws_background.remove(&id);
            if index == self.active
                && (self.ws_active || self.ws_commands.is_some() || !self.ws_log.is_empty())
            {
                let http = if !self.ws_active {
                    self.sending.take()
                } else {
                    None
                };
                let mut state = background_tasks_ui::WsSessionState::empty(theme, cx);
                self.swap_ws_state(&mut state);
                self.sending = http;
                ws = Some(state);
            }
            let http = if index == self.active && ws.as_ref().is_none_or(|ws| !ws.active) {
                self.sending.take()
            } else {
                None
            };
            let trace_started = if index == self.active {
                self.trace_started.take()
            } else {
                None
            };
            let error = if index == self.active {
                self.error.clone()
            } else {
                None
            };
            let selected_version = self.current_versions[index];
            let storage_error = self.storage_error.clone();
            let editor = cx.new(|cx| {
                let mut editor = Self::with_workspace(storage, data, cx);
                editor.trigger_requests = Some(self.requests.clone());
                editor.sync_pair_tables(cx);
                editor.request_binding = Some(Binding {
                    namespace: namespace.clone(),
                    detached: false,
                    storage_error,
                });
                editor.sync_commands(cx);
                editor.oauth_namespace = namespace;
                editor.config = config;
                editor.disabled_plugins = disabled_plugins;
                editor.plugin_catalog = plugin_catalog;
                editor.current_versions[0] = selected_version;
                editor.trace_started = trace_started;
                editor.error = error;
                editor.sending = http;
                editor.set_theme(theme, cx);
                if let Some(mut ws) = ws {
                    editor.swap_ws_state(&mut ws);
                    editor.ws_sync_detail(cx);
                }
                editor
                    .subscriptions
                    .push(cx.observe(&owner, |editor, owner, cx| {
                        let error = owner.read(cx).storage_error.clone();
                        if editor.request_binding.as_ref().is_some_and(|binding| {
                            binding.namespace != owner.read(cx).oauth_namespace
                        }) {
                            editor.detach_request_window(cx);
                        } else if let Some(binding) = &mut editor.request_binding {
                            if binding.storage_error != error {
                                binding.storage_error = error;
                                cx.notify();
                            }
                        }
                    }));
                editor.subscriptions.push(
                    cx.observe_release(&owner, |editor, _, cx| editor.detach_request_window(cx)),
                );
                editor
            });
            let observed_id = id.clone();
            let subscription = cx.observe(&editor, move |owner, editor, cx| {
                // Publish execution transitions immediately. Ordinary keystrokes must
                // not clone response bodies/history or serialize the workspace.
                let current = editor.read(cx);
                let running =
                    current.sending.is_some() || current.ws_active || current.ws_connected;
                let transition = owner
                    .request_editors
                    .get(&observed_id)
                    .is_some_and(|shared| {
                        shared.running != running || shared.connected != current.ws_connected
                    });
                if transition && owner.sync_request_editor(&observed_id, cx) {
                    cx.notify();
                }
                owner.schedule_persist(cx);
            });
            self.request_editors.insert(
                id.clone(),
                SharedRequest {
                    editor: editor.clone(),
                    running: false,
                    websocket: false,
                    connected: false,
                    signature: String::new(),
                    context: String::new(),
                    selected_environment: self.selected_environment(cx),
                    collections: self.collections.clone(),
                    _subscription: subscription,
                },
            );
            self.sync_request_editor(&id, cx);
            editor
        };
        let display_id = window.display(cx).map(|display| display.id());
        let bounds = Bounds::centered(display_id, size(px(1100.), px(850.)), cx);
        if let Err(error) = cx.open_window(
            WindowOptions {
                display_id,
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.), px(560.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(format!("{name} · 共享请求 — Glint API").into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|cx| {
                    let subscription = cx.observe(&editor, |_, _, cx| cx.notify());
                    RequestWindow {
                        editor,
                        _subscription: subscription,
                    }
                })
            },
        ) {
            self.notice = format!("无法打开请求窗口：{error}");
        }
        cx.notify();
    }

    pub(super) fn sync_request_editor(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.requests.iter().position(|r| r.id == id) else {
            return false;
        };
        let Some(shared) = self.request_editors.get_mut(id) else {
            return false;
        };
        let editor = shared.editor.read(cx);
        if editor
            .request_binding
            .as_ref()
            .is_some_and(|binding| binding.detached)
        {
            return false;
        }
        let mut request = editor.requests[0].clone();
        request.draft = editor.snapshot(cx);
        let selected_environment = editor.selected_environment(cx);
        let signature = serde_json::to_string(&(
            &request,
            &editor.responses,
            &editor.traces,
            &editor.sent_requests,
            &editor.sent_curls,
            editor.current_versions[0],
            editor.sending.is_some(),
            editor.ws_active,
            editor.ws_connected,
            &editor.error,
            editor
                .collections
                .iter()
                .map(|c| (&c.id, &c.environments))
                .collect::<Vec<_>>(),
            selected_environment,
        ))
        .unwrap();
        if signature == shared.signature {
            return false;
        }
        // A request editor can extract collection variables. Merge only changes
        // since its last publication, preserving other requests' concurrent edits.
        for collection in &editor.collections {
            let baseline = shared.collections.iter().find(|c| c.id == collection.id);
            if let Some(target) = self.collections.iter_mut().find(|c| c.id == collection.id) {
                for (environment, variables) in &collection.environments {
                    let old = baseline.and_then(|c| c.environments.get(environment));
                    let target = target.environments.entry(environment.clone()).or_default();
                    for variable in &variables.variables {
                        if old.and_then(|v| v.variables.iter().find(|v| v.name == variable.name))
                            != Some(variable)
                        {
                            let target_id = target
                                .variables
                                .iter()
                                .find(|v| v.name == variable.name)
                                .map(|v| v.id);
                            let _ = target.save(
                                target_id,
                                &variable.name,
                                &variable.value,
                                variable.secret,
                            );
                            if let Some(saved) = target
                                .variables
                                .iter_mut()
                                .find(|v| v.name == variable.name)
                            {
                                saved.expires_at = variable.expires_at;
                            }
                        }
                    }
                }
            }
        }
        shared.collections = editor.collections.clone();
        let environment_changed = shared.selected_environment != selected_environment;
        shared.selected_environment = selected_environment;
        shared.signature = signature;
        shared.running = editor.sending.is_some() || editor.ws_active || editor.ws_connected;
        shared.websocket = editor.ws_active || editor.ws_connected;
        shared.connected = editor.ws_connected;
        self.requests[index] = request;
        self.responses[index] = editor.response_data.clone();
        self.traces[index] = editor.traces[0].clone();
        self.sent_requests[index] = editor.sent_requests[0].clone();
        self.sent_curls[index] = editor.sent_curls[0].clone();
        self.current_versions[index] = editor.current_versions[0];
        if environment_changed {
            self.environment.update(cx, |select, cx| {
                select.set_selected(Some(selected_environment.to_string().into()), cx)
            });
        }
        self.storage_dirty = true;
        true
    }

    pub(super) fn flush_request_editors(&mut self, cx: &mut Context<Self>) {
        for id in self.request_editors.keys().cloned().collect::<Vec<_>>() {
            self.sync_request_editor(&id, cx);
        }
    }

    pub(super) fn refresh_request_editors(&mut self, cx: &mut Context<Self>) {
        if self.request_editors.is_empty() {
            return;
        }
        let selected = self.selected_environment(cx);
        let context = serde_json::to_string(&(
            self.requests
                .iter()
                .map(|r| (&r.id, &r.name, &r.collection, &r.group, &r.draft))
                .collect::<Vec<_>>(),
            &self.collections,
            &self.environment_names,
            &self.environment_inputs,
            &self.proxies,
            &self.disabled_plugins,
            &self.plugin_catalog,
            selected,
        ))
        .unwrap();
        for (id, shared) in &mut self.request_editors {
            let request = self
                .requests
                .iter()
                .find(|request| &request.id == id)
                .unwrap();
            let current = shared.editor.read(cx);
            let metadata_changed = current.requests[0].name != request.name
                || current.requests[0].collection != request.collection
                || current.requests[0].group != request.group
                || current.requests[0].tree_order != request.tree_order;
            if shared.context == context && current.theme == self.theme && !metadata_changed {
                continue;
            }
            shared.context = context.clone();
            shared.selected_environment = selected;
            shared.editor.update(cx, |editor, cx| {
                editor.requests[0].name = request.name.clone();
                editor.requests[0].collection = request.collection.clone();
                editor.requests[0].group = request.group.clone();
                editor.requests[0].tree_order = request.tree_order;
                let environment_changed = editor.environment_names != self.environment_names
                    || editor.selected_environment(cx) != selected;
                editor.trigger_requests = Some(self.requests.clone());
                editor.sync_pair_tables(cx);
                editor.collections = self.collections.clone();
                editor.environment_names = self.environment_names.clone();
                editor.environment_inputs = self.environment_inputs.clone();
                editor.proxies = self.proxies.clone();
                editor.disabled_plugins = self.disabled_plugins.clone();
                editor.plugin_catalog = self.plugin_catalog.clone();
                if environment_changed {
                    editor.mcp_environment_options(selected, cx);
                }
                if editor.theme != self.theme {
                    editor.set_theme(self.theme, cx);
                }
                editor.sync_reference_inputs(cx);
                cx.notify();
            });
        }
    }
}
