//! Companion grouping and recipient selection use the same persistent session identity.
use super::*;

pub fn selector_chip(partners: &[partners::Partner], active: Option<Uuid>) -> ComposerChip {
    let mut chip = ComposerChip::new("partner")
        .icon(Icon::User)
        .chevron()
        .option("none", "普通 Agent");
    for partner in partners {
        chip = chip.option(partner.id.to_string(), partner.name.clone());
    }
    chip.default_selected(
        active
            .filter(|id| partners.iter().any(|p| p.id == *id))
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".into()),
    )
}
pub fn context_section(
    partners: &[partners::Partner],
    active: Option<Uuid>,
) -> ComposerContextSection {
    let mut entries: Vec<_> = partners
        .iter()
        .map(|p| {
            ComposerContextEntry::new(format!("partner:{}", p.id), p.name.clone())
                .icon(Icon::User)
                .description(if active == Some(p.id) {
                    "当前会话伙伴".into()
                } else if p.description.is_empty() {
                    "选择这位伙伴，带上草稿开始交流".into()
                } else {
                    preview_line(&p.description, 100)
                })
                .insert_mention(false)
        })
        .collect();
    entries.push(
        ComposerContextEntry::new("manage-partners", "管理伙伴")
            .icon(Icon::Settings)
            .description("创建伙伴、查看经历与设定")
            .insert_mention(false),
    );
    ComposerContextSection::new("伙伴", entries)
}
impl AgentWorkspace {
    pub(crate) fn select_partner_key(
        &mut self,
        key: &str,
        from_context: bool,
        cx: &mut Context<Self>,
    ) {
        let id = if key == "none" {
            None
        } else {
            match Uuid::parse_str(key) {
                Ok(id) => Some(id),
                Err(_) => {
                    self.provider_error = Some("伙伴 ID 无效".into());
                    self.sync_skill_context(cx);
                    cx.notify();
                    return;
                }
            }
        };
        if let Some(id) = id {
            match partners::Repository::new(self.path.clone()).list() {
                Ok(ps) if ps.iter().any(|p| p.id == id) => {}
                _ => {
                    self.provider_error = Some("这位伙伴已不可用，请重新选择".into());
                    self.sync_skill_context(cx);
                    cx.notify();
                    return;
                }
            }
        }
        if from_context {
            self.composer.update(cx, |c, cx| c.clear_context_query(cx));
        }
        self.capture_draft(cx);
        self.history.preferred_partner = id;
        if self.history.active().partner_id != id {
            let active = self.history.active;
            let in_flight = self
                .pending
                .as_ref()
                .or(self.preparing.as_ref())
                .is_some_and(|(sid, _)| *sid == active)
                || self.queued_requests.values().any(|(sid, _)| *sid == active);
            if self.history.active().messages.is_empty() && !in_flight {
                self.history.active_mut().partner_id = id;
                self.sync_skill_context(cx);
            } else {
                let source = self.history.active_mut();
                let project = source.project.clone();
                let draft = std::mem::take(&mut source.draft);
                let images = std::mem::take(&mut source.draft_images);
                self.history.new_session_for(project);
                let target = self.history.active_mut();
                target.draft = draft;
                target.draft_images = images;
                self.restore_draft(cx);
                self.scroll = ScrollHandle::new();
            }
        } else {
            self.sync_skill_context(cx);
        }
        self.page = crate::Page::Chat;
        self.provider_error = None;
        self.save(cx);
        cx.notify();
    }
    pub(crate) fn partner_group_action(&mut self, key: &str, index: usize, cx: &mut Context<Self>) {
        if index == 0 {
            let id = if key == "none" {
                None
            } else {
                let Ok(id) = Uuid::parse_str(key) else {
                    return;
                };
                Some(id)
            };
            if let Some(id) = id {
                if !partners::Repository::new(self.path.clone())
                    .list()
                    .is_ok_and(|ps| ps.iter().any(|p| p.id == id))
                {
                    self.provider_error = Some("伙伴已不可用".into());
                    cx.notify();
                    return;
                }
            }
            self.capture_draft(cx);
            self.history.preferred_partner = id;
            self.history
                .new_session_for(self.history.active().project.clone());
            self.restore_draft(cx);
            self.page = crate::Page::Chat;
            self.scroll = ScrollHandle::new();
            self.save(cx);
            cx.notify();
        } else if index == 1 {
            let Ok(id) = Uuid::parse_str(key) else {
                return;
            };
            self.open_partner_settings(id, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(
        cx: &mut gpui::TestAppContext,
    ) -> (
        tempfile::TempDir,
        Entity<AgentWorkspace>,
        &mut gpui::VisualTestContext,
        Uuid,
        Uuid,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut h = AgentWorkspace::with_path(dir.path().join("sessions.json"), cx);
            let repo = partners::Repository::new(h.path.clone());
            let a = repo.create().unwrap();
            let b = repo.create().unwrap();
            repo.update(a, "知微", "◌", "").unwrap();
            repo.update(b, "林舟", "◌", "").unwrap();
            h.history.projects.push(dir.path().to_path_buf());
            h.history.active_mut().project = Some(dir.path().to_path_buf());
            h.sync_skill_context(cx);
            h
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let (a, b) = host.read_with(window, |h, _| {
            let ps = partners::Repository::new(h.path.clone()).list().unwrap();
            (
                ps.iter().find(|p| p.name == "知微").unwrap().id,
                ps.iter().find(|p| p.name == "林舟").unwrap().id,
            )
        });
        (dir, host, window, a, b)
    }
    fn select_mention(
        host: &Entity<AgentWorkspace>,
        window: &mut gpui::VisualTestContext,
        id: Uuid,
        text: &str,
    ) {
        host.update(window, |h, cx| {
            h.composer.update(cx, |c, cx| {
                c.set_value(text, cx);
                c.open_context_panel(cx);
            });
            h.sync_skill_context(cx);
            cx.notify();
        });
        window.run_until_parked();
        let selector = Box::leak(format!("composer-context:partner:{id}").into_boxed_str());
        let row = window.debug_bounds(selector).unwrap();
        window.simulate_click(row.center(), gpui::Modifiers::default());
        window.run_until_parked();
    }
    #[gpui::test]
    fn at_picker_binds_blank_chat_and_forks_existing_chat_without_reassigning_history(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_dir, host, window, a, b) = fixture(cx);
        let first = host.read_with(window, |h, _| h.history.active);
        select_mention(&host, window, a, "检查滚动 @知");
        host.read_with(window, |h, cx| {
            assert_eq!(h.history.active, first);
            assert_eq!(h.history.active().partner_id, Some(a));
            assert_eq!(h.composer.read(cx).value(cx).as_ref(), "检查滚动 ");
            assert!(!h.composer.read(cx).context_panel_open());
        });
        host.update(window, |h, _| {
            h.history
                .active_mut()
                .push(Role::User, "之前交给知微的任务".into());
            h.history.active_mut().push(Role::Agent, "已完成".into());
        });
        select_mention(&host, window, b, "继续检查 @林");
        host.read_with(window, |h, cx| {
            assert_ne!(h.history.active, first);
            assert_eq!(h.history.active().partner_id, Some(b));
            assert_eq!(h.history.preferred_partner, Some(b));
            assert_eq!(h.composer.read(cx).value(cx).as_ref(), "继续检查 ");
            let old = h.history.sessions.iter().find(|s| s.id == first).unwrap();
            assert_eq!(old.partner_id, Some(a));
            assert_eq!(old.messages.len(), 2);
            assert!(old.draft.is_empty());
            assert_eq!(old.project, h.history.active().project);
        });
    }
    #[gpui::test]
    fn partner_dropdown_changes_recipient_and_keeps_attachments(cx: &mut gpui::TestAppContext) {
        let (_dir, host, window, a, _b) = fixture(cx);
        host.update(window, |h, cx| {
            h.composer.update(cx, |c, cx| {
                c.set_draft(
                    "带上图片检查",
                    vec![gpui_glint::components::PastedImage::new(
                        gpui::ImageFormat::Png,
                        include_bytes!("../assets/glint-agent-icon.png").to_vec(),
                    )],
                    cx,
                )
            })
        });
        window.run_until_parked();
        let chip = window.debug_bounds("composer-chip:partner").unwrap();
        window.simulate_click(chip.center(), gpui::Modifiers::default());
        window.run_until_parked();
        let selector = Box::leak(format!("composer-option:partner:{a}").into_boxed_str());
        let option = window.debug_bounds(selector).unwrap();
        window.simulate_click(option.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, cx| {
            assert_eq!(h.history.active().partner_id, Some(a));
            assert_eq!(h.composer.read(cx).attachments().len(), 1);
            assert_eq!(h.composer.read(cx).value(cx).as_ref(), "带上图片检查");
        });
    }
    #[gpui::test]
    fn group_new_chat_uses_its_partner_and_preference_survives_storage(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_dir, host, window, a, b) = fixture(cx);
        host.update(window, |h, cx| {
            h.select_partner_key(&a.to_string(), false, cx);
            h.history.active_mut().push(Role::User, "已有会话".into());
            h.history.group_by_partner = true;
            h.group_list.update(cx, |_, cx| {
                cx.emit(GroupListEvent::HeaderAction {
                    group: format!("partner:{b}").into(),
                    index: 0,
                })
            });
        });
        window.run_until_parked();
        host.read_with(window, |h, _| {
            assert_eq!(h.page, crate::Page::Chat);
            assert_eq!(h.history.active().partner_id, Some(b));
            assert!(h.history.active().messages.is_empty());
            database::save_history(&h.path, &h.history).unwrap();
            let loaded = History::load(&h.path).unwrap();
            assert!(loaded.group_by_partner);
            assert_eq!(loaded.active().partner_id, Some(b));
        });
    }
    #[gpui::test]
    fn switching_conversations_restores_recipient_and_next_session_inherits_it(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_dir, host, window, a, b) = fixture(cx);
        host.update(window, |h, cx| {
            h.select_partner_key(&a.to_string(), false, cx);
            let original = h.history.active;
            h.history.active_mut().push(Role::User, "知微的历史".into());
            h.select_partner_key(&b.to_string(), false, cx);
            assert_eq!(h.history.preferred_partner, Some(b));
            h.switch(original, cx);
            assert_eq!(h.history.preferred_partner, Some(a));
            let chip = selector_chip(
                &partners::Repository::new(h.path.clone()).list().unwrap(),
                h.history.active().partner_id,
            );
            assert_eq!(chip.default_selected.unwrap().as_ref(), a.to_string());
            h.history
                .new_session_for(h.history.active().project.clone());
            assert_eq!(h.history.active().partner_id, Some(a));
        });
    }

    #[gpui::test]
    fn stale_partner_selection_does_not_consume_the_prompt(cx: &mut gpui::TestAppContext) {
        let (_dir, host, window, a, _) = fixture(cx);
        host.update(window, |h, cx| {
            partners::Repository::new(h.path.clone())
                .remove_partner(a)
                .unwrap();
            h.composer
                .update(cx, |c, cx| c.set_value("保留这段草稿 @知", cx));
            h.select_partner_key(&a.to_string(), true, cx);
            assert!(h.provider_error.as_ref().unwrap().contains("不可用"));
            assert_eq!(h.composer.read(cx).value(cx).as_ref(), "保留这段草稿 @知");
            assert!(h.history.active().partner_id.is_none());
        });
    }
}
