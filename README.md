# Glint UI

[![crates.io](https://img.shields.io/crates/v/gpui-glint.svg)](https://crates.io/crates/gpui-glint)
[![Documentation](https://docs.rs/gpui-glint/badge.svg)](https://docs.rs/gpui-glint)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Glint UI（`gpui-glint`）是基于 GPUI-CE 的 Rust UI 组件库，提供统一的亮暗主题、原生输入与常用桌面应用组件。视觉上采用柔和高光、细腻边框和玻璃材质。

主包仅提供 lib。组件从 `gpui_glint::components` 导入，主题从 `gpui_glint::theme` 导入；窗口创建、应用状态、业务操作和数据持久化由宿主应用负责。Playground 和 Markdown 工作区都是独立示例。

## 接入

需要 Rust 1.95 或更新版本。本仓库的 `rust-toolchain.toml` 使用 stable 工具链。

在应用的 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
gpui-glint = "0.1.3"
gpui = { package = "gpui-ce", version = "=0.2.2" }
gpui_platform = { package = "gpui_ce_platform", version = "=0.1.0", features = ["font-kit", "wayland", "x11"] }
```

`gpui` 是 `gpui-ce` 的导入别名。`gpui_platform` 由宿主应用添加，用于启动原生应用；它在组件库仓库中仅作为开发依赖。

需要原生网页渲染时，使用 `gpui-glint = { version = "0.1.3", features = ["webview"] }`；该功能目前适配 macOS 和 Windows。开发库本身时可使用 `gpui-glint = { path = "../gpui-glint" }`。

### 最小应用

注册 `GlintAssets` 后即可使用内置 SVG 图标。以下代码可放入应用的 `src/main.rs`：

```rust
use gpui::{App, AppContext, Context, Render, Window, WindowOptions, div, prelude::*};
use gpui_glint::{
    components::{Button, ButtonKind, GlintAssets, Icon},
    theme::ThemePreset,
};

struct Demo;

impl Render for Demo {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemePreset::Sage;
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(gpui::rgb(theme.palette().canvas))
            .text_color(gpui::rgb(theme.palette().text))
            .child(Icon::Markdown.render(20.))
            .child(Button::new("新建文档").kind(ButtonKind::Primary).theme(theme))
    }
}

fn main() {
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| {
            cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Demo))
                .unwrap();
            cx.activate(true);
        });
}
```

如果应用已有资源加载器，将其与 `GlintAssets` 合并，保留 `glint-ui/icons/*.svg` 命名空间。

## 组件

| 分类 | 组件 |
| --- | --- |
| 布局与导航 | `AppLayout`、`Tabs`、`ClosableTabs`、`SideTabs`、`GroupList` |
| 按钮与提示 | `Button`、`IconButton`、`Tooltip` |
| 输入与选择 | `TextEntry`、`InputField`、`SearchInput`、`Select`、`SelectField`、`TokenInput`、`SegmentedControl` |
| 基础表单 | `Checkbox`、`RadioOption`、`Switch`、`Slider`、`Stepper` |
| 菜单与命令 | `GroupedDropdown`、`CommandPalette` |
| 日期与时间 | `DatePicker`、`TimePicker`、`DateTimePicker`、`DateRangePicker`、`TimeRangePicker`、`DateTimeRangePicker` |
| 内容展示 | `DataTable`、`CodeCard`、`DiffCard`、`CollapsePanel` |
| 文档与图表 | `DocumentEditor`、`MermaidDiagram` |
| 对话输入 | `Composer`、`PromptEditor` |
| 状态与身份 | `ToastHost`、`Avatar`、`AvatarGroup`、`ContentBadge`、`TagBadge`、`CountBadge`、`OverlayBadge`、`StatusBadge` |
| 图标与主题 | `Icon`、`GlintAssets`、`ThemeSelector`、`ThemePreset` |

组件的代码片段、事件、键盘行为和运行时 API 见 [组件使用指南](docs/components.md)。完整类型与方法可通过 `cargo doc --no-deps --open` 查看。

## 状态与事件

简单组件通过 builder 配置并作为 GPUI 元素渲染；输入、表格和编辑器等有状态组件通过 `cx.new(...)` 创建 `Entity`。宿主持有实体，使用 `update` 调用运行时方法，并通过 `cx.subscribe` 处理用户操作。

例如，创建一个原生文本输入框：

```rust
use gpui_glint::components::{SearchInput, TextEntry, bind_text_entry_keys};

// 应用启动时注册一次。
bind_text_entry_keys(cx);

// 在宿主视图中创建、持有并渲染。
let query = cx.new(|cx| TextEntry::new(cx, "搜索", ""));
let input = SearchInput::new("搜索").control(query.clone()).render();
```

这些片段中的 `cx` 来自宿主的 GPUI 上下文。事件订阅返回的 `Subscription` 也需要保存在宿主中，否则订阅会失效。组件是否自行提交状态取决于具体组件，例如 `Select` 使用受控选择，日期选择器会提交用户选择；详见组件指南。

按使用的编辑器注册快捷键：

| 使用场景 | 启动时调用 |
| --- | --- |
| 单行输入、选择器搜索、命令搜索 | `bind_text_entry_keys(cx)` |
| 独立代码编辑器 | `bind_code_card_keys(cx)` |
| Markdown 文档编辑器 | `bind_document_editor_keys(cx)`，同时注册代码卡与单行输入快捷键 |
| 对话输入框 | `bind_prompt_editor_keys(cx)` 和 `bind_composer_keys(cx)` |

## 主题

`ThemePreset` 提供六种浅色和五种深色配色，默认使用 `Sage`：

| 配色 | 浅色 | 深色 |
| --- | --- | --- |
| 雾白 | `FogWhite` | `DarkFogWhite` |
| 浅湾 | `ShallowBay` | `DarkShallowBay` |
| 鼠尾草 | `Sage` | — |
| 亚麻 | `Linen` | `DarkLinen` |
| 银灰 | `SilverGray` | `DarkSilverGray` |
| 月光白 / 曜石黑 | `Moonstone` | `DarkObsidian` |

使用 `.theme(theme)` 设置初始主题，有状态组件使用 `set_theme(theme, cx)` 更新。`palette()` 提供表面、文字、边界和强调色，`is_dark()` 用于自定义内容。

`AppLayout` 与子组件分别接收主题。组合组件（如文档编辑器、复合输入和命令面板）会同步内部子组件；宿主仍需更新自己持有的独立组件。`CodeCard` 随主题切换亮暗配色，也支持 `.dark(bool)` 显式覆盖。

`Icon` 默认继承父容器文字颜色，也可通过 `.text_color(...)` 指定。内置语言图标覆盖 Markdown、Rust、Go、JS/TS、Python、Java、C/C++、HTML/CSS、JSON、Shell、SQL、YAML；`Icon::for_language("md")` 支持常用别名，未知语言回退到通用代码图标。

`ThemeSelector` 默认包含全部主题，可用 `.themes([ThemePreset::Moonstone, ThemePreset::DarkObsidian])` 限制可选列表，或通过 `set_themes(..., cx)` 动态更新。详细行为见 [主题选择器](docs/components.md#主题选择器)。

玻璃背景使用 GPUI-CE 内置的 `.backdrop_blur(...)`，无需本地 GPUI 补丁。主题选择与持久化由宿主负责。

## 文档与代码组件

- `CodeCard` 提供原生输入法、选区、剪贴板、撤销重做、固定行号和横向滚动。Rust、Go、JSON、HTML、XML、CSS、JavaScript 支持语法高亮，其他语言显示纯文本；JSON 支持实时校验和格式化。语言图标覆盖范围与高亮支持范围分别配置。
- `DocumentEditor` 提供 Markdown 块编辑、目录、跨块选区、搜索高亮和只读预览。悬停标题时显示 `H1`～`H6`；块菜单支持转换、复制、移动和删除。通过 `DocumentEvent` 接入保存、标题变更和链接操作，组件不自行写入文档文件。
- `MermaidDiagram` 在后台离线渲染图表，支持源码切换、复制与 PNG 导出。语法覆盖取决于上游解析器；不执行原始 HTML，也不支持外部图片节点。文档中的 Mermaid 围栏会自动使用该组件。

远程图片需要宿主配置 GPUI HTTP 客户端；Markdown 数学内容保留为文本，不进行 TeX 排版。

## 示例

在仓库根目录运行：

| 命令 | 内容 |
| --- | --- |
| `cargo run --example playground` | 全部组件的展示与交互入口 |
| `cargo run -p glint-api` | API 工作台、请求草稿、真实 HTTP 响应与版本历史 |
| `cargo run -p glint-agent` | Agent 对话工作台、会话记录、ChatGPT 订阅与 API Provider |
| `cargo run --example webview --features webview` | HTML 源码高亮与 Wry 原生网页渲染（macOS / Windows 适配） |
| `cargo run --example mdeditor` | 本地 Markdown 工作区、文档库和保存恢复 |
| `cargo run --example mermaid` | Mermaid 图表与错误状态 |
| `cargo run --example avatars` | 头像、头像组与图片回退 |
| `cargo run --example feedback` | 通知与日期选择器 |
| `cargo run --example temporal` | 时间、日期时间与范围选择器 |

Playground 的入口和页面位于 [`examples/playground`](examples/playground)。API 工作台是独立的 workspace 应用，运行方式、Windows 安装包、发布流程与 HTTP 请求配置见 [应用指南](crates/glint-api/README.md)。Markdown 应用的快捷键、文件存储与打包方式见 [示例指南](examples/mdeditor/README.md)。

Playground 将主题选择写入 `gpui-glint/theme`，兼容读取旧的 `gpui-glass/theme`；演示文档保存在主题配置旁的 `document.md`。这些是示例应用的行为，不是组件库的默认行为。

## 开发

```sh
cargo check --lib
cargo check --examples --tests
cargo test --lib
cargo doc --no-deps
```

文档滚动的 CPU 基准需手动执行：

```sh
cargo test --test document_performance -- --ignored --nocapture
```

该基准不包含真实 GPU 绘制和显示帧率。

## 发布

发布前运行 `cargo test --lib`、`cargo check --all-targets --all-features` 和 `cargo publish --dry-run --locked`。版本变更同时记录在 [CHANGELOG.md](CHANGELOG.md)。发布包包含库源码、文档和示例，不包含设计草稿及本地产物。

## 许可证

本项目使用 [MIT 许可证](LICENSE)。

```text
src/
  lib.rs             库入口
  components/        组件与公共事件类型
  theme.rs           主题预设与颜色
examples/
  playground/        组件展示程序
  mdeditor/          Markdown 工作区示例
  ...                独立组件示例
crates/
  glint-api/         可独立打包发布的 API 工作台
  glint-agent/       原生 Agent 对话工作台与本地会话记录
docs/
  components.md      详细组件用法
tests/               集成测试与性能基准
```

可选插件体系：启用 `plugins` feature 后，可使用通用 JS 插件契约与原生 `PluginPage` 页面组件。默认 UI 库不加载 JS 引擎。详见 [插件集成](docs/plugins.md)。
