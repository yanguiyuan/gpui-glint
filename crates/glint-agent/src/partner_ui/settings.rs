//! Native companion settings view and interactions.
use super::*;

impl AgentWorkspace {
    pub(super) fn partner_settings(
        &self,
        partner: &partners::Partner,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.theme.palette();
        let id = partner.id;
        let dirty = self.partners_ui.name.read(cx).value() != partner.name
            || self.partners_ui.avatar.read(cx).value() != partner.avatar
            || self.partners_ui.description.read(cx).value() != partner.description;
        div()
            .max_w(px(600.))
            .w_full()
            .flex()
            .flex_col()
            .gap_6()
            .debug_selector(|| "partner-settings-page".into())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.partner_section("伙伴设定"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.text_muted))
                            .child("只定义你想定义的部分，其余留给相处。"),
                    ),
            )
            .child(self.partner_avatar_settings(partner, cx))
            .child(
                InputField::new("名字可以以后再起")
                    .label("名字")
                    .control(self.partners_ui.name.clone())
                    .theme(self.theme)
                    .render()
                    .w_full()
                    .debug_selector(|| "partner-name-field".into()),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("也可以保留“未命名伙伴”，等伙伴提出自己的名字。"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("初始设定"),
                    )
                    .child(
                        div()
                            .id("partner-description-editor")
                            .debug_selector(|| "partner-description-editor".into())
                            .h(px(150.))
                            .overflow_y_scroll()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(rgb(p.border))
                            .bg(rgb(p.panel))
                            .p_3()
                            .child(self.partners_ui.description.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child("可以留空。明确设定会优先保留，成长不会改变你的授权范围。"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("保存设定")
                            .when(!dirty, |button| button.disabled())
                            .theme(self.theme)
                            .render()
                            .id("save-partner-settings")
                            .debug_selector(|| "save-partner-settings".into())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let result = this.partner_repo().update(
                                    id,
                                    this.partners_ui.name.read(cx).value(),
                                    this.partners_ui.avatar.read(cx).value(),
                                    this.partners_ui.description.read(cx).value(),
                                );
                                if result.is_ok() {
                                    this.sync_skill_context(cx);
                                }
                                this.partners_ui.notice =
                                    Some(result.err().unwrap_or_else(|| "设定已保存".into()));
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(if dirty {
                                "有未保存的更改"
                            } else {
                                "设定已保存"
                            }),
                    ),
            )
            .into_any_element()
    }
}
