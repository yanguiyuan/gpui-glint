use super::*;
use gpui_glint::components::{ComposerContextState, ComposerContextUsage};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextSnapshot {
    pub provider_id: Uuid,
    pub model: String,
    pub used_tokens: Option<u64>,
    pub capacity_tokens: Option<u64>,
}

pub fn state(config: &provider::Config, session: &model::Session) -> ComposerContextState {
    let sample = session
        .context
        .as_ref()
        .filter(|sample| sample.provider_id == config.id && sample.model == config.model);
    let capacity = config
        .context_window_overrides
        .get(&config.model)
        .copied()
        .filter(|capacity| *capacity > 0)
        .or_else(|| {
            sample
                .and_then(|sample| sample.capacity_tokens)
                .filter(|capacity| *capacity > 0)
        })
        .or_else(|| {
            config
                .model_context_windows
                .get(&config.model)
                .copied()
                .filter(|capacity| *capacity > 0)
        });
    if session.messages.is_empty() {
        return ComposerContextState::Initial {
            capacity_tokens: capacity,
        };
    }
    let used = sample.and_then(|sample| sample.used_tokens);
    match (used, capacity) {
        (Some(used), Some(capacity)) => {
            ComposerContextState::Known(ComposerContextUsage::new(used, capacity))
        }
        _ => ComposerContextState::Unknown {
            used_tokens: used,
            capacity_tokens: capacity,
        },
    }
}

impl AgentWorkspace {
    pub fn sync_context_usage(&self, cx: &mut Context<Self>) {
        let state = state(&self.provider, self.history.active());
        self.composer.update(cx, |composer, cx| {
            composer.set_context_state(Some(state), cx)
        });
    }

    pub fn refresh_context_models(&mut self, cx: &mut Context<Self>) {
        let config = self.provider.clone();
        if !config.enabled
            || config.kind != provider::Kind::Compatible
            || self.context_models.is_some()
        {
            return;
        }
        let job = config.key().and_then(|key| {
            provider::Job::start(provider::list_model_catalog(config.clone(), key))
        });
        let Ok(job) = job else {
            return;
        };
        self.context_models = Some(cx.spawn(async move |weak, cx| {
            let result = job.result().await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.context_models.take() {
                    task.detach();
                }
                if let Ok(models) = result {
                    this.cache_context_windows(
                        config.id,
                        &config.base_url,
                        models.models,
                        models.context_windows,
                        cx,
                    );
                }
            });
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_sessions_model_switches_overrides_and_unknown_usage_are_distinct() {
        let mut config = provider::Config {
            model: "a".into(),
            ..Default::default()
        };
        config.model_context_windows.insert("a".into(), 1000);
        config.model_context_windows.insert("b".into(), 2000);
        let mut session = model::Session::new();
        assert_eq!(
            state(&config, &session),
            ComposerContextState::Initial {
                capacity_tokens: Some(1000)
            }
        );
        session.push(Role::User, "hello".into());
        assert_eq!(
            state(&config, &session),
            ComposerContextState::Unknown {
                used_tokens: None,
                capacity_tokens: Some(1000)
            }
        );
        session.context = Some(ContextSnapshot {
            provider_id: config.id,
            model: "a".into(),
            used_tokens: Some(250),
            capacity_tokens: Some(1200),
        });
        assert_eq!(
            state(&config, &session),
            ComposerContextState::Known(ComposerContextUsage::new(250, 1200))
        );
        config.context_window_overrides.insert("a".into(), 800);
        assert_eq!(
            state(&config, &session),
            ComposerContextState::Known(ComposerContextUsage::new(250, 800))
        );
        config.model = "b".into();
        assert_eq!(
            state(&config, &session),
            ComposerContextState::Unknown {
                used_tokens: None,
                capacity_tokens: Some(2000)
            }
        );
        config.id = Uuid::new_v4();
        config.model = "a".into();
        assert_eq!(
            state(&config, &session),
            ComposerContextState::Unknown {
                used_tokens: None,
                capacity_tokens: Some(800)
            }
        );
        config.model = "unknown".into();
        assert!(matches!(
            state(&config, &session),
            ComposerContextState::Unknown {
                used_tokens: None,
                capacity_tokens: None
            }
        ));
    }
}
