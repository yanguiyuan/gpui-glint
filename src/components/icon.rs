use crate::theme::ThemePreset;
use std::{
    borrow::Cow,
    hash::{Hash, Hasher},
};

use gpui::{
    AssetSource, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb, svg,
};

#[derive(Clone, Copy, Debug)]
pub enum Icon {
    Calendar,
    Search,
    Settings,
    Plus,
    Pencil,
    Trash,
    Copy,
    Save,
    Upload,
    Download,
    Refresh,
    Shuffle,
    Play,
    More,
    Database,
    Globe,
    Workflow,
    Pulse,
    Key,
    Folders,
    Folder,
    FolderOpen,
    Compass,
    Clock,
    User,
    Sort,
    Filter,
    Close,
    ArrowLeft,
    ArrowUp,
    ChevronDown,
    ChevronRight,
    Pause,
    Video,
    ImageGroup,
    Text,
    Link,
    ApiDoc,
    Script,
    Code,
    Markdown,
    Rust,
    Go,
    JavaScript,
    TypeScript,
    Python,
    Java,
    C,
    Cpp,
    Html,
    Css,
    Json,
    Shell,
    Sql,
    Yaml,
    Tag,
    Star,
    CheckCircle,
    HalfCircle,
    Seal,
    List,
    External,
    Eye,
    Sidebar,
    Shield,
    Spinner,
    Grip,
}

impl Icon {
    fn name(self) -> &'static str {
        match self {
            Self::Calendar => "calendar",
            Self::Search => "search",
            Self::Settings => "settings",
            Self::Plus => "plus",
            Self::Pencil => "pencil",
            Self::Trash => "trash",
            Self::Copy => "copy",
            Self::Save => "save",
            Self::Upload => "upload",
            Self::Download => "download",
            Self::Refresh => "refresh",
            Self::Shuffle => "shuffle",
            Self::Play => "play",
            Self::More => "more",
            Self::Database => "database",
            Self::Globe => "globe",
            Self::Workflow => "workflow",
            Self::Pulse => "pulse",
            Self::Key => "key",
            Self::Folders => "folders",
            Self::Folder => "folder",
            Self::FolderOpen => "folder-open",
            Self::Compass => "compass",
            Self::Clock => "clock",
            Self::User => "user",
            Self::Sort => "sort",
            Self::Filter => "filter",
            Self::Close => "close",
            Self::ArrowLeft => "arrow-left",
            Self::ArrowUp => "arrow-up",
            Self::ChevronDown => "chevron-down",
            Self::ChevronRight => "chevron-right",
            Self::Pause => "pause",
            Self::Video => "video",
            Self::ImageGroup => "imagegroup",
            Self::Text => "text",
            Self::Link => "link",
            Self::ApiDoc => "apidoc",
            Self::Script => "script",
            Self::Code => "code",
            Self::Markdown => "markdown",
            Self::Rust => "rust",
            Self::Go => "go",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Python => "python",
            Self::Java => "java",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Html => "html",
            Self::Css => "css",
            Self::Json => "json",
            Self::Shell => "shell",
            Self::Sql => "sql",
            Self::Yaml => "yaml",
            Self::Tag => "tag",
            Self::Star => "star",
            Self::CheckCircle => "check-circle",
            Self::HalfCircle => "half-circle",
            Self::Seal => "seal",
            Self::List => "list",
            Self::External => "external",
            Self::Eye => "eye",
            Self::Sidebar => "sidebar",
            Self::Shield => "shield",
            Self::Spinner => "spinner",
            Self::Grip => "grip",
        }
    }

    /// Resolve a Markdown fence language, falling back to the generic code icon.
    pub fn for_language(language: &str) -> Self {
        match language.trim().to_ascii_lowercase().as_str() {
            "md" | "markdown" | "mdown" | "mkd" => Self::Markdown,
            "rs" | "rust" => Self::Rust,
            "go" | "golang" => Self::Go,
            "js" | "javascript" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "typescript" | "tsx" | "mts" | "cts" => Self::TypeScript,
            "py" | "python" | "python3" => Self::Python,
            "java" => Self::Java,
            "c" | "h" => Self::C,
            "cpp" | "c++" | "cc" | "cxx" | "hpp" => Self::Cpp,
            "html" | "htm" => Self::Html,
            "css" | "scss" | "sass" | "less" => Self::Css,
            "json" | "jsonc" | "json5" => Self::Json,
            "sh" | "shell" | "bash" | "zsh" | "fish" => Self::Shell,
            "sql" | "mysql" | "postgresql" | "sqlite" => Self::Sql,
            "yaml" | "yml" => Self::Yaml,
            _ => Self::Code,
        }
    }

    pub fn render(self, size: f32) -> IconSvg {
        IconSvg(
            svg()
                .path(SharedString::from(format!(
                    "glint-ui/icons/{}.svg",
                    self.name()
                )))
                .size(px(size)),
        )
    }
}

impl IntoElement for Icon {
    type Element = IconSvg;

    fn into_element(self) -> Self::Element {
        self.render(16.)
    }
}

/// SVG icon which inherits the surrounding text color unless explicitly styled.
/// GPUI's raw Svg requires its own color to paint; resolve inheritance at paint
/// time so parent hover, selection and theme changes also affect the icon.
pub struct IconSvg(gpui::Svg);

impl IntoElement for IconSvg {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Styled for IconSvg {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        self.0.style()
    }
}

impl InteractiveElement for IconSvg {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.0.interactivity()
    }
}

impl IconSvg {
    fn inherit_color(&mut self, color: gpui::Hsla) -> gpui::TextStyleRefinement {
        let explicit = self.0.style().text.clone();
        let text = &mut self.0.style().text;
        if text.color.is_none() {
            text.color = Some(color);
        }
        explicit
    }

    /// Applies the same transformation supported by a raw GPUI SVG.
    pub fn with_transformation(mut self, transformation: gpui::Transformation) -> Self {
        self.0 = self.0.with_transformation(transformation);
        self
    }
}

impl gpui::Element for IconSvg {
    type RequestLayoutState = <gpui::Svg as gpui::Element>::RequestLayoutState;
    type PrepaintState = <gpui::Svg as gpui::Element>::PrepaintState;

    fn id(&self) -> Option<gpui::ElementId> {
        gpui::Element::id(&self.0)
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.0.source_location()
    }
    fn request_layout(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        self.0.request_layout(id, inspector, window, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        self.0.prepaint(id, inspector, bounds, state, window, cx)
    }
    fn paint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        layout: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        let explicit = self.inherit_color(window.text_style().color);
        self.0
            .paint(id, inspector, bounds, layout, state, window, cx);
        // Keep inheritance live if GPUI reuses this element on another frame.
        self.0.style().text = explicit;
    }
}

pub struct IconButton {
    theme: ThemePreset,
    icon: Icon,
    label: SharedString,
    plain: bool,
}

impl IconButton {
    /// Render only the icon, without a background or border, including on hover.
    pub fn plain(mut self) -> Self {
        self.plain = true;
        self
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(icon: Icon, label: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            icon,
            label: label.into(),
            plain: false,
        }
    }

    pub fn render(self) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let plain = self.plain;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.label.hash(&mut hasher);
        let button = div()
            .id(("glass-icon-button", hasher.finish()))
            .size(px(30.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.))
            .text_color(rgb(theme.color(0x505661)))
            .cursor_pointer()
            .hover(move |style| {
                let style = style.text_color(rgb(theme.color(0x0874e8)));
                if plain {
                    style
                } else {
                    style.bg(rgb(theme.color(0xf0f4f9)))
                }
            })
            .active(move |style| style.opacity(0.82))
            .child(self.icon.render(16.));
        if plain {
            button
        } else {
            button
                .border_1()
                .border_color(rgb(theme.color(0xd5dae2)))
                .bg(rgb(theme.color(0xfafcff)))
        }
    }
}

impl IntoElement for IconButton {
    type Element = gpui::Stateful<gpui::Div>;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}

/// Embedded 24×24, 1.8px stroke icon assets for consumers' GPUI asset source.
#[derive(Clone, Default)]
pub struct GlintAssets;

impl AssetSource for GlintAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let Some(name) = path
            .strip_prefix("glint-ui/icons/")
            .and_then(|p| p.strip_suffix(".svg"))
        else {
            return Ok(None);
        };
        let svg = icon_svg(name);
        Ok(svg.map(|s| Cow::Owned(s.into_bytes())))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

fn icon_svg(name: &str) -> Option<String> {
    let (body, _fill) = match name {
        "folder" => (
            "<path d='M3 7V5.5A1.5 1.5 0 0 1 4.5 4H9l2.5 3H19.5A1.5 1.5 0 0 1 21 8.5v10A1.5 1.5 0 0 1 19.5 20h-15A1.5 1.5 0 0 1 3 18.5V7Z'/><path d='M3 9h18'/>",
            "none",
        ),
        "folder-open" => (
            "<path d='M3 17V5.5A1.5 1.5 0 0 1 4.5 4H9l2.5 3H19.5A1.5 1.5 0 0 1 21 8.5V10'/><path d='M5.5 10h15a1.5 1.5 0 0 1 1.4 2L19.5 19a1.5 1.5 0 0 1-1.4 1H4.5a1.5 1.5 0 0 1-1.4-2l2.4-7a1.5 1.5 0 0 1 1.4-1Z'/>",
            "none",
        ),
        "sidebar" => (
            "<rect x='3' y='3' width='18' height='18' rx='4'/><path d='M9 3v18'/>",
            "none",
        ),
        "shield" => (
            "<path d='M12 3.2 19.5 6v5.4c0 4.6-3.1 7.9-7.5 9.4-4.4-1.5-7.5-4.8-7.5-9.4V6L12 3.2Z'/><path d='m8.8 12 2.2 2.2 4.2-4.4'/>",
            "none",
        ),
        "grip" => (
            "<circle cx='9' cy='6' r='1.4'/><circle cx='15' cy='6' r='1.4'/><circle cx='9' cy='12' r='1.4'/><circle cx='15' cy='12' r='1.4'/><circle cx='9' cy='18' r='1.4'/><circle cx='15' cy='18' r='1.4'/>",
            "none",
        ),
        "spinner" => ("<path d='M12 3a9 9 0 1 1-9 9'/>", "none"),
        "calendar" => (
            "<rect x='3' y='5' width='18' height='16' rx='2'/><path d='M7 3v4M17 3v4M3 11h18M7 15h2M13 15h2'/>",
            "none",
        ),
        "search" => (
            "<circle cx='11' cy='11' r='6.5'/><path d='m20 20-4-4'/>",
            "none",
        ),
        "settings" => (
            "<circle cx='12' cy='12' r='3.2'/><path d='M12 2.8v2.4M12 18.8v2.4M2.8 12h2.4M18.8 12h2.4M5.4 5.4l1.7 1.7M16.9 16.9l1.7 1.7M18.6 5.4l-1.7 1.7M7.1 16.9l-1.7 1.7'/>",
            "none",
        ),
        "plus" => ("<path d='M12 5v14M5 12h14'/>", "none"),
        "pencil" => (
            "<path d='M17 3.5a2.6 2.6 0 0 1 3.7 3.7L8 20l-4.5 1L4.5 16.5z'/>",
            "none",
        ),
        "trash" => ("<path d='M3 6h18M8 6V4h8v2m-9 0 1 14h8l1-14'/>", "none"),
        "save" => (
            "<path d='M5 3h12l4 4v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z'/><path d='M7 3v6h9V3M7 21v-8h10v8'/>",
            "none",
        ),
        "copy" => (
            "<rect x='9' y='9' width='11' height='11' rx='2'/><path d='M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1'/>",
            "none",
        ),
        "upload" => (
            "<path d='M12 15V3.5m0 0L7.5 8M12 3.5 16.5 8M4 17v2.5A1.5 1.5 0 0 0 5.5 21h13a1.5 1.5 0 0 0 1.5-1.5V17'/>",
            "none",
        ),
        "download" => (
            "<path d='M12 3.5V15m0 0 4.5-4.5M12 15 7.5 10.5M4 17v2.5A1.5 1.5 0 0 0 5.5 21h13a1.5 1.5 0 0 0 1.5-1.5V17'/>",
            "none",
        ),
        "refresh" => (
            "<path d='M20 12a8 8 0 1 1-2.3-5.6L20 8.5M20 3.5v5h-5'/>",
            "none",
        ),
        "shuffle" => (
            "<path d='M3 7h3.5L17 17h4m0 0-2.5-2.5M21 17l-2.5 2.5M3 17h3.5l2.8-2.9M14.6 9.6 17 7h4m0 0-2.5-2.5M21 7l-2.5 2.5'/>",
            "none",
        ),
        "play" => (
            "<path fill='currentColor' stroke='none' d='M7.5 4.8v14.4L19.5 12z'/>",
            "currentColor",
        ),
        "more" => (
            "<circle cx='5' cy='12' r='1.5' fill='currentColor'/><circle cx='12' cy='12' r='1.5' fill='currentColor'/><circle cx='19' cy='12' r='1.5' fill='currentColor'/>",
            "none",
        ),
        "globe" => (
            "<circle cx='12' cy='12' r='8.5'/><path d='M3.5 12h17M12 3.5c2.6 2.3 3.9 5.1 3.9 8.5s-1.3 6.2-3.9 8.5c-2.6-2.3-3.9-5.1-3.9-8.5s1.3-6.2 3.9-8.5z'/>",
            "none",
        ),
        "clock" => (
            "<circle cx='12' cy='12' r='8.5'/><path d='M12 7.5V12l3 2'/>",
            "none",
        ),
        "user" => (
            "<circle cx='12' cy='8' r='3.6'/><path d='M4.8 20a7.2 7.2 0 0 1 14.4 0'/>",
            "none",
        ),
        "close" => ("<path d='M6 6l12 12M18 6 6 18'/>", "none"),
        "arrow-left" => ("<path d='M20 12H4m0 0 6-6m-6 6 6 6'/>", "none"),
        "arrow-up" => ("<path d='M12 20V4m0 0-6 6m6-6 6 6'/>", "none"),
        "chevron-down" => ("<path d='m6 9.5 6 6 6-6'/>", "none"),
        "chevron-right" => ("<path d='m9.5 6 6 6-6 6'/>", "none"),
        "pause" => ("<path d='M8.5 5.5v13M15.5 5.5v13'/>", "none"),
        "video" => (
            "<rect x='3' y='6' width='13' height='12' rx='2.5'/><path d='M9.5 9.5v5l4.3-2.5zM19.5 9.5l2-1v7l-2-1'/>",
            "none",
        ),
        "imagegroup" => (
            "<rect x='4' y='4' width='7' height='7' rx='1.5'/><rect x='13' y='4' width='7' height='7' rx='1.5'/><rect x='4' y='13' width='7' height='7' rx='1.5'/><rect x='13' y='13' width='7' height='7' rx='1.5'/>",
            "none",
        ),
        "text" => ("<path d='M4 6h16M4 10.5h16M4 15h10M4 19.5h13'/>", "none"),
        "link" => (
            "<path d='M9.8 14.2l4.4-4.4M11.5 7.3l1.7-1.7a3.7 3.7 0 0 1 5.2 5.2l-1.7 1.7M12.5 16.7l-1.7 1.7a3.7 3.7 0 0 1-5.2-5.2l1.7-1.7'/>",
            "none",
        ),
        "tag" => (
            "<path d='M3.5 11V5A1.5 1.5 0 0 1 5 3.5h6L20.5 13a1.5 1.5 0 0 1 0 2.1l-5.4 5.4a1.5 1.5 0 0 1-2.1 0z'/><circle cx='8' cy='8' r='1.3'/>",
            "none",
        ),
        "star" => (
            "<path d='m12 4 2.4 5 5.6.7-4.1 3.8 1.1 5.5-5-2.8-5 2.8 1.1-5.5L4 9.7l5.6-.7z'/>",
            "none",
        ),
        "list" => (
            "<path d='M8.5 6H21M8.5 12H21M8.5 18H17M4 5v2M4 11v2M4 17v2'/>",
            "none",
        ),
        "external" => (
            "<path d='M13.5 5H19v5.5M19 5l-8 8M19 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 18.5v-11A1.5 1.5 0 0 1 6.5 6H11'/>",
            "none",
        ),
        "eye" => (
            "<path d='M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z'/><circle cx='12' cy='12' r='2.8'/>",
            "none",
        ),
        "check-circle" => (
            "<circle cx='12' cy='12' r='8.5'/><path d='m8.3 12.2 2.5 2.5 4.9-5.2'/>",
            "none",
        ),
        "half-circle" => (
            "<circle cx='12' cy='12' r='8.5'/><path fill='currentColor' stroke='none' d='M12 3.5a8.5 8.5 0 0 1 0 17z'/>",
            "currentColor",
        ),
        "seal" => (
            "<rect x='4.5' y='4.5' width='15' height='15' rx='3'/><rect x='9' y='9' width='6' height='6' rx='1'/>",
            "none",
        ),
        "database" => (
            "<ellipse cx='12' cy='5.5' rx='8' ry='3'/><path d='M4 5.5v13c0 1.7 3.6 3 8 3s8-1.3 8-3v-13M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3'/>",
            "none",
        ),
        "workflow" => (
            "<circle cx='5.5' cy='5.5' r='2.2'/><circle cx='18.5' cy='5.5' r='2.2'/><circle cx='12' cy='18.5' r='2.2'/><path d='M7.7 5.5h8.6M12 7.7v8.6'/>",
            "none",
        ),
        "pulse" => ("<path d='M2.5 12h4.2L9 6l4 12 2.3-6h6.2'/>", "none"),
        "key" => (
            "<circle cx='8' cy='14.5' r='4'/><path d='M11 11.5 20 3m-4.5 4.5L18 10m-4 0 2.5 2.5'/>",
            "none",
        ),
        "folders" => (
            "<path d='M3 6.5A1.5 1.5 0 0 1 4.5 5h4.6l1.9 2H19a1.5 1.5 0 0 1 1.5 1.5V10M3 8.5A1.5 1.5 0 0 1 4.5 7h6.4l1.9 2.3H19.5a1.5 1.5 0 0 1 1.5 1.5v7.7a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 19.5z'/>",
            "none",
        ),
        "compass" => (
            "<circle cx='12' cy='12' r='8.5'/><path d='m14.8 9.2-1.8 4.6-4.6 1.8 1.8-4.6z'/>",
            "none",
        ),
        "sort" => (
            "<path d='M7 4v14m0 0-3-3m3 3 3-3M17 20V6m0 0-3 3m3-3 3 3'/>",
            "none",
        ),
        "filter" => (
            "<path d='M4 7h8m6 0h2M4 17h2m4 0h10'/><circle cx='15' cy='7' r='2'/><circle cx='8' cy='17' r='2'/>",
            "none",
        ),
        "apidoc" => (
            "<path d='M8.5 4C6.7 4 6.2 5 6.2 6.5v2.2c0 1-.6 1.9-2.2 2.3 1.6.4 2.2 1.3 2.2 2.3v2.2C6.2 19 6.7 20 8.5 20M15.5 4c1.8 0 2.3 1 2.3 2.5v2.2c0 1 .6 1.9 2.2 2.3-1.6.4-2.2 1.3-2.2 2.3v2.2c0 1.5-.5 2.5-2.3 2.5'/>",
            "none",
        ),
        "markdown" => (
            "<rect x='2' y='5' width='20' height='14' rx='2'/><path d='M5 15V9l3 3 3-3v6m6-6v6m-2-2 2 2 2-2'/>",
            "none",
        ),
        "rust" => (
            "<path d='m10 2 4 0 1 3 3-1 2 3-2 2 3 2v3l-3 1 1 3-3 2-2-2-2 3h-3l-1-3-3 1-2-3 2-2-3-2v-3l3-1-1-3 3-2 2 2z'/><path d='M9 16V8h4a2 2 0 0 1 0 4H9m3 0 3 4'/>",
            "none",
        ),
        "go" => (
            "<path d='M3 7H1m2 5H0m3 5H1m11-8a4 4 0 1 0 0 6v-3H9'/><ellipse cx='18' cy='12' rx='3.5' ry='5'/>",
            "none",
        ),
        "javascript" => (
            "<rect x='3' y='3' width='18' height='18' rx='2'/><path d='M10 8v7a2 2 0 0 1-4 0m12-6c-3-2-5 1-2 3s1 5-2 3'/>",
            "none",
        ),
        "typescript" => (
            "<rect x='3' y='3' width='18' height='18' rx='2'/><path d='M5 8h7m-3.5 0v9M18 9c-3-2-5 1-2 3s1 5-2 3'/>",
            "none",
        ),
        "python" => (
            "<path d='M12 12H7a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3h7a3 3 0 0 1 3 3v5H9a2 2 0 0 0-2 2v3a3 3 0 0 0 3 3h7a3 3 0 0 0 3-3v-2a3 3 0 0 0-3-3M7 7H4a2 2 0 0 0-2 2v5a2 2 0 0 0 2 2h3'/><path d='M10 7h.01M14 17h.01'/>",
            "none",
        ),
        "java" => (
            "<path d='M5 11h12v5a4 4 0 0 1-4 4H9a4 4 0 0 1-4-4zm12 1h2a2 2 0 0 1 0 4h-2M3 22h17M9 8c-4-3 4-3 0-6m5 6c-4-3 4-3 0-6'/>",
            "none",
        ),
        "c" => (
            "<path d='m12 2 9 5v10l-9 5-9-5V7zM16 8a5 5 0 1 0 0 8'/>",
            "none",
        ),
        "cpp" => (
            "<path d='m12 2 9 5v10l-9 5-9-5V7zM11 8a4 4 0 1 0 0 8m3-6v4m-2-2h4m3-2v4m-2-2h4'/>",
            "none",
        ),
        "html" => (
            "<path d='M4 3h16l-2 17-6 2-6-2zm4 5h8l-.5 4h-7l.5 4 3 1 3-1 .5-4'/>",
            "none",
        ),
        "css" => (
            "<path d='M4 3h16l-2 17-6 2-6-2zm4 5h8l-.5 4h-6m6 0-.5 4-3 1-3-1'/>",
            "none",
        ),
        "json" => (
            "<path d='M8 3H6a2 2 0 0 0-2 2v4a3 3 0 0 1-2 3 3 3 0 0 1 2 3v4a2 2 0 0 0 2 2h2m8-18h2a2 2 0 0 1 2 2v4a3 3 0 0 0 2 3 3 3 0 0 0-2 3v4a2 2 0 0 1-2 2h-2'/><circle cx='12' cy='9' r='.8'/><circle cx='12' cy='15' r='.8'/>",
            "none",
        ),
        "shell" => (
            "<rect x='2' y='4' width='20' height='16' rx='2'/><path d='m6 9 4 3-4 3m7 1h5'/>",
            "none",
        ),
        "sql" => (
            "<ellipse cx='12' cy='5' rx='8' ry='3'/><path d='M4 5v14c0 4 16 4 16 0V5M4 11c0 4 16 4 16 0M4 17c0 4 16 4 16 0'/>",
            "none",
        ),
        "yaml" => (
            "<path d='M5 3h10l5 5v13H5zm10 0v5h5M8 12l3 3 3-3m-3 3v4m5-7h1m-1 5h1'/>",
            "none",
        ),
        "code" => ("<path d='m8 8-4 4 4 4m8-8 4 4-4 4m-3-13-2 18'/>", "none"),
        "script" => (
            "<rect x='3' y='4.5' width='18' height='15' rx='2'/><path d='m7.5 9.5 2.8 2.5-2.8 2.5M12.5 15h4'/>",
            "none",
        ),
        _ => return None,
    };
    Some(format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><g fill='none' stroke='currentColor' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'>{body}</g></svg>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_icon_loads_and_renders() {
        let svg = GlintAssets
            .load(&format!("glint-ui/icons/{}.svg", Icon::Save.name()))
            .unwrap()
            .expect("embedded save icon");
        let tree = resvg::usvg::Tree::from_data(&svg, &Default::default()).unwrap();
        let mut pixels = resvg::tiny_skia::Pixmap::new(24, 24).unwrap();
        resvg::render(&tree, Default::default(), &mut pixels.as_mut());
        assert!(pixels.pixels().iter().any(|pixel| pixel.alpha() > 0));
    }

    #[test]
    fn open_and_closed_folder_icons_have_distinct_renderable_silhouettes() {
        let render = |icon: Icon| {
            let asset = GlintAssets
                .load(&format!("glint-ui/icons/{}.svg", icon.name()))
                .unwrap()
                .unwrap();
            let tree = resvg::usvg::Tree::from_data(&asset, &Default::default()).unwrap();
            let mut pixmap = resvg::tiny_skia::Pixmap::new(24, 24).unwrap();
            resvg::render(&tree, Default::default(), &mut pixmap.as_mut());
            assert!(pixmap.pixels().iter().any(|pixel| pixel.alpha() > 0));
            pixmap.data().to_vec()
        };
        assert_ne!(render(Icon::Folder), render(Icon::FolderOpen));
    }

    #[test]
    fn language_aliases_load_renderable_embedded_icons() {
        let assets = GlintAssets;
        for (language, name) in [
            (" MD ", "markdown"),
            ("rs", "rust"),
            ("Golang", "go"),
            ("jsx", "javascript"),
            ("tsx", "typescript"),
            ("py", "python"),
            ("java", "java"),
            ("h", "c"),
            ("c++", "cpp"),
            ("htm", "html"),
            ("scss", "css"),
            ("jsonc", "json"),
            ("bash", "shell"),
            ("postgresql", "sql"),
            ("yml", "yaml"),
            ("", "code"),
            ("unknown-language", "code"),
        ] {
            let icon = Icon::for_language(language);
            assert_eq!(icon.name(), name, "{language}");
            let svg = assets
                .load(&format!("glint-ui/icons/{name}.svg"))
                .unwrap()
                .expect("embedded language icon");
            let tree = resvg::usvg::Tree::from_data(&svg, &Default::default()).unwrap();
            let mut pixels = resvg::tiny_skia::Pixmap::new(24, 24).unwrap();
            resvg::render(&tree, Default::default(), &mut pixels.as_mut());
            assert!(
                pixels.pixels().iter().any(|pixel| pixel.alpha() > 0),
                "empty language icon: {name}"
            );
        }
    }

    #[test]
    fn icon_color_follows_parent_and_preserves_explicit_overrides() {
        let mut icon = Icon::Search.render(20.);
        for theme in ThemePreset::ALL {
            let color: gpui::Hsla = gpui::rgb_to_hsla(rgb(theme.palette().text));
            let previous = icon.inherit_color(color);
            assert_eq!(icon.style().text.color, Some(color));
            icon.style().text = previous;
        }
        // A parent hover color must be resolved anew on the next paint.
        let hover: gpui::Hsla = gpui::rgb_to_hsla(rgb(0x0a84ff));
        let previous = icon.inherit_color(hover);
        assert_eq!(icon.style().text.color, Some(hover));
        icon.style().text = previous;

        let explicit: gpui::Hsla = gpui::rgb_to_hsla(rgb(0xff9500));
        let mut icon = Icon::Star.render(20.).text_color(explicit);
        icon.inherit_color(hover);
        assert_eq!(icon.style().text.color, Some(explicit));
    }
}
