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

API Key 存入操作系统凭据库，不写入 SQLite 或旧配置 JSON。也可指定环境变量名，发送时读取应用环境；Finder 启动的应用可能不继承终端环境。两种认证方式二选一。

内置 DeepSeek 自动发现，依次检测 `DEEPSEEK_API_KEY` 和 `deepseek_api_key`。默认采用 Anthropic 协议，地址 `https://api.deepseek.com/anthropic`，模型通过认证的 `https://api.deepseek.com/models` 获取。旧版 DeepSeek 根地址配置会迁移，明确保存的协议保持不变。

`src/discovery.rs` 提供 `ProviderDiscovery` trait 与 `DiscoveryRegistry`。实现 `id()` 和 `discover(&dyn DiscoveryEnvironment) -> Option<Config>` 后注册即可扩展。发现器只能查询环境变量是否非空，配置仅保存变量名。重复 ID 替换实现并保持顺序；已保存配置和断开状态优先，不会被重新激活。

ChatGPT 订阅通过本机官方 `codex app-server` 接入：在设置中添加 ChatGPT、检查已有登录或在官方浏览器页面登录、获取模型并启用。凭据由 Codex 管理，应用不复制 OAuth 令牌。模型与额度取决于账户权限。每次请求在隔离的临时目录启动 ephemeral thread，本地上下文随请求传入，不恢复 Codex 原生 thread。

Codex 原生 shell、插件、浏览器等能力关闭，工具通过 app-server 的 dynamic tools 接入本应用。其他客户端审批请求默认拒绝。此集成面向本地应用；商业和托管服务请参阅官方 [Sign in with ChatGPT](https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server)。

新会话首页将标题、输入框和下方快捷提示居中排布，发送后输入框固定到底部。新建会话在输入框上方选择项目；项目菜单向上展开并与按钮左侧对齐。空项目保留在侧栏，可通过分组的“+”新建会话。旧独立会话在已有或首次选择的项目中保留，发送前必须选择项目。删除会话通过确认 popover 执行。

首次成功回复后，兼容 API 供应商会使用独立任务生成标题；标题生成不会延长会话运行状态，删除会话时取消对应任务，失败时保留首条消息摘要。

输入框默认显示上下文余量环。新会话显示初始状态；收到服务端 usage 后，按当前请求的输入、输出和缓存输入 token 更新，不累加多轮工具请求的账单用量。切换会话或模型时同步刷新；未知窗口/用量会明确提示，不用字符长度估算。ChatGPT 使用 app-server 返回的 `modelContextWindow` 和最近用量；兼容 API 使用模型列表的 `context_window` / `context_length` 等元数据。

在“供应商 → 模型”中，每个模型都可填写上下文窗口，手动值优先于自动获取；留空恢复自动值。支持 `1M`、`256K`、`1.5M` 和精确整数，K/M 按十进制换算。服务模型列表的“添加”操作以模型 ID 独立定位，添加后原位显示“已添加”，保存后可用于对话。

## 工具与日程

常用工具包括列文件、读文件、搜索、写文件、精确替换、运行程序，以及日程的增删查改。输入框提供持久保存的权限选择器：默认“请求批准”对写入、命令、日程修改和敏感文件读取请求确认；“自动批准”跳过确认并保留项目文件边界；“完全访问权限”同时允许文件工具访问项目外路径。运行期间切换权限即时生效，选择自动批准或完全访问也会放行当前待批准调用。运行程序使用 program/argv 参数，支持取消和超时；用户批准的程序自身能够访问项目外部。

点击输入框的“+”或输入独立的 `@` 打开上下文面板，可通过系统选择器多选文件或目录。路径以 `@路径` 插入提示词，项目内使用相对路径，模型按需调用工具读取；项目外路径需要完全访问权限。

会话导航位于消息区域左侧居中，刻度间隔 10px。鼠标移动时邻近刻度平滑伸缩；导航是独立视图，hover 不触发整段会话重建。

任务保存项目、供应商和模型。应用运行时检查到期任务，后台创建独立会话并复用相同工具与确认流程；执行期间不抢占正在进行的对话。关闭应用期间不执行，重启后补执行尚未领取的到期任务一次。执行中断会记录错误，单次任务不会自动重复执行。

共享 Calendar 的示例位于 playground 的 Inputs 页面；终端折叠示例位于 Code Cards 页面。

## 记忆

设置中的“记忆”页可查看当前项目与全局记忆、读取来源、确认删除，以及开关语义回忆和每天整理。发送前使用当前模型理解意图并扩展检索，再审核候选摘要的相关性；例如“明天吃什么”可以关联饮食偏好和忌口。jieba-rs 的停用词过滤、SQLite FTS5 和 strsim 模糊匹配仅辅助找候选，不按泛词命中直接注入。当前项目与全局记忆参与召回，其他项目隔离。

有记忆时，每次提问最多增加两次当前模型调用；无记忆或关闭回忆时不调用。召回限时 30 秒，失败仍继续对话。只把选中的标题、摘要、ID 和相关原因作为历史参考加入上下文，模型通过 `memory_read` 按需读取全文。对话中可使用 `memory_search`、`memory_save`、`memory_delete`；写入、删除遵循权限选择器。

每天默认本地时间 23:00，应用运行且对话空闲时使用当前模型整理已有对话，错过时间在下次运行补处理。仅提取长期偏好、事实、项目决定和未完成目标，保存来源消息与进度，不删除原始会话。每轮最多 16 个批次，剩余内容留待下一轮或“立即整理”；失败保留进度，自动重试每次至少间隔 10 分钟，每个调度日最多失败 3 次。应用退出或任务中断后通过租约恢复。自动整理和语义回忆会使用供应商额度，可分别关闭。

运行时发送按钮切换为圆形停止按钮，点击终止当前回复；不再额外显示“停止回复”按钮。共享 Composer 通过 `stop_on_busy(true)` 启用，默认保持原有忙碌行为。

## 存储与验证

数据目录：macOS `~/Library/Application Support/Glint Agent`，Windows `%APPDATA%/Glint Agent`，Linux `$XDG_DATA_HOME/Glint Agent`（默认 `~/.local/share/Glint Agent`）。可用 `GLINT_AGENT_DATA_DIR` 覆盖。

会话、消息、项目、供应商、日程与记忆存入 `glint-agent.sqlite3`，使用 WAL 和事务；项目独立保存，允许没有会话。首次打开自动导入旧 `sessions.json`、`providers.json`（兼容 `provider.json`）、`schedules.json`，原文件保留；导入后以 SQLite 为准。图片与未发送图片草稿独立存入 `sessions.images/`，旧内联图片会在保存时迁移。草稿保存合并 300ms 内的变化，磁盘写入在后台串行完成，退出时刷新最新快照。读取或迁移失败不会覆盖原文件。历史、草稿、工具参数与结果均为本地明文，应避免在提示词中粘贴密钥。

蓝白机器人图标的 SVG、PNG、ICO、ICNS 位于 [assets](assets/icon-design.md)，用于 macOS Dock 和 Windows 可执行文件。

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
