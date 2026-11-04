use crate::{
    Workbench,
    plugins::{self, AGENT_TOOL, AgentTool},
};
use gpui::Context;
use gpui_glint::plugins::{Approval, Contribution, Plugin};
use serde_json::{Value, json};

impl Workbench {
    fn approved_agent_tools(&self) -> impl Iterator<Item = (&Plugin, &Contribution, String)> {
        self.plugin_catalog
            .iter()
            .filter(|plugin| self.plugin_enabled(plugin))
            .filter(|plugin| {
                plugin
                    .manifest
                    .contributions
                    .iter()
                    .any(|c| c.kind == AGENT_TOOL)
            })
            .flat_map(move |plugin| {
                let fingerprint = Approval::for_plugin(plugin).fingerprint;
                plugin
                    .manifest
                    .contributions
                    .iter()
                    .filter_map(move |contribution| {
                        (contribution.kind == AGENT_TOOL
                            && self.approved_agent_plugins.contains(&fingerprint))
                        .then(|| {
                            (
                                plugin,
                                contribution,
                                AgentTool::name(&contribution.id, &fingerprint),
                            )
                        })
                    })
            })
    }

    pub fn agent_plugin_tool_catalog(&self) -> Vec<Value> {
        self.approved_agent_tools()
            .filter_map(|(_, contribution, name)| {
                let tool = AgentTool::parse(contribution).ok()?;
                Some(json!({
                    "name": name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                    "annotations": {"readOnlyHint": true}
                }))
            })
            .collect()
    }

    /// Resolve against current permissions on the UI thread, then run the owned job off-thread.
    pub fn prepare_agent_plugin_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> Result<impl FnOnce() -> Result<Value, String> + Send + use<>, String> {
        let (plugin, contribution, _) = self
            .approved_agent_tools()
            .find(|(_, _, tool)| tool == name)
            .ok_or("Agent 插件工具已停用、撤销授权或更换版本")?;
        if !arguments.is_object() {
            return Err("Agent 插件工具参数必须为 JSON 对象".into());
        }
        let job = plugins::Job {
            plugin: plugin.clone(),
            action_id: contribution.id.clone(),
            request: arguments,
            variables: Default::default(),
        };
        Ok(move || plugins::isolated(&job))
    }

    pub(crate) fn set_agent_tool_approval(
        &mut self,
        fingerprint: String,
        approved: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.workspace
            .storage
            .set_agent_plugin_approved(&fingerprint, approved)?;
        if approved {
            self.approved_agent_plugins.insert(fingerprint);
        } else {
            self.approved_agent_plugins.remove(&fingerprint);
        }
        cx.notify();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;

    #[gpui::test]
    fn agent_tool_approval_is_pinned_and_revocable(cx: &mut gpui::TestAppContext) {
        let data = tempfile::tempdir().unwrap();
        let plugin = Plugin::from_source(
            r#"{
                "id": "org.test.agent", "name": "Example", "version": "1.0.0",
                "api_version": 1, "entry": "main.js", "hooks": ["agent.echo"],
                "contributions": [{
                    "kind": "agent.tool", "id": "echo", "title": "Echo", "hook": "agent.echo",
                    "data": {"description": "Echo the argument", "inputSchema": {"type": "object"}}
                }]
            }"#,
            "glint.on('agent.echo', args => ({ echo: args.text }));",
        )
        .unwrap();
        let fingerprint = Approval::for_plugin(&plugin).fingerprint;
        let host = cx.new(|cx| {
            let store =
                crate::storage::Store::open(&data.path().join("workspace.sqlite3")).unwrap();
            Workbench::with_store(store, cx)
        });
        host.update(cx, |host, cx| {
            host.plugin_catalog = vec![plugin.clone()];
            assert!(host.agent_plugin_tool_catalog().is_empty());
            host.set_agent_tool_approval(fingerprint.clone(), true, cx)
                .unwrap();

            let tools = host.agent_plugin_tool_catalog();
            let name = tools[0]["name"].as_str().unwrap();
            assert!(
                host.prepare_agent_plugin_tool(name, json!({"text": "hello"}))
                    .is_ok()
            );
            assert!(
                host.prepare_agent_plugin_tool(name, json!(["hello"]))
                    .is_err()
            );
            assert_eq!(
                plugins::run_job(plugins::Job {
                    plugin: plugin.clone(),
                    action_id: "echo".into(),
                    request: json!({"text": "hello"}),
                    variables: Default::default(),
                })
                .unwrap(),
                json!({"echo": "hello"})
            );

            host.plugin_catalog[0].source.push_str("\n// changed");
            assert!(host.agent_plugin_tool_catalog().is_empty());
            assert!(host.prepare_agent_plugin_tool(name, json!({})).is_err());
            host.plugin_catalog[0] = plugin;
            host.set_agent_tool_approval(fingerprint, false, cx)
                .unwrap();
            assert!(host.agent_plugin_tool_catalog().is_empty());
        });
    }
}
