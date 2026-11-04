//! Agent command palette, shared by the topbar and Cmd/Ctrl+K.
use super::*;
use gpui_glint::components::{CommandItem, CommandPalette, CommandPaletteEvent};
pub struct State {
    pub palette: Entity<CommandPalette>,
    pub pending: Option<SharedString>,
}
impl State {
    pub fn new(cx: &mut Context<AgentWorkspace>) -> Self {
        let palette = cx.new(|cx| CommandPalette::new([], cx));
        cx.subscribe(&palette, |this, _, event: &CommandPaletteEvent, cx| {
            if let CommandPaletteEvent::Executed(id) = event {
                this.commands.pending = Some(id.clone());
            }
            cx.notify();
        })
        .detach();
        Self {
            palette,
            pending: None,
        }
    }
}
impl AgentWorkspace {
    pub fn open_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let busy = self.session_busy(self.history.active);
        let session = self.history.active();
        let can_compact = session
            .messages
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, m)| m.role == Role::User)
            .nth(1)
            .is_some_and(|(index, _)| index > session.compaction.as_ref().map_or(0, |s| s.through));
        let mut items = vec![
            CommandItem::new("new", "新建会话")
                .icon(Icon::Plus)
                .group("会话"),
            CommandItem::new("search", "搜索会话")
                .icon(Icon::Search)
                .group("会话"),
            CommandItem::new("compact", "压缩会话")
                .icon(Icon::Shuffle)
                .group("会话")
                .keywords("压缩 上下文 context compact summarize")
                .disabled(busy || !self.active_model_enabled() || !can_compact),
            CommandItem::new("local", "打开本地项目")
                .icon(Icon::Folder)
                .group("项目"),
            CommandItem::new("ssh", "打开 SSH 项目")
                .icon(Icon::Globe)
                .group("项目"),
            CommandItem::new("gitgraph", "打开 GitGraph 侧栏")
                .icon(Icon::GitBranch)
                .group("项目")
                .keywords("git graph 分支 提交 历史 对比 差异")
                .disabled(session.project.is_none()),
            CommandItem::new("workspace-diff", "打开 Git 工作区 Diff 侧栏")
                .icon(Icon::GitBranch)
                .group("项目")
                .keywords("diff 差异 变更 工作区 暂存 workspace changes")
                .disabled(session.project.is_none()),
            CommandItem::new("project-files", "打开项目目录预览侧栏")
                .icon(Icon::Folder)
                .group("项目")
                .keywords("文件 目录 预览 浏览 file preview explorer")
                .disabled(session.project.is_none()),
            CommandItem::new("project-terminal", "打开项目终端")
                .icon(Icon::Shell)
                .group("项目")
                .keywords("终端 terminal shell bash gitbash powershell 命令行")
                .disabled(session.project.is_none()),
            CommandItem::new("settings", "设置")
                .icon(Icon::Settings)
                .group("应用"),
        ];
        items.extend(self.history.sessions.iter().map(|session| {
            CommandItem::new(format!("session:{}", session.id), session.title.clone())
                .group("切换会话")
                .icon(Icon::MessageCircle)
        }));
        self.commands.palette.update(cx, |palette, cx| {
            palette.set_theme(self.theme, cx);
            palette.set_items(items, cx);
            palette.open(window, cx);
        });
        cx.notify();
    }
    pub fn execute_pending_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = self.commands.pending.take() else {
            return;
        };
        match command.as_ref() {
            "new" => self.new_session(window, cx),
            "search" => {
                self.page = crate::Page::Chat;
                self.search_sessions(window, cx);
            }
            "compact" => self.compact_session(cx),
            "local" => self.open_project(cx),
            "ssh" => self.open_ssh_project(window, cx),
            "gitgraph" => self.open_git_graph(cx),
            "workspace-diff" => self.open_workspace_diff(cx),
            "project-files" => self.open_project_directory(cx),
            "project-terminal" => self.open_project_terminal(window, cx),
            "settings" => self.open_settings(window, cx),
            value if value.starts_with("session:") => {
                if let Ok(id) = Uuid::parse_str(&value[8..]) {
                    self.switch(id, cx);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn project_sidebar_commands_can_be_searched_and_executed(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        cx.update(crate::initialize);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        host.update(window, |host, _| {
            host.history.active_mut().project = Some(dir.path().into())
        });
        for (query, panel) in [
            ("打开 Git 工作区 Diff 侧栏", 0),
            ("打开项目目录预览侧栏", 1),
            ("打开项目终端", 2),
        ] {
            window.update(|window, cx| host.update(cx, |host, cx| host.open_commands(window, cx)));
            host.update(window, |host, cx| {
                host.commands
                    .palette
                    .update(cx, |palette, cx| palette.set_query(query, cx))
            });
            window.run_until_parked();
            window.simulate_keystrokes("enter");
            window.run_until_parked();
            host.read_with(window, |host, cx| {
                assert!(!host.commands.palette.read(cx).is_open());
                assert_eq!(host.edits_ui.open, panel == 0);
                assert_eq!(host.file_preview.open, panel == 1);
                assert_eq!(host.terminal_ui.open, panel == 2);
                assert!(!host.git_ui.open);
            });
        }
        assert!(window.debug_bounds("project-terminal-sidebar").is_some());
    }
    #[gpui::test]
    fn topbar_and_shortcut_open_palette_escape_and_execution_remove_overlay(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(crate::initialize);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        host.update(window, |h, cx| {
            h.history.active_mut().push(Role::User, "已有问题".into());
            cx.notify();
        });
        let button = window.debug_bounds("agent-topbar-search").unwrap();
        window.simulate_click(button.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |h, cx| {
            assert!(h.commands.palette.read(cx).is_open())
        });
        assert!(window.debug_bounds("glass-command-palette").is_some());
        window.simulate_keystrokes("escape");
        window.run_until_parked();
        assert!(window.debug_bounds("glass-command-palette").is_none());
        window.update(|window, cx| {
            let focus = host.read(cx).focus.clone();
            window.focus(&focus, cx);
        });
        window.simulate_keystrokes("cmd-k");
        window.run_until_parked();
        host.update(window, |h, cx| {
            h.commands
                .palette
                .update(cx, |p, cx| p.set_query("新建会话", cx))
        });
        window.run_until_parked();
        let count = host.read_with(window, |h, _| h.history.sessions.len());
        window.simulate_keystrokes("enter");
        window.run_until_parked();
        host.read_with(window, |h, cx| {
            assert!(!h.commands.palette.read(cx).is_open());
            assert_eq!(h.history.sessions.len(), count + 1);
        });
        assert!(window.debug_bounds("glass-command-palette").is_none());
    }
}
