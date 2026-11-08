use super::*;
use crate::code_fonts::{self, Library};

pub struct State {
    library: Library,
    input: Entity<TextEntry>,
    task: Option<Task<()>>,
    notice: Option<String>,
    readable: bool,
    registered: std::collections::HashSet<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn real_font_import_selection_and_restart_restore_use_managed_assets(
        cx: &mut gpui::TestAppContext,
    ) {
        let Some(source) = std::env::var_os("GLINT_TEST_FONT") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let source_copy = dir.path().join("source.woff2");
        std::fs::copy(PathBuf::from(source), &source_copy).unwrap();
        let anchor = dir.path().join("sessions.json");
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(anchor.clone(), cx);
            host.page = crate::Page::Settings;
            host.provider_settings.section = provider_ui::Section::Appearance;
            host
        });
        window.simulate_resize(size(px(1400.), px(1000.)));
        host.update(window, |host, cx| {
            host.change_code_font(Some(source_copy.clone()), None, cx)
        });
        window.run_until_parked();
        let (id, family) = host.read_with(window, |host, _| {
            assert!(
                host.code_fonts.task.is_none(),
                "{:?}",
                host.code_fonts.notice
            );
            assert_eq!(
                host.code_fonts.library.fonts.len(),
                1,
                "{:?}",
                host.code_fonts.notice
            );
            let entry = &host.code_fonts.library.fonts[0];
            assert_eq!(gpui_glint::code_font::family().as_ref(), entry.family);
            (entry.id.clone(), entry.family.clone())
        });
        std::fs::remove_file(source_copy).unwrap();
        host.update(window, |host, cx| host.change_code_font(None, None, cx));
        window.run_until_parked();
        assert_eq!(gpui_glint::code_font::family().as_ref(), "Menlo");
        host.update(window, |host, cx| {
            host.change_code_font(None, Some(id.clone()), cx)
        });
        window.run_until_parked();
        assert_eq!(code_fonts::load(&anchor).unwrap().selected, Some(id));
        gpui_glint::code_font::set_family(None);
        let (restored, window) =
            window.add_window_view(|_, cx| AgentWorkspace::with_path(anchor, cx));
        window.run_until_parked();
        restored.read_with(window, |restored, _| {
            assert!(
                restored
                    .code_fonts
                    .notice
                    .as_deref()
                    .unwrap()
                    .contains("已应用")
            )
        });
        assert_eq!(gpui_glint::code_font::family().as_ref(), family);
        gpui_glint::code_font::set_family(None);
    }
}

impl State {
    pub fn new(anchor: &std::path::Path, cx: &mut Context<AgentWorkspace>) -> Self {
        let result = code_fonts::load(anchor);
        let readable = result.is_ok();
        let notice = result.as_ref().err().cloned();
        Self {
            library: result.unwrap_or_default(),
            input: cx.new(|cx| TextEntry::new(cx, "/path/to/font.ttf", "")),
            task: None,
            notice,
            readable,
            registered: Default::default(),
        }
    }
}

impl AgentWorkspace {
    pub(super) fn restore_code_font(&mut self, cx: &mut Context<Self>) {
        let selected = self.code_fonts.library.selected.clone();
        if selected.is_some() {
            self.change_code_font(None, selected, cx);
        }
    }

    fn pick_code_font(&mut self, cx: &mut Context<Self>) {
        if self.code_fonts.task.is_some() {
            return;
        }
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择代码字体（TTF / OTF / WOFF2）".into()),
        });
        self.code_fonts.task = Some(cx.spawn(async move |weak, cx| {
            let result = picker.await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.code_fonts.task.take() {
                    task.detach();
                }
                match result {
                    Ok(Ok(Some(paths))) if !paths.is_empty() => {
                        this.change_code_font(Some(paths[0].clone()), None, cx)
                    }
                    Ok(Ok(_)) => {}
                    _ => this.code_fonts.notice = Some("无法打开字体选择器".into()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn change_code_font(
        &mut self,
        source: Option<PathBuf>,
        selected: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.code_fonts.task.is_some() || !self.code_fonts.readable {
            return;
        }
        let anchor = self.path.clone();
        let library = self.code_fonts.library.clone();
        let work = cx.background_executor().spawn(async move {
            let (mut library, entry, bytes) = if let Some(source) = source {
                let (library, entry, bytes) = code_fonts::import(&anchor, &source, library)?;
                (library, Some(entry), bytes)
            } else if let Some(id) = selected {
                let entry = library
                    .fonts
                    .iter()
                    .find(|entry| entry.id == id)
                    .cloned()
                    .ok_or("历史字体不存在")?;
                let bytes = code_fonts::read_font(&anchor, &entry)?;
                (library, Some(entry), bytes)
            } else {
                (library, None, Vec::new())
            };
            library.selected = entry.as_ref().map(|entry| entry.id.clone());
            Ok::<_, String>((library, entry.map(|entry| entry.family), bytes))
        });
        self.code_fonts.notice = Some("正在读取并验证字体…".into());
        self.code_fonts.task = Some(cx.spawn(async move |weak, cx| {
            let result = async {
                let (library, family, bytes) = work.await?;
                if !bytes.is_empty() {
                    let id = library.selected.clone().unwrap();
                    weak.update(cx, |this, cx| {
                        if this.code_fonts.registered.contains(&id) {
                            return Ok(());
                        }
                        cx.text_system()
                            .add_fonts(vec![std::borrow::Cow::Owned(bytes)])
                            .map_err(|e| format!("字体引擎无法加载：{e}"))?;
                        this.code_fonts.registered.insert(id);
                        Ok::<_, String>(())
                    })
                    .map_err(|e| e.to_string())??;
                }
                let anchor = weak
                    .update(cx, |this, _| this.path.clone())
                    .map_err(|e| e.to_string())?;
                let persisted = library.clone();
                cx.background_executor()
                    .spawn(async move { code_fonts::save(&anchor, &persisted) })
                    .await?;
                Ok::<_, String>((library, family))
            }
            .await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.code_fonts.task.take() {
                    task.detach();
                }
                match result {
                    Ok((library, family)) => {
                        this.code_fonts.library = library;
                        gpui_glint::code_font::set_family(family.as_deref());
                        this.code_fonts.notice =
                            Some("代码字体已应用，字体文件保存在软件数据目录中。".into());
                        cx.refresh_windows();
                    }
                    Err(error) => this.code_fonts.notice = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn code_font_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let busy = self.code_fonts.task.is_some() || !self.code_fonts.readable;
        let current_family = gpui_glint::code_font::family();
        self.code_fonts
            .input
            .update(cx, |input, cx| input.set_theme(self.theme, cx));

        let mut fonts = div().flex().flex_col().gap_2();
        for (id, family, label) in
            std::iter::once((None, "Menlo".to_owned(), "系统默认".to_owned())).chain(
                self.code_fonts.library.fonts.iter().map(|entry| {
                    (
                        Some(entry.id.clone()),
                        entry.family.clone(),
                        entry.source_name.clone(),
                    )
                }),
            )
        {
            let selected = self.code_fonts.library.selected == id;
            let selector = format!("code-font:{}", id.as_deref().unwrap_or("default"));
            fonts = fonts.child(
                Button::new("")
                    .kind(if selected {
                        ButtonKind::Tinted
                    } else {
                        ButtonKind::Plain
                    })
                    .theme(self.theme)
                    .when(busy, |b| b.disabled())
                    .render()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .w_full()
                    .h_auto()
                    .py_3()
                    .justify_start()
                    .border_color(rgb(if selected { p.accent } else { p.border }))
                    .child(
                        if selected {
                            Icon::CheckCircle
                        } else {
                            Icon::HalfCircle
                        }
                        .render(16.),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(family),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(rgb(p.text_muted))
                                    .truncate()
                                    .child(label),
                            ),
                    )
                    .when(selected, |row| {
                        row.child(div().text_size(px(11.)).child("使用中"))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !busy && !selected {
                            this.change_code_font(None, id.clone(), cx)
                        }
                    })),
            );
        }

        let preview = div()
            .min_w_0()
            .flex_1()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(p.border))
            .overflow_hidden()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(Icon::Code.render(13.))
                    .child(div().flex_1().child("字体预览"))
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .child(current_family.clone()),
                    ),
            )
            .child(
                div()
                    .p_4()
                    .bg(rgb(p.surface_subtle))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .font_family(current_family)
                    .text_size(px(13.))
                    .line_height(px(22.))
                    .children(
                        [
                            ("1", "fn main() {"),
                            ("2", "    println!(\"Hello, Glint!\");"),
                            ("3", "}"),
                        ]
                        .into_iter()
                        .map(|(number, code)| {
                            div()
                                .flex()
                                .gap_3()
                                .child(
                                    div()
                                        .w(px(14.))
                                        .flex_shrink_0()
                                        .text_color(rgb(p.text_muted))
                                        .child(number),
                                )
                                .child(div().min_w_0().child(code))
                        }),
                    )
                    .child(
                        div()
                            .mt_3()
                            .pt_3()
                            .border_t_1()
                            .border_color(rgb(p.border))
                            .text_size(px(12.))
                            .text_color(rgb(p.text_muted))
                            .child("0O  1lI  {} []  => !=  0123456789"),
                    ),
            );

        let mut panel = div()
            .w_full()
            .min_w_0()
            .mt_5()
            .p_5()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(200.))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("代码字体"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text_muted))
                                    .child("用于 Diff、代码卡片、文件预览和代码片段。"),
                            ),
                    )
                    .child(
                        Button::new(if self.code_fonts.task.is_some() {
                            "正在处理…"
                        } else {
                            "选择并导入…"
                        })
                        .icon(Icon::Plus)
                        .theme(self.theme)
                        .when(busy, |b| b.disabled())
                        .render()
                        .debug_selector(|| "pick-code-font".into())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !busy {
                                this.pick_code_font(cx);
                            }
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(220.))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child(format!(
                                        "可用字体 · {}",
                                        self.code_fonts.library.fonts.len() + 1
                                    )),
                            )
                            .child(fonts),
                    )
                    .child(div().flex_1().min_w(px(280.)).child(preview)),
            )
            .child(
                div()
                    .pt_4()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child("也可以粘贴文件路径 · 支持 TTF、OTF、WOFF2"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(32.))
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .rounded(px(7.))
                                    .border_1()
                                    .border_color(rgb(p.border))
                                    .bg(rgb(p.surface))
                                    .child(self.code_fonts.input.clone()),
                            )
                            .child(
                                Button::new("导入并使用")
                                    .kind(ButtonKind::Glass)
                                    .theme(self.theme)
                                    .when(busy, |b| b.disabled())
                                    .render()
                                    .debug_selector(|| "import-code-font".into())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if busy {
                                            return;
                                        }
                                        let path = this
                                            .code_fonts
                                            .input
                                            .read(cx)
                                            .value()
                                            .trim()
                                            .trim_matches('"')
                                            .to_owned();
                                        if path.is_empty() {
                                            this.code_fonts.notice = Some(
                                                "请粘贴字体文件路径，或点击“选择并导入”".into(),
                                            );
                                            cx.notify();
                                            return;
                                        }
                                        this.change_code_font(Some(PathBuf::from(path)), None, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("导入的字体会保存在本机，可随时切换。"),
            );
        if let Some(notice) = &self.code_fonts.notice {
            panel = panel.child(div().text_size(px(12.)).child(notice.clone()));
        }
        panel
    }
}
