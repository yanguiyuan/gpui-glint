#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app_icon;

use glint_mdeditor::{
    CloseDocument, MarkdownWorkspace, NewDocument, OpenDocument, QuitWorkspace, SaveDocument,
    bind_workspace_keys, open_external,
};
use gpui::{
    App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};
use gpui_glint::components::GlintAssets;

fn main() {
    let cli_path = std::env::args().nth(1);
    if cli_path
        .as_deref()
        .is_some_and(|arg| arg == "-h" || arg == "--help")
    {
        println!("用法：glint-mdeditor [Markdown 文件路径]");
        println!("  不带参数时打开本机文档库；带路径时直接打开该文件预览。");
        return;
    }
    let root = MarkdownWorkspace::default_library_root();
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(move |cx: &mut App| {
            app_icon::install();
            bind_workspace_keys(cx);
            cx.set_menus(vec![
                gpui::Menu {
                    disabled: false,
                    name: "Glint Markdown".into(),
                    items: vec![gpui::MenuItem::action("退出 Glint Markdown", QuitWorkspace)],
                },
                gpui::Menu {
                    disabled: false,
                    name: "文件".into(),
                    items: vec![
                        gpui::MenuItem::action("新建文档", NewDocument),
                        gpui::MenuItem::action("打开 Markdown…", OpenDocument),
                        gpui::MenuItem::action("保存", SaveDocument),
                        gpui::MenuItem::action("关闭文档", CloseDocument),
                    ],
                },
            ]);
            cx.bind_keys([
                gpui::KeyBinding::new("cmd-q", QuitWorkspace, None),
                gpui::KeyBinding::new("ctrl-q", QuitWorkspace, None),
            ]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(16.), px(14.))),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |window, cx| {
                    let workspace = cx.new(|cx| MarkdownWorkspace::new(root, cx));
                    if let Some(path) = cli_path.clone() {
                        open_external(&workspace, path.into(), cx);
                    }
                    let focus = workspace.read(cx).focus.clone();
                    window.focus(&focus, cx);
                    let quit_workspace = workspace.downgrade();
                    cx.on_action(move |_: &QuitWorkspace, cx| {
                        let saved = quit_workspace
                            .update(cx, |this, cx| this.flush(cx))
                            .unwrap_or(true);
                        if saved {
                            cx.quit();
                        }
                    });
                    let weak = workspace.downgrade();
                    window.on_window_should_close(cx, move |_, cx| {
                        weak.update(cx, |this, cx| this.flush(cx)).unwrap_or(true)
                    });
                    workspace
                },
            )
            .expect("无法打开编辑器窗口");
            cx.activate(true);
        });
}
