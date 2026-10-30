//! Bridges service commands to live editors and GPUI execution tasks.
use super::*;

impl Workbench {
    pub(super) fn execute_workspace_tool(
        &mut self,
        call: workspace_service::Invocation,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        let mut secrets = self.workspace.log_secrets();
        let result = if Instant::now() >= call.deadline {
            Err("MCP 操作已过期，未执行".into())
        } else if let Some(error) = &call.rejection {
            Err(error.clone())
        } else {
            self.mcp_call(&call.name, call.arguments.clone(), cx)
        };
        secrets.extend(self.workspace.log_secrets());
        match self.workspace.record_call(&call, &result, &secrets) {
            Ok(()) => {
                self.mcp_history_error = None;
                if self.page == crate::Page::Mcp {
                    self.reload_mcp_history(cx);
                }
            }
            Err(error) => {
                self.mcp_history_error = Some(format!("调用记录保存失败：{error}"));
                cx.notify();
            }
        }
        result
    }
}
