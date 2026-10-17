use super::*;

impl Workbench {
    pub(super) fn prepend_history(&mut self, request: usize, record: Record) {
        self.requests[request].history.insert(0, record);
        if let Some((r, index)) = &mut self.history_delete {
            if *r == request {
                *index += 1;
            }
        }
        self.selected_history = Some((request, 0));
    }

    fn delete_history_record(&mut self, request: usize, index: usize, cx: &mut Context<Self>) {
        self.history_delete = None;
        let Some(history) = self.requests.get_mut(request).map(|r| &mut r.history) else {
            return;
        };
        if index >= history.len() {
            return;
        }
        history.remove(index);
        let deleted_selected = self.selected_history == Some((request, index));
        self.selected_history = self.selected_history.and_then(|(r, i)| {
            if r != request {
                Some((r, i))
            } else if i == index {
                None
            } else {
                Some((r, i - usize::from(i > index)))
            }
        });
        if deleted_selected && request == self.active && self.sending.is_none() {
            self.response_data = None;
            self.response_json = None;
            self.status = None;
            self.error = None;
            self.responses[request] = None;
            self.sent_requests[request] = None;
            self.sent_curls[request] = None;
            self.traces[request].clear();
            self.sync_request_preview(cx);
            self.update_response(cx);
        }
        self.storage_dirty = true;
        self.notice = "已删除请求历史".into();
        self.persist(cx);
        cx.notify();
    }
    pub(super) fn history_delete_popover(
        &self,
        index: usize,
        record: &Record,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let target = (self.active, index);
        let open = self.history_delete == Some(target);
        let group = gpui::SharedString::from(format!("history-record-{index}"));
        let trigger = IconButton::new(Icon::Trash, "删除此条请求历史")
            .theme(self.theme)
            .render()
            .size(px(20.))
            .debug_selector(move || format!("delete-history-{index}").into())
            .when(!open, |s| {
                s.opacity(0.).group_hover(group, |s| s.opacity(1.))
            });
        let content = div()
            .debug_selector(|| "history-delete-confirmation".into())
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("删除此条请求历史？"),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child(format!("{} · {}", record.label, record.time)),
            )
            .child("该次请求的响应与执行记录将一并删除。")
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("取消")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "cancel-history-delete".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.history_delete = None;
                                window.focus(&this.focus, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("确认删除")
                            .kind(ButtonKind::Danger)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "confirm-history-delete".into())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.history_delete == Some(target) {
                                    this.delete_history_record(target.0, target.1, cx);
                                }
                                window.focus(&this.focus, cx);
                            })),
                    ),
            );
        let owner = cx.entity().downgrade();
        Popover::new(format!("history-delete-{index}"), trigger, content)
            .theme(self.theme)
            .width(280.)
            .trigger_height(20.)
            .open(open)
            .on_open_change(move |open, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.history_delete = open.then_some(target);
                    window.focus(&this.focus, cx);
                    cx.notify();
                });
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn click(window: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = window.debug_bounds(selector).unwrap();
        window.simulate_event(gpui::MouseMoveEvent {
            position: bounds.center(),
            ..Default::default()
        });
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
    }
    #[gpui::test]
    fn history_delete_confirms_preserves_selection_and_persists(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            for (i, status) in [200, 404, 200].into_iter().enumerate() {
                let mut record = host.requests[0].versions[0].clone();
                record.label = status_label(status);
                record.draft.url = format!("https://example.test/{i}");
                record.exchange = Some(model::Exchange {
                    curl: None,
                    response: Some(HttpResponse {
                        status,
                        text: format!("response-{i}"),
                        bytes: 10,
                        headers: vec![],
                        json: None,
                    }),
                    error: None,
                    preview: Some(format!("GET /{i}")),
                    traces: vec![],
                });
                host.requests[0].history.push(record);
            }
            host.activity = 1;
            host.show_activity = true;
            host.storage_dirty = true;
            host.persist(cx);
            host
        });
        let original = host.read_with(window, |h, cx| h.snapshot(cx));
        for dismissal in ["cancel", "escape", "outside"] {
            click(window, "delete-history-1");
            assert!(window.debug_bounds("history-delete-confirmation").is_some());
            host.update(window, |h, cx| {
                assert_eq!(h.requests[0].history.len(), 3);
                assert_eq!(h.snapshot(cx), original);
                assert!(h.selected_history.is_none());
            });
            match dismissal {
                "cancel" => click(window, "cancel-history-delete"),
                "escape" => window.simulate_keystrokes("escape"),
                _ => window.simulate_click(point(px(400.), px(350.)), gpui::Modifiers::default()),
            }
            host.update(window, |h, _| {
                assert!(h.history_delete.is_none());
                assert_eq!(h.requests[0].history.len(), 3);
            });
        }
        click(window, "history-record-2");
        host.update(window, |h, cx| {
            assert_eq!(h.selected_history, Some((0, 2)));
            h.history_filter = 2;
            cx.notify();
        });
        assert!(window.debug_bounds("history-record-0").is_none());
        click(window, "delete-history-1");
        click(window, "confirm-history-delete");
        host.update(window, |h, cx| {
            assert_eq!(h.requests[0].history.len(), 2);
            assert_eq!(h.selected_history, Some((0, 1)));
            assert_eq!(h.response_data.as_ref().unwrap().text, "response-2");
            assert_eq!(h.snapshot(cx).url, "https://example.test/2");
            assert_eq!(h.requests[0].versions.len(), 1);
            h.history_filter = 0;
            cx.notify();
        });
        // A completed response arriving during confirmation must not change the deletion target.
        click(window, "delete-history-1");
        host.update(window, |h, cx| {
            let mut fresh = h.requests[0].history[0].clone();
            fresh.draft.url = "https://example.test/new".into();
            h.prepend_history(0, fresh);
            assert_eq!(h.history_delete, Some((0, 2)));
            cx.notify();
        });
        click(window, "confirm-history-delete");
        host.update(window, |h, _| {
            assert_eq!(h.requests[0].history.len(), 2);
            assert_eq!(
                h.requests[0].history[0].draft.url,
                "https://example.test/new"
            );
            assert_eq!(h.requests[0].history[1].draft.url, "https://example.test/0");
        });
        click(window, "history-record-1");
        click(window, "delete-history-1");
        click(window, "confirm-history-delete");
        host.update(window, |h, cx| {
            assert_eq!(h.requests[0].history.len(), 1);
            assert!(h.selected_history.is_none());
            assert!(h.response_data.is_none());
            assert!(h.responses[0].is_none());
            assert!(h.sent_requests[0].is_none());
            assert_eq!(h.snapshot(cx).url, "https://example.test/0");
        });
        click(window, "delete-history-0");
        click(window, "confirm-history-delete");
        let restored = storage::Store::open(&path).unwrap().load().unwrap();
        assert!(restored.requests[0].history.is_empty());
        assert_eq!(restored.requests[0].versions.len(), 1);
        assert_eq!(restored.requests[0].draft.url, "https://example.test/0");
        assert!(window.debug_bounds("history-record-0").is_none());
    }
}
