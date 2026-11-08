//! Edit a failed user turn in place and replace its failed continuation on send.
use super::*;
use base64::Engine as _;
use gpui_glint::components::{PastedImage, Tooltip};

pub(super) struct Edit {
    pub session: Uuid,
    pub index: usize,
    composer: Entity<Composer>,
    pub error: Option<String>,
    _subscription: Subscription,
}

impl AgentWorkspace {
    pub(super) fn is_editing_message(&self, index: usize) -> bool {
        self.message_edit
            .as_ref()
            .is_some_and(|edit| edit.session == self.history.active && edit.index == index)
    }

    fn start_message_edit(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.history.active();
        if self.session_busy(session.id)
            || session.channel_group_member
            || !session.can_retry_message(index)
        {
            return;
        }
        let message = &session.messages[index];
        let images = message
            .images
            .iter()
            .map(|image| {
                let format = gpui::ImageFormat::from_mime_type(&image.media_type)
                    .ok_or("无法识别原消息的图片格式")?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(image.data.as_bytes())
                    .map_err(|_| "无法读取原消息的图片")?;
                Ok::<_, &str>(PastedImage::new(format, bytes))
            })
            .collect::<Result<Vec<_>, _>>();
        let images = match images {
            Ok(images) => images,
            Err(error) => {
                self.provider_error = Some(error.into());
                cx.notify();
                return;
            }
        };
        let id = session.id;
        let text = message
            .display
            .as_ref()
            .map_or_else(|| message.text.clone(), |display| display.text.clone());
        let references = message
            .display
            .as_ref()
            .map(|display| display.attachments.clone())
            .unwrap_or_default();
        let composer = cx.new(|cx| {
            let mut composer = Composer::new("编辑这条消息…", cx)
                .theme(self.theme)
                .clear_on_submit(false);
            composer.set_draft(text, images, cx);
            composer.set_references(references, cx);
            composer
        });
        let subscription = cx.subscribe(
            &composer,
            |this, _, event: &ComposerEvent, cx| match event {
                ComposerEvent::Submitted(submission) => {
                    this.retry_edited_message(submission.clone(), cx)
                }
                ComposerEvent::AttachmentError(error) => {
                    if let Some(edit) = &mut this.message_edit {
                        edit.error = Some(error.to_string());
                    }
                    cx.notify();
                }
                _ => {}
            },
        );
        window.focus(&composer.focus_handle(cx), cx);
        self.message_edit = Some(Edit {
            session: id,
            index,
            composer,
            error: None,
            _subscription: subscription,
        });
        self.sync_composer_run(cx);
        cx.notify();
    }

    fn retry_edited_message(&mut self, submission: ComposerSubmission, cx: &mut Context<Self>) {
        let Some(edit) = self.message_edit.as_ref() else {
            return;
        };
        let (id, index) = (edit.session, edit.index);
        if id != self.history.active || self.session_busy(id) {
            return;
        }
        if let Some(edit) = &mut self.message_edit {
            edit.error = None;
        }
        if let Err(error) = self.try_send_replacing(
            id,
            submission,
            self.provider.clone(),
            false,
            Some(index),
            cx,
        ) {
            if let Some(edit) = &mut self.message_edit {
                edit.error = Some(error);
            }
        }
        cx.notify();
    }

    pub(super) fn message_retry_action(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Div> {
        let session = self.history.active();
        if self.session_busy(session.id)
            || session.channel_group_member
            || self.is_editing_message(index)
            || !session.can_retry_message(index)
        {
            return None;
        }
        let button = IconButton::new(Icon::Pencil, "编辑并重新发送")
            .plain()
            .theme(self.theme)
            .render()
            .id(("edit-failed-message", index))
            .debug_selector(move || format!("edit-failed-message-{index}"))
            .on_click(
                cx.listener(move |this, _, window, cx| this.start_message_edit(index, window, cx)),
            );
        Some(
            div()
                .flex()
                .items_center()
                .h(px(26.))
                .invisible()
                .group_hover(
                    SharedString::from(format!("message-actions-{index}")),
                    |style| style.visible(),
                )
                .child(
                    Tooltip::new(button, "编辑并重新发送")
                        .theme(self.theme)
                        .render(cx),
                ),
        )
    }

    pub(super) fn message_retry_editor(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Div> {
        let edit = self
            .message_edit
            .as_ref()
            .filter(|_| self.is_editing_message(index))?;
        let busy = self.session_busy(edit.session);
        let p = self.theme.palette();
        edit.composer.update(cx, |composer, cx| {
            composer.set_theme(self.theme, cx);
            composer.set_disabled(busy, cx);
        });
        Some(div().child(
            div()
                .id(("message-retry-editor", index))
                .on_mouse_down_out(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if event.button == gpui::MouseButton::Left && !this.session_busy(this.history.active) {
                        this.message_edit = None;
                        this.sync_composer_run(cx);
                        cx.notify();
                    }
                }))
                .debug_selector(move || format!("message-retry-editor-{index}"))
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(p.border))
                .overflow_hidden()
                .child(edit.composer.clone())
                .when_some(edit.error.clone(), |panel, error| {
                    panel.child(div().px_3().py_2().text_size(px(12.)).child(error))
                }),
        ))
    }
}
