# Glint API

独立的 GPUI 原生 API 工作台，位于 workspace 的 `crates/glint-api`，通过路径依赖使用根目录的 `gpui-glint` 组件库。应用版本与库版本独立管理，不发布到 crates.io。

在仓库根目录运行：

```sh
cargo run -p glint-api
```

默认启用 HTML 预览；需要关闭时使用 `cargo run -p glint-api --no-default-features`。

## 工作区导入与导出

点击左上角工作区名称，在工作区弹窗中选择“导出当前工作区”或“导入工作区”；也可通过主窗口的命令面板（Ctrl/Cmd K）搜索这两个操作，支持 import、export、workspace、迁移、备份等关键词。迁移内容默认全选，可分别选择全部集合与请求、集合/目录设置、全局和集合环境变量、代理配置、版本、请求历史与响应、MCP 操作记录，以及当前环境和更新源设置。版本与请求历史需要同时选择集合与请求；单独迁移集合设置或集合变量时会保留必要的集合/目录结构。导出包含未保存的请求草稿、有效的环境变量和代理编辑内容。

文件为带格式版本的 `.glint-workspace.json`，导入后先查看文件摘要并选择内容，再填写新工作区名称。导入创建独立工作区并切换，不覆盖原有工作区；同名时自动建议新名称，写入使用 SQLite 事务。运行中的请求或 WS 需要结束后才能执行导入切换。文件最大 256 MiB，可取消历史与响应以减小体积。

迁移文件保留请求和环境变量中的 Token、密钥原值与敏感标记，请妥善保管。OAuth 系统凭据、全局安装的插件、本机 CA 证书以及运行中的连接不包含在工作区文件中；新设备上需要重新授权、安装相应插件和配置证书。导入不会自动发送请求、执行插件或启动代理监听。

## 环境变量有效期与规则触发器

在 Token 请求的“后置动作”中配置变量名 `access_token`、JSONPath `$.access_token` 和有效期 JSONPath `$.expires_in`。有效期必须匹配一个非负整数，单位为秒，也支持数字字符串；留空表示不过期。成功提取时保存绝对过期时间，环境变量表可查看该时间。提取失败不会写入部分变量；修改变量值会清除旧有效期，保存未修改的值则保留有效期。

在业务请求的“前置动作 → 规则触发器”中，配置条件“变量缺失或过期”、变量名 `access_token`，动作选择“执行集合请求”，目标选择同集合的 Token 请求。发送前按行检查条件，执行命中的动作及目标请求的后置提取，再使用更新后的变量解析当前请求。动作失败会停止发送；循环调用会报错。触发器按发送时的环境与集合作用域运行，集合变量优先于全局变量；这是发送前检查，不是后台定时刷新。

触发器分别保存 `condition` 和 `action`，由独立条件求值与动作执行接口组合，后续可分别扩展条件和动作类型。首版目标支持普通 HTTP 请求，暂不支持目标请求的 OAuth 自动授权和 WebSocket。独立请求窗口共享同集合请求列表及提取结果。

## 版本历史与对比

每个版本显示请求配置的 16 位 SHA-256 短 hash。保存时检查当前请求的全部历史版本；有效参数相同会弹窗提示，可取消或切换到已有版本，不会新增重复快照。旧记录即时计算 hash，无需迁移。短 hash 命中后还会比较完整的规范化配置，避免碰撞误判。

hash 使用未展开的模板配置，覆盖地址、方法、参数、请求头、Cookie、鉴权、请求体、动作和请求设置；不包含环境运行值、时间、版本标签或响应。默认字段、未启用的参数/动作、空白占位行、未启用的 Token 和非当前格式的请求体内容不参与计算。参数及动作的顺序保留。新增可选字段须在 `Draft`、`Pair`、`BeforeAction` 或 OAuth 配置的 `Default` 中提供兼容默认值，保留已有字段名和默认语义；不启用新功能时 hash 保持一致。弃用字段应解析为默认或未启用状态。

点击版本行中的“对比”打开独立只读窗口，默认比较该版本与当前请求（包含未保存修改）。窗口内可选择任意两个版本，按配置项查看行级、词级差异，并切换显示未修改项。对比使用打开时的完整快照，包含未启用项；主窗口后续编辑、切换工作区或关闭请求不会改变已打开的对比。

MCP 的 `request_create`、`curl_import` 会保存初始版本；`request_update` 修改草稿、`plugin_bind` 和 `plugin_bindings_reorder` 修改动作后自动保存并选中版本，无需再调用 `version_save`。新版本带独立的 `MCP` 来源标记（`source: "mcp"`），不会占用自定义标签。参数与已有版本相同时自动复用，保留该版本的来源和标签，不弹窗、不产生重复版本。返回值包含 `saved`、`version_created` 和版本摘要；仅修改请求名称不新增版本。`version_save` 仍可显式保存当前草稿，它的 `tag` 仅作用于新建版本，已有版本请使用 `record_tag` 修改标签。页面以选中版本的有效参数判断未保存状态。

## 端口代理、浏览器与抓包

代理管理分为“代理设置”和“代理抓包”两个 tab，设置面板内部滚动。域名映射表仍独立滚动，底部添加入口固定可见。每个代理配置可手动启动一个 HTTP/HTTPS 正向代理监听服务，默认仅监听 `127.0.0.1:8899`，可修改端口；启用“允许局域网访问”后监听 `0.0.0.0`，外部设备可将 HTTP 和 HTTPS 代理配置为这台机器的局域网 IP 与端口。服务不要求代理账号；应用退出、工作区切换或删除对应代理时会关闭监听和连接，重启应用不会自动开放端口。

监听服务使用启动时的配置快照，复用当前代理的域名映射或外部上游代理；修改配置后需要停止并重新启动服务。外部客户端可使用 `curl --proxy http://127.0.0.1:8899 http://example.com/`，HTTPS 使用 `curl --proxy http://127.0.0.1:8899 --cacert glint-proxy-ca.pem https://example.com/`。

“打开应用”下拉菜单自动检测 Chrome、Edge、Firefox 和 Brave，为所选浏览器创建独立配置并设置 HTTP/HTTPS 代理，包含本机地址，关闭 QUIC/HTTP3 以便请求通过代理。不会改动日常浏览器配置或系统全局代理。再次点击可打开另一个独立窗口；浏览器配置保存在应用数据目录的 `proxy-service/browser-sessions`。

首次启动监听服务自动生成本地 CA，也可在证书区域生成、导入、导出或安装。导入支持一个合并 PEM 文件或 CA 证书与未加密私钥两个 PEM 文件，校验证书有效期、CA 用途以及私钥匹配；导入替换前需停止全部代理监听。导出 `.pem` 为 PEM，其他扩展名为 DER，均只包含公开 CA 证书；私钥独立保存在数据库同目录的 `proxy-service/ca/identity.pem`，不写入工作区。Windows 的安装操作写入当前用户受信任根证书，macOS 写入登录钥匙串；Linux 按提示将导出的 CA 手动导入系统或浏览器。其他设备也需安装这个 CA 才能解密 HTTPS；证书固定的网站可能拒绝连接。证书导入或安装后请重新打开代理浏览器窗口。

抓包 tab 按当前代理显示 HTTP/HTTPS 请求、状态、耗时、响应大小、请求/响应头和正文，支持关键词过滤和高亮。转发采用流式读写，SSE、下载不会等待整体完成；WebSocket 升级后保持双向转发，抓包记录展示握手。仅在内存保留最近 1000 条记录，请求和响应各预览前 32 KiB，超出部分标记截断；清空记录不打断请求，停止监听后保留记录以供查看。

相关验证：`cargo test -p glint-api --no-default-features proxy_`。

## macOS 安装包

在 macOS 上安装 Rust 和 Xcode Command Line Tools 后，运行：

```sh
bash crates/glint-api/packaging/macos/build-macos.sh
```

脚本构建本机架构的 release 版本，默认包含 HTML 预览，生成带图标的 `Glint API.app`、可拖入 Applications 安装的 DMG 和 SHA-256 校验文件，输出到 `target/macos-installer/output/`。Apple Silicon 文件名为 `Glint-API-<版本>-macos-arm64.dmg`，Intel 主机输出 `macos-x86_64.dmg`；不是 Universal 通用包。最低系统版本从实际 Mach-O 可执行文件读取。

默认使用本地 ad-hoc 签名，并验证签名、磁盘镜像、版本号和系统库依赖。可通过 `GLINT_API_CODESIGN_IDENTITY` 指定本机已有的 Developer ID 签名证书；脚本不执行 Apple 公证。应用替换和移除不会删除已有工作区。安装后，MCP 命令入口是 `/Applications/Glint API.app/Contents/MacOS/glint-api --mcp`，也可从应用内复制连接配置。

## Windows 安装包与发布

Windows 10/11 x64 的安装包由 Inno Setup 6 生成，安装到 `%LOCALAPPDATA%\Programs\Glint API`，无需管理员权限。安装自动创建桌面和开始菜单快捷方式，EXE、安装包和快捷方式使用应用图标；卸载移除程序与快捷方式，保留工作区数据库。缺少 WebView2 时安装包会通过微软签名的 bootstrapper 下载并安装运行时，此步骤需要联网。

安装 Rust MSVC 工具链、Visual Studio C++ Build Tools（含 Windows SDK）和 Inno Setup 6 后，在仓库根目录使用系统自带的 Windows PowerShell 5.1（也支持 PowerShell 7）：

```powershell
powershell.exe -NoProfile -File packaging/windows/build-windows.ps1 -Product api
```

也可在仓库根目录通过 GNU Make 执行同一构建流程：

```sh
make win-installer PRODUCT=api
```

Windows 下 Make 默认调用 `powershell.exe`，无需安装 `pwsh`；如需使用 PowerShell 7，可执行 `make win-installer PRODUCT=api POWERSHELL="C:/path/to/pwsh.exe"`。脚本自动查找 PATH、当前用户安装目录和 Program Files 下的 Inno Setup 6。自定义安装位置可执行 `make win-installer PRODUCT=api ISCC="C:/path/to/Inno Setup 6/ISCC.exe"`，或直接给 PowerShell 脚本传入 `-IsccPath`。默认读取应用 `Cargo.toml` 的版本号；可用 `VERSION=0.1.6` 显式校验版本，须与清单一致。

已下载的 WebView2 bootstrapper 会复用，每次打包仍验证其微软数字签名；首次构建需要联网下载。需要重新下载时，可删除 `target/windows-installer/stage/MicrosoftEdgeWebview2Setup.exe` 后重试。

输出为 `target/windows-installer/output/Glint-API-<版本>-windows-x64-setup.exe` 及 `.sha256` 校验文件。脚本使用 release 构建和静态 MSVC 运行时；发布版不弹出控制台。安装包目前未做代码签名。

静默安装：

```powershell
.\Glint-API-0.1.6-windows-x64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
```

更新本应用 `Cargo.toml` 的版本后，推送对应标签自动构建、验证静默安装／重装升级／快捷方式／图标／卸载，再发布到 GitHub Releases：

```sh
git tag glint-api-v0.1.6
git push origin glint-api-v0.1.6
```

手动运行 `Glint API Windows Release` workflow 只构建并上传 Actions artifact，不发布 Release。发布 workflow 检查标签与应用版本一致；库使用原来的 `v…` 标签，应用使用 `glint-api-v…` 标签。Windows CI 的安装测试不验证实际 GPU 窗口和 HTML 预览渲染。

## Windows 更新与辅助进程

MCP 客户端通过 `glint-api.exe --mcp` 各自维护一个 stdio 代理，因此任务管理器中可能出现多个同名进程。插件执行器和更新助手也使用同一程序的辅助模式；请求预览子窗口不启动新进程。

自动更新先等待更新助手取得安装锁，再退出主窗口；安装期间阻止新的工作台、MCP 代理及插件执行器启动。主进程退出后，只关闭同一安装路径下的 MCP 和插件辅助进程。其他工作台实例会保留，并提示先关闭这些实例后重试。安装失败的提示包含安装日志路径（更新任务目录中的 `job.install.log`）。

## 请求多窗口编辑与预览

右键请求标签页，选择“在独立窗口中打开”，可在子窗口编辑、保存和发送 HTTP / WebSocket 请求。主窗口选中同一请求时，两边直接复用同一个请求编辑器与执行会话：地址、参数、鉴权、请求体、版本、响应及 WS 事件实时同步，MCP 修改也写入这一共享实例。重复打开同一请求只增加视图，不会重复建立连接。切到其他请求或关闭子窗口不终止该请求的执行；可从后台任务菜单返回，再次打开窗口仍是同一会话。删除请求、替换工作区或关闭主窗口会停止对应共享请求。

WS 消息栏也提供“独立窗口”按钮。可先连接 WS 并打开子窗口监听，再在主窗口或另一个请求窗口发送 HTTP 请求触发事件；WS 子窗口持续显示新事件，并可编辑发送消息、选择解密插件或断开连接。

选择“打开只读快照”仍可保留打开时的请求配置、未保存内容、最近发送报文和已有响应；WebSocket 请求展示已有消息快照，后续编辑不会改变它。

只读预览中的 URL、参数与请求头沿用主窗口的模板变量显示及解析规则，应用所选环境与请求所属集合的变量覆盖。JSON 响应支持对象和数组折叠，切换响应标签保留折叠状态，复制仍包含完整内容。只读预览不提供发送或保存操作。

## WebSocket

在地址栏方法中选择 `WS`，输入 `ws://` 或 `wss://` 地址，点击“连接”。连接后在“请求体”中编辑文本，点击“发送消息”（或按 `Cmd/Ctrl+Enter`）；消息中的环境变量在每次发送时解析。上方“断开”关闭当前连接，连接中可取消；切换请求会保留后台监听，加载其他版本会结束该请求的当前连接。

握手复用参数、Bearer、Cookie、自定义请求头、前置签名插件、环境代理和 TLS 校验设置。`Sec-WebSocket-Protocol` 可通过请求头指定；握手必需头由客户端管理，不支持压缩扩展。消息面板采用事件列表与详情分栏，按时间显示方向、摘要和大小；选中事件后独立查看消息体，JSON 自动格式化，支持对象和数组块级折叠及长行横向滚动；切换原文时自动换行，也可复制完整原始消息。默认跟随最新事件，手动选择后暂停跟随；二进制消息以 Base64 查看。消息仅保存在当前会话，最多保留 500 条、合计 8 MiB，支持清空；目前只发送文本消息，后置提取动作不作用于消息流。

请求配置照常保存到 SQLite；消息记录仅在当前会话保留，最多 500 条 / 8 MiB。单条收发消息上限为 1 MiB，详情保留完整消息。

WebSocket 消息新增“原始事件 / 解密后”独立标签。在“解密后”选择内置 **WPS 事件解密** 插件，当前环境配置 `wps_app_id`、`wps_app_key`（建议设为敏感变量）即可查看签名验证后的事件。配置按请求保存并参与版本恢复。插件按 [WPS 事件安全校验](https://365.kdocs.cn/3rd/open/documents/app-integration-dev/wps365/server/event-subscription/security-verification) 先校验 HMAC-SHA256（URL-safe Base64，无填充），再以应用密钥的 MD5 十六进制字符串作为 AES-256-CBC 密钥，使用 nonce 前 16 字节和 PKCS7 解密。失败仅显示错误，保留原始事件；可切换环境修正密钥后“重新解密”。解密结果仅保留在当前详情中，不写入 SQLite、请求历史或原始消息日志。插件处理不发送 ACK，也不改变握手签名与连接行为。

## SSE 流式响应

照常发送 HTTP 请求，响应头为 `Content-Type: text/event-stream` 时自动持续接收并实时显示原始事件流；需要时在请求头设置 `Accept: text/event-stream`。支持 GET 和 POST，沿用请求体、鉴权、Cookie、环境变量、签名插件、代理和 TLS 设置。事件中的 `data`、`event`、`id`、`retry` 与心跳注释均保留，UTF-8 跨网络分块正确解码。

连接期间可查看响应头、搜索或复制已收到的内容，点击“取消”断开并保留当前结果。SSE 收到响应头后没有总时长或空闲超时，普通 HTTP 仍保留 30 秒总超时，等待响应头也有 30 秒超时。流结束或出错后保留已接收内容；显示最近 1 MiB，字节数累计计算。界面最多约每 50 ms 刷新一次。

当前展示原始事件流，不自动重连、不自动合并模型输出，也不对每个事件执行后置提取动作；`[DONE]` 作为普通事件内容显示，连接由服务器关闭或手动取消。

## MCP / Agent 操作接口

提供 60 个 MCP 工具，直接操作正在运行的原生工作台，覆盖工作区、集合/目录、请求编辑与 HTTP/WS/SSE 执行、响应、版本/历史、环境、插件源、插件授权、界面和应用更新。请求和记录使用持久 ID，修改同步到 GUI 和 SQLite。

按 `Cmd/Ctrl+K` 搜索 **MCP**，执行“复制 MCP 连接配置”，或运行 `glint-api --mcp-config`。agent 通过 `glint-api --mcp` 使用标准 stdio；没有运行中的应用时自动打开工作台。本机桥接使用随机令牌，读取结果默认隐藏已知密钥字段。

侧边图标栏的 **MCP** 页面可按日期查看工具调用历史，支持搜索、结果筛选、分页和参数/结果详情。调用记录在保存前隐藏已知密钥，重启后保留。

完整配置、工具列表、参数示例和长连接操作说明见 [MCP.md](MCP.md)。

## 工作区

点击顶部工作区名称打开管理面板，可创建、切换、重命名工作区。每个工作区独立保存集合、目录配置、请求草稿、环境、版本、响应及请求历史和 MCP 调用记录；重启后恢复上次选择。新工作区从空白开始，插件安装和应用更新仍由应用统一管理。

旧数据自动归入“个人工作区”，保留已有请求与授权。切换前自动保存请求草稿和环境设置；请求、WebSocket 或 OAuth 授权进行中，以及目录或名称编辑尚未结束时，先完成当前操作再切换。保存失败时保留当前工作区。

删除其他工作区需要二次确认，并一并删除其中的数据和调用记录；先切换到其他工作区才能删除当前工作区。默认工作区可以重命名，始终保留。数据库升级到版本 3，旧版应用会拒绝打开。

## 更新检查与自动更新

Windows 发布版启动时自动检查稳定的 `glint-api-v…` Release；左下角图标栏提供更新入口，点击以 Popover 展示当前版本、检查结果、下载进度及更新操作，可在浮层中手动检查；点击外部、再次点击入口或按 Esc 收起。只按应用的语义版本比较，排除库版本、草稿和预发布版本，不降级。发现新版后点击“更新并重启”：后台下载并显示进度，核对 GitHub 资产 API 提供的文件大小和 SHA-256，再保存请求工作区、退出旧进程、静默安装到当前目录并重新打开。下载可以取消；网络失败、限流或校验失败可以重试。临时下载和超过一天的旧更新缓存会清理。

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

窗口采用参考稿的玻璃外壳、图标导航、请求集合、请求编辑器、响应区及右侧版本 / 请求历史。左下角图标在月光白（`Moonstone`）和曜石黑（`DarkObsidian`）之间切换，默认窗口 1440 × 900，最小 1050 × 700；右侧记录栏可收起。

集合、目录和环境的创建／重命名统一使用 `Dialog` 表单，支持名称校验、Enter 保存、Esc 或点击遮罩取消。没有环境和集合时，主面板以“暂无环境”和“创建环境”按钮显示空状态，创建首个环境后左侧继续提供创建入口。工作区管理、插件源添加与 cURL 导入也复用相同 Dialog 容器。

- 标签栏复用 `ClosableTabs`，使用 `on_add` 在末尾显示加号，点击打开请求选择器。
- 请求标题栏加号旁的更多菜单支持“创建集合”，输入名称并确认后显示空集合；空名称和重复集合名称会显示提示。点击集合标题切换展开，顶部加号在当前选中的集合或目录内新建请求。筛选支持集合、目录和请求名称；集合与目录独立管理并持久化到 SQLite。菜单支持点击外部或 Esc 关闭。
- 左侧是一棵目录树：至少先创建一个集合，请求可直接放在集合下；目录内可以继续创建请求和子目录，没有固定层级限制。收起／展开状态分别用 `Icon::Folder` 和 `Icon::FolderOpen` 表示，不使用箭头。各节点独立收起，打开请求会展开其父级路径；筛选显示匹配项及其祖先，清空后恢复原收起状态。深层目录支持横向滚动。
- 悬停集合或目录行只显示一个“更多”入口；下拉菜单提供新建请求、新建目录，集合支持“重命名集合”，普通目录另有重命名目录和删除目录。集合重命名会同步请求归属并保留目录默认配置、草稿、版本和历史；空名称或重名会被拒绝。打开菜单不会切换收起状态，点击外部或 Esc 关闭。目录名称在同级内唯一；重命名同步修改整棵子树，删除目录将子树内请求保留到上级，保留草稿、版本、历史与打开标签。
- 侧栏右边缘支持拖拽调宽（180–480px，上限随窗口宽度收缩），松手后结束调整，同一窗口内切换页面保持宽度。
- 侧栏使用通用 `TreeList` 与 `TreeListItem`；拖到目录中部移入该目录，拖到节点上／下边缘调整同级位置，集合可在顶层排序。支持跨集合移动目录子树和请求，循环嵌套或目标目录重名会被拒绝，位置写入 SQLite。
- 双击集合或目录打开目录设置，也可从“更多 → 目录设置”进入。URL / 方法、鉴权、参数、请求头、Cookie、请求体、前置动作、提取规则与 TLS 设置按集合到子目录逐层继承，并显示继承来源；每一项可独立覆盖，关闭覆盖恢复继承。覆盖按整项替换，空值、No Auth、空规则列表均可清除父级默认值。地址和 Token 支持 `{{baseurl}}` 等模板补全。保存默认配置后，新请求复制所在目录的最终配置，初始版本同步记录；已有请求可以自行修改，不随模板变更。目录重命名和跨集合移动携带自身及子目录配置，移动后继承新的父级；删除时移除对应默认配置。
- 旧数据库会将普通分组迁移为目录，将“未分组”和自动创建的“自定义”请求移到集合根部；当前使用数据库版本 3，旧版应用会拒绝打开以保护新结构。
- 请求与目录的删除均通过 Popover 二次确认；请求确认后删除版本和历史，目录确认后移除子目录并将请求保留到上级。支持取消、Esc 和点击外部关闭，鼠标移出原行时确认浮层保持显示。
- 首次启动没有预置集合、请求或环境；创建集合后可筛选、打开 / 关闭标签、新建空白请求；悬停请求行可删除请求，同时移除其标签、版本与历史。删除最后一个请求后展示可新建请求的空状态。
- 点击请求编辑区顶部的请求名进行重命名，悬停显示铅笔提示；Enter 或勾选按钮保存，Esc 或取消按钮放弃。名称不能为空，保存后同步侧栏、标签和命令搜索，并自动写入 SQLite；切换或删除请求会取消尚未提交的名称编辑。
- HTTP 方法和 URL 复用 `SelectField` 组合输入；环境在 topbar 通过独立 `Select` 全局切换并保存到工作区，所有请求共用；切换请求、恢复版本和历史不会更改全局环境。环境不保存到请求草稿或版本中。各请求独立保留方法、URL、参数、请求头、Bearer Token 引用和 JSON 请求体。
- 请求头页增加 Cookie 覆盖表格，支持启用、添加、删除和编辑键、值、说明；发送时解析原始 Cookie 请求头并合并，同名 Cookie 使用覆盖表格的值，禁用条目不参与覆盖。覆盖配置随草稿、版本与请求历史保存。
- 参数支持启用、添加、删除；发送前替换 `{{base_url}}` 并编码已启用的查询参数。
- “更多”下拉菜单与命令面板提供“从 cURL 导入请求”（可搜索 curl / 导入），粘贴 Bash / POSIX 命令后在当前选中的集合／目录内创建请求；没有集合时自动创建“导入的请求”。支持多行续行、`-X` / `--request`、请求头、Cookie、Bearer / Basic、原始正文、`--json`、`--data-urlencode`、`-G` 与 `-k`。导入按命令内容替换默认配置，不携带目录默认鉴权或动作，不自动发送；成功时写入初始版本。格式错误、文件引用、multipart、重定向或其他不支持的参数在弹窗内显示具体原因，不执行命令或读取本地文件。
- 响应面板提供“复制 cURL”，复制发送时已解析环境变量、合并 Cookie、应用前置动作请求头的实际请求参数，并包含实际鉴权、正文、TLS 和代理设置，参数使用 POSIX 单引号转义。每条请求历史与当前响应缓存独立保存 cURL 快照，切换历史时同步切换，后续编辑和环境变化不会重新计算。旧历史没有该快照时入口禁用，重新发送后可用。
- 请求体支持“JSON / 原始”和“x-www-form-urlencoded”两种模式，切换保留各自内容。表单字段支持启用、添加、删除、说明、重复键、空值和模板补全；先合并当前集合变量与全局变量并解析，再编码，自动添加 `application/x-www-form-urlencoded` Content-Type（显式请求头优先）。POST / PUT / PATCH / DELETE 使用该正文，GET 不发送正文。请求预览、前置动作签名与实际发送共享同一份编码结果；模式与字段随草稿、版本和历史保存。
- “后置动作”支持按请求配置多条环境变量提取规则，可启用、添加、删除并填写说明。例如变量名 `api_token`、JSONPath `$.data.token`，收到 JSON 响应后自动写入请求所属集合在发送时环境下的集合变量，不写入全局变量，随后可用 `{{api_token}}` 引用。规则随草稿、版本和历史保存；同名变量更新并保留敏感属性。字符串直接保存，其他单个值保存为 JSON，多个匹配保存为 JSON 数组。任一启用规则无匹配、路径或变量名无效、变量名重复或响应非 JSON 时，本次全部提取不写入，状态栏和执行轨迹显示失败原因；HTTP 响应仍正常保留。
- JSONPath 单元格悬停显示“从响应选取”：自动切到格式化 JSON，悬停字段高亮并显示路径，点击回填；空变量名按字段名生成，已有名称保留。可点击“取消选取”或按 Esc 退出。响应字段右键可“添加为提取规则”，优先复用空行或同路径规则；只配置规则，不立即写环境。每条路径同一行实时显示匹配数量和结果摘要，过长时截断，Token、密码及敏感环境变量只显示掩码；对象和数组显示结构摘要。
- 版本历史使用圆点与连线时间线，显示当前／基线标签、版本时间及启用参数摘要；点击旧版本恢复时同步移动当前标记。版本号旁可编辑自定义标签，悬停版本行时显示编辑图标，已有标签文字保持可见；支持 Enter 保存、取消 / Esc / 点击外部放弃编辑；留空保存移除标签。标签独立于版本号，不恢复配置、不生成新版本，长标签单行省略并可重新打开查看完整内容，随 SQLite 工作区持久化。
- 新请求历史保存当次响应、响应头、请求预览和执行轨迹，点击记录时同步恢复请求与响应面板。旧历史缺少响应快照时会清空响应并显示提示。响应头名称和值支持只读选中复制和长文本横向滚动；请求区与响应区之间使用 `Separator`。
- 请求设置提供“跳过 TLS 证书校验”，默认关闭；开启后允许自签名、过期或域名不匹配的 HTTPS 证书，仅影响当前请求。设置随草稿、版本和请求历史保存，旧配置默认保留正常校验。
- 左侧图标栏提供独立“代理管理”，可创建、重命名、保存或删除代理。环境只绑定一个代理，多个环境可共用同一代理；改名保留绑定，删除时解除全部绑定。环境绑定通过“保存更改”生效，代理修改通过“保存代理”对全部绑定环境生效；未保存的编辑不参与发送。旧环境内的代理配置自动迁移，相同配置合并为共享代理。
- 代理支持外部 HTTP / HTTPS 地址，或内置域名 / IP 映射表。映射表支持“从剪贴板导入 Hosts”：每行 `IP 域名 [更多域名]`，支持空行、`#` 注释和 IPv6，同名域名更新为后出现的 IP，格式错误时整批不导入；导入后保存代理生效。内置模式精确匹配域名并连接指定 IPv4 / IPv6，保留原始 Host、TLS SNI 和请求端口；未匹配域名正常解析，不改写系统 hosts，也不需要额外代理进程。HTTP、SSE、WebSocket 和 OAuth Token 请求共用环境绑定；OAuth 始终验证证书。可按代理启用或停用、配置 TLS 校验策略；不使用环境或不绑定代理时直接连接。
- 保存时仅在请求配置与最新版本不同时生成完整版本快照，重复保存或修改后撤销不会新增版本；自动持久化草稿不影响此判断。点击版本或请求历史恢复配置。请求历史按日期从新到旧分组，显示状态标签、方法与路径、精确时间、实际耗时和发送时的环境，可按成功／失败和路径筛选。悬停历史记录显示删除图标，通过 Popover 二次确认删除该次请求及响应、预览和执行轨迹；支持取消、Esc、点击外部关闭。删除前面的记录保留当前查看项，删除正在查看的记录清空对应响应展示，保留请求配置与版本；删除结果持久化到 SQLite。
- 响应栏提供独立的“请求”Tab，使用只读代码面板展示最近一次发送时的HTTP 请求快照：最终 URL、启用的请求头、身份验证生成的 Authorization、合并后的 Cookie 和实际参与发送的请求体，支持复制。快照按请求保留，后续编辑草稿不会改变快照；快照使用已解析的环境变量，与本次发送配置一致。
- 响应栏提供独立的“轨迹”Tab，可在发送中实时查看并独立滚动。每个请求保留最近一次的准备、校验、等待、响应、失败和取消记录，时间为相对本次发送的实际耗时；不模拟 DNS／TLS 等网络阶段。
- 响应代码使用 `CodeCard::viewport()` 虚拟列表，仅排版可见行；源码 / 原始切换复用相同内容的缓存，预览 / 源码切换复用原生 WebView，纯 UI 通知不再序列化 SQLite 工作区。
- 响应支持 JSON / HTML / CSS / JavaScript / XML 高亮、源码 / 原始视图、响应头、内置检查结果及复制。HTML 响应增加“预览”，运行 `cargo run -p glint-api --features webview` 启用 Wry 原生渲染（macOS / Windows 适配，macOS 验证）。预览使用实际响应正文，展示静态 HTML / CSS，不执行脚本；未启用 feature 时提供运行命令提示。
- 环境变量分为**全局变量**和**集合变量**，都按开发、生产等环境分别保存。已有工作区的变量保留为全局变量，供所有集合读取；集合变量仅对所属集合可见，同名时覆盖全局值，删除覆盖后恢复读取全局值。模板引用、OAuth、HTTP/SSE、WebSocket 与插件共享此规则，补全也只显示当前请求可见的变量。
- 环境管理顶部切换“全局变量 / 集合变量”；也可从集合的“…”菜单进入“集合变量”。切换作用域前需保存或放弃当前草稿；代理绑定仍为环境的全局设置。集合重命名保留变量，删除环境同时删除该环境下的集合变量。选择“不使用环境”时仍可管理与提取该集合的独立变量。
- 环境管理使用独立详情页，变量复用 `DataTable` 展示；表格内直接编辑名称、值和类型，支持添加行、名称搜索、复制引用和删除确认。通过“保存更改”统一校验并提交，“放弃更改”恢复已保存配置；未保存的编辑不参与请求。每个环境保留独立草稿。敏感值默认隐藏，点击眼睛后编辑。名称须以英文字母或下划线开头，只含英文字母、数字、下划线，同一环境的同一作用域内不能重名。浏览环境不会更改当前请求，点击“设为全局环境”应用。支持明暗主题。

| 快捷键 | 操作 |
| --- | --- |
| ⌘ / Ctrl + Enter | 发送 / 取消 HTTP 请求 |
| ⌘ / Ctrl + S | 保存，有修改时生成新版本 |
| ⌘ / Ctrl + K | 搜索命令与请求 |

点击发送执行真实 HTTP / HTTPS 请求，支持 GET / POST / PUT / PATCH / DELETE。使用 `{{变量名}}` 引用当前环境中的任意变量，发送前替换 URL、参数、请求头、Cookie 和请求体中的引用；变量值也可引用其他变量。未定义、空值、循环引用或嵌套过深会在发送前报错。Bearer 可直接输入 Token，也可引用 `{{api_token}}`；No Auth 不生成 Authorization。新请求不预填 URL、认证、参数或请求体，请输入实际服务地址。

顶栏“后台任务”下拉列出运行中的请求和 WebSocket 会话，点击可快速跳回对应请求。WebSocket 按请求 ID 独立维护连接、消息列表和解密结果，切换或关闭请求标签页会继续后台收发；可同时保持多个 WS 连接，显式断开或删除请求才结束相应会话。断开按钮使用 Danger 变体。WS 连接尝试、错误、收发事件和列表标题会保存到请求历史，重启后可回放；清空消息列表不会删除历史。每个会话最多保留 500 条事件和 8 MiB 原文。

HTTP 客户端使用 reqwest + Tokio，网络执行在独立运行时中，不阻塞 GPUI。连接超时 10 秒、总超时 30 秒，响应内容最多 16 MiB；保持正常 TLS 校验，3xx 响应直接展示，不自动跳转。HTTP 请求在取消、切换请求、删除请求或恢复配置时会中断执行和响应读取；服务端已经收到的操作无法撤回。

响应展示真实状态码、响应头（保留重复项）、耗时和解压后字节数。JSON 支持格式化；原始视图保留响应文本，纯文本、HTML 和空响应也可查看。文本按 Content-Type 的 charset 解码，未指定时使用 UTF-8；二进制响应暂以文本显示，不提供文件保存。4xx / 5xx 仍展示服务端响应体，连接失败和超时进入失败历史。检查页使用实际状态码和 JSON 解析结果，不执行用户脚本。轨迹记录准备、校验、发送、完成、失败和取消，不生成虚构的 DNS / TLS 阶段。

格式化 JSON 响应支持按对象、数组及数组项折叠，点击行号旁的箭头收起／展开。收起时显示隐藏行数，保留原始行号；查找隐藏内容会自动展开，复制仍包含完整 JSON。原始模式显示全文；JSONPath 选取期间临时展开，退出后恢复折叠状态。

响应和请求预览的长行可通过底部横向滚动条、触控板横滑或 Shift + 滚轮查看，行号保持固定。

响应体复制按钮旁的搜索图标或 ⌘ / Ctrl + F 打开工具栏内嵌查找控件，与“格式化 / 原始”保持同一行，不占用额外代码高度，不区分大小写，显示当前匹配序号与总数。上一处／下一处按钮或 Shift+Enter／Enter 循环跳转并滚动到匹配位置，Esc 关闭并清除高亮。搜索使用当前源码或格式化／原始文本；从 HTML 预览打开搜索会切回源码。切换响应或显示格式后自动刷新匹配结果。
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

URL、Token、参数、请求头、Cookie 和 JSON 请求体支持模板补全：输入 `{{` 展示当前环境中该集合可见的变量名和常用模板函数，继续输入可筛选；↑ / ↓ 选择，Tab / Enter 插入，Esc 或点击外部关闭。补全会补齐 `}}`，已有结束括号不会重复插入。敏感变量只显示名称；全局环境切换和后置提取后候选自动刷新。

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

## OAuth 2.0 用户授权

在请求的“身份验证”中选择 **OAuth 2.0**，填写授权地址、Token 地址、Client ID、回调地址和 Scope。从 **授权预设 → WPS 用户授权** 中选择内置插件的预设，可填入 WPS 的接口地址与 `kso.user_base.read`，默认关闭该文档尚未声明支持的 PKCE。WPS 多个 Scope 使用英文逗号分隔；其他服务商按其文档配置。Client ID 与 Secret 并排填写，接口地址使用紧凑对齐表单，底部固定授权状态与主操作。客户端认证默认将 Client ID / Secret 放在表单中；“高级设置”默认收起，其中可选用 Basic 认证、PKCE S256 和手动回调。

先在服务商后台登记完全一致的回调地址；自动回调默认 `http://127.0.0.1:17943/oauth/callback`，只监听本机 IPv4 回环地址。点击 **浏览器授权** 后在系统浏览器中登录并同意权限，工作台校验本次随机 state、回调地址、参数唯一性和会话有效期，再用授权码换取 Token。会话 10 分钟超时，支持取消；端口被占用时可换端口。服务商是否接受 HTTP 本机回调以其后台规则为准，WPS 文档没有保证支持本机回调。

如果服务商要求已登记的 HTTPS 回调，可启用 **手动回调**：先点击“浏览器授权”，在浏览器跳转至你自己的回调页后，将包含 `code` 和 `state` 的完整地址粘贴回来，点击 **完成授权**。这仍校验本次 state 和回调地址；不能直接粘贴任意历史授权码。回调页需由开发者提供，并保留地址中的参数。

**飞书用户授权**：选择 **授权预设 → 飞书用户授权**，填入飞书应用的 App ID（Client ID）与 App Secret（Client Secret），并在开发者后台“安全设置”登记相同回调地址。预设使用 `https://accounts.feishu.cn/oauth/v3/token`，默认通过表单传递应用凭据、关闭 PKCE。Scope 使用空格分隔，按需添加应用已开通的用户权限；默认的 `offline_access` 用于获取刷新令牌，需在“权限管理”开通。如后台显示“刷新 user_access_token”开关，还需启用并发布。仅保留 `offline_access` 不代表已获取业务权限。参考飞书官方的[授权码](https://open.feishu.cn/document/authentication-management/access-token/obtain-oauth-code)、[Token 交换](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/authentication-management/access-token/get-user-access-token-v3)与[刷新](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/authentication-management/access-token/refresh-user-access-token-v3)文档。

OAuth 的授权地址、Token 地址、Client ID、回调地址、Scope 和本次输入的 Client Secret 均支持当前环境中的 `{{变量名}}`，例如 `{{wps_app_id}}` 与 `{{wps_app_secret}}`，并提供变量补全。授权、刷新和发送前解析，编辑器与请求草稿保留模板原文；未定义、空值或循环引用会提示错误。凭据按解析后的配置隔离，同一环境的 Client ID 等配置值变更后需重新授权，不会复用旧应用的 Token。Secret 引用解析后仅用于本次授权并保存在系统凭据库，环境变量本身仍按已有工作区存储规则保存。

首次授权或更换密钥时填写 Client Secret（输入内容可见，提交时清空）；留空时沿用此连接已有的系统凭据。Client Secret、access_token 和 refresh_token 成对保存在 macOS Keychain、Windows Credential Store 或 Linux Secret Service 中，不能访问凭据库时会报错，不回退到明文文件。SQLite、版本、工作区导出和 MCP 中仅保存协议配置与连接 ID，不保存这些新凭据。请求快照与 cURL 始终隐藏注入的 OAuth Authorization，包括 MCP 的 `include_secrets: true`；原有手工 Bearer / 环境变量的存储方式不变。

HTTP、SSE 和 WebSocket 握手发送前检查授权，Token 临近到期（30 秒内）时刷新；同一进程的刷新操作串行化，避免重复使用已轮换的 refresh_token。刷新成功后在系统凭据库一次替换整组凭据，默认保留首次授权的 refresh_token 截止时间；飞书预设按每次响应的 `refresh_token_expires_in` 更新轮换令牌的截止时间，响应缺失时保留旧值；过期或刷新失败会提示重新授权。OAuth Token 接口使用当前环境的代理，始终验证 TLS 证书，不跟随重定向。WPS 的授权和 Token 接口不执行 KSO-1 插件；业务请求仍可自行启用已有签名插件。

凭据按工作区路径、环境名、连接 ID 和协议配置隔离；修改应用、Scope 或回调配置后需要重新授权。切换请求或环境会取消等待中的授权。目录设置的“身份验证”可选择 **使用当前请求的 OAuth 配置**，保存后新建请求继承同一连接；现有请求不受目录默认配置的后续变更影响。**清除本地授权** 删除当前连接的本机凭据，服务商端撤销需到其账号设置中操作。

WPS 与飞书的预设配置分别来自独立插件 `plugins/wps-oauth/glint-plugin.json` 和 `plugins/feishu-oauth/glint-plugin.json`，贡献点为 `oauth.preset`；选择时不执行 JavaScript，也不向预设插件提供密钥或 Token。自定义授权服务同样可以通过本地或远程插件提供，清单格式见根目录 [插件说明](../../docs/plugins.md#oauth-presets)。预设复制后由请求自行保存，插件更新或移除不会修改已有配置。

此功能用于 API 工作台调试开发者自己的应用。WPS 官方流程要求在服务端换取、刷新 Token 及调用接口；若面向最终用户发布，需要部署授权后端，不能在安装包内放入统一 APPKEY。参见 [WPS 用户授权流程](https://365.kdocs.cn/3rd/open/documents/app-integration-dev/wps365/server/certification-authorization/user-authorization/flow)。

## 请求插件

导航栏提供独立“插件管理”页面，负责应用级安装、卸载和插件源配置。请求配置的“前置动作”页只展示注册后的动作卡片，通过 Switch 启用或禁用，拖动左侧手柄调整执行顺序；所需环境变量显示在卡片下方。来源、版本与权限声明在独立插件管理页查看。批准的代码指纹和权限会随草稿、版本、历史和 SQLite 工作区保存；刷新或升级导致代码／权限变化时，需要重新授权。旧工作区默认不启用任何插件。

内置 `WPS KSO-1 签名`：在当前环境中保存 `wps_app_id` 和 `wps_app_key`，将后者标记为敏感，再在请求的前置动作页启用。它可以与 Bearer 同时使用。算法只存在于 `plugins/wps-kso1/main.js`，不写入公共运行时；使用实际编码后的 URI、原始发送 body 和最终 Content-Type，空 body 的摘要为空字符串。

卡片的「更多设置」默认收起，可指定优先读取的变量名，例如 wps_app_id1；名称不存在或设置留空时回退到原变量名。匹配到空值时会提示补全。配置按请求保存，并随版本、历史和工作区恢复；插件仅接收已声明的输入。

自定义插件放在工作区 SQLite 文件旁的 `plugins/<插件目录>/`，包含 `glint-plugin.json` 和 `main.js`，在插件管理页点击“刷新已安装插件”重新发现，再在请求中显式启用。可从内置模板复制，使用不同的插件 ID。通过 `contributions` 注册 `request.before-send` 类型动作，每个动作独立指定 ID、标题和同步 hook；不接受页面贡献；未安装或权限无效的已启用插件会阻止发送。

环境读取需要逐变量声明 `environment.read.<变量名>`；请求头修改需要 `request.header.<小写请求头名>`；读取请求需 `request.read`，加密需 `crypto`。Rust 宿主拒绝未声明变量、未授权请求头、Host/Content-Length 等传输头、CRLF 和过大输出。钩子接收 `{ method, uri, body, headers }`，返回 `{ setHeaders: { "Header": "value" } }`；不能修改 URL、方法或 body。需变更 body 的算法应在未来增加独立的准备阶段钩子，不能在签名后偷偷修改正文。

辅助进程通过 stdin/stdout 传递有限 JSON 数据，三秒未完成就终止，请求不发送；不会把密钥放进命令行或脚本错误消息。主界面不执行脚本。此机制不是 OS 权限沙箱或总内存硬限制。详细脚本异常不展示，以防异常文本泄露密钥。签名后的请求预览隐藏 Authorization、X-Kso-Authorization 和 Cookie。

通用插件契约与原生页面组件见根目录 `docs/plugins.md`。第一版未实现集合继承、在线插件市场、ES 模块依赖、热重载或异步钩子。

### 远程插件源

独立插件管理页支持添加多个远程源（名称、HTTPS 地址、可信公钥）、获取已验签的兼容版本列表并安装。安装后需要单独启用。已安装版本按源公钥身份隔离；同一 ID 的不同源不会覆盖，旧版本保留用于显式切换。共享 CLI 时，`--home` 指向工作区旁的 `plugin-registry/`。

自部署服务、发布 CLI 和协议见根目录 [插件仓库说明](../../docs/plugin-registry.md)。当前服务提供管理 API 与 CLI，尚无网页管理后台；客户端不保存发布令牌。

发送前校验失败或前置动作执行失败时显示错误 Toast（10 秒后自动收起，悬停暂停，可手动关闭），说明当前环境和具体原因。缺失或未填写的动作环境变量会按名称列出；提示不包含变量值。相同原因保留在响应错误区与执行轨迹中。

## Custom update source and Mac installers

Version 0.1.4 adds **软件更新 → 更新源设置**. Save a distribution base URL and a trusted Ed25519 public key to replace GitHub checks/downloads; use the gateway preset for this project's mirror, or restore GitHub at any time. Publishing instructions and HTTP routes are in [the distribution guide](../../docs/plugin-registry.md#software-distribution).

The release workflow builds Windows x64 Setup plus macOS Apple Silicon and Intel DMGs. Run `bash crates/glint-api/packaging/macos/build-macos.sh` on a Mac for a native-architecture `.app` and DMG. Mac installation is drag-to-Applications; Mac updates currently require manually replacing the app. Default Mac builds have ad-hoc signing only, without Developer ID signing or Apple notarization. Set `GLINT_API_CODESIGN_IDENTITY` to use an available signing certificate (notarization remains separate). Release artifacts include SHA-256 sidecars.

事件解密扩展声明贡献 `websocket.message.decode`、权限 `event.read` 和按名授权的 `environment.read.<name>`。钩子接收原始 JSON 事件，返回 `{data: <JSON>, title?: "列表摘要"}` 或 `{error: "说明"}`；通过同一个受监督插件子进程执行，事件解密的 IPC 输出上限为 2 MiB。可选 `title` 会替换左侧事件列表的密文预览，去除空白换行并限制为 160 字符；缺省、空标题或失败时保留原始预览。接收事件串行在后台处理，原文不被改写。跟随最新和显示格式与解密插件一起在“请求设置”配置并随请求保存。应用通用密码学由共享运行时提供，WPS 的字段拼接和协议规则位于独立插件脚本。

插件管理的每个已安装版本均提供标准 Switch，内置插件同样可禁用。开关按来源与插件 ID 保存到应用数据库，在所有工作区共享，同来源的全部版本共用状态；更新或刷新列表不会自动重新启用。禁用后，后续请求不执行已绑定的前置动作，不再提供对应 OAuth 预设和事件解密能力；原有绑定及已复制的 OAuth 配置保留，重新启用后恢复。已经发出的网络请求不受影响。`plugins_list` 同时返回 `enabled` 状态。
