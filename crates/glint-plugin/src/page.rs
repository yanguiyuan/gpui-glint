use crate::{PluginError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
/// Engine-neutral page snapshot; the host renders its own native controls.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub title: String,
    pub body: Node,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Text {
        text: String,
    },
    Column {
        children: Vec<Node>,
    },
    Row {
        children: Vec<Node>,
    },
    Button {
        id: String,
        label: String,
        action: String,
        #[serde(default)]
        data: Value,
    },
    Table {
        id: String,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}
impl Page {
    pub fn validate(&self) -> Result<()> {
        if self.title.len() > 256 {
            return Err(PluginError("Plugin page title too large".into()));
        }
        let mut ids = BTreeSet::new();
        let mut count = 0;
        fn visit(
            node: &Node,
            depth: usize,
            count: &mut usize,
            ids: &mut BTreeSet<String>,
        ) -> Result<()> {
            *count += 1;
            if depth > 16 || *count > 256 {
                return Err(PluginError("Plugin page nesting limit exceeded".into()));
            }
            match node {
                Node::Text { text } if text.len() > 8192 => {
                    return Err(PluginError("Plugin text too large".into()));
                }
                Node::Column { children } | Node::Row { children } => {
                    for child in children {
                        visit(child, depth + 1, count, ids)?;
                    }
                }
                Node::Button {
                    id,
                    label,
                    action,
                    data,
                } => {
                    if label.len() > 256
                        || action.is_empty()
                        || action.len() > 128
                        || data.to_string().len() > 8192
                        || id.is_empty()
                        || id.len() > 128
                        || !ids.insert(id.clone())
                    {
                        return Err(PluginError("Invalid plugin action".into()));
                    }
                }
                Node::Table { id, columns, rows } => {
                    if id.is_empty()
                        || id.len() > 128
                        || !ids.insert(id.clone())
                        || columns.is_empty()
                        || columns.len() > 32
                        || rows.len() > 1000
                        || columns.iter().any(|c| c.len() > 256)
                        || rows
                            .iter()
                            .any(|r| r.len() != columns.len() || r.iter().any(|c| c.len() > 8192))
                    {
                        return Err(PluginError("Invalid plugin table".into()));
                    }
                }
                _ => {}
            }
            Ok(())
        }
        visit(&self.body, 0, &mut count, &mut ids)?;
        if serde_json::to_vec(self)
            .map_err(|_| PluginError("Invalid page".into()))?
            .len()
            > 4 * 1024 * 1024
        {
            return Err(PluginError("Plugin page too large".into()));
        }
        Ok(())
    }
}
