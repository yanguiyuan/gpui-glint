# Glint Agent

使用 Glint / GPUI-CE 构建的原生 Agent 工作台，复用 Glint API 的 `AppLayout`、左侧图标栏、顶部栏和 Moonstone 主题。

```sh
cargo run -p glint-agent
```

- 使用现有 `Composer` / `PromptEditor` 输入与模型选择器，支持发送、停止、独立草稿和自动恢复历史。
- 打开本地项目；会话栏用 `SegmentedControl` 切换项目或日期分组，列表复用 `GroupList`。
- 设置左右分栏，侧栏列出供应商；每个供应商可配置多个模型及默认模型。输入框可直接切换已启用供应商的模型。
- 工具调用复用 `CollapsePanel`，展示状态、耗时、参数和结果，更新时保留展开状态。
- 思考面板在流式输出时滚动预览最新一行，完成后显示“已思考 x.x s”。每段从首次收到思考内容开始计时，到正文或工具调用开始时结束；耗时随消息保存，旧历史缺少计时数据时显示“已思考”。
- 真实 token 用量、首 token 延迟和输出速度；缺少服务端用量时不按字符估算。
- 独立日程页面使用共享 `Calendar` 组件，可添加、编辑、删除单次、每天和每周任务。
- `Enter` / `Cmd/Ctrl+Enter` 发送，`Shift+Enter` 换行，`Cmd/Ctrl+N` 新建会话，`Cmd/Ctrl+K` 搜索，`Cmd/Ctrl+,` 设置。

## 供应商

API 供应商支持 OpenAI Chat Completions 和 Anthropic Messages 协议。设置中添加供应商、填写地址和认证方式，在模型页获取模型列表或手动添加，再保存并启用。未配置时不会生成模拟回复。

API Key 以明文存入本机 `glint-agent.sqlite3` 的独立 `provider_secrets` 表，供应商配置 JSON 和通用配置元数据不包含密钥。也可指定环境变量名，发送时读取应用环境；Finder 启动的应用可能不继承终端环境。两种认证方式二选一，环境变量模式不会回退到 SQLite 密钥。

模型 API Key 仅从 SQLite 或所选环境变量读取，不再包含系统钥匙串访问或导入入口；清除按钮删除 SQLite 中的密钥。ChatGPT 登录仍由官方 Codex 管理。

内置 DeepSeek 自动发现，依次检测 `DEEPSEEK_API_KEY` 和 `deepseek_api_key`。默认采用 Anthropic 协议，地址 `https://api.deepseek.com/anthropic`，模型通过认证的 `https://api.deepseek.com/models` 获取。旧版 DeepSeek 根地址配置会迁移，明确保存的协议保持不变。

`src/discovery.rs` 提供 `ProviderDiscovery` trait 与 `DiscoveryRegistry`。实现 `id()` 和 `discover(&dyn DiscoveryEnvironment) -> Option<Config>` 后注册即可扩展。发现器只能查询环境变量是否非空，配置仅保存变量名。重复 ID 替换实现并保持顺序；已保存配置和断开状态优先，不会被重新激活。

ChatGPT 订阅通过本机官方 `codex app-server` 接入：在设置中添加 ChatGPT、检查已有登录或在官方浏览器页面登录、获取模型并启用。凭据由 Codex 管理，应用不复制 OAuth 令牌。模型与额度取决于账户权限。每次请求在隔离的临时目录启动 ephemeral thread，本地上下文随请求传入，不恢复 Codex 原生 thread。

Codex 原生 shell、插件、浏览器等能力关闭，工具通过 app-server 的 dynamic tools 接入本应用。其他客户端审批请求默认拒绝。此集成面向本地应用；商业和托管服务请参阅官方 [Sign in with ChatGPT](https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server)。

新会话首页将标题、输入框和下方快捷提示居中排布，发送后输入框固定到底部。新建会话在输入框上方选择项目；项目菜单向上展开并与按钮左侧对齐。空项目保留在侧栏，可通过分组的“+”新建会话。旧独立会话在已有或首次选择的项目中保留，发送前必须选择项目。删除会话通过确认 popover 执行。

首次成功回复后，兼容 API 供应商会使用独立任务生成标题；标题生成不会延长会话运行状态，删除会话时取消对应任务，失败时保留首条消息摘要。

输入框默认显示上下文余量环。新会话显示初始状态；收到服务端 usage 后，按当前请求的输入、输出和缓存输入 token 更新，不累加多轮工具请求的账单用量。切换会话或模型时同步刷新；未知窗口/用量会明确提示，不用字符长度估算。ChatGPT 使用 app-server 返回的 `modelContextWindow` 和最近用量；兼容 API 使用模型列表的 `context_window` / `context_length` 等元数据。

在“供应商 → 模型”中，每个模型都可填写上下文窗口，手动值优先于自动获取；留空恢复自动值。支持 `1M`、`256K`、`1.5M` 和精确整数，K/M 按十进制换算。服务模型列表的“添加”操作以模型 ID 独立定位，添加后原位显示“已添加”，保存后可用于对话。

顶部点击“命令面板”或按 Cmd/Ctrl+K，搜索并执行“压缩会话”可手动压缩上下文；设置 → 常规可开启自动压缩并设置 1–99% 的阈值，默认 75%。发送前按当前模型最近一次实测占用检查，未知用量时不自动触发。压缩使用当前模型生成历史摘要，保留最近两轮完整对话，原始消息仍保存在本机；失败或取消不会替换上下文。压缩后等待下一次回复更新实测用量。

伙伴 → 设定可单独选择供应商、模型和思考强度，也可跟随普通 Agent 的模型。选择伙伴后输入框隐藏模型和思考选择器；交互、频道、日程请求及会话压缩使用伙伴配置，切回普通 Agent 后恢复选择器。

## SSH 远程项目

会话侧栏的地球图标或项目菜单中的“打开 SSH 项目…”可连接远端。连接栏可填写 `ssh user@example.com`、`ssh -p 2222 user@host`、SSH config 主机别名，或 `ssh://user@host:2222/absolute/project`。另填远程目录，例如 `~/projects/demo`；默认 `~` 打开该用户的远程主目录。

使用系统 `ssh`，复用密钥、ssh-agent 和 `~/.ssh/config`，不保存密码。连接是非交互模式；如果依赖密码或交互解锁，请先在终端配置可用的密钥/agent。首次连接采用 `accept-new` 记录主机密钥；已记录密钥改变时拒绝连接。远端需有 `python3`；Git 变更页还需远端有 `git`。

项目以 SSH URI 保存并在重启后恢复，项目标签显示主机和目录。文件读取、列表、搜索、写入、精确替换、`run_command` 和 Git 工作区变更通过 SSH 在远端执行；`@` 上下文列表读取远程文件索引。文件修改复用原有批准规则和差异展示，项目范围模式在远端解析真实路径并阻止 `..`、绝对路径及指向项目外的符号链接；完全访问权限允许访问项目外的远程路径。SSH 搜索使用 Python 正则。

宿主 API 工具仍操作本地工作台，外部 MCP 保持原配置的执行位置；用户/自定义技能从本机加载，不自动加载远程项目技能。远程命令为非交互执行，前台默认 60 秒、后台默认 3600 秒超时；超时或断线后应先核对远端状态，再决定是否重试写入。

## 频道与主人通知

侧栏“频道”当前支持 WPS 协作，可添加多个自建应用，每个应用绑定一个伙伴和本地项目。Workbench 可从 API 当前请求的环境填入 `wps_app_id` / `wps_app_key`，并遵循 KSO-1 插件的自定义变量映射；也可手动填写 AK/SK。应用配置、密钥与消息队列直接保存到同目录的 `channels.sqlite3`。密钥以本地明文保存，不再访问系统钥匙串；旧应用需重新填写或从 API 导入密钥并保存一次。

1. 在 WPS 开放平台启用应用机器人，申请 `app:kso.chat_message.readwrite` 消息权限，将事件订阅方式设为长连接，并订阅 `kso.app_chat.message.create`。
2. 保存应用、绑定伙伴和项目后启用监听。首次在机器人**单聊**中发送 `我是你的主人`，以企业 ID 与 sender ID 完成持久认领。
3. 此后只接受主人的消息。消息按应用、伙伴、项目、企业、会话和 sender 隔离，进入实际伙伴会话，最终回复自动发回原 WPS 会话。群聊也只处理已认领主人的消息；其他发送者和机器人事件不触发模型。
4. 伙伴可通过 `channel_list` 查询自己的应用，通过 `channel_send` 主动通知主人。日程任务同样可用，例如“每天 9 点检查进度，并用 channel_send 通知我”。通知仅发到主人的认领单聊；启用绑定即授权此范围内的通知，无需逐次在桌面批准。工具会等待发送结果，超过等待时间返回 queued；可用 `channel_delivery_status` 继续查询。`sent` 表示 WPS 接受消息，不代表用户已读。

收到主人消息后，优先用 `emoji_receive` 表情表示收到；回复的全部分段被 WPS 接收后替换为 `emoji_done`，生成或发送失败时替换为 `emoji_error`。表情任务落盘并在后台执行，失败会显示原因，不阻断正文回复。主动通知不针对用户原始消息添加表情。

事件通过 KSO-1 建连、验签及 AES 解密后落盘，再 ACK；重复消息不会重复执行。断线自动退避重连，连接被其他客户端替换时停止重连，避免争抢（包括同一应用在 API 工作台中的事件监听连接）。文本回复超过长度限制会分段。中断或送达状态不明的发送不会自动重放，频道页可手动重试并提示可能重复。解除主人绑定须先停用应用并等待伙伴结束。当前接入文本内容；图片、附件不会作为可见图片传给模型。退出应用后停止收发，未处理消息和发送队列保留。

协议参考：[发送消息](https://365.kdocs.cn/3rd/open/documents/app-integration-dev/wps365/server/message/message_managment/create_message)、[事件长连接](https://365.kdocs.cn/3rd/open/documents/app-integration-dev/wps365/server/event-subscription/event-websocket)。

### 会话标签与用户授权

WPS 会话标签属于用户，只支持用户授权。频道配置的“授权 WPS 账号”使用该应用 App ID 发起 OAuth；也可让伙伴调用 `channel_user_authorize` 返回授权链接。先认领主人，再在运行 Glint 的电脑浏览器中打开链接并同意。应用后台只需开通 `kso.chat.readwrite` 用户权限，登记回调地址 `http://127.0.0.1:17944/oauth/wps/callback`（配置页可复制）。回调校验随机 state、一次性 code，以及发起授权的频道主人记录仍然有效。只申请会话权限，不读取账号基础信息；授权记录绑定到发起流程的频道主人，实际操作范围属于浏览器中同意授权的账号，因此请确认登录的是主人自己的 WPS 账号。只读权限 `kso.chat.read` 可查询会话信息，但打标签需要 `kso.chat.readwrite`。

用户 token 和 refresh token 保存在 `channels.sqlite3`，到期时自动刷新，保持原 refresh token 的截止时间。伙伴只能读取授权状态，不能读取令牌。配置页可取消待完成授权、清除本地令牌；解除主人绑定也会清除用户授权。清除本地令牌不等同于在 WPS 账号侧撤销应用权限。

工具：`channel_user_authorization_status` 检查授权；`wps_chats_list` 分页查询用户会话；`wps_chat_tags_list/create/delete` 管理标签；`wps_chat_tags_get/apply/remove` 查询或修改指定会话的标签；`wps_chat_tags_chats` 查询标签下的会话。会话 ID 省略时优先使用当前频道入站对话，否则使用主人的机器人单聊。`delete` 删除标签及所有会话关联，`remove` 仅移除一个会话的标签关联。标签写入仍遵循当前 Agent 工具权限。可以直接说“给这个会话打上工作和紧急标签”；伙伴会先检查用户授权，创建或复用标签，再关联到会话。

## 工具与日程

常用工具包括列文件、读文件、搜索、写文件、精确替换、运行程序，以及日程的增删查改。输入框提供持久保存的权限选择器：默认“请求批准”对写入、命令、日程修改和敏感文件读取请求确认；“自动批准”跳过确认并保留项目文件边界；“完全访问权限”同时允许文件工具访问项目外路径。运行期间切换权限即时生效，选择自动批准或完全访问也会放行当前待批准调用。运行程序使用 program/argv 参数，支持取消和超时；用户批准的程序自身能够访问项目外部。

编辑已有文件时，`read_file` 返回带行号的正文和完整文件 `expected_sha256`；写入/替换可传入该值，拒绝覆盖读取之后发生的变化。新文件可用 `create_only:true`。普通编辑采用同目录临时文件、同步和原子替换，并保留原文件权限；写前完整快照先落入 SQLite `edit_journal`，完成状态再更新。`session_changes` 合并保存的工具记录、当前请求和写前日志，按 ID 去重；`prepared` / `uncertain` / `failed` 表示需要核对实际文件，不应直接当作成功修改。

`run_command` 返回 JSON 状态、退出码、信号、耗时和输出，非零退出码表示失败；只读搜索的 `rg` 无匹配仍作为空结果。可指定项目内 `cwd`、`timeout_seconds`（1–86400，前台默认 60 秒、后台默认 3600 秒）和 `background:true`。后台返回 `run_id`，通过只读 `command_poll` 查询/分页读取日志，通过需要批准的 `command_stop` 停止；运行记录按会话、伙伴和项目隔离。前台取消、超时和退出应用都会清理本地进程组；SSH 使用远端进程组清理辅助程序，断线或远程停止无法确认时明确返回未确认状态。命令使用当前用户权限，工作目录不是操作系统沙箱。

输出每 80ms 合并刷新，后台任务仅按查询显示进度；预览保留每路输出的头尾各 32KiB，单路磁盘日志上限 32MiB。`command_poll` 的 `stream` / `offset` / `limit` 按字节分页读取保留的日志，截断标志单独返回。日志保存在应用数据目录的私有 `command-runs` 下，完成日志最多保留 128 份、合计约 512MiB（正在运行的任务不清理），过期记录不可再次查询。应用异常退出后的旧运行记录标为中断，不假定远端命令已停止。

主代理可调用 `subagent({task:"完整的子任务与必要上下文",title:"短任务名"})`，使用当前供应商、模型和思考设置在独立对话中执行任务，并等待结果返回。支持兼容 OpenAI、Anthropic 和 ChatGPT；子代理继承系统与技能上下文，不自动复制主代理的聊天历史，与主代理共享项目文件、技能、MCP 和实时权限设置。会话顶部提供“子代理”下拉列表，显示运行中、待批准、完成、失败及取消的任务；主对话中保留可点击的任务摘要。点击列表或摘要进入独立执行页，复用主代理的思考、Markdown、工具卡片与实时命令输出，委派指令可展开查看，底部不显示输入框。返回主对话保留草稿、滚动位置与展开状态；子代理页面之间也保留各自滚动位置。审批注明所属子任务，可在详情页处理；批准后立即恢复运行状态。完整过程、思考耗时、结果及统计随会话保存，停止和失败后仍可回看；旧记录缺少的过程不会补造。每次用户请求最多执行 8 个子任务，每个子任务限时 120 秒；停止主代理会同时取消正在执行的子任务。子代理的工具列表不包含 `subagent`，执行入口也会拒绝继续派生，包括完全访问权限模式。当前按工具调用顺序等待各个子任务，页面切换不改变执行状态；不提供后台调度或跨轮消息。详情页的停止按钮明确作用于主任务及其子代理。

点击输入框的“+”或输入独立的 `@` 打开上下文面板，可通过系统选择器多选文件或目录。文件、目录与侧栏选区统一显示为带图标的引用块，保留完整路径与行范围。20 行且 2KB 以内的选区随请求附带原文，较长选区只发送路径与行范围，由模型按需调用工具读取；项目外路径需要完全访问权限。

`@` 面板也按关键词提供当前项目的文件和目录候选，匹配文件名与相对路径、不区分大小写，例如 `@README` 或 `@src/main`。选择后将当前 `@关键词` 替换为紧凑的文件引用块；重复文件名使用独立标记并保留各自完整路径。悬停引用可查看有高度限制的内容预览，较长内容底部逐级模糊渐隐。索引在后台建立，遵循 `.gitignore` / `.ignore`，排除 `.git`、`target`、`node_modules`、`.venv`、`.cache`，不跟随符号链接。每次打开面板刷新，输入期间只过滤缓存，不读取文件内容或遍历磁盘。索引最多 50000 个路径、每次显示前 60 个匹配，优先文件名与浅层路径；项目切换会丢弃旧索引，文件已删除或被替换成包外链接时不能选入。

会话导航位于消息区域左侧居中，刻度间隔 10px。鼠标移动时邻近刻度平滑伸缩；导航是独立视图，hover 不触发整段会话重建。

任务保存项目、供应商和模型。应用运行时检查到期任务，后台创建独立会话并复用相同工具与确认流程；执行期间不抢占正在进行的对话。关闭应用期间不执行，重启后补执行尚未领取的到期任务一次。执行中断会记录错误，单次任务不会自动重复执行。

共享 Calendar 的示例位于 playground 的 Inputs 页面；终端折叠示例位于 Code Cards 页面。

执行清单通过 `todo_write({todos:[{content:"检查实现",status:"in_progress"},{content:"运行验证",status:"pending"}]})` 全量更新，`todo_read` 读取，`todos:[]` 清空。支持 pending、in_progress、completed，最多 30 项且至多一项进行中；参数校验失败保留原清单。清单仅用于展示执行进度，不创建日程或自动执行任务，不需要额外审批。清单面板与 @ 面板一样锚定在输入区上方，可展开/收起并显示完成比例；打开 @ 面板时暂时隐藏，全部完成时自动折叠。各会话独立保存清单，切换会话或重启可恢复，停止任务不会把未完成事项标成完成。子代理使用独立清单，在其执行页展示并随执行记录保存，不会覆盖主代理清单。共享组件为 `TodoList`，Composer 通过 `set_todos` 更新。

## 记忆

设置中的“记忆”页可查看当前项目与全局记忆、读取来源、确认删除，以及开关“记忆主动匹配”和每天整理。主动匹配默认开启，沿用原“自动语义回忆”的保存设置；关闭后取消后台匹配，仍可通过工具按需搜索和读取记忆。发送后在后台使用当前模型理解意图并扩展检索，再审核候选摘要的相关性；例如“明天吃什么”可以关联饮食偏好和忌口。jieba-rs 的停用词过滤、SQLite FTS5 和 strsim 模糊匹配仅辅助找候选，不按泛词命中直接注入。当前项目与全局记忆参与召回，其他项目隔离。

有记忆时，每次提问最多增加两次当前模型调用；无记忆或关闭主动匹配时不调用。主 agent 立即开始，不等待召回。兼容 API 在下一轮模型请求前注入一次结果，保持工具调用和结果配对；ChatGPT 在事件边界通过 `turn/steer` 追加历史参考，或在工具返回时附带结果。已经发送的 HTTP 请求不会被重启或修改。若回答先结束，召回继续运行并保存到原用户消息，后续对话可读取；停止回复或删除会话会取消该会话的召回。召回限时 30 秒，失败不影响对话。只把选中的标题、摘要、ID 和相关原因作为历史参考加入上下文，模型通过 `memory_read` 按需读取全文。对话中可使用 `memory_search`、`memory_save`、`memory_delete`；写入、删除遵循权限选择器。

`memory_read` 中夜间归档记忆的来源包含 `session_id` 和 `evidence`（从 0 开始的消息序号）。模型可调用只读工具 `session_read({session_id, around_message: evidence序号})` 查看来源附近的对话，或用 `start_message` 分页读取。默认 20 条，最多 50 条、16000 个内容字符；长消息通过 `next` 中的 `start_message/start_offset` 无损续读。`include_tools: true` 可包含原始工具调用和结果；图片仅返回数量，不读取草稿或思考内容。只允许读取当前项目会话，完全访问权限也不会放宽此边界。已删除的会话无法追溯。

只读工具 `session_changes({})` 默认汇总当前请求所属会话的文件写入；指定 `session_id` 可查看同项目中有权读取的历史会话。`session_changes({path:"src/main.rs"})` 返回该文件的逐次记录，使用返回的 `next` 继续分页；`session_changes({change_id:"…",offset:0,chars:4000})` 按 Unicode 字符分页读取写入前后的内容，`before: null` 表示此前文件不存在。结果同时包含本轮已完成但尚未落盘的工具写入，子代理记录会标明来源；项目和伙伴隔离不会因完全访问权限放宽。只包含 `write_file` / `replace_in_file` 捕获的变更，命令、脚本及外部编辑不在记录中；工具不会自动恢复或改写文件。

会话变更侧栏支持逐文件“撤销”和“全部撤销”。恢复该会话第一次写入前的内容，新建文件则删除；执行前核对当前文件是否仍为记录中的修改后内容，发现冲突，或两次写入之间存在未记录修改时会停止。撤销采用同目录临时文件原子替换，保留当前文件权限位；记录与撤销标记随会话保存，`session_changes` 通过 `reverted` 标识已撤销的历史记录。模型执行或有排队请求时暂不可撤销，Git 工作区视图不提供会话撤销，以保留原有工作区改动。



每天默认本地时间 23:00，应用运行且对话空闲时使用当前模型整理已有对话，错过时间在下次运行补处理。仅提取长期偏好、事实、项目决定和未完成目标，保存来源消息与进度，不删除原始会话。每轮最多 16 个批次，剩余内容留待下一轮或“立即整理”；失败保留进度，自动重试每次至少间隔 10 分钟，每个调度日最多失败 3 次。应用退出或任务中断后通过租约恢复。自动整理和语义回忆会使用供应商额度，可分别关闭。

运行时发送按钮切换为圆形停止按钮，点击终止当前回复；不再额外显示“停止回复”按钮。共享 Composer 通过 `stop_on_busy(true)` 启用，默认保持原有忙碌行为。

## 技能（Skills）

支持 [Agent Skills 的 SKILL.md 格式](https://agentskills.io/specification)。打开输入框“+”或输入独立的 `@`，在“技能”分组中选择当前项目可用的技能，在 `@` 后继续输入时，面板按名称和描述即时过滤（不区分大小写）。选择后提示词中插入高亮的 `$技能名`，替换整段 `@关键词`，保留已有文字、后续内容和图片。恢复草稿后，当前可用技能标记也会高亮。面板还提供“管理技能”，可进入“设置 → 技能”查看指令、搜索、启停、刷新或添加/移除自定义扫描目录。切换项目会清除旧的候选，重新打开面板时在后台刷新，不在 hover 或渲染时读取文件。

默认按顺序发现：当前项目 `.agents/skills`、`.glint/skills`、`.codex/skills`，手动添加的目录，Agent 数据目录下的 `skills/`，以及用户的 `~/.agents/skills`、`~/.codex/skills`（Windows 使用 USERPROFILE）。同名技能使用最先发现的版本；停用项目版本不会悄悄退回用户版本。自定义目录既可直接选择一个技能包，也可以选择包含技能包的目录，最多向下扫描两层，跳过隐藏目录。最多发现 256 个技能、检查 2048 个目录。

```text
.agents/skills/code-review/
├── SKILL.md
├── references/checklist.md
└── scripts/check.py
```

```markdown
---
name: code-review
description: Review code changes for bugs, regressions and missing validation.
---
Read the diff first. See references/checklist.md for review criteria.
```

先向模型提供已启用技能的名称和描述（最多 40 个，显式指定的优先），不提前把指令全文塞进上下文。模型可用 `skill_list` 搜索和分页查询，再用 `skill_read({name:"code-review"})` 读取指令。`skill_read` 还支持包内的 `path`、`start_line` 和 `lines`，返回 `next_line` 提示继续读取。资源相对技能包根目录解析，不是相对项目目录；禁止 `..`、绝对路径及逃逸到包外的符号链接。SKILL.md 最大 64KB、元数据 16KB、引用文本 256KB，单次返回最多 500 行和 64KB。

目录和启停设置存入现有 SQLite。每次请求捕获技能目录快照；运行中停用会阻止之后的技能读取，指令文件在请求期间改变则要求重新发送以加载新版本。已经进入对话历史的内容不会自动删除。`allowed-tools` 不会授予额外权限，脚本不会自动执行；如需执行，模型仍通过 `run_command`，按输入框当前审批模式处理。首版不包含在线技能市场、自动下载依赖或为技能凭据授权；需要宿主尚未提供的工具时，应说明缺失能力。

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

## MCP

支持 MCP stdio 子进程和 Streamable HTTP，使用官方 Rust SDK [rmcp](https://github.com/modelcontextprotocol/rust-sdk)。在 Agent 数据目录（与 `sessions.json` 同级）创建 `mcp.json`；macOS 默认是 `~/Library/Application Support/Glint Agent/mcp.json`，也可用 `GLINT_AGENT_DATA_DIR` 指定目录。

```json
{
  "mcpServers": {
    "local": {
      "command": "/absolute/path/to/mcp-server",
      "args": ["--example"],
      "env": { "EXAMPLE_SETTING": "value" },
      "enabled": true
    },
    "remote": {
      "url": "http://localhost:8000/mcp",
      "enabled": false
    }
  }
}
```

每个服务器只能配置 `command` 或 `url`。配置在下次发送消息时读取，无需重启；stdio 的工作目录为当前项目，Finder 启动时建议使用可执行文件绝对路径。每次请求独立连接并发现工具，支持工具列表分页；兼容 OpenAI、Anthropic 和 ChatGPT。工具名称在模型侧映射为 `mcp_数字_数字`，描述保留服务器和原始工具名，避免名称冲突。

MCP 调用复用工具卡片、记录和权限选择器；“请求批准”对所有 MCP 工具逐次确认，“自动批准”和“完全访问权限”直接调用。服务器进程启动及工具发现发生在调用批准前，配置中只应启用信任的服务器。MCP 服务器自身的文件、网络访问权限由服务器控制，项目文件边界不约束外部服务器。连接/发现超时 30 秒，工具调用超时 60 秒；服务器连接失败时，本次请求继续使用内置工具，模型会收到错误信息并可修复配置。

Agent 系统上下文包含实际配置路径和格式，可直接要求它配置 MCP。内置 `mcp_config_read` / `mcp_config_write` 仅访问该配置文件，写入在默认模式请求批准，无需开启完全访问权限；写入时校验结构并原子替换。修改在下次用户请求生效，保存成功不代表连接已验证。

“设置 → MCP”可添加、编辑、启停或删除服务器；支持本地程序、JSON 参数与环境变量、远程 URL。点击“检查连接”可检查未保存的配置并查看服务器工具列表，检查不会保存或启用服务器。环境变量以明文配置保存。

目前接入工具能力；暂未提供 OAuth 登录、资源和提示模板浏览器或旧版 HTTP SSE transport。远程示例适用于无需认证的服务，配置文件中的 `env` 为明文，勿提交含密钥的配置。

## Code Mode

`code_mode` 接收 JavaScript 异步函数体，可在一次模型往返中使用 `await tools.工具名(参数)` 编排调用，支持条件、循环和 `Promise.all`。只读调用最多并行 4 个，写入和审批调用为顺序屏障；现有工具、权限、文件变更和撤销流程继续适用。脚本在独立 Boa 进程执行，无需 Node，模块加载及系统绑定禁用；模型只收到脚本最终返回的必要数据，UI 可展开查看脚本及子调用。

单批最多 64 次调用、代码/最终结果各 64KiB，默认 120 秒。失败即停，已完成副作用保留，不自动回滚或整批重放；已启动的后台命令需按 `run_id` 单独停止。[设计、边界及示例](../../docs/code-mode.md)。
