//! Workspace-local shortcuts take effect immediately without app-global rebinding.
use super::*;

pub fn default_terminal_shortcut() -> String {
    "ctrl-`".into()
}

pub fn default_diff_shortcut() -> String {
    if cfg!(target_os = "macos") {
        "cmd-shift-d"
    } else {
        "ctrl-shift-d"
    }
    .into()
}

pub fn default_preview_shortcut() -> String {
    if cfg!(target_os = "macos") {
        "cmd-shift-e"
    } else {
        "ctrl-shift-e"
    }
    .into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectAction {
    Terminal,
    Diff,
    Preview,
}

impl ProjectAction {
    pub const ALL: [Self; 3] = [Self::Terminal, Self::Diff, Self::Preview];
    pub fn id(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Diff => "diff",
            Self::Preview => "preview",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Terminal => "打开项目终端",
            Self::Diff => "打开 Diff 面板",
            Self::Preview => "打开项目预览面板",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Terminal => "在当前项目目录打开并聚焦终端",
            Self::Diff => "查看当前项目的 Git 工作区变更",
            Self::Preview => "浏览当前项目的目录和文件内容",
        }
    }
    pub fn default_binding(self) -> String {
        match self {
            Self::Terminal => default_terminal_shortcut(),
            Self::Diff => default_diff_shortcut(),
            Self::Preview => default_preview_shortcut(),
        }
    }
    fn stored(self, history: &History) -> &str {
        match self {
            Self::Terminal => &history.terminal_shortcut,
            Self::Diff => &history.diff_shortcut,
            Self::Preview => &history.preview_shortcut,
        }
    }
    fn binding(self, history: &History) -> gpui::Keystroke {
        gpui::Keystroke::parse(self.stored(history))
            .ok()
            .filter(|key| validate_terminal_shortcut(key).is_ok())
            .unwrap_or_else(|| gpui::Keystroke::parse(&self.default_binding()).unwrap())
    }
    fn store(self, history: &mut History, value: String) {
        match self {
            Self::Terminal => history.terminal_shortcut = value,
            Self::Diff => history.diff_shortcut = value,
            Self::Preview => history.preview_shortcut = value,
        }
    }
    pub fn label(self, history: &History) -> String {
        keystroke_label(&self.binding(history))
    }
}

/// New defaults must not take over a shortcut customized by an older version.
pub(super) fn repair_project_shortcut_conflicts(history: &mut History) {
    let mut used = std::collections::HashSet::new();
    for action in ProjectAction::ALL {
        if used.insert(action.binding(history).unparse()) { continue; }
        let letter = match action { ProjectAction::Terminal => "t", ProjectAction::Diff => "d", ProjectAction::Preview => "e" };
        let fallback = [action.default_binding(), format!("ctrl-alt-shift-{letter}"), format!("alt-shift-{letter}")]
            .into_iter().find(|candidate| !used.contains(candidate)).unwrap();
        used.insert(fallback.clone());
        action.store(history, fallback);
    }
}

fn terminal_keystroke(value: &str) -> gpui::Keystroke {
    gpui::Keystroke::parse(value)
        .ok()
        .filter(|key| validate_terminal_shortcut(key).is_ok())
        .unwrap_or_else(|| gpui::Keystroke::parse(&default_terminal_shortcut()).unwrap())
}

pub fn terminal_shortcut_label(value: &str) -> String {
    let key = terminal_keystroke(value);
    keystroke_label(&key)
}

fn keystroke_label(key: &gpui::Keystroke) -> String {
    let mut parts = Vec::new();
    if key.modifiers.control {
        parts.push("Ctrl".to_owned());
    }
    if key.modifiers.alt {
        parts.push("Alt".to_owned());
    }
    if key.modifiers.shift {
        parts.push("Shift".to_owned());
    }
    if key.modifiers.platform {
        parts.push(
            if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Win"
            }
            .to_owned(),
        );
    }
    if key.modifiers.function {
        parts.push("Fn".to_owned());
    }
    parts.push(key.key.to_uppercase());
    parts.join(" + ")
}

fn validate_terminal_shortcut(key: &gpui::Keystroke) -> Result<(), &'static str> {
    let modifiers = key.modifiers;
    if !(modifiers.control || modifiers.alt || modifiers.platform) {
        return Err("请同时按下 Ctrl、Alt 或 Command，避免影响正常输入。");
    }
    if !modifiers.alt
        && (modifiers.control || modifiers.platform)
        && ((!modifiers.shift
            && matches!(
                key.key.as_str(),
                "n" | "k" | "," | "enter" | "a" | "c" | "v" | "x" | "z"
            ))
            || (modifiers.shift && matches!(key.key.as_str(), "c" | "v" | "z")))
    {
        return Err("该组合键已用于应用或文本编辑，请选择其他组合键。");
    }
    Ok(())
}

impl AgentWorkspace {
    fn validate_project_shortcut(
        &self,
        action: ProjectAction,
        key: &gpui::Keystroke,
    ) -> Result<(), String> {
        validate_terminal_shortcut(key).map_err(str::to_owned)?;
        for other in ProjectAction::ALL {
            let binding = other.binding(&self.history);
            if other != action && binding.modifiers == key.modifiers && binding.key == key.key {
                return Err(format!(
                    "该组合键已用于“{}”，请选择其他组合键。",
                    other.title()
                ));
            }
        }
        Ok(())
    }

    pub(super) fn reset_project_shortcut(&mut self, action: ProjectAction, cx: &mut Context<Self>) {
        let key = gpui::Keystroke::parse(&action.default_binding()).unwrap();
        self.provider_settings.shortcut_recording = None;
        match self.validate_project_shortcut(action, &key) {
            Ok(()) => {
                action.store(&mut self.history, key.unparse());
                self.provider_settings.shortcut_message = None;
                self.save(cx);
            }
            Err(message) => self.provider_settings.shortcut_message = Some(message),
        }
        cx.notify();
    }

    pub(super) fn project_shortcut(
        &mut self,
        event: &gpui::KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let on_settings = self.page == crate::Page::Settings
            && self.provider_settings.section == provider_ui::Section::Shortcuts;
        if !on_settings {
            self.provider_settings.shortcut_recording = None;
        }
        if let Some(action) = self.provider_settings.shortcut_recording {
            window.prevent_default();
            cx.stop_propagation();
            let key = &event.keystroke;
            if key.key == "escape" {
                self.provider_settings.shortcut_recording = None;
                self.provider_settings.shortcut_message = None;
            } else if !matches!(
                key.key.as_str(),
                "ctrl" | "control" | "alt" | "shift" | "cmd" | "super" | "win" | "fn"
            ) {
                match self.validate_project_shortcut(action, key) {
                    Ok(()) => {
                        action.store(&mut self.history, key.unparse());
                        self.provider_settings.shortcut_recording = None;
                        self.provider_settings.shortcut_message = None;
                        self.save(cx);
                    }
                    Err(message) => self.provider_settings.shortcut_message = Some(message),
                }
            }
            cx.notify();
            return;
        }
        if event.keystroke.is_ime_in_progress() || self.history.active().project.is_none() {
            return;
        }
        let Some(action) = ProjectAction::ALL.into_iter().find(|action| {
            let binding = action.binding(&self.history);
            event.keystroke.modifiers == binding.modifiers && event.keystroke.key == binding.key
        }) else {
            return;
        };
        window.prevent_default();
        cx.stop_propagation();
        self.commands
            .palette
            .update(cx, |palette, cx| palette.close(window, cx));
        match action {
            ProjectAction::Terminal => self.open_project_terminal(window, cx),
            ProjectAction::Diff => self.open_workspace_diff(cx),
            ProjectAction::Preview => self.open_project_directory(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_shortcuts_keep_old_settings_compatible_and_reject_conflicts() {
        let mut value = serde_json::to_value(model::History::default()).unwrap();
        value.as_object_mut().unwrap().remove("terminal_shortcut");
        value.as_object_mut().unwrap().remove("diff_shortcut");
        value.as_object_mut().unwrap().remove("preview_shortcut");
        let history: model::History = serde_json::from_value(value).unwrap();
        assert_eq!(history.terminal_shortcut, default_terminal_shortcut());
        assert_eq!(history.diff_shortcut, default_diff_shortcut());
        assert_eq!(history.preview_shortcut, default_preview_shortcut());
        let mut conflicting = history.clone();
        conflicting.terminal_shortcut = default_diff_shortcut();
        repair_project_shortcut_conflicts(&mut conflicting);
        assert_eq!(conflicting.terminal_shortcut, default_diff_shortcut());
        assert_ne!(conflicting.diff_shortcut, conflicting.terminal_shortcut);
        assert_eq!(terminal_shortcut_label("ctrl-shift-j"), "Ctrl + Shift + J");
        assert_eq!(
            terminal_keystroke("ctrl-k").unparse(),
            default_terminal_shortcut()
        );
        for key in [
            "a",
            "shift-a",
            "ctrl-k",
            "cmd-n",
            "ctrl-shift-v",
            "ctrl-enter",
        ] {
            assert!(
                validate_terminal_shortcut(&gpui::Keystroke::parse(key).unwrap()).is_err(),
                "{key}"
            );
        }
        for key in ["ctrl-`", "ctrl-shift-j", "alt-t", "cmd-shift-t"] {
            assert!(
                validate_terminal_shortcut(&gpui::Keystroke::parse(key).unwrap()).is_ok(),
                "{key}"
            );
        }
    }

    #[gpui::test]
    fn diff_and_preview_shortcuts_record_reject_duplicates_and_open_panels(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        cx.update(crate::initialize);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        std::fs::write(dir.path().join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        let (host, window) = cx.add_window_view(|_, cx| AgentWorkspace::with_path(path.clone(), cx));
        window.simulate_resize(size(px(1440.), px(1200.)));
        host.update(window, |host, cx| {
            host.history.active_mut().project = Some(dir.path().into());
            host.page = crate::Page::Settings;
            host.provider_settings.section = provider_ui::Section::Shortcuts;
            cx.notify();
        });
        window.run_until_parked();
        for (action, custom) in [(ProjectAction::Diff, "ctrl-alt-d"), (ProjectAction::Preview, "ctrl-alt-e")] {
            let selector = if action == ProjectAction::Diff { "record-diff-shortcut" } else { "record-preview-shortcut" };
            let record = window.debug_bounds(selector).unwrap();
            window.simulate_click(record.center(), Default::default());
            window.simulate_keystrokes("ctrl-`");
            host.read_with(window, |host, _| {
                assert_eq!(host.provider_settings.shortcut_recording, Some(action));
                assert!(host.provider_settings.shortcut_message.is_some());
                assert!(!host.terminal_ui.open);
            });
            window.simulate_keystrokes(custom);
            window.run_until_parked();
            host.read_with(window, |host, _| {
                assert_eq!(action.stored(&host.history), custom);
                assert!(host.provider_settings.shortcut_recording.is_none());
                host.history.save(&path).unwrap();
                assert_eq!(action.stored(&History::load(&path).unwrap()), custom);
            });
        }
        window.simulate_keystrokes("ctrl-alt-d");
        window.run_until_parked();
        host.read_with(window, |host, _| assert!(host.edits_ui.open && !host.file_preview.open));
        window.simulate_keystrokes("ctrl-alt-e");
        window.run_until_parked();
        host.read_with(window, |host, _| assert!(host.file_preview.open && !host.edits_ui.open));
        host.update(window, |host, cx| {
            host.page = crate::Page::Settings;
            cx.notify();
        });
        window.run_until_parked();
        for action in [ProjectAction::Diff, ProjectAction::Preview] {
            let selector = if action == ProjectAction::Diff { "reset-diff-shortcut" } else { "reset-preview-shortcut" };
            let reset = window.debug_bounds(selector).unwrap();
            window.simulate_click(reset.center(), Default::default());
            window.run_until_parked();
            host.read_with(window, |host, _| assert_eq!(action.stored(&host.history), action.default_binding()));
        }
    }

    #[gpui::test]
    fn terminal_shortcut_records_persists_resets_and_opens_from_the_composer(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        cx.update(crate::initialize);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) =
            cx.add_window_view(|_, cx| AgentWorkspace::with_path(path.clone(), cx));
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                window.focus(&host.composer.focus_handle(cx), cx);
            })
        });
        window.simulate_keystrokes("ctrl-`");
        host.read_with(window, |host, _| assert!(!host.terminal_ui.open));
        host.update(window, |host, cx| {
            host.history.active_mut().project = Some(dir.path().into());
            cx.notify();
        });
        window.run_until_parked();
        window.simulate_keystrokes("ctrl-`");
        window.run_until_parked();
        host.read_with(window, |host, _| assert!(host.terminal_ui.open));
        window.simulate_keystrokes("ctrl-`");
        host.read_with(window, |host, _| assert!(host.terminal_ui.open));

        host.update(window, |host, cx| {
            host.page = crate::Page::Settings;
            host.provider_settings.section = provider_ui::Section::Shortcuts;
            cx.notify();
        });
        window.run_until_parked();
        let record = window.debug_bounds("record-terminal-shortcut").unwrap();
        window.simulate_click(record.center(), Default::default());
        window.simulate_keystrokes("ctrl-k");
        host.read_with(window, |host, cx| {
            assert_eq!(
                host.provider_settings.shortcut_recording,
                Some(ProjectAction::Terminal)
            );
            assert!(host.provider_settings.shortcut_message.is_some());
            assert!(!host.commands.palette.read(cx).is_open());
        });
        window.simulate_keystrokes("ctrl-shift-j");
        window.run_until_parked();
        host.update(window, |host, cx| {
            assert_eq!(host.history.terminal_shortcut, "ctrl-shift-j");
            assert!(host.provider_settings.shortcut_recording.is_none());
            host.history.save(&path).unwrap();
            assert_eq!(
                model::History::load(&path).unwrap().terminal_shortcut,
                "ctrl-shift-j"
            );
            host.page = crate::Page::Chat;
            host.terminal_ui.open = false;
            cx.notify();
        });
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                window.focus(&host.composer.focus_handle(cx), cx)
            })
        });
        window.simulate_keystrokes("ctrl-`");
        host.read_with(window, |host, _| assert!(!host.terminal_ui.open));
        window.simulate_keystrokes("ctrl-shift-j");
        host.read_with(window, |host, _| assert!(host.terminal_ui.open));

        host.update(window, |host, cx| {
            host.page = crate::Page::Settings;
            cx.notify();
        });
        window.run_until_parked();
        let record = window.debug_bounds("record-terminal-shortcut").unwrap();
        window.simulate_click(record.center(), Default::default());
        window.simulate_keystrokes("escape");
        host.read_with(window, |host, _| {
            assert!(host.provider_settings.shortcut_recording.is_none());
            assert_eq!(host.history.terminal_shortcut, "ctrl-shift-j");
        });
        let reset = window.debug_bounds("reset-terminal-shortcut").unwrap();
        window.simulate_click(reset.center(), Default::default());
        host.read_with(window, |host, _| {
            assert_eq!(host.history.terminal_shortcut, default_terminal_shortcut())
        });
    }
}
