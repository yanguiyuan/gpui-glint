use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbKind {
    #[default]
    Sqlite,
    MySql,
    Postgres,
}

impl DbKind {
    pub const ALL: [DbKind; 3] = [DbKind::Sqlite, DbKind::MySql, DbKind::Postgres];

    pub fn label(self) -> &'static str {
        match self {
            DbKind::Sqlite => "SQLite",
            DbKind::MySql => "MySQL",
            DbKind::Postgres => "PostgreSQL",
        }
    }

    pub fn short_badge(self) -> &'static str {
        match self {
            DbKind::Sqlite => "SQLite",
            DbKind::MySql => "MySQL",
            DbKind::Postgres => "PG",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            DbKind::Sqlite => 0,
            DbKind::MySql => 3306,
            DbKind::Postgres => 5432,
        }
    }

    pub fn is_file_based(self) -> bool {
        matches!(self, DbKind::Sqlite)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: Uuid,
    pub name: String,
    pub kind: DbKind,
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

impl ConnectionConfig {
    pub fn sqlite(name: impl Into<String>, file_path: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind: DbKind::Sqlite,
            file_path: file_path.into(),
            host: String::new(),
            port: 0,
            database: String::new(),
            username: String::new(),
            password: String::new(),
        }
    }

    pub fn new_default(kind: DbKind, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            file_path: String::new(),
            host: if kind.is_file_based() {
                String::new()
            } else {
                "127.0.0.1".into()
            },
            port: kind.default_port(),
            database: match kind {
                DbKind::Sqlite => String::new(),
                DbKind::MySql => String::new(),
                DbKind::Postgres => "postgres".into(),
            },
            username: match kind {
                DbKind::Sqlite => String::new(),
                DbKind::MySql => "root".into(),
                DbKind::Postgres => "postgres".into(),
            },
            password: String::new(),
        }
    }

    pub fn summary(&self) -> String {
        match self.kind {
            DbKind::Sqlite => {
                if self.file_path.trim().is_empty() {
                    "未选择数据库文件".into()
                } else {
                    self.file_path.clone()
                }
            }
            DbKind::MySql | DbKind::Postgres => {
                let host = if self.host.trim().is_empty() {
                    "127.0.0.1"
                } else {
                    self.host.trim()
                };
                let port = if self.port == 0 {
                    self.kind.default_port()
                } else {
                    self.port
                };
                if self.database.trim().is_empty() {
                    format!("{host}:{port}")
                } else {
                    format!("{host}:{port}/{}", self.database.trim())
                }
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("连接名称不能为空".into());
        }
        match self.kind {
            DbKind::Sqlite => {
                if self.file_path.trim().is_empty() {
                    return Err("请选择或输入 SQLite 文件路径".into());
                }
            }
            DbKind::MySql | DbKind::Postgres => {
                if self.host.trim().is_empty() {
                    return Err("主机地址不能为空".into());
                }
                if self.port == 0 {
                    return Err("端口号必须大于 0".into());
                }
                if self.username.trim().is_empty() {
                    return Err("用户名不能为空".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableInfo {
    pub name: String,
    pub is_view: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub default_value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub total_rows: Option<u64>,
    pub affected_rows: Option<u64>,
    pub elapsed_ms: u64,
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceTab {
    #[default]
    Browse,
    Structure,
    Query,
}

impl WorkspaceTab {
    pub fn index(self) -> usize {
        match self {
            WorkspaceTab::Browse => 0,
            WorkspaceTab::Structure => 1,
            WorkspaceTab::Query => 2,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            1 => WorkspaceTab::Structure,
            2 => WorkspaceTab::Query,
            _ => WorkspaceTab::Browse,
        }
    }
}
