# 组件使用指南

返回 [Glint UI README](../README.md)。这里的代码片段用于已有 GPUI 视图，`cx`、`window`、`theme` 和应用状态由宿主提供；完整可运行示例见 [`examples`](../examples)。

## 布局与基础控件

`AppLayout` 填满父容器，顶部栏高 42px，下方为可伸缩的横向布局。导航轨道位于外层表面；侧栏和主内容共享带圆角、高光与柔和阴影的面板。各区域均可接收任意 GPUI 元素。

Windows 下 `AppLayout` 默认在顶栏右侧显示最小化、最大化／还原和关闭按钮，使用原生窗口控制区域（关闭操作会经过宿主的 `on_window_should_close` 回调）。窗口需使用透明标题栏 `TitlebarOptions { appears_transparent: true, ..Default::default() }`；嵌入预览或保留系统标题栏时设置 `.window_controls(false)`。品牌名仅在 macOS 为红绿灯预留左侧空间。把 `window_drag_region()` 放在顶栏控件之间作为可伸缩空白区域，可拖动窗口，双击最大化／还原；不要用拖拽区域包裹搜索框或按钮。

`SegmentedControl` 默认以 `new` 的调用位置区分控件，隔离每个选项的点击状态。在循环或公共工厂函数中创建多组控件时，为每组设置稳定且不同的 `.id(...)`。

`GroupedDropdown` 通过触发按钮展开或收起，支持带标题的分组和可选图标，选择后自动关闭。使用 `on_select` 接收所选标签，并将其连接到应用状态。

`Separator::horizontal()` 和 `Separator::vertical()` 提供随主题变化的 1px 分隔线，不参与拖拽；用于分隔上下或左右相邻区域。

## 通用树列表

`TreeList` 管理整棵树的层级、可见节点和筛选，节点数据使用 `TreeListNode::branch(id, label, children)` / `leaf(id, label)`。通过 `.collapsed(ids)`、`.selected(id)` 传入受控状态；`.on_toggle(...)` / `.on_select(...)` 通知宿主更新状态。`.query(...)` 筛选时保留匹配节点的祖先，临时展开匹配路径，清空查询后恢复原有折叠；节点 `.search_text(...)` 可补充 URL、方法等检索文本。

```rust
TreeList::new([
    TreeListNode::branch("api", "用户接口", [
        TreeListNode::leaf("list", "获取用户列表"),
        TreeListNode::leaf("create", "创建用户"),
    ]),
])
.selected("list")
.theme(theme)
.render();
```

默认使用 `TreeListItem` 渲染，整棵树可通过 `.draggable(scope)` / `.on_drop(...)` 接收拖拽投放。复杂业务可用 `render_with(|row| ...)` 定制节点（含 `id`、`depth`、`branch`、`expanded`），复用层级与筛选逻辑；侧栏滚动、数据保存和业务移动规则由宿主管理。`glint-api` 请求侧栏已使用这一通用容器。

`TreeListItem` 负责单行树节点的缩进、选中样式、目录开合图标与拖拽反馈，可用于请求列表、目录树等。传入唯一稳定 ID；`.depth(n)` 设置层级，`.branch(expanded)` 设置目录及开合状态，未设置时为叶子。`.content(element)` 自定义内容区域（例如方法与请求名），`.trailing(element)` 放菜单或操作按钮。树结构、筛选、展开状态和点击行为由宿主管理。

```rust
TreeListItem::new("folder-users", "用户接口")
    .depth(1)
    .branch(true)
    .selected(false)
    .theme(theme)
    .render();
```

`.draggable(cx.entity_id())` 开启同一作用域内的拖拽，不同作用域之间不能投放。`.on_drag_over(...)` 与 `.on_drop(...)` 接收 `TreeDrop { source, target, placement }`；目录中部为 `Inside`、顶部／底部为 `Before`／`After`，叶子仅支持前后插入。组件显示即时插入线，也可由宿主保存当前反馈并通过 `.drop_indicator(...)` 设置标记；投放后由宿主校验循环、重名等规则并更新数据。原生示例见 `glint-api`，基础展开与选中示例见 playground 布局页。

`TextEntry::read_only(true)` 保留拖选、双击选词、全选及复制，阻止键盘和 IME 修改；`.fit_content()` 让控件按完整文本宽度布局，可放在横向滚动单元格中。注册 `bind_text_entry_keys` 后支持 ⌘ / Ctrl + A、C。

## 可关闭标签页

`ClosableTabs` 统一提供可关闭标签与内容卡片，支持 `.on_add(...)`：设置回调后，在最后一个标签后显示加号；未设置则不显示。加号点击只触发新增回调，不触发选择或关闭；宿主负责创建或打开标签。全部标签关闭后，加号仍保留，可重新开始。

```rust
ClosableTabs::new([
    ClosableTab::new(1, "用户列表"),
    ClosableTab::new(2, "创建用户"),
])
.selected(Some(1))
.theme(theme)
.on_add(move |window, app| {
    // 在宿主中创建新标签或打开请求选择器。
});
```

`ClosableTab::modified(true)` 显示未保存标记；选择与关闭通过 `on_select`、`on_close` 交由宿主处理。标签使用稳定的 `u64` ID。

`ClosableTabs` 默认按内容高度布局，内容内边距为 20px。使用 `.fill_height().content_padding(0.)` 可承载填满工作区的编辑器，标签栏固定，内容使用剩余高度。

Playground 的“标签页”示例可通过加号新增请求；`glint-api` 使用 `ClosableTabs` 承载请求编辑器，加号打开命令与请求面板。

## 选择器与文本输入

`Select` 提供受控单选，支持稳定的选项值、禁用选项、占位文字、可选搜索、键盘导航及点击外部关闭。搜索默认关闭。注册 `bind_text_entry_keys(cx)` 以支持原生搜索输入，并在所属视图中保留事件订阅：

```rust
let environment = cx.new(|cx| {
    Select::new([
        SelectOption::new("dev", "开发环境"),
        SelectOption::new("prod", "生产环境"),
    ], cx).selected("dev").searchable(true)
});
let subscription = cx.subscribe(&environment, |_, select, event: &SelectEvent, cx| {
    let SelectEvent::Changed(value) = event;
    select.update(cx, |select, cx| select.set_selected(Some(value.clone()), cx));
});
```

使用 `set_options`、`set_selected`、`set_disabled` 和 `set_theme` 在运行时更新组件。未知的选项值显示占位文字。方向键跳过禁用项；Enter 确认，Esc 取消，Tab 关闭并继续切换焦点。紧凑选择器中，Home / End 跳转到首项或末项。展示程序的“输入与选择”页包含紧凑、可搜索和禁用状态示例。

`width(px)` 设置期望宽度，`min_width(px)` 设置最小宽度（默认 100 像素），两者调用顺序不影响最终尺寸。例如表格内可使用 `.width(78.).min_width(60.)`；触发器和下拉面板遵守最小宽度。使用 `set_min_width(px, cx)` 可在运行时更新。

`TextEntry` 是原生 GPUI 单行文本编辑器，支持输入法组合输入、文本选择、光标定位和剪贴板快捷键。通过 `bind_text_entry_keys(cx)` 注册一次快捷键，再使用 `.control(...)` 将其实体传入 `InputField` 或 `SearchInput`。其他输入组件通过受控状态和回调，由所属视图负责更新。

`TextEntry` 与 `SelectField` 可通过 `set_reference_resolver(Some(resolver), cx)` 开启 `{{变量名}}` 引用展示。`TextReferenceResolver` 是 `Rc<dyn Fn(&str) -> Result<String, String>>`，接收完整表达式；解析成功时，失焦输入显示值徽标，失败时用警告色保留表达式并在悬停时显示原因。点击徽标进入原文编辑，编辑时引用仍有背景高亮；`value()`、选区、输入法和剪贴板始终使用原始内容。默认不开启，传入 `None` 可关闭。环境变化时应更新 resolver。

```rust
use std::rc::Rc;
use gpui_glint::components::TextReferenceResolver;

let resolver: TextReferenceResolver = Rc::new(|expression| {
    match expression {
        "{{base_url}}" => Ok("https://api.example.com".into()),
        _ => Err("未定义变量".into()),
    }
});
input.update(cx, |input, cx| input.set_reference_resolver(Some(resolver), cx));
```

## 徽标

徽标组件涵盖资源类型计数、内容标签、未读及等级计数、封面叠加徽标，以及阅读和转换状态。`TagBadgeKind` 与 `CountBadgeKind` 提供克制的语义配色；红色计数用于未读内容，`ReadStatus::Failed` 用于标记错误。

## 图标与资源

`Icon::Save` 提供软盘形保存 SVG，继承文字颜色。按钮可通过 `Button::new("保存").icon(Icon::Save)` 在文本前显示图标。

`Icon` 渲染 `glint-ui/icons/` 命名空间下的内嵌 SVG 资源。创建 GPUI 应用时配置资源来源：

```rust
gpui_platform::application().with_assets(gpui_glint::components::GlintAssets)
```

如果接入应用已有资源来源，请将其加载器与 `GlintAssets` 合并，确保应用自身资源和 `glint-ui/icons/*.svg` 均可加载。

## 代码卡

`CodeCard` 提供可嵌入文档的代码卡：9px 圆角、中性的 35px 语言栏，以及字号 13px、行高 21px 的等宽源码。长行可横向滚动，行号保持固定；一次滚动手势锁定一个方向，横向滚动不带动文档纵向移动，纵向滚动交给文档容器。高度随文档内容自然增长。Rust（`rust` / `rs`）、Go、JSON、HTML（`html` / `htm`）、XML / SVG、CSS 和 JavaScript（`javascript` / `js`）围栏代码支持语法高亮，未知语言显示纯文本。Rust 支持关键字、类型、函数与宏、生命周期、数字、字符串与字符、原始字符串和嵌套块注释。JSON 在编辑时校验，仅有效且可编辑时允许格式化，并保留键的顺序。

```rust
use gpui_glint::components::{CodeCard, CodeCardEvent, bind_code_card_keys};

bind_code_card_keys(cx); // 与 bind_text_entry_keys 一起注册一次
let code = cx.new(|cx| CodeCard::new(cx, "json", r#"{"enabled":true}"#));
// 在文档中渲染 code.clone()；仅阅读时可使用只读模式：
let readonly = cx.new(|cx| CodeCard::new(cx, "go", "package main").read_only(true));
// 隐藏整条语言栏，直接显示代码内容：
let headerless = cx.new(|cx| {
    CodeCard::new(cx, "http", "GET https://api.example.com")
        .read_only(true)
        .show_header(false)
});
```

语言栏默认显示；`.show_header(false)` 隐藏语言图标、标签、JSON 校验提示和格式化按钮，且不保留头部高度。卡片边框、圆角、行号、高亮及编辑能力保持原样；JSON 仍可通过 `json_valid()` 校验、`format_json(cx)` 格式化。运行时可调用 `set_show_header(value, cx)` 切换显示，保留内容、选区和撤销历史。

只读大文件可使用 `CodeCard::new(cx, "html", source).read_only(true).show_header(false).viewport()`。宿主提供固定高度或 `flex_1().min_h_0()` 区域；该模式由卡片内部处理纵向滚动，按可见行排版，缓存最多约 512 行的字形。不要再放入按内容高度增长的外部纵向滚动容器。普通文档内嵌和可编辑卡仍保留自然高度。相同内容或语言的程序更新不再清空布局缓存，只读内容替换不进入撤销历史。

代码卡和 Diff 卡的语言栏自动使用内置单色 SVG 图标，支持 Markdown、Rust、Go、JavaScript / TypeScript、Python、Java、C / C++、HTML / CSS、JSON、Shell、SQL、YAML，兼容 `md`、`rs`、`js`、`tsx` 等常用别名。图标继承文字颜色，随主题变化；未知语言回退到通用代码图标。也可通过 `Icon::for_language("md")` 获取图标，或直接使用 `Icon::Markdown` 等枚举。

代码卡同时支持原生输入法、选区、剪贴板、缩进，以及局部撤销和重做。订阅 `CodeCardEvent::Changed` 接入文档历史，订阅 `CodeCardEvent::ContinueAfter` 处理 ⌘ / Ctrl + Enter。通过 `set_language` / `set_dark` 修改语言或代码配色时保留编辑器状态；使用 `.theme(theme)` 初始化主题，`set_theme(theme, cx)` 在运行时同步亮暗配色。`.dark(true)` 可显式启用深色代码配色。该组件负责源码卡片，不包含 Mermaid 渲染或 Front Matter 预览。

## 数据表格

`DataTable` 提供资源表格：中性的 35px 表头、53px 资源行、蓝色选中背景、悬停操作和向上展开的操作菜单。固定列可与弹性的资源列组合；窄布局下在表格内部滚动。底栏显示实际行数与选中数量，并提供分页。

空表默认显示居中的“暂无数据”。设置 `.empty_row("暂无 Cookie 字段覆盖。")` 后，改为横跨全部列的单行提示，左侧内边距与单元格一致，高度随普通／紧凑模式使用 53px／34px。提示行不计入数据、选择或分页，可与 `.add_row("添加 Cookie 字段覆盖")` 组合；添加真实数据后自动隐藏，删除到空表后恢复。运行时可通过 `set_empty_row(Some(label.into()), cx)` 更新文字，传入 `None` 恢复默认占位。

`.compact()` 启用紧凑布局：表头 28px、行高至少 34px、单元格水平内边距 8px／垂直内边距 4px。可与 `.document()` 组合用于参数和请求头编辑；较高的单元格内容仍会撑开行高，避免裁切。默认保留标准密度。

`.add_row("添加空行")` 在数据行末尾增加带加号的操作行，默认隐藏。点击或聚焦后按 Enter／空格会发出 `TableEvent::AddRow`，由调用方创建空行并调用 `set_rows`。空表同样显示该入口；操作行不计入数据总数，也不参与排序、选择和分页，可与 `.compact()`、`.document()` 组合使用。

`TableCell::custom(|| element)` 支持输入框等交互单元格，每次渲染返回新的元素，编辑状态保存在捕获的实体中。参数／请求头这类编辑表格可使用 `.document()` 隐藏行选择、内置操作栏和分页，再通过自定义单元格提供启用和删除操作。自定义单元格默认不提供排序值，可按需追加 `.sort_number(...)`。

```rust
use gpui_glint::components::{DataTable, TableColumn, TableCell, TableRow, TableEvent};

let table = cx.new(|cx| DataTable::new(cx, [
    TableColumn::new("编号", 72.).sortable(),
    TableColumn::new("资源", 230.).flexible().sortable(),
    TableColumn::new("大小", 82.).right().sortable(),
], [
    TableRow::new("007", [
        TableCell::mono("007").sort_number(7.),
        TableCell::resource("雾夜 · 慢门 30s 组图", 0x305343),
        TableCell::mono("1.2G").sort_number(1200.),
    ]),
]).page_size(20));
```

行使用稳定 ID。单击选中一行，⌘ / Ctrl + 单击切换该行的选中状态，Shift + 单击选择连续范围。方向键在行或页面之间移动；Enter 触发选中行的主要操作，Esc 关闭菜单。可排序列默认比较文本，单元格可提供数值排序值。页码从 0 开始。订阅 `TableEvent` 处理选择、排序、分页和应用操作。`TableAction` 提供主要操作与菜单操作，处理逻辑由所属视图负责。`set_rows` 保留仍有效的选中 ID，并将当前页限制在有效范围内；`set_theme` 更换表面配色时保留表格状态。

## Markdown 文档编辑器

`DocumentEditor` 将原生源码编辑、`CodeCard` 和可编辑的 `DataTable` 单元格组合成以稳定块 ID 组织的 Markdown 文档。目录默认展开；点击标题旁的 `«` 隐藏目录，为正文留出空间，点击左上角的 `» 目录` 恢复目录。应用工具栏在编辑器外部组合，可使用 `undo`、`redo`、`finish_editing`、`is_preview`、`set_preview` 和 `markdown`。展示程序默认打开文档页，将编辑内容保存到主题配置旁的 `document.md`，下次启动时恢复。

```rust
use gpui_glint::components::{DocumentEditor, DocumentEvent, bind_document_editor_keys};

bind_document_editor_keys(cx); // 同时注册代码卡快捷键
let document = cx.new(|cx| DocumentEditor::new(cx, "# 标题\n\n正文"));
// 将 document.clone() 放入高度受限的父容器中。
// 订阅 DocumentEvent::Changed(markdown) 以保存完整源码。
// 订阅 DocumentEvent::LinkActivated(url) 以处理链接跳转。
```

CommonMark / GFM 渲染支持标题、粗体、斜体、删除线、行内代码、链接、图片、嵌套列表、任务列表、引用、表格、围栏代码、脚注和分隔线。正文编辑时保留格式，点击即可定位光标，无需切换布局或显示 Markdown 标记。选中文字后显示浮动工具栏，可设置粗体、斜体、删除线、代码、链接或清除格式。代码与表格单元格使用各自的原生编辑器；块菜单包含表格行列操作和显式的 Markdown 源码编辑入口。预览模式只读。

`DocumentEditor::set_search_query(query, cx)` 为文档标题、正文、代码文本和表格单元格中的所有匹配词添加背景高亮，不区分大小写，编辑与预览模式均支持。传入空字符串清除高亮，`search_query()` 读取当前关键词。高亮仅影响显示，保留 Markdown 格式、链接和选区，不修改文档、不产生 `Changed` 事件，也不进入撤销历史；新建或重新加载的文本块继承当前关键词。独立的源码编辑器可使用 `CodeCard::set_search_query(query, cx)`。

`CodeCard::search_match_count()` 返回当前文本的匹配数量，`reveal_search_match(index, cx)` 加深指定匹配的高亮，索引从 0 开始，越界返回 `false`。使用 `.viewport()` 的代码卡会同时纵向、横向滚动到该匹配，不改变键盘焦点、文本选区或撤销历史，适合由宿主组合长响应的搜索栏。

```rust
document.update(cx, |editor, cx| editor.set_search_query("关键词", cx));
// 清除：editor.set_search_query("", cx)
```

跨文本块拖动，或按住 Shift 点击另一块，可扩展文档选区。Backspace / Delete 在一次撤销步骤中删除所选文本及完全覆盖的块，并保留两端未选中的文本。兼容的文本片段按起始块的样式合并，代码围栏和表格保留结构。⌘ + C / X 复制或剪切选中的纯文本；Esc 清除文档选区。空文档始终保留一个可编辑段落。

滚动时复用 Markdown 解析、目录标签、字符计数以及文本换行布局；搜索匹配仅在文本或关键词变化后重新计算。正文只渲染视口附近的块及正在编辑的块，屏幕外保留已测量的高度，目录使用虚拟列表。首次加载、块结构变化和窗口宽度变化时重新测量高度；单个超长块仍需完整布局。跨块选区保留完整文档模型，鼠标命中仅使用本次渲染的文本坐标。手动运行 CPU 滚动基准：`cargo test --test document_performance -- --ignored --nocapture`；该基准不包含真实 GPU 绘制和显示帧率。

原始 HTML 和数学内容保留为文本，不执行 HTML，也不进行 TeX 排版。Mermaid 图表由下文的 `MermaidDiagram` 渲染。远程图片依赖宿主应用配置的 GPUI HTTP 客户端。

悬停时显示块左侧的操作手柄；标题块显示 `H1`～`H6`，便于确认层级。块菜单与 `/` 菜单的滚动不会传播到文档容器。点击可打开编辑、复制、创建副本、转换、移动和删除命令；拖动可调整块顺序。空块的 `/` 菜单支持中文或英文过滤（如 `/code`、`/table`、`/quote`），使用方向键和 Enter 选择。Enter 拆分正文块，Shift + Enter 插入换行，块首的 Backspace 合并到前一个正文块，⌘ / Ctrl + Enter 插入下一个块。文档撤销与重做涵盖单元格编辑和结构变更。表格支持编辑表头与单元格、添加行列，以及移除最后一行或一列。`markdown()`、`blocks()`、`set_markdown()`、`move_block()`、`undo()` 和 `redo()` 向宿主应用提供文档状态与操作。

## Mermaid 图表

`MermaidDiagram` 使用 [mermaid-rs-renderer 0.3.1](https://docs.rs/mermaid-rs-renderer/0.3.1/mermaid_rs_renderer/) 和 resvg，以原生方式离线渲染图表。语法支持取决于上游解析器，包括流程图、时序图、类图和状态图，不保证与 mermaid.js 完全兼容。系统字体用于显示中文标签；该离线视图不支持外部图片节点。

```rust
let diagram = cx.new(|_| {
    MermaidDiagram::new("flowchart LR\n A[开始] --> B[结束]")
        .theme(ThemePreset::Sage)
        .max_height(480.)
});
// 运行时更新： diagram.update(cx, |view, cx| view.set_source(source, cx));
let svg = render_mermaid("flowchart LR; A-->B", ThemePreset::Sage)?;
// svg.svg 包含独立 SVG；diagram.read(cx).svg() 提供已完成的渲染结果。
```

渲染在后台执行，并采用 150ms 防抖。更新源码会使旧结果失效，过期任务无法覆盖当前源码。错误保持可见，不修改文档。栅格输出每边最多 4096 像素；源码限制为 256KiB / 2000 行、512 个节点和 1024 条边。

`DocumentEditor` 自动渲染 `mermaid` 围栏块，包括列表和引用内的围栏。左上角的分段控件用于切换图表与源码；源码在编辑模式可编辑，在预览模式只读。在空块输入 `/mermaid` 可插入示例。Markdown 导出保留围栏及源码，包括无效语法。运行 `cargo run --example mermaid` 查看流程图、时序图和错误状态示例。

`MermaidDiagram::source_editor(entity)` 接收由宿主维护的 `CodeCard`，保留其编辑事件和历史。未提供编辑器时，内置源码视图只读。左上角的“渲染 / 源码”切换使用共享的 `SegmentedControl`，与右上角的更多图标一起在悬停时显示。更多菜单可复制 Mermaid 源码，或通过原生保存对话框将当前图表导出为 PNG。渲染成功后才允许导出；`png()` 也可向宿主提供渲染后的 PNG 字节。

操作反馈由宿主负责。订阅 `MermaidEvent::SourceCopied(source)`、`PngExported(path)` 和 `ExportFailed(error)`，可显示通知或接入其他界面。嵌入文档的图表通过 `DocumentEvent::Mermaid(event)` 转发事件；组件本身不显示复制或导出的状态文字。取消保存不产生事件。

```rust
let subscription = cx.subscribe(&diagram, |host, _, event: &MermaidEvent, cx| {
    if let MermaidEvent::SourceCopied(_) = event {
        // 使用宿主应用的通知界面显示提示。
    }
}); // 在宿主中保留 Subscription，防止订阅失效。
```

## WebView（可选）

启用 Cargo feature `webview` 接入 Wry 原生子视图，默认构建不引入浏览器依赖。当前适配 macOS / Windows；macOS 使用 WKWebView，Windows 路径需要实机验证，Linux 暂未接入。Wry 提供系统浏览器能力，相关 API 见 [Wry 文档](https://docs.rs/wry/0.55.1/wry/)。

```toml
gpui-glint = { version = "0.1", features = ["webview"] }
```

在 GPUI 已提供有效的 `Window` 后创建，并由宿主持有 `Entity<WebView>`：

```rust
use gpui_glint::components::WebView;
let browser = cx.new(|cx| {
    WebView::html("<h1>Hello Glint</h1>", window, cx)
        .expect("Failed to create native WebView")
});
// 宿主 render 中：div().flex_1().min_h_0().child(browser.clone())
```

`WebView::html` 展示静态 HTML / CSS，关闭 JavaScript、页面导航、新窗口和下载；不是完整网站运行模式。通过 `WebView::new(wry::WebViewBuilder, window, cx)` 配置 URL、JavaScript、IPC、权限和导航等能力，或使用 `from_native` 包装由宿主创建的 Wry 子视图。构造函数返回 `Result`，宿主应展示创建失败信息。通过 `load_html` / `load_url` 替换文档，调用 `raw()` 使用 Wry 的其他 API；加载提交成功不代表网页完成加载，需要时通过 builder 的 page-load 回调获取状态。

给 WebView 分配明确的布局区域。组件在 GPUI prepaint 中同步逻辑坐标和尺寸，支持窗口调整大小。原生视图会覆盖同区域的 GPUI 弹窗和菜单；宿主移除它的布局节点、切换页面或展示重叠弹窗时，应先 `set_visible(false, cx)`，回到页面再设为 `true`。实体释放时隐藏并释放原生视图。原生区域不支持 GPUI 圆角或任意内容裁剪；放在完全可见的矩形区域内。Windows 使用此子视图方案时可能需要宿主在 GPUI 启动前禁用 DirectComposition，不能视为已验证跨平台支持。

运行 `cargo run --example webview --features webview` 查看源码和原生预览的对照示例；运行 `cargo run -p glint-api --features webview` 使用 HTML 响应“预览”。响应源码按 Content-Type 识别 HTML，也支持缺少 Content-Type 时识别 `<!doctype html>` / `<html`；预览渲染收到的正文，不重新发送原请求。`with_html` 文档没有原请求的 URL 基址，相对路径资源可能无法加载。

## Popover

`Popover` 是受控浮层，支持任意触发器与内容，默认在触发器下方靠右对齐，并在窗口边缘自动调整位置。点击触发器切换，点击外部或按 Esc 关闭；支持键盘 Enter / Space 打开。宿主在 `on_open_change` 中更新状态并重新渲染。

```rust
Popover::new("delete-confirm", trigger, confirmation_content)
    .open(show_confirmation)
    .width(260.)
    .trigger_height(22.)
    .theme(theme)
    .on_open_change(move |open, window, cx| {
        // 更新宿主的 show_confirmation 状态。
    })
```

每个实例使用唯一且稳定的 ID。`trigger_height` 默认 30px，`gap` 默认 6px；浮层使用延后绘制，避免被表格或滚动容器裁剪。确认按钮由宿主提供；组件本身不执行删除等业务操作。`glint-api` 环境变量表格提供删除确认示例，确认后移除草稿行，保存更改后生效。

## 通知与日期选择器

`ToastHost` 管理通知堆栈，默认显示在右下角。通过 `.position(ToastPosition::BottomRight / TopRight / TopCenter)` 可配置右下方、右上方或顶部居中；运行时调用 `set_position(position, cx)` 可移动现有通知，保留其剩余显示时间。在采用相对定位、填满容器的应用根元素中挂载一次，并放在滚动内容之外。`Toast::new` 默认创建持续四秒的信息通知。使用 `kind(ToastKind::{Info, Success, Warning, Error})`、`description(...)` 和 `duration(Some(duration))` 自定义通知；`duration(None)` 使其持续显示。悬停暂停剩余时间。堆栈默认最多显示三条，可通过 `max_visible` 配置；溢出时关闭最旧的一条。`push` 返回 `ToastId`，`dismiss` 移除一条，`clear` 清空全部。订阅 `ToastEvent::Dismissed { id, reason }` 可获取手动关闭、超时或溢出的原因。

```rust
use gpui_glint::components::{Toast, ToastHost, ToastKind, ToastPosition};
let toasts = cx.new(|_| ToastHost::new()
    .theme(ThemePreset::Sage)
    .position(ToastPosition::TopCenter));
toasts.update(cx, |host, cx| {
    host.push(Toast::new("已复制源码").kind(ToastKind::Success), cx);
});
// 宿主渲染： div().relative().size_full().child(content).child(toasts.clone())
```

`DatePicker` 提供单日期选择，使用周一为首日的六周日历，支持相邻月份日期、月份切换、今天、清空、禁用状态，以及包含端点的最小和最大日期限制。`Date` 是导出的 `chrono::NaiveDate` 类型；“今天”使用机器的本地时区。触发按钮显示 `YYYY-MM-DD`。反向范围会禁用所有日期；修改范围时会清除超出范围的值。用户选择后由组件提交，并发出 `DatePickerEvent::Changed(Option<Date>)`。程序调用 `set_selected` 和 `set_range` 时会校验，但不发出变更事件。

```rust
use gpui_glint::components::{Date, DatePicker, DatePickerEvent};
let picker = cx.new(|cx| DatePicker::new(cx)
    .range(Date::from_ymd_opt(2026, 10, 1), Date::from_ymd_opt(2026, 10, 31))
    .selected(Date::from_ymd_opt(2026, 10, 4).unwrap())
    .theme(ThemePreset::Sage));
// 订阅 DatePickerEvent::Changed，并在宿主中保留 Subscription。
```

聚焦后按 Enter、空格或 ↓ 打开选择器。方向键按天或周移动，PageUp / PageDown 切换月份，Enter 确认，Esc 或点击外部关闭。无需额外注册快捷键。运行时 API 包括 `value`、`set_selected`、`set_range`、`set_disabled`、`set_theme` 和 `close`。

运行 `cargo run --example feedback` 查看独立的日期与通知示例，或打开展示程序的“输入与选择”页。Markdown 编辑器示例通过 `DocumentEvent::Mermaid` 接收复制和导出事件，并使用 `ToastHost` 显示通知。

## 头像与头像组

`Avatar` 支持图片、姓名首字母、自定义文字，以及空值时的用户图标。默认形状为 `AvatarShape::Circle`；`Rectangle` 使用圆角矩形。`size` 设置正方形尺寸，`dimensions` 可设置宽矩形，圆形则使用较小的尺寸作为直径。图片采用覆盖裁剪，加载中或加载失败时回退到文字或图标。`image` 接收 GPUI `ImageSource`，包括 `PathBuf`、内嵌资源、URL 或解码后的图片；远程加载依赖宿主 HTTP 客户端。`theme`、`colors` 和 `ring` 自定义外观，`text_size` 覆盖自动字号。姓名首字母保留 Unicode 字素，包括组合 emoji 和带重音字符；`text` 覆盖自动生成的首字母。

```rust
use gpui_glint::components::{Avatar, AvatarGroup, AvatarShape};
let avatar = Avatar::new("Ada Lovelace").size(40.);
let team = Avatar::new("研发团队").text("研发")
    .shape(AvatarShape::Rectangle).dimensions(64., 40.);
let members = ["林晓雨", "Ada Lovelace", "陈晨", "Grace Hopper", "周舟", "吴桐"];
let group = AvatarGroup::new(members.map(Avatar::new))
    .shape(AvatarShape::Circle).size(40.).max_visible(4).overlap(8.);
// 两种组件均实现 IntoElement；render() 返回 Div，方便宿主设置样式。
```

头像组统一继承形状与尺寸，默认重叠 25%，并使用 2px 分隔环。后面的头像绘制在前面的头像之上。`overlap(0.)` 使头像相邻而不重叠；重叠距离最多为头像尺寸的一半。`max_visible` 默认显示五名成员，其后用 `+N` 头像表示隐藏人数。设为 0 时仅显示计数；空组不显示计数头像。`ring_color` 应与头像组背后的表面颜色匹配；`theme` 应用统一配色，同时保留成员显式指定的颜色。头像组水平排列且不换行，在空间受限的区域中请使用 `max_visible`。

运行 `cargo run --example avatars` 查看图片与文字头像、图片失败回退，以及圆形和矩形头像组，或打开展示程序的“徽标与标签”页。

## 时间、日期时间与范围选择器

`TimePicker` 通过三个滚动列选择 `HH:mm:ss`，使用 24 小时制。`DateTimePicker` 在同一个弹层中组合日历和时间列，选择 `YYYY-MM-DD HH:mm:ss`。`DateRangePicker`、`TimeRangePicker` 和 `DateTimeRangePicker` 均使用一个范围输入框和统一弹层：起止日历或时间列共享草稿、校验及一次确认，范围弹层中不嵌套独立的 `DatePicker` 或 `TimePicker` 触发按钮。弹层保持在窗口边界内。

所有选择器支持禁用、清空、主题、占位文字、宽度，以及包含端点的最小和最大值限制。确认提交草稿，取消、Esc 或点击外部放弃草稿，清空提交 `None`。用户变更发出 `TimePickerEvent::Changed`、`DateTimePickerEvent::Changed` 或 `RangePickerEvent::Changed`；同时导出三种范围选择器各自的事件别名。程序设置值时会校验，但不发出事件。

`Time` 和 `DateTime` 分别为 `chrono::NaiveTime` 与 `chrono::NaiveDateTime`；日期时间不含时区，时间值统一到整秒。`ValueRange::new(start, end)` 允许两端相等，拒绝反向范围。纯时间范围描述同一天，跨午夜请使用日期时间范围。日期时间边界还会约束边界日期的时、分、秒。

```rust
use gpui_glint::components::{
    Date, Time, TimePicker, DateTimePicker, DateTimeRange, DateTimeRangePicker,
};
let day = Date::from_ymd_opt(2026, 10, 5).unwrap();
let time = cx.new(|cx| TimePicker::new(cx)
    .selected(Time::from_hms_opt(9, 30, 15).unwrap()));
let datetime = cx.new(|cx| DateTimePicker::new(cx)
    .selected(day.and_hms_opt(10, 30, 15).unwrap(), cx));
let range = cx.new(|cx| DateTimeRangePicker::new(cx)
    .range(Some(day.and_hms_opt(8, 0, 0).unwrap()), None, cx)
    .selected(DateTimeRange::new(
        day.and_hms_opt(23, 59, 58).unwrap(),
        day.succ_opt().unwrap().and_hms_opt(0, 0, 2).unwrap(),
    ).unwrap(), cx));
// 在宿主中保留事件订阅，将各实体作为元素挂载。
```

聚焦输入框后按 Enter、空格或 ↓ 打开，Enter 确认，Esc 取消。范围弹层中 Tab 切换当前端点。仅日期模式下，方向键移动当前日期，PageUp / PageDown 切换月份。时间列中，← / → 选择列，↑ / ↓ 调整值。`TimePicker` 还支持在当前列输入数字。运行时 API 包括 `set_selected`、`set_range`、`set_disabled`、`set_theme` 和 `close`。`DateTimePicker::value` 需要应用上下文；范围和时间选择器可直接通过 `value()` 获取值。

运行 `cargo run --example temporal` 体验所有选择器，并通过宿主通知查看变更事件。展示程序的“输入与选择”页也包含这些组件。

## 命令面板

`CommandPalette` 提供可搜索的命令列表：36px 单行结果，图标、名称和可选的快捷键提示同排。搜索匹配名称和关键词；分组按首次出现的顺序排列，禁用命令无法选中或执行。注册一次 `bind_text_entry_keys` 即可支持原生搜索编辑。

```rust
use gpui_glint::components::{CommandItem, CommandPalette, CommandPaletteEvent, Icon};

let commands = cx.new(|cx| CommandPalette::new([
    CommandItem::new("new-collection", "新建集合")
        .group("常用动作").icon(Icon::Plus)
        .keywords("create collection").shortcut("⌘ N"),
    CommandItem::new("settings", "打开设置")
        .group("设置").icon(Icon::Settings),
], cx));
let subscription = cx.subscribe(&commands, |_, _, event: &CommandPaletteEvent, _| {
    match event {
        CommandPaletteEvent::Executed(id) => { /* 按稳定的命令 ID 分发操作 */ }
        CommandPaletteEvent::Dismissed => { /* 浮层已取消 */ }
    }
});
```

直接渲染实体可显示内嵌面板。浮层模式下，调用 `commands.update(cx, |palette, cx| palette.open(window, cx))`，并在采用相对定位、填满容器的宿主中，将 `palette.overlay(cx)` 放在内容上方。观察该实体，使宿主在打开状态变化时重新渲染。浮层阻止下方内容接收指针输入，将 Tab 焦点限制在搜索框中，并在执行、按 Esc 或点击遮罩关闭后恢复原焦点。方向键循环选择可用结果，并将当前行滚动到可见位置。输入法组合输入期间，Enter 和导航键交由输入法处理。内嵌模式下 Esc 清空搜索。

应用负责注册 ⌘ / Ctrl + K 和分发实际命令；快捷键标签仅作提示，不自动注册绑定。`set_items`、`set_query` 和 `set_theme` 支持运行时更新。打开展示程序的“命令面板”页，或在任意示例页按 ⌘ / Ctrl + K 体验浮层。组件使用统一的主题表面、边界、文字和强调色。

## 选择输入框

`SelectField` 将现有
`Select` 与 `TextEntry` 组合为一个 32px 高的输入框。选择器固定宽 92px，
文本输入占据剩余空间，两个字段共用边界与焦点环。下拉仍使用 `Select` 的选项、
搜索基础设施和键盘行为，发送按钮由宿主单独组合，使用标准的 30px `Button`。

```rust
use gpui_glint::components::{SelectField, SelectFieldEvent, SelectOption};

let request = cx.new(|cx| {
    let mut field = SelectField::new(
        ["GET", "POST", "PUT", "PATCH", "DELETE"]
            .map(|method| SelectOption::new(method, method)),
        "输入请求 URL 或路径",
        cx,
    );
    field.set_text("{{base_url}}/v1/resources", cx);
    field
});
let subscription = cx.subscribe(&request, |_, _, event: &SelectFieldEvent, _| {
    let SelectFieldEvent::Changed(value) = event;
    // value.selected 是稳定的选项值；value.text 是输入原文。
});
```

使用前注册一次 `bind_text_entry_keys(cx)`，并在宿主中保留订阅。
`value()` 返回两个字段的当前值。`set_selected`、`set_text`、`set_options`、
`set_disabled`、`set_error` 和 `set_theme` 支持运行时更新；程序赋值不发出
`Changed`，用户修改会提交组件状态并发出完整值。默认选择首个非禁用选项。
未知选项显示 Select 的占位文字。`set_disabled` 禁用整个组件时两个字段都不可编辑；
`.input_disabled(true)` 或 `set_input_disabled(true, cx)` 仅禁用文本输入，下拉仍可操作，原文本保留。重新启用即可继续编辑；整体禁用优先于输入框配置。输入框被禁用时，其焦点会转移到仍可操作的下拉框。
`set_error(Some(message))` 显示红色边框与字段下方的说明，传 `None` 清除。

将组件放入宽度确定的容器或弹性行中；路径在输入区域内滚动，
Select 弹层不受组合框裁剪；模板变量按原文显示。
展示程序的“输入与选择”页提供方法与路径组合及模拟发送反馈。

Select 选项支持 `.color(0xRRGGBB)` 配置文字颜色，普通 Select 与
SelectField 共用此配置。触发器显示当前选项的颜色；下拉中非激活行使用
各自的颜色，键盘激活与鼠标悬停统一为蓝底白字，禁用状态优先使用灰色。
未配置颜色时保留默认配色。

```rust
let methods = [
    SelectOption::new("GET", "GET").color(0x24864c),
    SelectOption::new("POST", "POST").color(0xb37716),
    SelectOption::new("DELETE", "DELETE").color(0xd9342b),
];
let request = cx.new(|cx| SelectField::new(methods, "输入请求路径", cx));
```

## AI 输入框

`Composer` 是面向 AI 对话的提示词输入框：上层是自动换行、随内容增高的
多行编辑器（`PromptEditor`，支持 IME、选择、剪贴板与方向键换行），
下层是工具条——左侧附件 chips、右侧状态 spinner、模型 chips 和发送按钮。
浅色主题使用纯白浮层，深色主题使用各主题专属的菜单玻璃并叠加背景模糊。

```rust
use gpui_glint::components::{Composer, ComposerChip, ComposerEvent, Icon};

let composer = cx.new(|cx| {
    Composer::new("提出后续修改要求…", cx)
        .theme(theme)
        .chip(ComposerChip::new("attach").icon(Icon::Plus))
        .chip(
            ComposerChip::new("access")
                .icon(Icon::Shield)
                .option("normal", "普通访问")
                .option("full", "完全访问")
                .default_selected("full")
                .tint(0xb37716)
                .chevron(),
        )
        .menu_chip(ComposerChip::new("model").label("GLM-5.3-Flash").chevron())
});
let subscription = cx.subscribe(&composer, |this, _, event: &ComposerEvent, cx| {
    match event {
        ComposerEvent::Submitted(text) => { /* 发送提示词 */ }
        ComposerEvent::Queued(text) => { /* 提示词进入队列 */ }
        ComposerEvent::ChipSelected { chip, value } => { /* 切换模型或档位 */ }
        ComposerEvent::ChipClicked(id) => { /* 打开附件等无选项 chip */ }
        ComposerEvent::Changed(text) => { /* 同步草稿 */ }
    }
});
```

使用前注册一次 `bind_prompt_editor_keys(cx)` 与 `bind_composer_keys(cx)`：
Enter 与 ⌘/Ctrl-Enter 触发提交，Shift-Enter 插入换行，编辑快捷键全部限定在
组件自己的 key context 内。发送按钮默认在有内容时可用；`send_enabled(bool)`
可接管该状态，`busy(true)` 显示加载指示，`clear_on_submit(false)` 保留原文。
chips 只负责样式与点击事件，弹出菜单由宿主用 `GroupedDropdown` 自行组合；
`set_theme` 与 `set_value` 支持运行时更新。展示程序的“输入与选择”页提供
完整演示：提交后短暂显示 spinner 并弹出 Toast。

带 `.option(value, label)` 的 chip 表现为 Select：点击在 chip 上方弹出
选项菜单，选中行标勾，chip 标签显示当前选项，选择通过
`ComposerEvent::ChipSelected` 上报；菜单自带 `occlude`，滚动与点击都不会
穿透到页面。

队列：busy 期间提交的提示词自动进入队列（`ComposerEvent::Queued`），以行
形式排在输入框上方，每行提供 立即（越序提交，`Submitted`）、编辑（回填到
输入框）与删除三个操作；宿主调用 `set_busy(false)` 结束当前请求时，最旧的
排队提示词会自动作为 `Submitted` 发出，直至队列清空。`queue_len()` 返回
当前排队数量。

上下文面板（@ 面板）：用 `.context_panel(trigger, title, sections)` 声明
后，点击对应 id 的 chip 会在卡片上方弹出面板——分组条目带图标与灰色描述，
底部是 `.context_hints(...)` 声明的快捷键提示。面板自身 `occlude`，滚动与
点击不会穿透，Esc / 点击外部 / 选中条目均可关闭。选中条目会自动把彩色的
`@label` 提及插入提示词（`PromptEditor` 对内容中的 `@word` 记号按注册的
颜色着色，编辑后自动重算，未注册的 `@` 记号用强调色），并发出
`ComposerEvent::ContextSelected { id, label }`；`add_mention(label, color)`
可供宿主在面板之外追加提及。

## 分组列表

`GroupList` 是侧边栏的分组 source list：分组头（图标 + 标题）与条目
（图标 + 文本 + 尾部元信息）。悬停分组头浮现尾部操作按钮，悬停条目时
前导图标可切换为快捷操作（如收藏）、尾部元信息切换为操作图标（如删除），
选中项以胶囊高亮。条目 id 需跨分组唯一。

`GroupListItem::prefix("GET").prefix_color(color)` 可在条目名称前显示带颜色的短标签，适用于 API 方法。

```rust
use gpui_glint::components::{
    GroupList, GroupListGroup, GroupListHeader, GroupListItem, GroupListEvent, Icon,
};

let list = cx.new(|cx| {
    GroupList::new([GroupListGroup::new(
        GroupListHeader::new("gpui-glint", "gpui-glint")
            .icon(Icon::Folders)
            .actions([Icon::More, Icon::List, Icon::Plus]),
    )
    .item(
        GroupListItem::new("dark-sidebar", "侧栏黑暗主题适配")
            .icon(Icon::Pulse)
            .meta("刚刚"),
    )
    .item(
        GroupListItem::new("closable-tabs", "可关闭标签页样式修改")
            .icon(Icon::Text)
            .meta("1天")
            .hover_icon(Icon::Star)
            .action(Icon::Trash),
    )])
    .selected(Some("dark-sidebar".into()))
    .theme(theme)
});
let subscription = cx.subscribe(&list, |_, _, event: &GroupListEvent, cx| {
    match event {
        GroupListEvent::ItemSelected { group, item } => { /* 打开条目 */ }
        GroupListEvent::ItemAction { group, item } => { /* 删除等快捷操作 */ }
        GroupListEvent::HeaderClicked { group } => { /* 折叠或跳转 */ }
        GroupListEvent::HeaderAction { group, index } => { /* 新建等操作 */ }
    }
});
```

组件自管悬停与选中：点击条目先更新内部选中再发出 `ItemSelected`，
宿主可用 `set_selected` 接管。点击分组头会折叠/展开该组（`HeaderClicked`
照常发出）；`collapsed(["rimworld"])` 声明初始收起的分组，
`set_collapsed` / `toggle_group` / `is_collapsed` 支持运行时控制。
`set_groups` 与 `set_theme` 支持运行时更新；
`width(px)` 固定宽度，默认填满容器。展示程序的“标签页”页提供演示。

## 标签页

`Tabs` 支持两种形态，由 `.kind(TabsKind::...)` 切换：
`Underline`（默认，活动项为主题色下划线）与 `Pill`（筛选胶囊：活动项为
圆角胶囊底色 + 加粗标签，非活动项保持安静文本，悬停出现淡底）。
`TabOption` 可带 `.icon(...)`、`.count("14")` 与 `.disabled(true)`；
父级持有选中索引并在 `on_change` 回调里更新。

## 代码 Diff 卡

`DiffCard` 用于查看代码改动：文件头（图标 + 文件名 + 目录 + `+N -M` 统计 +
整体折叠）之下是带行号的统一 diff——新增行绿底实线边条、删除行红底虚线
边条，成对的增删行还会做词级对比高亮（改动的词以更强的红/绿底标出），
语法高亮与 `CodeCard` 同源。较长的未修改段落折叠为 “N unmodified lines”
展开条，点击展开。组件自带行级 LCS diff，也可以用 `from_lines` 直接喂
预计算好的行。

```rust
use gpui_glint::components::DiffCard;

let diff = cx.new(|cx| {
    DiffCard::new(cx, "rust", old_source, new_source)
        .name("mod.rs")
        .path("src/components/")
        .theme(theme)
});
// 运行时更新：diff.set_diff(old, new, cx) 或 set_lines(rows, cx)
```

展开状态为组件内部状态，`set_diff` / `set_lines` 会重置；`expand_all`
一键展开全部折叠块。亮暗主题自动适配（`DiffColors::LIGHT/DARK`）。
展示程序的“文档代码卡”页提供 mod.rs 重命名场景的演示。

## 可收缩面板

`CollapsePanel` 是 AI 回复中的折叠块：触发器行为可选前缀图标 + 摘要 +
旋转箭头（展开 ⌄ / 收起 ›），展开后内容区带左侧细线缩进。内容是一个
渲染时调用的构建闭包（`CollapsePanelContent`），便于把流式思考文本、
代码预览卡（如 `CodeCard`）或终端输出实时填进去。

```rust
use gpui_glint::components::{CollapsePanel, CollapsePanelEvent, Icon};

let panel = cx.new(|cx| {
    CollapsePanel::new("思考 · 持续了 11 秒")
        .icon(Icon::Pulse)
        .theme(theme)
        .content(|_, cx| {
            gpui::div()
                .text_size(gpui::px(13.5))
                .child(thinking_text)
                .into_any_element()
        })
});
let subscription = cx.subscribe(&panel, |_, _, event: &CollapsePanelEvent, _| {
    let CollapsePanelEvent::Toggled { expanded } = event;
});
```

`set_summary` 更新摘要（如耗时），`set_content` 更新内容构建器，
`set_expanded` / `toggle` 由宿主或用户交互控制并以 `Toggled` 事件上报，
`set_theme` 运行时切换主题。内容仅在展开时构建，折叠时不构建内容。
展示程序的“文档代码卡”页提供思考文本、终端输出与代码预览三种演示。

## 主题选择器

`ThemeSelector` 默认列出 `ThemePreset::ALL` 的全部主题。通过 `.themes(...)` 设置可选列表，`set_themes(..., cx)` 在运行时替换；列表按传入顺序显示，重复主题只保留第一次出现的位置，`available_themes()` 读取当前选项。

```rust
use gpui_glint::components::{ThemeSelector, ThemeSelectorEvent};
use gpui_glint::theme::ThemePreset;

let selector = cx.new(|cx| {
    ThemeSelector::new(cx)
        .themes([ThemePreset::Moonstone, ThemePreset::DarkObsidian])
        .selected(ThemePreset::Moonstone)
        .theme(ThemePreset::Moonstone)
});
let subscription = cx.subscribe(&selector, |_, selector, event: &ThemeSelectorEvent, cx| {
    let ThemeSelectorEvent::Changed(theme) = event;
    selector.update(cx, |selector, cx| {
        selector.set_selected(*theme, cx);
        selector.set_theme(*theme, cx);
    });
    // 同步宿主应用及其他组件的主题，并保留 Subscription。
});
selector.update(cx, |selector, cx| {
    selector.set_themes([ThemePreset::Moonstone, ThemePreset::DarkObsidian], cx);
});
```

选中值由宿主通过 `set_selected` 提交，用户选择发出 `Changed`。修改列表不会改变选中值，也不会发出事件；当前主题不在列表中时仍显示在触发器上，展开后从首项开始键盘导航。空列表会关闭弹层并禁止展开。方向键、Home / End 和 Enter 只操作当前列表中的主题。

## 主题

主题预设、亮暗切换和子组件同步方式见 [README 的主题说明](../README.md#主题)。
