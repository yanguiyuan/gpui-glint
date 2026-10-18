use super::*;

/// Shared confirmation for request rows and directory defaults.
pub(super) fn delete_popover(
    selector: String,
    label: String,
    open: bool,
    theme: ThemePreset,
    on_change: impl Fn(bool, &mut Window, &mut App) + Clone + 'static,
    on_confirm: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> Popover {
    let cancel = on_change.clone();
    let trigger_selector = selector.clone();
    let cancel_selector = format!("{selector}-cancel");
    let confirm_selector = format!("{selector}-confirm");
    let content = div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(if label.trim().is_empty() {
                    "删除此空行？".into()
                } else {
                    format!("删除“{label}”这一行？")
                }),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("取消")
                        .kind(ButtonKind::Gray)
                        .size(ButtonSize::Mini)
                        .theme(theme)
                        .render()
                        .debug_selector(move || cancel_selector.clone())
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            cancel(false, window, cx);
                        }),
                )
                .child(
                    Button::new("确认删除")
                        .kind(ButtonKind::Danger)
                        .size(ButtonSize::Mini)
                        .theme(theme)
                        .render()
                        .debug_selector(move || confirm_selector.clone())
                        .on_click(move |event, window, cx| {
                            cx.stop_propagation();
                            on_confirm(event, window, cx);
                        }),
                ),
        );
    Popover::new(
        format!("{selector}-popover"),
        IconButton::new(Icon::Close, "删除行")
            .plain()
            .theme(theme)
            .render()
            .size(px(22.))
            .debug_selector(move || trigger_selector.clone()),
        content,
    )
    .theme(theme)
    .width(260.)
    .trigger_height(22.)
    .padding(12.)
    .open(open)
    .on_open_change(on_change)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn click(window: &mut gpui::VisualTestContext, selector: String) {
        let selector = Box::leak(selector.into_boxed_str());
        let bounds = window
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
    }
    #[gpui::test]
    fn row_delete_requires_confirmation_and_tracks_row_identity(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&dir.path().join("test.sqlite3")).unwrap(),
                cx,
            );
            host.collections
                .push(RequestGroups::create_collection("API", &[]).unwrap());
            host.add_request(cx);
            host
        });
        for kind in 0..5 {
            let (first, second) = host.update(window, |host, cx| {
                let one = if kind == 3 {
                    host.extractor_input(&Pair::new("same", "$.one", ""), cx)
                } else {
                    host.pair(&Pair::new("same", "one", ""), cx)
                };
                let two = if kind == 3 {
                    host.extractor_input(&Pair::new("same", "$.two", ""), cx)
                } else {
                    host.pair(&Pair::new("same", "two", ""), cx)
                };
                let ids = (one.key.entity_id(), two.key.entity_id());
                *host.pair_rows_mut(kind) = vec![one, two];
                host.config = match kind {
                    1 | 2 => 2,
                    3 => 6,
                    4 => 3,
                    _ => 0,
                };
                host.body_format = BodyFormat::UrlEncoded;
                host.sync_pair_tables(cx);
                cx.notify();
                ids
            });
            let scroll = window.debug_bounds("request-config-scroll").unwrap();
            window.simulate_event(gpui::ScrollWheelEvent {
                position: scroll.center(),
                delta: gpui::ScrollDelta::Pixels(gpui::point(
                    px(0.),
                    px(if kind == 2 { -1000. } else { 1000. }),
                )),
                ..Default::default()
            });
            click(window, format!("pair-delete-{kind}-0"));
            host.update(window, |host, _| {
                assert_eq!(host.pair_rows_mut(kind).len(), 2)
            });
            click(window, format!("pair-delete-{kind}-0-cancel"));
            host.update(window, |host, _| {
                assert!(host.pair_delete.is_none());
                assert_eq!(host.pair_rows_mut(kind).len(), 2);
            });
            click(window, format!("pair-delete-{kind}-0"));
            window.simulate_keystrokes("escape");
            host.update(window, |host, _| {
                assert!(host.pair_delete.is_none());
                assert_eq!(host.pair_rows_mut(kind).len(), 2);
            });
            click(window, format!("pair-delete-{kind}-0"));
            window.simulate_click(gpui::point(px(100.), px(600.)), gpui::Modifiers::default());
            host.update(window, |host, _| {
                assert!(host.pair_delete.is_none());
                assert_eq!(host.pair_rows_mut(kind).len(), 2);
            });
            click(window, format!("pair-delete-{kind}-0"));
            // Even duplicate names and a reorder must not redirect a pending deletion.
            host.update(window, |host, cx| {
                assert_eq!(host.pair_delete, Some((kind, first)));
                host.pair_rows_mut(kind).reverse();
                host.sync_pair_tables(cx);
                cx.notify();
            });
            click(window, format!("pair-delete-{kind}-1-confirm"));
            host.update(window, |host, cx| {
                assert!(host.pair_delete.is_none());
                let rows = host.pair_rows_mut(kind);
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].key.entity_id(), second);
                host.persist(cx);
                assert!(host.storage_error.is_none());
            });
        }
    }
}
