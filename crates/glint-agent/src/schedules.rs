use chrono::{DateTime, Duration, Local};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Copy, Default, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    #[default]
    Once,
    Daily,
    Weekly,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: Uuid,
    pub title: String,
    pub prompt: String,
    pub run_at: DateTime<Local>,
    pub repeat: Repeat,
    pub enabled: bool,
    pub project: Option<PathBuf>,
    pub provider_id: Option<Uuid>,
    pub model: String,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub last_run: Option<DateTime<Local>>,
    #[serde(default)]
    pub last_error: Option<String>,
}

pub struct ScheduleStore {
    pub tasks: Vec<Schedule>,
    path: PathBuf,
    readable: bool,
}

impl ScheduleStore {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let stored = crate::database::load_schedules(&path)?;
        let importing = stored.is_none();
        let tasks: Vec<Schedule> = if let Some(tasks) = stored {
            tasks
        } else {
            match std::fs::read(&path) {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .map_err(|_| "日程文件损坏，请检查 schedules.json")?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![],
                Err(_) => return Err("无法读取日程文件".into()),
            }
        };
        let mut store = Self {
            tasks,
            path,
            readable: true,
        };
        // An interrupted app must not leave tasks permanently marked as running.
        let mut recovered = false;
        for task in &mut store.tasks {
            if task.running {
                task.running = false;
                recovered = true;
                task.last_error = Some("上次执行被应用退出中断".into());
            }
        }
        if importing || recovered {
            store.save()?;
        }
        Ok(store)
    }
    pub fn unavailable(path: PathBuf) -> Self {
        Self {
            tasks: vec![],
            path,
            readable: false,
        }
    }
    pub fn save(&self) -> Result<(), String> {
        if !self.readable {
            return Err("日程存储不可用，原始文件已保留".into());
        }
        crate::database::save_schedules(&self.path, &self.tasks)
    }
    pub fn upsert(&mut self, task: Schedule) -> Result<(), String> {
        if task.title.trim().is_empty() || task.prompt.trim().is_empty() {
            return Err("任务标题和提示词不能为空".into());
        }
        if !self.readable {
            return Err("日程存储不可用，原始数据已保留".into());
        }
        let connection = crate::database::open(&self.path)?;
        crate::database::put_schedule(&connection, &task)?;
        self.tasks = crate::database::load_schedules(&self.path)?.unwrap_or_default();
        Ok(())
    }
    pub fn remove(&mut self, id: Uuid) -> Result<(), String> {
        if !self.readable {
            return Err("日程存储不可用".into());
        }
        let connection = crate::database::open(&self.path)?;
        let removed = connection
            .execute(
                "DELETE FROM schedules WHERE id=?1 AND running=0",
                [id.to_string()],
            )
            .map_err(|error| error.to_string())?;
        if removed == 0 {
            return Err("日程不存在或仍在执行中".into());
        }
        self.tasks = crate::database::load_schedules(&self.path)?.unwrap_or_default();
        Ok(())
    }
    pub fn claim_due(&mut self, now: DateTime<Local>) -> Result<Option<Schedule>, String> {
        use rusqlite::OptionalExtension;
        if !self.readable {
            return Err("日程存储不可用".into());
        }
        let mut connection = crate::database::open(&self.path)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let value: Option<String> = transaction.query_row("SELECT payload FROM schedules WHERE enabled=1 AND running=0 AND run_at<=?1 ORDER BY run_at,id LIMIT 1", [now.timestamp()], |row|row.get(0)).optional().map_err(|error|error.to_string())?;
        let Some(value) = value else {
            return Ok(None);
        };
        let mut task: Schedule = serde_json::from_str(&value).map_err(|error| error.to_string())?;
        task.running = true;
        task.last_run = Some(now);
        task.last_error = None;
        match task.repeat {
            Repeat::Once => task.enabled = false,
            repeat => {
                let days = if repeat == Repeat::Daily { 1 } else { 7 };
                while task.run_at <= now {
                    task.run_at += Duration::days(days);
                }
            }
        }
        crate::database::put_schedule(&transaction, &task)?;
        transaction.commit().map_err(|error| error.to_string())?;
        self.tasks = crate::database::load_schedules(&self.path)?.unwrap_or_default();
        Ok(Some(task))
    }
    pub fn finish(&mut self, id: Uuid, error: Option<String>) -> Result<(), String> {
        use rusqlite::OptionalExtension;
        if !self.readable {
            return Err("日程存储不可用".into());
        }
        let mut connection = crate::database::open(&self.path)?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let value: Option<String> = transaction
            .query_row(
                "SELECT payload FROM schedules WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(value) = value {
            let mut task: Schedule =
                serde_json::from_str(&value).map_err(|error| error.to_string())?;
            task.running = false;
            task.last_error = error;
            crate::database::put_schedule(&transaction, &task)?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        self.tasks = crate::database::load_schedules(&self.path)?.unwrap_or_default();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crud_persists_and_due_tasks_are_claimed_once_and_rescheduled() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("schedules.json");
        let mut store = ScheduleStore::open(path.clone()).unwrap();
        let now = Local::now();
        let mut task = Schedule {
            id: Uuid::new_v4(),
            title: "Check".into(),
            prompt: "Check project".into(),
            run_at: now - Duration::days(3),
            repeat: Repeat::Daily,
            enabled: true,
            project: None,
            provider_id: None,
            model: String::new(),
            running: false,
            last_run: None,
            last_error: None,
        };
        store.upsert(task.clone()).unwrap();
        let claimed = store.claim_due(now).unwrap().unwrap();
        assert!(claimed.run_at > now);
        assert!(claimed.running);
        assert!(store.claim_due(now).unwrap().is_none());
        store.finish(task.id, None).unwrap();
        task.title = "Updated".into();
        store.upsert(task.clone()).unwrap();
        assert_eq!(ScheduleStore::open(path).unwrap().tasks[0].title, "Updated");
        store.remove(task.id).unwrap();
        assert!(store.tasks.is_empty());
    }
}
