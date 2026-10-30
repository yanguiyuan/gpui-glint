use super::*;

pub(super) struct WsSessionState {
    pub(super) sequence: u64,
    pub(super) history: Option<(String, String, Instant)>,
    pub(super) commands: Option<async_channel::Sender<websocket::Command>>,
    pub(super) connected: bool,
    pub(super) active: bool,
    pub(super) log: std::collections::VecDeque<websocket_ui::WsMessage>,
    pub(super) scroll: gpui::ScrollHandle,
    pub(super) detail_scroll: gpui::ScrollHandle,
    pub(super) selected: Option<u64>,
    pub(super) follow: bool,
    pub(super) follow_latest: bool,
    pub(super) raw: bool,
    pub(super) detail_tab: usize,
    pub(super) menu: Option<websocket_ui::WsMenu>,
    pub(super) decrypted: Option<Result<String, String>>,
    pub(super) decode_task: Option<Task<()>>,
    pub(super) decode_generation: u64,
    pub(super) environment: Option<environment::EnvironmentValues>,
    pub(super) output: Entity<CodeCard>,
    pub(super) task: Option<Task<()>>,
    pub(super) notice: String,
    pub(super) error: Option<String>,
}
impl WsSessionState {
    pub(super) fn empty(theme: ThemePreset, cx: &mut Context<Workbench>) -> Self {
        Self {
            sequence: 0,
            history: None,
            commands: None,
            connected: false,
            active: false,
            log: Default::default(),
            scroll: gpui::ScrollHandle::new(),
            detail_scroll: gpui::ScrollHandle::new(),
            selected: None,
            follow: true,
            follow_latest: true,
            raw: false,
            detail_tab: 0,
            menu: None,
            decrypted: None,
            decode_task: None,
            decode_generation: 0,
            environment: None,
            output: cx.new(|cx| {
                CodeCard::new(cx, "text", "")
                    .theme(theme)
                    .read_only(true)
                    .show_header(false)
                    .prose(true)
                    .viewport()
            }),
            task: None,
            notice: String::new(),
            error: None,
        }
    }
}
impl Workbench {
    pub(super) fn swap_ws_state(&mut self, state: &mut WsSessionState) {
        std::mem::swap(&mut self.ws_sequence, &mut state.sequence);
        std::mem::swap(&mut self.ws_history, &mut state.history);
        std::mem::swap(&mut self.ws_commands, &mut state.commands);
        std::mem::swap(&mut self.ws_connected, &mut state.connected);
        std::mem::swap(&mut self.ws_active, &mut state.active);
        std::mem::swap(&mut self.ws_log, &mut state.log);
        std::mem::swap(&mut self.ws_scroll, &mut state.scroll);
        std::mem::swap(&mut self.ws_detail_scroll, &mut state.detail_scroll);
        std::mem::swap(&mut self.ws_selected, &mut state.selected);
        std::mem::swap(&mut self.ws_follow, &mut state.follow);
        std::mem::swap(&mut self.ws_follow_latest, &mut state.follow_latest);
        std::mem::swap(&mut self.ws_raw, &mut state.raw);
        std::mem::swap(&mut self.ws_detail_tab, &mut state.detail_tab);
        std::mem::swap(&mut self.ws_menu, &mut state.menu);
        std::mem::swap(&mut self.ws_decrypted, &mut state.decrypted);
        std::mem::swap(&mut self.ws_decode_task, &mut state.decode_task);
        std::mem::swap(&mut self.ws_decode_generation, &mut state.decode_generation);
        std::mem::swap(&mut self.ws_environment, &mut state.environment);
        std::mem::swap(&mut self.ws_output, &mut state.output);
        std::mem::swap(&mut self.sending, &mut state.task);
        std::mem::swap(&mut self.notice, &mut state.notice);
        std::mem::swap(&mut self.error, &mut state.error);
    }
    pub(super) fn park_websocket(&mut self, cx: &mut Context<Self>) {
        if !self.ws_active && self.ws_commands.is_none() && self.ws_log.is_empty() {
            return;
        }
        let Some(id) = self
            .workspace
            .requests
            .get(self.active)
            .map(|r| r.id.clone())
        else {
            return;
        };
        let http = if !self.ws_active {
            self.sending.take()
        } else {
            None
        };
        let mut state = WsSessionState::empty(self.theme, cx);
        self.swap_ws_state(&mut state);
        state.menu = None;
        self.ws_background.insert(id, state);
        self.sending = http;
    }
    pub(super) fn unpark_websocket(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self
            .workspace
            .requests
            .get(self.active)
            .map(|r| r.id.clone())
        else {
            return;
        };
        if let Some(mut state) = self.ws_background.remove(&id) {
            self.swap_ws_state(&mut state);
            self.ws_output
                .update(cx, |output, cx| output.set_theme(self.theme, cx));
            self.ws_refresh_decoder_select(cx);
            self.ws_sync_detail(cx);
        }
    }
    pub(super) fn with_ws_request<R>(
        &mut self,
        request_id: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Context<Self>) -> R,
    ) -> Option<R> {
        if let Some(editor) = self
            .request_editors
            .get(request_id)
            .map(|shared| shared.editor.clone())
        {
            return Some(editor.update(cx, |editor, cx| f(editor, cx)));
        }
        let index = self
            .workspace
            .requests
            .iter()
            .position(|r| r.id == request_id)?;
        if index == self.active {
            return Some(f(self, cx));
        }
        let mut state = self.ws_background.remove(request_id)?;
        let active = self.active;
        self.swap_ws_state(&mut state);
        self.active = index;
        self.ws_background_update = true;
        let result = f(self, cx);
        self.ws_background_update = false;
        self.active = active;
        self.swap_ws_state(&mut state);
        self.ws_background.insert(request_id.to_owned(), state);
        Some(result)
    }
    pub(super) fn websocket_variables(&self, cx: &App) -> environment::EnvironmentValues {
        self.ws_environment
            .clone()
            .unwrap_or_else(|| self.active_environment_values(cx))
    }
    pub(super) fn running_request_tasks(&self) -> Vec<(String, String)> {
        self.workspace
            .requests
            .iter()
            .enumerate()
            .filter_map(|(index, request)| {
                if let Some(shared) = self.request_editors.get(&request.id) {
                    return shared.running.then(|| {
                        (
                            request.id.clone(),
                            format!(
                                "{} · {}",
                                request.name,
                                if shared.websocket {
                                    if shared.connected {
                                        "WS 已连接"
                                    } else {
                                        "WS 连接中 / 断开中"
                                    }
                                } else {
                                    "请求执行中"
                                }
                            ),
                        )
                    });
                }
                let label = if index == self.active {
                    if self.ws_active {
                        if self.ws_connected {
                            "WS 已连接"
                        } else {
                            "WS 连接中 / 断开中"
                        }
                    } else if self.sending.is_some() {
                        "请求执行中"
                    } else {
                        return None;
                    }
                } else {
                    let state = self.ws_background.get(&request.id)?;
                    if !state.active && !state.connected {
                        return None;
                    }
                    if state.connected {
                        "WS 已连接"
                    } else {
                        "WS 连接中 / 断开中"
                    }
                };
                Some((request.id.clone(), format!("{} · {label}", request.name)))
            })
            .collect()
    }
    pub(super) fn background_tasks_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tasks = self.running_request_tasks();
        let count = tasks.len();
        let p = self.theme.palette();
        let mut rows = div()
            .id("background-task-list")
            .flex()
            .flex_col()
            .max_h(px(320.))
            .overflow_y_scroll()
            .occlude();
        if tasks.is_empty() {
            rows = rows.child(
                div()
                    .p_3()
                    .text_color(rgb(p.text_muted))
                    .child("暂无运行中的任务"),
            );
        }
        for (id, label) in tasks {
            let selector = id.clone();
            rows = rows.child(
                MenuItem::new(format!("background-task-{id}"), label)
                    .icon(Icon::Workflow)
                    .theme(self.theme)
                    .render()
                    .debug_selector(move || format!("background-task-{selector}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_background_tasks = false;
                        if let Some(index) = this.workspace.requests.iter().position(|r| r.id == id)
                        {
                            if index != this.active {
                                this.choose(index, cx);
                            } else {
                                if matches!(
                                    this.page,
                                    crate::Page::Proxies
                                        | crate::Page::Plugins
                                        | crate::Page::Environments
                                        | crate::Page::Mcp
                                ) {
                                    this.page = crate::Page::Requests;
                                }
                            }
                        }
                        cx.notify();
                    })),
            );
        }
        let owner = cx.entity().downgrade();
        DropdownMenu::new(
            "background-tasks-menu",
            Button::new(format!("后台任务 · {count}"))
                .kind(ButtonKind::Plain)
                .size(ButtonSize::Mini)
                .theme(self.theme)
                .render()
                .debug_selector(|| "background-tasks-trigger".into()),
            rows,
        )
        .width(320.)
        .open(self.show_background_tasks)
        .theme(self.theme)
        .on_open_change(move |open, _, cx| {
            let _ = owner.update(cx, |this, cx| {
                this.show_background_tasks = open;
                cx.notify();
            });
        })
        .render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait_for(
        host: &Entity<Workbench>,
        window: &mut gpui::VisualTestContext,
        ready: impl Fn(&Workbench) -> bool,
    ) {
        for _ in 0..500 {
            window.run_until_parked();
            if host.read_with(window, |host, _| ready(host)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("WebSocket task did not reach the expected state");
    }
    fn echo_server(label: &'static str) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}/events", listener.local_addr().unwrap());
        let task = std::thread::spawn(move || {
            use tokio_tungstenite::tungstenite::{Message, accept};
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut socket = accept(socket).unwrap();
            socket
                .send(Message::Text(format!("{label}:ready").into()))
                .unwrap();
            while let Ok(message) = socket.read() {
                match message {
                    Message::Text(text) => socket
                        .send(Message::Text(format!("{label}:{text}").into()))
                        .unwrap(),
                    Message::Close(_) => {
                        let _ = socket.flush();
                        break;
                    }
                    _ => {}
                }
            }
        });
        (url, task)
    }
    #[gpui::test]
    fn real_websocket_callbacks_stay_with_their_request_and_finish_independent_histories(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.background_executor.allow_parking();
        let (first_url, first_server) = echo_server("first");
        let (second_url, second_server) = echo_server("second");
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&dir.path().join("live-background.sqlite3")).unwrap(),
                cx,
            );
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("WS", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.load(
                &Draft {
                    method: "WS".into(),
                    url: first_url,
                    ..Default::default()
                },
                cx,
            );
            host.capture_draft(cx);
            host.add_request(cx);
            host.load(
                &Draft {
                    method: "WS".into(),
                    url: second_url,
                    ..Default::default()
                },
                cx,
            );
            host.capture_draft(cx);
            host.choose(0, cx);
            host.connect_websocket(cx);
            host
        });
        wait_for(&host, window, |host| host.ws_connected);
        let first = host.update(window, |host, cx| {
            let first = host.workspace.requests[0].id.clone();
            host.choose(1, cx);
            host.connect_websocket(cx);
            first
        });
        wait_for(&host, window, |host| {
            host.ws_connected
                && host
                    .ws_background
                    .get(&first)
                    .is_some_and(|state| state.connected)
        });
        host.update(window, |host, cx| {
            host.mcp_call("glint_websocket_send", serde_json::json!({"request_id":first, "text":"background", "resolve_templates":false}), cx).unwrap();
        });
        wait_for(&host, window, |host| {
            host.ws_background.get(&first).is_some_and(|state| {
                state
                    .log
                    .iter()
                    .any(|event| event.log_text().contains("first:background"))
            })
        });
        host.update(window, |host, cx| {
            assert_eq!(host.active, 1);
            assert!(
                host.ws_log
                    .iter()
                    .all(|event| !event.log_text().contains("first:background"))
            );
            assert_eq!(host.running_request_tasks().len(), 2);
            let second = host.workspace.requests[1].id.clone();
            host.mcp_call(
                "glint_request_cancel",
                serde_json::json!({"request_id":first}),
                cx,
            )
            .unwrap();
            host.mcp_call(
                "glint_request_cancel",
                serde_json::json!({"request_id":second}),
                cx,
            )
            .unwrap();
        });
        wait_for(&host, window, |host| {
            host.running_request_tasks().is_empty()
        });
        host.update(window, |host, cx| {
            for request in &host.workspace.requests {
                assert_eq!(request.history.len(), 1);
                let record = &request.history[0];
                assert_eq!(record.label, "WS 已断开");
                assert!(record.is_success());
                assert!(
                    !record
                        .exchange
                        .as_ref()
                        .unwrap()
                        .websocket_events
                        .is_empty()
                );
            }
            host.persist(cx);
            host.choose(0, cx);
            assert!(!host.ws_connected && !host.ws_active);
            assert!(
                host.ws_log
                    .iter()
                    .any(|event| event.log_text().contains("first:background"))
            );
        });
        first_server.join().unwrap();
        second_server.join().unwrap();
    }

    #[gpui::test]
    fn concurrent_websockets_survive_switches_route_mcp_and_jump_from_task_menu(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&dir.path().join("background.sqlite3")).unwrap(),
                cx,
            );
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("WS", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.load(
                &Draft {
                    method: "WS".into(),
                    url: "ws://first/".into(),
                    ..Default::default()
                },
                cx,
            );
            host.capture_draft(cx);
            host.add_request(cx);
            host.load(
                &Draft {
                    method: "WS".into(),
                    url: "ws://second/".into(),
                    ..Default::default()
                },
                cx,
            );
            host.capture_draft(cx);
            host.choose(0, cx);
            host
        });
        let (first, second, first_rx, second_rx) = host.update(window, |host, cx| {
            let first = host.workspace.requests[0].id.clone();
            let second = host.workspace.requests[1].id.clone();
            let (sender, first_rx) = async_channel::bounded(16);
            host.ws_commands = Some(sender);
            host.ws_active = true;
            host.ws_connected = true;
            host.sending = Some(cx.spawn(async |_, _| std::future::pending::<()>().await));
            host.ws_push(websocket_ui::MessageKind::Received, "first-visible".into(), cx);
            host.choose(1, cx);
            assert!(!first_rx.is_closed());
            assert!(host.ws_background[&first].connected);
            assert!(host.ws_background[&first].task.is_some());
            let (sender, second_rx) = async_channel::bounded(16);
            host.ws_commands = Some(sender);
            host.ws_active = true;
            host.ws_connected = true;
            host.sending = Some(cx.spawn(async |_, _| std::future::pending::<()>().await));
            host.notice = "second notice".into();
            host.ws_push(websocket_ui::MessageKind::Received, "second-visible".into(), cx);
            let second_output = host.ws_output.read(cx).content().to_string();
            host.with_ws_request(&first, cx, |host, cx| {
                host.ws_push(websocket_ui::MessageKind::Received, "first-background".into(), cx);
                host.notice = "first background notice".into();
            }).unwrap();
            assert_eq!(host.active, 1);
            assert_eq!(host.notice, "second notice");
            assert_eq!(host.ws_output.read(cx).content(), second_output);
            assert_eq!(host.running_request_tasks().len(), 2);
            let status = host.mcp_call("glint_request_status", serde_json::json!({"request_id":first}), cx).unwrap();
            assert_eq!(status["running"], true);
            assert_eq!(status["websocket_connected"], true);
            let events = host.mcp_call("glint_websocket_read", serde_json::json!({"request_id":first}), cx).unwrap();
            assert_eq!(events["messages"].as_array().unwrap().len(), 2);
            assert_eq!(events["messages"][1]["payload"], "first-background");
            host.mcp_call("glint_websocket_send", serde_json::json!({"request_id":first, "text":"ping", "resolve_templates":false}), cx).unwrap();
            assert!(matches!(first_rx.try_recv().unwrap(), websocket::Command::Text(text) if text == "ping"));
            assert_eq!(host.active, 1);
            (first, second, first_rx, second_rx)
        });
        let menu = window.debug_bounds("background-tasks-trigger").unwrap();
        window.simulate_click(menu.center(), gpui::Modifiers::default());
        let row = window
            .debug_bounds(Box::leak(
                format!("background-task-{first}").into_boxed_str(),
            ))
            .unwrap();
        window.simulate_click(row.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.active, 0);
            assert!(host.ws_connected && host.ws_active);
            assert!(host.sending.is_some());
            assert!(
                host.ws_output
                    .read(cx)
                    .content()
                    .contains("first-background")
            );
            assert!(host.ws_background[&second].connected);
            assert!(!first_rx.is_closed() && !second_rx.is_closed());
            host.mcp_call(
                "glint_request_cancel",
                serde_json::json!({"request_id":second}),
                cx,
            )
            .unwrap();
            assert!(matches!(
                second_rx.try_recv().unwrap(),
                websocket::Command::Close
            ));
            assert!(host.ws_connected);
            host.delete_request(1, cx);
            assert!(second_rx.is_closed());
            assert!(!first_rx.is_closed());
            assert!(host.ws_connected && host.sending.is_some());
        });
    }
}
