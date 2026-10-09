use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb,
};
use gpui_glint::components::{
    ClosableTab, ClosableTabs, GroupList, GroupListGroup, GroupListHeader, GroupListItem, Icon,
    SideTab, SideTabs, TabOption, Tabs, TabsKind,
};
use gpui_glint::theme::ThemePreset;

use super::common::{page, panel};

pub struct TabsPage {
    pub theme: ThemePreset,
    selected: usize,
    closable_tabs: Vec<(u64, &'static str)>,
    selected_closable_tab: Option<u64>,
    next_closable_id: u64,
    selected_side_tab: Option<u64>,
    selected_pill_tab: usize,
    group_list: Entity<GroupList>,
}

impl TabsPage {
    pub fn new(theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let group_list = cx.new(|_cx| {
            GroupList::new([
                GroupListGroup::new(
                    GroupListHeader::new("gpui-glint", "gpui-glint")
                        .icon(Icon::Folders)
                        .actions([Icon::More, Icon::List, Icon::Plus]),
                )
                .item(
                    GroupListItem::new("dark-sidebar", "markdown_editor 侧栏黑暗主题适配")
                        .icon(Icon::Pulse)
                        .meta("刚刚"),
                )
                .item(
                    GroupListItem::new("closable-tabs", "可关闭标签页样式修改")
                        .icon(Icon::Text)
                        .meta("1天")
                        .hover_icon(Icon::Star)
                        .action(Icon::Trash),
                )
                .item(
                    GroupListItem::new("checklist", "文档组件的 checklist 与有序列表")
                        .icon(Icon::Text)
                        .meta("5天")
                        .hover_icon(Icon::Star)
                        .action(Icon::Trash),
                ),
                GroupListGroup::new(
                    GroupListHeader::new("rimworld", "rimworld").icon(Icon::Folders),
                )
                .item(
                    GroupListItem::new("rimworld-ship", "环世界执剑者号飞船建造图纸")
                        .icon(Icon::Text)
                        .meta("31天")
                        .hover_icon(Icon::Star)
                        .action(Icon::Trash),
                ),
            ])
            .selected(Some("dark-sidebar".into()))
            .theme(theme)
        });
        cx.subscribe(&group_list, |_, _, _, cx| cx.notify())
            .detach();
        Self {
            theme,
            selected: 0,
            closable_tabs: vec![(1, "用户列表"), (2, "创建用户"), (3, "登录请求")],
            selected_closable_tab: Some(2),
            next_closable_id: 4,
            selected_side_tab: Some(1),
            selected_pill_tab: 0,
            group_list,
        }
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.group_list
            .update(cx, |list, cx| list.set_theme(theme, cx));
        cx.notify();
    }
}

impl Render for TabsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity();
        let tabs = Tabs::new([
            TabOption::new("概览").icon(Icon::Compass),
            TabOption::new("权限").icon(Icon::User),
            TabOption::new("历史").icon(Icon::Clock).disabled(true),
        ])
        .selected(self.selected)
        .theme(self.theme)
        .on_change(move |index, _, app| {
            owner.update(app, |this, cx| {
                this.selected = index;
                cx.notify();
            });
        });

        let palette = self.theme.palette();
        let (title, description) = match self.selected {
            1 => ("访问权限", "查看并管理当前工作区的成员与角色。"),
            2 => ("操作历史", "最近的文档操作会显示在这里。"),
            _ => ("工作区概览", "查看工作区状态、最近更新和常用入口。"),
        };
        let example = div().w_full().flex().flex_col().child(tabs).child(
            div()
                .min_h(px(68.))
                .pt_3()
                .flex()
                .flex_col()
                .gap_1()
                .text_size(px(13.))
                .text_color(rgb(palette.text))
                .child(div().font_weight(gpui::FontWeight::MEDIUM).child(title))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(palette.text_muted))
                        .child(description),
                ),
        );

        let owner = cx.entity();
        let close_owner = owner.clone();
        let add_owner = owner.clone();
        let closable_tabs = ClosableTabs::new(
            self.closable_tabs
                .iter()
                .map(|(id, label)| ClosableTab::new(*id, *label)),
        )
        .selected(self.selected_closable_tab)
        .theme(self.theme)
        .content(1, "用户列表内容")
        .content(2, "创建用户内容")
        .content(3, "登录请求内容")
        .on_select(move |id, _, app| {
            owner.update(app, |this, cx| {
                this.selected_closable_tab = Some(id);
                cx.notify();
            });
        })
        .on_close(move |id, _, app| {
            close_owner.update(app, |this, cx| {
                let closed_index = this
                    .closable_tabs
                    .iter()
                    .position(|(tab_id, _)| *tab_id == id);
                this.closable_tabs.retain(|(tab_id, _)| *tab_id != id);
                if this.selected_closable_tab == Some(id) {
                    this.selected_closable_tab = closed_index
                        .and_then(|index| {
                            this.closable_tabs
                                .get(index.min(this.closable_tabs.len().saturating_sub(1)))
                        })
                        .map(|(tab_id, _)| *tab_id);
                }
                cx.notify();
            });
        });
        let closable_tabs = closable_tabs.on_add(move |_, app| {
            add_owner.update(app, |this, cx| {
                let id = this.next_closable_id;
                this.next_closable_id += 1;
                this.closable_tabs.push((id, "新建请求"));
                this.selected_closable_tab = Some(id);
                cx.notify();
            });
        });
        let closable_example = div().w_full().flex().flex_col().child(closable_tabs);

        let side_owner = cx.entity();
        let side_tabs = SideTabs::new([
            SideTab::new(1, "常规"),
            SideTab::new(2, "连接"),
            SideTab::new(3, "外观"),
        ])
        .selected(self.selected_side_tab)
        .theme(self.theme)
        .content(1, "常规内容")
        .content(2, "连接内容")
        .content(3, "外观内容")
        .on_select(move |id, _, app| {
            side_owner.update(app, |this, cx| {
                this.selected_side_tab = Some(id);
                cx.notify();
            });
        });
        let side_example = div().w_full().flex().flex_col().child(side_tabs);

        let group_example = div()
            .w_full()
            .flex()
            .gap_5()
            .child(
                div()
                    .w(px(300.))
                    .flex_shrink_0()
                    .child(self.group_list.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(palette.text_muted))
                    .child(
                        "悬停分组头浮现操作按钮；悬停条目时前导图标换为收藏、尾部元信息换为删除。",
                    )
                    .child("点击条目即选中；事件通过 GroupListEvent 上报。"),
            );

        let pill_owner = cx.entity();
        let pill_tabs = Tabs::new([
            TabOption::new("插件").count("14"),
            TabOption::new("MCP").count("2"),
            TabOption::new("技能").count("24"),
        ])
        .kind(TabsKind::Pill)
        .selected(self.selected_pill_tab)
        .theme(self.theme)
        .on_change(move |index, _, app| {
            pill_owner.update(app, |this, cx| {
                this.selected_pill_tab = index;
                cx.notify();
            });
        });
        let pill_example = div().w_full().flex().flex_col().child(pill_tabs);

        page(
            "17",
            "标签页",
            "标准下划线标签栏、胶囊筛选标签、文档式可关闭标签与侧边设置分区：活动项分别以主题色下划线、胶囊底色、相连面板和左侧色条标记。",
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(panel(example, self.theme))
                .child(panel(pill_example, self.theme))
                .child(panel(closable_example, self.theme))
                .child(panel(side_example, self.theme))
                .child(panel(group_example, self.theme)),
            self.theme,
        )
    }
}
