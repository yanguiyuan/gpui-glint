mod connection_dialog;
pub mod driver;
pub mod model;
pub mod storage;

use connection_dialog::ConnectionForm;
use gpui::{
    AppContext, ClipboardItem, Context, Entity, MouseButton, SharedString, Subscription, Task,
    Window, div, prelude::*, px, rgb,
};
use gpui_glint::components::{
    Button, ButtonKind, ButtonSize, CodeCard, DataTable, Dialog, Icon, IconButton, PaginationBar,
    ResizableDivider, SegmentOption, SegmentedControl, Select, SelectEvent, SelectOption,
    ShellRenderer, TableCell, TableColumn, TableEvent, TableRow, TextEntry, Toast, ToastHost,
    ToastKind, WorkspaceSlots,
};
use gpui_glint::theme::ThemePreset;
use model::{ColumnInfo, ConnectionConfig, DbKind, QueryResult, TableInfo, WorkspaceTab};
use std::path::PathBuf;
use uuid::Uuid;

const SIDEBAR_MIN_WIDTH: f32 = 220.;
const SIDEBAR_MAX_WIDTH: f32 = 460.;
const QUERY_EDITOR_MIN_HEIGHT: f32 = 96.;
const QUERY_EDITOR_MAX_HEIGHT: f32 = 420.;

struct CellMenuState {
    row_index: usize,
    column_label: String,
    value: String,
    position: gpui::Point<gpui::Pixels>,
    source: CellMenuSource,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellMenuSource {
    Browse,
    Structure,
    Query,
}

struct CellDetailModal {
    title: String,
    raw_value: String,
    viewer: Entity<CodeCard>,
}

pub struct DbWorkspace {
    shell: Option<ShellRenderer>,
    theme: ThemePreset,
    storage_path: PathBuf,
    connections: Vec<ConnectionConfig>,
    active_connection: Option<Uuid>,
    databases: Vec<String>,
    selected_database: Option<String>,
    database_select: Entity<Select>,
    tables: Vec<TableInfo>,
    selected_table: Option<String>,
    table_filter: Entity<TextEntry>,
    tab: WorkspaceTab,
    browse_result: Option<QueryResult>,
    browse_table: Option<Entity<DataTable>>,
    browse_page: usize,
    browse_page_size: usize,
    structure_columns: Vec<ColumnInfo>,
    structure_table: Option<Entity<DataTable>>,
    sql_editor: Entity<CodeCard>,
    query_result: Option<QueryResult>,
    query_table: Option<Entity<DataTable>>,
    query_editor_height: f32,
    query_split_resize: Option<(gpui::Pixels, f32)>,
    sidebar_width: f32,
    sidebar_resize: Option<(gpui::Pixels, f32)>,
    loading: bool,
    error: Option<String>,
    dialog: Option<ConnectionForm>,
    cell_menu: Option<CellMenuState>,
    cell_detail: Option<CellDetailModal>,
    toasts: Entity<ToastHost>,
    _table_subscriptions: Vec<Subscription>,
    _load_task: Option<Task<()>>,
    _query_task: Option<Task<()>>,
    _test_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl DbWorkspace {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::new_with_storage(storage::default_path(), None, cx)
    }

    pub fn hosted(shell: ShellRenderer, cx: &mut Context<Self>) -> Self {
        Self::new_with_storage(storage::default_path(), Some(shell), cx)
    }

    pub fn new_with_storage(
        storage_path: PathBuf,
        shell: Option<ShellRenderer>,
        cx: &mut Context<Self>,
    ) -> Self {
        let saved = storage::load(&storage_path);
        let theme = ThemePreset::default();
        let database_select = cx.new(|cx| {
            let mut s = Select::new(Vec::<SelectOption>::new(), cx);
            s.set_theme(theme, cx);
            s
        });
        let table_filter = cx.new(|cx| {
            let mut e = TextEntry::new(cx, "过滤表或视图…", "");
            e.set_theme(theme, cx);
            e
        });
        let sql_editor = cx.new(|cx| {
            let mut card = CodeCard::new(cx, "sql", "SELECT 1;");
            card.set_theme(theme, cx);
            card
        });
        let toasts = cx.new(|cx| {
            let mut q = ToastHost::new();
            q.set_theme(theme, cx);
            q
        });

        let subscriptions = vec![
            cx.subscribe(&database_select, |this, _, event: &SelectEvent, cx| {
                let SelectEvent::Changed(db) = event;
                this.select_database(db.to_string(), cx);
            }),
            cx.observe(&table_filter, |_, _, cx| cx.notify()),
        ];

        let active_connection = saved
            .active_connection
            .filter(|id| saved.connections.iter().any(|c| c.id == *id))
            .or_else(|| saved.connections.first().map(|c| c.id));
        let sidebar_width = saved
            .sidebar_width
            .unwrap_or(260.)
            .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);

        let mut workspace = Self {
            shell,
            theme,
            storage_path,
            connections: saved.connections,
            active_connection,
            databases: Vec::new(),
            selected_database: None,
            database_select,
            tables: Vec::new(),
            selected_table: None,
            table_filter,
            tab: WorkspaceTab::Browse,
            browse_result: None,
            browse_table: None,
            browse_page: 1,
            browse_page_size: 50,
            structure_columns: Vec::new(),
            structure_table: None,
            sql_editor,
            query_result: None,
            query_table: None,
            query_editor_height: 170.,
            query_split_resize: None,
            sidebar_width,
            sidebar_resize: None,
            loading: false,
            error: None,
            dialog: None,
            cell_menu: None,
            cell_detail: None,
            toasts,
            _table_subscriptions: Vec::new(),
            _load_task: None,
            _query_task: None,
            _test_task: None,
            _subscriptions: subscriptions,
        };

        if let Some(id) = workspace.active_connection {
            workspace.connect_and_load(id, cx);
        }
        workspace
    }

    pub fn host_theme(&self) -> ThemePreset {
        self.theme
    }

    pub fn set_host_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        self.database_select.update(cx, |s, cx| s.set_theme(theme, cx));
        self.table_filter.update(cx, |e, cx| e.set_theme(theme, cx));
        self.sql_editor.update(cx, |c, cx| c.set_theme(theme, cx));
        self.toasts.update(cx, |t, cx| t.set_theme(theme, cx));
        if let Some(table) = &self.browse_table {
            table.update(cx, |t, cx| t.set_theme(theme, cx));
        }
        if let Some(table) = &self.structure_table {
            table.update(cx, |t, cx| t.set_theme(theme, cx));
        }
        if let Some(table) = &self.query_table {
            table.update(cx, |t, cx| t.set_theme(theme, cx));
        }
        if let Some(form) = &self.dialog {
            form.set_theme(theme, cx);
        }
        if let Some(detail) = &self.cell_detail {
            detail.viewer.update(cx, |v, cx| v.set_theme(theme, cx));
        }
        cx.notify();
    }

    fn active_config(&self) -> Option<&ConnectionConfig> {
        let id = self.active_connection?;
        self.connections.iter().find(|c| c.id == id)
    }

    fn persist(&self) {
        let state = storage::SavedWorkspace {
            connections: self.connections.clone(),
            active_connection: self.active_connection,
            sidebar_width: Some(self.sidebar_width),
        };
        let _ = storage::save(&self.storage_path, &state);
    }

    fn push_toast(&self, message: impl Into<SharedString>, kind: ToastKind, cx: &mut Context<Self>) {
        let msg = message.into();
        self.toasts.update(cx, |q, cx| {
            q.push(Toast::new(msg).kind(kind), cx);
        });
    }

    pub fn open_new_connection(&mut self, kind: DbKind, cx: &mut Context<Self>) {
        let n = self.connections.len() + 1;
        let config = ConnectionConfig::new_default(kind, format!("{} 连接 {n}", kind.label()));
        self.dialog = Some(ConnectionForm::from_config(&config, None, self.theme, cx));
        cx.notify();
    }

    pub fn open_edit_connection(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(cfg) = self.connections.iter().find(|c| c.id == id).cloned() else {
            return;
        };
        self.dialog = Some(ConnectionForm::from_config(&cfg, Some(id), self.theme, cx));
        cx.notify();
    }

    fn close_dialog(&mut self, cx: &mut Context<Self>) {
        self.dialog = None;
        self._test_task = None;
        cx.notify();
    }

    fn test_dialog_connection(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &mut self.dialog else {
            return;
        };
        let config = match form.to_config(cx) {
            Ok(c) => c,
            Err(err) => {
                form.error = Some(err);
                cx.notify();
                return;
            }
        };
        form.error = None;
        form.testing = true;
        form.test_status = None;
        cx.notify();

        let task = cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { driver::test_connection(&config) })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(form) = &mut this.dialog {
                    form.testing = false;
                    form.test_status = Some(result);
                    cx.notify();
                }
            });
        });
        self._test_task = Some(task);
    }

    fn save_dialog_connection(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &mut self.dialog else {
            return;
        };
        let config = match form.to_config(cx) {
            Ok(c) => c,
            Err(err) => {
                form.error = Some(err);
                cx.notify();
                return;
            }
        };
        let id = config.id;
        if let Some(existing) = self.connections.iter_mut().find(|c| c.id == id) {
            *existing = config;
        } else {
            self.connections.push(config);
        }
        self.dialog = None;
        self._test_task = None;
        self.active_connection = Some(id);
        self.persist();
        self.push_toast("连接已保存", ToastKind::Success, cx);
        self.connect_and_load(id, cx);
    }

    pub fn delete_connection(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.connections.retain(|c| c.id != id);
        if self.active_connection == Some(id) {
            self.active_connection = self.connections.first().map(|c| c.id);
            self.databases.clear();
            self.selected_database = None;
            self.tables.clear();
            self.selected_table = None;
            self.browse_result = None;
            self.browse_table = None;
            self.structure_columns.clear();
            self.structure_table = None;
            if let Some(next) = self.active_connection {
                self.connect_and_load(next, cx);
            }
        }
        self.persist();
        cx.notify();
    }

    pub fn connect_and_load(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.active_connection = Some(id);
        self.persist();
        let Some(config) = self.active_config().cloned() else {
            return;
        };
        self.loading = true;
        self.error = None;
        self.databases.clear();
        self.tables.clear();
        self.selected_table = None;
        self.browse_result = None;
        self.browse_table = None;
        self.structure_columns.clear();
        self.structure_table = None;
        self.selected_database = (!config.database.trim().is_empty())
            .then(|| config.database.trim().to_owned());
        cx.notify();

        let default_db = self.selected_database.clone();
        let task = cx.spawn(async move |weak, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let dbs = driver::list_databases(&config)?;
                    let chosen_db = default_db
                        .filter(|d| dbs.iter().any(|x| x == d))
                        .or_else(|| dbs.first().cloned());
                    let tables = match chosen_db.as_deref() {
                        Some(db) => driver::list_tables(&config, Some(db))?,
                        None if config.kind.is_file_based() => driver::list_tables(&config, None)?,
                        None => Vec::new(),
                    };
                    Ok::<_, String>((dbs, chosen_db, tables))
                })
                .await;

            let _ = weak.update(cx, |this, cx| {
                this.loading = false;
                match outcome {
                    Ok((dbs, chosen_db, tables)) => {
                        this.databases = dbs.clone();
                        this.selected_database = chosen_db.clone();
                        this.database_select.update(cx, |sel, cx| {
                            let opts: Vec<SelectOption> = dbs
                                .into_iter()
                                .map(|d| SelectOption::new(d.clone(), d))
                                .collect();
                            sel.set_options(opts, cx);
                            sel.set_selected(chosen_db.clone().map(SharedString::from), cx);
                        });
                        this.tables = tables;
                        if let Some(first) = this.tables.first().map(|t| t.name.clone()) {
                            this.select_table(first, cx);
                        } else {
                            cx.notify();
                        }
                    }
                    Err(err) => {
                        this.error = Some(err.clone());
                        this.push_toast(err, ToastKind::Error, cx);
                        cx.notify();
                    }
                }
            });
        });
        self._load_task = Some(task);
    }

    pub fn select_database(&mut self, database: String, cx: &mut Context<Self>) {
        if self.selected_database.as_deref() == Some(database.as_str()) {
            return;
        }
        self.selected_database = Some(database.clone());
        self.database_select.update(cx, |s, cx| {
            s.set_selected(Some(SharedString::from(database.clone())), cx);
        });
        let Some(config) = self.active_config().cloned() else {
            return;
        };
        self.loading = true;
        self.error = None;
        self.tables.clear();
        self.selected_table = None;
        self.browse_result = None;
        self.browse_table = None;
        self.structure_columns.clear();
        self.structure_table = None;
        cx.notify();

        let task = cx.spawn(async move |weak, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move { driver::list_tables(&config, Some(&database)) })
                .await;
            let _ = weak.update(cx, |this, cx| {
                this.loading = false;
                match outcome {
                    Ok(tables) => {
                        this.tables = tables;
                        if let Some(first) = this.tables.first().map(|t| t.name.clone()) {
                            this.select_table(first, cx);
                        } else {
                            cx.notify();
                        }
                    }
                    Err(err) => {
                        this.error = Some(err.clone());
                        this.push_toast(err, ToastKind::Error, cx);
                        cx.notify();
                    }
                }
            });
        });
        self._load_task = Some(task);
    }

    pub fn select_table(&mut self, table: String, cx: &mut Context<Self>) {
        self.selected_table = Some(table.clone());
        self.browse_page = 1;
        if self.tab == WorkspaceTab::Query {
            self.tab = WorkspaceTab::Browse;
        }
        let quoted = match self.active_config().map(|c| c.kind) {
            Some(DbKind::MySql) => format!("`{table}`"),
            _ => format!("\"{table}\""),
        };
        self.sql_editor
            .update(cx, |ed, cx| ed.set_content(format!("SELECT * FROM {quoted} LIMIT 100;"), cx));
        self.reload_active_table(cx);
    }

    pub fn reload_active_table(&mut self, cx: &mut Context<Self>) {
        let (Some(config), Some(table)) =
            (self.active_config().cloned(), self.selected_table.clone())
        else {
            return;
        };
        let db = self.selected_database.clone();
        let page = self.browse_page;
        let page_size = self.browse_page_size;
        self.loading = true;
        self.error = None;
        cx.notify();

        let task = cx.spawn(async move |weak, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let rows = driver::fetch_table_rows(&config, db.as_deref(), &table, page, page_size)?;
                    let cols = driver::describe_table(&config, db.as_deref(), &table)?;
                    Ok::<_, String>((rows, cols))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                this.loading = false;
                match outcome {
                    Ok((rows, cols)) => {
                        let browse_table = Self::build_result_table(&rows, this.theme, cx);
                        let structure_table = Self::build_structure_table(&cols, this.theme, cx);
                        this.subscribe_table(&browse_table, CellMenuSource::Browse, cx);
                        this.subscribe_table(&structure_table, CellMenuSource::Structure, cx);
                        this.browse_table = Some(browse_table);
                        this.browse_result = Some(rows);
                        this.structure_table = Some(structure_table);
                        this.structure_columns = cols;
                    }
                    Err(err) => {
                        this.error = Some(err.clone());
                        this.push_toast(err, ToastKind::Error, cx);
                    }
                }
                cx.notify();
            });
        });
        self._load_task = Some(task);
    }

    pub fn run_sql_query(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.active_config().cloned() else {
            self.push_toast("请先创建并选择一个数据库连接", ToastKind::Warning, cx);
            return;
        };
        let db = self.selected_database.clone();
        let sql = self.sql_editor.read(cx).content().to_owned();
        self.loading = true;
        self.error = None;
        cx.notify();

        let task = cx.spawn(async move |weak, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move { driver::execute_query(&config, db.as_deref(), &sql) })
                .await;
            let _ = weak.update(cx, |this, cx| {
                this.loading = false;
                match outcome {
                    Ok(res) => {
                        if let Some(msg) = &res.message {
                            this.push_toast(msg.clone(), ToastKind::Success, cx);
                        }
                        this.query_table = if res.columns.is_empty() {
                            None
                        } else {
                            let table = Self::build_result_table(&res, this.theme, cx);
                            this.subscribe_table(&table, CellMenuSource::Query, cx);
                            Some(table)
                        };
                        this.query_result = Some(res);
                    }
                    Err(err) => {
                        this.error = Some(err.clone());
                        this.push_toast(err, ToastKind::Error, cx);
                    }
                }
                cx.notify();
            });
        });
        self._query_task = Some(task);
    }

    fn subscribe_table(
        &mut self,
        table: &Entity<DataTable>,
        source: CellMenuSource,
        cx: &mut Context<Self>,
    ) {
        let sub = cx.subscribe(table, move |this, _, event: &TableEvent, cx| {
            if let TableEvent::CellContextMenu {
                row_index,
                column_label,
                value,
                position,
                ..
            } = event
            {
                this.cell_menu = Some(CellMenuState {
                    row_index: *row_index,
                    column_label: column_label.to_string(),
                    value: value.to_string(),
                    position: *position,
                    source,
                });
                cx.notify();
            }
        });
        self._table_subscriptions.push(sub);
        if self._table_subscriptions.len() > 16 {
            let drain = self._table_subscriptions.len() - 16;
            self._table_subscriptions.drain(..drain);
        }
    }

    fn copy_cell_value(&mut self, value: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(value));
        self.cell_menu = None;
        self.push_toast("已复制单元格值", ToastKind::Success, cx);
        cx.notify();
    }

    fn copy_row_json(&mut self, source: CellMenuSource, row_index: usize, cx: &mut Context<Self>) {
        let json_text = match source {
            CellMenuSource::Browse => self.browse_result.as_ref().and_then(|res| {
                res.rows.get(row_index).map(|row| {
                    let mut map = serde_json::Map::new();
                    for (col, val) in res.columns.iter().zip(row.iter()) {
                        map.insert(col.clone(), serde_json::Value::String(val.clone()));
                    }
                    serde_json::to_string_pretty(&map).unwrap_or_default()
                })
            }),
            CellMenuSource::Query => self.query_result.as_ref().and_then(|res| {
                res.rows.get(row_index).map(|row| {
                    let mut map = serde_json::Map::new();
                    for (col, val) in res.columns.iter().zip(row.iter()) {
                        map.insert(col.clone(), serde_json::Value::String(val.clone()));
                    }
                    serde_json::to_string_pretty(&map).unwrap_or_default()
                })
            }),
            CellMenuSource::Structure => self.structure_columns.get(row_index).map(|col| {
                serde_json::to_string_pretty(&serde_json::json!({
                    "name": col.name,
                    "data_type": col.data_type,
                    "nullable": col.nullable,
                    "primary_key": col.primary_key,
                    "default_value": col.default_value,
                }))
                .unwrap_or_default()
            }),
        };
        if let Some(text) = json_text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.push_toast("已复制整行 JSON", ToastKind::Success, cx);
        }
        self.cell_menu = None;
        cx.notify();
    }

    fn open_cell_detail(&mut self, title: String, value: String, cx: &mut Context<Self>) {
        let trimmed = value.trim();
        let (lang, display) = if (trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
        {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                (
                    "json",
                    serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| value.clone()),
                )
            } else {
                ("text", value.clone())
            }
        } else {
            ("text", value.clone())
        };
        let theme = self.theme;
        let viewer = cx.new(|cx| {
            let mut card = CodeCard::new(cx, lang, display).read_only(true).viewport();
            card.set_theme(theme, cx);
            card
        });
        self.cell_detail = Some(CellDetailModal {
            title,
            raw_value: value,
            viewer,
        });
        self.cell_menu = None;
        cx.notify();
    }

    fn display_text_width(text: &str, char_px: f32) -> f32 {
        let first_line = text.lines().next().unwrap_or("");
        let mut width = 0.0f32;
        for ch in first_line.chars().take(64) {
            width += if ch.is_ascii() { char_px } else { char_px * 1.65 };
        }
        width
    }

    fn compute_column_layout(result: &QueryResult) -> Vec<(f32, bool)> {
        let mut widths: Vec<f32> = result
            .columns
            .iter()
            .enumerate()
            .map(|(col_idx, name)| {
                let header_w = Self::display_text_width(name, 8.2) + 46.;
                let content_w = result
                    .rows
                    .iter()
                    .take(50)
                    .filter_map(|row| row.get(col_idx))
                    .map(|cell| Self::display_text_width(cell, 7.6) + 22.)
                    .fold(0.0f32, f32::max);
                header_w.max(content_w).clamp(110., 460.)
            })
            .collect();

        let total_w: f32 = widths.iter().sum();
        let mut flexible = vec![false; widths.len()];
        if total_w < 880. && !widths.is_empty() {
            let mut any_flex = false;
            for (idx, w) in widths.iter().enumerate() {
                if *w >= 180. {
                    flexible[idx] = true;
                    any_flex = true;
                }
            }
            if !any_flex {
                if let Some((max_idx, _)) = widths
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                {
                    flexible[max_idx] = true;
                }
            }
            let _ = &mut widths;
        }

        widths.into_iter().zip(flexible).collect()
    }

    fn build_result_table(
        result: &QueryResult,
        theme: ThemePreset,
        cx: &mut Context<Self>,
    ) -> Entity<DataTable> {
        let layout = Self::compute_column_layout(result);
        let columns: Vec<TableColumn> = result
            .columns
            .iter()
            .zip(layout)
            .map(|(name, (width, is_flexible))| {
                let mut col = TableColumn::new(name.clone(), width).sortable();
                if is_flexible {
                    col = col.flexible();
                }
                col
            })
            .collect();
        let rows: Vec<TableRow> = result
            .rows
            .iter()
            .enumerate()
            .map(|(row_idx, cells)| {
                TableRow::new(
                    format!("row-{row_idx}"),
                    cells.iter().map(|v| TableCell::mono(v.clone())),
                )
            })
            .collect();
        let page_size = rows.len().max(100);
        cx.new(|cx| {
            DataTable::new(cx, columns, rows)
                .compact()
                .document()
                .fill_height()
                .page_size(page_size)
                .theme(theme)
        })
    }

    fn build_structure_table(
        columns: &[ColumnInfo],
        theme: ThemePreset,
        cx: &mut Context<Self>,
    ) -> Entity<DataTable> {
        let table_cols = vec![
            TableColumn::new("字段名", 180.).sortable(),
            TableColumn::new("类型", 160.),
            TableColumn::new("允许空", 90.),
            TableColumn::new("主键", 80.),
            TableColumn::new("默认值", 180.).flexible(),
        ];
        let rows: Vec<TableRow> = columns
            .iter()
            .enumerate()
            .map(|(idx, col)| {
                TableRow::new(
                    format!("col-{idx}"),
                    [
                        TableCell::mono(col.name.clone()),
                        TableCell::mono(col.data_type.clone()),
                        TableCell::text(if col.nullable { "是" } else { "否" }),
                        TableCell::text(if col.primary_key { "PK" } else { "-" }),
                        TableCell::mono(if col.default_value.is_empty() {
                            "-".to_owned()
                        } else {
                            col.default_value.clone()
                        }),
                    ],
                )
            })
            .collect();
        let page_size = rows.len().max(100);
        cx.new(|cx| {
            DataTable::new(cx, table_cols, rows)
                .compact()
                .document()
                .fill_height()
                .page_size(page_size)
                .theme(theme)
        })
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let filter = self.table_filter.read(cx).value().trim().to_lowercase();
        let filtered_tables: Vec<&TableInfo> = self
            .tables
            .iter()
            .filter(|t| filter.is_empty() || t.name.to_lowercase().contains(&filter))
            .collect();

        let mut conn_list = div().flex().flex_col().gap_1().px_2().pb_2();
        if self.connections.is_empty() {
            conn_list = conn_list.child(
                div()
                    .px_2()
                    .py_3()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("暂无连接，点击右上角 + 新建 SQLite / MySQL / PostgreSQL 连接"),
            );
        }
        for conn in &self.connections {
            let active = Some(conn.id) == self.active_connection;
            let conn_id = conn.id;
            conn_list = conn_list.child(
                div()
                    .id(SharedString::from(format!("db-conn-{}", conn.id)))
                    .px_2p5()
                    .py_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(if active { p.accent } else { p.border }))
                    .bg(rgb(if active { p.accent_tint } else { p.surface }))
                    .cursor_pointer()
                    .hover(move |s| {
                        if active {
                            s
                        } else {
                            s.bg(rgb(p.surface_subtle))
                        }
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .px_1p5()
                                    .py(px(1.))
                                    .rounded(px(4.))
                                    .bg(rgb(p.accent_tint))
                                    .text_size(px(10.))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(p.accent))
                                    .child(conn.kind.short_badge()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(conn.name.clone()),
                            )
                            .child(
                                IconButton::new(Icon::Pencil, "编辑连接")
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.open_edit_connection(conn_id, cx);
                                    })),
                            )
                            .child(
                                IconButton::new(Icon::Trash, "删除连接")
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.delete_connection(conn_id, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .mt_1()
                            .font_family("Menlo")
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .truncate()
                            .child(conn.summary()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.connect_and_load(conn_id, cx);
                    })),
            );
        }

        let mut table_list = div()
            .id("db-tables-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .pb_3()
            .flex()
            .flex_col()
            .gap_0p5();

        if self.active_connection.is_some() && filtered_tables.is_empty() && !self.loading {
            table_list = table_list.child(
                div()
                    .px_2()
                    .py_3()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(if self.tables.is_empty() {
                        "当前库暂无表或视图"
                    } else {
                        "没有匹配的表"
                    }),
            );
        }

        for table in filtered_tables {
            let selected = self.selected_table.as_deref() == Some(table.name.as_str());
            let name = table.name.clone();
            table_list = table_list.child(
                div()
                    .id(SharedString::from(format!("db-table-{}", table.name)))
                    .px_2p5()
                    .py_1p5()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(rgb(p.accent_tint)).text_color(rgb(p.accent)))
                    .when(!selected, |d| {
                        d.text_color(rgb(p.text))
                            .hover(|s| s.bg(rgb(p.surface_subtle)))
                    })
                    .child(
                        Icon::Database
                            .render(13.)
                            .text_color(rgb(if selected { p.accent } else { p.text_muted }))
                            .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .child(table.name.clone()),
                    )
                    .when(table.is_view, |d| {
                        d.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child("视图"),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_table(name.clone(), cx);
                    })),
            );
        }

        div()
            .w(px(self.sidebar_width))
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .child(
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_size(px(13.))
                            .child("数据库连接"),
                    )
                    .child(
                        IconButton::new(Icon::Plus, "新建连接")
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "db-create-connection".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_new_connection(DbKind::Sqlite, cx);
                            })),
                    ),
            )
            .child(conn_list)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(p.text_muted))
                                    .child("数据表与视图"),
                            )
                            .child(
                                IconButton::new(Icon::Refresh, "刷新表列表")
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(id) = this.active_connection {
                                            this.connect_and_load(id, cx);
                                        }
                                    })),
                            ),
                    )
                    .child(self.table_filter.clone()),
            )
            .child(table_list)
    }

    fn render_topbar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let p = self.theme.palette();
        let active_name = self
            .active_config()
            .map(|c| format!("{} ({})", c.name, c.kind.label()))
            .unwrap_or_else(|| "未选择连接".into());

        div()
            .size_full()
            .flex()
            .items_center()
            .gap_3()
            .pr_3()
            .text_size(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::Database.render(15.).text_color(rgb(p.accent)))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(rgb(p.text))
                            .child(active_name),
                    ),
            )
            .when(!self.databases.is_empty(), |bar| {
                bar.child(div().w(px(180.)).child(self.database_select.clone()))
            })
            .child(div().flex_1())
            .child(
                Button::new("SQL 查询")
                    .kind(ButtonKind::Glass)
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tab = WorkspaceTab::Query;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("新建连接")
                    .kind(ButtonKind::Primary)
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_new_connection(DbKind::Sqlite, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_main(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        if self.active_connection.is_none() {
            return div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child(Icon::Database.render(36.).text_color(rgb(p.text_muted)))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child("数据库管理"),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child("支持 SQLite、MySQL 与 PostgreSQL 连接、表浏览与 SQL 查询"),
                )
                .child(
                    Button::new("新建数据库连接")
                        .kind(ButtonKind::Primary)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_new_connection(DbKind::Sqlite, cx);
                        })),
                );
        }

        let weak_tab = cx.entity().downgrade();
        let header = div()
            .h(px(44.))
            .flex_shrink_0()
            .px_3()
            .border_b_1()
            .border_color(rgb(p.border))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                SegmentedControl::new([
                    SegmentOption::new("数据浏览").debug_selector("db-tab-browse"),
                    SegmentOption::new("表结构").debug_selector("db-tab-structure"),
                    SegmentOption::new("SQL 查询").debug_selector("db-tab-query"),
                ])
                .id("db-workspace-tabs")
                .compact()
                .selected(self.tab.index())
                .theme(self.theme)
                .on_change(move |idx, _, cx| {
                    let _ = weak_tab.update(cx, |this, cx| {
                        this.tab = WorkspaceTab::from_index(idx);
                        cx.notify();
                    });
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(self.selected_table.clone(), |bar, table| {
                        bar.child(
                            div()
                                .text_size(px(12.))
                                .font_family("Menlo")
                                .text_color(rgb(p.text_muted))
                                .child(table),
                        )
                    })
                    .when(self.loading, |bar| {
                        bar.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.accent))
                                .child("加载中…"),
                        )
                    }),
            );

        let body: gpui::AnyElement = match self.tab {
            WorkspaceTab::Browse => {
                let mut content = div().flex_1().min_h_0().flex().flex_col();
                if let Some(table) = &self.browse_table {
                    content = content.child(div().flex_1().min_h_0().p_2().child(table.clone()));
                    if let Some(res) = &self.browse_result {
                        let weak = cx.entity().downgrade();
                        let weak_size = cx.entity().downgrade();
                        content = content.child(
                            PaginationBar::new(self.browse_page, self.browse_page_size, res.rows.len())
                                .total_items(res.total_rows)
                                .theme(self.theme)
                                .on_page_change(move |next_page, _, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.browse_page = next_page;
                                        this.reload_active_table(cx);
                                    });
                                })
                                .on_page_size_change(move |next_size, _, cx| {
                                    let _ = weak_size.update(cx, |this, cx| {
                                        this.browse_page_size = next_size;
                                        this.browse_page = 1;
                                        this.reload_active_table(cx);
                                    });
                                })
                                .render(),
                        );
                    }
                } else {
                    content = content.child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .text_color(rgb(p.text_muted))
                            .child("从左侧选择一张表查看数据"),
                    );
                }
                content.into_any_element()
            }
            WorkspaceTab::Structure => {
                if let Some(table) = &self.structure_table {
                    div()
                        .flex_1()
                        .min_h_0()
                        .p_2()
                        .child(table.clone())
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child("从左侧选择一张表查看字段结构")
                        .into_any_element()
                }
            }
            WorkspaceTab::Query => {
                let stats = self.query_result.as_ref().map(|r| {
                    if let Some(msg) = &r.message {
                        format!("{msg} · {} ms", r.elapsed_ms)
                    } else {
                        format!("返回 {} 行 · 耗时 {} ms", r.rows.len(), r.elapsed_ms)
                    }
                });
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(self.query_editor_height))
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .p_2()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        Button::new("执行 SQL")
                                            .icon(Icon::Play)
                                            .kind(ButtonKind::Primary)
                                            .size(ButtonSize::Small)
                                            .theme(self.theme)
                                            .render()
                                            .debug_selector(|| "db-run-sql".into())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.run_sql_query(cx);
                                            })),
                                    )
                                    .when_some(stats, |row, text| {
                                        row.child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(rgb(p.text_muted))
                                                .child(text),
                                        )
                                    }),
                            )
                            .child(div().flex_1().min_h_0().child(self.sql_editor.clone())),
                    )
                    .child(
                        ResizableDivider::vertical("db-query-split")
                            .theme(self.theme)
                            .dragging(self.query_split_resize.is_some())
                            .on_drag_start(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                                this.query_split_resize =
                                    Some((event.position.y, this.query_editor_height));
                                cx.stop_propagation();
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .p_2()
                            .child(if let Some(table) = &self.query_table {
                                table.clone().into_any_element()
                            } else if let Some(err) = &self.error {
                                div()
                                    .p_3()
                                    .rounded(px(8.))
                                    .bg(rgb(p.surface_subtle))
                                    .text_size(px(12.))
                                    .text_color(rgb(0xd9342b))
                                    .child(err.clone())
                                    .into_any_element()
                            } else {
                                div()
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text_muted))
                                    .child("输入 SQL 并点击“执行 SQL”查看结果")
                                    .into_any_element()
                            }),
                    )
                    .into_any_element()
            }
        };

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(p.workspace))
            .child(header)
            .child(body)
    }
}

impl Render for DbWorkspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let sidebar = self.render_sidebar(cx);
        let resize_handle = ResizableDivider::horizontal("db-sidebar-resize")
            .debug_selector("db-sidebar-resize")
            .theme(self.theme)
            .dragging(self.sidebar_resize.is_some())
            .on_drag_start(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                this.sidebar_resize = Some((event.position.x, this.sidebar_width));
                cx.stop_propagation();
            }));
        let content = self.render_main(cx);

        let main = if let Some(shell) = &self.shell {
            shell(WorkspaceSlots {
                topbar: self.render_topbar(cx),
                sidebar: sidebar.into_any_element(),
                content: content.into_any_element(),
                sidebar_width: self.sidebar_width,
                sidebar_visible: true,
                sidebar_resize_handle: Some(resize_handle.into_any_element()),
                theme: self.theme,
            })
        } else {
            div()
                .size_full()
                .flex()
                .min_h_0()
                .min_w_0()
                .child(sidebar)
                .child(resize_handle)
                .child(content)
                .into_any_element()
        };

        let weak_kind = cx.entity().downgrade();
        let weak_test = cx.entity().downgrade();
        let weak_cancel = cx.entity().downgrade();
        let weak_save = cx.entity().downgrade();
        let dialog = self.dialog.as_ref().map(|form| {
            connection_dialog::render_connection_dialog(
                form,
                self.theme,
                move |kind, _, cx| {
                    let _ = weak_kind.update(cx, |this, cx| {
                        if let Some(f) = &mut this.dialog {
                            f.set_kind(kind, cx);
                            cx.notify();
                        }
                    });
                },
                move |_, _, cx| {
                    let _ = weak_test.update(cx, |this, cx| this.test_dialog_connection(cx));
                },
                move |_, cx| {
                    let _ = weak_cancel.update(cx, |this, cx| this.close_dialog(cx));
                },
                move |_, _, cx| {
                    let _ = weak_save.update(cx, |this, cx| this.save_dialog_connection(cx));
                },
                cx,
            )
            .render()
        });

        let cell_menu_overlay = self.cell_menu.as_ref().map(|menu| {
            let value_copy = menu.value.clone();
            let value_detail = menu.value.clone();
            let title_detail = menu.column_label.clone();
            let row_index = menu.row_index;
            let source = menu.source;
            let pos = menu.position;
            div()
                .id("db-cell-menu-backdrop")
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.cell_menu = None;
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, _, cx| {
                        this.cell_menu = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .id("db-cell-context-menu")
                        .absolute()
                        .left(pos.x)
                        .top(pos.y)
                        .w(px(172.))
                        .p_1()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.surface))
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .text_size(px(12.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .id("db-ctx-copy-cell")
                                .px_2p5()
                                .py_1p5()
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(p.surface_subtle)))
                                .child("复制单元格值")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_cell_value(value_copy.clone(), cx);
                                })),
                        )
                        .child(
                            div()
                                .id("db-ctx-copy-row")
                                .px_2p5()
                                .py_1p5()
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(p.surface_subtle)))
                                .child("复制整行 (JSON)")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_row_json(source, row_index, cx);
                                })),
                        )
                        .child(
                            div()
                                .id("db-ctx-inspect")
                                .px_2p5()
                                .py_1p5()
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(p.surface_subtle)))
                                .child("查看完整内容")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_cell_detail(
                                        title_detail.clone(),
                                        value_detail.clone(),
                                        cx,
                                    );
                                })),
                        ),
                )
        });

        let weak_detail_close = cx.entity().downgrade();
        let weak_detail_copy = cx.entity().downgrade();
        let cell_detail_dialog = self.cell_detail.as_ref().map(|detail| {
            let raw = detail.raw_value.clone();
            let content = div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .font_family("Menlo")
                                .child(detail.title.clone()),
                        )
                        .child(
                            Button::new("复制全部")
                                .kind(ButtonKind::Glass)
                                .size(ButtonSize::Small)
                                .theme(self.theme)
                                .render()
                                .on_click(move |_, _, cx| {
                                    let _ = weak_detail_copy.update(cx, |this, cx| {
                                        this.copy_cell_value(raw.clone(), cx);
                                    });
                                }),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(420.))
                        .overflow_hidden()
                        .rounded(px(8.))
                        .child(detail.viewer.clone()),
                );
            Dialog::new("db-cell-detail-dialog", content)
                .width(640.)
                .theme(self.theme)
                .on_close(move |_, _, cx| {
                    let _ = weak_detail_close.update(cx, |this, cx| {
                        this.cell_detail = None;
                        cx.notify();
                    });
                })
                .render()
        });

        div()
            .relative()
            .size_full()
            .bg(rgb(p.workspace))
            .text_color(rgb(p.text))
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if let Some((start_x, start_w)) = this.sidebar_resize {
                    if event.dragging() {
                        this.sidebar_width = (start_w + f32::from(event.position.x - start_x))
                            .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
                    } else {
                        this.sidebar_resize = None;
                    }
                    cx.stop_propagation();
                    cx.notify();
                } else if let Some((start_y, start_h)) = this.query_split_resize {
                    if event.dragging() {
                        this.query_editor_height = (start_h + f32::from(event.position.y - start_y))
                            .clamp(QUERY_EDITOR_MIN_HEIGHT, QUERY_EDITOR_MAX_HEIGHT);
                    } else {
                        this.query_split_resize = None;
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    if this.sidebar_resize.take().is_some() {
                        this.persist();
                    }
                    this.query_split_resize = None;
                }),
            )
            .child(main)
            .children(dialog)
            .children(cell_menu_overlay)
            .children(cell_detail_dialog)
            .child(self.toasts.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn adaptive_column_layout_expands_content_heavy_columns() {
        let result = QueryResult {
            columns: vec!["resource_id".into(), "text".into()],
            rows: vec![vec![
                "4188".into(),
                "<p>九月份初的北辰区已经能感受到明显的秋意，这是一段较长的富文本内容</p>".into(),
            ]],
            total_rows: Some(1),
            affected_rows: None,
            elapsed_ms: 1,
            message: None,
        };
        let layout = DbWorkspace::compute_column_layout(&result);
        assert_eq!(layout.len(), 2);
        assert!(layout[0].0 < 160.);
        assert!(!layout[0].1);
        assert!(layout[1].0 >= 300.);
        assert!(layout[1].1);
    }

    #[gpui::test]
    async fn db_workspace_loads_sqlite_tables_and_executes_query(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("sample.sqlite");
        let storage_path = dir.path().join("connections.json");

        let cfg = ConnectionConfig::sqlite("sample", db_path.to_string_lossy().as_ref());
        driver::execute_query(
            &cfg,
            None,
            "CREATE TABLE items (id INTEGER PRIMARY KEY, label TEXT)",
        )
        .unwrap();
        driver::execute_query(&cfg, None, "INSERT INTO items (id, label) VALUES (1, 'alpha')")
            .unwrap();
        storage::save(
            &storage_path,
            &storage::SavedWorkspace {
                connections: vec![cfg.clone()],
                active_connection: Some(cfg.id),
                sidebar_width: None,
            },
        )
        .unwrap();

        let (workspace, window) =
            cx.add_window_view(|_, cx| DbWorkspace::new_with_storage(storage_path, None, cx));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            window.run_until_parked();
            let ready = workspace.read_with(window, |ws, _| {
                ws.tables.iter().any(|t| t.name == "items")
                    && ws.browse_result.as_ref().is_some_and(|r| r.rows.len() == 1)
            });
            if ready {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out loading sqlite table"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        workspace.update(window, |ws, cx| {
            assert_eq!(ws.structure_columns.len(), 2);
            ws.tab = WorkspaceTab::Query;
            ws.sql_editor
                .update(cx, |ed, cx| ed.set_content("SELECT label FROM items", cx));
            ws.run_sql_query(cx);
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            window.run_until_parked();
            let done = workspace.read_with(window, |ws, _| {
                ws.query_result
                    .as_ref()
                    .is_some_and(|r| r.rows.first().and_then(|row| row.first()).map(String::as_str) == Some("alpha"))
            });
            if done {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "timed out running sql query");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
