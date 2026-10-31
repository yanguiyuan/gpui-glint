//! Versioned, self-contained companion migration files. Images travel inline.
use crate::{
    database,
    partners::{Partner, Repository},
    schedules::Schedule,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Write},
    path::Path,
};
use uuid::Uuid;

const FORMAT: &str = "glint-agent.partner";
const VERSION: u32 = 1;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PICTURE: usize = 4 * 1024 * 1024;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize, Deserialize)]
struct Picture {
    sha256: String,
    png_base64: String,
}
#[derive(Serialize, Deserialize)]
struct Event {
    id: Uuid,
    session_id: Uuid,
    ordinal: u64,
    project: Option<String>,
    payload: serde_json::Value,
    created_at: i64,
    status: String,
}
#[derive(Serialize, Deserialize)]
struct Memory {
    id: Uuid,
    event_id: Option<Uuid>,
    project: Option<String>,
    kind: String,
    title: String,
    summary: String,
    content: String,
    strength: f64,
    touched_at: i64,
    pinned: bool,
}
#[derive(Serialize, Deserialize)]
pub struct Archive {
    format: String,
    version: u32,
    exported_at: String,
    profile: Partner,
    picture: Option<Picture>,
    proactive: bool,
    events: Vec<Event>,
    memories: Vec<Memory>,
    traits: Vec<(String, Uuid)>,
    reinforcements: Vec<(Uuid, Uuid)>,
    schedules: Vec<Schedule>,
    wake_log: Vec<(Uuid, i64)>,
    goal_runs: Vec<(Uuid, Uuid, i64)>,
}
fn collect<T: DeserializeOwned>(db: &Connection, sql: &str, id: Uuid) -> Result<Vec<T>, String> {
    let mut stmt = db.prepare(sql).map_err(err)?;
    let rows = stmt
        .query_map([id.to_string()], |r| r.get::<_, String>(0))
        .map_err(err)?;
    rows.map(|row| serde_json::from_str(&row.map_err(err)?).map_err(err))
        .collect()
}
impl Archive {
    pub fn export(repo: &Repository, id: Uuid) -> Result<Self, String> {
        let mut db = database::open(&repo.anchor)?;
        let tx = db.transaction().map_err(err)?;
        let profile: Partner = tx
            .query_row(
                "SELECT id,name,avatar,description,created_at FROM partners WHERE id=?1",
                [id.to_string()],
                |r| {
                    Ok(Partner {
                        id,
                        name: r.get(1)?,
                        avatar: r.get(2)?,
                        description: r.get(3)?,
                        created_at: r.get(4)?,
                    })
                },
            )
            .map_err(|_| "伙伴不存在，无法导出")?;
        let picture = if crate::partners::is_avatar_reference(&profile.avatar) {
            let path = repo
                .avatar_path(&profile.avatar)
                .ok_or("伙伴头像文件缺失，请重新设置头像后导出")?;
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(err)?
                .take(MAX_PICTURE as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(err)?;
            if bytes.len() > MAX_PICTURE {
                return Err("伙伴头像文件过大".into());
            }
            Some(Picture {
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                png_base64: STANDARD.encode(bytes),
            })
        } else {
            None
        };
        let events = collect(
            &tx,
            "SELECT json_object('id',id,'session_id',session_id,'ordinal',ordinal,'project',project,'payload',json(payload),'created_at',created_at,'status',status) FROM partner_events WHERE partner_id=?1 ORDER BY created_at,id",
            id,
        )?;
        let memories = collect(
            &tx,
            "SELECT json_object('id',id,'event_id',event_id,'project',project,'kind',kind,'title',title,'summary',summary,'content',content,'strength',strength,'touched_at',touched_at,'pinned',json(CASE WHEN pinned THEN 'true' ELSE 'false' END)) FROM partner_memories WHERE partner_id=?1 ORDER BY id",
            id,
        )?;
        let mut archive = Self {
            format: FORMAT.into(),
            version: VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            profile,
            picture,
            proactive: tx
                .query_row(
                    "SELECT coalesce((SELECT proactive FROM partner_policy WHERE partner_id=?1),0)",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .map_err(err)?,
            events,
            memories,
            traits: collect(
                &tx,
                "SELECT json_array(trait,event_id) FROM partner_traits WHERE partner_id=?1 ORDER BY trait,event_id",
                id,
            )?,
            reinforcements: collect(
                &tx,
                "SELECT json_array(r.memory_id,r.event_id) FROM partner_reinforcements r JOIN partner_memories m ON m.id=r.memory_id WHERE m.partner_id=?1 ORDER BY r.memory_id,r.event_id",
                id,
            )?,
            schedules: collect(
                &tx,
                "SELECT payload FROM schedules WHERE json_extract(payload,'$.partner_id')=?1 ORDER BY id",
                id,
            )?,
            wake_log: collect(
                &tx,
                "SELECT json_array(schedule_id,woke_at) FROM partner_wake_log WHERE partner_id=?1 ORDER BY id",
                id,
            )?,
            goal_runs: collect(
                &tx,
                "SELECT json_array(goal_id,schedule_id,created_at) FROM partner_goal_runs WHERE partner_id=?1 ORDER BY goal_id",
                id,
            )?,
        };
        // Deleted goals leave disabled schedules in the source database. Preserve
        // those plans without carrying a dangling link into the destination.
        let memory_ids: HashSet<_> = archive.memories.iter().map(|memory| memory.id).collect();
        for task in &mut archive.schedules {
            if task
                .partner_goal
                .is_some_and(|goal| !memory_ids.contains(&goal))
            {
                task.partner_goal = None;
            }
        }
        archive.validate()?;
        Ok(archive)
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|e| format!("无法读取伙伴文件：{e}"))?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(err)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("伙伴迁移文件不能超过 64 MiB".into());
        }
        let header: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "伙伴迁移文件不是有效 JSON")?;
        if header["format"] != FORMAT || header["version"] != VERSION {
            return Err("不支持的伙伴迁移文件格式或版本，请使用兼容版本的 Glint Workbench".into());
        }
        let archive: Self =
            serde_json::from_value(header).map_err(|e| format!("伙伴迁移文件不完整：{e}"))?;
        archive.validate()?;
        Ok(archive)
    }
    pub fn write(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(err)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("伙伴导出内容超过 64 MiB".into());
        }
        let parent = path.parent().ok_or("导出路径无效")?;
        let temp = parent.join(format!(".glint-partner-export-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp).map_err(err)?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(err)?;
            drop(file);
            std::fs::rename(&temp, path).map_err(err)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
    fn picture_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        let Some(picture) = &self.picture else {
            return Ok(None);
        };
        if picture.png_base64.len() > MAX_PICTURE * 4 / 3 + 4 {
            return Err("迁移文件中的头像过大".into());
        }
        let bytes = STANDARD
            .decode(&picture.png_base64)
            .map_err(|_| "迁移文件中的头像编码损坏")?;
        if bytes.len() > MAX_PICTURE || format!("{:x}", Sha256::digest(&bytes)) != picture.sha256 {
            return Err("迁移文件中的头像校验失败".into());
        }
        let image =
            image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Png);
        let (width, height) = image
            .into_dimensions()
            .map_err(|_| "迁移文件中的头像不是有效 PNG")?;
        if width == 0 || height == 0 || width > 512 || height > 512 {
            return Err("迁移头像必须在 512×512 像素以内".into());
        }
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|_| "迁移头像无法解码")?;
        Ok(Some(bytes))
    }
    fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT || self.version != VERSION {
            return Err("不支持的伙伴迁移文件版本".into());
        }
        let p = &self.profile;
        if p.name.trim().is_empty()
            || p.name.chars().count() > 60
            || p.description.chars().count() > 2000
            || chrono::DateTime::from_timestamp(p.created_at, 0).is_none()
        {
            return Err("伙伴设定无效".into());
        }
        match &self.picture {
            Some(picture) if p.avatar == format!("avatar:{}:{}", p.id, picture.sha256) => {}
            None if !crate::partners::is_avatar_reference(&p.avatar)
                && p.avatar.chars().count() <= 8 => {}
            _ => return Err("伙伴头像与迁移文件不一致".into()),
        }
        self.picture_bytes()?;
        let events: HashSet<_> = self.events.iter().map(|e| e.id).collect();
        let memories: HashSet<_> = self.memories.iter().map(|m| m.id).collect();
        let schedules: HashSet<_> = self.schedules.iter().map(|s| s.id).collect();
        if events.len() != self.events.len()
            || memories.len() != self.memories.len()
            || schedules.len() != self.schedules.len()
        {
            return Err("迁移文件含重复的数据 ID".into());
        }
        let invalid = self.events.iter().any(|e| {
            e.ordinal > i64::MAX as u64
                || chrono::DateTime::from_timestamp(e.created_at, 0).is_none()
        }) || self.memories.iter().any(|m| {
            m.event_id.is_some_and(|id| !events.contains(&id))
                || !m.strength.is_finite()
                || m.strength < 0.
                || chrono::DateTime::from_timestamp(m.touched_at, 0).is_none()
        }) || self.traits.iter().any(|(_, e)| !events.contains(e))
            || self
                .reinforcements
                .iter()
                .any(|(m, e)| !memories.contains(m) || !events.contains(e))
            || self.schedules.iter().any(|s| {
                s.partner_id != Some(p.id)
                    || s.title.trim().is_empty()
                    || s.prompt.trim().is_empty()
                    || s.partner_goal.is_some_and(|g| !memories.contains(&g))
            })
            || self.goal_runs.iter().any(|(m, _, _)| !memories.contains(m));
        if invalid {
            return Err("伙伴迁移文件中的数据关系或时间无效".into());
        }
        Ok(())
    }
    /// Clone into new identities; never overwrite a local companion or inherit active jobs.
    pub fn import(&self, repo: &Repository) -> Result<Partner, String> {
        self.validate()?;
        let picture = self.picture_bytes()?;
        let id = Uuid::new_v4();
        let events: HashMap<_, _> = self.events.iter().map(|e| (e.id, Uuid::new_v4())).collect();
        let memories: HashMap<_, _> = self
            .memories
            .iter()
            .map(|m| (m.id, Uuid::new_v4()))
            .collect();
        let mut schedules: HashMap<_, _> = self
            .schedules
            .iter()
            .map(|s| (s.id, Uuid::new_v4()))
            .collect();
        for (schedule, _) in &self.wake_log {
            schedules.entry(*schedule).or_insert_with(Uuid::new_v4);
        }
        for (_, schedule, _) in &self.goal_runs {
            schedules.entry(*schedule).or_insert_with(Uuid::new_v4);
        }
        let avatar = match &self.picture {
            Some(p) => format!("avatar:{id}:{}", p.sha256),
            None => self.profile.avatar.clone(),
        };
        let partner = Partner {
            id,
            name: self.profile.name.clone(),
            avatar,
            description: self.profile.description.clone(),
            created_at: self.profile.created_at,
        };
        let directory = repo.avatar_directory().join(id.to_string());
        let mut staged = false;
        let result = (|| {
            let mut db = database::open(&repo.anchor)?;
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(err)?;
            tx.execute("INSERT INTO partners(id,name,avatar,description,created_at) VALUES(?1,?2,?3,?4,?5)",params![id.to_string(),partner.name,partner.avatar,partner.description,partner.created_at]).map_err(err)?;
            for e in &self.events {
                tx.execute("INSERT INTO partner_events(id,partner_id,session_id,ordinal,project,payload,created_at,status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![events[&e.id].to_string(),id.to_string(),e.session_id.to_string(),e.ordinal as i64,e.project,e.payload.to_string(),e.created_at,if e.status=="done"{"done"}else{"pending"}]).map_err(err)?;
            }
            for m in &self.memories {
                tx.execute("INSERT INTO partner_memories(id,partner_id,event_id,project,kind,title,summary,content,strength,touched_at,pinned) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![memories[&m.id].to_string(),id.to_string(),m.event_id.map(|e|events[&e].to_string()),m.project,if m.kind=="goal_running"{"goal_paused"}else{&m.kind},m.title,m.summary,m.content,m.strength,m.touched_at,m.pinned]).map_err(err)?;
            }
            for (name, event) in &self.traits {
                tx.execute(
                    "INSERT INTO partner_traits VALUES(?1,?2,?3)",
                    params![id.to_string(), name, events[event].to_string()],
                )
                .map_err(err)?;
            }
            for (memory, event) in &self.reinforcements {
                tx.execute(
                    "INSERT INTO partner_reinforcements VALUES(?1,?2)",
                    params![memories[memory].to_string(), events[event].to_string()],
                )
                .map_err(err)?;
            }
            for s in &self.schedules {
                let mut task = s.clone();
                task.id = schedules[&s.id];
                task.partner_id = Some(id);
                task.partner_goal = s.partner_goal.map(|g| memories[&g]);
                task.enabled = false;
                task.running = false;
                task.provider_id = None;
                task.model.clear();
                database::put_schedule(&tx, &task)?;
            }
            for (schedule, at) in &self.wake_log {
                tx.execute(
                    "INSERT INTO partner_wake_log(partner_id,schedule_id,woke_at) VALUES(?1,?2,?3)",
                    params![id.to_string(), schedules[schedule].to_string(), at],
                )
                .map_err(err)?;
            }
            for (goal, schedule, at) in &self.goal_runs {
                tx.execute(
                    "INSERT INTO partner_goal_runs VALUES(?1,?2,?3,?4)",
                    params![
                        memories[goal].to_string(),
                        id.to_string(),
                        schedules[schedule].to_string(),
                        at
                    ],
                )
                .map_err(err)?;
            }
            tx.execute("INSERT INTO partner_policy VALUES(?1,0)", [id.to_string()])
                .map_err(err)?;
            if let (Some(bytes), Some(asset)) = (&picture, &self.picture) {
                std::fs::create_dir_all(repo.avatar_directory()).map_err(err)?;
                std::fs::create_dir(&directory).map_err(err)?;
                staged = true;
                let path = directory.join(format!("{}.png", asset.sha256));
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(err)?;
                file.write_all(bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(err)?;
            }
            tx.commit().map_err(err)?;
            Ok(partner)
        })();
        if result.is_err() && staged {
            let _ = std::fs::remove_dir_all(directory);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;
    fn fixture() -> (tempfile::TempDir, Repository, Uuid, Uuid) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::new(dir.path().join("sessions.json"));
        let id = repo.create().unwrap();
        let other = repo.create().unwrap();
        repo.update(other, "unrelated-private-partner", "☁", "")
            .unwrap();
        let image = dir.path().join("avatar.png");
        image::RgbaImage::from_pixel(32, 32, image::Rgba([12, 90, 180, 255]))
            .save(&image)
            .unwrap();
        let avatar = repo.import_avatar(id, &image).unwrap();
        repo.update(id, "知微", &avatar, "在相处中形成自己的习惯")
            .unwrap();
        let db = database::open(&repo.anchor).unwrap();
        let event = Uuid::new_v4();
        let second = Uuid::new_v4();
        let memory = Uuid::new_v4();
        let goal = Uuid::new_v4();
        let now = chrono::Utc::now().timestamp();
        for eid in [event, second] {
            db.execute("INSERT INTO partner_events(id,partner_id,session_id,ordinal,project,payload,created_at,status) VALUES(?1,?2,?3,2,'/old/project','{}',?4,'done')",params![eid.to_string(),id.to_string(),Uuid::new_v4().to_string(),now]).unwrap();
        }
        db.execute("INSERT INTO partner_memories(id,partner_id,event_id,project,kind,title,summary,content,strength,touched_at,pinned) VALUES(?1,?2,?3,'/old/project','preference','开发习惯','先检查','先验证实际渲染路径',5.5,?4,1)",params![memory.to_string(),id.to_string(),event.to_string(),now-86400*50]).unwrap();
        db.execute("INSERT INTO partner_memories(id,partner_id,kind,title,summary,content,touched_at) VALUES(?1,?2,'goal_running','学习 Rust','目标','继续整理笔记',?3)",params![goal.to_string(),id.to_string(),now]).unwrap();
        db.execute(
            "INSERT INTO partner_reinforcements VALUES(?1,?2)",
            params![memory.to_string(), second.to_string()],
        )
        .unwrap();
        for eid in [event, second] {
            db.execute(
                "INSERT INTO partner_traits VALUES(?1,'做事谨慎',?2)",
                params![id.to_string(), eid.to_string()],
            )
            .unwrap();
        }
        let schedule = Schedule {
            id: Uuid::new_v4(),
            title: "回顾笔记".into(),
            prompt: "整理昨天的笔记".into(),
            run_at: chrono::Local::now(),
            repeat: crate::schedules::Repeat::Daily,
            enabled: true,
            running: true,
            project: Some("/old/project".into()),
            provider_id: Some(Uuid::new_v4()),
            model: "old-model".into(),
            partner_id: Some(id),
            partner_goal: Some(goal),
            self_wake: true,
            wake_mode: crate::schedules::WakeMode::Reflect,
            last_run: None,
            last_error: None,
        };
        database::put_schedule(&db, &schedule).unwrap();
        db.execute("INSERT INTO partner_policy VALUES(?1,1)", [id.to_string()])
            .unwrap();
        db.execute(
            "INSERT INTO partner_wake_log(partner_id,schedule_id,woke_at) VALUES(?1,?2,?3)",
            params![id.to_string(), schedule.id.to_string(), now],
        )
        .unwrap();
        db.execute(
            "INSERT INTO partner_goal_runs VALUES(?1,?2,?3,?4)",
            params![
                goal.to_string(),
                id.to_string(),
                schedule.id.to_string(),
                now
            ],
        )
        .unwrap();
        (dir, repo, id, goal)
    }
    #[test]
    fn file_round_trip_restores_the_brain_and_picture_with_fresh_ids_and_paused_wakes() {
        let (source, repo, id, old_goal) = fixture();
        let archive = Archive::export(&repo, id).unwrap();
        let package = source.path().join("知微.glint-partner.json");
        archive.write(&package).unwrap();
        let text = std::fs::read_to_string(&package).unwrap();
        assert!(!text.contains("unrelated-private-partner"));
        let archive = Archive::read(&package).unwrap();
        let expected_avatar = archive.picture_bytes().unwrap().unwrap();
        // The package must work with the original image directory entirely gone.
        std::fs::remove_dir_all(repo.avatar_directory()).unwrap();
        let target = tempfile::tempdir().unwrap();
        let repo = Repository::new(target.path().join("sessions.json"));
        let existing = repo.create().unwrap();
        repo.update(existing, "知微", "☁", "本机原有伙伴").unwrap();
        let first = archive.import(&repo).unwrap();
        let second = archive.import(&repo).unwrap();
        assert_ne!(first.id, id);
        assert_ne!(first.id, second.id);
        assert_eq!(repo.list().unwrap().len(), 3);
        assert_eq!(
            repo.list()
                .unwrap()
                .into_iter()
                .find(|p| p.id == existing)
                .unwrap()
                .description,
            "本机原有伙伴"
        );
        assert_eq!(first.name, "知微");
        assert_eq!(first.created_at, archive.profile.created_at);
        assert_eq!(
            std::fs::read(repo.avatar_path(&first.avatar).unwrap()).unwrap(),
            expected_avatar
        );
        assert_eq!(repo.traits(first.id).unwrap(), vec![("做事谨慎".into(), 2)]);
        assert!(!repo.proactive(first.id).unwrap());
        let entries = repo
            .entries(first.id, Some(Path::new("/old/project")))
            .unwrap();
        assert_eq!(entries.len(), 2);
        let memory = entries.iter().find(|e| e.title == "开发习惯").unwrap();
        assert!(memory.pinned);
        assert_eq!(memory.strength, 5.5);
        assert_eq!(memory.content, "先验证实际渲染路径");
        let goal = entries.iter().find(|e| e.title == "学习 Rust").unwrap();
        assert_ne!(goal.id, old_goal);
        assert_eq!(goal.kind, "goal_paused");
        let tasks = database::load_schedules(&repo.anchor).unwrap().unwrap();
        assert_eq!(tasks.len(), 2);
        let task = tasks
            .iter()
            .find(|t| t.partner_id == Some(first.id))
            .unwrap();
        assert!(!task.enabled && !task.running);
        assert_eq!(task.partner_goal, Some(goal.id));
        assert!(task.provider_id.is_none());
        assert!(task.model.is_empty());
        let db = database::open(&repo.anchor).unwrap();
        let schedule: String = db
            .query_row(
                "SELECT schedule_id FROM partner_goal_runs WHERE partner_id=?1",
                [first.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(schedule, task.id.to_string());
        let reinforcement_count:i64=db.query_row("SELECT count(*) FROM partner_reinforcements r JOIN partner_memories m ON r.memory_id=m.id WHERE m.partner_id=?1",[first.id.to_string()],|r|r.get(0)).unwrap();
        assert_eq!(reinforcement_count, 1);
        let wake: String = db
            .query_row(
                "SELECT schedule_id FROM partner_wake_log WHERE partner_id=?1",
                [first.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(wake, schedule);
        assert!(
            db.query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
                .optional()
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn bad_assets_relationships_and_future_versions_never_modify_destination() {
        let (source, repo, id, _) = fixture();
        let mut archive = Archive::export(&repo, id).unwrap();
        let target = tempfile::tempdir().unwrap();
        let dest = Repository::new(target.path().join("sessions.json"));
        let before = dest.create().unwrap();
        let encoded = archive.picture.as_ref().unwrap().png_base64.clone();
        archive.picture.as_mut().unwrap().png_base64 = "invalid".into();
        assert!(archive.import(&dest).is_err());
        archive.picture.as_mut().unwrap().png_base64 = encoded;
        let original = archive.memories[0].event_id;
        archive.memories[0].event_id = Some(Uuid::new_v4());
        assert!(archive.import(&dest).is_err());
        archive.memories[0].event_id = original;
        let package = source.path().join("invalid.json");
        let mut value = serde_json::to_value(&archive).unwrap();
        value["version"] = serde_json::json!(999);
        std::fs::write(&package, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Archive::read(&package).err().unwrap().contains("版本"));
        // Force a database constraint failure after identity/memory insertion.
        archive.traits.push(archive.traits[0].clone());
        assert!(archive.import(&dest).is_err());
        assert_eq!(
            dest.list()
                .unwrap()
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            vec![before]
        );
        assert!(!dest.avatar_directory().exists());
    }
    #[test]
    fn missing_source_picture_and_failed_destination_write_are_reported_atomically() {
        let (_source, repo, id, _) = fixture();
        let archive = Archive::export(&repo, id).unwrap();
        let target = tempfile::tempdir().unwrap();
        let dest = Repository::new(target.path().join("sessions.json"));
        let before = dest.create().unwrap();
        std::fs::write(dest.avatar_directory(), "not a directory").unwrap();
        assert!(archive.import(&dest).is_err());
        assert_eq!(dest.list().unwrap().len(), 1);
        assert_eq!(dest.list().unwrap()[0].id, before);
        std::fs::remove_dir_all(repo.avatar_directory()).unwrap();
        assert!(Archive::export(&repo, id).is_err());
    }
}
