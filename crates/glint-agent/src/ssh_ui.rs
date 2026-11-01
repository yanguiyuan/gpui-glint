//! Connect to a remote directory without mounting or copying the workspace.
use super::*;
use gpui_glint::components::{Dialog, InputField};

pub struct State {
    pub open: bool,
    host: Entity<TextEntry>,
    directory: Entity<TextEntry>,
    task: Option<Task<()>>,
    error: Option<String>,
}
impl State {
    pub fn new(cx: &mut Context<AgentWorkspace>) -> Self {
        Self {
            open: false,
            host: cx.new(|cx| TextEntry::new(cx, "ssh user@example.com", "")),
            directory: cx.new(|cx| TextEntry::new(cx, "远程目录，例如 ~/projects/demo", "~")),
            task: None,
            error: None,
        }
    }
    pub fn set_theme(&self, theme: ThemePreset, cx: &mut Context<AgentWorkspace>) {
        for input in [&self.host, &self.directory] {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
    }
}
impl AgentWorkspace {
    pub(crate) fn open_ssh_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.project_picker_open = false;
        self.ssh_ui.open = true;
        self.ssh_ui.error = None;
        self.ssh_ui.set_theme(self.theme, cx);
        window.focus(&self.ssh_ui.host.focus_handle(cx), cx);
        cx.notify();
    }
    fn close_ssh_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ssh_ui.task = None;
        self.ssh_ui.open = false;
        window.focus(&self.composer.focus_handle(cx), cx);
        cx.notify();
    }
    fn connect_ssh_project(&mut self, cx: &mut Context<Self>) {
        if self.ssh_ui.task.is_some() {
            return;
        }
        let remote = match ssh_project::Remote::parse(
            self.ssh_ui.host.read(cx).value(),
            self.ssh_ui.directory.read(cx).value(),
        ) {
            Ok(remote) => remote,
            Err(error) => {
                self.ssh_ui.error = Some(error);
                cx.notify();
                return;
            }
        };
        let job = match provider::Job::start(remote.connect()) {
            Ok(job) => job,
            Err(error) => {
                self.ssh_ui.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.ssh_ui.error = None;
        self.ssh_ui.task = Some(cx.spawn(async move |weak, cx| {
            let result = job.result().await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.ssh_ui.task.take() {
                    task.detach();
                }
                match result {
                    Ok(remote) => {
                        this.ssh_ui.open = false;
                        this.choose_project(remote.uri(), cx);
                    }
                    Err(error) => this.ssh_ui.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(crate) fn ssh_project_dialog(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let content = div().flex().flex_col().gap_4()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("打开 SSH 项目"))
            .child(InputField::new("SSH 连接").label("SSH 连接").control(self.ssh_ui.host.clone()).theme(self.theme).render())
            .child(InputField::new("远程项目目录").label("远程项目目录").control(self.ssh_ui.directory.clone()).theme(self.theme).render())
            .child(div().text_size(px(12.)).text_color(rgb(self.theme.palette().text_muted))
                .child("使用系统 SSH 密钥、ssh-agent 和 ~/.ssh/config。远端需要 python3；留空目录或填写 ~ 将打开远程主目录。"))
            .when_some(self.ssh_ui.error.clone(), |body, error| body.child(div().text_size(px(12.)).text_color(rgb(0xd9342b)).child(error)))
            .child(div().flex().justify_end().gap_3()
                .child(Button::new("取消").kind(ButtonKind::Plain).theme(self.theme).render()
                    .debug_selector(|| "cancel-ssh-project".into())
                    .on_click(cx.listener(|this, _, window, cx| this.close_ssh_project(window, cx))))
                .child(Button::new(if self.ssh_ui.task.is_some() { "连接中…" } else { "连接并打开" }).theme(self.theme)
                    .when(self.ssh_ui.task.is_some(), |button| button.disabled()).render()
                    .debug_selector(|| "connect-ssh-project".into())
                    .on_click(cx.listener(|this, _, _, cx| this.connect_ssh_project(cx)))));
        let weak = cx.entity().downgrade();
        Dialog::new("ssh-project-dialog", content)
            .width(520.)
            .theme(self.theme)
            .focus_handles([
                self.ssh_ui.host.focus_handle(cx),
                self.ssh_ui.directory.focus_handle(cx),
            ])
            .on_close(move |_, window, cx| {
                let _ = weak.update(cx, |this, cx| this.close_ssh_project(window, cx));
            })
            .render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn ssh_project_dialog_opens_validates_and_cancels_without_losing_draft(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let host = AgentWorkspace::with_path(dir.path().join("sessions.json"), cx);
            host.composer
                .update(cx, |c, cx| c.set_value("keep draft", cx));
            host
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let open = window.debug_bounds("open-ssh-project").unwrap();
        window.simulate_click(open.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, _| assert!(h.ssh_ui.open));
        host.update(window, |h, cx| {
            h.ssh_ui.host.update(cx, |input, cx| {
                input.set_value("ssh -o Bad=option host", cx)
            })
        });
        let connect = window.debug_bounds("connect-ssh-project").unwrap();
        window.simulate_click(connect.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, _| {
            assert!(h.ssh_ui.error.is_some());
            assert!(h.ssh_ui.task.is_none());
        });
        let cancel = window.debug_bounds("cancel-ssh-project").unwrap();
        window.simulate_click(cancel.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, cx| {
            assert!(!h.ssh_ui.open);
            assert_eq!(h.composer.read(cx).value(cx).as_ref(), "keep draft");
            assert!(h.history.active().project.is_none());
        });
    }

    #[test]
    fn remote_project_identity_is_saved_and_grouped_with_its_host() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let remote = ssh_project::Remote::parse("ssh user@example.com", "/home/user/demo").unwrap();
        let mut h = History::default();
        h.projects.push(remote.uri());
        h.active_mut().project = Some(remote.uri());
        h.save(&path).unwrap();
        let restored = History::load(&path).unwrap();
        assert_eq!(restored.active().project.as_ref(), Some(&remote.uri()));
        assert_eq!(
            ssh_project::Remote::from_path(&restored.projects[0]).unwrap(),
            remote
        );
        let groups = project_ui::session_groups(&restored, "", None);
        assert!(
            groups
                .iter()
                .any(|g| g.header.label.contains("example.com"))
        );
    }
}
