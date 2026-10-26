# Glint Agent

使用 Glint / GPUI-CE 构建的原生 Agent 工作台，复用 Glint API 的 `AppLayout`、左侧图标栏、顶部栏和 Moonstone 主题。

```sh
cargo run -p glint-agent
```

- 使用现有 `Composer` / `PromptEditor` 输入与模型选择器，支持发送、停止、独立草稿和自动恢复历史。
- 打开本地项目；会话栏用 `SegmentedControl` 切换项目或日期分组，列表复用 `GroupList`。
- 设置左右分栏，侧栏列出供应商；每个供应商可配置多个模型及默认模型。输入框可直接切换已启用供应商的模型。
- 工具调用复用 `CollapsePanel`，展示状态、耗时、参数和结果，更新时保留展开状态。
- 真实 token 用量、首 token 延迟和输出速度；缺少服务端用量时不按字符估算。
- 独立日程页面使用共享 `Calendar` 组件，可添加、编辑、删除单次、每天和每周任务。
- `Enter` / `Cmd/Ctrl+Enter` 发送，`Shift+Enter` 换行，`Cmd/Ctrl+N` 新建会话，`Cmd/Ctrl+K` 搜索，`Cmd/Ctrl+,` 设置。

## 供应商

API 供应商支持 OpenAI Chat Completions 和 Anthropic Messages 协议。设置中添加供应商、填写地址和认证方式，在模型页获取模型列表或手动添加，再保存并启用。未配置时不会生成模拟回复。

API Key 存入操作系统凭据库，不写入配置 JSON。也可指定环境变量名，发送时读取应用环境；Finder 启动的应用可能不继承终端环境。两种认证方式二选一。

内置 DeepSeek 自动发现，依次检测 `DEEPSEEK_API_KEY` 和 `deepseek_api_key`。默认采用 Anthropic 协议，地址 `https://api.deepseek.com/anthropic`，模型通过认证的 `https://api.deepseek.com/models` 获取。旧版 DeepSeek 根地址配置会迁移，明确保存的协议保持不变。

`src/discovery.rs` 提供 `ProviderDiscovery` trait 与 `DiscoveryRegistry`。实现 `id()` 和 `discover(&dyn DiscoveryEnvironment) -> Option<Config>` 后注册即可扩展。发现器只能查询环境变量是否非空，配置仅保存变量名。重复 ID 替换实现并保持顺序；已保存配置和断开状态优先，不会被重新激活。

ChatGPT 订阅通过本机官方 `codex app-server` 接入：在设置中添加 ChatGPT、检查已有登录或在官方浏览器页面登录、获取模型并启用。凭据由 Codex 管理，应用不复制 OAuth 令牌。模型与额度取决于账户权限。每次请求在隔离的临时目录启动 ephemeral thread，本地上下文随请求传入，不恢复 Codex 原生 thread。

Codex 原生 shell、插件、浏览器等能力关闭，工具通过 app-server 的 dynamic tools 接入本应用。其他客户端审批请求默认拒绝。此集成面向本地应用；商业和托管服务请参阅官方 [Sign in with ChatGPT](https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server)。

## 工具与日程

常用工具包括列文件、读文件、搜索、写文件、精确替换、运行程序，以及日程的增删查改。文件工具限定在已打开项目内并检查路径和符号链接。写入、命令执行和日程修改需要界面确认；敏感文件读取也需要确认。运行程序使用 program/argv 参数，支持取消和超时；用户批准的程序自身能够访问项目外部。

任务保存项目、供应商和模型。应用运行时检查到期任务，后台创建独立会话并复用相同工具与确认流程；执行期间不抢占正在进行的对话。关闭应用期间不执行，重启后补执行尚未领取的到期任务一次。执行中断会记录错误，单次任务不会自动重复执行。

共享 Calendar 的示例位于 playground 的 Inputs 页面；终端折叠示例位于 Code Cards 页面。

## 存储与验证

数据目录：macOS `~/Library/Application Support/Glint Agent`，Windows `%APPDATA%/Glint Agent`，Linux `$XDG_DATA_HOME/Glint Agent`（默认 `~/.local/share/Glint Agent`）。可用 `GLINT_AGENT_DATA_DIR` 覆盖。

会话、供应商和日程分别存入 `sessions.json`、`providers.json`、`schedules.json`，旧 `provider.json` 自动迁移。文件读取失败时保留原文件。历史、草稿、工具参数与结果均为本地明文，应避免在提示词中粘贴密钥。

蓝白机器人图标的 SVG、PNG、ICO、ICNS 位于 [assets](assets/icon-design.md)，用于顶部栏、macOS Dock 和 Windows 可执行文件。

```sh
cargo check -p glint-agent
cargo check --example playground
cargo test -p glint-agent
cargo test --lib components::calendar::tests
```

真实凭据集成测试默认忽略；后两项会消耗少量模型额度：

```sh
cargo test -p glint-agent installed_codex_reports_chatgpt_account -- --ignored
cargo test -p glint-agent installed_codex_completes_one_text_turn -- --ignored
cargo test -p glint-agent deepseek_environment_key_completes_real_chat -- --ignored --nocapture
```
