//! Native companion deletion view and interactions.
use super::*;

impl AgentWorkspace {
    pub(super) fn request_partner_delete(
        &mut self,
        id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.partners_ui.confirming_partner = Some(id);
        window.focus(&self.partners_ui.cancel_focus, cx);
        cx.notify();
    }
    pub(super) fn dismiss_partner_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.partners_ui.confirming_partner = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }
    pub(super) fn confirm_partner_delete(
        &mut self,
        id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.partners_ui.confirming_partner != Some(id) {
            return;
        }
        let busy = self
            .pending
            .as_ref()
            .or(self.preparing.as_ref())
            .is_some_and(|(sid, _)| {
                self.history
                    .sessions
                    .iter()
                    .any(|s| s.id == *sid && s.partner_id == Some(id))
            })
            || self
                .subagent_ui
                .live
                .lock()
                .map(|traces| {
                    traces
                        .iter()
                        .any(|t| t.partner_id == Some(id) && t.status.active())
                })
                .unwrap_or(true);
        let result = if busy {
            Err("伙伴正在执行任务，请先停止再删除".into())
        } else {
            self.partner_repo().remove_partner(id)
        };
        if let Err(error) = result {
            self.partners_ui.notice = Some(error);
            self.dismiss_partner_delete(window, cx);
            return;
        }
        for session in &mut self.history.sessions {
            if session.partner_id == Some(id) {
                self.recall_runner
                    .update(cx, |runner, _| runner.cancel(session.id));
                session.partner_id = None;
                session.partner_goal = None;
            }
        }
        if self.history.preferred_partner == Some(id) {
            self.history.preferred_partner = None;
        }
        self.partners_ui.selected = None;
        if let Ok(partners) = self.partner_repo().list() {
            if let Some(p) = partners.first() {
                self.select_partner(p, cx);
            }
        }
        if let Ok(mut store) = self.schedules.lock() {
            let _ = store.refresh();
        }
        self.partners_ui.notice = Some("伙伴已删除，历史会话已保留。".into());
        self.save(cx);
        self.dismiss_partner_delete(window, cx);
    }
    pub(super) fn partner_delete_dialog(
        &self,
        id: Uuid,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.theme.palette();
        let name = self
            .partner_repo()
            .list()
            .ok()
            .and_then(|ps| ps.into_iter().find(|p| p.id == id))
            .map(|p| p.name)
            .unwrap_or_else(|| "这个伙伴".into());
        let weak = cx.entity().downgrade();
        Dialog::new("partner-delete-dialog",div().flex().flex_col().gap_4()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(format!("删除“{name}”？")))
            .child(div().text_size(px(13.)).line_height(px(24.)).text_color(rgb(p.text_muted)).child("将永久删除这个伙伴的身份、个人记忆、成长记录和所属日程。历史会话会保留，并解除与伙伴的绑定。此操作无法撤销。"))
            .child(div().flex().justify_end().gap_3()
                .child(Button::new("取消").kind(ButtonKind::Gray).theme(self.theme).render().id("cancel-delete-partner").debug_selector(||"cancel-delete-partner".into()).track_focus(&self.partners_ui.cancel_focus)
                    .on_key_down(cx.listener(|this,event:&gpui::KeyDownEvent,window,cx|{if event.keystroke.key=="enter"{this.dismiss_partner_delete(window,cx);cx.stop_propagation();}}))
                    .on_click(cx.listener(|this,_,window,cx|this.dismiss_partner_delete(window,cx))))
                .child(Button::new("删除伙伴").kind(ButtonKind::Danger).theme(self.theme).render().id("confirm-delete-partner").debug_selector(||"confirm-delete-partner".into()).track_focus(&self.partners_ui.confirm_focus)
                    .on_key_down(cx.listener(move|this,event:&gpui::KeyDownEvent,window,cx|{if event.keystroke.key=="enter"{this.confirm_partner_delete(id,window,cx);cx.stop_propagation();}}))
                    .on_click(cx.listener(move|this,_,window,cx|this.confirm_partner_delete(id,window,cx))))))
            .theme(self.theme).width(440.).focus_handles([self.partners_ui.cancel_focus.clone(),self.partners_ui.confirm_focus.clone()])
            .on_close(move|_,window,cx|{let _=weak.update(cx,|this,cx|this.dismiss_partner_delete(window,cx));}).render()
    }
}
