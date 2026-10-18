# Glint API MCP

MCP 控制正在运行的 Glint API 原生工作台。工具调用和鼠标操作使用同一份状态、请求执行器和 SQLite 数据；agent 修改请求、环境或目录后，界面立即同步。

## 连接

在应用内按 `Cmd/Ctrl+K`，搜索 **MCP**，执行“复制 MCP 连接配置”。也可从命令行生成包含当前可执行文件绝对路径的配置：

```sh
cargo build -p glint-api
./target/debug/glint-api --mcp-config
```

将输出填入 agent 客户端的 MCP 配置。支持 `mcpServers` 格式的客户端可直接使用：

```json
{
  "mcpServers": {
    "glint-api": {
      "command": "/absolute/path/to/glint-api",
      "args": ["--mcp"]
    }
  }
}
```

`--mcp` 提供标准 [MCP stdio 传输](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#stdio)。它连接现有应用；没有可连接的窗口时会启动应用，最多等待 15 秒。GUI 启动输出不会写入 MCP stdout。关闭 agent 的 stdio 会话不会关闭工作台或取消已启动的网络请求。

使用自定义数据库时，GUI 与 MCP 客户端都应设置相同的 `GLINT_API_DB_PATH`；从该 GUI 复制的配置会带上此环境变量。设置 `GLINT_API_MCP_NO_AUTOSTART=1` 可禁用自动打开应用。当前连接同一数据库最近启动的窗口，请避免同时开启多个窗口进程编辑同一数据库。

stdio 进程通过带随机令牌的本机 TCP 桥访问 GPUI 主线程，不向局域网提供监听端口。发现文件位于数据库旁，例如 `workspace.mcp.json`，Unix 权限为 `0600`；它属于运行时文件，不应加入版本库或共享给他人。agent 配置中无需保存连接令牌。

## 工具与对象标识

提供 60 个工具，名称均以 `glint_` 开头。`tools/list` 返回每个工具的参数 JSON Schema、说明和读写/删除标记。协议实现初始化、版本协商、ping、工具发现、参数校验以及标准工具错误结果。

| 操作域 | 工具名称（省略 `glint_`） |
| --- | --- |
| MCP 调用历史 | `mcp_history_list`, `mcp_history_get` |
| 工作区 | `workspace_get`, `workspace_export`, `workspace_import` |
| 集合 | `collection_create`, `collection_rename`, `collection_delete` |
| 目录与继承 | `directory_create`, `directory_rename`, `directory_delete`, `directory_defaults_get`, `directory_defaults_set`, `tree_move` |
| 请求 | `request_create`, `request_get`, `request_update`, `request_select`, `request_delete`, `request_send`, `request_cancel`, `request_status`, `request_preview` |
| 响应与 cURL | `response_read`, `response_clear`, `traces_read`, `curl_import`, `curl_export` |
| 版本与历史 | `version_save`, `record_get`, `record_restore`, `record_tag`, `record_delete` |
| 环境 | `environment_get`, `environment_create`, `environment_rename`, `environment_delete`, `environment_select`, `environment_variable_set`, `environment_variable_delete`, `environment_variables_replace`, `environment_proxy_set`, `environment_draft` |
| WebSocket | `websocket_send`, `websocket_read`, `websocket_clear`；连接/断开使用 `request_send` / `request_cancel` |
| 插件与插件源 | `plugins_list`, `plugin_bind`, `plugin_bindings_reorder`, `registry_source_add`, `registry_source_remove`, `registry_refresh`, `plugin_install`, `plugin_uninstall` |
| 界面 | `ui_set` |
| 应用更新 | `update_check`, `update_status`, `update_download`, `update_cancel`, `update_install` |

所有工具作用于 GUI 当前选择的工作区；`workspace_get` 返回 `workspace_id` 与 `workspace_name`。工作区通过顶部入口管理，切换后请重新获取请求 ID；MCP 连接保持有效，调用记录按工作区隔离。

请求使用持久 UUID `request_id`，版本和历史使用 `record_id`。新建接口返回 ID；`workspace_get` / `request_get` 可列出已有 ID。目录使用集合名称和完整路径，如 `{"collection":"API","path":"用户/认证"}`，空路径表示集合根目录。环境按名称选择。

旧工作区首次加载时会给请求、版本和历史补上 ID，并在验证通过后保存；之后删除、排序和重启不会改变剩余对象的 ID。不要把列表下标当作请求或记录 ID。

## 请求示例

以下是 `tools/call` 的 `params` 示例；应先按 MCP 生命周期完成 `initialize` 和 `notifications/initialized`。工具响应同时提供文本内容与 `structuredContent`，后者格式为 `{"result": ...}`；业务错误返回 `isError: true` 和 `{"error": ...}`。未知工具或参数错误使用 JSON-RPC 错误。

创建集合：

```json
{"name":"glint_collection_create","arguments":{"name":"Local API"}}
```

创建带环境变量的 POST 请求：

```json
{
  "name":"glint_request_create",
  "arguments":{
    "collection":"Local API",
    "name":"Create item",
    "draft":{
      "method":"POST",
      "url":"{{base_url}}/items",
      "headers":[{"key":"Accept","value":"application/json"}],
      "body":"{\"name\":\"example\"}"
    }
  }
}
```

`draft` 是局部修改；省略字段保留原值，参数、请求头、Cookie、表单、后置提取规则等数组按整项替换。表格行的 `enabled` 默认 `true`，`note` 默认空字符串。`request_create` 先复制所在目录的继承配置，再应用传入的 draft；`curl_import` 使用解析后的完整配置。

发送后立即返回：

```json
{"name":"glint_request_send","arguments":{"request_id":"<UUID>","environment":"Development"}}
```

通过 `request_status` 查询完成状态，再用 `response_read` 读取响应，或在执行中用 `traces_read` 检查轨迹。`response_read` 默认返回最多 64 KiB，每次最多 1 MiB；`offset` / `next_offset` 是当前文本快照的 UTF-8 字节偏移，边界会对齐到完整字符。指定 `record_id` 可以读取历史响应。

`request_preview` 只解析和校验，不执行签名插件，返回 `plugins_applied: false`。发送后使用 `curl_export` 可取得实际应用插件请求头后的 cURL 快照。

## WS / SSE

- WS 请求的 `draft.method` 为 `WS`，地址使用 `ws://` 或 `wss://`。`request_send` 建立连接，轮询 `request_status.websocket_connected`，再调用 `websocket_send`。文本模板默认在每次发送时解析，可设置 `resolve_templates: false` 发送字面文本。
- `websocket_read` 返回当前消息、序号和 `next_after`；下一次可传 `after` 只读取新消息。序号在当前进程中递增，清空消息不会重用旧序号；关闭/切换请求后的会话日志不持久化。窗口中的文本与二进制 Base64 接收内容都可读取。
- HTTP 响应自动识别 `text/event-stream`，SSE 未结束时 `request_send` 同样已经返回。用 `request_status` 和 `response_read` 读取进展，不需要等流结束。`request_cancel` 断开后保留已接收结果。
- SSE 保留最近 1 MiB，`bytes` 是累计接收字节数，`offset` 是当前保留文本的偏移；长流滚动截断后不能将它当作跨快照的持久游标。需要完整内容的调用方应及时保存快照。

HTTP/SSE 仍共用一个前台执行；忙时 agent 需要先显式取消再切换，GUI 切换会取消该前台执行。WebSocket 按请求独立运行，GUI 或 agent 切换请求后继续后台收发，可通过 request_id 读取或取消对应会话。`request_get`、`request_status`、`response_read` 不会切换当前请求。

## 环境、插件和持久化

`environment_get`、`environment_variable_set`、`environment_variable_delete`、`environment_variables_replace` 和 `environment_draft` 接受可选的 `collection` 集合名称：省略时操作全局变量，指定时只操作该集合在指定环境下的变量。代理仅为全局环境设置。请求预览、发送、OAuth、WebSocket 消息与模板补全采用集合覆盖全局的读取顺序，其他集合的局部变量不可见。后置动作自动写入发起请求的集合与环境，即使界面已切换也不改变目标。

环境变量与代理修改会同步到原生编辑器。存在未保存的 GUI 环境修改时，工具拒绝覆盖；先用 `environment_get.editor` 查看，再用 `environment_draft` 保存或放弃。`environment_variable_set` 未传 `secret` 时保留已有变量的密钥标记。运行中的 HTTP 请求仍使用发送时快照；删除环境会要求先取消执行。

插件安装、插件源刷新和应用更新检查/下载在后台执行。分别用 `plugins_list` / `update_status` 读取进度与错误。插件安装不自动授予请求执行权限：`plugin_bind` 使用 `plugins_list` 返回的 `identity` 和 `action_id`，授权绑定到确切安装代码的指纹。目录默认配置也可传入 `plugin_bindings` 并启用 `plugins` 继承项。请求的前后置动作与 TLS/代理策略仍由现有执行器校验。

应用自动安装更新仍受平台、构建类型、未保存编辑和正在执行请求的原有检查约束。安装后会重启 GUI，客户端需要重新连接。

读取结果默认隐藏 Token、Authorization、Cookie、Set-Cookie、密钥变量及原始 cURL/请求快照。需要读取这些字段时显式传 `include_secrets: true`。这按已知字段隐藏内容，不会检测或改写请求体、响应体、URL 或消息文本中的所有潜在密钥。

`workspace_export` 默认生成 `redacted: true` 的导出；恢复备份应使用 `include_secrets: true` 的完整 workspace。`workspace_import` 验证对象关系和 ID 后替换当前工作区，要求显式 `replace: true`，会取消当前请求并替换未保存编辑。保存错误会作为工具错误返回，说明内存修改已经应用但持久化失败。

原生工作区操作不等待用户点击确认弹层，MCP 客户端可根据工具的 `readOnlyHint` / `destructiveHint` 设置自己的审批方式。MCP 单条消息上限 64 MiB；大响应优先使用 `response_read` 分段读取。

## 验证

```sh
cargo test -p glint-api
cargo check -p glint-api --all-targets
```

测试覆盖 MCP 协议生命周期和错误、真实本地桥接/stdio 转发、认证、参数 Schema、原生工作台 CRUD、环境密钥隐藏、插件授权、执行中保护、消息序号、UTF-8 响应分段、旧数据 ID 迁移和保存恢复。

原生端到端验证（需要桌面会话）：

```sh
cargo build -p glint-api
python3 crates/glint-api/tests/mcp_smoke.py
```

脚本启动独立临时工作区和本地回显服务，经真正的 stdio 客户端验证 60 个工具发现、HTTP 与历史、WS 回显、SSE 流式读取和取消，以及 SQLite 落盘；结束后关闭它启动的窗口，不修改现有用户工作区。可用 `GLINT_API_BINARY` 指向其他构建产物。

## 调用历史页面

侧边图标栏的 MCP 入口显示所有已记录的工具调用，按北京时间日期分组，从新到旧排列。支持按工具名、客户端或错误搜索，筛选全部/成功/失败；每页 100 条，可继续翻页查看全部历史。详情包含调用时间、客户端、JSON-RPC 请求 ID、耗时、参数和结果；页头可以复制 MCP 连接配置。

记录包含成功、业务失败和工具名/参数校验拒绝，不包含 `initialize`、`ping`、`tools/list` 等协议握手。耗时和结果表示工具调用本身；启动 HTTP、WS、SSE 或插件下载成功不代表后续网络执行已经成功。记录独立保存在当前数据库的 `mcp_calls` 表，重启后保留，导入工作区不会清空已有调用历史；旧版本没有记录的调用无法补回。

参数和结果在保存前始终隐藏已知密钥字段及当前工作区已知的密钥值，即使原调用设置了 `include_secrets: true` 也不会保存这些原始值。环境变量单值写入的 value 一律隐藏。每份参数/结果超过 64 KiB 时保存截断摘要，不限制历史条数。未知的敏感内容仍可能存在于任意消息或响应文本中。

### OAuth 2.0 配置

`request_create`、`request_update` 与目录默认配置的 `draft.oauth` 可设置 `{ id, authorization_url, token_url, client_id, redirect_uri, scope, pkce, basic_auth, manual_callback }`；`oauth: null` 清除 OAuth 配置。凭据不通过此对象传入，`client_secret` 等未知字段会被拒绝。设置 OAuth 会关闭旧的 Bearer 模式；显式修改 `auth` 而未提供 `oauth` 会切回旧模式。浏览器授权和首次 Client Secret 输入在原生身份验证界面完成，发送工具会读取凭据库并按需刷新。`request_preview` 展示脱敏的 OAuth Authorization，不触发授权或刷新；OAuth 凭据不随工作区导出。

环境代理现为工作区内共享配置：`environment_get` 返回 `proxy_id`、生效的 `proxy` 和 `available_proxies`。`environment_proxy_set` 可传入 `proxy_id` 绑定已有代理，传 `null` 解除绑定；也可继续传 `enabled`、`url`、`skip_tls_verification` 创建或更新已绑定代理，更新会影响所有绑定环境。内置模式使用 `built_in: true` 和 `mappings: [{"domain":"api.example.com","ip":"127.0.0.1"}]`。工作区导出包含 `proxies`，旧内联代理导入时自动迁移。

WebSocket 会话按 `request_id` 独立运行，切换前台请求不会断开后台连接。`glint_request_status` 可读取后台会话状态，`glint_websocket_send/read/clear` 与 `glint_request_cancel` 可直接操作后台会话；读取事件额外返回原始 `payload`、方向、时间及插件生成的 `title`，保留原有 `message` 字段。`glint_workspace_get` 的 `background_tasks` 列出运行中的请求。
