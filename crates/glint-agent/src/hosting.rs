//! Public embedding surface; conversation state stays alive between page switches.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Chat,
    Calendar,
    Settings,
    Vault,
}

impl AgentWorkspace {
    /// Applies to the next request, including scheduled requests and delegated tools.
    pub fn set_host_tools(&mut self, tools: std::sync::Arc<dyn crate::host_tools::HostTools>) {
        self.host_tools = Some(tools);
    }
    pub fn hosted(shell: gpui_glint::components::ShellRenderer, cx: &mut Context<Self>) -> Self {
        Self::hosted_with_vault(shell, None, cx)
    }

    pub fn hosted_with_vault(
        shell: gpui_glint::components::ShellRenderer,
        vault: Option<Entity<glint_vault::VaultPage>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::with_path_vault(history_path(), vault, cx);
        this.host_shell = Some(shell);
        this
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.project_picker_open = false;
        match page {
            Page::Chat => window.focus(&self.composer.focus_handle(cx), cx),
            Page::Settings => self.open_settings(window, cx),
            Page::Vault => {
                window.focus(&self.focus, cx);
            }
            Page::Calendar => {
                self.show_auxiliary_sidebar = true;
                self.refresh_calendar(cx);
                window.focus(&self.focus, cx);
            }
        }
        cx.notify();
    }

    pub fn create_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_session(window, cx);
    }

    pub fn set_host_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.composer
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
        self.search
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
        self.provider_settings.set_theme(theme, cx);
        self.skills_ui.set_theme(theme, cx);
        self.mcp_ui.set_theme(theme, cx);
        self.sync_skill_context(cx);
        cx.notify();
    }

    pub fn host_theme(&self) -> ThemePreset {
        self.theme
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn hosted_pages_keep_session_draft_and_change_topbar(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        // Keep this UI test offline even when the developer has provider keys in the environment.
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let vault = cx.new(|cx| {
            glint_vault::VaultPage::new(directory.path().join("vault"), ThemePreset::Moonstone, cx)
        });
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path_vault(
                directory.path().join("sessions.json"),
                Some(vault.clone()),
                cx,
            );
            host.host_shell = Some(std::rc::Rc::new(|slots| {
                gpui_glint::components::AppLayout::from_slots(slots)
                    .navigation_rail(div().debug_selector(|| "host-navigation".into()))
                    .into_any_element()
            }));
            host
        });
        let (session, composer) = host.update(window, |host, cx| {
            host.composer
                .update(cx, |composer, cx| composer.set_value("保留这条草稿", cx));
            (host.history.active, host.composer.entity_id())
        });
        for page in [Page::Calendar, Page::Settings, Page::Chat] {
            window.update(|window, cx| host.update(cx, |host, cx| host.navigate(page, window, cx)));
            window.run_until_parked();
            assert!(window.debug_bounds("host-navigation").is_some());
            if page == Page::Calendar {
                window.update(|window, cx| assert!(host.read(cx).focus.is_focused(window)));
            }
            host.read_with(window, |host, cx| {
                assert_eq!(host.vault.entity_id(), vault.entity_id());
                assert_eq!(host.page(), page);
                assert_eq!(host.history.active, session);
                assert_eq!(host.composer.entity_id(), composer);
                assert_eq!(host.composer.read(cx).value(cx), "保留这条草稿");
            });
            assert_eq!(
                window.debug_bounds("agent-topbar-project").is_some(),
                page == Page::Chat
            );
            assert_eq!(
                window.debug_bounds("agent-topbar-search").is_some(),
                page != Page::Calendar
            );
            assert_eq!(
                window.debug_bounds("agent-topbar-provider").is_some(),
                page == Page::Chat
            );
        }
    }
}
