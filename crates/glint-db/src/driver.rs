use crate::model::{ColumnInfo, ConnectionConfig, DbKind, QueryResult, TableInfo};
use mysql::prelude::Queryable;
use rusqlite::types::ValueRef;
use std::time::{Duration, Instant};

const MAX_DISPLAY_BYTES: usize = 65_536;

fn quote_ident(kind: DbKind, ident: &str) -> String {
    match kind {
        DbKind::Sqlite | DbKind::Postgres => format!("\"{}\"", ident.replace('"', "\"\"")),
        DbKind::MySql => format!("`{}`", ident.replace('`', "``")),
    }
}

fn truncate_cell(value: String) -> String {
    if value.chars().count() > MAX_DISPLAY_BYTES {
        let mut short: String = value.chars().take(MAX_DISPLAY_BYTES).collect();
        short.push('…');
        short
    } else {
        value
    }
}

fn format_bytes_hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "0x".into();
    }
    let preview: String = bytes.iter().take(32).map(|b| format!("{b:02x}")).collect();
    if bytes.len() > 32 {
        format!("0x{preview}… ({} bytes)", bytes.len())
    } else {
        format!("0x{preview}")
    }
}

fn format_sqlite_value(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".into(),
        ValueRef::Integer(v) => v.to_string(),
        ValueRef::Real(v) => v.to_string(),
        ValueRef::Text(bytes) => truncate_cell(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => format_bytes_hex(bytes),
    }
}

fn format_mysql_value(value: mysql::Value) -> String {
    match value {
        mysql::Value::NULL => "NULL".into(),
        mysql::Value::Bytes(bytes) => match String::from_utf8(bytes) {
            Ok(text) => truncate_cell(text),
            Err(err) => format_bytes_hex(err.as_bytes()),
        },
        mysql::Value::Int(v) => v.to_string(),
        mysql::Value::UInt(v) => v.to_string(),
        mysql::Value::Float(v) => v.to_string(),
        mysql::Value::Double(v) => v.to_string(),
        mysql::Value::Date(y, m, d, h, min, s, us) => {
            if h == 0 && min == 0 && s == 0 && us == 0 {
                format!("{y:04}-{m:02}-{d:02}")
            } else if us == 0 {
                format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}")
            } else {
                format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}.{us:06}")
            }
        }
        mysql::Value::Time(neg, days, h, min, s, us) => {
            let sign = if neg { "-" } else { "" };
            let total_h = days * 24 + u32::from(h);
            if us == 0 {
                format!("{sign}{total_h:02}:{min:02}:{s:02}")
            } else {
                format!("{sign}{total_h:02}:{min:02}:{s:02}.{us:06}")
            }
        }
    }
}

fn format_postgres_cell(row: &postgres::Row, index: usize) -> String {
    use postgres::types::Type;
    let ty = row.columns()[index].type_();
    if *ty == Type::BOOL {
        return row
            .try_get::<_, Option<bool>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |v| v.to_string());
    }
    if *ty == Type::INT2 {
        return row
            .try_get::<_, Option<i16>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |v| v.to_string());
    }
    if *ty == Type::INT4 {
        return row
            .try_get::<_, Option<i32>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |v| v.to_string());
    }
    if *ty == Type::INT8 {
        return row
            .try_get::<_, Option<i64>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |v| v.to_string());
    }
    if *ty == Type::FLOAT4 {
        return row
            .try_get::<_, Option<f32>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |v| v.to_string());
    }
    if *ty == Type::FLOAT8 {
        return row
            .try_get::<_, Option<f64>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |v| v.to_string());
    }
    if *ty == Type::BYTEA {
        return row
            .try_get::<_, Option<Vec<u8>>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |b| format_bytes_hex(&b));
    }
    if *ty == Type::OID {
        return row
            .try_get::<_, Option<u32>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |v| v.to_string());
    }
    if *ty == Type::TIMESTAMPTZ {
        return row
            .try_get::<_, Option<chrono::DateTime<chrono::FixedOffset>>>(index)
            .ok()
            .flatten()
            .map_or_else(
                || "NULL".into(),
                |dt| {
                    dt.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                },
            );
    }
    if *ty == Type::TIMESTAMP {
        return row
            .try_get::<_, Option<chrono::NaiveDateTime>>(index)
            .ok()
            .flatten()
            .map_or_else(
                || "NULL".into(),
                |dt| dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            );
    }
    if *ty == Type::DATE {
        return row
            .try_get::<_, Option<chrono::NaiveDate>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |d| d.format("%Y-%m-%d").to_string());
    }
    if *ty == Type::TIME {
        return row
            .try_get::<_, Option<chrono::NaiveTime>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |t| t.format("%H:%M:%S").to_string());
    }
    if *ty == Type::UUID {
        return row
            .try_get::<_, Option<uuid::Uuid>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |u| u.to_string());
    }
    if *ty == Type::JSON || *ty == Type::JSONB {
        return row
            .try_get::<_, Option<serde_json::Value>>(index)
            .ok()
            .flatten()
            .map_or_else(|| "NULL".into(), |v| truncate_cell(v.to_string()));
    }
    match row.try_get::<_, Option<String>>(index) {
        Ok(Some(text)) => truncate_cell(text),
        Ok(None) => "NULL".into(),
        Err(_) => format!("<{}>", ty.name()),
    }
}

fn open_sqlite(config: &ConnectionConfig) -> Result<rusqlite::Connection, String> {
    if config.file_path.trim().is_empty() {
        return Err("SQLite 文件路径不能为空".into());
    }
    let conn =
        rusqlite::Connection::open(config.file_path.trim()).map_err(|e| format!("打开 SQLite 失败：{e}"))?;
    let _ = conn.busy_timeout(Duration::from_secs(3));
    Ok(conn)
}

fn open_mysql(config: &ConnectionConfig, database: Option<&str>) -> Result<mysql::Conn, String> {
    let db = database
        .filter(|d| !d.trim().is_empty())
        .or_else(|| (!config.database.trim().is_empty()).then_some(config.database.trim()));
    let mut builder = mysql::OptsBuilder::new()
        .ip_or_hostname(Some(config.host.trim()))
        .tcp_port(if config.port == 0 { 3306 } else { config.port })
        .user(Some(config.username.trim()))
        .tcp_connect_timeout(Some(Duration::from_secs(5)))
        .read_timeout(Some(Duration::from_secs(15)))
        .write_timeout(Some(Duration::from_secs(15)));
    if !config.password.is_empty() {
        builder = builder.pass(Some(config.password.as_str()));
    }
    if let Some(db) = db {
        builder = builder.db_name(Some(db));
    }
    mysql::Conn::new(builder).map_err(|e| format!("连接 MySQL 失败：{e}"))
}

fn open_postgres(
    config: &ConnectionConfig,
    database: Option<&str>,
) -> Result<postgres::Client, String> {
    let db = database
        .filter(|d| !d.trim().is_empty())
        .or_else(|| (!config.database.trim().is_empty()).then_some(config.database.trim()))
        .unwrap_or("postgres");
    let mut pg = postgres::Config::new();
    pg.host(config.host.trim());
    pg.port(if config.port == 0 { 5432 } else { config.port });
    pg.user(config.username.trim());
    if !config.password.is_empty() {
        pg.password(config.password.as_str());
    }
    pg.dbname(db);
    pg.connect_timeout(Duration::from_secs(5));
    pg.connect(postgres::NoTls)
        .map_err(|e| format!("连接 PostgreSQL 失败：{e}"))
}

fn looks_like_query(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    let first = trimmed
        .split(|c: char| c.is_whitespace() || c == '(' || c == ';')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(
        first.as_str(),
        "SELECT" | "WITH" | "PRAGMA" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN" | "VALUES" | "TABLE"
    )
}

pub fn test_connection(config: &ConnectionConfig) -> Result<String, String> {
    config.validate()?;
    let started = Instant::now();
    match config.kind {
        DbKind::Sqlite => {
            let conn = open_sqlite(config)?;
            let version: String = conn
                .query_row("SELECT sqlite_version()", [], |r| r.get(0))
                .map_err(|e| format!("查询版本失败：{e}"))?;
            Ok(format!(
                "连接成功（SQLite {version}，耗时 {} ms）",
                started.elapsed().as_millis()
            ))
        }
        DbKind::MySql => {
            let mut conn = open_mysql(config, None)?;
            let version: String = conn
                .query_first("SELECT VERSION()")
                .map_err(|e| format!("查询版本失败：{e}"))?
                .unwrap_or_else(|| "MySQL".into());
            Ok(format!(
                "连接成功（{version}，耗时 {} ms）",
                started.elapsed().as_millis()
            ))
        }
        DbKind::Postgres => {
            let mut client = open_postgres(config, None)?;
            let row = client
                .query_one("SELECT current_database()", &[])
                .map_err(|e| format!("查询数据库失败：{e}"))?;
            let db: String = row.get(0);
            Ok(format!(
                "连接成功（数据库 {db}，耗时 {} ms）",
                started.elapsed().as_millis()
            ))
        }
    }
}

pub fn list_databases(config: &ConnectionConfig) -> Result<Vec<String>, String> {
    match config.kind {
        DbKind::Sqlite => Ok(vec!["main".into()]),
        DbKind::MySql => {
            let mut conn = open_mysql(config, None)?;
            conn.query::<String, _>("SHOW DATABASES")
                .map_err(|e| format!("获取数据库列表失败：{e}"))
        }
        DbKind::Postgres => {
            let mut client = open_postgres(config, None)?;
            let rows = client
                .query(
                    "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
                    &[],
                )
                .map_err(|e| format!("获取数据库列表失败：{e}"))?;
            Ok(rows.into_iter().map(|r| r.get(0)).collect())
        }
    }
}

pub fn list_tables(
    config: &ConnectionConfig,
    database: Option<&str>,
) -> Result<Vec<TableInfo>, String> {
    match config.kind {
        DbKind::Sqlite => {
            let conn = open_sqlite(config)?;
            let mut stmt = conn
                .prepare(
                    "SELECT name, type FROM sqlite_master \
                     WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' \
                     ORDER BY type, name",
                )
                .map_err(|e| format!("读取表列表失败：{e}"))?;
            let rows = stmt
                .query_map([], |row| {
                    let name: String = row.get(0)?;
                    let kind: String = row.get(1)?;
                    Ok(TableInfo {
                        name,
                        is_view: kind == "view",
                    })
                })
                .map_err(|e| format!("读取表列表失败：{e}"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("读取表列表失败：{e}"))
        }
        DbKind::MySql => {
            let mut conn = open_mysql(config, database)?;
            let db = database
                .filter(|d| !d.trim().is_empty())
                .or_else(|| (!config.database.trim().is_empty()).then_some(config.database.trim()))
                .ok_or("请先选择要查看的 MySQL 数据库")?;
            let rows: Vec<(String, String)> = conn
                .exec(
                    "SELECT TABLE_NAME, TABLE_TYPE FROM information_schema.TABLES \
                     WHERE TABLE_SCHEMA = ? ORDER BY TABLE_TYPE, TABLE_NAME",
                    (db,),
                )
                .map_err(|e| format!("读取表列表失败：{e}"))?;
            Ok(rows
                .into_iter()
                .map(|(name, kind)| TableInfo {
                    name,
                    is_view: kind.eq_ignore_ascii_case("VIEW"),
                })
                .collect())
        }
        DbKind::Postgres => {
            let mut client = open_postgres(config, database)?;
            let rows = client
                .query(
                    "SELECT table_name, table_type FROM information_schema.tables \
                     WHERE table_schema = 'public' ORDER BY table_type, table_name",
                    &[],
                )
                .map_err(|e| format!("读取表列表失败：{e}"))?;
            Ok(rows
                .into_iter()
                .map(|r| {
                    let name: String = r.get(0);
                    let kind: String = r.get(1);
                    TableInfo {
                        name,
                        is_view: kind.eq_ignore_ascii_case("VIEW"),
                    }
                })
                .collect())
        }
    }
}

pub fn describe_table(
    config: &ConnectionConfig,
    database: Option<&str>,
    table: &str,
) -> Result<Vec<ColumnInfo>, String> {
    match config.kind {
        DbKind::Sqlite => {
            let conn = open_sqlite(config)?;
            let sql = format!("PRAGMA table_info({})", quote_ident(DbKind::Sqlite, table));
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| format!("读取表结构失败：{e}"))?;
            let rows = stmt
                .query_map([], |row| {
                    let name: String = row.get(1)?;
                    let data_type: String = row.get(2)?;
                    let notnull: i64 = row.get(3)?;
                    let dflt = format_sqlite_value(row.get_ref(4)?);
                    let pk: i64 = row.get(5)?;
                    Ok(ColumnInfo {
                        name,
                        data_type: if data_type.is_empty() {
                            "ANY".into()
                        } else {
                            data_type
                        },
                        nullable: notnull == 0,
                        primary_key: pk > 0,
                        default_value: if dflt == "NULL" { String::new() } else { dflt },
                    })
                })
                .map_err(|e| format!("读取表结构失败：{e}"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("读取表结构失败：{e}"))
        }
        DbKind::MySql => {
            let mut conn = open_mysql(config, database)?;
            let db = database
                .filter(|d| !d.trim().is_empty())
                .or_else(|| (!config.database.trim().is_empty()).then_some(config.database.trim()))
                .ok_or("请先选择 MySQL 数据库")?;
            let rows: Vec<(String, String, String, String, Option<String>)> = conn
                .exec(
                    "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, COLUMN_DEFAULT \
                     FROM information_schema.COLUMNS \
                     WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                     ORDER BY ORDINAL_POSITION",
                    (db, table),
                )
                .map_err(|e| format!("读取表结构失败：{e}"))?;
            Ok(rows
                .into_iter()
                .map(|(name, data_type, nullable, key, default)| ColumnInfo {
                    name,
                    data_type,
                    nullable: nullable.eq_ignore_ascii_case("YES"),
                    primary_key: key.eq_ignore_ascii_case("PRI"),
                    default_value: default.unwrap_or_default(),
                })
                .collect())
        }
        DbKind::Postgres => {
            let mut client = open_postgres(config, database)?;
            let rows = client
                .query(
                    "SELECT c.column_name, c.data_type, c.is_nullable, c.column_default, \
                     EXISTS ( \
                       SELECT 1 FROM information_schema.table_constraints tc \
                       JOIN information_schema.key_column_usage kcu \
                         ON tc.constraint_name = kcu.constraint_name \
                        AND tc.table_schema = kcu.table_schema \
                       WHERE tc.table_schema = c.table_schema \
                         AND tc.table_name = c.table_name \
                         AND kcu.column_name = c.column_name \
                         AND tc.constraint_type = 'PRIMARY KEY' \
                     ) AS is_pk \
                     FROM information_schema.columns c \
                     WHERE c.table_schema = 'public' AND c.table_name = $1 \
                     ORDER BY c.ordinal_position",
                    &[&table],
                )
                .map_err(|e| format!("读取表结构失败：{e}"))?;
            Ok(rows
                .into_iter()
                .map(|r| ColumnInfo {
                    name: r.get(0),
                    data_type: r.get(1),
                    nullable: r.get::<_, String>(2).eq_ignore_ascii_case("YES"),
                    default_value: r.get::<_, Option<String>>(3).unwrap_or_default(),
                    primary_key: r.get(4),
                })
                .collect())
        }
    }
}

pub fn fetch_table_rows(
    config: &ConnectionConfig,
    database: Option<&str>,
    table: &str,
    page: usize,
    page_size: usize,
) -> Result<QueryResult, String> {
    let page = page.max(1);
    let page_size = page_size.clamp(1, 500);
    let offset = (page - 1) * page_size;
    let quoted = quote_ident(config.kind, table);
    let started = Instant::now();

    let count_sql = format!("SELECT COUNT(*) FROM {quoted}");
    let select_sql = format!("SELECT * FROM {quoted} LIMIT {page_size} OFFSET {offset}");

    let total_rows = match config.kind {
        DbKind::Sqlite => {
            let conn = open_sqlite(config)?;
            conn.query_row(&count_sql, [], |r| r.get::<_, i64>(0))
                .ok()
                .map(|n| n.max(0) as u64)
        }
        DbKind::MySql => {
            let mut conn = open_mysql(config, database)?;
            conn.query_first::<u64, _>(&count_sql).ok().flatten()
        }
        DbKind::Postgres => {
            let mut client = open_postgres(config, database)?;
            client
                .query_one(&count_sql, &[])
                .ok()
                .map(|r| r.get::<_, i64>(0).max(0) as u64)
        }
    };

    let mut result = execute_query(config, database, &select_sql)?;
    result.total_rows = total_rows;
    result.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(result)
}

pub fn execute_query(
    config: &ConnectionConfig,
    database: Option<&str>,
    sql: &str,
) -> Result<QueryResult, String> {
    let sql = sql.trim();
    if sql.is_empty() {
        return Err("SQL 语句不能为空".into());
    }
    let started = Instant::now();
    match config.kind {
        DbKind::Sqlite => {
            let conn = open_sqlite(config)?;
            if looks_like_query(sql) {
                let mut stmt = conn
                    .prepare(sql)
                    .map_err(|e| format!("SQL 语法或执行错误：{e}"))?;
                let columns: Vec<String> =
                    stmt.column_names().iter().map(|s| (*s).to_owned()).collect();
                let col_count = columns.len();
                let mut rows_iter = stmt
                    .query([])
                    .map_err(|e| format!("查询执行失败：{e}"))?;
                let mut rows = Vec::new();
                while let Some(row) = rows_iter
                    .next()
                    .map_err(|e| format!("读取结果行失败：{e}"))?
                {
                    let mut cells = Vec::with_capacity(col_count);
                    for i in 0..col_count {
                        let cell = row
                            .get_ref(i)
                            .map(format_sqlite_value)
                            .unwrap_or_else(|_| "<err>".into());
                        cells.push(cell);
                    }
                    rows.push(cells);
                }
                Ok(QueryResult {
                    columns,
                    rows,
                    total_rows: None,
                    affected_rows: None,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    message: None,
                })
            } else {
                let affected = conn
                    .execute(sql, [])
                    .map_err(|e| format!("执行失败：{e}"))?;
                Ok(QueryResult {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    total_rows: None,
                    affected_rows: Some(affected as u64),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    message: Some(format!("执行成功，影响 {affected} 行")),
                })
            }
        }
        DbKind::MySql => {
            let mut conn = open_mysql(config, database)?;
            if looks_like_query(sql) {
                let result = conn
                    .query_iter(sql)
                    .map_err(|e| format!("MySQL 查询失败：{e}"))?;
                let columns: Vec<String> = result
                    .columns()
                    .as_ref()
                    .iter()
                    .map(|c| c.name_str().into_owned())
                    .collect();
                let col_count = columns.len();
                let mut rows = Vec::new();
                for row_res in result {
                    let row = row_res.map_err(|e| format!("读取行失败：{e}"))?;
                    let mut cells = Vec::with_capacity(col_count);
                    for i in 0..col_count {
                        let val = row
                            .get_opt::<mysql::Value, usize>(i)
                            .transpose()
                            .ok()
                            .flatten()
                            .unwrap_or(mysql::Value::NULL);
                        cells.push(format_mysql_value(val));
                    }
                    rows.push(cells);
                }
                Ok(QueryResult {
                    columns,
                    rows,
                    total_rows: None,
                    affected_rows: None,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    message: None,
                })
            } else {
                conn.query_drop(sql)
                    .map_err(|e| format!("MySQL 执行失败：{e}"))?;
                let affected = conn.affected_rows();
                Ok(QueryResult {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    total_rows: None,
                    affected_rows: Some(affected),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    message: Some(format!("执行成功，影响 {affected} 行")),
                })
            }
        }
        DbKind::Postgres => {
            let mut client = open_postgres(config, database)?;
            if looks_like_query(sql) {
                let pg_rows = client
                    .query(sql, &[])
                    .map_err(|e| format!("PostgreSQL 查询失败：{e}"))?;
                let columns: Vec<String> = pg_rows
                    .first()
                    .map(|r| r.columns().iter().map(|c| c.name().to_owned()).collect())
                    .unwrap_or_default();
                let col_count = columns.len();
                let mut rows = Vec::with_capacity(pg_rows.len());
                for row in &pg_rows {
                    let mut cells = Vec::with_capacity(col_count);
                    for i in 0..col_count {
                        cells.push(format_postgres_cell(row, i));
                    }
                    rows.push(cells);
                }
                Ok(QueryResult {
                    columns,
                    rows,
                    total_rows: None,
                    affected_rows: None,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    message: None,
                })
            } else {
                let affected = client
                    .execute(sql, &[])
                    .map_err(|e| format!("PostgreSQL 执行失败：{e}"))?;
                Ok(QueryResult {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    total_rows: None,
                    affected_rows: Some(affected),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    message: Some(format!("执行成功，影响 {affected} 行")),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn sqlite_driver_lists_tables_describes_columns_paginates_and_queries() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_string_lossy().into_owned();
        let config = ConnectionConfig::sqlite("test-sqlite", &path);

        let status = test_connection(&config).unwrap();
        assert!(status.contains("SQLite"));

        let create = execute_query(
            &config,
            None,
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL)",
        )
        .unwrap();
        assert_eq!(create.affected_rows, Some(0));

        for i in 1..=5 {
            execute_query(
                &config,
                None,
                &format!("INSERT INTO users (id, name, score) VALUES ({i}, 'user-{i}', {i}.5)"),
            )
            .unwrap();
        }

        let tables = list_tables(&config, None).unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "users");
        assert!(!tables[0].is_view);

        let cols = describe_table(&config, None, "users").unwrap();
        assert_eq!(cols.len(), 3);
        assert!(cols[0].primary_key);
        assert_eq!(cols[1].name, "name");
        assert!(!cols[1].nullable);

        let page1 = fetch_table_rows(&config, None, "users", 1, 2).unwrap();
        assert_eq!(page1.total_rows, Some(5));
        assert_eq!(page1.rows.len(), 2);
        assert_eq!(page1.columns, vec!["id", "name", "score"]);

        let page3 = fetch_table_rows(&config, None, "users", 3, 2).unwrap();
        assert_eq!(page3.rows.len(), 1);
        assert_eq!(page3.rows[0][1], "user-5");
    }
}
