use crate::tools::ToolRecord;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb, rgba,
};
use gpui_glint::components::Icon;
use gpui_glint::theme::ThemePreset;
use serde_json::Value;

const MAX_PREVIEW_ROWS: usize = 100;

/// Renders a structured preview for `glint_db_*` tool outputs when the result
/// is valid JSON, returning `None` so errors or non-JSON outputs fall back to
/// the standard text viewer.
pub(super) fn render_db_tool_preview(
    record: &ToolRecord,
    theme: ThemePreset,
) -> Option<AnyElement> {
    let name = record.call.name.as_str();
    if !name.starts_with("glint_db_") {
        return None;
    }
    let value: Value = serde_json::from_str(record.result.trim()).ok()?;

    let content = match name {
        "glint_db_list_connections" => render_connections(&value, theme),
        "glint_db_list_databases" => render_databases(&value, theme),
        "glint_db_list_tables" => render_tables(&value, theme),
        "glint_db_describe_table" => render_describe_table(&value, theme),
        "glint_db_query" => render_query_result(record, &value, theme),
        "glint_db_execute" => render_execute_result(record, &value, theme),
        _ => return None,
    };

    Some(
        div()
            .debug_selector(|| "db-tool-preview".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(content)
            .into_any_element(),
    )
}

fn badge(label: impl Into<SharedString>, bg_color: u32, text_color: u32) -> gpui::Div {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .bg(rgb(bg_color))
        .text_size(px(11.))
        .line_height(px(16.))
        .text_color(rgb(text_color))
        .child(label.into())
}

fn subtle_badge(label: impl Into<SharedString>, theme: ThemePreset) -> gpui::Div {
    let p = theme.palette();
    badge(label, p.surface_subtle, p.text_muted)
}

fn meta_header(
    title: impl Into<SharedString>,
    badges: Vec<gpui::Div>,
    theme: ThemePreset,
) -> gpui::Div {
    let p = theme.palette();
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.text))
                .child(Icon::Database.render(13.).text_color(rgb(p.text_muted)))
                .child(title.into()),
        )
        .children(badges)
}

fn connection_label(value: &Value) -> String {
    let conn = value["connection"].as_str().unwrap_or("默认连接");
    match value["database"].as_str().filter(|s| !s.is_empty()) {
        Some(db) => format!("{conn} / {db}"),
        None => conn.to_owned(),
    }
}

fn empty_state(text: &'static str, theme: ThemePreset) -> AnyElement {
    let p = theme.palette();
    div()
        .w_full()
        .py_2()
        .px_3()
        .rounded(px(6.))
        .bg(rgb(p.surface))
        .border_1()
        .border_color(rgb(p.border))
        .text_size(px(12.))
        .text_color(rgb(p.text_muted))
        .child(text)
        .into_any_element()
}

fn render_connections(value: &Value, theme: ThemePreset) -> AnyElement {
    let p = theme.palette();
    let items = value["connections"].as_array();
    let count = value["count"]
        .as_u64()
        .unwrap_or_else(|| items.map_or(0, Vec::len) as u64);
    let header = meta_header(
        "数据库连接",
        vec![subtle_badge(format!("{count} 个连接"), theme)],
        theme,
    );
    let Some(items) = items.filter(|list| !list.is_empty()) else {
        return div()
            .flex()
            .flex_col()
            .gap_2()
            .child(header)
            .child(empty_state("暂无已保存的数据库连接", theme))
            .into_any_element();
    };

    let rows = items.iter().map(|item| {
        let name = item["name"].as_str().unwrap_or("未命名").to_owned();
        let kind = item["kind"].as_str().unwrap_or("").to_owned();
        let summary = item["summary"].as_str().unwrap_or("").to_owned();
        let database = item["database"].as_str().unwrap_or("").to_owned();
        let active = item["active"].as_bool().unwrap_or(false);

        div()
            .w_full()
            .min_w_0()
            .px(px(10.))
            .py(px(7.))
            .rounded(px(6.))
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(if active { p.accent } else { p.border }))
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        Icon::Database
                            .render(13.)
                            .text_color(rgb(if active { p.accent } else { p.text_muted })),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(p.text))
                            .child(name),
                    )
                    .when(!kind.is_empty(), |row| row.child(subtle_badge(kind, theme)))
                    .when(active, |row| {
                        row.child(badge("当前激活", p.accent_tint, p.accent))
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .font_family("monospace")
                    .text_color(rgb(p.text_muted))
                    .when(!summary.is_empty(), |row| row.child(summary))
                    .when(!database.is_empty(), |row| {
                        row.child(subtle_badge(database, theme))
                    }),
            )
    });

    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(header)
        .child(div().flex().flex_col().gap(px(6.)).children(rows))
        .into_any_element()
}

fn render_databases(value: &Value, theme: ThemePreset) -> AnyElement {
    let p = theme.palette();
    let conn = value["connection"].as_str().unwrap_or("默认连接").to_owned();
    let kind = value["kind"].as_str().unwrap_or("").to_owned();
    let dbs = value["databases"].as_array();
    let count = dbs.map_or(0, Vec::len);

    let mut badges = Vec::new();
    if !kind.is_empty() {
        badges.push(subtle_badge(kind, theme));
    }
    badges.push(subtle_badge(format!("{count} 个数据库"), theme));
    let header = meta_header(conn, badges, theme);

    let Some(dbs) = dbs.filter(|list| !list.is_empty()) else {
        return div()
            .flex()
            .flex_col()
            .gap_2()
            .child(header)
            .child(empty_state("未发现数据库", theme))
            .into_any_element();
    };

    let chips = dbs.iter().filter_map(|db| db.as_str()).map(|name| {
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .flex()
            .items_center()
            .gap(px(5.))
            .text_size(px(12.))
            .font_family("monospace")
            .text_color(rgb(p.text))
            .child(Icon::Database.render(11.).text_color(rgb(p.text_muted)))
            .child(name.to_owned())
    });

    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_2()
        .child(header)
        .child(
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .children(chips),
        )
        .into_any_element()
}

fn render_tables(value: &Value, theme: ThemePreset) -> AnyElement {
    let p = theme.palette();
    let tables = value["tables"].as_array();
    let count = value["count"]
        .as_u64()
        .unwrap_or_else(|| tables.map_or(0, Vec::len) as u64);
    let header = meta_header(
        connection_label(value),
        vec![subtle_badge(format!("{count} 张表/视图"), theme)],
        theme,
    );

    let Some(tables) = tables.filter(|list| !list.is_empty()) else {
        return div()
            .flex()
            .flex_col()
            .gap_2()
            .child(header)
            .child(empty_state("当前数据库暂无数据表", theme))
            .into_any_element();
    };

    let chips = tables.iter().map(|item| {
        let name = item["name"].as_str().unwrap_or("").to_owned();
        let kind = item["kind"].as_str().unwrap_or("table");
        let is_view = kind == "view";
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_family("monospace")
                    .text_color(rgb(p.text))
                    .child(name),
            )
            .child(if is_view {
                badge("VIEW", p.accent_tint, p.accent)
            } else {
                subtle_badge("TABLE", theme)
            })
    });

    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_2()
        .child(header)
        .child(
            div()
                .id("db-tables-scroll")
                .max_h(px(240.))
                .overflow_y_scroll()
                .w_full()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .children(chips),
        )
        .into_any_element()
}

fn render_describe_table(value: &Value, theme: ThemePreset) -> AnyElement {
    let p = theme.palette();
    let dark = theme.is_dark();
    let table = value["table"].as_str().unwrap_or("表结构").to_owned();
    let cols = value["columns"].as_array();
    let count = cols.map_or(0, Vec::len);
    let header = meta_header(
        table,
        vec![
            subtle_badge(connection_label(value), theme),
            subtle_badge(format!("{count} 个字段"), theme),
        ],
        theme,
    );

    let Some(cols) = cols.filter(|list| !list.is_empty()) else {
        return div()
            .flex()
            .flex_col()
            .gap_2()
            .child(header)
            .child(empty_state("未获取到字段信息", theme))
            .into_any_element();
    };

    let alt_row = if dark {
        rgba(0xffffff06)
    } else {
        rgba(0x00000005)
    };

    let head_row = div()
        .w_full()
        .px(px(10.))
        .py(px(6.))
        .bg(rgb(p.surface_subtle))
        .border_b_1()
        .border_color(rgb(p.border))
        .flex()
        .items_center()
        .gap(px(10.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(p.text_muted))
        .child(div().w(px(160.)).flex_shrink_0().child("字段"))
        .child(div().w(px(130.)).flex_shrink_0().child("类型"))
        .child(div().w(px(100.)).flex_shrink_0().child("约束"))
        .child(div().flex_1().min_w(px(90.)).child("默认值"));

    let body_rows = cols.iter().enumerate().map(|(idx, col)| {
        let name = col["name"].as_str().unwrap_or("").to_owned();
        let data_type = col["data_type"].as_str().unwrap_or("").to_owned();
        let nullable = col["nullable"].as_bool().unwrap_or(true);
        let pk = col["primary_key"].as_bool().unwrap_or(false);
        let default_val = col["default_value"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("-")
            .to_owned();

        div()
            .w_full()
            .px(px(10.))
            .py(px(5.))
            .when(idx % 2 == 1, |row| row.bg(alt_row))
            .border_b_1()
            .border_color(rgb(p.border))
            .flex()
            .items_center()
            .gap(px(10.))
            .text_size(px(12.))
            .child(
                div()
                    .w(px(160.))
                    .flex_shrink_0()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .when(pk, |cell| {
                        cell.child(Icon::Key.render(11.).text_color(rgb(p.accent)))
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family("monospace")
                            .font_weight(if pk {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(rgb(p.text))
                            .child(name),
                    ),
            )
            .child(
                div()
                    .w(px(130.))
                    .flex_shrink_0()
                    .truncate()
                    .font_family("monospace")
                    .text_size(px(11.))
                    .text_color(rgb(p.accent))
                    .child(data_type),
            )
            .child(
                div()
                    .w(px(100.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .when(pk, |cell| cell.child(badge("PK", p.accent_tint, p.accent)))
                    .child(subtle_badge(
                        if nullable { "NULL" } else { "NOT NULL" },
                        theme,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(90.))
                    .truncate()
                    .font_family("monospace")
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(default_val),
            )
    });

    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_2()
        .child(header)
        .child(
            div()
                .id("db-describe-scroll")
                .w_full()
                .max_h(px(280.))
                .overflow_y_scroll()
                .overflow_x_scroll()
                .rounded(px(6.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.surface))
                .flex()
                .flex_col()
                .child(head_row)
                .children(body_rows),
        )
        .into_any_element()
}

fn sql_preview_box(sql: &str, theme: ThemePreset) -> gpui::Div {
    let p = theme.palette();
    div()
        .w_full()
        .min_w_0()
        .px(px(10.))
        .py(px(6.))
        .rounded(px(6.))
        .bg(rgb(p.surface_subtle))
        .border_1()
        .border_color(rgb(p.border))
        .text_size(px(11.))
        .line_height(px(17.))
        .font_family("monospace")
        .text_color(rgb(p.text))
        .child(sql.trim().to_owned())
}

fn render_query_result(record: &ToolRecord, value: &Value, theme: ThemePreset) -> AnyElement {
    let p = theme.palette();
    let dark = theme.is_dark();
    let columns: Vec<String> = value["columns"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let rows = value["rows"].as_array();
    let total_rows = value["row_count"]
        .as_u64()
        .unwrap_or_else(|| rows.map_or(0, Vec::len) as u64);
    let truncated = value["truncated"].as_bool().unwrap_or(false);
    let elapsed_ms = value["elapsed_ms"].as_u64();

    let mut badges = vec![subtle_badge(format!("返回 {total_rows} 行"), theme)];
    if truncated {
        badges.push(badge("已截断（最多 200 行）", p.accent_tint, p.accent));
    }
    if let Some(ms) = elapsed_ms {
        badges.push(subtle_badge(format!("{ms} ms"), theme));
    }
    let header = meta_header(connection_label(value), badges, theme);

    let mut root = div().w_full().min_w_0().flex().flex_col().gap_2().child(header);
    if let Some(sql) = record.call.arguments["sql"].as_str().filter(|s| !s.trim().is_empty()) {
        root = root.child(sql_preview_box(sql, theme));
    }

    let Some(rows) = rows.filter(|list| !list.is_empty() && !columns.is_empty()) else {
        return root
            .child(empty_state("查询成功，返回 0 行结果", theme))
            .into_any_element();
    };

    let col_width = match columns.len() {
        1 => px(320.),
        2..=3 => px(180.),
        _ => px(140.),
    };
    let alt_row = if dark {
        rgba(0xffffff06)
    } else {
        rgba(0x00000005)
    };

    let head_row = div()
        .flex()
        .items_center()
        .bg(rgb(p.surface_subtle))
        .border_b_1()
        .border_color(rgb(p.border))
        .children(columns.iter().map(|col| {
            div()
                .w(col_width)
                .flex_shrink_0()
                .px(px(10.))
                .py(px(6.))
                .border_r_1()
                .border_color(rgb(p.border))
                .truncate()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .font_family("monospace")
                .text_color(rgb(p.text))
                .child(col.clone())
        }));

    let shown_count = rows.len().min(MAX_PREVIEW_ROWS);
    let body_rows = rows.iter().take(MAX_PREVIEW_ROWS).enumerate().map(|(idx, row)| {
        div()
            .flex()
            .items_center()
            .when(idx % 2 == 1, |r| r.bg(alt_row))
            .border_b_1()
            .border_color(rgb(p.border))
            .children(columns.iter().map(|col| {
                let cell_text = match &row[col] {
                    Value::Null => "NULL".to_owned(),
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let is_null = row[col].is_null();
                div()
                    .w(col_width)
                    .flex_shrink_0()
                    .px(px(10.))
                    .py(px(5.))
                    .border_r_1()
                    .border_color(rgb(p.border))
                    .truncate()
                    .text_size(px(12.))
                    .font_family("monospace")
                    .text_color(rgb(if is_null { p.text_muted } else { p.text }))
                    .child(cell_text)
            }))
    });

    root = root.child(
        div()
            .id("db-query-table-scroll")
            .w_full()
            .max_h(px(280.))
            .overflow_y_scroll()
            .overflow_x_scroll()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .flex()
            .flex_col()
            .child(head_row)
            .children(body_rows),
    );
    if rows.len() > shown_count {
        root = root.child(
            div()
                .text_size(px(11.))
                .text_color(rgb(p.text_muted))
                .child(format!("表格预览仅展示前 {shown_count} 行（共返回 {} 行）", rows.len())),
        );
    }
    root.into_any_element()
}

fn render_execute_result(record: &ToolRecord, value: &Value, theme: ThemePreset) -> AnyElement {
    let p = theme.palette();
    let affected = value["affected_rows"].as_u64().unwrap_or(0);
    let message = value["message"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("执行成功")
        .to_owned();
    let elapsed_ms = value["elapsed_ms"].as_u64();

    let mut badges = vec![subtle_badge(format!("影响 {affected} 行"), theme)];
    if let Some(ms) = elapsed_ms {
        badges.push(subtle_badge(format!("{ms} ms"), theme));
    }
    let header = meta_header(connection_label(value), badges, theme);

    let mut root = div().w_full().min_w_0().flex().flex_col().gap_2().child(header);
    if let Some(sql) = record.call.arguments["sql"].as_str().filter(|s| !s.trim().is_empty()) {
        root = root.child(sql_preview_box(sql, theme));
    }
    root = root.child(
        div()
            .w_full()
            .px(px(10.))
            .py(px(7.))
            .rounded(px(6.))
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.))
            .text_color(rgb(p.text))
            .child(Icon::CheckCircle.render(13.).text_color(rgb(p.accent)))
            .child(message),
    );
    root.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolCall;
    use serde_json::json;

    fn make_record(name: &str, arguments: Value, result: Value) -> ToolRecord {
        ToolRecord {
            call: ToolCall {
                id: "db-1".into(),
                name: name.into(),
                arguments,
            },
            result: result.to_string(),
            success: true,
            elapsed_ms: 12,
            live_output: None,
            change: None,
            subagent: None,
            nested_tools: Vec::new().into(),
        }
    }

    #[test]
    fn renders_preview_for_all_glint_db_tools_and_skips_invalid_json() {
        let cases = vec![
            make_record(
                "glint_db_list_connections",
                json!({}),
                json!({
                    "connections": [{
                        "id": "1",
                        "name": "local-sqlite",
                        "kind": "SQLite",
                        "summary": "data.db",
                        "database": "main",
                        "active": true
                    }],
                    "count": 1
                }),
            ),
            make_record(
                "glint_db_list_databases",
                json!({}),
                json!({
                    "connection": "local-pg",
                    "kind": "PostgreSQL",
                    "databases": ["postgres", "app_db"]
                }),
            ),
            make_record(
                "glint_db_list_tables",
                json!({}),
                json!({
                    "connection": "local-pg",
                    "database": "app_db",
                    "tables": [
                        {"name": "users", "kind": "table"},
                        {"name": "active_users", "kind": "view"}
                    ],
                    "count": 2
                }),
            ),
            make_record(
                "glint_db_describe_table",
                json!({"table": "users"}),
                json!({
                    "connection": "local-pg",
                    "database": "app_db",
                    "table": "users",
                    "columns": [{
                        "name": "id",
                        "data_type": "bigint",
                        "nullable": false,
                        "primary_key": true,
                        "default_value": null
                    }]
                }),
            ),
            make_record(
                "glint_db_query",
                json!({"sql": "SELECT id, name FROM users LIMIT 10"}),
                json!({
                    "connection": "local-pg",
                    "database": "app_db",
                    "columns": ["id", "name"],
                    "rows": [{"id": "1", "name": "alice"}],
                    "row_count": 1,
                    "truncated": false,
                    "elapsed_ms": 4
                }),
            ),
            make_record(
                "glint_db_execute",
                json!({"sql": "UPDATE users SET active = 1"}),
                json!({
                    "connection": "local-pg",
                    "database": "app_db",
                    "affected_rows": 3,
                    "message": "执行成功",
                    "elapsed_ms": 6
                }),
            ),
        ];

        for record in cases {
            assert!(
                render_db_tool_preview(&record, ThemePreset::default()).is_some(),
                "expected preview for {}",
                record.call.name
            );
        }

        let invalid = ToolRecord {
            result: "执行失败：连接超时".into(),
            success: false,
            ..make_record("glint_db_query", json!({}), json!({}))
        };
        assert!(render_db_tool_preview(&invalid, ThemePreset::default()).is_none());

        let non_db = make_record("glint_workspace_get", json!({}), json!({}));
        assert!(render_db_tool_preview(&non_db, ThemePreset::default()).is_none());
    }
}
