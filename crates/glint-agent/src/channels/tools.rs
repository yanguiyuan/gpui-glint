//! Notification tools are scoped to the configured partner and its project's owner.
use super::*;
use crate::tools::{ToolCall, ToolContext};
use serde_json::{Value, json};
fn authorized(app: &Application, context: &ToolContext) -> bool {
    context.memory.partner == Some(app.partner)
        && context
            .root
            .as_ref()
            .is_some_and(|root| std::fs::canonicalize(root).ok().as_ref() == Some(&app.project))
}
pub(super) fn select(
    repo: &Repository,
    context: &ToolContext,
    id: Option<&str>,
) -> Result<Application, String> {
    let apps: Vec<_> = repo
        .applications()?
        .into_iter()
        .filter(|a| {
            a.enabled && authorized(a, context) && id.is_none_or(|id| a.id.to_string() == id)
        })
        .collect();
    if apps.len() != 1 {
        return Err(
            "请用 channel_list 查看当前伙伴可用频道；有多个应用时需指定 application_id".into(),
        );
    }
    Ok(apps.into_iter().next().unwrap())
}
fn status(repo: &Repository, inbox: i64) -> Result<Value, String> {
    let db = repo.open()?;
    let mut q = db
        .prepare("SELECT state,error FROM outbox WHERE inbox=?1 ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map([inbox], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Err("通知不存在".into());
    }
    let state = if rows.iter().any(|(s, _)| s == "failed") {
        "failed"
    } else if rows.iter().all(|(s, _)| s == "sent") {
        "sent"
    } else {
        "queued"
    };
    Ok(
        json!({"delivery_id":inbox,"status":state,"error":rows.iter().find_map(|(_,e)|e.as_ref()),"note":"Only status=sent means WPS accepted every message part; queued is not delivered."}),
    )
}
pub async fn execute(call: &ToolCall, context: &ToolContext) -> Result<String, String> {
    let repo = Repository::new(&context.memory.anchor);
    if call.name == "channel_list" {
        let apps=repo.applications()?.into_iter().filter(|a|authorized(a,context)).map(|a|{
            let owner=repo.owner(a.id)?;
            Ok(json!({"application_id":a.id,"platform":"wps","name":a.name,"enabled":a.enabled,"owner_bound":owner.is_some(),"user_authorization":super::auth::status(&repo,a.id)?}))
        }).collect::<Result<Vec<_>,String>>()?;
        return Ok(json!({"applications":apps,"instructions":"Only send notifications to the bound owner. Enable the application and have the owner send 我是你的主人 in its WPS robot direct chat first."}).to_string());
    }
    if call.name == "channel_user_authorize" || call.name == "channel_user_authorization_status" {
        let app = select(&repo, context, call.arguments["application_id"].as_str())?;
        if call.name == "channel_user_authorization_status" {
            return Ok(super::auth::status(&repo, app.id)?.to_string());
        }
        let url = super::auth::begin(repo, app)?;
        return Ok(json!({"authorization_url":url,"status":"pending","redirect_uri":super::auth::REDIRECT,"instructions":"请主人在运行 Glint 的电脑浏览器打开链接，确认当前登录的是自己的 WPS 账号后同意授权。此链接只是发起授权，不能声称已授权；完成后调用 channel_user_authorization_status 检查。应用后台须登记此 redirect_uri 并申请 kso.chat.readwrite 用户权限。不要要求用户提供密码、授权码或 token。"}).to_string());
    }
    if call.name == "channel_delivery_status" {
        let id = call.arguments["delivery_id"]
            .as_i64()
            .ok_or("缺少 delivery_id")?;
        let app: String = repo
            .open()?
            .query_row("SELECT app FROM inbox WHERE id=?1", [id], |r| r.get(0))
            .map_err(|_| "通知不存在")?;
        select(&repo, context, Some(&app))?;
        return Ok(status(&repo, id)?.to_string());
    }
    let text = call.arguments["text"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or("通知文本不能为空")?;
    if text.chars().count() > 16000 {
        return Err("通知不能超过 16000 字符".into());
    }
    let app = select(&repo, context, call.arguments["application_id"].as_str())?;
    // This transaction validates the owner at enqueue time. Sending rechecks the owner.
    let id = {
        let mut db = repo.open()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (company, sender, chat): (String, String, String) = tx
            .query_row(
                "SELECT company,sender,chat FROM owners WHERE app=?1",
                [app.id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| "应用还没有绑定主人，请先在机器人单聊发送：我是你的主人")?;
        let message = format!("notification:{}", call.id);
        let inserted=tx.execute("INSERT OR IGNORE INTO inbox(app,company,chat,message,sender,text,state) VALUES(?1,?2,?3,?4,?5,?6,'replied')",params![app.id.to_string(),company,chat,message,sender,text]).map_err(|e|e.to_string())?;
        let id: i64 = tx
            .query_row(
                "SELECT id FROM inbox WHERE app=?1 AND company=?2 AND chat=?3 AND message=?4",
                params![app.id.to_string(), company, chat, message],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if inserted > 0 {
            for part in split_reply(text) {
                tx.execute(
                    "INSERT INTO outbox(inbox,app,chat,text) VALUES(?1,?2,?3,?4)",
                    params![id, app.id.to_string(), chat, part],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        id
    };
    // Let the independent channel worker deliver; never report a queue insert as sent.
    for _ in 0..60 {
        let state = status(&repo, id)?;
        if state["status"] != "queued" {
            return Ok(state.to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Ok(status(&repo, id)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context(path: &Path, app: &Application) -> ToolContext {
        let (events, _) = async_channel::unbounded();
        ToolContext {
            steering: None,
            todos: Default::default(),
            subagents: Default::default(),
            delegation: Default::default(),
            registry: crate::tool_registry::ToolRegistry::new(path.join("mcp.json")),
            skills: std::sync::Arc::new(crate::skills::Catalog::empty(path.join("sessions.json"))),
            memory: crate::memory::Repository::new(path.join("sessions.json"))
                .for_partner(Some(app.partner)),
            access: std::sync::Arc::new(std::sync::Mutex::new(crate::tools::AccessMode::Ask)),
            root: Some(app.project.clone()),
            schedules: std::sync::Arc::new(std::sync::Mutex::new(
                crate::schedules::ScheduleStore::open(path.join("schedules.json")).unwrap(),
            )),
            provider_id: Uuid::new_v4(),
            model: "test".into(),
            events,
            recall: None,
        }
    }
    #[tokio::test]
    async fn proactive_notification_uses_bound_owner_and_reports_delivery_not_queueing() {
        let (dir, repo, app) = crate::channels::tests::fixture();
        let mut context = context(dir.path(), &app);
        // Reflection/scheduled tasks can notify their configured owner without a desktop prompt.
        context.memory.reflection_only = true;
        let call = ToolCall {
            id: "notify-once".into(),
            name: "channel_send".into(),
            arguments: json!({"text":"定时任务已完成"}),
        };
        let send = crate::tools::execute(call.clone(), &context);
        let deliver = async {
            loop {
                if let Some((id, chat, text)) = repo.outgoing(app.id).unwrap() {
                    assert_eq!(chat, "chat-a");
                    assert_eq!(text, "定时任务已完成");
                    repo.sent(id, Ok(())).unwrap();
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        let (record, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(send, deliver)
        })
        .await
        .unwrap();
        assert!(record.success, "{}", record.result);
        let result: Value = serde_json::from_str(&record.result).unwrap();
        assert_eq!(result["status"], "sent");
        let again: Value = serde_json::from_str(&execute(&call, &context).await.unwrap()).unwrap();
        assert_eq!(again["delivery_id"], result["delivery_id"]);
        assert!(repo.outgoing(app.id).unwrap().is_none());
        let status_call = ToolCall {
            id: "status".into(),
            name: "channel_delivery_status".into(),
            arguments: json!({"delivery_id":result["delivery_id"]}),
        };
        context.memory.partner = Some(Uuid::new_v4());
        assert!(execute(&status_call, &context).await.is_err());
        assert!(execute(&call, &context).await.is_err());
        context.memory.partner = Some(app.partner);
        context.root = Some(dir.path().join("other-project"));
        assert!(execute(&call, &context).await.is_err());
    }
}
