use gpui::{
    Context, Entity, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Window, div, px, rgb,
};
use gpui_glint::{
    components::{Button, ButtonKind, Icon, VideoPlayer, VideoPlayerEvent, VideoSource},
    theme::ThemePreset,
};

pub struct VideoPage {
    player: Option<Entity<VideoPlayer>>,
    theme: ThemePreset,
    title: String,
    error: Option<String>,
}
impl VideoPage {
    pub fn new(window: &mut Window, theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let (player, error) = match VideoPlayer::new_entity(window, cx) {
            Ok(player) => (Some(player), None),
            Err(error) => (None, Some(error)),
        };
        if let Some(player) = &player {
            let _ = player.update(cx, |player, cx| player.set_theme(theme, cx));
            cx.subscribe(player, |this, _, event: &VideoPlayerEvent, cx| {
                if std::env::var_os("GLINT_VIDEO_TRACE").is_some() {
                    println!("{event:?}");
                }
                if let VideoPlayerEvent::Error(error) = event {
                    this.error = Some(error.clone());
                }
                if matches!(event, VideoPlayerEvent::Loaded { .. }) {
                    this.error = None;
                }
                if !matches!(event, VideoPlayerEvent::Progress { .. }) {
                    cx.notify();
                }
            })
            .detach();
        }
        Self {
            player,
            theme,
            title: "视频播放器".into(),
            error,
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        if let Some(player) = &self.player {
            let _ = player.update(cx, |p, cx| p.set_theme(theme, cx));
        }
        cx.notify();
    }
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if let Some(player) = &self.player {
            if let Err(error) = player.update(cx, |p, cx| p.set_visible(visible, cx)) {
                self.error = Some(error);
            }
        }
    }
    pub fn load(&mut self, source: VideoSource, cx: &mut Context<Self>) {
        let title = match &source {
            VideoSource::File(path) => path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "本地视频".into()),
            VideoSource::Url(_) => "网络视频".into(),
        };
        if let Some(player) = &self.player {
            match player.update(cx, |p, cx| {
                p.set_source(source, cx)?;
                p.set_title(title.clone(), cx)
            }) {
                Ok(()) => {
                    self.title = title;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        cx.notify();
    }
    fn open(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("打开视频".into()),
        });
        cx.spawn(async move |weak, cx| match picker.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = weak.update(cx, |this, cx| this.load(VideoSource::file(path), cx));
                }
            }
            Ok(Ok(None)) => {}
            _ => {
                let _ = weak.update(cx, |this, cx| {
                    this.error = Some("无法选择视频文件".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }
}
impl Render for VideoPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let mut body = div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .p_5()
            .gap_4()
            .bg(rgb(p.canvas))
            .text_color(rgb(p.text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(self.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child("本地与网络视频 · 空格播放 / 暂停 · 方向键快进 / 后退"),
                            ),
                    )
                    .child(
                        Button::new("打开视频…")
                            .icon(Icon::FolderOpen)
                            .kind(ButtonKind::Tinted)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.open(cx))),
                    ),
            );
        if let Some(player) = &self.player {
            body = body.child(div().flex_1().min_h_0().child(player.clone()));
        }
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(self.theme.color(0xd9342b)))
                    .child(error.clone()),
            );
        }
        body
    }
}
