//! Native file pickers for portable companion archives.
use super::*;
use crate::partner_transfer::Archive;

impl AgentWorkspace {
    fn partner_draft_dirty(&self, cx: &Context<Self>) -> bool {
        let Some(id) = self.partners_ui.selected else {
            return false;
        };
        self.partner_repo()
            .list()
            .ok()
            .and_then(|ps| ps.into_iter().find(|p| p.id == id))
            .is_some_and(|p| {
                self.partners_ui.name.read(cx).value() != p.name
                    || self.partners_ui.avatar.read(cx).value() != p.avatar
                    || self.partners_ui.description.read(cx).value() != p.description
            })
    }
    pub(super) fn import_partner_file(&mut self, cx: &mut Context<Self>) {
        if self.partners_ui.transfer_task.is_some() {
            return;
        }
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择伙伴迁移文件（.glint-partner.json）".into()),
        });
        let repo = self.partner_repo();
        self.partners_ui.transfer_task = Some(cx.spawn(async move |weak, cx| {
            let result = async {
                let paths = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
                    return Ok::<_, String>(None);
                };
                cx.background_executor()
                    .spawn(async move { Archive::read(&path)?.import(&repo) })
                    .await
                    .map(Some)
            }
            .await;
            let _ = weak.update(cx, |this, cx| {
                this.partners_ui.transfer_task = None;
                match result {
                    Ok(Some(partner)) => {
                        if !this.partner_draft_dirty(cx) {
                            this.select_partner(&partner, cx);
                        }
                        this.partners_ui
                            .partner_search
                            .update(cx, |search, cx| search.set_value("", cx));
                        this.partners_ui.notice = Some(format!(
                            "已导入新伙伴「{}」。唤醒计划已暂停，请检查项目与模型后再启用。",
                            partner.name
                        ));
                        this.sync_skill_context(cx);
                        this.refresh_calendar(cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.partners_ui.notice = Some(format!("导入失败：{error}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(super) fn export_partner_file(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.partners_ui.transfer_task.is_some() {
            return;
        }
        if self.partners_ui.selected == Some(id) && self.partner_draft_dirty(cx) {
            self.partners_ui.notice = Some("请先保存当前伙伴设定，再导出。".into());
            cx.notify();
            return;
        }
        let name = self
            .partner_repo()
            .list()
            .ok()
            .and_then(|ps| ps.into_iter().find(|p| p.id == id))
            .map(|p| p.name)
            .unwrap_or_else(|| "伙伴".into());
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_control() || "<>:\"/\\|?*".contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let filename = format!("{}.glint-partner.json", safe.trim().trim_end_matches('.'));
        let directory = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_default();
        let picker = cx.prompt_for_new_path(&directory, Some(&filename));
        let repo = self.partner_repo();
        self.partners_ui.transfer_task = Some(cx.spawn(async move |weak, cx| {
            let result = async {
                let path = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                let Some(path) = path else {
                    return Ok::<_, String>(None);
                };
                let output = path.clone();
                cx.background_executor()
                    .spawn(async move { Archive::export(&repo, id)?.write(&output) })
                    .await?;
                Ok(Some(path))
            }
            .await;
            let _ = weak.update(cx, |this, cx| {
                this.partners_ui.transfer_task = None;
                match result {
                    Ok(Some(path)) => {
                        this.partners_ui.notice =
                            Some(format!("伙伴与头像已导出至 {}", path.display()))
                    }
                    Ok(None) => {}
                    Err(error) => this.partners_ui.notice = Some(format!("导出失败：{error}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
