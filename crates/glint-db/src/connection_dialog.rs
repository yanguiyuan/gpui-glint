use crate::model::{ConnectionConfig, DbKind};
use gpui::{AppContext, Context, Entity, Focusable, Window, div, prelude::*, px, rgb};
use gpui_glint::components::{
    Button, ButtonKind, ButtonSize, Dialog, SegmentOption, SegmentedControl, TextEntry,
};
use gpui_glint::theme::ThemePreset;
use uuid::Uuid;

pub struct ConnectionForm {
    pub editing_id: Option<Uuid>,
    pub kind: DbKind,
    pub name: Entity<TextEntry>,
    pub file_path: Entity<TextEntry>,
    pub host: Entity<TextEntry>,
    pub port: Entity<TextEntry>,
    pub database: Entity<TextEntry>,
    pub username: Entity<TextEntry>,
    pub password: Entity<TextEntry>,
    pub testing: bool,
    pub test_status: Option<Result<String, String>>,
    pub error: Option<String>,
}

impl ConnectionForm {
    pub fn from_config(
        config: &ConnectionConfig,
        editing_id: Option<Uuid>,
        theme: ThemePreset,
        cx: &mut gpui::App,
    ) -> Self {
        let name = cx.new(|cx| {
            let mut e = TextEntry::new(cx, "例如：本地测试库", &config.name);
            e.set_theme(theme, cx);
            e
        });
        let file_path = cx.new(|cx| {
            let mut e = TextEntry::new(cx, "/path/to/database.sqlite", &config.file_path);
            e.set_theme(theme, cx);
            e
        });
        let host = cx.new(|cx| {
            let mut e = TextEntry::new(cx, "127.0.0.1", &config.host);
            e.set_theme(theme, cx);
            e
        });
        let port = cx.new(|cx| {
            let port_val = if config.port > 0 {
                config.port.to_string()
            } else {
                String::new()
            };
            let mut e = TextEntry::new(cx, "3306", &port_val);
            e.set_theme(theme, cx);
            e
        });
        let database = cx.new(|cx| {
            let mut e = TextEntry::new(cx, "数据库名（可选）", &config.database);
            e.set_theme(theme, cx);
            e
        });
        let username = cx.new(|cx| {
            let mut e = TextEntry::new(cx, "用户名", &config.username);
            e.set_theme(theme, cx);
            e
        });
        let password = cx.new(|cx| {
            let mut e = TextEntry::new(cx, "密码（可选）", &config.password).masked(true);
            e.set_theme(theme, cx);
            e
        });
        Self {
            editing_id,
            kind: config.kind,
            name,
            file_path,
            host,
            port,
            database,
            username,
            password,
            testing: false,
            test_status: None,
            error: None,
        }
    }

    pub fn set_kind(&mut self, kind: DbKind, cx: &mut gpui::App) {
        if self.kind == kind {
            return;
        }
        self.kind = kind;
        self.test_status = None;
        self.error = None;
        let cur_name = self.name.read(cx).value().trim().to_owned();
        for old_kind in DbKind::ALL {
            let prefix = format!("{} 连接 ", old_kind.label());
            if let Some(suffix) = cur_name.strip_prefix(&prefix) {
                let next_name = format!("{} 连接 {suffix}", kind.label());
                self.name.update(cx, |e, cx| e.set_value(&next_name, cx));
                break;
            }
        }
        if !kind.is_file_based() {
            let cur_port = self.port.read(cx).value().trim().to_owned();
            if cur_port.is_empty() || cur_port == "3306" || cur_port == "5432" {
                self.port
                    .update(cx, |e, cx| e.set_value(&kind.default_port().to_string(), cx));
            }
            if self.host.read(cx).value().trim().is_empty() {
                self.host.update(cx, |e, cx| e.set_value("127.0.0.1", cx));
            }
            let cur_user = self.username.read(cx).value().trim().to_owned();
            if cur_user.is_empty() || cur_user == "root" || cur_user == "postgres" {
                let default_user = match kind {
                    DbKind::MySql => "root",
                    DbKind::Postgres => "postgres",
                    DbKind::Sqlite => "",
                };
                self.username
                    .update(cx, |e, cx| e.set_value(default_user, cx));
            }
            let cur_db = self.database.read(cx).value().trim().to_owned();
            if kind == DbKind::Postgres && cur_db.is_empty() {
                self.database.update(cx, |e, cx| e.set_value("postgres", cx));
            } else if kind == DbKind::MySql && cur_db == "postgres" {
                self.database.update(cx, |e, cx| e.set_value("", cx));
            }
        }
    }

    pub fn to_config(&self, cx: &gpui::App) -> Result<ConnectionConfig, String> {
        let port_str = self.port.read(cx).value().trim().to_owned();
        let port = if self.kind.is_file_based() {
            0
        } else if port_str.is_empty() {
            self.kind.default_port()
        } else {
            port_str
                .parse::<u16>()
                .map_err(|_| "端口必须是 1-65535 的数字".to_owned())?
        };
        let cfg = ConnectionConfig {
            id: self.editing_id.unwrap_or_else(Uuid::new_v4),
            name: self.name.read(cx).value().trim().to_owned(),
            kind: self.kind,
            file_path: self.file_path.read(cx).value().trim().to_owned(),
            host: self.host.read(cx).value().trim().to_owned(),
            port,
            database: self.database.read(cx).value().trim().to_owned(),
            username: self.username.read(cx).value().trim().to_owned(),
            password: self.password.read(cx).value().to_owned(),
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn set_theme(&self, theme: ThemePreset, cx: &mut gpui::App) {
        for entry in [
            &self.name,
            &self.file_path,
            &self.host,
            &self.port,
            &self.database,
            &self.username,
            &self.password,
        ] {
            entry.update(cx, |e, cx| e.set_theme(theme, cx));
        }
    }
}

fn field_row(
    label: &'static str,
    input: gpui::AnyElement,
    p: gpui_glint::theme::ThemePalette,
) -> gpui::Div {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(p.text_muted))
                .child(label),
        )
        .child(
            div()
                .w_full()
                .h(px(32.))
                .px_2p5()
                .rounded(px(7.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.workspace))
                .flex()
                .items_center()
                .child(div().w_full().min_w_0().child(input)),
        )
}

pub fn render_connection_dialog<V: 'static>(
    form: &ConnectionForm,
    theme: ThemePreset,
    on_kind: impl Fn(DbKind, &mut Window, &mut gpui::App) + 'static,
    on_test: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    on_cancel: impl Fn(&mut Window, &mut gpui::App) + 'static + Clone,
    on_save: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    _cx: &mut Context<V>,
) -> Dialog {
    let p = theme.palette();
    let title = if form.editing_id.is_some() {
        "编辑数据库连接"
    } else {
        "新建数据库连接"
    };
    let kind_index = DbKind::ALL
        .iter()
        .position(|k| *k == form.kind)
        .unwrap_or(0);

    let mut fields = div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_size(px(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(p.text))
                .child(title),
        )
        .child(
            SegmentedControl::new([
                SegmentOption::new("SQLite").debug_selector("db-kind-sqlite"),
                SegmentOption::new("MySQL").debug_selector("db-kind-mysql"),
                SegmentOption::new("PostgreSQL").debug_selector("db-kind-postgres"),
            ])
            .id("db-kind-segment")
            .selected(kind_index)
            .theme(theme)
            .on_change(move |idx, window, cx| {
                let kind = DbKind::ALL.get(idx).copied().unwrap_or(DbKind::Sqlite);
                on_kind(kind, window, cx);
            }),
        )
        .child(field_row("连接名称", form.name.clone().into_any_element(), p));

    if form.kind.is_file_based() {
        fields = fields.child(field_row(
            "SQLite 文件路径",
            form.file_path.clone().into_any_element(),
            p,
        ));
    } else {
        fields = fields
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap_3()
                    .child(div().flex_1().min_w_0().child(field_row(
                        "主机",
                        form.host.clone().into_any_element(),
                        p,
                    )))
                    .child(div().w(px(112.)).flex_shrink_0().child(field_row(
                        "端口",
                        form.port.clone().into_any_element(),
                        p,
                    ))),
            )
            .child(field_row(
                "默认数据库",
                form.database.clone().into_any_element(),
                p,
            ))
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap_3()
                    .child(div().flex_1().min_w_0().child(field_row(
                        "用户名",
                        form.username.clone().into_any_element(),
                        p,
                    )))
                    .child(div().flex_1().min_w_0().child(field_row(
                        "密码",
                        form.password.clone().into_any_element(),
                        p,
                    ))),
            );
    }

    if let Some(err) = &form.error {
        fields = fields.child(
            div()
                .px_2p5()
                .py_1p5()
                .rounded(px(6.))
                .bg(rgb(p.surface_subtle))
                .text_size(px(11.))
                .text_color(rgb(0xd9342b))
                .child(err.clone()),
        );
    } else if let Some(status) = &form.test_status {
        let (color, msg) = match status {
            Ok(msg) => (0x34a853, msg.clone()),
            Err(msg) => (0xd9342b, msg.clone()),
        };
        fields = fields.child(
            div()
                .px_2p5()
                .py_1p5()
                .rounded(px(6.))
                .bg(rgb(p.surface_subtle))
                .text_size(px(11.))
                .text_color(rgb(color))
                .child(msg),
        );
    }

    let cancel_btn_cb = on_cancel.clone();
    let content = fields.child(
        div()
            .pt_2()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                Button::new(if form.testing { "测试中…" } else { "测试连接" })
                    .kind(ButtonKind::Glass)
                    .size(ButtonSize::Small)
                    .theme(theme)
                    .render()
                    .debug_selector(|| "db-dialog-test".into())
                    .when(!form.testing, |b| b.on_click(on_test)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("取消")
                            .kind(ButtonKind::Gray)
                            .size(ButtonSize::Small)
                            .theme(theme)
                            .render()
                            .on_click(move |_, window, cx| cancel_btn_cb(window, cx)),
                    )
                    .child(
                        Button::new("保存")
                            .kind(ButtonKind::Primary)
                            .size(ButtonSize::Small)
                            .theme(theme)
                            .render()
                            .debug_selector(|| "db-dialog-save".into())
                            .on_click(on_save),
                    ),
            ),
    );

    let focus_handles = if form.kind.is_file_based() {
        vec![
            form.name.read(_cx).focus_handle(_cx),
            form.file_path.read(_cx).focus_handle(_cx),
        ]
    } else {
        vec![
            form.name.read(_cx).focus_handle(_cx),
            form.host.read(_cx).focus_handle(_cx),
            form.port.read(_cx).focus_handle(_cx),
            form.database.read(_cx).focus_handle(_cx),
            form.username.read(_cx).focus_handle(_cx),
            form.password.read(_cx).focus_handle(_cx),
        ]
    };

    Dialog::new("db-connection-dialog", content)
        .width(440.)
        .theme(theme)
        .focus_handles(focus_handles)
        .on_close(move |_, window, cx| on_cancel(window, cx))
}
