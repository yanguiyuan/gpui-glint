//! Request-scoped MCP clients. Connections are never shared across conversations.
use rmcp::{
    ServiceExt,
    service::{RoleClient, RunningService},
    transport::{StreamableHttpClientTransport, TokioChildProcess},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default, rename = "mcpServers")]
    servers: BTreeMap<String, Server>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Server {
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    url: Option<String>,
    #[serde(default = "enabled")]
    enabled: bool,
}
fn enabled() -> bool {
    true
}
struct RemoteTool {
    name: String,
    original: String,
    server: usize,
    schema: Value,
}
#[derive(Default)]
struct Connected {
    clients: Vec<RunningService<RoleClient, ()>>,
    tools: Vec<RemoteTool>,
}
#[derive(Clone, Default)]
pub struct Client {
    path: Option<PathBuf>,
    state: Arc<tokio::sync::OnceCell<Connected>>,
}
impl Client {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            ..Self::default()
        }
    }
    pub fn instructions(&self) -> String {
        let path = self
            .path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        format!(
            r#"MCP 自配置：配置文件为 {path}。用户要求配置 MCP 时，使用 mcp_config_read 读取，用 mcp_config_write 保存完整 JSON，保留其他服务器。格式：{{"mcpServers":{{"名称":{{"command":"程序绝对路径","args":[],"env":{{}},"enabled":true}}}}}}；远程服务器用 url 替代 command/args/env。command 与 url 二选一，enabled=false 停用。只能配置用户授权的服务器，不要猜测下载包或认证信息。配置写入不代表连接成功；在下一次用户请求建立连接并发现工具后才生效。配置工具仅访问这个文件，不需要完全访问权限，写入遵循批准模式。服务器进程/网络访问不受项目边界约束。当前支持 stdio 和 Streamable HTTP 工具，不支持 OAuth、旧 SSE、资源或提示模板。"#
        )
    }
    pub fn read_config(&self) -> Result<String, String> {
        let path = self.path.as_ref().ok_or("MCP 配置路径不可用")?;
        match std::fs::read_to_string(path) {
            Ok(content) => Ok(content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(json!({"mcpServers":{}}).to_string())
            }
            Err(e) => Err(format!("读取 MCP 配置失败：{e}")),
        }
    }
    pub fn write_config(&self, content: &Value) -> Result<String, String> {
        let config: Config =
            serde_json::from_value(content.clone()).map_err(|e| format!("MCP 配置无效：{e}"))?;
        for (name, server) in config.servers {
            if server.command.as_ref().is_some_and(|s| s.trim().is_empty())
                || server.url.as_ref().is_some_and(|s| s.trim().is_empty())
                || server.command.is_some() == server.url.is_some()
            {
                return Err(format!("MCP {name} 必须且只能配置非空 command 或 url"));
            }
        }
        let path = self.path.as_ref().ok_or("MCP 配置路径不可用")?;
        let bytes = serde_json::to_vec_pretty(content).map_err(|e| e.to_string())?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let save = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&temporary, path)
        })();
        if let Err(error) = save {
            let _ = std::fs::remove_file(temporary);
            return Err(format!("保存 MCP 配置失败：{error}"));
        }
        Ok("MCP 配置已保存，下一次请求连接并生效；尚未验证连接。".into())
    }
    pub async fn prepare(&self, root: Option<&std::path::Path>) -> Result<(), String> {
        self.state
            .get_or_try_init(|| async {
                let Some(path) = &self.path else {
                    return Ok(Connected::default());
                };
                let bytes = match std::fs::read(path) {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(Connected::default());
                    }
                    Err(e) => return Err(format!("无法读取 MCP 配置：{e}")),
                };
                let config: Config =
                    serde_json::from_slice(&bytes).map_err(|e| format!("MCP 配置无效：{e}"))?;
                Self::connect(config, root).await
            })
            .await?;
        Ok(())
    }
    async fn connect(config: Config, root: Option<&std::path::Path>) -> Result<Connected, String> {
        let mut state = Connected::default();
        for (name, server) in config.servers.into_iter().filter(|(_, s)| s.enabled) {
            let connect = async {
                let client = match (server.command, server.url) {
                    (Some(program), None) => {
                        let mut command = tokio::process::Command::new(program);
                        command
                            .args(server.args)
                            .envs(server.env)
                            .kill_on_drop(true);
                        command.stderr(std::process::Stdio::null());
                        if let Some(root) = root {
                            command.current_dir(root);
                        }
                        ().serve(TokioChildProcess::new(command).map_err(|e| e.to_string())?)
                            .await
                            .map_err(|e| e.to_string())?
                    }
                    (None, Some(url)) => {
                        ().serve(StreamableHttpClientTransport::from_uri(url))
                            .await
                            .map_err(|e| e.to_string())?
                    }
                    _ => return Err("必须且只能配置 command 或 url".to_owned()),
                };
                let tools = client.list_all_tools().await.map_err(|e| e.to_string())?;
                Ok::<_, String>((client, tools))
            };
            let (client, tools) = tokio::time::timeout(Duration::from_secs(30), connect)
                .await
                .map_err(|_| format!("MCP {name} 连接超时"))?
                .map_err(|e| format!("MCP {name}：{e}"))?;
            let index = state.clients.len();
            for tool in tools {
                // Numeric names remain portable across provider naming restrictions.
                let alias = format!("mcp_{}_{}", index, state.tools.len());
                state.tools.push(RemoteTool { name: alias.clone(), original: tool.name.to_string(), server: index,
                        schema: json!({"name":alias,"description":format!("MCP {name} / {}: {}",tool.name,tool.description.as_deref().unwrap_or("")),"parameters":tool.input_schema}) });
            }
            state.clients.push(client);
        }
        Ok(state)
    }
    pub async fn probe(
        name: String,
        server: Value,
        root: Option<PathBuf>,
    ) -> Result<Vec<(String, String)>, String> {
        let mut server = server;
        server["enabled"] = true.into();
        let config: Config = serde_json::from_value(json!({"mcpServers":{name:server}}))
            .map_err(|e| e.to_string())?;
        let connected = Self::connect(config, root.as_deref()).await?;
        Ok(connected
            .tools
            .iter()
            .map(|tool| {
                (
                    tool.original.clone(),
                    tool.schema["description"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect())
    }
    pub fn definitions(&self, anthropic: bool) -> Value {
        let mut definitions = crate::tools::definitions(anthropic);
        if let Some(state) = self.state.get() {
            for tool in &state.tools {
                let s = &tool.schema;
                definitions.as_array_mut().unwrap().push(if anthropic {
                    json!({"name":s["name"],"description":s["description"],"input_schema":s["parameters"]})
                } else { json!({"type":"function","function":s}) });
            }
        }
        definitions
    }
    pub async fn call(&self, name: &str, arguments: &Value) -> Result<String, String> {
        let state = self.state.get().ok_or("MCP 尚未连接")?;
        let tool = state
            .tools
            .iter()
            .find(|t| t.name == name)
            .ok_or("MCP 工具不存在")?;
        let params = serde_json::from_value(json!({"name":tool.original,"arguments":arguments}))
            .map_err(|e| format!("MCP 参数无效：{e}"))?;
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            state.clients[tool.server].call_tool(params),
        )
        .await
        .map_err(|_| "MCP 工具调用超时")?
        .map_err(|e| format!("MCP 调用失败：{e}"))?;
        let output = serde_json::to_string(&result).map_err(|e| e.to_string())?;
        if result.is_error == Some(true) {
            Err(output)
        } else {
            Ok(output)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn absent_and_invalid_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let empty = Client::new(path.clone());
        empty.prepare(None).await.unwrap();
        assert_eq!(empty.definitions(true), crate::tools::definitions(true));
        std::fs::write(&path, r#"{"mcpServers":{"bad":{"command":"a","url":"b"}}}"#).unwrap();
        assert!(
            Client::new(path)
                .prepare(None)
                .await
                .unwrap_err()
                .contains("bad")
        );
    }
    #[test]
    fn self_configuration_is_validated_and_preserves_existing_file_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let client = Client::new(path.clone());
        assert!(client.instructions().contains(&path.display().to_string()));
        assert_eq!(client.read_config().unwrap(), r#"{"mcpServers":{}}"#);
        let valid = json!({"mcpServers":{"local":{"command":"/bin/example","enabled":false}}});
        client.write_config(&valid).unwrap();
        assert!(
            client
                .write_config(&json!({"mcpServers":{"bad":{"command":"a","url":"b"}}}))
                .is_err()
        );
        assert_eq!(
            serde_json::from_str::<Value>(&client.read_config().unwrap()).unwrap(),
            valid
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn stdio_discovers_and_calls_tools_and_preserves_errors() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("server.py");
        std::fs::write(&script, r#"
import sys, json
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion':request['params']['protocolVersion'], 'capabilities':{'tools':{}}, 'serverInfo':{'name':'fixture','version':'1'}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'echo', 'description':'Echo', 'inputSchema':{'type':'object','properties':{'fail':{'type':'boolean'}}}}]}
    elif method == 'tools/call':
        result = {'content':[{'type':'text','text':'fixture reply'}], 'isError':request['params']['arguments'].get('fail',False)}
    else: result = {}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
"#).unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(
            &path,
            json!({"mcpServers":{"fixture":{"command":"python3","args":[script]}}}).to_string(),
        )
        .unwrap();
        let tools = Client::probe(
            "fixture".into(),
            json!({"command":"python3", "args":[script], "enabled":false}),
            Some(dir.path().to_owned()),
        )
        .await
        .unwrap();
        assert_eq!(tools[0].0, "echo");
        let client = Client::new(path);
        client.prepare(Some(dir.path())).await.unwrap();
        for anthropic in [true, false] {
            let definitions = client.definitions(anthropic);
            let tool = definitions.as_array().unwrap().last().unwrap();
            assert_eq!(
                if anthropic {
                    &tool["name"]
                } else {
                    &tool["function"]["name"]
                },
                "mcp_0_0"
            );
        }
        assert!(
            client
                .call("mcp_0_0", &json!({}))
                .await
                .unwrap()
                .contains("fixture reply")
        );
        assert!(
            client
                .call("mcp_0_0", &json!({"fail":true}))
                .await
                .unwrap_err()
                .contains("fixture reply")
        );
        assert!(client.call("mcp_missing", &json!({})).await.is_err());
    }
}
