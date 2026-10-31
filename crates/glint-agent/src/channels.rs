//! Platform-independent bindings, credentials and durable delivery queue in SQLite.
mod auth;
pub(crate) mod tags;
pub(crate) mod tools;
pub(crate) mod ui;
mod wps;
use futures_util::FutureExt;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Platform {
    Wps,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Application {
    pub id: Uuid,
    pub platform: Platform,
    pub name: String,
    pub app_id: String,
    pub partner: Uuid,
    pub project: PathBuf,
    pub enabled: bool,
}
#[derive(Clone)]
pub struct Repository(pub PathBuf);
#[derive(Clone, Debug)]
pub struct Incoming {
    pub app: Uuid,
    pub company: String,
    pub chat: String,
    pub message: String,
    pub sender: String,
    pub text: String,
}
pub struct Job {
    pub row: i64,
    pub app: Application,
    pub session: Uuid,
    pub text: String,
}
impl Repository {
    pub fn new(path: &Path) -> Self {
        Self(path.with_file_name("channels.sqlite3"))
    }
    fn open(&self) -> Result<Connection, String> {
        if let Some(parent) = self.0.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let db = Connection::open(&self.0).map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS applications(id TEXT PRIMARY KEY,payload TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS application_secrets(app TEXT PRIMARY KEY REFERENCES applications(id) ON DELETE CASCADE,secret TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS user_grants(app TEXT PRIMARY KEY,company TEXT NOT NULL,sender TEXT NOT NULL,payload TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS user_authorizations(state TEXT PRIMARY KEY,app TEXT NOT NULL,company TEXT NOT NULL,sender TEXT NOT NULL,expires INTEGER NOT NULL,status TEXT NOT NULL,error TEXT);
            CREATE TABLE IF NOT EXISTS owners(app TEXT PRIMARY KEY,company TEXT NOT NULL,sender TEXT NOT NULL,chat TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS conversations(app TEXT,partner TEXT,project TEXT,company TEXT,chat TEXT,sender TEXT NOT NULL,session TEXT NOT NULL,PRIMARY KEY(app,partner,project,company,chat,sender));
            CREATE TABLE IF NOT EXISTS inbox(id INTEGER PRIMARY KEY,app TEXT NOT NULL,company TEXT NOT NULL,chat TEXT NOT NULL,message TEXT NOT NULL,sender TEXT NOT NULL,text TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'queued',session TEXT,error TEXT,UNIQUE(app,company,chat,message));
            CREATE TABLE IF NOT EXISTS outbox(id INTEGER PRIMARY KEY,inbox INTEGER NOT NULL,app TEXT NOT NULL,chat TEXT NOT NULL,text TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'queued',error TEXT);
            CREATE TABLE IF NOT EXISTS reactions(inbox INTEGER NOT NULL,stage TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'queued',error TEXT,PRIMARY KEY(inbox,stage));
            CREATE INDEX IF NOT EXISTS inbox_pending ON inbox(state,id);
            CREATE INDEX IF NOT EXISTS outbox_pending ON outbox(app,state,id);").map_err(|e|e.to_string())?;
        Ok(db)
    }
    pub fn applications(&self) -> Result<Vec<Application>, String> {
        let db = self.open()?;
        let mut q = db
            .prepare("SELECT payload FROM applications ORDER BY rowid")
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        })
        .collect()
    }
    pub fn secret(&self, app: Uuid) -> Result<String, String> {
        self.open()?
            .query_row(
                "SELECT secret FROM application_secrets WHERE app=?1",
                [app.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .filter(|secret| !secret.trim().is_empty())
            .ok_or_else(|| "请在频道配置中重新填写或从 API 导入应用密钥并保存".into())
    }
    pub fn save(&self, app: &Application) -> Result<(), String> {
        self.save_with_secret(app, None)
    }
    /// Keep credentials outside serialized application metadata and tool output.
    /// Saving an edited application without a secret preserves its current value.
    pub fn save_with_secret(&self, app: &Application, secret: Option<&str>) -> Result<(), String> {
        if secret.is_some_and(|value| value.trim().is_empty()) {
            return Err("应用密钥不能为空".into());
        }

        if app.app_id.trim().is_empty() || app.name.trim().is_empty() {
            return Err("请填写应用名称和 App ID".into());
        }
        if !app.project.is_dir() {
            return Err("请选择存在的项目目录".into());
        }
        let mut db = self.open()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let apps = {
            let mut q = tx
                .prepare("SELECT payload FROM applications")
                .map_err(|e| e.to_string())?;
            let rows = q
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.map(|r| {
                serde_json::from_str::<Application>(&r.map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?
        };
        if apps
            .iter()
            .any(|a| a.app_id == app.app_id && a.id != app.id)
        {
            return Err("这个 WPS 应用已添加，请编辑已有应用".into());
        }
        if let Some(old) = apps.iter().find(|a| a.id == app.id) {
            if old.app_id != app.app_id {
                return Err("已有应用的 App ID 不可修改".into());
            }
            if old.partner != app.partner || old.project != app.project {
                let running: i64 = tx
                    .query_row(
                        "SELECT count(*) FROM inbox WHERE app=?1 AND state='running'",
                        [app.id.to_string()],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if running > 0 {
                    return Err("伙伴正在回复，请完成后修改绑定".into());
                }
                tx.execute("UPDATE inbox SET state='failed',error='应用重新绑定，旧绑定的待处理消息已停止' WHERE app=?1 AND state='queued'",[app.id.to_string()]).map_err(|e|e.to_string())?;
            }
        }
        tx.execute("INSERT INTO applications VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![app.id.to_string(),serde_json::to_string(app).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
        if let Some(secret) = secret {
            tx.execute("INSERT INTO application_secrets(app,secret) VALUES(?1,?2) ON CONFLICT(app) DO UPDATE SET secret=excluded.secret",params![app.id.to_string(),secret]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    /// Commit before ACK; retries never enqueue a second generation.
    pub fn receive(&self, app: &Application, m: &Incoming) -> Result<(), String> {
        let mut db = self.open()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let payload: String = tx
            .query_row(
                "SELECT payload FROM applications WHERE id=?1",
                [app.id.to_string()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let current: Application = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        if &current != app || !current.enabled || m.app != app.id {
            return Err("频道配置已变更，请重新连接".into());
        }
        let owner: Option<(String, String)> = tx
            .query_row(
                "SELECT company,sender FROM owners WHERE app=?1",
                [app.id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match owner {
            None if m.text.trim().trim_end_matches(['。', '!', '！', '.']) == "我是你的主人" =>
            {
                tx.execute(
                    "INSERT INTO owners VALUES(?1,?2,?3,?4)",
                    params![app.id.to_string(), m.company, m.sender, m.chat],
                )
                .map_err(|e| e.to_string())?;
            }
            Some((company, sender)) if company == m.company && sender == m.sender => {}
            _ => return Ok(()), // ACK ignored messages; no model or response to other senders.
        }
        let inserted=tx.execute("INSERT OR IGNORE INTO inbox(app,company,chat,message,sender,text) VALUES(?1,?2,?3,?4,?5,?6)",params![m.app.to_string(),m.company,m.chat,m.message,m.sender,m.text]).map_err(|e|e.to_string())?;
        if inserted > 0 {
            tx.execute(
                "INSERT INTO reactions(inbox,stage) VALUES(last_insert_rowid(),'received')",
                [],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn owner(&self, app: Uuid) -> Result<Option<(String, String, String)>, String> {
        self.open()?
            .query_row(
                "SELECT company,sender,chat FROM owners WHERE app=?1",
                [app.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn clear_owner(&self, app: Uuid) -> Result<(), String> {
        let mut db = self.open()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let payload: String = tx
            .query_row(
                "SELECT payload FROM applications WHERE id=?1",
                [app.to_string()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let config: Application = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        let running: i64 = tx
            .query_row(
                "SELECT count(*) FROM inbox WHERE app=?1 AND state='running'",
                [app.to_string()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if config.enabled || running > 0 {
            return Err("请先停用应用，并等待伙伴回复结束，再解除主人绑定".into());
        }
        tx.execute("DELETE FROM user_grants WHERE app=?1", [app.to_string()])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM user_authorizations WHERE app=?1",
            [app.to_string()],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM owners WHERE app=?1", [app.to_string()])
            .map_err(|e| e.to_string())?;
        tx.execute("UPDATE inbox SET state='failed',error='主人绑定已解除' WHERE app=?1 AND state='queued'",[app.to_string()]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE outbox SET state='failed',error='主人绑定已解除' WHERE app=?1 AND state='queued'",[app.to_string()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        auth::cancel(app)
    }
    pub fn recover(&self) -> Result<(), String> {
        self.open()?.execute_batch("INSERT OR IGNORE INTO reactions(inbox,stage) SELECT id,'failed' FROM inbox WHERE state='running' AND message NOT LIKE 'notification:%'; UPDATE inbox SET state='failed',error='上次运行中断；请在会话记录中检查' WHERE state='running'; UPDATE outbox SET state='failed',error='发送时中断，送达状态未知，请核对 WPS 会话' WHERE state='sending';").map_err(|e|e.to_string())
    }
    pub fn next(&self) -> Result<Option<Job>, String> {
        let mut db = self.open()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let apps = self.applications()?;
        for app in apps.into_iter().filter(|a| a.enabled) {
            let row:Option<(i64,String,String,String,String)>=tx.query_row("SELECT id,company,chat,text,sender FROM inbox WHERE app=?1 AND state='queued' AND EXISTS(SELECT 1 FROM owners WHERE owners.app=inbox.app AND owners.company=inbox.company AND owners.sender=inbox.sender) ORDER BY id LIMIT 1",[app.id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|e|e.to_string())?;
            if let Some((row, company, chat, text, sender)) = row {
                let owner = app.partner.to_string();
                let project = app.project.to_string_lossy();
                let app_id = app.id.to_string();
                let session:Option<String>=tx.query_row("SELECT session FROM conversations WHERE app=?1 AND partner=?2 AND project=?3 AND company=?4 AND chat=?5 AND sender=?6",params![app_id,owner,project,company,chat,sender],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
                let session = session
                    .map(|s| Uuid::parse_str(&s).map_err(|e| e.to_string()))
                    .transpose()?
                    .unwrap_or_else(Uuid::new_v4);
                tx.execute(
                    "INSERT OR IGNORE INTO conversations VALUES(?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        app_id,
                        owner,
                        project,
                        company,
                        chat,
                        sender,
                        session.to_string()
                    ],
                )
                .map_err(|e| e.to_string())?;
                tx.execute(
                    "UPDATE inbox SET state='running',session=?2 WHERE id=?1",
                    params![row, session.to_string()],
                )
                .map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| e.to_string())?;
                return Ok(Some(Job {
                    row,
                    app,
                    session,
                    text,
                }));
            }
        }
        Ok(None)
    }
    pub fn finish(&self, row: i64, result: Result<&str, &str>) -> Result<(), String> {
        let mut db = self.open()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        match result {
            Ok(reply) => {
                let (app, chat): (String, String) = tx
                    .query_row(
                        "SELECT app,chat FROM inbox WHERE id=?1 AND state='running'",
                        [row],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(|e| e.to_string())?;
                for text in split_reply(reply) {
                    tx.execute(
                        "INSERT INTO outbox(inbox,app,chat,text) VALUES(?1,?2,?3,?4)",
                        params![row, app, chat, text],
                    )
                    .map_err(|e| e.to_string())?;
                }
                tx.execute("UPDATE inbox SET state='replied' WHERE id=?1", [row])
                    .map_err(|e| e.to_string())?;
            }
            Err(error) => {
                tx.execute(
                    "UPDATE inbox SET state='failed',error=?2 WHERE id=?1",
                    params![row, error],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        tx.execute("INSERT OR IGNORE INTO reactions(inbox,stage) SELECT id,'failed' FROM inbox WHERE id=?1 AND state='failed' AND message NOT LIKE 'notification:%'",[row]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    fn outgoing(&self, app: Uuid) -> Result<Option<(i64, String, String)>, String> {
        let mut db = self.open()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let row=tx.query_row("SELECT o.id,o.chat,o.text FROM outbox o WHERE o.app=?1 AND o.state='queued' AND EXISTS(SELECT 1 FROM owners JOIN inbox ON inbox.id=o.inbox WHERE owners.app=o.app AND owners.company=inbox.company AND owners.sender=inbox.sender) AND NOT EXISTS(SELECT 1 FROM outbox earlier WHERE earlier.inbox=o.inbox AND earlier.id<o.id AND earlier.state!='sent') ORDER BY o.id LIMIT 1",[app.to_string()],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional().map_err(|e|e.to_string())?;
        if let Some((id, _, _)) = &row {
            tx.execute("UPDATE outbox SET state='sending' WHERE id=?1", [id])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(row)
    }
    fn sent(&self, id: i64, result: Result<(), String>) -> Result<(), String> {
        let mut db = self.open()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let success = result.is_ok();
        tx.execute(
            "UPDATE outbox SET state=?2,error=?3 WHERE id=?1",
            params![id, if success { "sent" } else { "failed" }, result.err()],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("INSERT OR IGNORE INTO reactions(inbox,stage) SELECT i.id,?2 FROM inbox i JOIN outbox o ON o.inbox=i.id WHERE o.id=?1 AND i.message NOT LIKE 'notification:%' AND (?2='failed' OR NOT EXISTS(SELECT 1 FROM outbox pending WHERE pending.inbox=i.id AND pending.state!='sent'))",params![id,if success{"done"}else{"failed"}]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    fn reaction(&self, app: Uuid) -> Result<Option<(i64, String, String, String)>, String> {
        self.open()?.query_row("SELECT i.id,i.chat,i.message,r.stage FROM reactions r JOIN inbox i ON i.id=r.inbox JOIN owners own ON own.app=i.app AND own.company=i.company AND own.sender=i.sender WHERE i.app=?1 AND r.state='queued' ORDER BY r.rowid LIMIT 1",[app.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())
    }
    fn reacted(&self, row: i64, stage: &str, result: Result<(), String>) -> Result<(), String> {
        self.open()?
            .execute(
                "UPDATE reactions SET state=?3,error=?4 WHERE inbox=?1 AND stage=?2",
                params![
                    row,
                    stage,
                    if result.is_ok() { "sent" } else { "failed" },
                    result.err()
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    fn removed_reaction(&self, row: i64, stage: &str) -> Result<(), String> {
        self.open()?
            .execute(
                "UPDATE reactions SET state='removed' WHERE inbox=?1 AND stage=?2",
                params![row, stage],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    fn applied_reactions(&self, row: i64) -> Result<Vec<String>, String> {
        let db = self.open()?;
        let mut q = db
            .prepare("SELECT stage FROM reactions WHERE inbox=?1 AND state='sent'")
            .map_err(|e| e.to_string())?;
        q.query_map([row], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn interrupt_sends(&self, app: Uuid) -> Result<(), String> {
        self.open()?.execute("UPDATE outbox SET state='failed',error='监听已停止，送达状态未知，请核对会话' WHERE app=?1 AND state='sending'",[app.to_string()]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn retry_sends(&self, app: Uuid) -> Result<(), String> {
        if !self
            .applications()?
            .iter()
            .any(|a| a.id == app && a.enabled)
        {
            return Err("请先启用应用再重试发送".into());
        }
        if self.owner(app)?.is_none() {
            return Err("请先绑定主人，再重试发送".into());
        }
        self.open()?.execute("UPDATE outbox SET state='queued',error=NULL WHERE app=?1 AND state='failed' AND EXISTS(SELECT 1 FROM owners JOIN inbox ON inbox.id=outbox.inbox WHERE owners.app=outbox.app AND owners.company=inbox.company AND owners.sender=inbox.sender)",[app.to_string()]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn summary(&self, app: Uuid) -> Result<String, String> {
        let db = self.open()?;
        let incoming: i64 = db
            .query_row(
                "SELECT count(*) FROM inbox WHERE app=?1 AND message NOT LIKE 'notification:%'",
                [app.to_string()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let sent: i64 = db
            .query_row(
                "SELECT count(*) FROM outbox WHERE app=?1 AND state='sent'",
                [app.to_string()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let error:Option<String>=db.query_row("SELECT error FROM (SELECT error,id FROM inbox WHERE app=?1 AND state='failed' UNION ALL SELECT error,id FROM outbox WHERE app=?1 AND state='failed' UNION ALL SELECT r.error,i.id FROM reactions r JOIN inbox i ON i.id=r.inbox WHERE i.app=?1 AND r.state='failed') ORDER BY id DESC LIMIT 1",[app.to_string()],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        Ok(format!(
            "已收 {incoming} · 已发 {sent}{}",
            error.map(|e| format!(" · {e}")).unwrap_or_default()
        ))
    }
}
fn split_reply(text: &str) -> Vec<String> {
    // Leave space below WPS's 5000-character limit, including UTF-16 clients.
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    let mut units = 0;
    for ch in text.chars() {
        if units + ch.len_utf16() > 4000 {
            chunks.push(std::mem::take(&mut chunk));
            units = 0;
        }
        chunk.push(ch);
        units += ch.len_utf16();
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
}

pub struct Worker(pub tokio::task::JoinHandle<()>);
impl Drop for Worker {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub fn start(
    app: Application,
    repo: Repository,
    status: async_channel::Sender<(Uuid, String)>,
) -> Result<Worker, String> {
    let runtime = crate::provider::runtime()?;
    Ok(Worker(runtime.spawn(async move {
        let id = app.id;
        let secret = match repo.secret(app.id) {
            Ok(s) => s,
            Err(e) => {
                let _ = status.send((id, e)).await;
                return;
            }
        };
        let mut delay = 1u64;
        loop {
            let _ = status.send((id, "正在连接…".into())).await;
            let connected = std::time::Instant::now();
            let result = match app.platform {
                Platform::Wps => match std::panic::AssertUnwindSafe(wps::listen(&app, &secret, &repo, &status)).catch_unwind().await {
                    Ok(result)=>result,
                    Err(_)=>{let _=status.send((id,"频道连接任务异常退出，请停用后重新启用；若仍失败请检查运行日志".into())).await;return;}
                },
            };
            // Another active client must not be fought with an endless reconnect loop.
            if matches!(&result,Err(e) if e=="连接被另一客户端替换，请断开其他监听器后重新启用")
            {
                let _ = status.send((id, result.unwrap_err())).await;
                return;
            }
            if connected.elapsed().as_secs() > 90 {
                delay = 1;
            }
            let _ = status
                .send((
                    id,
                    format!(
                        "{} · {delay} 秒后重连",
                        result.err().unwrap_or_else(|| "连接已关闭".into())
                    ),
                ))
                .await;
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            delay = (delay * 2).min(60);
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture() -> (tempfile::TempDir, Repository, Application) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::new(&dir.path().join("sessions.json"));
        let app = Application {
            id: Uuid::new_v4(),
            platform: Platform::Wps,
            name: "测试机器人".into(),
            app_id: "AK-test".into(),
            partner: Uuid::new_v4(),
            project: dir.path().canonicalize().unwrap(),
            enabled: true,
        };
        repo.save(&app).unwrap();
        repo.open()
            .unwrap()
            .execute(
                "INSERT INTO owners VALUES(?1,'tenant-a','user-a','chat-a')",
                [app.id.to_string()],
            )
            .unwrap();
        (dir, repo, app)
    }
    fn message(app: Uuid, id: &str) -> Incoming {
        Incoming {
            app,
            company: "tenant-a".into(),
            chat: "chat-a".into(),
            message: id.into(),
            sender: "user-a".into(),
            text: "你好".into(),
        }
    }
    #[test]
    fn durable_dedup_and_original_conversation_delivery() {
        let (_dir, repo, app) = fixture();
        let m = message(app.id, "message-1");
        repo.receive(&app, &m).unwrap();
        repo.receive(&app, &m).unwrap();
        let repo = Repository(repo.0.clone());
        let first = repo.next().unwrap().unwrap();
        assert!(repo.next().unwrap().is_none());
        repo.finish(first.row, Ok("伙伴回复")).unwrap();
        assert!(repo.finish(first.row, Ok("重复完成")).is_err());
        let (row, chat, text) = repo.outgoing(app.id).unwrap().unwrap();
        assert_eq!(chat, "chat-a");
        assert_eq!(text, "伙伴回复");
        repo.sent(row, Ok(())).unwrap();
        assert!(repo.outgoing(app.id).unwrap().is_none());
        repo.receive(&app, &message(app.id, "message-2")).unwrap();
        let second = repo.next().unwrap().unwrap();
        assert_eq!(first.session, second.session);
        repo.finish(second.row, Err("已停止")).unwrap();
        let mut other = message(app.id, "message-3");
        other.company = "tenant-b".into();
        repo.receive(&app, &other).unwrap();
        assert!(repo.next().unwrap().is_none());
    }
    #[test]
    fn rebind_isolated_and_disabled_apps_pause_work() {
        let (_dir, repo, mut app) = fixture();
        repo.receive(&app, &message(app.id, "before")).unwrap();
        let old = repo.next().unwrap().unwrap();
        let original_partner = app.partner;
        app.partner = Uuid::new_v4();
        assert!(repo.save(&app).is_err());
        app.partner = original_partner;
        repo.finish(old.row, Err("取消")).unwrap();
        repo.receive(&app, &message(app.id, "queued-before-rebind"))
            .unwrap();
        app.partner = Uuid::new_v4();
        repo.save(&app).unwrap();
        assert!(repo.next().unwrap().is_none());
        repo.receive(&app, &message(app.id, "after")).unwrap();
        app.enabled = false;
        repo.save(&app).unwrap();
        assert!(repo.next().unwrap().is_none());
        app.enabled = true;
        repo.save(&app).unwrap();
        let new = repo.next().unwrap().unwrap();
        assert_ne!(old.session, new.session);
        assert_eq!(new.app.partner, app.partner);
    }
    #[test]
    fn recovery_and_partial_send_failures_do_not_silently_replay() {
        let (_dir, repo, app) = fixture();
        repo.receive(&app, &message(app.id, "m1")).unwrap();
        let job = repo.next().unwrap().unwrap();
        let text = "😀中文".repeat(3000);
        repo.finish(job.row, Ok(&text)).unwrap();
        let first = repo.outgoing(app.id).unwrap().unwrap();
        assert!(first.2.encode_utf16().count() <= 4000);
        // A restart during HTTP delivery is ambiguous; require a visible manual retry.
        repo.recover().unwrap();
        assert!(repo.outgoing(app.id).unwrap().is_none());
        assert!(repo.summary(app.id).unwrap().contains("送达状态未知"));
        repo.retry_sends(app.id).unwrap();
        let first_again = repo.outgoing(app.id).unwrap().unwrap();
        assert_eq!(first.0, first_again.0);
        repo.sent(first_again.0, Ok(())).unwrap();
        let mut pieces = vec![first_again.2];
        while let Some((id, _, text)) = repo.outgoing(app.id).unwrap() {
            assert!(text.encode_utf16().count() <= 4000);
            pieces.push(text);
            repo.sent(id, Ok(())).unwrap();
        }
        assert_eq!(pieces.concat(), text);
        repo.receive(&app, &message(app.id, "m2")).unwrap();
        repo.next().unwrap().unwrap();
        repo.recover().unwrap();
        assert!(repo.next().unwrap().is_none());
    }
}

#[cfg(test)]
mod owner_tests {
    use super::*;
    #[test]
    fn first_explicit_claim_persists_and_other_senders_cannot_take_over() {
        let (_dir, repo, mut app) = super::tests::fixture();
        app.enabled = false;
        repo.save(&app).unwrap();
        repo.clear_owner(app.id).unwrap();
        app.enabled = true;
        repo.save(&app).unwrap();
        let mut m = Incoming {
            app: app.id,
            company: "company".into(),
            chat: "owner-direct-chat".into(),
            sender: "owner".into(),
            message: "1".into(),
            text: "你好".into(),
        };
        repo.receive(&app, &m).unwrap();
        assert!(repo.owner(app.id).unwrap().is_none());
        assert!(repo.next().unwrap().is_none());
        m.message = "2".into();
        m.text = "我是你的主人。".into();
        repo.receive(&app, &m).unwrap();
        repo.receive(&app, &m).unwrap();
        assert_eq!(
            Repository(repo.0.clone()).owner(app.id).unwrap().unwrap(),
            ("company".into(), "owner".into(), "owner-direct-chat".into())
        );
        let claim = repo.next().unwrap().unwrap();
        repo.finish(claim.row, Ok("已记住你是我的主人")).unwrap();
        assert!(repo.next().unwrap().is_none());
        m.sender = "stranger".into();
        m.message = "3".into();
        repo.receive(&app, &m).unwrap();
        assert!(repo.next().unwrap().is_none());
        assert_eq!(repo.owner(app.id).unwrap().unwrap().1, "owner");
        m.sender = "owner".into();
        m.message = "4".into();
        m.text = "继续工作".into();
        repo.receive(&app, &m).unwrap();
        let followup = repo.next().unwrap().unwrap();
        assert_eq!(claim.session, followup.session);
        repo.finish(followup.row, Err("停止")).unwrap();
        assert!(repo.clear_owner(app.id).is_err());
        app.enabled = false;
        repo.save(&app).unwrap();
        repo.clear_owner(app.id).unwrap();
        assert!(repo.owner(app.id).unwrap().is_none());
        assert!(repo.outgoing(app.id).unwrap().is_none());
    }
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    #[test]
    fn sqlite_credentials_persist_preserve_on_edit_and_stay_out_of_application_metadata() {
        let (_dir, repo, mut app) = super::tests::fixture();
        assert!(repo.secret(app.id).is_err());
        repo.save_with_secret(&app, Some("fixture-secret-旧"))
            .unwrap();
        let reopened = Repository(repo.0.clone());
        assert_eq!(reopened.secret(app.id).unwrap(), "fixture-secret-旧");
        app.name = "重命名应用".into();
        reopened.save(&app).unwrap();
        assert_eq!(reopened.secret(app.id).unwrap(), "fixture-secret-旧");
        reopened
            .save_with_secret(&app, Some("fixture-secret-new"))
            .unwrap();
        assert_eq!(reopened.secret(app.id).unwrap(), "fixture-secret-new");
        assert!(
            !serde_json::to_string(&reopened.applications().unwrap())
                .unwrap()
                .contains("fixture-secret")
        );
        let payload: String = reopened
            .open()
            .unwrap()
            .query_row(
                "SELECT payload FROM applications WHERE id=?1",
                [app.id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!payload.contains("fixture-secret"));
        let mut invalid = app.clone();
        invalid.name = String::new();
        assert!(
            reopened
                .save_with_secret(&invalid, Some("should-not-save"))
                .is_err()
        );
        assert!(reopened.save_with_secret(&app, Some(" ")).is_err());
        assert_eq!(reopened.secret(app.id).unwrap(), "fixture-secret-new");
    }
}

#[cfg(test)]
mod reaction_tests {
    use super::*;
    #[test]
    fn receipt_is_deduplicated_and_completion_waits_for_all_reply_parts() {
        let (_dir, repo, app) = super::tests::fixture();
        let message = Incoming {
            app: app.id,
            company: "tenant-a".into(),
            chat: "chat-a".into(),
            message: "original-message".into(),
            sender: "user-a".into(),
            text: "执行任务".into(),
        };
        repo.receive(&app, &message).unwrap();
        repo.receive(&app, &message).unwrap();
        let (row, chat, original, stage) = repo.reaction(app.id).unwrap().unwrap();
        assert_eq!(chat, "chat-a");
        assert_eq!(original, "original-message");
        assert_eq!(stage, "received");
        repo.reacted(row, &stage, Ok(())).unwrap();
        assert!(repo.reaction(app.id).unwrap().is_none());
        let job = repo.next().unwrap().unwrap();
        repo.finish(job.row, Ok(&"结果".repeat(3000))).unwrap();
        assert!(repo.reaction(app.id).unwrap().is_none());
        let (id, _, _) = repo.outgoing(app.id).unwrap().unwrap();
        repo.sent(id, Ok(())).unwrap();
        assert!(repo.reaction(app.id).unwrap().is_none());
        let (id, _, _) = repo.outgoing(app.id).unwrap().unwrap();
        repo.sent(id, Ok(())).unwrap();
        let done = repo.reaction(app.id).unwrap().unwrap();
        assert_eq!(done.3, "done");
        assert_eq!(repo.applied_reactions(row).unwrap(), vec!["received"]);
        repo.removed_reaction(row, "received").unwrap();
        repo.reacted(row, "done", Ok(())).unwrap();
        assert_eq!(repo.applied_reactions(row).unwrap(), vec!["done"]);
    }
    #[test]
    fn failed_receipt_does_not_block_reply_and_generation_errors_get_terminal_reaction() {
        let (_dir, repo, app) = super::tests::fixture();
        let m = Incoming {
            app: app.id,
            company: "tenant-a".into(),
            chat: "chat-a".into(),
            message: "fail-test".into(),
            sender: "user-a".into(),
            text: "hello".into(),
        };
        repo.receive(&app, &m).unwrap();
        let (row, _, _, stage) = repo.reaction(app.id).unwrap().unwrap();
        repo.reacted(row, &stage, Err("表情接口失败".into()))
            .unwrap();
        let job = repo.next().unwrap().unwrap();
        repo.finish(job.row, Err("生成失败")).unwrap();
        assert_eq!(repo.reaction(app.id).unwrap().unwrap().3, "failed");
        assert!(repo.applied_reactions(row).unwrap().is_empty());
    }
}
