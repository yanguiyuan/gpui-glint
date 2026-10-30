# Glint Workbench

将 Glint API 和 Glint Agent 集成到一个原生窗口，用统一侧边图标栏切换功能。

```sh
cargo run -p glint-workbench
```

默认启用 API 的原生 HTML 预览；不需要时可用 `--no-default-features` 构建。

## Windows 安装包

在 Windows 上安装 Inno Setup 6 后，从仓库根目录执行：

```sh
make win-workbench-setup
```

命令构建 Windows x64 release 程序，打包当前用户安装的 setup，并生成 SHA-256 校验文件。默认版本读取本项目 `Cargo.toml`。产物位于 `target/windows-workbench-installer/output/Glint-Workbench-<版本>-windows-x64-setup.exe`，校验文件为同名 `.exe.sha256`。

与 `win-setup` 相同，支持 `VERSION`、`ISCC`、`POWERSHELL` 覆盖，例如：`make win-workbench-setup VERSION=0.1.0 ISCC="C:/Program Files (x86)/Inno Setup 6/ISCC.exe"`。脚本会校验版本、静态链接 MSVC 运行库，并下载和校验 Microsoft WebView2 安装引导程序。安装目录、快捷方式及卸载标识独立于 Glint API。

## 导航与顶部栏

侧栏上半部分提供 API 请求、环境、代理、插件、请求历史和 MCP 调用历史；中间提供 Agent 会话、新建会话和日程；底部提供公共密码箱、Agent 设置和主题切换。选中图标随页面变化，主题同时应用到两个模块。

API 请求页顶部显示工作区、命令搜索、环境选择、后台任务和记录开关；环境、代理、插件和 MCP 页隐藏请求专用控件。密码箱隐藏工作区控件。Agent 会话页显示项目、会话搜索和 Provider；日程页隐藏这些控件；设置页保留设置搜索。窗口标题保持 Glint Workbench。

两个模块的 GPUI entity 在切换时保持存活，请求草稿、会话输入和后台任务不会因切换而重建。原生 HTML 预览在离开 API 时隐藏。编辑和持久化继续复用原模块实现。

## Agent 操作 API

Agent 页面自动注册 Glint API 已有的全部 `glint_*` MCP 工具，可直接查询和修改当前 API 工作区的集合、目录、请求、环境及历史，也可以发送请求并查询执行结果。例如：“创建一个测试集合，新增 GET 请求，然后发送并检查响应”。无需配置 MCP 服务，也无需先打开项目。

内置工具通过进程内通道调用当前 API entity，复用 MCP 的参数校验、业务处理、脱敏和调用历史；修改会同步到 API 页面。外部 MCP 配置仍可使用，配置或连接失败不会移除内置 API 工具。兼容 OpenAI、Anthropic 和 ChatGPT Provider，定时任务和子代理沿用同一工具连接。

“请求批准”模式下，写操作、发送请求及显式读取敏感信息需要批准；普通只读操作直接执行。自动批准和完全访问权限模式沿用原有行为。调用会显示在 Agent 工具记录中，API 的 MCP 调用历史中来源标记为 `Glint Workbench Agent`。取消会话后，尚未开始的排队调用不会继续修改工作区；已经启动的 HTTP / WebSocket 任务使用原有 API 工具查询或取消。

## 数据与兼容性

沿用独立应用的数据目录和凭据存储，无需迁移；`GLINT_API_DB_PATH`、`GLINT_AGENT_DATA_DIR`、`GLINT_VAULT_DATA_DIR` 仍有效。密码箱使用公共入口。

Glint Workbench 与 Glint API 共用 API 工作区的进程锁，因此同一 API 数据目录只允许一个程序运行。原有 `cargo run -p glint-api` 和 `cargo run -p glint-agent` 入口保留。

统一程序支持 `--mcp`、`--mcp-config` 和内部 `--plugin-worker` 协议，插件子进程和 MCP 自动启动可以使用当前可执行文件。API 独立程序的自动更新器不会在统一程序中启动，避免用 API 安装包覆盖整个工作台。

## 验证

```sh
cargo check -p glint-workbench -p glint-api -p glint-agent --all-targets
cargo test -p glint-api -p glint-agent hosting::tests
cargo test -p glint-workbench
```
