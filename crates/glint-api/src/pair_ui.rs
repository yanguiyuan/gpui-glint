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

impl Workbench {
    pub(super) fn pair(&mut self, pair: &Pair, cx: &mut Context<Self>) -> PairInput {
        let key = Self::entry(&pair.key, "键", cx);
        let value = Self::entry(&pair.value, "值", cx);
        let note = Self::entry(&pair.note, "说明", cx);
        for entry in [&key, &value, &note] {
            entry.update(cx, |entry, cx| entry.set_theme(self.theme, cx));
            let mut previous = entry.read(cx).value().to_string();
            self.row_subscriptions
                .push(cx.observe(entry, move |this, entry, cx| {
                    let current = entry.read(cx).value().to_string();
                    if current != previous {
                        previous = current;
                        this.sync_reference_inputs(cx);
                    }
                    cx.notify();
                }));
        }
        let resolver = self.reference_resolver(cx);
        for entry in [&key, &value] {
            entry.update(cx, |entry, cx| {
                entry.set_reference_resolver(Some(resolver.clone()), cx)
            });
        }
        PairInput {
            enabled: pair.enabled,
            key,
            value,
            note,
        }
    }

    pub(super) fn pair_rows_mut(&mut self, kind: usize) -> &mut Vec<PairInput> {
        match kind {
            1 => &mut self.headers,
            2 => &mut self.cookies,
            3 => &mut self.extractors,
            4 => &mut self.form,
            _ => &mut self.params,
        }
    }

    pub(super) fn sync_pair_tables(&mut self, cx: &mut Context<Self>) {
        if let Some((kind, id)) = self.pair_delete {
            if !self
                .pair_rows_mut(kind)
                .iter()
                .any(|row| row.key.entity_id() == id)
            {
                self.pair_delete = None;
            }
        }
        for kind in [0, 1, 2, 3, 4] {
            let rows = match kind {
                1 => &self.headers,
                2 => &self.cookies,
                3 => &self.extractors,
                4 => &self.form,
                _ => &self.params,
            };
            let theme = self.theme;
            let rows: Vec<_> = rows
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let owner = cx.entity().downgrade();
                    let enabled = row.enabled;
                    let check = TableCell::custom(move || {
                        let owner = owner.clone();
                        Checkbox::new("")
                            .theme(theme)
                            .checked(enabled)
                            .on_change(move |checked, _, cx| {
                                cx.stop_propagation();
                                let _ = owner.update(cx, |this, cx| {
                                    let rows = match kind {
                                        1 => &mut this.headers,
                                        2 => &mut this.cookies,
                                        3 => &mut this.extractors,
                                        4 => &mut this.form,
                                        _ => &mut this.params,
                                    };
                                    rows[i].enabled = checked;
                                    this.sync_pair_tables(cx);
                                    cx.notify();
                                });
                            })
                            .render()
                    });
                    let key = row.key.clone();
                    let value = row.value.clone();
                    let note = row.note.clone();
                    let workbench = cx.entity().downgrade();
                    let extractor_cell = (kind == 3)
                        .then(|| cx.new(|cx| json_picker::ExtractorCell::new(workbench, i, cx)));
                    let owner = cx.entity().downgrade();
                    let row_id = row.key.entity_id();
                    let open = self.pair_delete == Some((kind, row_id));
                    let label = row.key.read(cx).value().to_owned();
                    let delete = TableCell::custom(move || {
                        let toggle_owner = owner.clone();
                        let confirm_owner = owner.clone();
                        pair_ui::delete_popover(
                            format!("pair-delete-{kind}-{i}"),
                            label.clone(),
                            open,
                            theme,
                            move |open, window, cx| {
                                let _ = toggle_owner.update(cx, |this, cx| {
                                    this.pair_delete = open.then_some((kind, row_id));
                                    window.focus(&this.focus, cx);
                                    this.sync_pair_tables(cx);
                                    cx.notify();
                                });
                            },
                            move |_, _, cx| {
                                let _ = confirm_owner.update(cx, |this, cx| {
                                    if this.pair_delete != Some((kind, row_id)) {
                                        return;
                                    }
                                    let rows = this.pair_rows_mut(kind);
                                    if let Some(index) =
                                        rows.iter().position(|row| row.key.entity_id() == row_id)
                                    {
                                        rows.remove(index);
                                    }
                                    this.pair_delete = None;
                                    if kind == 3 {
                                        this.cancel_json_pick(cx);
                                    }
                                    this.sync_pair_tables(cx);
                                    cx.notify();
                                });
                            },
                        )
                    });
                    TableRow::new(
                        i.to_string(),
                        [
                            check,
                            TableCell::custom(move || {
                                div()
                                    .w_full()
                                    .h(px(26.))
                                    .flex()
                                    .items_center()
                                    .font_family("Menlo")
                                    .child(key.clone())
                            }),
                            TableCell::custom(move || {
                                if let Some(cell) = &extractor_cell {
                                    return cell.clone().into_any_element();
                                }
                                div()
                                    .w_full()
                                    .h(px(26.))
                                    .flex()
                                    .items_center()
                                    .font_family("Menlo")
                                    .child(value.clone())
                                    .into_any_element()
                            }),
                            TableCell::custom(move || {
                                div()
                                    .w_full()
                                    .h(px(26.))
                                    .flex()
                                    .items_center()
                                    .child(note.clone())
                            }),
                            delete,
                        ],
                    )
                    .focus_handle(row.key.read(cx).focus_handle(cx))
                })
                .collect();
            let table = match kind {
                1 => &self.headers_table,
                2 => &self.cookies_table,
                3 => &self.extractors_table,
                4 => &self.form_table,
                _ => &self.params_table,
            };
            table.update(cx, |table, cx| {
                table.set_theme(theme, cx);
                table.set_rows(rows, cx);
            });
        }
        self.sync_reference_inputs(cx);
    }

    pub(super) fn pair_table(&self, headers: bool) -> gpui::Div {
        let p = self.theme.palette();
        let table = div().child(if headers {
            self.headers_table.clone()
        } else {
            self.params_table.clone()
        });
        table.child(
            div()
                .flex()
                .items_center()
                .pt_3()
                .text_size(px(10.))
                .text_color(rgb(p.text_muted))
                .child(if headers {
                    "Authorization 由身份验证配置生成。"
                } else {
                    "发送时仅包含已勾选的参数。"
                }),
        )
    }
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
