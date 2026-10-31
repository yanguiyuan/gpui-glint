//! Fixed WPS chat/tag API contracts; no arbitrary request URLs from model input.
use super::*;
use crate::tools::{ToolCall, ToolContext};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
fn identifier(args: &Value, key: &str) -> Result<String, String> {
    args[key]
        .as_str()
        .filter(|v| {
            !v.trim().is_empty()
                && v.len() <= 256
                && !matches!(*v, "." | "..")
                && !v.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .ok_or_else(|| format!("请提供有效的 {key}"))
}
fn array(args: &Value, key: &str, names: bool) -> Result<Vec<String>, String> {
    let list = args[key]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 50)
        .ok_or_else(|| format!("{key} 必须包含 1–50 项"))?;
    list.iter()
        .map(|v| {
            v.as_str()
                .filter(|v| {
                    !v.trim().is_empty()
                        && if names {
                            v.chars().count() <= 16
                        } else {
                            v.len() <= 256
                        }
                })
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} 包含无效值；标签名最多 16 个字"))
        })
        .collect()
}
fn url(segments: &[&str]) -> reqwest::Url {
    let mut url = reqwest::Url::parse("https://openapi.wps.cn/").unwrap();
    url.path_segments_mut()
        .unwrap()
        .extend(segments.iter().copied());
    url
}
fn page(url: &mut reqwest::Url, args: &Value) -> Result<(), String> {
    let size = match args.get("page_size") {
        None => 20,
        Some(v) => v
            .as_u64()
            .filter(|v| (1..=100).contains(v))
            .ok_or("page_size 必须为 1–100")?,
    };
    url.query_pairs_mut()
        .append_pair("page_size", &size.to_string());
    if let Some(token) = args.get("page_token") {
        let token = token
            .as_str()
            .filter(|v| v.len() <= 4096)
            .ok_or("page_token 无效")?;
        url.query_pairs_mut().append_pair("page_token", token);
    }
    Ok(())
}
fn contract(
    name: &str,
    args: &Value,
    chat: &str,
) -> Result<(reqwest::Method, reqwest::Url, Option<Value>), String> {
    let base = ["v7", "recent_chats"];
    match name {
        "wps_chats_list" => {
            let mut u = url(&["v7", "chats"]);
            page(&mut u, args)?;
            Ok((reqwest::Method::GET, u, None))
        }
        "wps_chat_tags_list" => {
            let mut u = url(&[&base[..], &["tags"]].concat());
            page(&mut u, args)?;
            Ok((reqwest::Method::GET, u, None))
        }
        "wps_chat_tags_create" => Ok((
            reqwest::Method::POST,
            url(&["v7", "recent_chats", "tags", "batch_create"]),
            Some(json!({"tag_names":array(args,"tag_names",true)?})),
        )),
        "wps_chat_tags_delete" => Ok((
            reqwest::Method::POST,
            url(&["v7", "recent_chats", "tags", "batch_delete"]),
            Some(json!({"tag_ids":array(args,"tag_ids",false)?})),
        )),
        "wps_chat_tags_get" => Ok((
            reqwest::Method::GET,
            url(&["v7", "recent_chats", chat, "tags"]),
            None,
        )),
        "wps_chat_tags_apply" | "wps_chat_tags_remove" => Ok((
            reqwest::Method::POST,
            url(&[
                "v7",
                "recent_chats",
                chat,
                "tags",
                if name == "wps_chat_tags_apply" {
                    "batch_create"
                } else {
                    "batch_delete"
                },
            ]),
            Some(json!({"tag_ids":array(args,"tag_ids",false)?})),
        )),
        "wps_chat_tags_chats" => {
            let id = identifier(args, "tag_id")?;
            let mut u = url(&["v7", "recent_chats", "tags", &id, "chats"]);
            page(&mut u, args)?;
            Ok((reqwest::Method::GET, u, None))
        }
        _ => Err("未知 WPS 标签工具".into()),
    }
}
static LAST_REQUEST: tokio::sync::Mutex<Option<Instant>> = tokio::sync::Mutex::const_new(None);
pub async fn execute(call: &ToolCall, context: &ToolContext) -> Result<String, String> {
    let repo = Repository::new(&context.memory.anchor);
    let app = super::tools::select(&repo, context, call.arguments["application_id"].as_str())?;
    let chat = if call.arguments.get("chat_id").is_some() {
        identifier(&call.arguments, "chat_id")?
    } else {
        repo.open()?
            .query_row(
                "SELECT chat FROM inbox WHERE app=?1 AND state='running' ORDER BY id DESC LIMIT 1",
                [app.id.to_string()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .or_else(|| repo.owner(app.id).ok().flatten().map(|(_, _, chat)| chat))
            .ok_or("请指定 chat_id")?
    };
    let (method, url, body) = contract(&call.name, &call.arguments, &chat)?;
    let access = super::auth::access(&repo, &app).await?;
    {
        let mut last = LAST_REQUEST.lock().await;
        if let Some(time) = *last {
            if let Some(wait) = Duration::from_millis(220).checked_sub(time.elapsed()) {
                tokio::time::sleep(wait).await;
            }
        }
        *last = Some(Instant::now());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法创建 WPS 标签客户端")?;
    let result = super::wps::user_api(
        &client,
        &app,
        &repo.secret(app.id)?,
        &access,
        method,
        url,
        body,
    )
    .await?;
    serde_json::to_string(&result).map_err(|_| "无法编码 WPS 标签结果".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_chat_tag_operations_use_documented_endpoints_and_fields() {
        let cases = [
            (
                "wps_chat_tags_list",
                json!({"page_size":50,"page_token":"a&b"}),
                "GET",
                "/v7/recent_chats/tags",
            ),
            (
                "wps_chat_tags_create",
                json!({"tag_names":["工作","紧急"]}),
                "POST",
                "/v7/recent_chats/tags/batch_create",
            ),
            (
                "wps_chat_tags_delete",
                json!({"tag_ids":["tag-1"]}),
                "POST",
                "/v7/recent_chats/tags/batch_delete",
            ),
            (
                "wps_chat_tags_get",
                json!({}),
                "GET",
                "/v7/recent_chats/chat-1/tags",
            ),
            (
                "wps_chat_tags_apply",
                json!({"tag_ids":["tag-1"]}),
                "POST",
                "/v7/recent_chats/chat-1/tags/batch_create",
            ),
            (
                "wps_chat_tags_remove",
                json!({"tag_ids":["tag-1"]}),
                "POST",
                "/v7/recent_chats/chat-1/tags/batch_delete",
            ),
            (
                "wps_chat_tags_chats",
                json!({"tag_id":"tag-1"}),
                "GET",
                "/v7/recent_chats/tags/tag-1/chats",
            ),
            ("wps_chats_list", json!({}), "GET", "/v7/chats"),
        ];
        for (name, args, method, path) in cases {
            let (m, u, body) = contract(name, &args, "chat-1").unwrap();
            assert_eq!(m.as_str(), method);
            assert_eq!(u.path(), path);
            if name == "wps_chat_tags_create" {
                assert_eq!(body.unwrap()["tag_names"], args["tag_names"]);
            } else if method == "POST" {
                assert_eq!(body.unwrap()["tag_ids"], args["tag_ids"]);
            } else {
                assert!(body.is_none());
            }
            if name == "wps_chat_tags_list" {
                let query: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
                assert_eq!(query["page_token"], "a&b");
            }
        }
    }
    #[test]
    fn validates_batch_sizes_unicode_names_pagination_and_path_segments() {
        assert!(
            contract(
                "wps_chat_tags_create",
                &json!({"tag_names":["中".repeat(16)]}),
                "chat"
            )
            .is_ok()
        );
        for names in [
            json!([]),
            json!(["中".repeat(17)]),
            json!([" "]),
            json!([123]),
            json!(vec!["标签"; 51]),
        ] {
            assert!(contract("wps_chat_tags_create", &json!({"tag_names":names}), "chat").is_err());
        }
        for size in [json!(0), json!(101), json!("20")] {
            assert!(contract("wps_chat_tags_list", &json!({"page_size":size}), "chat").is_err());
        }
        let (_, u, _) = contract("wps_chat_tags_get", &json!({}), "chat/with?query").unwrap();
        assert_eq!(u.path(), "/v7/recent_chats/chat%2Fwith%3Fquery/tags");
    }
}
