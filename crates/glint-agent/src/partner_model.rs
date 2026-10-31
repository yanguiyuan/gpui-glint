//! Per-companion model routing, shared by interactive, channel and scheduled sends.
use super::*;
use gpui_glint::components::SelectOption;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub provider_id: Uuid,
    pub model: String,
    #[serde(default)]
    pub thinking: String,
}

impl AgentWorkspace {
    pub(crate) fn active_model_enabled(&self) -> bool {
        self.session_model(self.history.active, self.provider.clone())
            .is_ok_and(|c| c.enabled)
    }

    pub(crate) fn session_model(
        &self,
        session: Uuid,
        fallback: provider::Config,
    ) -> Result<provider::Config, String> {
        let selection = self
            .history
            .sessions
            .iter()
            .find(|s| s.id == session)
            .and_then(|s| s.partner_id)
            .and_then(|id| self.history.partner_models.get(&id));
        let Some(selection) = selection else {
            return Ok(fallback);
        };
        let mut config = self
            .catalog
            .providers
            .iter()
            .find(|p| p.id == selection.provider_id && p.enabled)
            .cloned()
            .ok_or("伙伴的供应商不可用，请到伙伴设定中重新选择模型")?;
        if !selection.model.is_empty() && !config.models.contains(&selection.model) {
            return Err("伙伴的模型已被移除，请到伙伴设定中重新选择".into());
        }
        config.model = selection.model.clone();
        config.validate()?;
        Ok(config)
    }
    pub(crate) fn session_thinking(&self, session: Uuid) -> provider::Thinking {
        self.history
            .sessions
            .iter()
            .find(|s| s.id == session)
            .and_then(|s| s.partner_id)
            .and_then(|id| self.history.partner_models.get(&id))
            .map(|s| provider::Thinking::from_value(&s.thinking))
            .unwrap_or(self.thinking)
    }
    pub(super) fn partner_model_options(&self) -> Vec<SelectOption> {
        let chip = model_chip(&self.catalog);
        std::iter::once(SelectOption::new("inherit", "跟随普通 Agent 模型"))
            .chain(
                chip.options
                    .into_iter()
                    .map(|o| SelectOption::new(o.value, o.label)),
            )
            .collect()
    }
}
