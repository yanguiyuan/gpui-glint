use super::*;
#[gpui::test]
fn template_suggestions_follow_global_environment_and_extraction(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace.sqlite3");
    let (host, window) = cx.add_window_view(|_, cx| {
        let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
        for (kind, name) in [
            (GroupEdit::CreateCollection, "API"),
            (GroupEdit::CreateEnvironment, "Development"),
            (GroupEdit::CreateEnvironment, "Production"),
        ] {
            host.group_edit = Some(kind);
            host.group_name
                .update(cx, |entry, cx| entry.set_value(name, cx));
            host.save_group(cx);
        }
        host.environment_inputs[1]
            .save(None, "token_dev", "secret-dev", true)
            .unwrap();
        host.environment_inputs[2]
            .save(None, "token_prod", "secret-prod", true)
            .unwrap();
        host.add_request(cx);
        host.show_activity = false;
        let draft = Draft {
            params: vec![Pair::new("q", "{{tok", "")],
            ..Draft::default()
        };
        host.load(&draft, cx);
        host
    });
    for (index, expected) in [
        (1, "{{token_dev}}"),
        (2, "{{token_prod}}"),
        (0, "{{uuid()}}"),
    ] {
        host.update(window, |host, cx| {
            host.environment.update(cx, |_, cx| {
                cx.emit(SelectEvent::Changed(index.to_string().into()))
            });
            host.params[0].value.update(cx, |entry, cx| {
                entry.set_value(if index == 0 { "{{uu" } else { "{{tok" }, cx)
            });
        });
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                window.focus(&host.params[0].value.read(cx).focus_handle(cx), cx)
            });
        });
        window.update(|window, _| window.refresh());
        assert!(
            window.debug_bounds("template-completion-menu").is_some(),
            "missing completion in environment {index}"
        );
        window.simulate_keystrokes("tab");
        host.update(window, |host, cx| {
            assert_eq!(host.params[0].value.read(cx).value(), expected)
        });
    }
    window.update(|window, cx| {
        host.update(cx, |host, cx| {
            let draft = Draft {
                extractors: vec![Pair::new("new_token", "$.token", "")],
                ..Draft::default()
            };
            let response = HttpResponse {
                status: 200,
                headers: vec![],
                text: String::new(),
                bytes: 0,
                json: Some(serde_json::json!({"token":"new-secret"})),
            };
            host.environment
                .update(cx, |select, cx| select.set_selected(Some("2".into()), cx));
            host.apply_extractors(
                &draft,
                2,
                &host.requests[host.active].collection.clone(),
                &response,
                cx,
            )
            .unwrap();
            host.params[0]
                .value
                .update(cx, |entry, cx| entry.set_value("{{new_", cx));
            window.focus(&host.params[0].value.read(cx).focus_handle(cx), cx);
        });
    });
    window.simulate_keystrokes("enter");
    host.update(window, |host, cx| {
        assert_eq!(host.params[0].value.read(cx).value(), "{{new_token}}")
    });
}

#[gpui::test]
fn global_environment_survives_request_switch_versions_and_restart(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace.sqlite3");
    let (host, window) = cx.add_window_view(|_, cx| {
        let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
        for (kind, name) in [
            (GroupEdit::CreateCollection, "API"),
            (GroupEdit::CreateEnvironment, "Development"),
            (GroupEdit::CreateEnvironment, "Production"),
        ] {
            host.group_edit = Some(kind);
            host.group_name
                .update(cx, |entry, cx| entry.set_value(name, cx));
            host.save_group(cx);
        }
        host.add_request(cx);
        host.add_request(cx);
        host
    });
    let selector = window.debug_bounds("global-environment-selector").unwrap();
    assert!(selector.top() < px(42.));
    host.update(window, |host, cx| {
        host.environment
            .update(cx, |_, cx| cx.emit(SelectEvent::Changed("2".into())));
    });
    host.update(window, |host, cx| {
        let drafts: Vec<_> = host
            .requests
            .iter()
            .map(|request| request.draft.clone())
            .collect();
        host.choose(0, cx);
        assert_eq!(host.selected_environment(cx), 2);
        host.save(cx);
        assert_eq!(
            host.requests[0].versions.len(),
            1,
            "global environment changes must not create request versions"
        );
        // A legacy draft's request-local environment must no longer change the selector.
        let mut legacy = serde_json::to_value(&drafts[0]).unwrap();
        legacy["environment"] = 1.into();
        let legacy: Draft = serde_json::from_value(legacy).unwrap();
        host.load(&legacy, cx);
        assert_eq!(host.selected_environment(cx), 2);
        host.choose(1, cx);
        assert_eq!(host.selected_environment(cx), 2);
        assert_eq!(host.snapshot(cx), drafts[1]);
        host.persist(cx);
    });
    let data = storage::Store::open(&path).unwrap().load().unwrap();
    assert_eq!(data.selected_environment, 2);
    assert!(
        serde_json::to_value(&data.requests[0].draft)
            .unwrap()
            .get("environment")
            .is_none()
    );
    let reopened = window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
    reopened.update(window, |host, cx| {
        assert_eq!(host.selected_environment(cx), 2);
        host.choose(1, cx);
        assert_eq!(host.selected_environment(cx), 2);
    });
}

#[gpui::test]
fn sidebar_resize_drags_across_content_clamps_and_stops_on_release(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace.sqlite3");
    let (host, window) =
        cx.add_window_view(|_, cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
    let initial = window.debug_bounds("sidebar-resize-handle").unwrap();
    let initial_search = window.debug_bounds("sidebar-search-input").unwrap();
    let drag = |window: &mut gpui::VisualTestContext, target_x: f32| {
        let source = window
            .debug_bounds("sidebar-resize-handle")
            .unwrap()
            .center();
        let target = point(px(target_x), source.y);
        window.simulate_event(gpui::MouseDownEvent {
            position: source,
            button: gpui::MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        window.simulate_event(gpui::MouseMoveEvent {
            position: target,
            pressed_button: Some(gpui::MouseButton::Left),
            ..Default::default()
        });
        window.simulate_event(gpui::MouseUpEvent {
            position: target,
            button: gpui::MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
    };
    drag(window, f32::from(initial.center().x) + 50.);
    let widened = window.debug_bounds("sidebar-resize-handle").unwrap();
    assert_eq!(widened.left() - initial.left(), px(50.));
    let widened_search = window.debug_bounds("sidebar-search-input").unwrap();
    assert_eq!(widened_search.left(), initial_search.left());
    assert_eq!(
        widened_search.size.width - initial_search.size.width,
        px(50.)
    );
    assert_eq!(
        widened.left() - widened_search.right(),
        initial.left() - initial_search.right()
    );
    host.update(window, |host, _| {
        assert_eq!(host.sidebar_width, 270.);
        assert!(host.sidebar_resize_start.is_none());
    });
    window.simulate_event(gpui::MouseMoveEvent {
        position: point(px(700.), initial.center().y),
        ..Default::default()
    });
    assert_eq!(
        window.debug_bounds("sidebar-resize-handle").unwrap(),
        widened
    );
    drag(window, 900.);
    host.update(window, |host, _| assert!(host.sidebar_width <= 480.));
    drag(window, 10.);
    host.update(window, |host, _| {
        assert_eq!(host.sidebar_width, 180.);
    });
    let narrow_search = window.debug_bounds("sidebar-search-input").unwrap();
    assert_eq!(
        narrow_search.size.width,
        initial_search.size.width - px(40.)
    );
    assert_eq!(narrow_search.left(), initial_search.left());
    host.update(window, |host, cx| {
        host.show_environments = true;
        cx.notify();
    });
    assert_eq!(
        window.debug_bounds("sidebar-resize-handle").unwrap().left(),
        initial.left() - px(40.)
    );
}

#[gpui::test]
fn native_tree_drag_reorders_moves_and_persists(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "glint-api-drag-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (host, window) = cx.add_window_view(|_, cx| {
        let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
        host.group_edit = Some(GroupEdit::CreateCollection);
        host.group_name
            .update(cx, |entry, cx| entry.set_value("API", cx));
        host.save_group(cx);
        host.add_request(cx);
        host.add_request(cx);
        host.group_edit = Some(GroupEdit::Create("".into()));
        host.group_name
            .update(cx, |entry, cx| entry.set_value("Folder", cx));
        host.save_group(cx);
        host
    });
    let drag = |window: &mut gpui::VisualTestContext,
                source: gpui::Point<gpui::Pixels>,
                target: gpui::Point<gpui::Pixels>| {
        window.simulate_event(gpui::MouseDownEvent {
            position: source,
            button: gpui::MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        for _ in 0..2 {
            window.simulate_event(gpui::MouseMoveEvent {
                position: target,
                pressed_button: Some(gpui::MouseButton::Left),
                ..Default::default()
            });
        }
        window.simulate_event(gpui::MouseUpEvent {
            position: target,
            button: gpui::MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
    };
    let source = window.debug_bounds("request-leaf:0").unwrap();
    let target = window.debug_bounds("request-leaf:1").unwrap();
    drag(
        window,
        source.center(),
        point(target.center().x, target.bottom() - px(2.)),
    );
    host.update(window, |host, _| {
        let order = model::siblings(&host.collections, &host.requests, 0, "");
        assert!(
            order
                .iter()
                .position(|node| *node == model::TreeNode::Request(1))
                .unwrap()
                < order
                    .iter()
                    .position(|node| *node == model::TreeNode::Request(0))
                    .unwrap()
        );
        assert_eq!(
            host.active, 1,
            "dragging must not switch the active request"
        );
        assert_eq!(host.requests[0].versions.len(), 1);
    });
    let source = window.debug_bounds("request-leaf:0").unwrap();
    let folder = window.debug_bounds("request-directory:0:Folder").unwrap();
    drag(window, source.center(), folder.center());
    host.update(window, |host, _| {
        assert_eq!(host.requests[0].group, "Folder")
    });
    host.update(window, |host, cx| {
        host.group_edit = Some(GroupEdit::CreateCollection);
        host.group_name
            .update(cx, |entry, cx| entry.set_value("Other", cx));
        host.save_group(cx);
    });
    let source = window.debug_bounds("request-directory:0:Folder").unwrap();
    let target = window.debug_bounds("request-directory:1:").unwrap();
    drag(window, source.center(), target.center());
    host.update(window, |host, _| {
        assert_eq!(host.requests[0].collection, "Other");
        assert_eq!(host.requests[0].group, "Folder");
        assert!(host.collections[1].contains("Folder"));
    });
    host.update(window, |host, cx| {
        host.collapsed_directories.insert((1, "Folder".into()));
        cx.notify();
    });
    let source = window.debug_bounds("request-directory:1:").unwrap();
    let target = window.debug_bounds("request-directory:0:").unwrap();
    drag(
        window,
        source.center(),
        point(target.center().x, target.top() + px(2.)),
    );
    host.update(window, |host, _| {
        assert_eq!(host.collections[0].collection, "Other");
        assert!(host.collapsed_directories.contains(&(0, "Folder".into())));
    });
    let reopened = window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
    reopened.update(window, |host, _| {
        assert_eq!(host.requests[0].collection, "Other");
        assert_eq!(host.requests[0].group, "Folder");
        assert!(host.requests[0].tree_order > 0);
        assert_eq!(host.requests[0].versions.len(), 1);
        assert_eq!(host.collections[0].collection, "Other");
    });
    std::fs::remove_file(path).unwrap();
}
#[gpui::test]
fn history_click_restores_matching_response_headers_and_preview(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "glint-api-history-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (host, window) = cx.add_window_view(|_, cx| {
        let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
        host.group_edit = Some(GroupEdit::CreateCollection);
        host.group_name
            .update(cx, |entry, cx| entry.set_value("API", cx));
        host.save_group(cx);
        host.add_request(cx);
        for (status, text) in [(200, "first"), (404, "second")] {
            let mut record = host.requests[0].versions[0].clone();
            record.id = model::new_id();
            record.label = status_label(status);
            record.draft.url = format!("https://example.test/{text}");
            record.exchange = Some(model::Exchange {
                websocket_connected: false,
                websocket_events: vec![],
                curl: None,
                response: Some(HttpResponse {
                    status,
                    headers: vec![("x-record".into(), text.into())],
                    text: text.into(),
                    bytes: text.len(),
                    json: None,
                }),
                error: None,
                preview: Some(format!("GET /{text}")),
                traces: vec![TraceEntry {
                    elapsed_ms: 10,
                    message: text.into(),
                }],
            });
            host.requests[0].history.push(record);
        }
        let mut legacy = host.requests[0].versions[0].clone();
        legacy.id = model::new_id();
        host.requests[0].history.push(legacy);
        host.show_activity = true;
        host.activity = 1;
        host.storage_dirty = true;
        host.persist(cx);
        host
    });
    for (index, status, text) in [(0, 200, "first"), (1, 404, "second"), (0, 200, "first")] {
        let bounds = window
            .debug_bounds(if index == 0 {
                "history-record-0"
            } else {
                "history-record-1"
            })
            .unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.status, Some(status));
            assert_eq!(host.response_data.as_ref().unwrap().text, text);
            assert_eq!(host.response.read(cx).content(), text);
            assert_eq!(host.response_header_entries[0][1].read(cx).value(), text);
            assert_eq!(
                host.request_preview.read(cx).content(),
                format!("GET /{text}")
            );
            assert_eq!(host.traces[0][0].message, text);
        });
    }
    let bounds = window.debug_bounds("history-record-2").unwrap();
    window.simulate_click(bounds.center(), gpui::Modifiers::default());
    host.update(window, |host, _| {
        assert!(
            host.response_data.is_none()
                && host.status.is_none()
                && host.response_header_entries.is_empty()
        );
        assert!(host.notice.contains("未保存响应"));
    });
    let reopened = window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
    reopened.update(window, |host, _| {
        assert_eq!(
            host.requests[0].history[1]
                .exchange
                .as_ref()
                .unwrap()
                .response
                .as_ref()
                .unwrap()
                .text,
            "second"
        )
    });
    std::fs::remove_file(path).unwrap();
}
#[gpui::test]
fn tls_setting_click_persists_and_restores_per_request(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "glint-api-tls-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (host, window) = cx.add_window_view(|_, cx| {
        let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
        host.group_edit = Some(GroupEdit::CreateCollection);
        host.group_name
            .update(cx, |entry, cx| entry.set_value("API", cx));
        host.save_group(cx);
        host.add_request(cx);
        host.config = 4;
        host
    });
    let bounds = window.debug_bounds("skip-tls-verification").unwrap();
    window.simulate_click(bounds.center(), gpui::Modifiers::default());
    host.update(window, |host, cx| {
        assert!(host.snapshot(cx).skip_tls_verification);
        host.save(cx);
        assert!(host.requests[0].versions[0].draft.skip_tls_verification);
        host.add_request(cx);
        assert!(!host.snapshot(cx).skip_tls_verification);
        host.choose(0, cx);
        assert!(host.snapshot(cx).skip_tls_verification);
        let baseline = host.requests[0].versions[1].draft.clone();
        host.load(&baseline, cx);
        assert!(!host.snapshot(cx).skip_tls_verification);
        let saved = host.requests[0].versions[0].draft.clone();
        host.load(&saved, cx);
        host.persist(cx);
    });
    let reopened = window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
    reopened.update(window, |host, cx| {
        assert!(host.snapshot(cx).skip_tls_verification)
    });
    let mut legacy = serde_json::to_value(Draft::default()).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("skip_tls_verification");
    assert!(
        !serde_json::from_value::<Draft>(legacy)
            .unwrap()
            .skip_tls_verification
    );
    std::fs::remove_file(path).unwrap();
}
#[gpui::test]
fn request_tree_menus_create_root_and_nested_requests_without_toggling_folders(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "glint-api-tree-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (host, window) = cx.add_window_view(|_, cx| {
        let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
        host.add_request(cx);
        assert!(
            host.requests.is_empty(),
            "a collection is required before creating a request"
        );
        host.group_name
            .update(cx, |entry, cx| entry.set_value("My API", cx));
        host.save_group(cx);
        host.add_request(cx);
        assert!(host.collections[0].names.is_empty());
        assert!(host.requests[0].group.is_empty());
        host.collections[0].add_child("", "目录").unwrap();
        host.collections[0].add_child("目录", "子目录").unwrap();
        host.add_request_to("目录/子目录".into(), cx);
        host
    });
    let root = window.debug_bounds("request-directory:0:").unwrap();
    window.simulate_event(gpui::MouseMoveEvent {
        position: root.center(),
        ..Default::default()
    });
    let more = window
        .debug_bounds("directory-menu:0:")
        .expect("hover reveals a single menu trigger");
    window.simulate_click(more.center(), gpui::Modifiers::default());
    host.update(window, |host, _| {
        assert_eq!(host.tree_menu, Some((0, String::new())));
        assert!(!host.collapsed_directories.contains(&(0, String::new())));
    });
    assert!(window.debug_bounds("directory-menu-action:0::2").is_some());
    let new_request = window.debug_bounds("directory-menu-action:0::0").unwrap();
    window.simulate_click(new_request.center(), gpui::Modifiers::default());
    host.update(window, |host, _| {
        assert_eq!(host.requests.len(), 3);
        assert!(host.requests[2].group.is_empty());
        assert!(host.tree_menu.is_none());
    });
    let folder = window
        .debug_bounds("request-directory:0:目录/子目录")
        .unwrap();
    window.simulate_click(folder.center(), gpui::Modifiers::default());
    host.update(window, |host, _| {
        assert!(
            host.collapsed_directories
                .contains(&(0, "目录/子目录".into()))
        )
    });
    window.simulate_event(gpui::MouseMoveEvent {
        position: folder.center(),
        ..Default::default()
    });
    let more = window.debug_bounds("directory-menu:0:目录/子目录").unwrap();
    window.simulate_click(more.center(), gpui::Modifiers::default());
    host.update(window, |host, _| {
        assert!(
            host.collapsed_directories
                .contains(&(0, "目录/子目录".into()))
        )
    });
    window.simulate_keystrokes("escape");
    host.update(window, |host, _| assert!(host.tree_menu.is_none()));
    window.simulate_click(more.center(), gpui::Modifiers::default());
    let create_folder = window
        .debug_bounds("directory-menu-action:0:目录/子目录:1")
        .unwrap();
    window.simulate_click(create_folder.center(), gpui::Modifiers::default());
    host.update(window, |host, cx| {
        assert!(
            matches!(&host.group_edit, Some(GroupEdit::Create(parent)) if parent == "目录/子目录")
        );
        host.group_name
            .update(cx, |entry, cx| entry.set_value("第三层", cx));
    });
    window.simulate_keystrokes("enter");
    host.update(window, |host, cx| {
        assert!(host.collections[0].contains("目录/子目录/第三层"));
        host.add_request(cx);
        assert_eq!(host.requests.last().unwrap().group, "目录/子目录/第三层");
        host.persist(cx);
    });
    let saved = storage::Store::open(&path).unwrap().load().unwrap();
    assert!(saved.requests[0].group.is_empty());
    assert_eq!(saved.requests.last().unwrap().group, "目录/子目录/第三层");
    std::fs::remove_file(path).unwrap();
}

#[gpui::test]
fn form_body_mode_edits_switches_versions_and_survives_restart(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace.sqlite3");
    let (host, window) = cx.add_window_view(|_, cx| {
        let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
        host.group_edit = Some(GroupEdit::CreateCollection);
        host.group_name
            .update(cx, |entry, cx| entry.set_value("API", cx));
        host.save_group(cx);
        host.add_request(cx);
        host.body
            .update(cx, |body, cx| body.set_content(r#"{"raw":true}"#, cx));
        host.config = 3;
        host.show_activity = false;
        host
    });
    let format = window.debug_bounds("request-body-format").unwrap();
    window.simulate_click(
        point(format.right() - px(20.), format.center().y),
        gpui::Modifiers::default(),
    );
    host.update(window, |host, cx| {
        assert_eq!(host.body_format, BodyFormat::UrlEncoded);
        host.form_table
            .update(cx, |_, cx| cx.emit(TableEvent::AddRow));
    });
    host.update(window, |host, cx| {
        assert_eq!(host.form.len(), 1);
        host.form[0]
            .key
            .update(cx, |entry, cx| entry.set_value("token", cx));
        host.form[0]
            .value
            .update(cx, |entry, cx| entry.set_value("{{api_token}}", cx));
        host.save(cx);
    });
    let format = window.debug_bounds("request-body-format").unwrap();
    window.simulate_click(
        point(format.left() + px(20.), format.center().y),
        gpui::Modifiers::default(),
    );
    host.update(window, |host, cx| {
        assert_eq!(host.body_format, BodyFormat::Raw);
        assert_eq!(host.body.read(cx).content(), r#"{"raw":true}"#);
        let saved = host.requests[0].versions[0].draft.clone();
        host.load(&saved, cx);
        assert_eq!(host.snapshot(cx).body_format, BodyFormat::UrlEncoded);
        assert_eq!(host.snapshot(cx).form[0].value, "{{api_token}}");
        host.add_request(cx);
        assert_eq!(host.body_format, BodyFormat::Raw);
        assert!(host.form.is_empty());
        host.choose(0, cx);
        assert_eq!(host.body_format, BodyFormat::UrlEncoded);
        host.persist(cx);
    });
    let reopened = window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
    reopened.update(window, |host, cx| {
        assert_eq!(host.snapshot(cx).body_format, BodyFormat::UrlEncoded);
        assert_eq!(host.snapshot(cx).form[0].key, "token");
    });
    let mut legacy = serde_json::to_value(Draft::default()).unwrap();
    legacy.as_object_mut().unwrap().remove("body_format");
    legacy.as_object_mut().unwrap().remove("form");
    let draft: Draft = serde_json::from_value(legacy).unwrap();
    assert_eq!(draft.body_format, BodyFormat::Raw);
    assert!(draft.form.is_empty());
}

#[gpui::test]
fn send_validation_shows_specific_error_toasts_without_starting_network(
    cx: &mut gpui::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let (host, window) = cx.add_window_view(|_, cx| {
        let mut host = Workbench::with_store(
            storage::Store::open(&directory.path().join("workspace.sqlite3")).unwrap(),
            cx,
        );
        host.group_edit = Some(GroupEdit::CreateCollection);
        host.group_name
            .update(cx, |entry, cx| entry.set_value("API", cx));
        host.save_group(cx);
        host.add_request(cx);
        host
    });
    host.update(window, |host, cx| {
        host.endpoint.update(cx, |field, cx| {
            field.set_text("{{missing_base_url}}/test", cx)
        });
        host.send(cx);
        assert!(host.sending.is_none());
        let toast = host.toasts.read(cx).toasts().last().unwrap();
        assert_eq!(toast.kind, ToastKind::Error);
        assert_eq!(toast.message.as_ref(), "无法发送请求");
        assert!(
            toast
                .description
                .as_ref()
                .unwrap()
                .contains("missing_base_url")
        );
        host.endpoint.update(cx, |field, cx| {
            field.set_text("https://example.invalid/", cx)
        });
        let plugin = host
            .plugin_catalog
            .iter()
            .find(|plugin| plugin.manifest.id == "com.wps.kso1")
            .unwrap();
        host.requests[0].draft.plugins = vec![model::BeforeAction {
            environment_bindings: Default::default(),
            approval: gpui_glint::plugins::Approval::for_plugin(plugin),
            action_id: "sign-request".into(),
            enabled: true,
        }];
        host.send(cx);
        assert!(host.sending.is_none());
        let toast = host.toasts.read(cx).toasts().last().unwrap();
        let description = toast.description.as_ref().unwrap();
        assert!(description.contains("WPS KSO-1 签名"));
        assert!(description.contains("wps_app_id") && description.contains("wps_app_key"));
        assert!(description.contains("不使用环境"));
        assert!(host.requests[0].history.is_empty());
    });
}

#[gpui::test]
fn request_body_card_scrolls_fully_above_response_panel(cx: &mut gpui::TestAppContext) {
    struct BodyPanel(Entity<Workbench>, gpui::ScrollHandle);
    impl Render for BodyPanel {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(700.))
                .h(px(400.))
                .flex()
                .flex_col()
                .child(
                    self.0
                        .update(cx, |host, cx| host.configuration(cx))
                        .track_scroll(&self.1),
                )
                .child(
                    div()
                        .debug_selector(|| "body-test-response".into())
                        .h(px(42.))
                        .flex_shrink_0(),
                )
        }
    }
    let directory = tempfile::tempdir().unwrap();
    for method in ["POST", "WS"] {
        let host = cx.new(|cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join(format!("{method}.sqlite3"))).unwrap(),
                cx,
            );
            let mut draft = host.snapshot(cx);
            draft.method = method.into();
            draft.body = (0..40).map(|i| format!("line {i}\n")).collect();
            host.load(&draft, cx);
            host.config = 3;
            host
        });
        let scroll = gpui::ScrollHandle::new();
        let (_, window) = cx.add_window_view(|_, _| BodyPanel(host, scroll.clone()));
        let viewport = window.debug_bounds("request-config-scroll").unwrap();
        let editor = window.debug_bounds("request-body-editor").unwrap();
        let response = window.debug_bounds("body-test-response").unwrap();
        assert!(
            editor.size.height > viewport.size.height,
            "{method}: card must retain its content height"
        );
        assert!(viewport.bottom() <= response.top());
        window.simulate_event(gpui::ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-10000.))),
            ..Default::default()
        });
        assert!(
            scroll.offset().y < px(0.),
            "{method}: body must scroll vertically"
        );
        let bottom = window.debug_bounds("request-body-editor").unwrap().bottom();
        assert!(
            bottom <= viewport.bottom(),
            "{method}: card bottom must be reachable"
        );
        assert!(
            bottom > viewport.top(),
            "{method}: card must remain inside its viewport"
        );
        assert_eq!(window.debug_bounds("body-test-response").unwrap(), response);
    }
}

#[gpui::test]
fn response_search_shortcuts_open_and_focus_inline_controls(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("cmd-f", SearchResponse, Some("ApiWorkbench")),
            KeyBinding::new("ctrl-f", SearchResponse, Some("ApiWorkbench")),
        ])
    });
    let directory = tempfile::tempdir().unwrap();
    let (host, window) = cx.add_window_view(|_, cx| {
        let mut host = Workbench::with_store(
            storage::Store::open(&directory.path().join("workspace.sqlite3")).unwrap(),
            cx,
        );
        host.group_edit = Some(GroupEdit::CreateCollection);
        host.group_name
            .update(cx, |entry, cx| entry.set_value("API", cx));
        host.save_group(cx);
        host.add_request(cx);
        host.response_data = Some(HttpResponse {
            status: 200,
            headers: vec![],
            text: "response".into(),
            bytes: 8,
            json: None,
        });
        host.update_response(cx);
        host
    });
    for shortcut in ["cmd-f", "ctrl-f"] {
        window.update(|window, cx| {
            let focus = host.read(cx).focus.clone();
            window.focus(&focus, cx);
        });
        window.simulate_keystrokes(shortcut);
        let focus = host.update(window, |host, cx| {
            assert!(host.show_response_search);
            assert_eq!(host.response_tab, 0);
            host.response_search.focus_handle(cx)
        });
        window.update(|window, _| assert!(focus.is_focused(window)));
        window.simulate_keystrokes(shortcut);
        host.update(window, |host, _| assert!(host.show_response_search));
        window.simulate_keystrokes("escape");
        host.update(window, |host, _| assert!(!host.show_response_search));
    }
}

#[gpui::test]
fn response_search_opens_cycles_refreshes_and_closes(cx: &mut gpui::TestAppContext) {
    cx.update(bind_text_entry_keys);
    struct ResponsePanel(Entity<Workbench>, f32);
    impl Render for ResponsePanel {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(self.1))
                .h(px(300.))
                .flex()
                .flex_col()
                .child(self.0.update(cx, |host, cx| host.response_panel(cx)))
        }
    }
    let path =
        std::env::temp_dir().join(format!("glint-api-search-{}.sqlite3", uuid::Uuid::new_v4()));
    let host = cx.new(|cx| {
        let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
        host.response_data = Some(HttpResponse {
            status: 200,
            headers: vec![],
            text: "Rust 中文\nRUST 中文".into(),
            bytes: 25,
            json: None,
        });
        host.update_response(cx);
        host
    });
    let (panel, window) = cx.add_window_view(|_, _| ResponsePanel(host.clone(), 850.));
    let original_code_bounds = window.debug_bounds("response-scroll").unwrap();
    let toggle = window.debug_bounds("response-search-toggle").unwrap();
    window.simulate_click(toggle.center(), gpui::Modifiers::default());
    let toolbar = window.debug_bounds("response-body-toolbar").unwrap();
    let search = window.debug_bounds("response-search-bar").unwrap();
    assert!(search.top() >= toolbar.top() && search.bottom() <= toolbar.bottom());
    assert!(search.right() <= toolbar.right());
    assert_eq!(
        window.debug_bounds("response-scroll").unwrap(),
        original_code_bounds,
        "opening search must not move or shrink the code viewport"
    );
    panel.update(window, |panel, cx| {
        panel.1 = 480.;
        cx.notify();
    });
    let toolbar = window.debug_bounds("response-body-toolbar").unwrap();
    let search = window.debug_bounds("response-search-bar").unwrap();
    assert!(
        search.right() <= toolbar.right(),
        "search must fit a narrow toolbar"
    );
    assert!(
        window
            .debug_bounds("response-search-close")
            .unwrap()
            .right()
            <= toolbar.right()
    );
    let focus = host.update(window, |host, cx| {
        assert!(host.show_response_search);
        host.response_search.focus_handle(cx)
    });
    window.update(|window, _| assert!(focus.is_focused(window)));
    window.simulate_input("rust");
    host.update(window, |host, _| {
        assert_eq!(host.response_search_count, 2);
        assert_eq!(host.response_search_index, Some(0));
    });
    window.simulate_keystrokes("enter");
    host.update(window, |host, _| {
        assert_eq!(host.response_search_index, Some(1))
    });
    window.simulate_keystrokes("enter");
    host.update(window, |host, _| {
        assert_eq!(host.response_search_index, Some(0))
    });
    window.simulate_keystrokes("shift-enter");
    host.update(window, |host, cx| {
        assert_eq!(host.response_search_index, Some(1));
        host.response_data.as_mut().unwrap().text = "RUST 新响应".into();
        host.update_response(cx);
        assert_eq!(host.response_search_count, 1);
        assert_eq!(host.response_search_index, Some(0));
        host.response_search
            .update(cx, |entry, cx| entry.set_value("不存在", cx));
    });
    window.run_until_parked();
    host.update(window, |host, _| {
        assert_eq!(host.response_search_count, 0);
        assert_eq!(host.response_search_index, None);
    });
    window.simulate_keystrokes("escape");
    host.update(window, |host, cx| {
        assert!(!host.show_response_search);
        assert_eq!(host.response.read(cx).search_query(), "");
        assert_eq!(host.response_content(), "RUST 新响应");
    });
    drop(host);
    std::fs::remove_file(path).unwrap();
}

#[gpui::test]
fn many_response_headers_scroll_within_the_panel(cx: &mut gpui::TestAppContext) {
    cx.update(bind_text_entry_keys);
    struct ResponsePanel(Entity<Workbench>);
    impl Render for ResponsePanel {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(480.))
                .h(px(240.))
                .flex()
                .flex_col()
                .child(self.0.update(cx, |host, cx| host.response_panel(cx)))
                .child(div().h(px(25.)).flex_shrink_0().child("状态栏"))
        }
    }
    let path = std::env::temp_dir().join(format!(
        "glint-api-headers-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let host = cx.new(|cx| {
        let store = storage::Store::open(&path).unwrap();
        let mut host = Workbench::with_store(store, cx);
        host.add_request(cx);
        host.response_tab = 1;
        host.response_data = Some(HttpResponse {
            status: 200,
            headers: (0..60)
                .map(|index| {
                    (
                        "set-cookie".into(),
                        format!("cookie-{index}={}", "x".repeat(600)),
                    )
                })
                .collect(),
            text: String::new(),
            bytes: 0,
            json: None,
        });
        host.update_response(cx);
        host
    });
    let (_, window) = cx.add_window_view(|_, _| ResponsePanel(host.clone()));
    let scroll = host.update(window, |host, _| host.response_headers_scroll.clone());
    assert!(scroll.bounds().bottom() <= px(215.));
    assert!(scroll.bounds().size.width <= px(480.));
    // Every repeated header remains a row, well beyond the viewport.
    assert!(scroll.max_offset().y > px(1800.));
    let value = window.debug_bounds("response-header-0-1").unwrap();
    window.simulate_click(value.center(), gpui::Modifiers::default());
    window.simulate_keystrokes("cmd-a cmd-c");
    assert_eq!(
        window.read_from_clipboard().unwrap().text().unwrap(),
        format!("cookie-0={}", "x".repeat(600))
    );
    window.simulate_keystrokes("backspace");
    host.update(window, |host, cx| {
        assert_eq!(
            host.response_header_entries[0][1].read(cx).value(),
            format!("cookie-0={}", "x".repeat(600))
        )
    });
    let name = window.debug_bounds("response-header-0-0").unwrap();
    window.simulate_click(name.center(), gpui::Modifiers::default());
    window.simulate_keystrokes("cmd-a cmd-c");
    assert_eq!(
        window.read_from_clipboard().unwrap().text().unwrap(),
        "set-cookie"
    );
    window.simulate_event(gpui::ScrollWheelEvent {
        position: scroll.bounds().center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-200.))),
        ..Default::default()
    });
    assert!(scroll.offset().y < px(0.));
    std::fs::remove_file(path).unwrap();
}

#[gpui::test]
fn request_title_rename_saves_cancels_and_survives_restart(cx: &mut gpui::TestAppContext) {
    struct NamePanel {
        host: Entity<Workbench>,
        bounds: std::rc::Rc<std::cell::Cell<Bounds<gpui::Pixels>>>,
    }
    impl Render for NamePanel {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let bounds = self.bounds.clone();
            div()
                .w(px(500.))
                .h(px(100.))
                .child(self.host.update(cx, |host, cx| host.request_name_view(cx)))
                .on_children_prepainted(move |children, _, _| bounds.set(children[0]))
        }
    }
    let path = std::env::temp_dir().join(format!(
        "glint-api-rename-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let host = cx.new(|cx| {
        let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
        host.group_edit = Some(GroupEdit::CreateCollection);
        host.group_name
            .update(cx, |entry, cx| entry.set_value("My API", cx));
        host.save_group(cx);
        host.add_request(cx);
        host.add_request(cx);
        host.choose(0, cx);
        host
    });
    let bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::default()));
    let (_, window) = cx.add_window_view(|_, _| NamePanel {
        host: host.clone(),
        bounds: bounds.clone(),
    });
    let click_title = |window: &mut gpui::VisualTestContext| {
        let title = window.debug_bounds("edit-request-name").unwrap();
        window.simulate_click(title.center(), gpui::Modifiers::default());
    };
    click_title(window);
    host.update(window, |host, cx| {
        assert!(
            host.editing_request_name,
            "clicking the title must enter editing"
        );
        host.request_name
            .update(cx, |entry, cx| entry.set_value("  ", cx));
    });
    window.simulate_keystrokes("enter");
    host.update(window, |host, cx| {
        assert!(host.editing_request_name && host.request_name_error.is_some());
        assert_eq!(host.requests[0].name, "新建请求 1");
        host.request_name
            .update(cx, |entry, cx| entry.set_value("  获取用户 🦀  ", cx));
    });
    window.simulate_keystrokes("enter");
    host.update(window, |host, cx| {
        assert!(!host.editing_request_name);
        assert_eq!(host.requests[0].name, "获取用户 🦀");
        assert_eq!(host.requests[0].versions.len(), 1);
        host.persist(cx);
    });
    click_title(window);
    host.update(window, |host, cx| {
        host.request_name
            .update(cx, |entry, cx| entry.set_value("取消的名字", cx));
    });
    window.simulate_keystrokes("escape");
    host.update(window, |host, _| {
        assert!(!host.editing_request_name);
        assert_eq!(host.requests[0].name, "获取用户 🦀");
    });
    click_title(window);
    host.update(window, |host, cx| {
        host.request_name
            .update(cx, |entry, cx| entry.set_value("切换时未提交", cx));
        host.choose(1, cx);
        host.save_request_name(cx);
        assert_eq!(host.requests[1].name, "新建请求 2");
        host.persist(cx);
    });
    let saved = storage::Store::open(&path).unwrap().load().unwrap();
    assert_eq!(saved.requests[0].name, "获取用户 🦀");
    assert_eq!(saved.requests[1].name, "新建请求 2");
    let reopened = window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
    reopened.update(window, |host, _| {
        assert_eq!(host.requests[0].name, "获取用户 🦀")
    });
    click_title(window);
    host.update(window, |host, cx| {
        host.request_name
            .update(cx, |entry, cx| entry.set_value("删除时未提交", cx));
        host.delete_request(0, cx);
        host.save_request_name(cx);
        assert!(!host.editing_request_name);
        assert_eq!(host.requests[0].name, "新建请求 2");
    });
    std::fs::remove_file(path).unwrap();
}

#[gpui::test]
fn saving_only_versions_unique_request_parameters(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("versions.sqlite3");
    let host = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
    host.update(cx, |host, cx| {
        host.group_edit = Some(GroupEdit::CreateCollection);
        host.group_name
            .update(cx, |entry, cx| entry.set_value("API", cx));
        host.save_group(cx);
        host.add_request(cx);
        let original = host.requests[0].versions[0].draft.clone();
        host.save(cx);
        host.save(cx);
        assert_eq!(host.requests[0].versions.len(), 1);
        host.endpoint.update(cx, |entry, cx| {
            entry.set_text("https://example.test/changed", cx)
        });
        // Drafts are persisted automatically; the version baseline must stay separate.
        host.persist(cx);
        host.save(cx);
        assert_eq!(host.requests[0].versions.len(), 2);
        assert_eq!(host.requests[0].versions[0].label, "v2");
        host.save(cx);
        assert_eq!(host.requests[0].versions.len(), 2);
        host.load(&original, cx);
        host.save(cx);
        assert_eq!(host.requests[0].versions.len(), 2);
        assert_eq!(
            host.duplicate_version.as_ref().unwrap().version_id,
            host.requests[0].versions[1].id
        );
        // Editing and undoing back to the latest saved content is also unchanged.
        host.endpoint.update(cx, |entry, cx| {
            entry.set_text("https://example.test/temporary", cx)
        });
        host.load(&original, cx);
        host.save(cx);
        assert_eq!(host.requests[0].versions.len(), 2);
        host.persist(cx);
    });
    let reopened = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
    reopened.update(cx, |host, cx| {
        host.save(cx);
        assert_eq!(host.requests[0].versions.len(), 2);
        assert_eq!(host.current_versions[0], 1);
        assert!(host.duplicate_version.is_some());
    });
    drop(host);
    drop(reopened);
}

#[gpui::test]
fn edits_and_environment_save_survive_workbench_restart(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "glint-api-ui-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let store = storage::Store::open(&path).unwrap();
    let host = cx.new(|cx| Workbench::with_store(store, cx));
    host.update(cx, |host, cx| {
        assert!(host.requests.is_empty() && host.collections.is_empty());
        host.group_edit = Some(GroupEdit::CreateCollection);
        host.group_name
            .update(cx, |e, cx| e.set_value("My API", cx));
        host.save_group(cx);
        host.add_request(cx);
        host.endpoint
            .update(cx, |e, cx| e.set_text("https://example.test/edited", cx));
        host.group_edit = Some(GroupEdit::CreateEnvironment);
        host.group_name
            .update(cx, |e, cx| e.set_value("My environment", cx));
        host.save_group(cx);
        host.add_variable(cx);
        host.variable_rows[1][0]
            .name
            .update(cx, |e, cx| e.set_value("base_url", cx));
        host.variable_rows[1][0]
            .value
            .update(cx, |e, cx| e.set_value("https://example.test", cx));
        host.persist(cx);
        assert!(
            storage::Store::open(&path)
                .unwrap()
                .load()
                .unwrap()
                .variables[1]
                .variables
                .is_empty()
        );
        host.save_environment_variables(cx);
        cx.notify();
    });
    cx.run_until_parked();
    let serializations = host.update(cx, |host, _| host.storage.serializations);
    host.update(cx, |host, cx| {
        host.response_tab = 1;
        host.show_activity = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(
        host.update(cx, |host, _| host.storage.serializations),
        serializations,
        "UI tab changes must not serialize the workspace"
    );
    let saved = storage::Store::open(&path).unwrap().load().unwrap();
    assert_eq!(saved.requests[0].draft.url, "https://example.test/edited");
    assert_eq!(saved.environments[1], "My environment");
    assert_eq!(
        saved.variables[1].variables[0].value,
        "https://example.test"
    );
    let store = storage::Store::open(&path).unwrap();
    let reopened = cx.new(|cx| Workbench::with_store(store, cx));
    reopened.update(cx, |host, cx| {
        assert_eq!(
            host.endpoint.read(cx).value().text.as_ref(),
            "https://example.test/edited"
        );
        host.delete_request(0, cx);
    });
    cx.run_until_parked();
    assert!(
        storage::Store::open(&path)
            .unwrap()
            .load()
            .unwrap()
            .requests
            .is_empty()
    );
    drop(host);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}
