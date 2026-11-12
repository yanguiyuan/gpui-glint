//! Workspace-local shortcuts take effect immediately without app-global rebinding.
use super::*;

pub fn default_terminal_shortcut() -> String {
    "ctrl-`".into()
}

fn terminal_keystroke(value: &str) -> gpui::Keystroke {
    gpui::Keystroke::parse(value)
        .ok()
        .filter(|key| validate_terminal_shortcut(key).is_ok())
        .unwrap_or_else(|| gpui::Keystroke::parse(&default_terminal_shortcut()).unwrap())
}

pub fn terminal_shortcut_label(value: &str) -> String {
    let key = terminal_keystroke(value);
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
    pub(super) fn project_terminal_shortcut(
        &mut self,
        event: &gpui::KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let recording = self.provider_settings.terminal_shortcut_recording
            && self.page == crate::Page::Settings
            && self.provider_settings.section == provider_ui::Section::Shortcuts;
        if self.provider_settings.terminal_shortcut_recording && !recording {
            self.provider_settings.terminal_shortcut_recording = false;
        }
        if recording {
            window.prevent_default();
            cx.stop_propagation();
            let key = &event.keystroke;
            if key.key == "escape" {
                self.provider_settings.terminal_shortcut_recording = false;
                self.provider_settings.terminal_shortcut_message = None;
            } else if !matches!(
                key.key.as_str(),
                "ctrl" | "control" | "alt" | "shift" | "cmd" | "super" | "win" | "fn"
            ) {
                match validate_terminal_shortcut(key) {
                    Ok(()) => {
                        self.history.terminal_shortcut = key.unparse();
                        self.provider_settings.terminal_shortcut_recording = false;
                        self.provider_settings.terminal_shortcut_message = None;
                        self.save(cx);
                    }
                    Err(message) => {
                        self.provider_settings.terminal_shortcut_message = Some(message.into())
                    }
                }
            }
            cx.notify();
            return;
        }
        let binding = terminal_keystroke(&self.history.terminal_shortcut);
        if event.keystroke.is_ime_in_progress()
            || event.keystroke.modifiers != binding.modifiers
            || event.keystroke.key != binding.key
            || self.history.active().project.is_none()
        {
            return;
        }
        window.prevent_default();
        cx.stop_propagation();
        self.commands
            .palette
            .update(cx, |palette, cx| palette.close(window, cx));
        self.open_project_terminal(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_shortcuts_keep_old_settings_compatible_and_reject_conflicts() {
        let mut value = serde_json::to_value(model::History::default()).unwrap();
        value.as_object_mut().unwrap().remove("terminal_shortcut");
        let history: model::History = serde_json::from_value(value).unwrap();
        assert_eq!(history.terminal_shortcut, default_terminal_shortcut());
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
            assert!(host.provider_settings.terminal_shortcut_recording);
            assert!(host.provider_settings.terminal_shortcut_message.is_some());
            assert!(!host.commands.palette.read(cx).is_open());
        });
        window.simulate_keystrokes("ctrl-shift-j");
        window.run_until_parked();
        host.update(window, |host, cx| {
            assert_eq!(host.history.terminal_shortcut, "ctrl-shift-j");
            assert!(!host.provider_settings.terminal_shortcut_recording);
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
            assert!(!host.provider_settings.terminal_shortcut_recording);
            assert_eq!(host.history.terminal_shortcut, "ctrl-shift-j");
        });
        let reset = window.debug_bounds("reset-terminal-shortcut").unwrap();
        window.simulate_click(reset.center(), Default::default());
        host.read_with(window, |host, _| {
            assert_eq!(host.history.terminal_shortcut, default_terminal_shortcut())
        });
    }
}
