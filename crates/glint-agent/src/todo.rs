use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub type Store = Arc<Mutex<Vec<Item>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub content: String,
    pub status: Status,
}

pub fn write(arguments: &serde_json::Value, store: &Store) -> Result<Vec<Item>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        todos: Vec<Item>,
    }
    let args: Args = serde_json::from_value(arguments.clone())
        .map_err(|_| "任务清单参数无效，需要 todos 与有效的 content/status")?;
    if args.todos.len() > 30 {
        return Err("任务清单最多 30 项".into());
    }
    let mut todos = args.todos;
    for item in &mut todos {
        item.content = item.content.trim().to_owned();
        if item.content.is_empty() || item.content.chars().count() > 500 {
            return Err("任务内容须为 1–500 个字符".into());
        }
    }
    if todos
        .iter()
        .filter(|item| item.status == Status::InProgress)
        .count()
        > 1
    {
        return Err("同一清单最多只能有一项进行中任务".into());
    }
    *store.lock().map_err(|_| "无法更新任务清单")? = todos.clone();
    Ok(todos)
}

pub fn items(todos: &[Item]) -> Vec<gpui_glint::components::TodoListItem> {
    use gpui_glint::components::{TodoListItem, TodoListStatus};
    todos
        .iter()
        .map(|item| TodoListItem {
            content: item.content.clone().into(),
            status: match item.status {
                Status::Pending => TodoListStatus::Pending,
                Status::InProgress => TodoListStatus::InProgress,
                Status::Completed => TodoListStatus::Completed,
            },
        })
        .collect()
}

impl crate::AgentWorkspace {
    pub fn sync_todos(&self, cx: &mut gpui::Context<Self>) {
        let items = items(&self.history.active().todos);
        self.composer
            .update(cx, |composer, cx| composer.set_todos(items, cx));
    }

    pub fn finish_todos(&mut self, session_id: uuid::Uuid, cx: &mut gpui::Context<Self>) {
        if let Ok(todos) = self.todo_live.lock() {
            if let Some(session) = self
                .history
                .sessions
                .iter_mut()
                .find(|session| session.id == session_id)
            {
                session.todos = todos.clone();
            }
        }
        self.sync_todos(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn updates_validate_atomically_and_clear_explicitly() {
        let store = Store::default();
        let initial = write(&json!({"todos":[{"content":" 检查实现 ","status":"in_progress"},{"content":"验证","status":"pending"}]}), &store).unwrap();
        assert_eq!(initial[0].content, "检查实现");
        for invalid in [
            json!({"todos":[{"content":" ","status":"pending"}]}),
            json!({"todos":[{"content":"a","status":"in_progress"},{"content":"b","status":"in_progress"}]}),
            json!({"todos":[{"content":"a","status":"unknown"}]}),
            json!({"todos":[],"other":true}),
        ] {
            assert!(write(&invalid, &store).is_err());
            assert_eq!(*store.lock().unwrap(), initial);
        }
        assert!(write(&json!({"todos":[]}), &store).unwrap().is_empty());
    }
    #[test]
    fn session_lists_survive_storage_and_old_sessions_default_to_empty() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        let mut history = crate::model::History::default();
        history.active_mut().todos = vec![Item {
            content: "验证存储".into(),
            status: Status::Completed,
        }];
        history.save(&path).unwrap();
        let loaded = crate::model::History::load(&path).unwrap();
        assert_eq!(loaded.active().todos, history.active().todos);
        let mut legacy = serde_json::to_value(loaded.active()).unwrap();
        legacy.as_object_mut().unwrap().remove("todos");
        assert!(
            serde_json::from_value::<crate::model::Session>(legacy)
                .unwrap()
                .todos
                .is_empty()
        );
    }
}
