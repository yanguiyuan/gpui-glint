use crate::model::{Message, Role};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, sync::OnceLock, time::Duration};
use uuid::Uuid;

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Compatible,
    ChatGpt,
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Protocol {
    #[default]
    OpenAi,
    Anthropic,
}

/// Requested reasoning effort for providers that expose a thinking switch
/// (DeepSeek documents `thinking` plus `output_config.effort`). `Default`
/// sends nothing and keeps the provider's own behavior.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Thinking {
    #[default]
    Default,
    Off,
    Low,
    High,
    Max,
}

impl Thinking {
    pub fn from_value(value: &str) -> Self {
        match value {
            "off" => Self::Off,
            "low" => Self::Low,
            "high" => Self::High,
            "max" => Self::Max,
            _ => Self::Default,
        }
    }

    /// Injects the protocol-specific thinking fields into a request body.
    fn apply(self, body: &mut Value, protocol: Protocol) {
        match protocol {
            Protocol::Anthropic => match self {
                Self::Default => {}
                Self::Off => body["thinking"] = json!({"type": "disabled"}),
                Self::Low | Self::High | Self::Max => {
                    body["thinking"] = json!({"type": "enabled"});
                    // Reasoning consumes the token budget; raise the ceiling.
                    body["max_tokens"] = json!(32768);
                    body["output_config"] = json!({"effort": match self {
                        Self::Low => "low",
                        Self::High => "high",
                        _ => "max",
                    }});
                }
            },
            Protocol::OpenAi => match self {
                Self::Default => {}
                Self::Off => {
                    body["thinking"] = json!({"type": "disabled", "reasoning_effort": "none"})
                }
                Self::Low | Self::High | Self::Max => {
                    body["thinking"] = json!({"type": "enabled", "reasoning_effort": match self {
                        Self::Low => "low",
                        Self::High => "high",
                        _ => "max",
                    }});
                }
            },
        }
    }
}

/// Non-secret provider configuration. API keys live in the OS credential store.
#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub models_url: Option<String>,
    #[serde(default)]
    pub protocol: Protocol,
    #[serde(default)]
    pub discovery_id: Option<String>,
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub model_context_windows: std::collections::HashMap<String, u64>,
    #[serde(default)]
    pub context_window_overrides: std::collections::HashMap<String, u64>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default = "default_codex")]
    pub codex_path: String,
    pub enabled: bool,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub credential_id: Uuid,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            models_url: None,
            protocol: Protocol::OpenAi,
            discovery_id: None,
            id: Uuid::new_v4(),
            models: vec![],
            model_context_windows: Default::default(),
            context_window_overrides: Default::default(),
            api_key_env: None,
            kind: Kind::Compatible,
            codex_path: default_codex(),
            enabled: false,
            name: "OpenAI 兼容服务".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: String::new(),
            credential_id: Uuid::new_v4(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let mut value: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| "Provider 配置损坏，请检查 provider.json")?;
                Self::migrate(&mut value);
                serde_json::from_value(value)
                    .map_err(|_| "Provider 配置损坏，请检查 provider.json".into())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(crate::discovery::DiscoveryRegistry::default()
                    .discover()
                    .into_iter()
                    .next()
                    .map(|found| found.config)
                    .unwrap_or_default())
            }
            Err(_) => Err("无法读取 Provider 配置".into()),
        }
    }

    #[cfg(test)]
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| "无法创建配置目录")?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(self).map_err(|_| "无法编码 Provider 配置")?,
        )
        .map_err(|_| "无法写入 Provider 配置")?;
        std::fs::rename(temporary, path).map_err(|_| "无法保存 Provider 配置".into())
    }

    pub fn endpoint(&self) -> Result<Url, String> {
        if self.protocol == Protocol::Anthropic {
            let base = Url::parse(&self.base_url).map_err(|_| "请输入完整的 Base URL")?;
            self.endpoint_for(if base.path().trim_end_matches('/').ends_with("/v1") {
                "messages"
            } else {
                "v1/messages"
            })
        } else {
            self.endpoint_for("chat/completions")
        }
    }

    pub fn migrate(value: &mut Value) {
        if value.get("protocol").is_none()
            && matches!(
                value["base_url"].as_str(),
                Some("https://api.deepseek.com" | "https://api.deepseek.com/v1")
            )
        {
            value["protocol"] = json!("Anthropic");
            value["base_url"] = json!("https://api.deepseek.com/anthropic");
            value["models_url"] = json!("https://api.deepseek.com/models");
            value["discovery_id"] = json!("deepseek");
        }
    }

    fn models_endpoint(&self) -> Result<Url, String> {
        if let Some(address) = &self.models_url {
            let url = Url::parse(address).map_err(|_| "模型列表地址无效")?;
            if url.origin() != self.endpoint()?.origin()
                || url.query().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err("模型列表地址必须与 Provider 同源且不能包含凭据".into());
            }
            Ok(url)
        } else if self.protocol == Protocol::Anthropic {
            let url = Url::parse(&self.base_url).map_err(|_| "Provider 地址无效")?;
            self.endpoint_for(if url.path().trim_end_matches('/').ends_with("/v1") {
                "models"
            } else {
                "v1/models"
            })
        } else {
            self.endpoint_for("models")
        }
    }

    fn endpoint_for(&self, suffix: &str) -> Result<Url, String> {
        let mut url = Url::parse(self.base_url.trim()).map_err(|_| "请输入完整的 Base URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Base URL 必须为 HTTP(S) 地址，且不能包含凭据、查询参数或片段".into());
        }
        let path = format!("{}/{suffix}", url.path().trim_end_matches('/'));
        url.set_path(&path);
        Ok(url)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.kind == Kind::ChatGpt {
            return if self.codex_path.trim().is_empty() {
                Err("请填写 Codex CLI 路径".into())
            } else {
                Ok(())
            };
        }
        self.endpoint()?;
        if self.name.trim().is_empty() {
            return Err("请输入 Provider 名称".into());
        }
        if self.model.trim().is_empty() {
            return Err("请输入服务支持的模型 ID".into());
        }
        Ok(())
    }

    fn credential(&self) -> Result<keyring::Entry, String> {
        keyring::Entry::new("dev.glint.agent.provider", &self.credential_id.to_string())
            .map_err(|_| "无法打开系统凭据库".into())
    }

    pub fn key(&self) -> Result<String, String> {
        if let Some(name) = &self.api_key_env {
            return std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    format!("环境变量 {name} 未设置或为空，请从已配置该变量的终端启动应用")
                });
        }
        match self.credential()?.get_password() {
            Ok(key) => Ok(key),
            Err(keyring::Error::NoEntry) => Ok(String::new()),
            Err(_) => Err("无法读取 API Key，请解锁或允许访问系统凭据库".into()),
        }
    }

    pub fn save_key(&self, key: &str) -> Result<(), String> {
        self.credential()?
            .set_password(key)
            .map_err(|_| "无法保存 API Key 到系统凭据库".into())
    }

    pub fn remove_key(&self) -> Result<(), String> {
        match self.credential()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("无法清除已保存的 API Key".into()),
        }
    }
}

pub struct Job<T>(tokio::task::JoinHandle<Result<T, String>>);

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T: Send + 'static> Job<T> {
    pub fn start(
        future: impl std::future::Future<Output = Result<T, String>> + Send + 'static,
    ) -> Result<Self, String> {
        Ok(Self(crate::provider::runtime()?.spawn(future)))
    }
    pub async fn result(mut self) -> Result<T, String> {
        (&mut self.0)
            .await
            .map_err(|_| "后台操作已中断".to_owned())?
    }
}

#[derive(Default)]
pub struct ModelCatalog {
    pub models: Vec<String>,
    pub context_windows: std::collections::HashMap<String, u64>,
}

#[cfg(test)]
pub async fn list_models(config: Config, key: String) -> Result<Vec<String>, String> {
    Ok(list_model_catalog(config, key).await?.models)
}

pub async fn list_model_catalog(config: Config, key: String) -> Result<ModelCatalog, String> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法初始化模型发现客户端")?;
    let mut request = client.get(config.models_endpoint()?);
    if !key.is_empty() {
        request = if config.protocol == Protocol::Anthropic && config.models_url.is_none() {
            let mut value =
                reqwest::header::HeaderValue::from_str(&key).map_err(|_| "API Key 格式无效")?;
            value.set_sensitive(true);
            request
                .header("x-api-key", value)
                .header("anthropic-version", "2023-06-01")
        } else {
            request.bearer_auth(key)
        };
    }
    let response = request
        .send()
        .await
        .map_err(|_| "无法获取模型列表，请检查服务地址、认证与网络")?;
    let data = response_json(response).await?;
    let context_windows = data["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item["id"].as_str()?;
            let capacity = item["context_window"]
                .as_u64()
                .or(item["context_length"].as_u64())
                .or(item["max_context_tokens"].as_u64())?;
            (capacity > 0).then(|| (name.to_owned(), capacity))
        })
        .collect();
    let mut models: Vec<String> = data["data"]
        .as_array()
        .ok_or("服务未返回模型列表")?
        .iter()
        .filter_map(|item| item["id"].as_str().map(str::to_owned))
        .take(1000)
        .collect();
    models.sort();
    models.dedup();
    if models.is_empty() {
        return Err("服务没有返回可用模型".into());
    }
    Ok(ModelCatalog {
        models,
        context_windows,
    })
}

pub fn messages(messages: &[Message], protocol: Protocol) -> Vec<Value> {
    messages
        .iter()
        .filter(|message| message.role != Role::Notice)
        .flat_map(|message| {
            let content = if message.role == Role::User {
                user_content(&message.text, &message.images, protocol)
            } else {
                json!(message.text)
            };
            let mut result = vec![json!({
                "role": if message.role == Role::User { "user" } else { "assistant" },
                "content": content
            })];
            if let Some(context) = message
                .memory_recall
                .as_ref()
                .and_then(|recall| recall.context())
            {
                result.push(json!({"role":"system","content":context}));
            }
            result
        })
        .collect()
}

/// Builds a user turn's `content` value: a plain string when there are no
/// images, otherwise a typed part array with the images inlined as base64
/// (`image_url` data URLs for OpenAI-compatible services, `image` blocks for
/// Anthropic-compatible ones such as DeepSeek's Anthropic endpoint).
pub fn user_content(
    text: &str,
    images: &[crate::model::MessageImage],
    protocol: Protocol,
) -> Value {
    if images.is_empty() {
        return json!(text);
    }
    let mut parts = Vec::new();
    if !text.trim().is_empty() {
        parts.push(json!({"type": "text", "text": text}));
    }
    for image in images {
        parts.push(match protocol {
            Protocol::Anthropic => json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": image.media_type,
                    "data": image.data,
                }
            }),
            Protocol::OpenAi => json!({
                "type": "image_url",
                "image_url": {"url": format!("data:{};base64,{}", image.media_type, image.data)}
            }),
        });
    }
    json!(parts)
}

static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();

pub fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|_| "无法启动 Provider 请求执行器".into())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Aborts the underlying HTTP task when the owning UI task is dropped.
pub struct Request(tokio::task::JoinHandle<Result<crate::generation::Completion, String>>);

impl Drop for Request {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Request {
    pub fn start(
        config: Config,
        key: String,
        messages: Vec<Value>,
        thinking: Thinking,
    ) -> Result<Self, String> {
        Self::start_with_tools(config, key, messages, thinking, None)
    }

    pub fn start_with_tools(
        config: Config,
        key: String,
        mut messages: Vec<Value>,
        thinking: Thinking,
        mut context: Option<crate::tools::ToolContext>,
    ) -> Result<Self, String> {
        config.validate()?;
        if let Some(context) = &mut context {
            if !matches!(context.delegation, crate::subagent::Delegation::Child) {
                context.delegation = crate::subagent::Delegation::Root(
                    crate::subagent::Runner::new(config.clone(), key.clone(), thinking, &messages),
                );
            }
        }
        Ok(Self(runtime()?.spawn(async move {
            if let Some(context) = &context {
                let plan = context.todos.lock().ok().and_then(|todos| {
                    (!todos.is_empty()).then(|| serde_json::json!({"todos": &*todos}).to_string())
                });
                if let Some(plan) = plan {
                    messages.insert(0, json!({"role":"system","content":format!("Current execution checklist (historical progress; adjust it to the latest user task using todo_write, or todo_read to inspect it). Only mark work completed after doing and verifying it. {plan}")}));
                }
                messages.insert(0, json!({"role":"system","content":context.registry.instructions()}));
                if let Err(error) = context.registry.remote.prepare(context.root.as_deref()).await {
                    messages.insert(0, json!({"role":"system","content":format!("本次外部 MCP 连接失败，外部 MCP 工具不可用，可通过配置工具修复；已注册的内置工作台工具仍可使用：{error}")}));
                }
            }
            if config.kind == Kind::ChatGpt {
                // Codex CLI manages its own reasoning configuration.
                tokio::time::timeout(
                    Duration::from_secs(180),
                    crate::chatgpt::complete(&config, messages, context),
                )
                .await
                .map_err(|_| "ChatGPT 请求超时，请稍后重试".to_owned())?
            } else {
                complete(config, key, messages, thinking, context).await
            }
        })))
    }

    pub async fn response(mut self) -> Result<crate::generation::Completion, String> {
        (&mut self.0)
            .await
            .map_err(|_| "Provider 请求已中断".to_owned())?
    }
}

fn default_codex() -> String {
    // Finder-launched applications often have a minimal PATH.
    if let Some(home) = std::env::var_os("HOME") {
        for relative in [".bun/bin/codex", ".cargo/bin/codex", ".local/bin/codex"] {
            let path = std::path::PathBuf::from(&home).join(relative);
            if path.is_file() {
                return path.to_string_lossy().into();
            }
        }
    }
    for path in ["/opt/homebrew/bin/codex", "/usr/local/bin/codex"] {
        if Path::new(path).is_file() {
            return path.into();
        }
    }
    "codex".into()
}

async fn complete(
    config: Config,
    key: String,
    mut messages: Vec<Value>,
    thinking: Thinking,
    context: Option<crate::tools::ToolContext>,
) -> Result<crate::generation::Completion, String> {
    let started = std::time::Instant::now();
    let mut records = Vec::new();
    let mut blocks = Vec::new();
    let mut total: Option<crate::generation::GenerationStats> = None;
    let mut argument_repairs = 0;
    loop {
        // Never wait for recall. Consume it once at the next request boundary,
        // after all preceding tool calls have their matching results.
        if let Some(context) = &context {
            messages.extend(
                context
                    .take_steering()
                    .into_iter()
                    .map(|text| json!({"role":"user","content":text})),
            );
        }
        if let Some(recalled) = context
            .as_ref()
            .and_then(|context| context.take_recalled_context())
        {
            messages.push(json!({"role":"system","content":recalled}));
        }
        let mut reply = single_request(
            config.clone(),
            key.clone(),
            messages.clone(),
            thinking,
            context.as_ref(),
            &blocks,
        )
        .await?;
        if let Some(context) = &context {
            let _ = context
                .events
                .try_send(crate::tools::ToolEvent::ContextUsage {
                    provider_id: config.id,
                    model: config.model.clone(),
                    used_tokens: reply.stats.context_tokens,
                    capacity_tokens: reply.stats.context_window,
                });
        }
        blocks.extend(reply.blocks.clone());
        if let Some(context) = &context {
            let _ = context
                .events
                .try_send(crate::tools::ToolEvent::Transcript(blocks.clone()));
        }
        if let Some(stats) = &mut total {
            stats.context_tokens = reply.stats.context_tokens;
            stats.context_window = reply.stats.context_window.or(stats.context_window);
            stats.input_tokens = stats
                .input_tokens
                .zip(reply.stats.input_tokens)
                .map(|(a, b)| a + b);
            stats.output_tokens = stats
                .output_tokens
                .zip(reply.stats.output_tokens)
                .map(|(a, b)| a + b);
            stats.generation_ms = stats
                .generation_ms
                .zip(reply.stats.generation_ms)
                .map(|(a, b)| a + b);
        } else {
            total = Some(reply.stats.clone());
        }
        if reply.tool_calls.is_empty() {
            let steering = context
                .as_ref()
                .map(|context| context.take_steering())
                .unwrap_or_default();
            if !steering.is_empty() {
                if config.protocol == Protocol::Anthropic {
                    messages.push(json!({"role":"assistant","content":reply.raw_content}));
                } else {
                    messages.push(json!({"role":"assistant","content":reply.text,"reasoning_content":reply.reasoning}));
                }
                messages.extend(
                    steering
                        .into_iter()
                        .map(|text| json!({"role":"user","content":text})),
                );
                continue;
            }
            reply.stats = total.unwrap();
            reply.stats.elapsed_ms = started.elapsed().as_millis() as u64;
            reply.tool_records = records;
            reply.text = blocks
                .iter()
                .filter_map(|block| match block {
                    crate::generation::ContentBlock::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            reply.blocks = blocks;
            return Ok(reply);
        }
        let context = context
            .as_ref()
            .ok_or("Provider 请求调用工具，但当前未启用工具")?;
        if reply.tool_calls.len() > 16 {
            return Err("单轮工具调用数量超过限制".into());
        }
        if config.protocol == Protocol::Anthropic {
            messages.push(json!({"role":"assistant","content":reply.raw_content}));
        } else {
            let mut assistant = json!({"role":"assistant","content":if reply.text.is_empty() { Value::Null } else { Value::String(reply.text) },
                "tool_calls":reply.tool_calls.iter().map(|call| json!({"id":call.id,"type":"function","function":{"name":call.name,"arguments":call.arguments.to_string()}})).collect::<Vec<_>>()});
            if !reply.reasoning.is_empty() {
                assistant["reasoning_content"] = reply.reasoning.into();
            }
            messages.push(assistant);
        }
        let mut anthropic_results = Vec::new();
        if !reply.argument_errors.is_empty() {
            argument_repairs += 1;
        }
        let repair_limit = argument_repairs > 2;
        for call in reply.tool_calls {
            let rejected = repair_limit || reply.argument_errors.contains_key(&call.id);
            let record = if repair_limit {
                crate::tools::rejected(
                    call,
                    "未执行：工具参数连续修正失败，已停止本轮调用。此前成功的操作不会重复执行。"
                        .into(),
                )
            } else if let Some(error) = reply.argument_errors.remove(&call.id) {
                crate::tools::rejected(call, error)
            } else {
                crate::tools::execute(call, context).await
            };
            if rejected {
                let _ = context
                    .events
                    .try_send(crate::tools::ToolEvent::Completed(record.clone()));
            }
            if config.protocol == Protocol::Anthropic {
                anthropic_results.push(json!({"type":"tool_result","tool_use_id":record.call.id,"content":record.result,"is_error":!record.success}));
            } else {
                messages.push(
                    json!({"role":"tool","tool_call_id":record.call.id,"content":record.result}),
                );
            }
            blocks.push(crate::generation::ContentBlock::ToolResult(record.clone()));
            let _ = context
                .events
                .try_send(crate::tools::ToolEvent::Transcript(blocks.clone()));
            records.push(record);
        }
        if !anthropic_results.is_empty() {
            messages.push(json!({"role":"user","content":anthropic_results}));
        }
        if repair_limit {
            return Err("模型连续返回无效工具参数，自动修正两次后仍未成功；已停止，不会重放此前完成的操作。".into());
        }
    }
}

async fn single_request(
    config: Config,
    key: String,
    messages: Vec<Value>,
    thinking: Thinking,
    context: Option<&crate::tools::ToolContext>,
    prefix: &[crate::generation::ContentBlock],
) -> Result<crate::generation::Completion, String> {
    let started = std::time::Instant::now();
    let endpoint = config.endpoint()?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法初始化 Provider 客户端")?;
    let mut body = if config.protocol == Protocol::Anthropic {
        let system: Vec<_> = messages
            .iter()
            .filter(|message| message["role"] == "system")
            .filter_map(|message| message["content"].as_str())
            .collect();
        let dialogue: Vec<_> = messages
            .iter()
            .filter(|message| message["role"] != "system")
            .cloned()
            .collect();
        let mut body =
            json!({"model":config.model,"messages":dialogue,"max_tokens":8192,"stream":true});
        // DeepSeek's default reasoning can consume the entire old 8K budget
        // before producing an answer. Match the budget already used for High.
        if config.model.starts_with("deepseek") && thinking != Thinking::Off {
            body["max_tokens"] = json!(32768);
        }
        if !system.is_empty() {
            body["system"] = system.join("\n\n").into();
        }
        body
    } else {
        json!({"model":config.model,"messages":messages,"stream":true,"stream_options":{"include_usage":true}})
    };
    if let Some(context) = context {
        body["tools"] = context.definitions(config.protocol == Protocol::Anthropic);
    }
    thinking.apply(&mut body, config.protocol);
    let mut request = client.post(endpoint).json(&body);
    if config.protocol == Protocol::Anthropic {
        request = request.header("anthropic-version", "2023-06-01");
    }
    if !key.is_empty() {
        request = if config.protocol == Protocol::Anthropic {
            let mut value =
                reqwest::header::HeaderValue::from_str(&key).map_err(|_| "API Key 格式无效")?;
            value.set_sensitive(true);
            request.header("x-api-key", value)
        } else {
            request.bearer_auth(&key)
        };
    }
    let mut response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            "Provider 请求超时，请稍后重试".to_owned()
        } else if error.is_connect() {
            "无法连接 Provider，请检查地址与网络".to_owned()
        } else {
            "Provider 网络请求失败".to_owned()
        }
    })?;
    if response.status().is_success()
        && response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"))
    {
        let mut stream = crate::generation::StreamCompletion::default();
        let mut bytes = 0;
        let mut last_context = None;
        let mut last_snapshot = std::time::Instant::now() - Duration::from_millis(50);
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "读取 Provider 响应流失败")?
        {
            bytes += chunk.len();
            if bytes > 4 * 1024 * 1024 {
                return Err("Provider 响应超过 4MB 限制".into());
            }
            stream.push(&chunk, started.elapsed().as_millis() as u64)?;
            if let Some(context) = context {
                let used = stream.context_tokens();
                if used.is_some() && used != last_context {
                    last_context = used;
                    let _ = context
                        .events
                        .try_send(crate::tools::ToolEvent::ContextUsage {
                            provider_id: config.id,
                            model: config.model.clone(),
                            used_tokens: used,
                            capacity_tokens: None,
                        });
                }
                if stream.ended || last_snapshot.elapsed() >= Duration::from_millis(50) {
                    let mut blocks = prefix.to_vec();
                    blocks.extend(stream.snapshot());
                    let _ = context
                        .events
                        .try_send(crate::tools::ToolEvent::Transcript(blocks));
                    last_snapshot = std::time::Instant::now();
                }
            }
            if stream.ended {
                break;
            }
        }
        if let Some(context) = context {
            let mut blocks = prefix.to_vec();
            blocks.extend(stream.snapshot());
            let _ = context
                .events
                .try_send(crate::tools::ToolEvent::Transcript(blocks));
        }
        return stream.finish(started.elapsed().as_millis() as u64);
    }
    let parsed = response_json(response).await?;
    if let Some(context) = context {
        let mut received = prefix.to_vec();
        if config.protocol == Protocol::Anthropic {
            received.extend(crate::generation::anthropic_blocks(&parsed["content"]));
        } else {
            let message = &parsed["choices"][0]["message"];
            if let Some(text) = message["reasoning_content"]
                .as_str()
                .filter(|text| !text.is_empty())
            {
                received.push(crate::generation::ContentBlock::Thinking(text.into()));
            }
            if let Some(text) = message["content"].as_str().filter(|text| !text.is_empty()) {
                received.push(crate::generation::ContentBlock::Text(text.into()));
            }
        }
        let _ = context
            .events
            .try_send(crate::tools::ToolEvent::Transcript(received));
    }
    let stop = if config.protocol == Protocol::Anthropic {
        parsed["stop_reason"].as_str()
    } else {
        parsed["choices"][0]["finish_reason"].as_str()
    };
    if let Some(error) = crate::generation::stop_error(stop) {
        return Err(error);
    }
    let mut metrics = crate::generation::Metrics::default();
    metrics.usage(&parsed["usage"]);
    let stats = metrics.finish(started.elapsed().as_millis() as u64);
    if config.protocol == Protocol::Anthropic {
        let text = parsed["content"]
            .as_array()
            .ok_or("Provider 未返回 Anthropic 文本内容")?
            .iter()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let decoded = crate::tools::decode_calls(&parsed["content"], true)?;
        let tool_calls = decoded.calls;
        let mut raw_content = parsed["content"].clone();
        if let Some(parts) = raw_content.as_array_mut() {
            for part in parts {
                if part["id"]
                    .as_str()
                    .is_some_and(|id| decoded.errors.contains_key(id))
                {
                    part["input"] = json!({});
                }
            }
        }
        return if text.trim().is_empty() && tool_calls.is_empty() {
            Err(crate::generation::empty_reply_error(
                stop,
                parsed["content"]
                    .as_array()
                    .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "thinking")),
            ))
        } else {
            Ok(crate::generation::Completion {
                text,
                stats,
                tool_calls,
                blocks: crate::generation::anthropic_blocks(&raw_content),
                argument_errors: decoded.errors,
                raw_content,
                ..Default::default()
            })
        };
    }
    let message = &parsed["choices"][0]["message"];
    let text = message["content"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            message["content"].as_array().map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        })
        .or_else(|| message["refusal"].as_str().map(str::to_owned));
    let text = text.unwrap_or_default();
    let decoded = crate::tools::decode_calls(message, false)?;
    let tool_calls = decoded.calls;
    if text.trim().is_empty() && tool_calls.is_empty() {
        return Err(crate::generation::empty_reply_error(
            stop,
            message["reasoning_content"]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
        ));
    }
    let mut blocks = Vec::new();
    if let Some(reasoning) = message["reasoning_content"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        blocks.push(crate::generation::ContentBlock::Thinking(reasoning.into()));
    }
    if !text.is_empty() {
        blocks.push(crate::generation::ContentBlock::Text(text.clone()));
    }
    blocks.extend(
        tool_calls
            .iter()
            .cloned()
            .map(crate::generation::ContentBlock::ToolCall),
    );
    Ok(crate::generation::Completion {
        blocks,
        text,
        stats,
        tool_calls,
        argument_errors: decoded.errors,
        reasoning: message["reasoning_content"].as_str().unwrap_or("").into(),
        ..Default::default()
    })
}

async fn response_json(mut response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    // Bound the response body, including chunked responses without Content-Length.
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "读取 Provider 响应失败")?
    {
        if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
            return Err("Provider 响应超过 4MB 限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !status.is_success() {
        let explanation = match status.as_u16() {
            401 | 403 => "认证失败，请检查 API Key 与访问权限",
            404 => "接口或模型不存在，请检查 Base URL 和模型 ID",
            429 => "请求受限，请检查额度或稍后重试",
            _ => "服务返回错误，请检查 Provider 状态",
        };
        return Err(format!("HTTP {}：{explanation}", status.as_u16()));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn malformed_streamed_calls_are_repaired_without_replaying_successful_tools_and_retries_are_bounded()
     {
        for anthropic in [false, true] {
            for keep_failing in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                listener.set_nonblocking(true).unwrap();
                let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
                let server = std::thread::spawn(move || {
                    let mut requests = Vec::new();
                    for round in 0..3 {
                        let deadline = std::time::Instant::now() + Duration::from_secs(10);
                        let mut socket = loop {
                            match listener.accept() {
                                Ok((socket, _)) => break socket,
                                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                    assert!(
                                        std::time::Instant::now() < deadline,
                                        "missing retry request"
                                    );
                                    std::thread::sleep(Duration::from_millis(10));
                                }
                                Err(e) => panic!("{e}"),
                            }
                        };
                        // macOS inherits O_NONBLOCK on sockets accepted from this listener.
                        socket.set_nonblocking(false).unwrap();
                        socket
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let mut bytes = Vec::new();
                        let offset = loop {
                            let mut byte = [0];
                            socket.read_exact(&mut byte).unwrap();
                            bytes.push(byte[0]);
                            if bytes.ends_with(b"\r\n\r\n") {
                                break bytes.len();
                            }
                        };
                        let headers = String::from_utf8_lossy(&bytes).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .unwrap()
                            .trim()
                            .parse()
                            .unwrap();
                        bytes.resize(offset + length, 0);
                        socket.read_exact(&mut bytes[offset..]).unwrap();
                        requests.push(serde_json::from_slice::<Value>(&bytes[offset..]).unwrap());
                        let mut calls = Vec::new();
                        if round == 0 {
                            calls.push(("good".to_owned(),"todo_write",json!({"todos":[{"content":"keep previous work","status":"completed"}]}).to_string()));
                        }
                        if keep_failing || round == 0 {
                            calls.push((
                                format!("bad-{round}"),
                                "run_command",
                                "{\"program\":\"sh\"".into(),
                            ));
                        } else if round == 1 {
                            calls.push(("corrected".into(), "todo_read", "{}".into()));
                        }
                        let mut events = Vec::new();
                        if anthropic {
                            for (index, (id, name, args)) in calls.iter().enumerate() {
                                events.push(json!({"type":"content_block_start","index":index,"content_block":{"type":"tool_use","id":id,"name":name,"input":{}}}));
                                events.push(json!({"type":"content_block_delta","index":index,"delta":{"partial_json":args}}));
                            }
                            if calls.is_empty() {
                                events.push(json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}));
                            }
                            if calls.is_empty() {
                                events.push(json!({"type":"content_block_delta","index":0,"delta":{"text":"done"}}));
                            }
                            events.push(json!({"type":"message_delta","delta":{"stop_reason":if calls.is_empty(){"end_turn"}else{"tool_use"}}}));
                            events.push(json!({"type":"message_stop"}));
                        } else {
                            let delta = if calls.is_empty() {
                                json!({"content":"done"})
                            } else {
                                json!({"tool_calls":calls.iter().enumerate().map(|(index,(id,name,args))|json!({"index":index,"id":id,"type":"function","function":{"name":name,"arguments":args}})).collect::<Vec<_>>()})
                            };
                            events.push(json!({"choices":[{"delta":delta,"finish_reason":if calls.is_empty(){"stop"}else{"tool_calls"}}]}));
                        }
                        let body = events
                            .iter()
                            .map(|event| format!("data: {event}\n\n"))
                            .collect::<String>();
                        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                    }
                    requests
                });
                let config = Config {
                    base_url,
                    model: "test-model".into(),
                    protocol: if anthropic {
                        Protocol::Anthropic
                    } else {
                        Protocol::OpenAi
                    },
                    ..Default::default()
                };
                let (events, receiver) = async_channel::unbounded();
                let context = crate::tools::ToolContext {
                    delegation: Default::default(),
                    subagents: Default::default(),
                    todos: Default::default(),
                    steering: None,
                    registry: Default::default(),
                    skills: std::sync::Arc::new(crate::skills::Catalog::empty(
                        directory.path().join("sessions.json"),
                    )),
                    memory: crate::memory::Repository::new(directory.path().join("sessions.json")),
                    access: std::sync::Arc::new(std::sync::Mutex::new(
                        crate::tools::AccessMode::Ask,
                    )),
                    root: Some(directory.path().into()),
                    schedules: std::sync::Arc::new(std::sync::Mutex::new(
                        crate::schedules::ScheduleStore::open(
                            directory.path().join("schedules.json"),
                        )
                        .unwrap(),
                    )),
                    provider_id: config.id,
                    model: config.model.clone(),
                    events,
                    recall: None,
                };
                let result = runtime().unwrap().block_on(complete(
                    config,
                    "test-only-key".into(),
                    vec![json!({"role":"user","content":"test"})],
                    Thinking::Off,
                    Some(context.clone()),
                ));
                if keep_failing {
                    assert!(result.unwrap_err().contains("自动修正两次"));
                } else {
                    let reply = result.unwrap();
                    assert_eq!(reply.text, "done");
                    assert_eq!(reply.tool_records.len(), 3);
                    assert_eq!(reply.tool_records.iter().filter(|r| r.success).count(), 2);
                }
                let mut started = Vec::new();
                let mut rejected = 0;
                while let Ok(event) = receiver.try_recv() {
                    match event {
                        crate::tools::ToolEvent::Started(call) => started.push(call.id),
                        crate::tools::ToolEvent::Completed(record)
                            if record.call.id.starts_with("bad-") =>
                        {
                            assert!(!record.success);
                            assert!(record.result.starts_with("未执行："));
                            rejected += 1;
                        }
                        crate::tools::ToolEvent::Approval { .. } => {
                            panic!("invalid tool reached approval")
                        }
                        _ => {}
                    }
                }
                assert_eq!(
                    started,
                    if keep_failing {
                        vec!["good"]
                    } else {
                        vec!["good", "corrected"]
                    }
                );
                assert_eq!(rejected, if keep_failing { 3 } else { 1 });
                assert_eq!(context.todos.lock().unwrap().len(), 1);
                let requests = server.join().unwrap();
                assert_eq!(requests.len(), 3);
                let messages = requests[1]["messages"].as_array().unwrap();
                if anthropic {
                    let results = messages.last().unwrap()["content"].as_array().unwrap();
                    assert_eq!(results[0]["tool_use_id"], "good");
                    assert_eq!(results[0]["is_error"], false);
                    assert_eq!(results[1]["tool_use_id"], "bad-0");
                    assert_eq!(results[1]["is_error"], true);
                } else {
                    let results: Vec<_> = messages.iter().filter(|m| m["role"] == "tool").collect();
                    assert_eq!(results.len(), 2);
                    assert_eq!(results[0]["tool_call_id"], "good");
                    assert_eq!(results[1]["tool_call_id"], "bad-0");
                    assert!(
                        results[1]["content"]
                            .as_str()
                            .unwrap()
                            .starts_with("未执行：")
                    );
                }
            }
        }
    }

    #[test]
    fn model_catalog_keeps_real_window_metadata_and_manual_limits_round_trip() {
        let (url, server) = server_for(
            "get /v1/models ",
            "200 OK",
            r#"{"data":[{"id":"a","context_window":1048576},{"id":"b","context_length":128000},{"id":"unknown"}]}"#,
        );
        let mut config = Config {
            base_url: url,
            ..Default::default()
        };
        let models = runtime()
            .unwrap()
            .block_on(list_model_catalog(config.clone(), "test-only-key".into()))
            .unwrap();
        assert_eq!(models.context_windows["a"], 1048576);
        assert_eq!(models.context_windows["b"], 128000);
        assert!(!models.context_windows.contains_key("unknown"));
        config.model_context_windows = models.context_windows;
        config.context_window_overrides.insert("a".into(), 500000);
        let loaded: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(loaded.context_window_overrides["a"], 500000);
        assert_eq!(loaded.model_context_windows["a"], 1048576);
        server.join().unwrap();
    }

    #[test]
    fn thinking_levels_use_the_documented_protocol_fields() {
        let mut body = json!({"max_tokens": 8192});
        Thinking::Default.apply(&mut body, Protocol::Anthropic);
        assert!(body.get("thinking").is_none());

        let mut body = json!({"max_tokens": 8192});
        Thinking::Off.apply(&mut body, Protocol::Anthropic);
        assert_eq!(body["thinking"], json!({"type": "disabled"}));
        assert_eq!(body["max_tokens"], 8192);

        let mut body = json!({"max_tokens": 8192});
        Thinking::High.apply(&mut body, Protocol::Anthropic);
        assert_eq!(body["thinking"], json!({"type": "enabled"}));
        assert_eq!(body["output_config"], json!({"effort": "high"}));
        assert_eq!(body["max_tokens"], 32768);

        let mut body = json!({});
        Thinking::Max.apply(&mut body, Protocol::OpenAi);
        assert_eq!(
            body["thinking"],
            json!({"type": "enabled", "reasoning_effort": "max"})
        );

        let mut body = json!({});
        Thinking::Off.apply(&mut body, Protocol::OpenAi);
        assert_eq!(
            body["thinking"],
            json!({"type": "disabled", "reasoning_effort": "none"})
        );

        assert_eq!(Thinking::from_value("off"), Thinking::Off);
        assert_eq!(Thinking::from_value("max"), Thinking::Max);
        assert_eq!(Thinking::from_value("unknown"), Thinking::Default);
    }

    #[test]
    fn saved_configuration_wins_over_environment_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider.json");
        let config = Config {
            enabled: false,
            name: "My provider".into(),
            ..Config::default()
        };
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.name, "My provider");
        assert!(!loaded.enabled);
        assert_eq!(loaded.credential_id, config.credential_id);
    }

    #[test]
    fn endpoint_preserves_prefix_and_rejects_embedded_credentials() {
        let mut config = Config::default();
        config.base_url = "http://localhost:11434/v1/".into();
        assert_eq!(
            config.endpoint().unwrap().as_str(),
            "http://localhost:11434/v1/chat/completions"
        );
        config.base_url = "https://user:secret@example.com/v1".into();
        assert!(config.endpoint().is_err());
    }

    fn server(status: &str, body: &'static str) -> (String, std::thread::JoinHandle<Value>) {
        server_for("post /v1/chat/completions ", status, body)
    }

    #[test]
    fn deepseek_default_reasoning_has_answer_headroom_and_budget_failures_are_specific() {
        for thinking in [Thinking::Default, Thinking::Off] {
            let (base_url, server) = server_for(
                "post /v1/messages ",
                "200 OK",
                r#"{"content":[{"type":"thinking","thinking":"reasoning only"}],"stop_reason":"max_tokens"}"#,
            );
            let config = Config {
                base_url,
                model: "deepseek-flash".into(),
                protocol: Protocol::Anthropic,
                ..Default::default()
            };
            let request = Request::start(
                config,
                "test-only-key".into(),
                vec![json!({"role":"user","content":"Create a page"})],
                thinking,
            )
            .unwrap();
            let error = runtime().unwrap().block_on(request.response()).unwrap_err();
            assert!(error.contains("token 上限"));
            let body = server.join().unwrap();
            assert_eq!(
                body["max_tokens"],
                if thinking == Thinking::Off {
                    8192
                } else {
                    32768
                }
            );
        }
    }

    fn server_for(
        expected: &'static str,
        status: &str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let status = status.to_owned();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let offset;
            loop {
                let mut buffer = [0u8; 1024];
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    offset = index + 4;
                    break;
                }
            }
            let headers = String::from_utf8_lossy(&bytes[..offset]).to_lowercase();
            assert!(headers.starts_with(expected));
            if expected.contains("messages") {
                assert!(headers.contains("x-api-key: test-only-key"));
                assert!(headers.contains("anthropic-version: 2023-06-01"));
            } else {
                assert!(headers.contains("authorization: bearer test-only-key"));
            }
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap_or("0")
                .trim()
                .parse()
                .unwrap();
            while bytes.len() < offset + length {
                let mut buffer = [0u8; 1024];
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            if length == 0 {
                Value::Null
            } else {
                serde_json::from_slice(&bytes[offset..offset + length]).unwrap()
            }
        });
        (url, handle)
    }

    #[test]
    fn both_protocols_execute_tools_and_return_matching_results_to_the_model() {
        for deliver_recall in [false, true] {
            for anthropic in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
                let (recall_sender, recall_receiver) = async_channel::bounded(1);
                let (steering_sender, steering_receiver) = async_channel::unbounded();
                let server = std::thread::spawn(move || {
                    for round in 0..2 {
                        let (mut stream, _) = listener.accept().unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let mut bytes = Vec::new();
                        let offset = loop {
                            let mut byte = [0];
                            stream.read_exact(&mut byte).unwrap();
                            bytes.push(byte[0]);
                            if bytes.ends_with(b"\r\n\r\n") {
                                break bytes.len();
                            }
                        };
                        let headers = String::from_utf8_lossy(&bytes).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap()
                            .trim()
                            .parse()
                            .unwrap();
                        bytes.resize(offset + length, 0);
                        stream.read_exact(&mut bytes[offset..]).unwrap();
                        let request: Value = serde_json::from_slice(&bytes[offset..]).unwrap();
                        assert!(!request["tools"].as_array().unwrap().is_empty());
                        let occurrences = request.to_string().matches("recall-sentinel").count();
                        assert_eq!(occurrences, usize::from(deliver_recall && round == 1));
                        assert_eq!(
                            request.to_string().matches("steering-sentinel").count(),
                            usize::from(deliver_recall && round == 1)
                        );
                        if round == 0 && deliver_recall {
                            steering_sender
                                .try_send("steering-sentinel".into())
                                .unwrap();
                            // Recall cannot start/finish before the main request:
                            // release it only once the server has seen round one.
                            recall_sender
                                .try_send(crate::memory_recall::tests::sample())
                                .ok()
                                .unwrap();
                        }
                        if round == 1 {
                            let messages = request["messages"].as_array().unwrap();
                            let result = messages
                                .iter()
                                .rev()
                                .find(|message| {
                                    if anthropic {
                                        message["content"][0]["type"] == "tool_result"
                                    } else {
                                        message["role"] == "tool"
                                    }
                                })
                                .unwrap();
                            if anthropic {
                                assert_eq!(result["content"][0]["tool_use_id"], "call-1");
                                assert_eq!(result["content"][0]["is_error"], false);
                            } else {
                                assert_eq!(result["role"], "tool");
                                assert_eq!(result["tool_call_id"], "call-1");
                            }
                        }
                        let mut body = match (anthropic, round) {
                            (true, 0) => {
                                json!({"content":[{"type":"thinking","thinking":"plan"},{"type":"text","text":"before"},{"type":"tool_use","id":"call-1","name":"schedule_list","input":{}}]})
                            }
                            (false, 0) => {
                                json!({"choices":[{"message":{"reasoning_content":"plan","content":"before","tool_calls":[{"id":"call-1","type":"function","function":{"name":"schedule_list","arguments":"{}"}}]}}]})
                            }
                            (true, _) => json!({"content":[{"type":"text","text":"done"}]}),
                            (false, _) => json!({"choices":[{"message":{"content":"done"}}]}),
                        };
                        body["usage"] = if anthropic {
                            json!({"input_tokens":100 * (round + 1),"output_tokens":10})
                        } else {
                            json!({"prompt_tokens":100 * (round + 1),"completion_tokens":10})
                        };
                        let body = body.to_string();
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    }
                });
                let config = Config {
                    base_url,
                    model: "test-model".into(),
                    protocol: if anthropic {
                        Protocol::Anthropic
                    } else {
                        Protocol::OpenAi
                    },
                    ..Config::default()
                };
                let (events, _receiver) = async_channel::unbounded();
                let context = crate::tools::ToolContext {
                    delegation: Default::default(),
                    subagents: Default::default(),
                    todos: Default::default(),
                    steering: Some(steering_receiver),
                    registry: Default::default(),
                    skills: std::sync::Arc::new(crate::skills::Catalog::empty(
                        directory.path().join("sessions.json"),
                    )),
                    memory: crate::memory::Repository::new(directory.path().join("sessions.json")),
                    access: std::sync::Arc::new(std::sync::Mutex::new(
                        crate::tools::AccessMode::Ask,
                    )),
                    root: None,
                    schedules: std::sync::Arc::new(std::sync::Mutex::new(
                        crate::schedules::ScheduleStore::open(
                            directory.path().join("schedules.json"),
                        )
                        .unwrap(),
                    )),
                    provider_id: config.id,
                    model: config.model.clone(),
                    events,
                    recall: Some(recall_receiver.clone()),
                };
                let request = Request::start_with_tools(
                    config,
                    "test-only-key".into(),
                    vec![json!({"role":"user","content":"list schedules"})],
                    Thinking::Default,
                    Some(context),
                )
                .unwrap();
                let reply = runtime().unwrap().block_on(request.response()).unwrap();
                assert_eq!(reply.text, "before\n\ndone");
                use crate::generation::ContentBlock;
                assert!(
                    matches!(&reply.blocks[..], [ContentBlock::Thinking(plan), ContentBlock::Text(before), ContentBlock::ToolCall(call), ContentBlock::ToolResult(result), ContentBlock::Text(after)] if plan == "plan" && before == "before" && call.id == "call-1" && result.success && after == "done")
                );
                assert_eq!(reply.stats.input_tokens, Some(300));
                assert_eq!(reply.stats.output_tokens, Some(20));
                assert_eq!(reply.stats.context_tokens, Some(210));
                assert_eq!(reply.tool_records.len(), 1);
                assert!(reply.tool_records[0].success);
                assert!(recall_receiver.try_recv().is_err());
                server.join().unwrap();
            }
        }
    }

    #[test]
    fn late_recall_is_included_as_historical_context_in_followup_turns() {
        let mut session = crate::model::Session::new();
        session.push(Role::User, "明天吃什么".into());
        session.push(Role::Agent, "先给出建议".into());
        session.messages[0].memory_recall = Some(crate::memory_recall::tests::sample());
        session.push(Role::User, "换一个建议".into());
        for protocol in [Protocol::OpenAi, Protocol::Anthropic] {
            let messages = messages(&session.messages, protocol);
            assert_eq!(messages.len(), 4);
            assert_eq!(messages[1]["role"], "system");
            let context = messages[1]["content"].as_str().unwrap();
            assert!(context.contains("recall-sentinel"));
            assert!(context.contains("不是当前指令"));
            assert_eq!(messages.last().unwrap()["content"], "换一个建议");
        }
    }

    #[test]
    fn real_http_sends_auth_model_and_history_and_parses_reply() {
        let (url, server) = server(
            "200 OK",
            r#"{"choices":[{"message":{"content":"真实 HTTP 回复"}}]}"#,
        );
        let config = Config {
            base_url: url,
            model: "test-model".into(),
            ..Config::default()
        };
        let request = Request::start(
            config,
            "test-only-key".into(),
            vec![json!({"role":"user","content":"你好"})],
            Thinking::Default,
        )
        .unwrap();
        assert_eq!(
            runtime()
                .unwrap()
                .block_on(request.response())
                .unwrap()
                .text,
            "真实 HTTP 回复"
        );
        let body = server.join().unwrap();
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["content"], "你好");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn anthropic_sends_correct_path_headers_system_and_reads_text_blocks() {
        let (url, server) = server_for(
            "post /anthropic/v1/messages ",
            "200 OK",
            r#"{"content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"OK"}]}"#,
        );
        let config = Config {
            protocol: Protocol::Anthropic,
            base_url: url.trim_end_matches("/v1").to_owned() + "/anthropic",
            model: "test-model".into(),
            ..Config::default()
        };
        let request = Request::start(
            config,
            "test-only-key".into(),
            vec![
                json!({"role":"system","content":"Be concise"}),
                json!({"role":"user","content":"Hello"}),
            ],
            Thinking::Default,
        )
        .unwrap();
        assert_eq!(
            runtime()
                .unwrap()
                .block_on(request.response())
                .unwrap()
                .text,
            "OK"
        );
        let body = server.join().unwrap();
        assert_eq!(body["system"], "Be concise");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["max_tokens"], 8192);
    }

    #[test]
    fn anthropic_v1_path_and_protocol_migration_preserve_explicit_choices() {
        let config = Config {
            protocol: Protocol::Anthropic,
            base_url: "https://example.com/v1/".into(),
            ..Config::default()
        };
        assert_eq!(
            config.endpoint().unwrap().as_str(),
            "https://example.com/v1/messages"
        );
        let mut legacy = json!({"base_url":"https://api.deepseek.com"});
        Config::migrate(&mut legacy);
        assert_eq!(legacy["protocol"], "Anthropic");
        assert_eq!(legacy["base_url"], "https://api.deepseek.com/anthropic");
        let mut explicit = json!({"base_url":"https://api.deepseek.com","protocol":"OpenAi"});
        Config::migrate(&mut explicit);
        assert_eq!(explicit["protocol"], "OpenAi");
    }

    #[test]
    fn models_discovery_uses_authenticated_get_and_deduplicates_ids() {
        let (url, server) = server_for(
            "get /v1/models ",
            "200 OK",
            r#"{"data":[{"id":"b"},{"id":"a"},{"id":"b"}]}"#,
        );
        let config = Config {
            base_url: url,
            ..Config::default()
        };
        let models = runtime()
            .unwrap()
            .block_on(list_models(config, "test-only-key".into()))
            .unwrap();
        assert_eq!(models, ["a", "b"]);
        server.join().unwrap();
    }

    #[test]
    fn auth_error_does_not_expose_response_secrets() {
        let (url, server) = server(
            "401 Unauthorized",
            r#"{"error":{"message":"test-only-key"}}"#,
        );
        let config = Config {
            base_url: url,
            model: "test-model".into(),
            ..Config::default()
        };
        let request =
            Request::start(config, "test-only-key".into(), vec![], Thinking::Default).unwrap();
        let error = runtime().unwrap().block_on(request.response()).unwrap_err();
        assert!(error.contains("401"));
        assert!(!error.contains("test-only-key"));
        server.join().unwrap();
    }

    #[test]
    fn notices_are_excluded_and_real_assistant_replies_are_preserved() {
        let mut session = crate::model::Session::new();
        session.push(Role::User, "你好".into());
        session.push(Role::Agent, "你好，有什么需要帮助？".into());
        session.push(Role::Notice, "已停止回复".into());
        assert_eq!(
            messages(&session.messages, Protocol::OpenAi),
            vec![
                json!({"role":"user","content":"你好"}),
                json!({"role":"assistant","content":"你好，有什么需要帮助？"})
            ]
        );
    }

    #[test]
    fn user_images_use_the_protocol_specific_base64_shape() {
        use crate::model::MessageImage;
        let image = MessageImage {
            media_type: "image/png".into(),
            data: "aGVsbG8=".into(),
            file: None,
        };
        assert_eq!(
            user_content("看这张图", &[image.clone()], Protocol::Anthropic),
            json!([
                {"type":"text","text":"看这张图"},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":"aGVsbG8="}}
            ])
        );
        assert_eq!(
            user_content("看这张图", &[image.clone()], Protocol::OpenAi),
            json!([
                {"type":"text","text":"看这张图"},
                {"type":"image_url","image_url":{"url":"data:image/png;base64,aGVsbG8="}}
            ])
        );
        // An image-only prompt still yields a valid user turn on both protocols.
        assert_eq!(
            user_content("", &[image], Protocol::Anthropic),
            json!([
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":"aGVsbG8="}}
            ])
        );
        assert_eq!(
            user_content("纯文本", &[], Protocol::OpenAi),
            json!("纯文本")
        );
    }

    #[test]
    fn config_round_trip_and_chatgpt_mode_do_not_require_api_keys_or_models() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider.json");
        let config = Config {
            kind: Kind::ChatGpt,
            enabled: true,
            ..Config::default()
        };
        assert!(config.validate().is_ok());
        config.save(&path).unwrap();
        let saved = Config::load(&path).unwrap();
        assert!(saved.kind == Kind::ChatGpt);
        assert!(saved.enabled);
        assert_eq!(saved.credential_id, config.credential_id);
        assert!(!std::fs::read_to_string(path).unwrap().contains("api_key\""));
    }

    #[test]
    #[ignore = "uses DEEPSEEK_API_KEY for a short real provider inference"]
    fn deepseek_environment_key_completes_real_chat() {
        let config = crate::discovery::DiscoveryRegistry::default()
            .discover()
            .into_iter()
            .find(|found| found.id == "deepseek")
            .expect("DeepSeek was not discovered")
            .config;
        assert!(config.protocol == Protocol::Anthropic);
        let key = config.key().expect("DEEPSEEK_API_KEY is not available");
        let models = runtime()
            .unwrap()
            .block_on(list_models(config.clone(), key.clone()))
            .expect("DeepSeek model discovery failed");
        assert!(models.contains(&config.model));
        println!("DeepSeek models discovered: {}", models.join(", "));
        let request = Request::start(
            config.clone(),
            key,
            vec![json!({"role":"user","content":"Reply with OK only."})],
            Thinking::Default,
        )
        .unwrap();
        let text = runtime()
            .unwrap()
            .block_on(request.response())
            .expect("DeepSeek request failed");
        assert!(!text.text.trim().is_empty());
        assert!(text.stats.output_tokens.is_some());
        assert!(text.stats.output_tps().is_some());
        println!(
            "DeepSeek live response verified ({} characters)",
            text.text.chars().count()
        );
        println!("{}", text.stats.caption());
    }
}
