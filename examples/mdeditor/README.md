# Glint Markdown

基于 `design/glass-ui.html` 和 `gpui_glint::components` 的独立原生 Markdown 编辑器。

```sh
cargo run --example mdeditor

# 直接打开并预览任意 Markdown 文件（文件留在原处，不复制进文档库）
cargo run --example mdeditor /path/to/notes.md
```

macOS 直接运行时会使用内嵌的应用图标。也可生成带 Finder / Dock 图标的应用包：

```sh
bash examples/mdeditor/build-macos.sh
open "target/Glint Markdown.app"
```

图标源文件位于 `assets/mdeditor-icon.png`；打包脚本生成多尺寸 `.icns` 并写入应用包。

可通过环境变量指定文档库，用于隔离演示数据：

```sh
GLINT_MARKDOWN_HOME=/tmp/glint-markdown-demo cargo run --example mdeditor
```

首次创建文档库时生成欢迎文档。侧栏显示**最近打开历史**（含通过命令行
打开的库外文件，每个条目绑定绝对路径，随会话持久化，上限 50 条）；
新建的文档始终放入默认工作区目录。
默认位置为 macOS 的
`~/Library/Application Support/gpui-glint/mdeditor`、Windows 的
`%APPDATA%/gpui-glint/mdeditor`，或 Linux 的
`$XDG_DATA_HOME/gpui-glint/mdeditor`（未设置时使用 `~/.local/share`）。

已有的 `gpui-glass/mdeditor` 或更早的 `gpui-glass/markdown-editor` 文档库继续原地使用，
保留会话中的绝对路径和恢复记录。`GLINT_MARKDOWN_HOME` 优先于兼容的旧变量
`GLASS_MARKDOWN_HOME`。

## 使用

- 左侧按标题或正文内容搜索本机文档，点击打开；搜索关键词同步到文档组件，在文档标题、正文、代码文本和表格单元格中高亮命中词，清空搜索清除高亮。上方标签切换、关闭文档。
- 顶部左侧的侧栏按钮隐藏或展开文档列表，重启后恢复上次的显示状态。
- `⌘/Ctrl N` 新建，`⌘/Ctrl O` 导入，`⌘/Ctrl S` 保存，`⌘/Ctrl W` 关闭标签。
- 直接编辑正文，选中文字设置格式；空白块输入 `/` 插入内容。现有组件提供撤销、重做、预览、表格、代码块、任务列表、块拖动以及目录跳转和收起。
- 软件 topbar 的「更多」分组提供工作区、当前文档和编辑操作；正文上方仅显示文档路径。点击外部或按 Escape 关闭菜单。
- 「更多 → 重命名」修改文件名；顶部「排版 / 源码」分段控件切换块编辑与完整 Markdown 编辑。
- 文档顶部标题可直接编辑，输入停顿后同步文件名、标签和文档列表；标题独立于正文，不计入 Markdown 块。
- 编辑自动保存 UTF-8 Markdown；「导出…」通过系统文件对话框保存副本。
- 「打开 Markdown…」导入 `.md` / `.markdown` 副本，保留原文件；同名文档自动添加序号。
- 顶部主题选择器可预览并选择五种 Glass 配色；「更多 → 打开文档库」打开本机文档库。
- 再次启动恢复标签、当前文档及主题。

## 保存与恢复

保存前写入 `.recovery` 恢复记录，再通过临时文件替换正文文件。成功保存后清理恢复记录。
磁盘文件与上次保存内容不一致时阻止覆盖，保留当前改动和恢复记录；可以导出到另一位置后关闭并重新打开。
保存失败时禁止关闭该标签及窗口。恢复记录会在下次打开对应文档时读取。

源码和块编辑器分别维护历史；源码发生修改后返回排版会重新解析文档并重置块编辑历史。
外部文件修改不会自动热加载，可关闭并重新打开。示例使用同步本机文件读写，适用于普通文档；大规模文档库应把读写、全文索引移到后台执行。

## 结构

- `main.rs`：工作区、文档生命周期、组件组合、快捷键和系统文件对话框。
- `storage.rs`：文档库、文件命名、同名副本、保存冲突检查、原子替换、恢复记录及测试。
- `welcome.md`：首次启动的通用占位文档，演示 Markdown 格式；Playground 文档页复用同一份内容。
- 应用层 topbar 的「更多」菜单通过 `DocumentEditor` 的公开命令组织撤销、重做、完成编辑、预览和复制；文档组件不内置应用工具栏。
- `TextEntry::value` / `set_value`：新增的通用文本输入访问接口。

```sh
cargo check --examples
cargo test --example mdeditor
cargo test --lib
```

Mermaid 代码块离线渲染，支持编辑源码并实时更新图表。空块输入 `/mermaid` 可插入示例；组件左上角切换渲染和源码视图，预览模式下源码只读；导出保留原始 Markdown 源码。语法覆盖以 mermaid-rs-renderer 为准。
