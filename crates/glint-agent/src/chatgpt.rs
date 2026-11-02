//! Direct ChatGPT subscription transport: OAuth and stateless Responses HTTP/SSE.
use crate::generation::{Completion, ContentBlock, Metrics};
use crate::provider::{Config, ModelCatalog, Thinking};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};
mod auth;
pub use auth::{account, finish_login, saved_account_label, sign_out, start_login};

fn http_error(status: reqwest::StatusCode) -> String {
    format!(
        "ChatGPT HTTP {}：{}",
        status.as_u16(),
        match status.as_u16() {
            401 => "登录已失效，请重新登录",
            403 => "未授权使用订阅，或账号/工作区不允许访问",
            429 => "订阅额度不足或请求受限，请稍后重试",
            _ => "请求失败，请检查网络、模型权限或重新授权",
        }
    )
}

async fn response_json(mut response: reqwest::Response) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(http_error(response.status()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(30), response.chunk())
        .await
        .map_err(|_| "ChatGPT 网络响应超时")?
        .map_err(|_| "读取 ChatGPT 响应失败")?
    {
        if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
            return Err("ChatGPT 响应超过 4MB 限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "ChatGPT 返回了无效 JSON".into())
}

pub async fn models(config: &Config) -> Result<ModelCatalog, String> {
    let client = http_client().await?;
    let mut token = auth::access(config, None).await?;
    let mut response = client
        .get("https://api.openai.com/v1/models")
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|_| "无法连接 ChatGPT 模型服务")?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        token = auth::access(config, Some(&token)).await?;
        response = client
            .get("https://api.openai.com/v1/models")
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| "无法连接 ChatGPT 模型服务")?;
    }
    parse_models(&response_json(response).await?)
}

fn parse_models(value: &Value) -> Result<ModelCatalog, String> {
    let entries = value["models"]
        .as_array()
        .ok_or("ChatGPT 未返回账号模型列表")?;
    let mut catalog = ModelCatalog::default();
    for model in entries.iter().filter(|m| m["visibility"] == "list") {
        if let Some(slug) = model["slug"].as_str().filter(|s| !s.is_empty()) {
            if !catalog.models.iter().any(|m| m == slug) {
                catalog.models.push(slug.into());
            }
            if let Some(capacity) = model["context_window"].as_u64().filter(|v| *v > 0) {
                catalog.context_windows.insert(slug.into(), capacity);
            }
        }
    }
    if catalog.models.is_empty() {
        return Err("ChatGPT 账号没有可用模型，请检查订阅授权".into());
    }
    Ok(catalog)
}

fn conversation_input(messages: Vec<Value>) -> Result<Vec<Value>, String> {
    let mut input = Vec::new();
    for message in messages {
        if let Some(output) = message["responses_output"].as_array() {
            input.extend(output.iter().cloned());
            continue;
        }
        let role = message["role"].as_str().ok_or("对话缺少角色")?;
        if role == "tool" {
            input.push(json!({"type":"function_call_output","call_id":message["tool_call_id"],"output":message["content"]}));
            continue;
        }
        let role = match role {
            "system" => "developer",
            "developer" | "user" | "assistant" => role,
            _ => return Err("不支持的对话角色".into()),
        };
        let content = if let Some(text) = message["content"].as_str() {
            json!(text)
        } else {
            let mut content = Vec::new();
            for part in message["content"].as_array().ok_or("无效的对话内容")? {
                content.push(match part["type"].as_str() {
                    Some("text") => json!({"type":"input_text","text":part["text"]}),
                    Some("image_url") => json!({"type":"input_image","image_url":part["image_url"]["url"]}),
                    Some("image") if part["source"]["type"] == "base64" => json!({"type":"input_image","image_url":format!("data:{};base64,{}",part["source"]["media_type"].as_str().ok_or("图片缺少类型")?,part["source"]["data"].as_str().ok_or("图片缺少数据")?)}),
                    _ => return Err("不支持的 ChatGPT 消息内容".into()),
                });
            }
            json!(content)
        };
        input.push(json!({"role":role,"content":content}));
    }
    Ok(input)
}

fn request_body(
    model: &str,
    messages: Vec<Value>,
    thinking: Thinking,
    definitions: Option<Value>,
) -> Result<Value, String> {
    let mut body = json!({"model":model,"input":conversation_input(messages)?,"store":false,"stream":true,"include":["reasoning.encrypted_content"]});
    let effort = match thinking {
        Thinking::Default => None,
        Thinking::Off => Some("none"),
        Thinking::Low => Some("low"),
        Thinking::Medium => Some("medium"),
        Thinking::High => Some("high"),
        Thinking::Max => Some("xhigh"),
    };
    if let Some(effort) = effort {
        body["reasoning"] = json!({"effort":effort,"summary":"auto"});
    }
    if let Some(definitions) = definitions {
        let tools: Vec<_> = definitions.as_array().ok_or("无效工具定义")?.iter().map(|t| json!({"type":"function","name":t["name"],"description":t["description"],"parameters":t["input_schema"],"strict":false})).collect();
        if !tools.is_empty() {
            body["tools"] = json!([{"type":"namespace","name":"glint","description":"Tools executed by the Glint host under the user's access policy.","tools":tools}]);
        }
    }
    Ok(body)
}

#[derive(Default)]
struct ResponseStream {
    pending: Vec<u8>,
    data: Vec<String>,
    items: BTreeMap<usize, Value>,
    metrics: Metrics,
    completed: bool,
}

impl ResponseStream {
    fn push(&mut self, bytes: &[u8], elapsed: u64) -> Result<(), String> {
        self.pending.extend_from_slice(bytes);
        while let Some(index) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = self.pending.drain(..=index).collect();
            let line = std::str::from_utf8(&line)
                .map_err(|_| "ChatGPT 流包含无效 UTF-8")?
                .trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    let data = std::mem::take(&mut self.data).join("\n");
                    if data != "[DONE]" {
                        self.event(
                            serde_json::from_str(&data).map_err(|_| "ChatGPT 流事件格式无效")?,
                            elapsed,
                        )?;
                    }
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data
                    .push(data.strip_prefix(' ').unwrap_or(data).into());
            }
        }
        Ok(())
    }

    fn event(&mut self, event: Value, elapsed: u64) -> Result<(), String> {
        if self.completed {
            return Ok(());
        }
        let index = event["output_index"].as_u64().unwrap_or(0) as usize;
        match event["type"].as_str() {
            Some("response.output_item.added" | "response.output_item.done") => {
                self.items.insert(index, event["item"].clone());
            }
            Some("response.content_part.added") => {
                let i = event["content_index"].as_u64().unwrap_or(0) as usize;
                if i > 4096 {
                    return Err("ChatGPT 内容片段过多".into());
                }
                if let Some(parts) = self
                    .items
                    .get_mut(&index)
                    .and_then(|item| item["content"].as_array_mut())
                {
                    while parts.len() <= i {
                        parts.push(Value::Null);
                    }
                    parts[i] = event["part"].clone();
                }
            }
            Some("response.reasoning_summary_part.added") => {
                let i = event["summary_index"].as_u64().unwrap_or(0) as usize;
                if i > 4096 {
                    return Err("ChatGPT 思考片段过多".into());
                }
                if let Some(parts) = self
                    .items
                    .get_mut(&index)
                    .and_then(|item| item["summary"].as_array_mut())
                {
                    while parts.len() <= i {
                        parts.push(Value::Null);
                    }
                    parts[i] = event["part"].clone();
                }
            }
            Some(
                "response.output_text.delta"
                | "response.reasoning_summary_text.delta"
                | "response.refusal.delta",
            ) => {
                if let Some(delta) = event["delta"].as_str() {
                    self.metrics.output(elapsed);
                    let thinking = event["type"] == "response.reasoning_summary_text.delta";
                    let field = if thinking { "summary" } else { "content" };
                    let part = event[if thinking {
                        "summary_index"
                    } else {
                        "content_index"
                    }]
                    .as_u64()
                    .unwrap_or(0) as usize;
                    let text_field = if event["type"] == "response.refusal.delta" {
                        "refusal"
                    } else {
                        "text"
                    };
                    if let Some(value) = self
                        .items
                        .get_mut(&index)
                        .and_then(|item| item[field].as_array_mut())
                        .and_then(|parts| parts.get_mut(part))
                    {
                        let mut text = value[text_field].as_str().unwrap_or_default().to_owned();
                        text.push_str(delta);
                        value[text_field] = text.into();
                    }
                }
            }
            Some("response.function_call_arguments.delta") => {
                if let Some(item) = self.items.get_mut(&index) {
                    let mut args = item["arguments"].as_str().unwrap_or_default().to_owned();
                    args.push_str(event["delta"].as_str().unwrap_or_default());
                    item["arguments"] = args.into();
                }
            }
            Some("response.completed") => {
                if event["response"]["status"] != "completed" {
                    return Err("ChatGPT 回复未正常完成".into());
                }
                let output = event["response"]["output"]
                    .as_array()
                    .ok_or("ChatGPT 完成事件缺少输出")?;
                self.items = output.iter().cloned().enumerate().collect();
                self.metrics.usage(&event["response"]["usage"]);
                self.completed = true;
            }
            Some("response.failed" | "response.incomplete" | "error") => {
                let code = event["response"]["error"]["code"]
                    .as_str()
                    .or(event["error"]["code"].as_str())
                    .or(event["code"].as_str())
                    .unwrap_or_default();
                return Err(match code {
                    "subscription_sharing_usage_limit_exceeded" => {
                        "ChatGPT 订阅授权额度已用尽，请在 ChatGPT 设置 → 用量中检查应用限额"
                    }
                    "subscription_sharing_usage_unavailable" => {
                        "ChatGPT 订阅当前不可用，请检查订阅授权与账号权限"
                    }
                    "context_length_exceeded" => "ChatGPT 上下文已满，请压缩或新建会话",
                    _ => "ChatGPT 回复未完成，请检查额度、网络或重新登录",
                }
                .into());
            }
            _ => {}
        }
        Ok(())
    }

    fn snapshot(&self) -> Vec<ContentBlock> {
        let mut blocks = Vec::new();
        for item in self.items.values() {
            match item["type"].as_str() {
                Some("message") => {
                    for part in item["content"].as_array().into_iter().flatten() {
                        if let Some(text) = part["text"]
                            .as_str()
                            .or(part["refusal"].as_str())
                            .filter(|s| !s.is_empty())
                        {
                            blocks.push(ContentBlock::Text(text.into()));
                        }
                    }
                }
                Some("reasoning") => {
                    for part in item["summary"].as_array().into_iter().flatten() {
                        if let Some(text) = part["text"].as_str().filter(|s| !s.is_empty()) {
                            blocks.push(ContentBlock::Thinking(text.into()));
                        }
                    }
                }
                Some("function_call") => {
                    if let (Some(id), Some(name)) =
                        (item["call_id"].as_str(), item["name"].as_str())
                    {
                        blocks.push(ContentBlock::ToolCall(crate::tools::ToolCall {
                            id: id.into(),
                            name: name.into(),
                            arguments: serde_json::from_str(
                                item["arguments"].as_str().unwrap_or_default(),
                            )
                            .unwrap_or_else(|_| json!({})),
                        }));
                    }
                }
                _ => {}
            }
        }
        blocks
    }

    fn finish(self, elapsed: u64) -> Result<Completion, String> {
        if !self.completed {
            return Err("ChatGPT 响应流中断，未收到完成事件；请重试".into());
        }
        let raw: Vec<_> = self.items.values().cloned().collect();
        let mut ids = std::collections::HashSet::new();
        let mut calls = Vec::new();
        for item in raw.iter().filter(|item| item["type"] == "function_call") {
            if item["namespace"].as_str().is_some_and(|n| n != "glint") {
                return Err("ChatGPT 请求了未知工具命名空间".into());
            }
            let id = item["call_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("ChatGPT 工具调用缺少标识")?;
            if !ids.insert(id) {
                return Err("ChatGPT 返回重复的工具调用标识".into());
            }
            calls.push(
                json!({"id":id,"function":{"name":item["name"],"arguments":item["arguments"]}}),
            );
        }
        let decoded = crate::tools::decode_calls(&json!({"tool_calls":calls}), false)?;
        let blocks = self.snapshot();
        let text = blocks
            .iter()
            .filter_map(|b| {
                if let ContentBlock::Text(text) = b {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        if text.trim().is_empty() && decoded.calls.is_empty() {
            return Err("ChatGPT 未返回文本或工具调用".into());
        }
        Ok(Completion {
            blocks,
            text,
            stats: self.metrics.finish(elapsed),
            tool_calls: decoded.calls,
            argument_errors: decoded.errors,
            raw_content: json!(raw),
            ..Default::default()
        })
    }
}

pub async fn single_request(
    config: &Config,
    mut messages: Vec<Value>,
    thinking: Thinking,
    context: Option<&crate::tools::ToolContext>,
    prefix: &[ContentBlock],
) -> Result<Completion, String> {
    let started = std::time::Instant::now();
    let client = http_client().await?;
    let mut token = auth::access(config, None).await?;
    let model = if config.model.trim().is_empty() {
        models(config)
            .await?
            .models
            .into_iter()
            .next()
            .ok_or("请选择 ChatGPT 模型")?
    } else {
        config.model.clone()
    };
    if context.is_some() {
        messages.insert(0,json!({"role":"developer","content":"You are Glint Agent. Use the registered host tools to complete the user's task. Host tools enforce the user's selected access mode. Only claim an action succeeded after a tool confirms success."}));
    }
    let body = request_body(
        &model,
        messages,
        thinking,
        context.map(|c| c.definitions(true)),
    )?;
    let mut response = tokio::time::timeout(
        Duration::from_secs(30),
        client
            .post("https://api.openai.com/v1/responses")
            .bearer_auth(&token)
            .json(&body)
            .send(),
    )
    .await
    .map_err(|_| "ChatGPT 连接超时")?
    .map_err(|_| "无法连接 ChatGPT 模型服务")?;
    // Retry authentication once, only before any SSE output or host tool effects.
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        token = auth::access(config, Some(&token)).await?;
        response = tokio::time::timeout(
            Duration::from_secs(30),
            client
                .post("https://api.openai.com/v1/responses")
                .bearer_auth(&token)
                .json(&body)
                .send(),
        )
        .await
        .map_err(|_| "ChatGPT 连接超时")?
        .map_err(|_| "无法连接 ChatGPT 模型服务")?;
    }
    receive_response(config, &model, response, context, prefix, started).await
}

async fn receive_response(
    config: &Config,
    model: &str,
    mut response: reqwest::Response,
    context: Option<&crate::tools::ToolContext>,
    prefix: &[ContentBlock],
    started: std::time::Instant,
) -> Result<Completion, String> {
    if !response.status().is_success() {
        return Err(http_error(response.status()));
    }
    if !response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"))
    {
        return Err("ChatGPT 未返回预期的事件流".into());
    }
    let mut stream = ResponseStream::default();
    let mut bytes = 0;
    let mut publish = std::time::Instant::now() - Duration::from_millis(50);
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(180), response.chunk())
        .await
        .map_err(|_| "ChatGPT 响应等待超时")?
        .map_err(|_| "读取 ChatGPT 响应流失败")?
    {
        bytes += chunk.len();
        if bytes > 4 * 1024 * 1024 {
            return Err("ChatGPT 响应超过 4MB 限制".into());
        }
        stream.push(&chunk, started.elapsed().as_millis() as u64)?;
        if let Some(context) = context {
            if stream.completed || publish.elapsed() >= Duration::from_millis(50) {
                let mut blocks = prefix.to_vec();
                blocks.extend(stream.snapshot());
                let _ = context
                    .events
                    .try_send(crate::tools::ToolEvent::Transcript(blocks));
                publish = std::time::Instant::now();
            }
        }
        if stream.completed {
            break;
        }
    }
    stream.metrics.set_context_window(
        config
            .context_window_overrides
            .get(model)
            .or_else(|| config.model_context_windows.get(model))
            .copied(),
    );
    let reply = stream.finish(started.elapsed().as_millis() as u64)?;
    Ok(reply)
}

async fn http_client() -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::none());
    #[cfg(target_os = "macos")]
    let builder = {
        let mut builder = builder;
        if ![
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ]
        .iter()
        .any(|key| std::env::var_os(key).is_some())
        {
            if let Ok(output) = crate::child_process::async_command("/usr/sbin/scutil")
                .arg("--proxy")
                .kill_on_drop(true)
                .output()
                .await
            {
                if output.status.success() {
                    for (key, address) in system_proxy_env(&String::from_utf8_lossy(&output.stdout))
                    {
                        let proxy = match key {
                            "HTTP_PROXY" => reqwest::Proxy::http(address),
                            "HTTPS_PROXY" => reqwest::Proxy::https(address),
                            _ => reqwest::Proxy::all(address),
                        };
                        if let Ok(proxy) = proxy {
                            builder = builder.proxy(proxy);
                        }
                    }
                }
            }
        }
        builder
    };
    builder
        .build()
        .map_err(|_| "无法初始化 ChatGPT 网络客户端".into())
}

#[cfg(any(target_os = "macos", test))]
fn system_proxy_env(settings: &str) -> Vec<(&'static str, String)> {
    let fields: std::collections::HashMap<_, _> = settings
        .lines()
        .filter_map(|line| line.trim().split_once(" : "))
        .collect();
    let mut proxies = Vec::new();
    for (prefix, key, scheme) in [
        ("HTTP", "HTTP_PROXY", "http"),
        ("HTTPS", "HTTPS_PROXY", "http"),
        ("SOCKS", "ALL_PROXY", "socks5h"),
    ] {
        if fields.get(format!("{prefix}Enable").as_str()) != Some(&"1") {
            continue;
        }
        let Some(host) = fields.get(format!("{prefix}Proxy").as_str()) else {
            continue;
        };
        let Some(port) = fields
            .get(format!("{prefix}Port").as_str())
            .and_then(|port| port.parse::<u16>().ok())
            .filter(|port| *port != 0)
        else {
            continue;
        };
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        let candidate = format!("{scheme}://{host}:{port}");
        if let Ok(url) = reqwest::Url::parse(&candidate)
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(url.path(), "" | "/")
            && url.query().is_none()
            && url.fragment().is_none()
        {
            proxies.push((key, candidate));
        }
    }
    proxies
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_proxy_parser_rejects_bad_ports_and_preserves_protocols() {
        assert_eq!(
            system_proxy_env(
                "HTTPEnable : 1\nHTTPProxy : proxy.example\nHTTPPort : 8080\nHTTPSEnable : 1\nHTTPSProxy : localhost\nHTTPSPort : 7890"
            ),
            vec![
                ("HTTP_PROXY", "http://proxy.example:8080".into()),
                ("HTTPS_PROXY", "http://localhost:7890".into())
            ]
        );
        assert!(system_proxy_env("HTTPEnable : 1\nHTTPProxy : localhost\nHTTPPort : 0").is_empty());
    }

    fn event(value: Value) -> String {
        format!("data: {value}\r\n\r\n")
    }
    fn completed(output: Value) -> Value {
        json!({"type":"response.completed","response":{"status":"completed","output":output,"usage":{"input_tokens":100,"output_tokens":10,"input_tokens_details":{"cached_tokens":50}}}})
    }
    fn message(text: &str) -> Value {
        json!({"id":"msg_1","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":text,"annotations":[]}]})
    }

    #[test]
    fn request_uses_direct_subscription_contract_and_preserves_roles_and_images() {
        let body = request_body("account-model",vec![json!({"role":"system","content":"instructions"}), json!({"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"data:image/png;base64,YQ=="}}]}),json!({"role":"assistant","content":"earlier"})],Thinking::High,Some(json!([{"name":"read_file","description":"Read","input_schema":{"type":"object"}}]))).unwrap();
        assert_eq!(body["model"], "account-model");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["input"][0]["role"], "developer");
        assert!(body["input"][0].get("type").is_none());
        assert_eq!(
            body["input"][1]["content"][1],
            json!({"type":"input_image","image_url":"data:image/png;base64,YQ=="})
        );
        assert_eq!(body["input"][2]["role"], "assistant");
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["tools"][0]["type"], "namespace");
        assert_eq!(body["tools"][0]["tools"][0]["name"], "read_file");
        for field in [
            "previous_response_id",
            "conversation",
            "max_output_tokens",
            "temperature",
            "metadata",
        ] {
            assert!(body.get(field).is_none());
        }
    }

    #[test]
    fn fragmented_sse_streams_thinking_and_text_and_counts_usage_once() {
        let thinking =
            json!({"id":"rs_1","type":"reasoning","summary":[],"encrypted_content":"opaque"});
        let mut final_thinking = thinking.clone();
        final_thinking["summary"] = json!([{"type":"summary_text","text":"plan"}]);
        let frames = vec![
            json!({"type":"response.output_item.added","output_index":0,"item":thinking}),
            json!({"type":"response.reasoning_summary_part.added","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":""}}),
            json!({"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":0,"delta":"plan"}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"msg_1","type":"message","role":"assistant","content":[]}}),
            json!({"type":"response.content_part.added","output_index":1,"content_index":0,"part":{"type":"output_text","text":""}}),
            json!({"type":"response.output_text.delta","output_index":1,"content_index":0,"delta":"你好"}),
        ];
        let data = frames.into_iter().map(event).collect::<String>();
        let mut stream = ResponseStream::default();
        for (i, byte) in data.as_bytes().iter().enumerate() {
            stream.push(&[*byte], i as u64).unwrap();
        }
        assert!(
            matches!(&stream.snapshot()[..],[ContentBlock::Thinking(plan),ContentBlock::Text(text)] if plan=="plan" && text=="你好")
        );
        let done = event(completed(json!([final_thinking, message("你好")])));
        for chunk in done.as_bytes().chunks(3) {
            stream.push(chunk, 1000).unwrap();
        }
        let reply = stream.finish(1100).unwrap();
        assert_eq!(reply.text, "你好");
        assert_eq!(reply.stats.input_tokens, Some(100));
        assert_eq!(reply.stats.context_tokens, Some(110));
        assert_eq!(reply.raw_content[0]["encrypted_content"], "opaque");
        assert_eq!(reply.blocks.len(), 2);
    }

    #[test]
    fn tools_keep_encrypted_reasoning_and_call_result_pairing_without_replaying_effects() {
        let output = json!([{"id":"rs_1","type":"reasoning","encrypted_content":"opaque","summary":[]},{"id":"fc_1","type":"function_call","namespace":"glint","call_id":"call_1","name":"read_file","arguments":"{\"path\":\"a.rs\"}"}]);
        let mut stream = ResponseStream::default();
        stream.event(completed(output.clone()), 10).unwrap();
        let reply = stream.finish(20).unwrap();
        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].arguments["path"], "a.rs");
        let next = conversation_input(vec![
            json!({"responses_output":reply.raw_content}),
            json!({"role":"tool","tool_call_id":"call_1","content":"file contents"}),
            json!({"role":"system","content":"recalled memory"}),
        ])
        .unwrap();
        assert_eq!(&next[..2], output.as_array().unwrap().as_slice());
        assert_eq!(
            next[2],
            json!({"type":"function_call_output","call_id":"call_1","output":"file contents"})
        );
        assert_eq!(next[3]["role"], "developer");
        let mut invalid = output.clone();
        invalid[1]["arguments"] = json!("{broken");
        let mut stream = ResponseStream::default();
        stream.event(completed(invalid), 0).unwrap();
        assert!(
            stream
                .finish(1)
                .unwrap()
                .argument_errors
                .contains_key("call_1")
        );
        let mut duplicate = output;
        let call = duplicate[1].clone();
        duplicate.as_array_mut().unwrap().push(call);
        let mut stream = ResponseStream::default();
        stream.event(completed(duplicate), 0).unwrap();
        assert!(stream.finish(1).is_err());
    }

    #[test]
    fn done_marker_and_partial_output_do_not_mask_incomplete_or_usage_failure() {
        let mut stream = ResponseStream::default();
        stream.push(b"data: [DONE]\n\n", 0).unwrap();
        assert!(stream.finish(1).is_err());
        let mut stream = ResponseStream::default();
        stream.event(json!({"type":"response.output_item.done","output_index":0,"item":message("partial")}),0).unwrap();
        assert!(stream.finish(1).is_err());
        for kind in ["response.failed", "response.incomplete", "error"] {
            let mut stream = ResponseStream::default();
            assert!(stream.event(json!({"type":kind,"response":{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"sensitive-server-detail"}}}),0).unwrap_err().contains("额度"));
        }
        let mut stream = ResponseStream::default();
        assert!(stream.event(json!({"type":"response.completed","response":{"status":"incomplete","output":[message("partial")]}}),0).is_err());
    }

    #[test]
    fn account_models_keep_server_order_and_exclude_hidden_models() {
        let catalog=parse_models(&json!({"models":[{"slug":"z-model","visibility":"list","context_window":128000},{"slug":"hidden","visibility":"hide"},{"slug":"a-model","visibility":"list"},{"slug":"z-model","visibility":"list"}]})).unwrap();
        assert_eq!(catalog.models, ["z-model", "a-model"]);
        assert_eq!(catalog.context_windows["z-model"], 128000);
        assert!(parse_models(&json!({"models":[]})).is_err());
    }

    async fn fixture(status: &str, data: String) -> reqwest::Response {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let _ = stream.read(&mut request).await.unwrap();
            let body = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}",
                data.len()
            );
            for chunk in body.as_bytes().chunks(7) {
                stream.write_all(chunk).await.unwrap();
            }
        });
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn http_sse_success_interruption_and_non_success_are_distinct() {
        let config = Config::default();
        let response = fixture("200 OK", event(completed(json!([message("done")])))).await;
        let reply = receive_response(&config, "m", response, None, &[], std::time::Instant::now())
            .await
            .unwrap();
        assert_eq!(reply.text, "done");
        let response=fixture("200 OK",event(json!({"type":"response.output_item.done","output_index":0,"item":message("partial")}))).await;
        assert!(
            receive_response(&config, "m", response, None, &[], std::time::Instant::now())
                .await
                .is_err()
        );
        let response = fixture("429 Too Many Requests", "server-private-message".into()).await;
        let error = receive_response(&config, "m", response, None, &[], std::time::Instant::now())
            .await
            .unwrap_err();
        assert!(error.contains("429"));
        assert!(!error.contains("private"));
    }

    #[test]
    fn legacy_codex_configuration_still_loads_without_serializing_a_cli_path() {
        let mut value = serde_json::to_value(Config::default()).unwrap();
        value["kind"] = json!("ChatGpt");
        value["codex_path"] = json!("/removed/codex");
        let config: Config = serde_json::from_value(value).unwrap();
        assert!(config.validate().is_ok());
        assert!(
            serde_json::to_value(config)
                .unwrap()
                .get("codex_path")
                .is_none()
        );
    }
}
