//! Public embedding surface for the unified desktop application.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Requests,
    Environments,
    Proxies,
    Plugins,
    History,
    Mcp,
    Vault,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn host_tools_modify_live_requests_validate_arguments_and_audit_calls(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let host = cx.new(|cx| {
            Workbench::with_store(
                storage::Store::open(&directory.path().join("api.sqlite3")).unwrap(),
                cx,
            )
        });
        host.update(cx, |host, cx| {
            let mut call = |name: &str, arguments: serde_json::Value| {
                host.execute_host_tool(name.into(), name.into(), arguments, Instant::now() + std::time::Duration::from_secs(30), cx)
            };
            call("glint_collection_create", serde_json::json!({"name":"Agent API"})).unwrap();
            let created = call("glint_request_create", serde_json::json!({"collection":"Agent API","name":"From Agent","draft":{"method":"GET","url":"https://example.test/"}})).unwrap();
            let id = created["request_id"].as_str().unwrap().to_owned();
            call("glint_request_update", serde_json::json!({"request_id":id,"name":"Updated by Agent","draft":{"url":"https://example.test/new"}})).unwrap();
            let request = call("glint_request_get", serde_json::json!({"request_id":id})).unwrap();
            assert!(request.to_string().contains("https://example.test/new"));
            assert!(call("glint_collection_create", serde_json::json!({"name":42})).is_err());
            assert!(call("glint_missing_tool", serde_json::json!({})).is_err());
            assert_eq!(host.workspace.requests[0].id, id);
            assert_eq!(host.workspace.requests[0].name, "Updated by Agent");
            assert!(host.execute_host_tool("expired".into(), "glint_collection_create".into(), serde_json::json!({"name":"Must not exist"}), Instant::now() - std::time::Duration::from_secs(1), cx).is_err());
            assert!(!host.workspace.collections.iter().any(|c| c.collection == "Must not exist"));
            let history = host.workspace.storage.mcp_call_page("Glint Workbench Agent", None, None, 100).unwrap();
            assert_eq!(history.total, 7);
            assert_eq!(history.rows.iter().filter(|row| !row.success).count(), 3);
            host.persist(cx);
        });
        let saved = storage::Store::open(&directory.path().join("api.sqlite3"))
            .unwrap()
            .load()
            .unwrap();
        assert_eq!(saved.requests[0].name, "Updated by Agent");
    }

    #[gpui::test]
    fn hosted_navigation_preserves_request_and_hides_unrelated_topbar_controls(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join("api.sqlite3")).unwrap(),
                cx,
            );
            host.host_shell = Some(std::rc::Rc::new(|slots| {
                gpui_glint::components::AppLayout::from_slots(slots)
                    .navigation_rail(div().debug_selector(|| "host-navigation".into()))
                    .into_any_element()
            }));
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("Test", cx));
            host.save_group(cx);
            host.add_request(cx);
            host
        });
        let endpoint = host.read_with(window, |host, _| host.endpoint.entity_id());
        for page in [
            Page::Environments,
            Page::Proxies,
            Page::Plugins,
            Page::Mcp,
            Page::Vault,
            Page::History,
            Page::Requests,
        ] {
            window.update(|window, cx| host.update(cx, |host, cx| host.navigate(page, window, cx)));
            window.run_until_parked();
            assert!(window.debug_bounds("host-navigation").is_some());
            host.read_with(window, |host, _| {
                assert_eq!(host.page(), page);
                assert_eq!(host.endpoint.entity_id(), endpoint);
                assert_eq!(host.workspace.requests.len(), 1);
            });
            assert_eq!(
                window.debug_bounds("global-environment-selector").is_some(),
                matches!(page, Page::Requests | Page::History)
            );
        }
        let vault_id = host.read_with(window, |host, _| host.vault.entity_id());
        let shell = host.read_with(window, |host, _| host.host_shell.clone().unwrap());
        let workspace_id = host.update(window, |host, _| {
            host.workspace.storage.create_workspace("Second").unwrap()
        });
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.switch_workspace(&workspace_id, window, cx).unwrap()
            })
        });
        window.run_until_parked();
        assert!(window.debug_bounds("host-navigation").is_some());
        host.read_with(window, |host, _| {
            assert_eq!(host.vault.entity_id(), vault_id);
            assert!(std::rc::Rc::ptr_eq(
                host.host_shell.as_ref().unwrap(),
                &shell
            ))
        });
        host.update(window, |host, cx| {
            for (section, expected) in [
                ("proxies", Page::Proxies),
                ("mcp", Page::Mcp),
                ("plugins", Page::Plugins),
                ("environments", Page::Environments),
                ("requests", Page::Requests),
            ] {
                host.mcp_call("glint_ui_set", serde_json::json!({"section":section}), cx)
                    .unwrap();
                assert_eq!(host.page(), expected);
            }
        });
    }
}

impl Workbench {
    /// The same schema and hints exposed to external MCP clients.
    pub fn tool_catalog() -> Vec<serde_json::Value> {
        mcp_tools::catalog()
    }

    /// Must be called on the GPUI thread. Reuses MCP validation, dispatch and redacted audit history.
    pub fn execute_host_tool(
        &mut self,
        id: String,
        name: String,
        arguments: serde_json::Value,
        deadline: Instant,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        self.execute_workspace_tool(
            crate::workspace_service::Invocation {
                metadata: mcp::CallMetadata {
                    client: "Glint Workbench Agent".into(),
                    rpc_id: mcp_history::clip(&id, 256),
                    started_at_ms: chrono::Utc::now().timestamp_millis(),
                    started: Instant::now(),
                },
                name,
                arguments,
                deadline,
                rejection: None,
            },
            cx,
        )
    }
    /// Uses the existing API store and services, with navigation owned by the host.
    /// API-specific application updates are intentionally only started by `run`.
    pub fn hosted(shell: gpui_glint::components::ShellRenderer, cx: &mut Context<Self>) -> Self {
        Self::hosted_with_vault(shell, None, cx)
    }

    pub fn hosted_with_vault(
        shell: gpui_glint::components::ShellRenderer,
        vault: Option<Entity<glint_vault::VaultPage>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let path = storage::Store::default_path().expect("无法确定 SQLite 数据目录");
        let mut storage = storage::Store::open(&path).expect("无法打开 SQLite 数据库");
        let data = storage.load().expect("无法加载 SQLite 工作区");
        let mut this = Self::with_workspace_vault(storage, data, vault, cx);
        this.host_shell = Some(shell);
        this.start_mcp(cx);
        this
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.show_collection_menu = false;
        self.tree_menu = None;
        self.tab_menu = None;
        self.directory_settings = None;
        match page {
            Page::Environments => {
                self.editing_environment = self.selected_environment(cx);
                self.revealed_variables.clear();
                self.variable_delete = None;
                self.variable_error = None;
                self.variable_search
                    .update(cx, |entry, cx| entry.set_value("", cx));
            }
            Page::Plugins => self.open_plugin_manager(cx),
            Page::Mcp => self.open_mcp_history(cx),
            Page::History | Page::Requests => {
                self.activity = usize::from(page == Page::History);
                self.show_activity = page == Page::History;
                if let Some(editor) = self.shared_request_editor(self.active) {
                    editor.update(cx, |editor, cx| {
                        editor.activity = self.activity;
                        editor.show_activity = self.show_activity;
                        cx.notify();
                    });
                }
            }
            _ => {}
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Native child windows must be hidden when the API page is unmounted.
    pub fn deactivate(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.vault
            .update(cx, |vault, cx| vault.set_host_state(false, self.theme, cx));
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        {
            self.activate_html_window(window, cx);
            if let Some(view) = &self.html_webview {
                let _ = view.update(cx, |view, cx| view.set_visible(false, cx));
            }
        }
        if let Some(editor) = self.shared_request_editor(self.active) {
            editor.update(cx, |editor, cx| editor.deactivate(window, cx));
        }
        let _ = window;
    }

    pub fn set_host_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.set_theme(theme, cx);
    }

    pub fn host_theme(&self) -> ThemePreset {
        self.theme
    }

    pub fn flush(&mut self, cx: &mut Context<Self>) {
        self.persist(cx);
    }
}

impl Workbench {
    /// Explicit native UI import into Agent; never exposed as a model tool.
    pub fn wps_channel_credentials(&self, cx: &App) -> Result<(String, String), String> {
        let values = self.active_environment_values(cx);
        let draft = self.snapshot(cx);
        let binding = draft
            .plugins
            .iter()
            .find(|p| p.enabled && p.id == "com.wps.kso1")
            .or(draft.event_decoder.as_ref());
        let read = |name: &str| -> Result<String, String> {
            let custom = binding
                .and_then(|b| b.environment_bindings.get(name))
                .map(String::as_str)
                .unwrap_or(name);
            let key = if values.0.contains_key(custom) {
                custom
            } else {
                name
            };
            let value = values
                .0
                .get(key)
                .ok_or_else(|| format!("当前 API 环境缺少 {name}，请先选择 WPS 请求及其环境"))?;
            let resolved = template::TemplateResolver::new(&values).resolve(value)?;
            if resolved.trim().is_empty() {
                return Err(format!("当前 API 环境的 {name} 为空"));
            }
            Ok(resolved)
        };
        Ok((read("wps_app_id")?, read("wps_app_key")?))
    }
}

#[cfg(test)]
mod channel_import_tests {
    use super::*;
    #[gpui::test]
    fn channel_import_respects_wps_action_variable_bindings(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let host = cx.new(|cx| {
            Workbench::with_store(
                storage::Store::open(&dir.path().join("api.sqlite3")).unwrap(),
                cx,
            )
        });
        host.update(cx, |host, cx| {
            assert!(host.wps_channel_credentials(cx).is_err());
            host.workspace.environment_inputs[0]
                .save(None, "channel_ak", "fixture-ak", false)
                .unwrap();
            host.workspace.environment_inputs[0]
                .save(None, "channel_sk", "fixture-sk", true)
                .unwrap();
            host.execute_host_tool("collection".into(),"glint_collection_create".into(),serde_json::json!({"name":"WPS"}),std::time::Instant::now()+std::time::Duration::from_secs(30),cx).unwrap();
            host.execute_host_tool("request".into(),"glint_request_create".into(),serde_json::json!({"collection":"WPS","name":"WPS request","draft":{"method":"GET","url":"https://example.test/"}}),std::time::Instant::now()+std::time::Duration::from_secs(30),cx).unwrap();
            let mut action = model::BeforeAction::default();
            action.approval.id = "com.wps.kso1".into();
            action
                .environment_bindings
                .insert("wps_app_id".into(), "channel_ak".into());
            action
                .environment_bindings
                .insert("wps_app_key".into(), "channel_sk".into());
            host.workspace.requests[host.active].draft.plugins = vec![action];
            assert_eq!(
                host.wps_channel_credentials(cx).unwrap(),
                ("fixture-ak".into(), "fixture-sk".into())
            );
        });
    }
}
