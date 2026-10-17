Glint API 0.1.4 — 自定义更新源与 macOS 分发

- 软件更新支持保存自定义分发源地址和可信公钥，可一键填入 gateway 分发源或恢复 GitHub 默认源。自建源校验 Ed25519 签名、版本序号和安装包 SHA-256，检查与下载均不访问 GitHub API。
- 新增 macOS Apple Silicon / Intel DMG，包含软件图标及拖入 Applications 的安装入口；GitHub Actions 同时构建 Windows 和两个 Mac 架构。
- 插件管理采用独立页面、添加源对话框和可拖拽排序的前置动作；支持声明式插件 data 与 OAuth 授权预设。
- 增加 OAuth 授权、SSE / WebSocket 请求及 MCP 连接能力；完善目录模板、变量提取、请求历史和 cURL 工作流。
- gateway 插件服务新增独立软件下载页、签名软件版本目录、安装包分发及 CLI 发布/撤回/恢复管理，保留现有插件源身份。

Windows：下载 `Glint-API-0.1.4-windows-x64-setup.exe`。已安装版本可通过 GitHub 更新；遇到限流时手动安装本版一次，然后在左下角“软件更新 → 更新源设置”中填入 gateway 并保存检查，后续可自动下载、安装和重启。

macOS：Apple Silicon 下载 `Glint-API-0.1.4-macos-arm64.dmg`，Intel 下载 `Glint-API-0.1.4-macos-x86_64.dmg`，将应用拖入 Applications。Mac 通过下载 DMG 手动替换升级，尚未提供自动安装。Mac 包使用 ad-hoc 签名，尚无 Developer ID 签名和 Apple 公证；Windows 包也未进行代码签名。安装包均附 `.sha256` 文件。

升级保留工作区数据。使用自定义更新源前，请通过可信渠道核对公钥。
