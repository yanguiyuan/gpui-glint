//! Command palette entries and command synchronization.

use super::*;

impl Workbench {
    pub(super) fn command_items(requests: &[Request]) -> Vec<CommandItem> {
        requests
            .iter()
            .enumerate()
            .map(|(i, r)| {
                CommandItem::new(i.to_string(), r.name.clone())
                    .group("请求")
                    .keywords(format!("{} {}", r.draft.method, r.draft.url))
                    .icon(Icon::ApiDoc)
            })
            .chain([
                CommandItem::new("import-curl", "从 cURL 导入请求")
                    .group("操作")
                    .keywords("import curl 导入 粘贴")
                    .icon(Icon::Download),
                CommandItem::new("import-workspace", "导入工作区")
                    .group("操作")
                    .keywords("import workspace 导入 工作区 迁移 还原 备份 json")
                    .icon(Icon::Download),
                CommandItem::new("export-workspace", "导出当前工作区")
                    .group("操作")
                    .keywords("export workspace 导出 工作区 迁移 备份 json")
                    .icon(Icon::Upload),
                CommandItem::new("mcp-config", "复制 MCP 连接配置")
                    .group("操作")
                    .keywords("agent mcp stdio 连接 配置")
                    .icon(Icon::Copy),
                CommandItem::new("send", "发送 HTTP 请求")
                    .group("操作")
                    .icon(Icon::Play)
                    .shortcut("⌘ Enter"),
                CommandItem::new("save", "保存请求版本")
                    .group("操作")
                    .icon(Icon::Save)
                    .shortcut("⌘ S"),
                CommandItem::new("theme", "切换明暗主题")
                    .group("操作")
                    .icon(Icon::HalfCircle),
            ])
            .collect()
    }

    pub(super) fn sync_commands(&mut self, cx: &mut Context<Self>) {
        let items = Self::command_items(&self.workspace.requests)
            .into_iter()
            .filter(|item| {
                self.request_binding.is_none()
                    || !matches!(item.id.as_ref(), "import-workspace" | "export-workspace")
            })
            .collect::<Vec<_>>();
        self.commands
            .update(cx, |commands, cx| commands.set_items(items, cx));
    }

    pub(super) fn open_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_commands(cx);
        self.commands
            .update(cx, |commands, cx| commands.open(window, cx));
    }
}
