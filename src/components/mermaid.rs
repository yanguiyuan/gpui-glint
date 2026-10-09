use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

use super::{CodeCard, Icon, IconButton, SegmentOption, SegmentedControl};
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{
    AppContext, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, MouseButton, ObjectFit, ParentElement, Render, RenderImage,
    SharedString, StatefulInteractiveElement, Styled, StyledImage, Task, Window, deferred, div,
    img, px, rgb,
};
use image::{Frame, RgbaImage};
use mermaid_rs_renderer::{
    RenderOptions, compute_layout, measure_svg_dimensions, parse_mermaid_strict, render_svg,
};

/// Offline SVG output suitable for export. Syntax coverage follows mermaid-rs-renderer.
#[derive(Clone, Debug)]
pub struct MermaidSvg {
    pub svg: String,
    pub width: f32,
    pub height: f32,
}

/// Parse, lay out, and render a Mermaid source using the Glass palette.
pub fn render_mermaid(source: &str, theme: ThemePreset) -> Result<MermaidSvg, String> {
    if source.len() > 256 * 1024 || source.lines().count() > 2000 {
        return Err("图表过大，请拆分为多个 Mermaid 块".into());
    }
    let parsed = parse_mermaid_strict(source).map_err(|e| e.to_string())?;
    if parsed.graph.nodes.len() > 512 || parsed.graph.edges.len() > 1024 {
        return Err("图表节点或连线过多，请拆分为多个 Mermaid 块".into());
    }
    let p = theme.palette();
    let color = |value| format!("#{value:06x}");
    let mut options = RenderOptions::default();
    options.theme.font_family = "Arial, PingFang SC, Noto Sans CJK SC, sans-serif".into();
    options.theme.background = color(p.surface);
    options.theme.primary_color = color(p.accent_tint);
    options.theme.primary_text_color = color(p.text);
    options.theme.primary_border_color = color(p.accent);
    options.theme.text_color = color(p.text);
    options.theme.line_color = color(p.text_muted);
    options.theme.secondary_color = color(p.surface_subtle);
    options.theme.tertiary_color = color(p.panel);
    options.theme.edge_label_background = color(p.surface);
    options.theme.cluster_background = color(p.panel);
    options.theme.cluster_border = color(p.border);
    options.theme.sequence_actor_fill = color(p.accent_tint);
    options.theme.sequence_actor_border = color(p.accent);
    let layout = compute_layout(&parsed.graph, &options.theme, &options.layout);
    let size = measure_svg_dimensions(&layout, &options.layout, None);
    if !size.width.is_finite() || !size.height.is_finite() || size.width <= 0. || size.height <= 0.
    {
        return Err("图表尺寸无效".into());
    }
    let svg = render_svg(&layout, &options.theme, &options.layout);
    // Embedded external images are outside the offline component's contract.
    if svg.contains("<image") {
        return Err("离线图表暂不支持 image 节点，请使用文本节点".into());
    }
    Ok(MermaidSvg {
        svg,
        width: size.width,
        height: size.height,
    })
}

struct DiagramImage {
    svg: MermaidSvg,
    image: Arc<RenderImage>,
    png: Arc<[u8]>,
}
fn rasterize(svg: MermaidSvg) -> Result<DiagramImage, String> {
    let scale = 2_f32.min(4096. / svg.width.max(svg.height));
    static FONTS: LazyLock<Arc<resvg::usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut fonts = resvg::usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        Arc::new(fonts)
    });
    let options = resvg::usvg::Options {
        fontdb: FONTS.clone(),
        ..Default::default()
    };
    let tree =
        resvg::usvg::Tree::from_data(svg.svg.as_bytes(), &options).map_err(|e| e.to_string())?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(
        (tree.size().width() * scale).ceil().max(1.) as u32,
        (tree.size().height() * scale).ceil().max(1.) as u32,
    )
    .ok_or("图表图像尺寸超出限制")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let (width, height) = (pixmap.width(), pixmap.height());
    let png = pixmap
        .encode_png()
        .map_err(|error| error.to_string())?
        .into();
    let mut pixels = pixmap.take();
    // tiny-skia uses premultiplied RGBA; GPUI's RenderImage expects straight BGRA.
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        if alpha > 0 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u32 * 255 / alpha).min(255)) as u8;
            }
        }
        pixel.swap(0, 2);
    }
    let buffer = RgbaImage::from_raw(width, height, pixels).ok_or("无法创建图表图像")?;
    Ok(DiagramImage {
        svg,
        png,
        image: Arc::new(RenderImage::new(vec![Frame::new(buffer)])),
    })
}

/// Host notifications; the diagram does not display action feedback internally.
#[derive(Clone, Debug)]
pub enum MermaidEvent {
    SourceCopied(SharedString),
    PngExported(std::path::PathBuf),
    ExportFailed(SharedString),
}

/// A reusable native Mermaid view. Source changes are debounced and rendered off the UI thread.
/// It retains only the current image; stale jobs cannot replace a newer source or theme.
pub struct MermaidDiagram {
    source: SharedString,
    theme: ThemePreset,
    result: Option<Result<DiagramImage, String>>,
    task: Option<Task<()>>,
    revision: u64,
    max_height: f32,
    source_view: bool,
    source_editor: Option<Entity<CodeCard>>,
    external_editor: bool,
    focus: Option<FocusHandle>,
    hovered: bool,
    menu_open: bool,
    exporting: bool,
    export_task: Option<Task<()>>,
}
impl EventEmitter<MermaidEvent> for MermaidDiagram {}
impl MermaidDiagram {
    pub fn new(source: impl Into<SharedString>) -> Self {
        Self {
            source: source.into(),
            theme: ThemePreset::default(),
            result: None,
            task: None,
            revision: 0,
            max_height: 480.,
            source_view: false,
            source_editor: None,
            external_editor: false,
            focus: None,
            hovered: false,
            menu_open: false,
            exporting: false,
            export_task: None,
        }
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = height.max(32.);
        self
    }
    /// Supply an editor owned by the host to keep its edit events, history, and read-only policy.
    /// Without one, the source view is read-only.
    pub fn source_editor(mut self, editor: Entity<CodeCard>) -> Self {
        self.source_editor = Some(editor);
        self.external_editor = true;
        self
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn svg(&self) -> Option<&str> {
        self.result
            .as_ref()?
            .as_ref()
            .ok()
            .map(|r| r.svg.svg.as_str())
    }
    /// PNG of the current rendered diagram, without the component controls.
    pub fn png(&self) -> Option<&[u8]> {
        self.result.as_ref()?.as_ref().ok().map(|r| r.png.as_ref())
    }
    pub fn error(&self) -> Option<&str> {
        self.result.as_ref()?.as_ref().err().map(String::as_str)
    }
    pub fn set_source(&mut self, source: impl Into<SharedString>, cx: &mut Context<Self>) {
        let source = source.into();
        if self.source != source {
            self.source = source;
            if !self.external_editor
                && let Some(editor) = &self.source_editor
            {
                editor.update(cx, |editor, cx| editor.set_content(self.source.clone(), cx));
            }
            self.invalidate(cx);
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            if let Some(editor) = &self.source_editor {
                editor.update(cx, |editor, cx| editor.set_theme(theme, cx));
            }
            self.invalidate(cx);
        }
    }
    fn invalidate(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        self.task = None;
        self.result = None;
        cx.notify();
    }
    fn export_png(&mut self, cx: &mut Context<Self>) {
        let Some(Ok(result)) = &self.result else {
            return;
        };
        if self.exporting {
            return;
        }
        let png = result.png.clone();
        let directory = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let picker = cx.prompt_for_new_path(&directory, Some("mermaid.png"));
        self.exporting = true;
        self.export_task = Some(cx.spawn(async move |this, cx| {
            let outcome = match picker.await {
                Ok(Ok(Some(path))) => Some(
                    cx.background_executor()
                        .spawn(async move {
                            std::fs::write(&path, &png)
                                .map(|_| MermaidEvent::PngExported(path))
                                .unwrap_or_else(|error| {
                                    MermaidEvent::ExportFailed(error.to_string().into())
                                })
                        })
                        .await,
                ),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some(MermaidEvent::ExportFailed(error.to_string().into())),
                Err(error) => Some(MermaidEvent::ExportFailed(
                    format!("文件选择器异常：{error}").into(),
                )),
            };
            let _ = this.update(cx, |this, cx| {
                this.exporting = false;
                if let Some(event) = outcome {
                    cx.emit(event);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn start(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() || self.result.is_some() {
            return;
        }
        let source = self.source.clone();
        let theme = self.theme;
        let revision = self.revision;
        let timer = cx.background_executor().timer(Duration::from_millis(150));
        let work = cx.background_executor().spawn(async move {
            timer.await;
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                render_mermaid(&source, theme).and_then(|svg| rasterize(svg))
            }))
            .unwrap_or_else(|_| Err("图表渲染失败，请检查 Mermaid 语法".into()))
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.revision == revision {
                    this.result = Some(result);
                    cx.notify();
                }
            });
        }));
    }
}
impl Render for MermaidDiagram {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        self.start(cx);
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        let editor = self
            .source_editor
            .get_or_insert_with(|| {
                cx.new(|cx| {
                    CodeCard::new(cx, "mermaid", self.source.clone())
                        .read_only(true)
                        .theme(self.theme)
                })
            })
            .clone();
        let entity = cx.entity();
        let switch =
            SegmentedControl::new([SegmentOption::new("渲染"), SegmentOption::new("源码")])
                .selected(usize::from(self.source_view))
                .tinted()
                .on_change(move |index, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.source_view = index == 1;
                        if this.source_view {
                            window
                                .focus(&this.source_editor.as_ref().unwrap().focus_handle(cx), cx);
                        } else {
                            window.focus(this.focus.as_ref().unwrap(), cx);
                        }
                        cx.notify();
                    });
                })
                .render();
        let p = self.theme.palette();
        // A column keeps the image's percentage width from changing its measured
        // block height after layout, which would overlap following document blocks.
        let mut frame = div()
            .id("glass-mermaid")
            .debug_selector(|| "mermaid-frame".into())
            .track_focus(&focus)
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .p_3()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .on_hover(cx.listener(|this, hovered, _, cx| {
                if this.hovered != *hovered {
                    this.hovered = *hovered;
                    cx.notify();
                }
            }));
        let mut more = div().relative().child(
            IconButton::new(Icon::More, "Mermaid 更多")
                .render()
                .opacity(if self.hovered { 1. } else { 0. })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.menu_open = !this.menu_open;
                    cx.notify();
                })),
        );
        if self.menu_open {
            let available = self.png().is_some() && !self.exporting;
            let mut menu = div()
                .p_1()
                .rounded(px(8.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.surface))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
            for (index, label, icon, enabled) in [
                (0, "复制源码", Icon::Copy, true),
                (
                    1,
                    if self.exporting {
                        "正在导出…"
                    } else {
                        "导出为 PNG…"
                    },
                    Icon::Download,
                    available,
                ),
            ] {
                menu = menu.child(
                    div()
                        .id(("mermaid-action", index as usize))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_2()
                        .rounded(px(5.))
                        .text_size(px(12.))
                        .text_color(rgb(if enabled { p.text } else { p.text_muted }))
                        .when(enabled, |el| {
                            el.cursor_pointer().hover(move |s| s.bg(rgb(p.accent_tint)))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if !enabled {
                                return;
                            }
                            this.menu_open = false;
                            if index == 0 {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    this.source.to_string(),
                                ));
                                cx.emit(MermaidEvent::SourceCopied(this.source.clone()));
                            } else {
                                this.export_png(cx);
                            }
                            cx.notify();
                        }))
                        .child(icon.render(14.))
                        .child(label),
                );
            }
            // Outside-click bounds include the trigger so a second click closes the menu.
            more = more.child(
                deferred(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .w(px(180.))
                        .pt(px(36.))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.menu_open = false;
                            cx.notify();
                        }))
                        .child(menu),
                )
                .with_priority(10),
            );
        }
        frame = frame.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(30.))
                .flex_shrink_0()
                .mb_3()
                .child(div().when(self.hovered, |el| el.child(switch)))
                .child(more),
        );
        if self.source_view {
            return frame.child(editor);
        }
        match &self.result {
            Some(Ok(result)) => {
                frame = frame.child(
                    img(result.image.clone())
                        .debug_selector(|| "mermaid-image".into())
                        .w_full()
                        .h(px(result.svg.height.min(self.max_height)))
                        .flex_shrink_0()
                        .object_fit(ObjectFit::Contain),
                );
            }
            Some(Err(error)) => {
                frame = frame
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(theme.color(0xd95245)))
                            .child("Mermaid 无法渲染"),
                    )
                    .child(
                        div()
                            .mt_2()
                            .text_size(px(12.))
                            .text_color(rgb(p.text_muted))
                            .child(error.clone()),
                    );
            }
            None => {
                frame = frame.child(
                    div()
                        .py_4()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child("正在渲染图表…"),
                );
            }
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders_flowchart_sequence_and_unicode() {
        for source in [
            "flowchart LR\n A[开始] --> B{是否完成}\n B -->|是| C[结束]",
            "sequenceDiagram\n participant A as 客户端\n participant B as 服务端\n A->>B: 请求\n B-->>A: 响应",
            "classDiagram\n Animal <|-- Duck",
            "stateDiagram-v2\n [*] --> Ready\n Ready --> [*]",
        ] {
            let rendered = render_mermaid(source, ThemePreset::Sage).unwrap();
            assert!(rendered.svg.contains("<svg"));
            assert!(rendered.width > 0. && rendered.height > 0.);
        }
    }
    #[test]
    fn dark_presets_render_dark_surfaces_and_light_labels() {
        for theme in ThemePreset::ALL.into_iter().filter(|theme| theme.is_dark()) {
            let rendered = render_mermaid("flowchart LR\n A[Start] --> B[Done]", theme).unwrap();
            let p = theme.palette();
            assert!(rendered.svg.contains(&format!("#{:06x}", p.surface)));
            assert!(rendered.svg.contains(&format!("#{:06x}", p.text)));
            assert!(rendered.svg.contains(&format!("#{:06x}", p.accent_tint)));
            assert!(rendered.width > 0. && rendered.height > 0.);
        }
    }

    #[test]
    fn invalid_input_reports_errors_and_limits() {
        assert!(render_mermaid("not_a_diagram", ThemePreset::Sage).is_err());
        assert!(render_mermaid(&"A".repeat(256 * 1024 + 1), ThemePreset::Sage).is_err());
    }
    #[test]
    fn png_export_preserves_dimensions_and_rgba_colors() {
        let rendered = rasterize(MermaidSvg {
            svg: "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='1'><rect width='1' height='1' fill='#ff0000'/><rect x='1' width='1' height='1' fill='#0000ff'/></svg>".into(),
            width: 2., height: 1.,
        }).unwrap();
        assert!(rendered.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let png = resvg::tiny_skia::Pixmap::decode_png(&rendered.png).unwrap();
        assert_eq!((png.width(), png.height()), (4, 2));
        assert_eq!(&png.data()[..4], &[255, 0, 0, 255]);
        assert_eq!(&png.data()[12..16], &[0, 0, 255, 255]);
        assert_eq!(&rendered.image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
    }
    #[test]
    fn native_raster_keeps_text_and_color_channels() {
        let svg = render_mermaid("flowchart LR\n A[开始] --> B[结束]", ThemePreset::Sage).unwrap();
        let image = rasterize(svg).unwrap();
        assert!(image.image.size(0).width.0 > 0);
        assert!(
            image
                .image
                .as_bytes(0)
                .unwrap()
                .chunks_exact(4)
                .any(|p| p[3] > 0 && p[0] != p[2])
        );
    }
}
