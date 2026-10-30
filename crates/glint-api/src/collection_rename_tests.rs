use super::*;
fn click(window: &mut gpui::VisualTestContext, selector: &'static str) {
    let bounds = window.debug_bounds(selector).unwrap();
    window.simulate_click(bounds.center(), gpui::Modifiers::default());
}
fn open_menu(window: &mut gpui::VisualTestContext) {
    let root = window.debug_bounds("request-directory:0:").unwrap();
    window.simulate_event(gpui::MouseMoveEvent {
        position: root.center(),
        ..Default::default()
    });
    click(window, "directory-menu:0:");
    click(window, "directory-menu-action:0::2");
}
#[gpui::test]
fn collection_rename_menu_validates_preserves_requests_defaults_and_reopens(
    cx: &mut gpui::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace.sqlite3");
    let (host, window) = cx.add_window_view(|_, cx| {
        let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
        host.workspace
            .collections
            .push(RequestGroups::create_collection("Original", &[]).unwrap());
        host.workspace.collections[0]
            .add_child("", "Folder")
            .unwrap();
        host.workspace.collections[0].defaults.insert(
            String::new(),
            model::DirectoryDefaults {
                sections: [model::DefaultSection::Endpoint].into(),
                draft: Draft {
                    url: "https://example.test/api".into(),
                    ..Draft::default()
                },
            },
        );
        host.add_request_to("Folder".into(), cx);
        host.save(cx);
        host.workspace
            .collections
            .push(RequestGroups::create_collection("Other", &host.workspace.collections).unwrap());
        host.current_collection = 1;
        host.current_directory.clear();
        host.add_request(cx);
        host.choose(0, cx);
        host.body.update(cx, |editor, cx| {
            editor.set_content("unsaved body survives", cx)
        });
        host
    });
    open_menu(window);
    assert!(window.debug_bounds("group-name-editor").is_some());
    host.update(window, |host, cx| {
        assert_eq!(host.group_name.read(cx).value(), "Original");
        assert_eq!(host.current_directory, "Folder");
        for invalid in ["   ", "Other"] {
            host.group_name
                .update(cx, |entry, cx| entry.set_value(invalid, cx));
            host.save_group(cx);
            assert!(host.group_error.is_some());
            assert_eq!(host.workspace.collections[0].collection, "Original");
            assert_eq!(host.workspace.requests[0].collection, "Original");
        }
        host.group_name
            .update(cx, |entry, cx| entry.set_value("  Renamed  ", cx));
    });
    click(window, "group-name-save");
    host.update(window, |host, cx| {
        assert!(host.group_edit.is_none());
        assert!(host.group_error.is_none());
        assert_eq!(host.workspace.collections[0].collection, "Renamed");
        assert_eq!(host.workspace.requests[0].collection, "Renamed");
        assert_eq!(host.workspace.requests[1].collection, "Other");
        assert_eq!(host.workspace.requests[0].group, "Folder");
        assert_eq!(host.current_directory, "Folder");
        assert_eq!(host.active, 0);
        assert_eq!(host.snapshot(cx).body, "unsaved body survives");
        assert!(!host.workspace.requests[0].versions.is_empty());
        assert_eq!(
            host.workspace.collections[0].request_defaults("Folder").url,
            "https://example.test/api"
        );
        host.persist(cx);
    });
    open_menu(window);
    host.update(window, |host, cx| {
        host.group_name
            .update(cx, |entry, cx| entry.set_value("Cancelled", cx))
    });
    click(window, "group-name-cancel");
    host.update(window, |host, cx| {
        assert!(host.group_edit.is_none());
        assert_eq!(host.workspace.collections[0].collection, "Renamed");
        host.persist(cx);
    });
    let workspace = storage::Store::open(&path).unwrap().load().unwrap();
    assert_eq!(workspace.collections[0].collection, "Renamed");
    assert_eq!(workspace.requests[0].collection, "Renamed");
    assert_eq!(workspace.requests[0].draft.body, "unsaved body survives");
    assert_eq!(workspace.requests[1].collection, "Other");
    assert_eq!(
        workspace.collections[0].request_defaults("Folder").url,
        "https://example.test/api"
    );
}
