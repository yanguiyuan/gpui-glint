#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
use glint_vault::VaultPage;
use gpui::{
    App, AppContext, Bounds, Context, Entity, FontWeight, Render, TitlebarOptions, Window,
    WindowBounds, WindowOptions, div, point, prelude::*, px, size,
};
use gpui_glint::{
    components::{
        AppLayout, GlintAssets, Icon, IconButton, bind_text_entry_keys, window_drag_region,
    },
    theme::ThemePreset,
};
struct Workspace {
    page: Entity<VaultPage>,
    theme: ThemePreset,
}
impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let topbar = div()
            .size_full()
            .flex()
            .items_center()
            .gap_3()
            .pl(px(if cfg!(target_os = "macos") { 80. } else { 16. }))
            .pr_3()
            .child(Icon::Key.render(18.))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Glint Vault"))
            .child(window_drag_region())
            .child(
                IconButton::new(Icon::HalfCircle, "切换明暗主题")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.theme = if this.theme.is_dark() {
                            ThemePreset::Moonstone
                        } else {
                            ThemePreset::DarkObsidian
                        };
                        this.page
                            .update(cx, |page, cx| page.set_host_state(true, this.theme, cx));
                        cx.notify();
                    })),
            );
        AppLayout::new(topbar, div(), self.page.clone())
            .sidebar_visible(false)
            .theme(self.theme)
    }
}
fn main() {
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| {
            bind_text_entry_keys(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1100.), px(760.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(760.), px(580.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Glint Vault".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(16.), px(14.))),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| Workspace {
                        theme: ThemePreset::Moonstone,
                        page: cx.new(|cx| {
                            VaultPage::new(glint_vault::default_path(), ThemePreset::Moonstone, cx)
                        }),
                    })
                },
            )
            .expect("无法打开密码箱窗口");
            cx.activate(true);
        });
}
