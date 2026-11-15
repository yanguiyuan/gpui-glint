# AgentSpace

AgentSpace 是跨仓库任务的 home repo，也可用于不属于某个项目的临时任务。目录不需要初始化为 Git 仓库。

在输入框的项目选择菜单中，找到「AgentSpace · 新会话默认目录」分区，点击「选择默认目录…」，或打开一个目录后点击「将当前目录设为默认空间」。应用会保存默认目录，并在目录中创建 `agentspace.toml`（已有配置不会被覆盖）。之后点击会话栏加号或使用新建会话快捷键，默认选择 AgentSpace；仍可通过项目选择器切换到普通项目。已有会话保留原项目。

编辑 AgentSpace 目录下的 `agentspace.toml`：

```toml
[[repositories]]
name = "frontend"
path = "../frontend"
description = "前端应用与组件"

[[repositories]]
name = "backend"
path = "/absolute/path/to/backend"
description = "后端服务与 API"
```

`name` 和 `path` 必填，`description` 可省略。相对路径以 AgentSpace 为基准；仓库目录必须存在，名称和解析后的路径不能重复。不支持将 SSH 目录配置为本地 AgentSpace 的关联仓库。

每次发送消息时读取配置，将 home repo 和关联仓库的名称、绝对路径及说明自动加入模型上下文，供 agent 按需读取代码。不会自动载入整个仓库的文件内容。无效配置或不可用目录会明确报错，修复后重新发送即可。修改配置在下一次请求生效，本次请求及其子代理使用固定的目录清单。

文件工具的相对路径以 home repo 为基准；访问其他仓库请使用清单中的绝对路径。`search_files` 支持 `path` 参数，`run_command` 支持 `cwd` 参数。询问 / 自动批准模式下，文件工具及命令工作目录限于 home repo 和声明的关联仓库，仍检查符号链接实际目标；完全访问模式保留原有的项目外访问能力。命令工作目录不是进程沙箱，宿主工具及外部 MCP 继续使用各自的执行位置。

项目选择菜单中的「取消默认空间」恢复沿用当前项目的新建会话行为；配置文件保留。移除 AgentSpace 项目也会取消默认设置，不删除目录或历史会话。
