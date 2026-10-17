use super::*;

impl Workbench {
    pub(super) fn save_version_tag(&mut self, cx: &mut Context<Self>) {
        let Some((request, label)) = self.version_tag_edit.take() else {
            return;
        };
        let tag = self.version_tag_input.read(cx).value().trim().to_owned();
        if let Some(record) = self
            .requests
            .get_mut(request)
            .and_then(|r| r.versions.iter_mut().find(|r| r.label == label))
        {
            if record.tag != tag {
                record.tag = tag;
                self.storage_dirty = true;
                self.persist(cx);
            }
        }
        cx.notify();
    }
    pub(super) fn version_tag_popover(
        &self,
        index: usize,
        record: &Record,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let target = (self.active, record.label.clone());
        let open = self.version_tag_edit.as_ref() == Some(&target);
        let p = self.theme.palette();
        let hover_group = gpui::SharedString::from(format!("version-record-{index}"));
        let trigger = div()
            .debug_selector(move || format!("version-tag-{index}").into())
            .h(px(22.))
            .max_w(px(100.))
            .px_1()
            .flex()
            .items_center()
            .gap_1()
            .rounded_md()
            .text_size(px(10.))
            .text_color(rgb(if record.tag.is_empty() {
                p.text_muted
            } else {
                p.accent
            }))
            .when(!record.tag.is_empty(), |s| s.bg(rgb(p.accent_tint)))
            .when(record.tag.is_empty() && !open, |s| {
                s.opacity(0.)
                    .group_hover(hover_group.clone(), |s| s.opacity(1.))
            })
            .child(Icon::Pencil.render(11.).flex_shrink_0().when(!open, |s| {
                s.opacity(0.).group_hover(hover_group, |s| s.opacity(1.))
            }))
            .child(div().min_w_0().truncate().child(if record.tag.is_empty() {
                "标签".to_owned()
            } else {
                record.tag.clone()
            }));
        let content = div()
            .debug_selector(|| "version-tag-editor".into())
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.version_tag_edit.is_some() && event.keystroke.key == "enter" {
                    this.save_version_tag(cx);
                    window.focus(&this.focus, cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("{} · 自定义标签", record.label)),
            )
            .child(
                div()
                    .h(px(30.))
                    .w_full()
                    .child(self.version_tag_input.clone()),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("留空保存可移除标签，版本号保持不变。"),
            )
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
                            .debug_selector(|| "cancel-version-tag".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.version_tag_edit = None;
                                window.focus(&this.focus, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("保存")
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "save-version-tag".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_version_tag(cx);
                                window.focus(&this.focus, cx);
                            })),
                    ),
            );
        let owner = cx.entity().downgrade();
        Popover::new(format!("version-tag-{}", record.label), trigger, content)
            .theme(self.theme)
            .width(280.)
            .trigger_height(22.)
            .open(open)
            .on_open_change(move |open, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    if open {
                        let tag = this
                            .requests
                            .get(target.0)
                            .and_then(|r| r.versions.iter().find(|r| r.label == target.1))
                            .map(|r| r.tag.clone())
                            .unwrap_or_default();
                        this.version_tag_input
                            .update(cx, |input, cx| input.set_value(tag, cx));
                        this.version_tag_edit = Some(target.clone());
                        window.focus(&this.version_tag_input.read(cx).focus_handle(cx), cx);
                    } else {
                        this.version_tag_edit = None;
                        window.focus(&this.focus, cx);
                    }
                    cx.notify();
                });
            })
            .render()
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(window: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = window.debug_bounds(selector).unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
    }
    #[gpui::test]
    fn version_tags_edit_cancel_clear_and_persist_without_restoring_versions(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.endpoint.update(cx, |entry, cx| {
                entry.set_text("https://example.test/latest", cx)
            });
            host.save(cx);
            host.activity = 0;
            host.show_activity = true;
            host
        });
        let latest = host.read_with(window, |h, cx| h.snapshot(cx));
        click(window, "version-tag-1");
        assert!(window.debug_bounds("version-tag-editor").is_some());
        host.update(window, |h, cx| {
            assert_eq!(h.current_versions[0], 0);
            assert_eq!(h.snapshot(cx), latest);
            h.version_tag_input
                .update(cx, |e, cx| e.set_value("  联调通过 ✅  ", cx));
        });
        window.simulate_keystrokes("enter");
        host.update(window, |h, cx| {
            assert!(h.version_tag_edit.is_none());
            assert_eq!(h.requests[0].versions.len(), 2);
            assert_eq!(h.requests[0].versions[1].label, "v1");
            assert_eq!(h.requests[0].versions[1].tag, "联调通过 ✅");
            assert_eq!(h.snapshot(cx), latest);
            assert_eq!(h.current_versions[0], 0);
        });
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert_eq!(saved.requests[0].versions[1].tag, "联调通过 ✅");
        let mut legacy = serde_json::to_value(&saved.requests[0].versions[1]).unwrap();
        legacy.as_object_mut().unwrap().remove("tag");
        assert!(
            serde_json::from_value::<Record>(legacy)
                .unwrap()
                .tag
                .is_empty()
        );
        for dismissal in ["cancel", "escape", "outside"] {
            click(window, "version-tag-1");
            host.update(window, |h, cx| {
                h.version_tag_input
                    .update(cx, |e, cx| e.set_value("discard", cx))
            });
            match dismissal {
                "cancel" => click(window, "cancel-version-tag"),
                "escape" => window.simulate_keystrokes("escape"),
                _ => window.simulate_click(point(px(400.), px(350.)), gpui::Modifiers::default()),
            }
            host.update(window, |h, cx| {
                assert!(h.version_tag_edit.is_none());
                assert_eq!(h.requests[0].versions[1].tag, "联调通过 ✅");
                assert_eq!(h.snapshot(cx), latest);
            });
        }
        click(window, "version-tag-1");
        let long_tag = "已验证的发布基线".repeat(30);
        host.update(window, |h, cx| {
            h.version_tag_input
                .update(cx, |e, cx| e.set_value(long_tag.clone(), cx))
        });
        click(window, "save-version-tag");
        assert!(window.debug_bounds("version-tag-1").unwrap().size.width <= px(100.));
        host.update(window, |h, cx| {
            assert_eq!(h.requests[0].versions[1].tag, long_tag);
            assert_eq!(h.snapshot(cx), latest);
        });
        click(window, "version-tag-1");
        host.update(window, |h, cx| {
            h.version_tag_input
                .update(cx, |e, cx| e.set_value("   ", cx))
        });
        click(window, "save-version-tag");
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert!(saved.requests[0].versions[1].tag.is_empty());
        assert_eq!(saved.requests[0].versions.len(), 2);
        assert_eq!(saved.requests[0].draft, latest);
    }
}
