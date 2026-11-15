use gpui::{
    Context, Entity, Focusable, IntoElement, ParentElement, Render, Styled, Window, div,
    prelude::*, px, rgb,
};
use gpui_glint::{
    components::{Form, FormActions, FormField, FormSection, FormSubmit, TextEntry},
    theme::ThemePreset,
};

pub struct FormsPage {
    theme: ThemePreset,
    name: Entity<TextEntry>,
    address: Entity<TextEntry>,
    error: Option<String>,
    submitted: bool,
}
impl FormsPage {
    pub fn new(theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| TextEntry::new(cx, "连接名称", "").theme(theme));
        let address = cx.new(|cx| TextEntry::new(cx, "https://example.com", "").theme(theme));
        for entry in [&name, &address] {
            cx.observe(entry, |_, _, cx| cx.notify()).detach();
        }
        Self {
            theme,
            name,
            address,
            error: None,
            submitted: false,
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        for entry in [&self.name, &self.address] {
            entry.update(cx, |entry, cx| entry.set_theme(theme, cx));
        }
        cx.notify();
    }
}
impl Render for FormsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme.palette();
        let entries = [self.name.clone(), self.address.clone()];
        let mut form = Form::new()
            .submit_guard(move |_, cx| entries.iter().all(|entry| !entry.read(cx).is_composing()))
            .theme(self.theme)
            .submit_on_enter(true)
            .child(
                FormSection::new()
                    .theme(self.theme)
                    .title("连接设置")
                    .description("字段值与校验由宿主管理，提交不会发送网络请求。")
                    .child(
                        FormField::new("connection-name", "名称", self.name.clone())
                            .theme(self.theme)
                            .required(true)
                            .focused(self.name.read(cx).focus_handle(cx).is_focused(window))
                            .render(),
                    )
                    .child(
                        FormField::new("connection-address", "服务地址", self.address.clone())
                            .theme(self.theme)
                            .focused(self.address.read(cx).focus_handle(cx).is_focused(window))
                            .description("可留空，使用默认服务地址")
                            .render(),
                    )
                    .render(),
            )
            .actions(FormActions::new("保存设置"))
            .on_submit(cx.listener(|this, _: &FormSubmit, _, cx| {
                if this.name.read(cx).is_composing() || this.address.read(cx).is_composing() {
                    return;
                }
                this.error = if this.name.read(cx).value().trim().is_empty() {
                    Some("请填写连接名称".into())
                } else {
                    None
                };
                this.submitted = this.error.is_none();
                cx.notify();
            }));
        if let Some(error) = &self.error {
            form = form.error(error.clone());
        }
        if self.submitted {
            form = form.hint("示例提交成功");
        }
        div()
            .size_full()
            .p_5()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .max_w(px(420.))
                    .w_full()
                    .p_5()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette.border))
                    .bg(rgb(palette.panel))
                    .child(form.render()),
            )
    }
}
