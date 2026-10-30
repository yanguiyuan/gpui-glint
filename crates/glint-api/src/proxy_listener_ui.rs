use super::*;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Copy)]
enum CertificateAction {
    Generate,
    Import,
    Export,
    Install,
}
pub(super) fn certificate_status(store: &storage::Store) -> String {
    let directory = store
        .connection
        .path()
        .map(PathBuf::from)
        .and_then(|path| path.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir)
        .join("proxy-service/ca");
    proxy_service::ca::Authority::load(&directory)
        .map(|ca| {
            format!(
                "{} · 有效期至 {}\nSHA-256 {}",
                ca.name, ca.expires, ca.fingerprint
            )
        })
        .unwrap_or_default()
}
impl Workbench {
    fn proxy_directory(&self) -> PathBuf {
        self.workspace
            .storage
            .connection
            .path()
            .map(PathBuf::from)
            .and_then(|path| path.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(std::env::temp_dir)
            .join("proxy-service")
    }
    fn proxy_ca_directory(&self) -> PathBuf {
        self.proxy_directory().join("ca")
    }
    fn start_proxy_service(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if self.proxy_certificate_task.is_some() {
            return Err("请等待证书操作完成".into());
        }
        let index = self.editing_proxy;
        let id = self.workspace.proxies[index].id.clone();
        if self.proxy_services.contains_key(&id) {
            return Ok(());
        }
        let settings = self.profile_draft(index, cx);
        settings.listener.validate()?;
        let authority = Arc::new(proxy_service::ca::Authority::generate(
            &self.proxy_ca_directory(),
        )?);
        self.proxy_certificate_status = format!(
            "{} · 有效期至 {}\nSHA-256 {}",
            authority.name, authority.expires, authority.fingerprint
        );
        let captures = self.proxy_captures.entry(id.clone()).or_default().clone();
        let service = proxy_service::RunningProxy::start(settings, Some(authority), captures)?;
        self.notice = format!(
            "代理服务已启动：{}；修改代理配置后需停止并重新启动服务",
            service.endpoint()
        );
        self.proxy_services.insert(id, service);
        cx.notify();
        Ok(())
    }
    fn launch_proxy_browser(
        &mut self,
        browser: proxy_service::browser::Browser,
        cx: &mut Context<Self>,
    ) {
        self.proxy_app_menu = false;
        let result = self.start_proxy_service(cx).and_then(|_| {
            let service = &self.proxy_services[&self.workspace.proxies[self.editing_proxy].id];
            proxy_service::browser::launch(
                &browser,
                &self.proxy_directory(),
                service.address.port(),
            )
        });
        match result {
            Ok(()) => {
                self.variable_error = None;
                self.notice = format!(
                    "已打开 {} 独立代理窗口；HTTPS 抓包需先安装代理 CA",
                    browser.name
                );
            }
            Err(error) => self.variable_error = Some(error),
        }
        cx.notify();
    }
    fn proxy_browser_menu(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let mut content = div().flex().flex_col();
        for (index, browser) in self.proxy_browsers.iter().enumerate() {
            let browser = browser.clone();
            content = content.child(
                MenuItem::new(("proxy-browser", index), browser.name.clone())
                    .theme(self.theme)
                    .render()
                    .debug_selector(move || format!("proxy-browser-{index}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.launch_proxy_browser(browser.clone(), cx)
                    })),
            );
        }
        if self.proxy_browsers.is_empty() {
            content = content.child(div().p_2().child("未检测到 Chrome、Edge、Firefox 或 Brave"));
        }
        let owner = cx.entity().downgrade();
        DropdownMenu::new(
            "proxy-open-application-menu",
            Button::new("打开应用")
                .icon(Icon::ChevronDown)
                .kind(ButtonKind::Gray)
                .size(ButtonSize::Small)
                .theme(self.theme)
                .render()
                .debug_selector(|| "proxy-open-application".into()),
            content,
        )
        .width(240.)
        .theme(self.theme)
        .open(self.proxy_app_menu)
        .on_open_change(move |open, _, cx| {
            let _ = owner.update(cx, |this, cx| {
                this.proxy_app_menu = open;
                if open {
                    this.proxy_browsers = proxy_service::browser::discover();
                }
                cx.notify();
            });
        })
        .render()
    }
    pub(super) fn proxy_workspace(&self, cx: &mut Context<Self>) -> gpui::Div {
        if self.workspace.proxies.is_empty() {
            return self.proxy_settings_workspace(cx);
        }
        let profile = &self.workspace.proxies[self.editing_proxy];
        self.proxy_capture_panel.update(cx, |panel, cx| {
            panel.bind_profile(
                &profile.id,
                self.proxy_captures.get(&profile.id).cloned(),
                cx,
            );
            panel.set_theme(self.theme, cx);
        });
        let owner = cx.entity().downgrade();
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_6()
                    .pt_4()
                    .pb_2()
                    .flex()
                    .items_center()
                    .gap_3()
                    .flex_shrink_0()
                    .child(
                        SegmentedControl::new([
                            SegmentOption::new("代理设置"),
                            SegmentOption::new("代理抓包"),
                        ])
                        .id("proxy-workspace-tabs")
                        .selected(self.proxy_tab)
                        .theme(self.theme)
                        .on_change(move |tab, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.proxy_tab = tab;
                                this.proxy_app_menu = false;
                                cx.notify();
                            });
                        })
                        .render()
                        .debug_selector(|| "proxy-workspace-tabs".into()),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(self.theme.palette().text_muted))
                            .child(
                                self.proxy_services
                                    .get(&profile.id)
                                    .map_or("代理服务未启动".into(), |s| {
                                        format!("监听 {}", s.address)
                                    }),
                            ),
                    )
                    .child(self.proxy_browser_menu(cx)),
            )
            .when(self.proxy_tab == 1, |root| {
                root.when_some(self.variable_error.clone(), |root, error| {
                    root.child(
                        div()
                            .px_6()
                            .pb_2()
                            .flex_shrink_0()
                            .text_color(rgb(self.theme.color(0xd9342b)))
                            .child(error),
                    )
                })
            })
            .child(if self.proxy_tab == 0 {
                self.proxy_settings_workspace(cx).into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .p_6()
                    .child(self.proxy_capture_panel.clone())
                    .into_any_element()
            })
    }
    pub(super) fn proxy_listener_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let index = self.editing_proxy;
        let input = &self.profile_inputs[index];
        let id = self.workspace.proxies[index].id.clone();
        let running = self.proxy_services.get(&id);
        let owner = cx.entity().downgrade();
        let mut actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                gpui_glint::components::InputField::new("监听端口")
                    .control(input.listener_port.clone())
                    .width(110.)
                    .theme(self.theme)
                    .render(),
            )
            .child(
                Button::new(if running.is_some() {
                    "停止代理服务"
                } else {
                    "启动代理服务"
                })
                .kind(if running.is_some() {
                    ButtonKind::Danger
                } else {
                    ButtonKind::Primary
                })
                .size(ButtonSize::Small)
                .theme(self.theme)
                .render()
                .debug_selector(|| "proxy-listener-toggle".into())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.proxy_services.remove(&id).is_some() {
                        this.notice = "代理服务已停止，现有连接已关闭".into();
                        this.variable_error = None;
                    } else if let Err(error) = this.start_proxy_service(cx) {
                        this.variable_error = Some(error);
                    }
                    cx.notify();
                })),
            );
        if let Some(service) = running {
            let endpoint = service.endpoint();
            actions = actions.child(
                Button::new("复制代理地址")
                    .kind(ButtonKind::Plain)
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(endpoint.clone()))
                    }),
            );
        }
        let mut certificate_actions = div().flex().items_center().gap_2();
        for (label, selector, action) in [
            ("生成证书", "proxy-ca-generate", CertificateAction::Generate),
            (
                "导入证书和私钥",
                "proxy-ca-import",
                CertificateAction::Import,
            ),
            ("导出 CA 证书", "proxy-ca-export", CertificateAction::Export),
            (
                "安装到当前用户",
                "proxy-ca-install",
                CertificateAction::Install,
            ),
        ] {
            certificate_actions =
                certificate_actions.child(
                    Button::new(label)
                        .kind(ButtonKind::Gray)
                        .size(ButtonSize::Mini)
                        .theme(self.theme)
                        .when(self.proxy_certificate_task.is_some(), |b| b.disabled())
                        .render()
                        .debug_selector(move || selector.into())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.proxy_certificate_action(action, cx)
                        })),
                );
        }
        div().debug_selector(|| "proxy-listener-settings".into()).flex().flex_col().gap_3().pt_3().border_t_1().border_color(rgb(self.theme.palette().border))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("端口代理"))
            .child(actions)
            .child(Switch::new("允许局域网访问").checked(input.allow_lan).theme(self.theme)
                .on_change(move |allow, _, cx| { let _ = owner.update(cx, |this, cx| { this.profile_inputs[index].allow_lan = allow; cx.notify(); }); }).render())
            .child(div().text_size(px(11.)).text_color(rgb(self.theme.palette().text_muted))
                .child("默认仅本机 127.0.0.1；允许局域网后监听 0.0.0.0，其他设备可使用本机 IP 与此端口，无需代理账号。修改配置后需重启代理服务。"))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("HTTPS 抓包证书"))
            .child(certificate_actions)
            .child(div().text_size(px(11.)).text_color(rgb(self.theme.palette().text_muted))
                .child(if self.proxy_certificate_status.is_empty() { "首次启动自动生成本地 CA；已有 CA 可导入 PEM 证书和未加密私钥。".into() } else { self.proxy_certificate_status.clone() }))
            .child(div().text_size(px(11.)).text_color(rgb(self.theme.palette().text_muted))
                .child("HTTPS 解密需要客户端信任此 CA；局域网设备需导出后安装。导出仅包含公钥证书；安装影响当前用户信任库。独立浏览器窗口关闭后不改变日常浏览器的代理设置。"))
    }
    fn proxy_certificate_action(&mut self, action: CertificateAction, cx: &mut Context<Self>) {
        if self.proxy_certificate_task.is_some() {
            return;
        }
        if matches!(action, CertificateAction::Import) && !self.proxy_services.is_empty() {
            self.variable_error = Some("请先停止所有代理服务，再导入替换 CA 证书".into());
            cx.notify();
            return;
        }
        let directory = self.proxy_ca_directory();
        let picker = match action {
            CertificateAction::Import => Some(cx.prompt_for_paths(gpui::PathPromptOptions {
                files: true,
                directories: false,
                multiple: true,
                prompt: Some("选择 CA PEM 证书和私钥（可为一个合并文件）".into()),
            })),
            _ => None,
        };
        let destination = matches!(action, CertificateAction::Export)
            .then(|| cx.prompt_for_new_path(&self.proxy_directory(), Some("glint-proxy-ca.cer")));
        self.proxy_certificate_task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let import = if let Some(picker) = picker {
                    let paths = picker
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?;
                    let Some(paths) = paths else {
                        return Ok(None);
                    };
                    if paths.is_empty() || paths.len() > 2 {
                        return Err("请选择一个合并 PEM 文件，或证书和私钥两个文件".into());
                    }
                    Some(paths)
                } else {
                    None
                };
                let export = if let Some(destination) = destination {
                    let path = destination
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?;
                    let Some(path) = path else {
                        return Ok(None);
                    };
                    Some(path)
                } else {
                    None
                };
                cx.background_executor()
                    .spawn(async move {
                        match action {
                            CertificateAction::Generate => {
                                proxy_service::ca::Authority::generate(&directory)?;
                            }
                            CertificateAction::Import => {
                                let paths = import.unwrap();
                                let pem = paths
                                    .iter()
                                    .map(std::fs::read_to_string)
                                    .collect::<Result<Vec<_>, _>>()
                                    .map_err(|e| e.to_string())?
                                    .join("\n");
                                proxy_service::ca::Authority::import(&directory, &pem, &pem)?;
                            }
                            CertificateAction::Export => {
                                proxy_service::ca::Authority::load(&directory)?
                                    .export(&export.unwrap())?
                            }
                            CertificateAction::Install => {
                                proxy_service::ca::install(&directory)?;
                            }
                        }
                        let ca = proxy_service::ca::Authority::load(&directory)?;
                        Ok(Some(format!(
                            "{} · 有效期至 {}\nSHA-256 {}",
                            ca.name, ca.expires, ca.fingerprint
                        )))
                    })
                    .await
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.proxy_certificate_task = None;
                match result {
                    Ok(Some(status)) => {
                        this.proxy_certificate_status = status;
                        this.variable_error = None;
                        this.notice = match action {
                            CertificateAction::Install => "已安装代理 CA，请重启代理浏览器",
                            CertificateAction::Export => "已导出公开 CA 证书",
                            CertificateAction::Import => "代理 CA 已导入",
                            CertificateAction::Generate => "代理 CA 已就绪",
                        }
                        .into();
                    }
                    Ok(None) => {}
                    Err(error) => this.variable_error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn proxy_listener_starts_on_configured_port_captures_and_stops_on_workspace_switch(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::io::{Read, Write};
        let directory = tempfile::tempdir().unwrap();
        let origin = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_port = origin.local_addr().unwrap().port();
        let origin_task = std::thread::spawn(move || {
            let (mut socket, _) = origin.accept().unwrap();
            let mut request = [0u8; 4096];
            socket.read(&mut request).unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\ncaptured",
                )
                .unwrap();
        });
        let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join("listener.sqlite3")).unwrap(),
                cx,
            );
            host.workspace.proxies.push(environment::ProxyProfile {
                id: uuid::Uuid::new_v4().to_string(),
                name: "端口测试".into(),
                settings: environment::ProxySettings {
                    built_in: true,
                    mappings: vec![environment::HostMapping {
                        domain: "captured.test".into(),
                        ip: "127.0.0.1".into(),
                    }],
                    ..Default::default()
                },
            });
            host.reset_proxy_profiles(cx);
            host.page = crate::Page::Proxies;
            host
        });
        let endpoint = host.update(window, |host, cx| {
            assert!(host.proxy_services.is_empty());
            host.profile_inputs[0]
                .listener_port
                .update(cx, |entry, cx| entry.set_value("0", cx));
            assert!(host.start_proxy_service(cx).unwrap_err().contains("端口"));
            host.profile_inputs[0]
                .listener_port
                .update(cx, |entry, cx| entry.set_value(port.to_string(), cx));
            host.start_proxy_service(cx).unwrap();
            assert!(host.proxy_certificate_status.contains("SHA-256"));
            let service = &host.proxy_services[&host.workspace.proxies[0].id];
            assert_eq!(service.address.port(), port);
            assert!(service.address.ip().is_loopback());
            service.endpoint()
        });
        let result = reqwest::blocking::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(endpoint).unwrap())
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap()
            .get(format!("http://captured.test:{origin_port}/test"))
            .send()
            .unwrap()
            .text()
            .unwrap();
        assert_eq!(result, "captured");
        origin_task.join().unwrap();
        host.update(window, |host, cx| {
            host.proxy_tab = 1;
            cx.notify();
        });
        assert!(window.debug_bounds("proxy-capture-page").is_some());
        assert!(window.debug_bounds("table-empty-state").is_none());
        let captures = host.read_with(window, |host, _| {
            host.proxy_captures[&host.workspace.proxies[0].id].clone()
        });
        assert_eq!(captures.summaries().len(), 1);
        let other = host.update(window, |host, _| {
            host.workspace
                .storage
                .create_workspace("另一工作区")
                .unwrap()
        });
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.switch_workspace(&other, window, cx).unwrap()
            })
        });
        host.read_with(window, |host, _| assert!(host.proxy_services.is_empty()));
        // Drop requests asynchronously on the proxy runtime; eventually no listener remains.
        for _ in 0..100 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
    }
}
