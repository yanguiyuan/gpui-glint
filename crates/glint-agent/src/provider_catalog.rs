use crate::{
    discovery::DiscoveredProvider,
    provider::{Config, Kind},
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub struct ProviderCatalog {
    pub providers: Vec<Config>,
    pub active: Option<Uuid>,
    pub active_model: String,
}

fn discovered_config(found: &DiscoveredProvider) -> Config {
    let mut config = found.config.clone();
    config.discovery_id = Some(found.id.to_owned());
    config
}

impl ProviderCatalog {
    pub fn load(path: &Path, discovered: &[DiscoveredProvider]) -> Result<Self, String> {
        let stored = crate::database::load_catalog(path)?;
        let importing = stored.is_none();
        let mut catalog = if let Some(value) = stored {
            serde_json::from_value(value).map_err(|error| format!("供应商数据库损坏：{error}"))?
        } else if path.exists() {
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).map_err(|_| "无法读取供应商列表")?)
                    .map_err(|_| "供应商列表损坏，请检查 providers.json")?;
            if let Some(providers) = value["providers"].as_array_mut() {
                for provider in providers {
                    Config::migrate(provider);
                }
            }
            serde_json::from_value(value).map_err(|_| "供应商列表损坏，请检查 providers.json")?
        } else if path.with_file_name("provider.json").exists() {
            let provider = Config::load(&path.with_file_name("provider.json"))?;
            Self {
                active: provider.enabled.then_some(provider.id),
                active_model: provider.model.clone(),
                providers: vec![provider],
            }
        } else {
            let providers: Vec<_> = discovered.iter().map(discovered_config).collect();
            let active = providers.first().map(|provider| provider.id);
            let active_model = providers
                .first()
                .map(|provider| provider.model.clone())
                .unwrap_or_default();
            Self {
                providers,
                active,
                active_model,
            }
        };
        for found in discovered {
            if !catalog.providers.iter().any(|provider| {
                provider.discovery_id.as_deref() == Some(found.id)
                    || (provider.kind == found.config.kind
                        && provider.base_url == found.config.base_url)
            }) {
                catalog.providers.push(discovered_config(found));
            }
        }
        for provider in &mut catalog.providers {
            normalize_models(provider);
        }
        if catalog
            .active
            .is_some_and(|id| !catalog.providers.iter().any(|provider| provider.id == id))
        {
            catalog.active = None;
        }
        if let Some(active) = catalog.active {
            let provider = catalog
                .providers
                .iter()
                .find(|provider| provider.id == active)
                .unwrap();
            if !provider.models.contains(&catalog.active_model) {
                catalog.active_model = provider.model.clone();
            }
        }
        if importing {
            catalog.save(path)?;
        }
        Ok(catalog)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        crate::database::save_catalog(path, self)
    }

    pub fn current(&self) -> Config {
        if let Some(mut provider) = self
            .providers
            .iter()
            .find(|provider| Some(provider.id) == self.active)
            .cloned()
        {
            provider.model = self.active_model.clone();
            provider
        } else {
            Config::default()
        }
    }

    pub fn upsert(&mut self, mut config: Config) {
        normalize_models(&mut config);
        if let Some(existing) = self
            .providers
            .iter_mut()
            .find(|provider| provider.id == config.id)
        {
            *existing = config;
        } else {
            self.providers.push(config);
        }
    }

    pub fn activate(&mut self, id: Uuid, model: String) -> Result<(), String> {
        let provider = self
            .providers
            .iter()
            .find(|provider| provider.id == id)
            .ok_or("供应商不存在")?;
        if !provider.enabled {
            return Err("请先保存供应商配置".into());
        }
        if !provider.models.contains(&model)
            && !(provider.kind == Kind::ChatGpt && model.is_empty())
        {
            return Err("请先在供应商中添加该模型".into());
        }
        let mut selected = provider.clone();
        selected.model = model.clone();
        selected.validate()?;
        self.active = Some(id);
        self.active_model = model;
        Ok(())
    }
}

pub fn normalize_models(provider: &mut Config) {
    let mut unique = Vec::new();
    for model in &provider.models {
        let model = model.trim();
        if !model.is_empty() && !unique.iter().any(|item| item == model) {
            unique.push(model.to_owned());
        }
    }
    if !provider.model.trim().is_empty() && !unique.contains(&provider.model) {
        unique.insert(0, provider.model.clone());
    }
    if provider.model.is_empty() && !unique.is_empty() {
        provider.model = unique[0].clone();
    }
    provider.models = unique;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::DiscoveryRegistry;

    #[test]
    fn several_providers_and_models_survive_restart_with_selection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("providers.json");
        let mut catalog = ProviderCatalog::load(&path, &[]).unwrap();
        let a = Config {
            enabled: true,
            name: "A".into(),
            model: "small".into(),
            models: vec!["small".into(), "large".into()],
            ..Config::default()
        };
        let b = Config {
            enabled: true,
            name: "B".into(),
            model: "other".into(),
            models: vec!["other".into()],
            ..Config::default()
        };
        let a_id = a.id;
        catalog.upsert(a);
        catalog.upsert(b);
        catalog.activate(a_id, "large".into()).unwrap();
        catalog.save(&path).unwrap();
        let loaded = ProviderCatalog::load(&path, &[]).unwrap();
        assert_eq!(loaded.providers.len(), 2);
        assert_eq!(loaded.current().model, "large");
        assert_eq!(loaded.providers[0].model, "small");
        assert!(loaded.clone().activate(a_id, "unknown".into()).is_err());
    }

    #[test]
    fn legacy_config_migrates_and_detection_does_not_duplicate_or_reactivate() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("providers.json");
        let found = DiscoveryRegistry::default().discover_with(&|_: &str| true);
        let mut legacy = found[0].config.clone();
        legacy.enabled = false;
        legacy.save(&path.with_file_name("provider.json")).unwrap();
        let catalog = ProviderCatalog::load(&path, &found).unwrap();
        assert_eq!(catalog.providers.len(), 1);
        assert!(catalog.active.is_none());
        assert_eq!(catalog.providers[0].models, [legacy.model]);
    }
}
