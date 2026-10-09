use super::common::{page, panel};
use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, px, rgb,
};
use gpui_glint::{
    components::{
        ContentType, DataTable, Icon, TableAction, TableCell, TableColumn, TableEvent, TableRow,
    },
    theme::ThemePreset,
};

pub struct TablesPage {
    pub theme: ThemePreset,
    table: Entity<DataTable>,
    rows: Vec<TableRow>,
    notice: SharedString,
    _subscription: Subscription,
}
impl TablesPage {
    pub fn new(theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let rows = sample_rows();
        let table = cx.new(|cx| {
            DataTable::new(
                cx,
                [
                    TableColumn::new("编号", 72.).sortable(),
                    TableColumn::new("资源", 230.).flexible().sortable(),
                    TableColumn::new("类型", 128.),
                    TableColumn::new("时长", 80.).right().sortable(),
                    TableColumn::new("大小", 82.).right().sortable(),
                    TableColumn::new("状态", 164.),
                ],
                rows.clone(),
            )
            .theme(theme)
            .page_size(19)
            .page(2)
            .selected([SharedString::from("008")])
        });
        let subscription = cx.subscribe(&table, |this, table, event, cx| {
            if let TableEvent::Action { row_id, action_id } = event {
                let name = this
                    .rows
                    .iter()
                    .find(|row| &row.id == row_id)
                    .map(|_| row_id.clone())
                    .unwrap_or_else(|| "资源".into());
                match action_id.as_ref() {
                    "delete" => {
                        this.rows.retain(|row| &row.id != row_id);
                        table.update(cx, |table, cx| table.set_rows(this.rows.clone(), cx));
                        this.notice = format!("已删除资源 {name}").into();
                    }
                    "seen" => {
                        if let Some(row) = this.rows.iter_mut().find(|row| &row.id == row_id) {
                            row.cells[5] = TableCell::status("已看", 0x007aff);
                        }
                        table.update(cx, |table, cx| table.set_rows(this.rows.clone(), cx));
                        this.notice = format!("资源 {name} 已标记为已看").into();
                    }
                    _ => {
                        this.notice = format!("已打开资源 {name}（示例）").into();
                    }
                }
                cx.notify();
            }
        });
        Self {
            theme,
            table,
            rows,
            notice: "点击表头排序；⌘ / Ctrl 点击多选，Shift 点击连续选择。".into(),
            _subscription: subscription,
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.table
            .update(cx, |table, cx| table.set_theme(theme, cx));
        cx.notify();
    }
}
impl Render for TablesPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let specs = div()
            .flex()
            .flex_wrap()
            .gap_5()
            .text_size(px(12.))
            .text_color(rgb(self.theme.palette().text_muted))
            .child("表头：11.5px 半粗 + 内嵌底")
            .child("行：9px / 12px 内距 · hover 浅填充")
            .child("选中：蓝色调底，不用描边")
            .child("操作列：hover 浮现 · 菜单向上弹");
        let content = div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_5()
            .child(self.table.clone())
            .child(specs);
        page(
            "12",
            "表格",
            "密集数据按 mac 列表视图的规矩来：小号灰色表头、轻分隔线、悬停浅填充。数字列右对齐使用等宽字体，操作列悬停时浮现，菜单向上弹出。",
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_4()
                .child(panel(content, self.theme))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(self.theme.palette().text_muted))
                        .child(self.notice.clone()),
                ),
            self.theme,
        )
    }
}

fn resource(
    id: &str,
    title: &str,
    kind: ContentType,
    duration: (&str, Option<f64>),
    size: (&str, Option<f64>),
    state: (&str, u32),
    thumbnail: (u32, Option<Icon>),
) -> TableRow {
    let (duration, seconds) = duration;
    let (size, bytes) = size;
    let (state, state_color) = state;
    let (thumbnail, icon) = thumbnail;
    let mut title = TableCell::resource(title.to_owned(), thumbnail);
    if let Some(icon) = icon {
        title = title.thumbnail_icon(icon);
    }
    let mut duration = TableCell::mono(duration.to_owned());
    if let Some(seconds) = seconds {
        duration = duration.sort_number(seconds);
    }
    let mut size = TableCell::mono(size.to_owned());
    if let Some(bytes) = bytes {
        size = size.sort_number(bytes);
    }
    let open_icon = if matches!(kind, ContentType::Link) {
        Icon::External
    } else {
        Icon::Play
    };
    let open_color = if state == "看到一半" {
        0xff9500
    } else {
        0x48484a
    };
    TableRow::new(
        id.to_owned(),
        [
            TableCell::mono(id.to_owned()).sort_number(id.parse::<f64>().unwrap()),
            title,
            TableCell::content_type(kind),
            duration,
            size,
            TableCell::status(state.to_owned(), state_color),
        ],
    )
    .primary_action(TableAction::new("open", "打开", open_icon).color(open_color))
    .action(TableAction::new("open", "打开资源", open_icon))
    .action(TableAction::new("seen", "标记为已看", Icon::CheckCircle))
    .action(TableAction::new("delete", "删除", Icon::Trash).danger())
}
fn sample_rows() -> Vec<TableRow> {
    let mut rows = Vec::new();
    // 42 actual demo records; the last page contains the four reference rows.
    for index in 0..38 {
        let id = format!("{:03}", 100 + index);
        let title = format!("城市影像 · 拍摄记录 {:02}", index + 1);
        rows.push(resource(
            &id,
            &title,
            ContentType::Video,
            ("03:20", Some(200.)),
            ("240M", Some(240.)),
            ("未看", 0xaeaeb2),
            (0x2b3c4c, None),
        ));
    }
    rows.extend([
        resource(
            "007",
            "雾夜 · 慢门 30s 组图",
            ContentType::ImageGroup,
            ("—", None),
            ("1.2G", Some(1200.)),
            ("已看", 0x007aff),
            (0x305343, None),
        ),
        resource(
            "008",
            "环线夜航 · 无人机素材",
            ContentType::Video,
            ("15:47", Some(947.)),
            ("1.8G", Some(1800.)),
            ("看到一半", 0xff9500),
            (0x604832, Some(Icon::Play)),
        ),
        resource(
            "009",
            "天桥 · 行人轨迹",
            ContentType::Video,
            ("09:58", Some(598.)),
            ("820M", Some(820.)),
            ("未看", 0xaeaeb2),
            (0x2b3c4c, None),
        ),
        resource(
            "012",
            "器材与滤镜清单",
            ContentType::Link,
            ("—", None),
            ("—", None),
            ("未看", 0xaeaeb2),
            (0x20242c, Some(Icon::External)),
        ),
    ]);
    rows
}
