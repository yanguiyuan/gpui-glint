mod pages;

use std::path::PathBuf;

use gpui::{
    App, AppContext, Bounds, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Subscription, TitlebarOptions, Window,
    WindowBounds, WindowOptions, div, point, px, rgb, rgba, size,
};
use gpui_glint::components::{
    AppLayout, DropdownGroup, DropdownItem, GlintAssets, GroupedDropdown, Icon, IconButton,
    ThemeSelectorEvent, Tooltip,
};
use gpui_glint::theme::ThemePreset;

use pages::{
    BadgesPage, ButtonsPage, CodeCardsPage, CommandsPage, DocumentsPage, IconsPage, InputsPage,
    LayoutPage, OverviewPage, TablesPage, TabsPage, ThemeShowcase,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Buttons,
    Icons,
    Layout,
    Inputs,
    Badges,
    CodeCards,
    Commands,
    Tables,
    Documents,
    Tabs,
    Themes,
}

impl Page {
    const ALL: [(Self, &'static str); 12] = [
        (Self::Overview, "概览"),
        (Self::Buttons, "按钮"),
        (Self::Icons, "图标"),
        (Self::Layout, "布局容器"),
        (Self::Inputs, "输入与选择"),
        (Self::Badges, "徽标与标签"),
        (Self::CodeCards, "文档代码卡"),
        (Self::Commands, "命令面板"),
        (Self::Tables, "表格"),
        (Self::Documents, "文档编辑器"),
        (Self::Tabs, "标签页"),
        (Self::Themes, "主题"),
    ];
}

struct Playground {
    selected: Page,
    theme: ThemePreset,
    resource_dropdown: Entity<GroupedDropdown>,
    overview: Entity<OverviewPage>,
    buttons: Entity<ButtonsPage>,
    icons: Entity<IconsPage>,
    layout: Entity<LayoutPage>,
    inputs: Entity<InputsPage>,
    badges: Entity<BadgesPage>,
    code_cards: Entity<CodeCardsPage>,
    commands: Entity<CommandsPage>,
    tables: Entity<TablesPage>,
    documents: Entity<DocumentsPage>,
    tabs: Entity<TabsPage>,
    _theme_subscription: Subscription,
    _commands_subscription: Subscription,
}

impl Playground {
    fn page_entity(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        match self.selected {
            Page::Overview => self.overview.clone().into_any_element(),
            Page::Buttons => self.buttons.clone().into_any_element(),
            Page::Icons => self.icons.clone().into_any_element(),
            Page::Layout => self.layout.clone().into_any_element(),
            Page::Inputs => self.inputs.clone().into_any_element(),
            Page::Badges => self.badges.clone().into_any_element(),
            Page::CodeCards => self.code_cards.clone().into_any_element(),
            Page::Commands => self.commands.clone().into_any_element(),
            Page::Tables => self.tables.clone().into_any_element(),
            Page::Documents => self.documents.clone().into_any_element(),
            Page::Tabs => self.tabs.clone().into_any_element(),
            Page::Themes => {
                let playground = cx.entity();
                ThemeShowcase::new(self.theme)
                    .on_select(move |theme, _, app| {
                        playground.update(app, |this, cx| this.set_theme(theme, cx));
                    })
                    .render()
                    .into_any_element()
            }
        }
    }

    fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.resource_dropdown
            .update(cx, |menu, cx| menu.set_theme(theme, cx));
        persist_theme(theme);
        self.overview.update(cx, |page, cx| {
            page.theme = theme;
            cx.notify();
        });
        self.buttons.update(cx, |page, cx| {
            page.theme = theme;
            cx.notify();
        });
        self.icons.update(cx, |page, cx| {
            page.theme = theme;
            cx.notify();
        });
        self.layout.update(cx, |page, cx| {
            page.theme = theme;
            cx.notify();
        });
        self.inputs.update(cx, |page, cx| page.set_theme(theme, cx));
        self.badges.update(cx, |page, cx| {
            page.theme = theme;
            cx.notify();
        });
        self.code_cards.update(cx, |page, cx| {
            page.theme = theme;
            cx.notify();
        });
        self.commands
            .update(cx, |page, cx| page.set_theme(theme, cx));
        self.tables.update(cx, |page, cx| page.set_theme(theme, cx));
        self.documents
            .update(cx, |page, cx| page.set_theme(theme, cx));
        self.tabs.update(cx, |page, cx| page.set_theme(theme, cx));
        cx.notify();
    }

    fn nav_item(
        &self,
        page: Page,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let active = self.selected == page;
        let dark = self.theme.is_dark();
        let palette = self.theme.palette();
        let (background, foreground) = if active {
            (palette.accent, 0xffffff)
        } else {
            (palette.surface_subtle, palette.text)
        };
        div()
            .id(label)
            .w_full()
            .px_3()
            .py(px(if dark { 5.5 } else { 8. }))
            .rounded(px(7.))
            .bg(if active || !dark {
                rgb(background)
            } else {
                rgba(0x00000000)
            })
            .text_color(rgb(foreground))
            .text_size(px(13.))
            .cursor_pointer()
            .hover(move |style| {
                if active {
                    style
                } else {
                    style.bg(rgba(if dark {
                        0xffffff14
                    } else {
                        (palette.border << 8) | 255
                    }))
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = page;
                this.inputs.update(cx, |inputs, cx| {
                    inputs.close_menu(cx);
                });
                this.resource_dropdown.update(cx, |dropdown, cx| {
                    dropdown.close();
                    cx.notify();
                });
                cx.notify();
            }))
            .child(label)
    }
}

impl Render for Playground {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let sidebar_items = Page::ALL
            .into_iter()
            .fold(div().flex().flex_col().gap_1(), |items, (page, label)| {
                items.child(self.nav_item(page, label, cx))
            });
        let sidebar = div().h_full().flex().child(
            div()
                .flex_1()
                .h_full()
                .flex()
                .flex_col()
                .p_4()
                .gap_2()
                .text_color(rgb(theme.color(0x48484a)))
                .child(
                    div()
                        .pb_2()
                        .text_size(px(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("组件目录"),
                )
                .child(sidebar_items),
        );

        let rail_icon = |icon: Icon| {
            div()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(theme.color(0xe7ebef))))
                .child(icon.render(17.))
        };
        let rail = div()
            .w(px(42.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .pt_3()
            .bg(if self.theme.is_dark() {
                rgba(0x00000000)
            } else {
                rgb(self.theme.palette().canvas)
            })
            .child(
                Tooltip::new(rail_icon(Icon::Folders), "工作区")
                    .theme(self.theme)
                    .render(cx),
            )
            .child(
                Tooltip::new(rail_icon(Icon::Database), "资源")
                    .theme(self.theme)
                    .render(cx),
            )
            .child(
                Tooltip::new(rail_icon(Icon::Workflow), "流程")
                    .theme(self.theme)
                    .render(cx),
            )
            .child(
                Tooltip::new(rail_icon(Icon::Settings), "设置")
                    .theme(self.theme)
                    .render(cx),
            );

        let topbar = div()
            .h_full()
            .flex()
            .items_center()
            .gap_4()
            .px_4()
            .text_color(rgb(self.theme.palette().text))
            .child(div().w(px(56.)))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(self.theme.palette().text))
                    .child("GLINT UI"),
            )
            .child(div().text_size(px(13.)).child("组件预览"))
            .child(div().flex_1())
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child("GPUI · 桌面组件"),
            )
            .child(IconButton::new(Icon::Settings, "设置").theme(self.theme))
            .child(self.resource_dropdown.clone());

        let content = div()
            .id("playground-page-content")
            .size_full()
            .overflow_hidden()
            .child(self.page_entity(cx));
        let layout = AppLayout::new(topbar, sidebar, content)
            .navigation_rail(rail)
            .navigation_rail_width(42.)
            .sidebar_width(if self.theme.is_dark() { 214. } else { 236. })
            .theme(self.theme)
            .render();

        div()
            .id("glint-ui-playground")
            .relative()
            .size_full()
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "k"
                    && (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
                {
                    this.commands.update(cx, |page, cx| page.open(window, cx));
                    cx.stop_propagation();
                    window.prevent_default();
                }
            }))
            .on_any_mouse_down(cx.listener(|this, _, _, cx| {
                this.resource_dropdown.update(cx, |dropdown, cx| {
                    dropdown.close();
                    cx.notify();
                });
            }))
            .child(layout)
            .children(self.commands.update(cx, |page, cx| page.overlay(cx)))
    }
}

pub fn open(cx: &mut App) {
    let resource_dropdown = cx.new(|_| {
        GroupedDropdown::new("资源")
            .trigger_icon(Icon::Database)
            .group(
                DropdownGroup::new("工作区")
                    .item(DropdownItem::new("工作区").icon(Icon::Folders))
                    .item(DropdownItem::new("环境").icon(Icon::Filter))
                    .item(DropdownItem::new("插件").icon(Icon::Workflow)),
            )
            .group(
                DropdownGroup::new("资源")
                    .item(DropdownItem::new("连接").icon(Icon::Database))
                    .item(DropdownItem::new("API").icon(Icon::Globe))
                    .item(DropdownItem::new("Proxy").icon(Icon::Globe))
                    .item(DropdownItem::new("Redis").icon(Icon::Database))
                    .item(DropdownItem::new("SSH").icon(Icon::Script))
                    .item(DropdownItem::new("媒体配置").icon(Icon::ImageGroup)),
            )
            .group(
                DropdownGroup::new("工具")
                    .item(DropdownItem::new("类型结构").icon(Icon::Text))
                    .item(DropdownItem::new("文档").icon(Icon::Script))
                    .item(DropdownItem::new("文档源码编辑器").icon(Icon::Script))
                    .item(DropdownItem::new("HTTP 网关文档").icon(Icon::ApiDoc)),
            )
    });
    let sort_dropdown = cx.new(|_| {
        GroupedDropdown::new("排序：")
            .selected("最近更新")
            .show_selected_in_trigger(true)
            .group(
                DropdownGroup::new("排序")
                    .item(DropdownItem::new("最近更新"))
                    .item(DropdownItem::new("评分最高"))
                    .item(DropdownItem::new("资源数最多")),
            )
    });
    let search_input =
        cx.new(|cx| gpui_glint::components::TextEntry::new(cx, "搜索集合或资源", ""));
    let name_input = cx.new(|cx| gpui_glint::components::TextEntry::new(cx, "输入以筛选…", ""));
    let error_input =
        cx.new(|cx| gpui_glint::components::TextEntry::new(cx, "", "不存在的集合 ID"));
    let slider = cx.new(|_| gpui_glint::components::Slider::new(62.));
    gpui_glint::components::bind_text_entry_keys(cx);
    gpui_glint::components::bind_document_editor_keys(cx);
    gpui_glint::components::bind_prompt_editor_keys(cx);
    gpui_glint::components::bind_composer_keys(cx);
    let theme = load_theme().unwrap_or_default();
    resource_dropdown.update(cx, |menu, cx| menu.set_theme(theme, cx));
    let overview = cx.new(|_| OverviewPage { theme });
    let buttons = cx.new(|_| ButtonsPage { theme });
    let icons = cx.new(|_| IconsPage { theme });
    let layout = cx.new(|_| LayoutPage::new(theme));
    let inputs = cx.new(|cx| {
        InputsPage::new(
            theme,
            sort_dropdown,
            slider,
            search_input,
            name_input,
            error_input,
            cx,
        )
    });
    let badges = cx.new(|_| BadgesPage { theme });
    let code_cards = cx.new(|cx| CodeCardsPage::new(theme, cx));
    let commands = cx.new(|cx| CommandsPage::new(theme, cx));
    let tables = cx.new(|cx| TablesPage::new(theme, cx));
    let documents = cx.new(|cx| DocumentsPage::new(theme, cx));
    let tabs = cx.new(|cx| TabsPage::new(theme, cx));

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
        move |_, cx| {
            cx.new(|cx| {
                let theme_subscription = cx.subscribe(
                    &inputs,
                    |this: &mut Playground, _, event: &ThemeSelectorEvent, cx| {
                        let ThemeSelectorEvent::Changed(theme) = event;
                        this.set_theme(*theme, cx);
                    },
                );
                let commands_subscription = cx.observe(&commands, |_, _, cx| cx.notify());
                Playground {
                    selected: Page::Documents,
                    theme,
                    resource_dropdown,
                    overview,
                    buttons,
                    icons,
                    layout,
                    inputs,
                    badges,
                    code_cards,
                    commands,
                    tables,
                    documents,
                    tabs,
                    _theme_subscription: theme_subscription,
                    _commands_subscription: commands_subscription,
                }
            })
        },
    )
    .expect("failed to open playground window");
}

fn theme_storage_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    base.map(|path| path.join("gpui-glint").join("theme"))
}

fn load_theme() -> Option<ThemePreset> {
    let path = theme_storage_path()?;
    let legacy = path.parent()?.parent()?.join("gpui-glass/theme");
    let id = std::fs::read_to_string(path)
        .or_else(|_| std::fs::read_to_string(legacy))
        .ok()?;
    ThemePreset::from_id(id.trim())
}

fn persist_theme(theme: ThemePreset) {
    let Some(path) = theme_storage_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, theme.id());
}

fn main() {
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| open(cx));
}
