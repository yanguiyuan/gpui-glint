# Glint API

独立的 GPUI 原生 API 工作台，位于 workspace 的 `crates/glint-api`，通过路径依赖使用根目录的 `gpui-glint` 组件库。应用版本与库版本独立管理，不发布到 crates.io。

在仓库根目录运行：

```sh
cargo run -p glint-api
```

默认启用 HTML 预览；需要关闭时使用 `cargo run -p glint-api --no-default-features`。

## Windows 安装包与发布

Windows 10/11 x64 的安装包由 Inno Setup 6 生成，安装到 `%LOCALAPPDATA%\Programs\Glint API`，无需管理员权限。安装自动创建桌面和开始菜单快捷方式，EXE、安装包和快捷方式使用应用图标；卸载移除程序与快捷方式，保留工作区数据库。缺少 WebView2 时安装包会通过微软签名的 bootstrapper 下载并安装运行时，此步骤需要联网。

安装 Rust MSVC 工具链、Visual Studio C++ Build Tools（含 Windows SDK）和 Inno Setup 6 后，在仓库根目录使用 PowerShell 7：

```powershell
pwsh -File crates/glint-api/packaging/windows/build-windows.ps1
```

输出为 `target/windows-installer/output/Glint-API-<版本>-windows-x64-setup.exe` 及 `.sha256` 校验文件。脚本使用 release 构建和静态 MSVC 运行时；发布版不弹出控制台。安装包目前未做代码签名。

静默安装：

```powershell
.\Glint-API-0.1.1-windows-x64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
```

更新本应用 `Cargo.toml` 的版本后，推送对应标签自动构建、验证静默安装／重装升级／快捷方式／图标／卸载，再发布到 GitHub Releases：

```sh
git tag glint-api-v0.1.1
git push origin glint-api-v0.1.1
```

手动运行 `Glint API Windows Release` workflow 只构建并上传 Actions artifact，不发布 Release。发布 workflow 检查标签与应用版本一致；库使用原来的 `v…` 标签，应用使用 `glint-api-v…` 标签。Windows CI 的安装测试不验证实际 GPU 窗口和 HTML 预览渲染。

## 更新检查与自动更新

Windows 发布版启动时自动检查稳定的 `glint-api-v…` Release；标题栏更新图标也可手动检查。只按应用的语义版本比较，排除库版本、草稿和预发布版本，不降级。发现新版后点击“更新并重启”：后台下载并显示进度，核对 GitHub 资产 API 提供的文件大小和 SHA-256，再保存请求工作区、退出旧进程、静默安装到当前目录并重新打开。下载可以取消；网络失败、限流或校验失败可以重试。临时下载和超过一天的旧更新缓存会清理。

自动安装适用于通过 setup 安装的 Windows x64 发布版。其他平台和开发构建提供“查看发布”。执行中的请求、未保存的环境设置、未完成的名称编辑或 SQLite 保存错误会阻止重启，处理后可继续安装已下载的更新。更新助手在安装前再次校验安装包，安装失败时会尝试重新打开原应用并显示错误。

`0.1.0` 尚无更新器，首次需要手动安装带更新器的版本；之后可应用内更新。发布源为公开的 `yanguiyuan/gpui-glint`，默认检查和下载更新无需 GitHub 登录或令牌。如需提高 GitHub API 限流额度，可选用进程环境变量 `GLINT_API_UPDATE_TOKEN` 提供令牌；令牌不存入数据库、不内嵌到安装包，仅用于 GitHub API。

命令行检查（不打开 UI）：

```sh
cargo run -p glint-api -- --check-update
```

Windows CI 验证从 `0.1.0` 升级到当前版本、拒绝错误校验值、等待旧进程退出及数据保留，不验证实际 GPU 窗口的重启渲染。联网下载测试独立于默认测试，需要访问发布源：

```sh
cargo test -p glint-api downloads_published_installer_using_the_asset_api -- --ignored
```

应用图标位于 [`assets/glint-api-icon.png`](assets/glint-api-icon.png)：纯蓝圆角底座、白色代码括号与请求／响应连接线，采用简约平面设计。可编辑 SVG 源文件为 [`assets/glint-api-icon.svg`](assets/glint-api-icon.svg)，PNG 外围透明；macOS 直接使用 Cargo 启动时会自动设置 Dock 图标。设计说明见 [`assets/icon-design.md`](assets/icon-design.md)。

窗口采用参考稿的玻璃外壳、图标导航、请求集合、请求编辑器、响应区及右侧版本 / 请求历史。主题选择器仅提供月光白（`Moonstone`）和曜石黑（`DarkObsidian`），默认窗口 1440 × 900，最小 1050 × 700；右侧记录栏可收起。

- 标签栏复用 `ClosableTabs`，使用 `on_add` 在末尾显示加号，点击打开请求选择器。
- 请求标题栏加号旁的更多菜单支持“创建集合”，输入名称并确认后显示空集合；空名称和重复集合名称会显示提示。点击集合标题切换展开，顶部加号在当前选中的集合或目录内新建请求。筛选支持集合、目录和请求名称；集合与目录独立管理并持久化到 SQLite。菜单支持点击外部或 Esc 关闭。
- 左侧是一棵目录树：至少先创建一个集合，请求可直接放在集合下；目录内可以继续创建请求和子目录，没有固定层级限制。收起／展开状态分别用 `Icon::Folder` 和 `Icon::FolderOpen` 表示，不使用箭头。各节点独立收起，打开请求会展开其父级路径；筛选显示匹配项及其祖先，清空后恢复原收起状态。深层目录支持横向滚动。
- 悬停集合或目录行只显示一个“更多”入口；下拉菜单提供新建请求、新建目录，普通目录另有重命名目录和删除目录。打开菜单不会切换收起状态，点击外部或 Esc 关闭。目录名称在同级内唯一；重命名同步修改整棵子树，删除目录将子树内请求保留到上级，保留草稿、版本、历史与打开标签。
- 侧栏使用通用 `TreeList` 与 `TreeListItem`；拖到目录中部移入该目录，拖到节点上／下边缘调整同级位置，集合可在顶层排序。支持跨集合移动目录子树和请求，循环嵌套或目标目录重名会被拒绝，位置写入 SQLite。
- 旧数据库会将普通分组迁移为目录，将“未分组”和自动创建的“自定义”请求移到集合根部；保存后使用数据库版本 2，旧版应用会拒绝打开以保护新结构。
- 首次启动没有预置集合、请求或环境；创建集合后可筛选、打开 / 关闭标签、新建空白请求；悬停请求行可删除请求，同时移除其标签、版本与历史。删除最后一个请求后展示可新建请求的空状态。
- 点击请求编辑区顶部的请求名进行重命名，悬停显示铅笔提示；Enter 或勾选按钮保存，Esc 或取消按钮放弃。名称不能为空，保存后同步侧栏、标签和命令搜索，并自动写入 SQLite；切换或删除请求会取消尚未提交的名称编辑。
- HTTP 方法和 URL 复用 `SelectField` 组合输入；环境使用独立 `Select`。各请求独立保留草稿。方法、URL、环境、参数、请求头、Bearer Token 引用和 JSON 请求体可编辑。
- 请求头页增加 Cookie 覆盖表格，支持启用、添加、删除和编辑键、值、说明；发送时解析原始 Cookie 请求头并合并，同名 Cookie 使用覆盖表格的值，禁用条目不参与覆盖。覆盖配置随草稿、版本与请求历史保存。
- 参数支持启用、添加、删除；发送前替换 `{{base_url}}` 并编码已启用的查询参数。
- 版本历史使用圆点与连线时间线，显示当前／基线标签、版本时间及启用参数摘要；点击旧版本恢复时同步移动当前标记。
- 新请求历史保存当次响应、响应头、请求预览和执行轨迹，点击记录时同步恢复请求与响应面板。旧历史缺少响应快照时会清空响应并显示提示。响应头名称和值支持只读选中复制和长文本横向滚动；请求区与响应区之间使用 `Separator`。
- 请求设置提供“跳过 TLS 证书校验”，默认关闭；开启后允许自签名、过期或域名不匹配的 HTTPS 证书，仅影响当前请求。设置随草稿、版本和请求历史保存，旧配置默认保留正常校验。
- 环境设置支持 HTTP / HTTPS 代理地址、启用代理和跳过 TLS 证书校验。通过“保存更改”统一校验变量与代理设置，未保存的编辑不参与发送；启用后，使用该环境的所有 HTTP / HTTPS 请求走对应代理，连接失败不会回退直连。代理的 TLS 选项同时作用于目标服务和 HTTPS 代理，关闭代理后恢复当前请求的 TLS 配置；不使用环境时直接连接。
- 保存时仅在请求配置与最新版本不同时生成完整版本快照，重复保存或修改后撤销不会新增版本；自动持久化草稿不影响此判断。点击版本或请求历史恢复配置。请求历史按日期从新到旧分组，显示状态标签、方法与路径、精确时间、实际耗时和发送时的环境，可按成功／失败和路径筛选。
- 响应栏提供独立的“请求”Tab，使用只读代码面板展示最近一次发送时的HTTP 请求快照：最终 URL、启用的请求头、身份验证生成的 Authorization、合并后的 Cookie 和实际参与发送的请求体，支持复制。快照按请求保留，后续编辑草稿不会改变快照；快照使用已解析的环境变量，与本次发送配置一致。
- 响应栏提供独立的“轨迹”Tab，可在发送中实时查看并独立滚动。每个请求保留最近一次的准备、校验、等待、响应、失败和取消记录，时间为相对本次发送的实际耗时；不模拟 DNS／TLS 等网络阶段。
- 响应代码使用 `CodeCard::viewport()` 虚拟列表，仅排版可见行；源码 / 原始切换复用相同内容的缓存，预览 / 源码切换复用原生 WebView，纯 UI 通知不再序列化 SQLite 工作区。
- 响应支持 JSON / HTML / CSS / JavaScript / XML 高亮、源码 / 原始视图、响应头、内置检查结果及复制。HTML 响应增加“预览”，运行 `cargo run -p glint-api --features webview` 启用 Wry 原生渲染（macOS / Windows 适配，macOS 验证）。预览使用实际响应正文，展示静态 HTML / CSS，不执行脚本；未启用 feature 时提供运行命令提示。
- 环境管理使用独立详情页，变量复用 `DataTable` 展示；表格内直接编辑名称、值和类型，支持添加行、名称搜索、复制引用和删除确认。通过“保存更改”统一校验并提交，“放弃更改”恢复已保存配置；未保存的编辑不参与请求。每个环境保留独立草稿。敏感值默认隐藏，点击眼睛后编辑。名称须以英文字母或下划线开头，只含英文字母、数字、下划线，同一环境不能重名。浏览环境不会更改当前请求，点击“用于当前请求”应用。支持明暗主题。

| 快捷键 | 操作 |
| --- | --- |
| ⌘ / Ctrl + Enter | 发送 / 取消 HTTP 请求 |
| ⌘ / Ctrl + S | 保存，有修改时生成新版本 |
| ⌘ / Ctrl + K | 搜索命令与请求 |

点击发送执行真实 HTTP / HTTPS 请求，支持 GET / POST / PUT / PATCH / DELETE。使用 `{{变量名}}` 引用当前环境中的任意变量，发送前替换 URL、参数、请求头、Cookie 和请求体中的引用；变量值也可引用其他变量。未定义、空值、循环引用或嵌套过深会在发送前报错。Bearer 可直接输入 Token，也可引用 `{{api_token}}`；No Auth 不生成 Authorization。新请求不预填 URL、认证、参数或请求体，请输入实际服务地址。

HTTP 客户端使用 reqwest + Tokio，网络执行在独立运行时中，不阻塞 GPUI。连接超时 10 秒、总超时 30 秒，响应内容最多 16 MiB；保持正常 TLS 校验，3xx 响应直接展示，不自动跳转。取消、切换请求、删除请求或恢复配置会中断正在执行的请求和响应读取；服务端已经收到的操作无法撤回。

响应展示真实状态码、响应头（保留重复项）、耗时和解压后字节数。JSON 支持格式化；原始视图保留响应文本，纯文本、HTML 和空响应也可查看。文本按 Content-Type 的 charset 解码，未指定时使用 UTF-8；二进制响应暂以文本显示，不提供文件保存。4xx / 5xx 仍展示服务端响应体，连接失败和超时进入失败历史。检查页使用实际状态码和 JSON 解析结果，不执行用户脚本。轨迹记录准备、校验、发送、完成、失败和取消，不生成虚构的 DNS / TLS 阶段。

响应体复制按钮旁的搜索图标展开搜索栏，不区分大小写，显示当前匹配序号与总数。上一处／下一处按钮或 Shift+Enter／Enter 循环跳转并滚动到匹配位置，Esc 关闭并清除高亮。搜索使用当前源码或格式化／原始文本；从 HTML 预览打开搜索会切回源码。切换响应或显示格式后自动刷新匹配结果。
数据使用本机 SQLite 存储，包含集合、嵌套目录、请求草稿、版本、发送历史、最近响应、请求快照、执行轨迹和已保存的环境变量。请求编辑自动保存；环境变量仍通过“保存更改”提交，未保存编辑不落盘。环境侧栏支持创建具名空环境；“不使用环境”是空变量上下文，不是预置环境。数据库使用事务写入，数据库读取或版本校验失败会停止启动并保留原文件，写入失败在界面显示错误。

URL、Token、参数、请求头和 Cookie 的键值输入在失焦时以徽标显示已保存环境变量的解析值；点击徽标编辑原始 `{{变量名}}` 表达式。未定义、空值、循环引用及未闭合表达式使用警告色，悬停查看表达式和失败原因。切换请求环境或保存环境变量后即时刷新；保存和发送仍使用原始表达式及相同的环境解析规则。请求体保持源码编辑。

macOS 默认数据库路径为 `~/Library/Application Support/Glint API/workspace.sqlite3`；Windows 使用 `%LOCALAPPDATA%\Glint API\workspace.sqlite3`，`LOCALAPPDATA` 未设置或为空时回退到 `%USERPROFILE%\AppData\Local\Glint API\workspace.sqlite3`，不依赖 `HOME`；其他平台使用 `$XDG_DATA_HOME/glint-api/workspace.sqlite3`，未设置时为 `~/.local/share/glint-api/workspace.sqlite3`。所有平台均可通过环境变量 `GLINT_API_DB_PATH` 指定数据库路径，优先于默认路径，父目录会自动创建。Windows PowerShell 示例：

```powershell
$env:GLINT_API_DB_PATH = "$env:LOCALAPPDATA\Glint API\workspace.sqlite3"
```

敏感变量、请求认证和响应与其他数据一起存储，数据库没有加密；Unix 数据库文件权限为 `0600`。首次启动不生成示例数据，删除后的数据不会在重启时重新注入。

验证：

```sh
cargo check --workspace --all-targets --all-features
cargo test -p glint-api
```

模板方法可用于 URL、参数、请求头、Cookie 覆盖、Bearer Token、请求体和环境变量值。环境设置页提供点击复制的示例；不使用环境也能使用内置方法。

| 表达式 | 结果 |
| --- | --- |
| `{{random_int(1, 100)}}` | 包含上下界的随机整数；无参数默认 0–100 |
| `{{uuid()}}` / `{{random_id()}}` | UUID v4 |
| `{{random_string(16)}}` | 英文字母与数字组成的字符串；默认 16 字符，长度 1–4096 |
| `{{random_name()}}` | 随机英文测试姓名 |
| `{{xss()}}` / `{{xss('script')}}` | `<script>alert('glint-xss')</script>` |
| `{{xss('img')}}` / `{{xss('svg')}}` | 使用 onerror / onload 的 XSS 测试文本 |
| `{{now()}}` | UTC ISO 8601 时间，含毫秒 |
| `{{now('%Y-%m-%d %H:%M:%S')}}` | 按格式生成 UTC 时间 |
| `{{timestamp()}}` / `{{timestamp_ms()}}` | Unix 秒 / 毫秒时间戳 |
| `{{cookie('csrftoken')}}` | 当前请求中名为 csrftoken 的 Cookie 值 |

参数支持单引号或双引号。同次发送中，相同方法表达式（去除两端空白后）复用结果，所有时间方法使用同一时刻；再次发送会重新生成。需要不同随机值时，可使用不同参数，如 `random_string(16)` 和 `random_string(24)`。编辑徽标中的随机值是预览，实际发送时重新生成；响应区的“请求”快照展示实际值。方法不自动进行 JSON 转义，查询参数仍由参数表自动编码。

Cookie 提取区分名称大小写，读取已启用的 Cookie 请求头并应用 Cookie 覆盖项，同名覆盖项优先。支持 Cookie 值中的环境变量和方法引用；缺失 Cookie、循环引用、非法参数或时间格式均在发送前报错。这里只读取当前请求配置，不读取响应 Set-Cookie 或浏览器 Cookie。

## 请求插件

导航栏提供独立“插件管理”页面，负责应用级安装、卸载和插件源配置。请求配置的“前置动作”页只展示注册后的动作卡片，通过 Switch 启用或禁用，拖动左侧手柄调整执行顺序；所需环境变量显示在卡片下方。来源、版本与权限声明在独立插件管理页查看。批准的代码指纹和权限会随草稿、版本、历史和 SQLite 工作区保存；刷新或升级导致代码／权限变化时，需要重新授权。旧工作区默认不启用任何插件。

内置 `WPS KSO-1 签名`：在当前环境中保存 `wps_app_id` 和 `wps_app_key`，将后者标记为敏感，再在请求的前置动作页启用。它可以与 Bearer 同时使用。算法只存在于 `plugins/wps-kso1/main.js`，不写入公共运行时；使用实际编码后的 URI、原始发送 body 和最终 Content-Type，空 body 的摘要为空字符串。

自定义插件放在工作区 SQLite 文件旁的 `plugins/<插件目录>/`，包含 `glint-plugin.json` 和 `main.js`，在插件管理页点击“刷新已安装插件”重新发现，再在请求中显式启用。可从内置模板复制，使用不同的插件 ID。通过 `contributions` 注册 `request.before-send` 类型动作，每个动作独立指定 ID、标题和同步 hook；不接受页面贡献；未安装或权限无效的已启用插件会阻止发送。

环境读取需要逐变量声明 `environment.read.<变量名>`；请求头修改需要 `request.header.<小写请求头名>`；读取请求需 `request.read`，加密需 `crypto`。Rust 宿主拒绝未声明变量、未授权请求头、Host/Content-Length 等传输头、CRLF 和过大输出。钩子接收 `{ method, uri, body, headers }`，返回 `{ setHeaders: { "Header": "value" } }`；不能修改 URL、方法或 body。需变更 body 的算法应在未来增加独立的准备阶段钩子，不能在签名后偷偷修改正文。

辅助进程通过 stdin/stdout 传递有限 JSON 数据，三秒未完成就终止，请求不发送；不会把密钥放进命令行或脚本错误消息。主界面不执行脚本。此机制不是 OS 权限沙箱或总内存硬限制。详细脚本异常不展示，以防异常文本泄露密钥。签名后的请求预览隐藏 Authorization、X-Kso-Authorization 和 Cookie。

通用插件契约与原生页面组件见根目录 `docs/plugins.md`。第一版未实现集合继承、在线插件市场、ES 模块依赖、热重载或异步钩子。

### 远程插件源

独立插件管理页支持添加多个远程源（名称、HTTPS 地址、可信公钥）、获取已验签的兼容版本列表并安装。安装后需要单独启用。已安装版本按源公钥身份隔离；同一 ID 的不同源不会覆盖，旧版本保留用于显式切换。共享 CLI 时，`--home` 指向工作区旁的 `plugin-registry/`。

自部署服务、发布 CLI 和协议见根目录 [插件仓库说明](../../docs/plugin-registry.md)。当前服务提供管理 API 与 CLI，尚无网页管理后台；客户端不保存发布令牌。
