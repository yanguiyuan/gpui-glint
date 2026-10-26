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
        let tasks: Vec<Schedule> = match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|_| "日程文件损坏，请检查 schedules.json")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(_) => return Err("无法读取日程文件".into()),
        };
        let mut store = Self {
            tasks,
            path,
            readable: true,
        };
        // An interrupted app must not leave tasks permanently marked as running.
        for task in &mut store.tasks {
            if task.running {
                task.running = false;
                task.last_error = Some("上次执行被应用退出中断".into());
            }
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
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| "无法创建日程目录")?;
        }
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(&self.tasks).map_err(|_| "无法编码日程")?,
        )
        .map_err(|_| "无法写入日程")?;
        std::fs::rename(temp, &self.path).map_err(|_| "无法保存日程".into())
    }
    pub fn upsert(&mut self, task: Schedule) -> Result<(), String> {
        if task.title.trim().is_empty() || task.prompt.trim().is_empty() {
            return Err("任务标题和提示词不能为空".into());
        }
        let old = self.tasks.clone();
        if let Some(existing) = self.tasks.iter_mut().find(|item| item.id == task.id) {
            *existing = task;
        } else {
            self.tasks.push(task);
        }
        if let Err(error) = self.save() {
            self.tasks = old;
            return Err(error);
        }
        Ok(())
    }
    pub fn remove(&mut self, id: Uuid) -> Result<(), String> {
        if self.tasks.iter().any(|task| task.id == id && task.running) {
            return Err("请等待任务执行完成后再删除".into());
        }
        let old = self.tasks.clone();
        self.tasks.retain(|task| task.id != id);
        if self.tasks.len() == old.len() {
            return Err("日程不存在".into());
        }
        if let Err(error) = self.save() {
            self.tasks = old;
            return Err(error);
        }
        Ok(())
    }
    pub fn claim_due(&mut self, now: DateTime<Local>) -> Result<Option<Schedule>, String> {
        let Some(index) = self
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| task.enabled && !task.running && task.run_at <= now)
            .min_by_key(|(_, task)| task.run_at)
            .map(|(index, _)| index)
        else {
            return Ok(None);
        };
        let old = self.tasks[index].clone();
        let task = &mut self.tasks[index];
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
        if let Err(error) = self.save() {
            self.tasks[index] = old;
            return Err(error);
        }
        Ok(Some(self.tasks[index].clone()))
    }
    pub fn finish(&mut self, id: Uuid, error: Option<String>) -> Result<(), String> {
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            task.running = false;
            task.last_error = error;
        }
        self.save()
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
