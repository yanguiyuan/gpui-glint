//! Native companion avatar view and interactions.
use super::*;

impl AgentWorkspace {
    pub(super) fn pick_partner_avatar(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择伙伴头像".into()),
        });
        let repo = self.partner_repo();
        self.partners_ui.avatar_task = Some(cx.spawn(async move |weak, cx| {
            let result = match picker.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                    let path = paths[0].clone();
                    cx.background_executor()
                        .spawn(async move { repo.import_avatar(id, &path) })
                        .await
                        .map(Some)
                }
                Ok(Ok(_)) => Ok(None),
                _ => Err("无法打开图片选择器".into()),
            };
            let _ = weak.update(cx, |this, cx| {
                this.partners_ui.avatar_task = None;
                if this.partners_ui.selected != Some(id) {
                    return;
                }
                match result {
                    Ok(Some(reference)) => {
                        this.partners_ui
                            .avatar
                            .update(cx, |e, cx| e.set_value(reference, cx));
                        this.partners_ui.notice = None;
                    }
                    Ok(None) => {}
                    Err(error) => this.partners_ui.notice = Some(error),
                }
                cx.notify();
            });
        }));
    }
    pub(super) fn partner_avatar_settings(
        &self,
        partner: &partners::Partner,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let p = self.theme.palette();
        let id = partner.id;
        let mut draft = partner.clone();
        draft.avatar = self.partners_ui.avatar.read(cx).value().to_owned();
        let reference = partners::is_avatar_reference(&draft.avatar);
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child("头像"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .debug_selector(|| "partner-avatar-preview".into())
                            .child(self.partner_avatar(&draft, 72.)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new(if self.partners_ui.avatar_task.is_some() {
                                            "正在读取…"
                                        } else if reference {
                                            "更换图片"
                                        } else {
                                            "选择本地图片"
                                        })
                                        .kind(ButtonKind::Gray)
                                        .size(ButtonSize::Small)
                                        .theme(self.theme)
                                        .when(self.partners_ui.avatar_task.is_some(), |b| {
                                            b.disabled()
                                        })
                                        .render()
                                        .id("choose-partner-avatar")
                                        .debug_selector(|| "choose-partner-avatar".into())
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                if this.partners_ui.avatar_task.is_none() {
                                                    this.pick_partner_avatar(id, cx);
                                                }
                                            }),
                                        ),
                                    )
                                    .child(
                                        Button::new("恢复默认")
                                            .kind(ButtonKind::Plain)
                                            .size(ButtonSize::Small)
                                            .theme(self.theme)
                                            .render()
                                            .id("reset-partner-avatar")
                                            .debug_selector(|| "reset-partner-avatar".into())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.partners_ui
                                                    .avatar
                                                    .update(cx, |e, cx| e.set_value("◌", cx));
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child("PNG、JPEG、WebP · 最多 20MB · 图片会复制到本地"),
                            ),
                    ),
            )
            .when(!reference, |d| {
                d.child(
                    InputField::new("◌")
                        .label("也可以使用头像符号")
                        .width(160.)
                        .control(self.partners_ui.avatar.clone())
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "partner-avatar-field".into()),
                )
            })
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child(format!(
                        "存放目录：{}",
                        self.partner_repo().avatar_directory().display()
                    )),
            )
    }
}
