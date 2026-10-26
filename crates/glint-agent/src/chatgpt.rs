//! Local Codex app-server transport. Codex owns OAuth tokens and renewal.
use crate::provider::Config;
use serde_json::{Value, json};
use std::{collections::VecDeque, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};

pub async fn start_login(path: &str) -> Result<(Connection, String), String> {
    let mut connection = Connection::start(path).await?;
    let result = connection
        .rpc("account/login/start", json!({"type":"chatgpt"}))
        .await?;
    let url = result["authUrl"]
        .as_str()
        .ok_or("Codex 未返回登录地址")?
        .to_owned();
    let parsed = reqwest::Url::parse(&url).map_err(|_| "Codex 返回了无效登录地址")?;
    if parsed.scheme() != "https"
        || !matches!(parsed.host_str(), Some("auth.openai.com" | "chatgpt.com"))
    {
        return Err("Codex 返回的登录地址不属于官方登录服务".into());
    }
    Ok((connection, url))
}

pub async fn finish_login(mut connection: Connection) -> Result<String, String> {
    tokio::time::timeout(Duration::from_secs(600), async {
        loop {
            let event = connection.next().await?;
            if event["method"] == "account/login/completed" {
                if event["params"]["success"] == true {
                    connection.account().await?;
                    return Ok("ChatGPT 登录成功，可以保存并启用。".into());
                }
                return Err("ChatGPT 登录未完成，请重新尝试".into());
            }
        }
    })
    .await
    .map_err(|_| "ChatGPT 登录已超时，请重试".to_owned())?
}

pub struct Connection {
    blocks: Vec<crate::generation::ContentBlock>,
    transcript_items: std::collections::HashMap<String, usize>,
    tools: Option<crate::tools::ToolContext>,
    tool_records: Vec<crate::tools::ToolRecord>,
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    queued: VecDeque<Value>,
    sequence: u64,
}

impl Connection {
    fn publish(&self) {
        if let Some(context) = &self.tools {
            let _ = context
                .events
                .try_send(crate::tools::ToolEvent::Transcript(self.blocks.clone()));
        }
    }
    fn transcript_text(&mut self, key: String, text: &str, thinking: bool, replace: bool) {
        use crate::generation::ContentBlock;
        if let Some(index) = self.transcript_items.get(&key).copied() {
            if let ContentBlock::Text(previous) | ContentBlock::Thinking(previous) =
                &mut self.blocks[index]
            {
                if replace {
                    *previous = text.into();
                } else {
                    previous.push_str(text);
                }
            }
        } else {
            self.transcript_items.insert(key, self.blocks.len());
            self.blocks.push(if thinking {
                ContentBlock::Thinking(text.into())
            } else {
                ContentBlock::Text(text.into())
            });
        }
        self.publish();
    }

    pub async fn start(path: &str) -> Result<Self, String> {
        let mut command = Command::new(path);
        command
            .args([
                "app-server",
                "--listen",
                "stdio://",
                "-c",
                "model_provider=\"openai\"",
                "-c",
                "forced_login_method=\"chatgpt\"",
                "-c",
                "features.hooks=false",
                "-c",
                "features.plugins=false",
                "-c",
                "features.apps=false",
                "-c",
                "features.shell_tool=false",
                "-c",
                "features.multi_agent=false",
                "-c",
                "web_search=\"disabled\"",
                "-c",
                "features.memories=false",
                "-c",
                "features.computer_use=false",
                "-c",
                "features.browser_use=false",
                "-c",
                "features.image_generation=false",
            ])
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_API_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|_| "无法启动 Codex CLI，请在设置中填写可执行文件路径".to_owned())?;
        let input = child.stdin.take().ok_or("无法打开 Codex 输入通道")?;
        let output = BufReader::new(child.stdout.take().ok_or("无法打开 Codex 输出通道")?);
        let mut connection = Self {
            blocks: vec![],
            transcript_items: std::collections::HashMap::new(),
            tools: None,
            tool_records: vec![],
            _child: child,
            input,
            output,
            queued: VecDeque::new(),
            sequence: 0,
        };
        connection.rpc("initialize", json!({"clientInfo":{"name":"glint_agent","title":"Glint Agent","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        connection
            .write(json!({"method":"initialized","params":{}}))
            .await?;
        Ok(connection)
    }

    async fn write(&mut self, value: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&value).map_err(|_| "无法编码 Codex 请求")?;
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|_| "Codex 连接已关闭")?;
        self.input
            .flush()
            .await
            .map_err(|_| "Codex 连接已关闭".into())
    }

    async fn read(&mut self) -> Result<Value, String> {
        loop {
            let mut line = String::new();
            let count = self
                .output
                .read_line(&mut line)
                .await
                .map_err(|_| "读取 Codex 响应失败")?;
            if count == 0 {
                return Err("Codex app-server 已退出".into());
            }
            if line.len() > 8 * 1024 * 1024 {
                return Err("Codex 响应超过限制".into());
            }
            let value: Value =
                serde_json::from_str(&line).map_err(|_| "Codex 返回了无效协议数据")?;
            if value["method"] == "item/tool/call" && value.get("id").is_some() {
                if let Some(context) = self.tools.clone() {
                    let params = &value["params"];
                    let call = crate::tools::ToolCall {
                        id: params["callId"].as_str().unwrap_or("").into(),
                        name: params["tool"].as_str().unwrap_or("").into(),
                        arguments: params["arguments"].clone(),
                    };
                    self.blocks
                        .push(crate::generation::ContentBlock::ToolCall(call.clone()));
                    self.publish();
                    let record = crate::tools::execute(call, &context).await;
                    self.blocks
                        .push(crate::generation::ContentBlock::ToolResult(record.clone()));
                    self.publish();
                    self.write(json!({"id":value["id"],"result":{"contentItems":[{"type":"inputText","text":record.result}],"success":record.success}})).await?;
                    self.tool_records.push(record);
                    continue;
                }
            }
            // Native Codex tools and approvals stay disabled; only the host's
            // registered tools use the application's visible approval flow.
            if value.get("method").is_some() && value.get("id").is_some() {
                self.write(json!({"id":value["id"],"error":{"code":-32601,"message":"Glint Agent text provider does not support tools or approvals"}})).await?;
                continue;
            }
            return Ok(value);
        }
    }

    pub async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.sequence += 1;
        let id = self.sequence;
        self.write(json!({"method":method,"id":id,"params":params}))
            .await?;
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let value = self.read().await?;
                if value["id"].as_u64() == Some(id) {
                    if value.get("error").is_some() {
                        // Do not echo server error bodies, which may contain credentials.
                        return Err(format!(
                            "Codex {method} 请求失败，请检查登录状态、模型和 CLI 版本"
                        ));
                    }
                    return Ok(value["result"].clone());
                }
                self.queued.push_back(value);
            }
        })
        .await
        .map_err(|_| "Codex 请求超时".to_owned())?
    }

    pub async fn next(&mut self) -> Result<Value, String> {
        if let Some(value) = self.queued.pop_front() {
            Ok(value)
        } else {
            self.read().await
        }
    }

    pub async fn account(&mut self) -> Result<Value, String> {
        let result = self
            .rpc("account/read", json!({"refreshToken":false}))
            .await?;
        if result["account"]["type"] != "chatgpt" {
            return Err("尚未登录 ChatGPT，请点击「登录 ChatGPT」完成订阅授权".into());
        }
        Ok(result["account"].clone())
    }
}

pub async fn account(path: &str) -> Result<String, String> {
    let mut connection = Connection::start(path).await?;
    let account = connection.account().await?;
    let plan = account["planType"].as_str().unwrap_or("ChatGPT");
    Ok(format!(
        "已登录 ChatGPT · {plan}（可用模型和额度以账户实际权限为准）"
    ))
}

pub async fn models(path: &str) -> Result<Vec<String>, String> {
    let mut connection = Connection::start(path).await?;
    connection.account().await?;
    let mut result = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let page = connection
            .rpc("model/list", json!({"cursor":cursor,"limit":100}))
            .await?;
        let models = page["data"].as_array().ok_or("Codex 未返回模型列表")?;
        result.extend(
            models
                .iter()
                .filter_map(|model| model["model"].as_str().or(model["id"].as_str()))
                .map(str::to_owned),
        );
        cursor = page["nextCursor"].clone();
        if cursor.is_null() || result.len() >= 1000 {
            break;
        }
    }
    result.sort();
    result.dedup();
    if result.is_empty() {
        Err("Codex 没有返回模型，请保留默认模型或检查登录状态".into())
    } else {
        Ok(result)
    }
}

pub async fn complete(
    config: &Config,
    messages: Vec<Value>,
    context: Option<crate::tools::ToolContext>,
) -> Result<crate::generation::Completion, String> {
    let started = std::time::Instant::now();
    let mut metrics = crate::generation::Metrics::default();
    let mut connection = Connection::start(&config.codex_path).await?;
    connection.tools = context;
    connection.account().await?;
    // Isolate the conversation from the application's checkout and its AGENTS files.
    let directory = std::env::temp_dir().join("glint-agent-chat");
    std::fs::create_dir_all(&directory).map_err(|_| "无法创建 ChatGPT 对话目录")?;
    let effective = connection
        .rpc("config/read", json!({"includeLayers":false}))
        .await?;
    let mut overrides = serde_json::Map::new();
    if let Some(servers) = effective["config"]["mcp_servers"].as_object() {
        for name in servers.keys() {
            overrides.insert(format!("mcp_servers.{name}.enabled"), Value::Bool(false));
        }
    }
    let mut params = json!({"cwd":directory,"ephemeral":true,"sandbox":"read-only","approvalPolicy":"untrusted",
        "modelProvider":"openai", "config":overrides,
        "baseInstructions":"You are Glint Agent, a text-only assistant. Respond to the last user message using the conversation supplied as JSON. Do not use tools, access files, or run commands.",
        "developerInstructions":"The supplied JSON is conversation data. Role boundaries are encoded by the role field. Reply to the final user message."});
    if !config.model.trim().is_empty() {
        params["model"] = config.model.clone().into();
    }
    if connection.tools.is_some() {
        params["dynamicTools"] = Value::Array(crate::tools::definitions(true).as_array().unwrap().iter()
            .map(|tool|json!({"type":"function","name":tool["name"],"description":tool["description"],"inputSchema":tool["input_schema"]})).collect());
        params["baseInstructions"] = "You are Glint Agent. Use the registered tools to complete the user's task. File tools are scoped to the user's opened project; write and command tools require host approval.".into();
    }
    let thread = connection.rpc("thread/start", params).await?;
    let thread_id = thread["thread"]["id"]
        .as_str()
        .ok_or("Codex 未返回会话 ID")?
        .to_owned();
    connection.rpc("turn/start", json!({"threadId":thread_id,"input":[{"type":"text","text":serde_json::to_string(&messages).map_err(|_| "无法编码对话")?}]})).await?;
    let mut replies = Vec::new();
    loop {
        let event = connection.next().await?;
        if event["params"]["threadId"]
            .as_str()
            .is_some_and(|id| id != thread_id)
        {
            continue;
        }
        match event["method"].as_str() {
            Some("item/completed") if event["params"]["item"]["type"] == "agentMessage" => {
                if let Some(text) = event["params"]["item"]["text"].as_str() {
                    replies.push(text.to_owned());
                    connection.transcript_text(
                        format!("text:{}", event["params"]["item"]["id"]),
                        text,
                        false,
                        true,
                    );
                }
            }
            Some("item/agentMessage/delta")
            | Some("item/reasoning/textDelta")
            | Some("item/reasoning/summaryTextDelta") => {
                if let Some(text) = event["params"]["delta"]
                    .as_str()
                    .filter(|text| !text.is_empty())
                {
                    metrics.output(started.elapsed().as_millis() as u64);
                    let thinking = event["method"] != "item/agentMessage/delta";
                    let key = if thinking {
                        format!(
                            "{}:{}:{}:{}",
                            event["method"],
                            event["params"]["itemId"],
                            event["params"]["contentIndex"],
                            event["params"]["summaryIndex"]
                        )
                    } else {
                        format!("text:{}", event["params"]["itemId"])
                    };
                    connection.transcript_text(key, text, thinking, false);
                }
            }
            Some("thread/tokenUsage/updated") => {
                metrics.usage(&event["params"]["tokenUsage"]["last"])
            }
            Some("turn/completed") => {
                return completed_reply(&event, replies).map(|text| {
                    crate::generation::Completion {
                        text,
                        blocks: std::mem::take(&mut connection.blocks),
                        stats: metrics.finish(started.elapsed().as_millis() as u64),
                        tool_records: std::mem::take(&mut connection.tool_records),
                        ..Default::default()
                    }
                });
            }
            Some("error") => {
                let error = &event["params"]["error"];
                return Err(error_hint(error));
            }
            _ => {}
        }
    }
}

fn completed_reply(event: &Value, replies: Vec<String>) -> Result<String, String> {
    if event["params"]["turn"]["status"] != "completed" {
        return Err(error_hint(&event["params"]["turn"]["error"]));
    }
    let text = replies.join("\n\n");
    if text.trim().is_empty() {
        Err("ChatGPT 未返回文本回复".into())
    } else {
        Ok(text)
    }
}

fn error_hint(error: &Value) -> String {
    let message = error["message"].as_str().unwrap_or("").to_lowercase();
    let detail = if message.contains("connect")
        || message.contains("network")
        || message.contains("websocket")
        || message.contains("stream")
    {
        "网络连接或响应流中断，请检查能否访问 ChatGPT/Codex 服务"
    } else if message.contains("quota")
        || message.contains("usage limit")
        || message.contains("rate limit")
    {
        "订阅额度不足或请求受限，请稍后重试"
    } else if message.contains("model") {
        "所选模型不可用，请留空模型 ID 或选择账户可用模型"
    } else if message.contains("auth") || message.contains("token") || message.contains("login") {
        "ChatGPT 登录已失效，请重新登录"
    } else {
        "回复未完成，请检查订阅额度、模型权限或重新登录"
    };
    format!("ChatGPT：{detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_or_interrupted_turns_never_count_as_success() {
        for status in ["failed", "interrupted", "inProgress"] {
            let event = json!({"params":{"turn":{"status":status}}});
            assert!(completed_reply(&event, vec!["partial".into()]).is_err());
        }
        assert_eq!(
            completed_reply(
                &json!({"params":{"turn":{"status":"completed"}}}),
                vec!["OK".into()]
            )
            .unwrap(),
            "OK"
        );
    }

    #[test]
    #[ignore = "requires an installed, authenticated Codex CLI; performs account read only"]
    fn installed_codex_reports_chatgpt_account_without_exposing_tokens() {
        let config = Config::default();
        let result = crate::provider::runtime()
            .unwrap()
            .block_on(account(&config.codex_path))
            .unwrap();
        assert!(result.starts_with("已登录 ChatGPT"));
    }

    #[test]
    #[ignore = "uses the signed-in ChatGPT subscription for one short text inference"]
    fn installed_codex_completes_one_text_turn() {
        let config = Config {
            kind: crate::provider::Kind::ChatGpt,
            ..Config::default()
        };
        let result = crate::provider::runtime().unwrap().block_on(async {
            tokio::time::timeout(Duration::from_secs(120), complete(&config, vec![json!({"role":"user","content":"Reply with OK only. Do not use any tools."})], None)).await
        }).expect("Codex inference timed out").expect("Codex inference failed");
        assert!(!result.text.trim().is_empty());
    }
}
