---
name: glint-plugin-dev
description: 为 Glint 应用开发 JavaScript 插件，编写插件清单、注册请求前置动作、声明宿主权限、提供 data 声明式配置与 OAuth 授权预设、验证并通过 glint-pluginctl 发布到自部署插件源。用于 Glint 插件开发和迁移，不用于安装 Codex 插件或修改 Glint 核心架构。
---

# Glint 插件开发

将业务算法放在插件，公共运行时保持与 HTTP、数据库和具体业务无关。宿主决定支持的贡献类型和权限；先确认用户的目标应用与版本。以下同步前置动作示例兼容 Glint API 0.1.2 的协议；`data` 与 `oauth.preset` 从 Glint API 0.1.4 起提供；0.1.2 / 0.1.3 发布版不支持这些声明。使用目标客户端实际支持的契约。

## 最小插件

目录只需 `glint-plugin.json` 与 `main.js`。清单 ID 使用小写字母、数字、点和连字符；版本遵循 SemVer，发布后不可覆盖。入口必须在插件目录内。

```json
{
  "id": "com.example.body-digest",
  "name": "正文摘要",
  "version": "1.0.0",
  "api_version": 1,
  "entry": "main.js",
  "capabilities": ["request.read", "crypto", "request.header.x-content-sha256"],
  "hooks": ["request.before-send"],
  "contributions": [
    {"id":"digest","kind":"request.before-send","title":"计算正文摘要","hook":"request.before-send"}
  ]
}
```

```javascript
glint.on('request.before-send', request => ({
  setHeaders: {
    'X-Content-Sha256': glint.host.call('crypto.sha256Hex', request.body)
  }
}));
```

贡献的 `id` 是稳定动作标识，`title` 是请求页展示的动作名称；`hook` 必须在清单 `hooks` 中声明，并由 `glint.on` 注册。同一插件可以贡献多个动作，分别绑定各自 hook；前置动作的 `kind` 仍是 `request.before-send`。客户端展示动作并独立保存开关与执行顺序，插件自行管理界面或排序不是此接口的一部分。

## 运行时与权限

- JavaScript 由嵌入式 Boa 执行。无隐式 Node.js、文件系统、进程、网络、npm 或 ES 模块加载；使用同步回调，不返回 Promise。
- 前置动作收到 `{method, uri, body, headers}`。URI 是实际编码后的路径与查询串，不含 fragment；body 是最终发送正文，不应在签名后更改。
- 前置动作只返回 `{setHeaders:{"Header-Name":"value"}}`。不要额外返回 URL、方法、body 或未知字段。
- 读取请求声明 `request.read`；修改每一个请求头分别声明 `request.header.<小写头名>`。宿主禁止 Host、Content-Length、Transfer-Encoding、Connection 和含 CR/LF 的头值。
- `crypto` 授予 `glint.host.call('crypto.sha256Hex', text)` 和 `glint.host.call('crypto.hmacSha256Hex', key, text)`。
- 读取单个环境变量声明 `environment.read.<变量名>`，调用 `glint.host.call('environment.get', '变量名')`。让用户将密钥标记为敏感；不把密钥写进源码、日志、错误、更新说明或示例真实数据。
- 清单最多 64 KiB，源码最多 256 KiB。执行存在 VM 限制与宿主进程超时，不编写无限循环、长耗时正则或依赖运行时跨调用保留状态的算法。
- 安装只下载数据；启用才创建内容与权限绑定。源码或权限改变后需要重新启用。插件不得绕过宿主授权。

## `data` 与授权预设

贡献的 `data` 是可选 JSON，由宿主解析和校验，既不是 JS 回调，也不授予权限。通用契约允许纯声明贡献省略 `hook`，并允许纯声明插件使用空 `hooks`。Glint API 的前置动作贡献不能附带 `data`；授权预设使用 `kind: "oauth.preset"`、省略 `hook`，将设置放进 `data`。不要将两类配置混在一个贡献里。

纯授权预设示例：

```json
{
  "id": "com.example.oauth",
  "name": "示例授权预设",
  "version": "1.0.0",
  "api_version": 1,
  "entry": "main.js",
  "capabilities": [],
  "hooks": [],
  "contributions": [{
    "id": "user-authorization",
    "kind": "oauth.preset",
    "title": "示例用户授权",
    "data": {
      "authorization_url": "https://accounts.example.com/authorize",
      "token_url": "https://accounts.example.com/token",
      "scope": "profile",
      "pkce": true
    }
  }]
}
```

仍保留空的或仅含注释的 `main.js`，以满足打包格式。发现、选择预设都不执行 JS。预设出现在原生 OAuth 编辑器的“授权预设”中；选中后复制到一个新的连接，Client ID 留空，由用户填写自己的应用凭据并显式开始授权。已复制的连接不随插件更新或卸载而自动改变。

`data` 的 OAuth 字段受白名单约束：

| 字段 | 约束或默认值 |
| --- | --- |
| `authorization_url`、`token_url` | 必填 HTTPS 地址（本机 127.0.0.1 调试可用 HTTP），无 URL 用户名、密码、fragment；授权地址不预填 OAuth 协议参数 |
| `scope` | 默认空字符串，按服务商要求配置 |
| `pkce` | 默认 `true`；仅在服务商明确要求时关闭 |
| `basic_auth` | 默认 `false`；决定 Token 请求的客户端认证方式 |
| `redirect_uri` | 可选，省略时使用宿主的本机回调地址；按宿主校验规则配置 |
| `manual_callback` | 默认 `false` |
| `rolling_refresh_expiry` | 默认 `false`；为 `true` 时按每次刷新返回的新 refresh token 有效期重算期限 |

禁止在预设 `data` 中携带 `client_id`、Client Secret、access token、refresh token 或其他未知字段；未知字段会被拒绝。纯预设插件不声明 `request.read`、`crypto` 或其他运行时权限，`hooks` 保持空。若同一插件同时提供前置动作与预设，分别注册贡献，并仅为实际执行的动作声明其所需权限。

先验证授权端点、Token 端点、回调、scope 和服务商要求，再提供预设。参考目标仓库的 `plugins::OAuthPreset`、`oauth::Config::validate` 和 `crates/glint-api/plugins/wps-oauth` / `feishu-oauth`；不要仅根据字段名猜测供应商语义。发布纯授权预设时最低宿主版本应包含预设能力，使用 `--host glint-api@0.1.4` 或兼容的更新版本，而不是动作示例的 `0.1.2`。仓库服务也必须包含支持 `data` 的协议版本。

## 本地验证

有源码仓库时，先阅读目标应用的 `crates/glint-api/src/plugins.rs`、`crates/glint-plugin/README.md` 和对应运行时 API，避免将其他版本的接口当作当前支持项。参考插件在 `crates/glint-api/plugins/`。仓库地址：https://github.com/yanguiyuan/gpui-glint 。

把开发目录放在工作区 SQLite 文件旁的 `plugins/<目录>/`，在独立插件管理页刷新；再在请求“前置动作”中启用注册的动作。CLI 的安装目录与客户端工作区相互独立，除非显式通过 `--home` 指向同一个插件存储。

验证业务协议提供的固定输入/输出向量、空正文、编码后的 URI、缺失环境变量，以及权限不足的错误。确认发送的请求头符合协议，错误路径不会发送请求，也不会泄漏密钥。有仓库时运行受影响的 Rust 测试；执行脚本时使用宿主隔离入口，不要仅凭编辑器 JS 检查就声称运行成功。

## 发布与安装

`glint-pluginctl` 是独立 CLI，不是客户端 JS API。需要时在源码仓库构建：

```sh
cargo build --release --locked -p glint-plugin-cli
```

将 CLI 加入 PATH 后，先配置用户指定的源与经可信渠道确认的公钥：

```sh
glint-pluginctl source add company https://plugins.example.com --public-key '<可信 Base64 公钥>'
glint-pluginctl list --source company
```

`source inspect` 只返回服务自称的公钥，不能代替可信渠道确认。不能因为安装了本技能就替用户发布、撤回插件或修改远程源；仅在用户请求的范围内执行远程管理。

用户已要求发布时，从安全的进程环境提供 `GLINT_PLUGIN_TOKEN`，不放在命令参数、源码或源配置里，然后运行：

```sh
glint-pluginctl publish ./my-plugin --source company --host glint-api@0.1.2 --changelog 'Describe the change'
glint-pluginctl install com.example.body-digest 1.0.0 --source company --host glint-api --host-version 0.1.2
```

使用真实已验证的最低兼容宿主版本。每次发布递增版本；遇到版本冲突不能覆盖或换 ID 绕过，先核实版本再处理。下载的 `.glintpkg` 包为有界 JSON 数据，安装客户端会验证签名索引、包摘要与权限声明。撤回命令为 `yank ID VERSION --source NAME`，恢复为 `restore ID VERSION --source NAME`，两者均需管理令牌。遇到认证或权限拒绝时保留失败信息并停止该管理操作，不尝试更换凭据或关闭校验。
