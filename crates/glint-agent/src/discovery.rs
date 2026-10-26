//! Extensible provider discovery. Detectors see key presence, never secret values.
use crate::provider::Config;

pub trait DiscoveryEnvironment {
    fn has_nonempty(&self, name: &str) -> bool;
}

impl<F: Fn(&str) -> bool> DiscoveryEnvironment for F {
    fn has_nonempty(&self, name: &str) -> bool {
        self(name)
    }
}

pub trait ProviderDiscovery: Send + Sync {
    /// Stable supplier ID used for registration and selection.
    fn id(&self) -> &'static str;
    /// Return non-secret connection configuration when credentials are discoverable.
    fn discover(&self, environment: &dyn DiscoveryEnvironment) -> Option<Config>;
}

#[derive(Clone)]
pub struct DiscoveredProvider {
    pub id: &'static str,
    pub config: Config,
}

pub struct DiscoveryRegistry {
    providers: Vec<Box<dyn ProviderDiscovery>>,
}

impl DiscoveryRegistry {
    pub fn new() -> Self {
        Self { providers: vec![] }
    }

    /// Registration order determines the first-run default. Re-registering an ID
    /// replaces its detector without changing that priority or creating duplicates.
    pub fn register(&mut self, provider: impl ProviderDiscovery + 'static) {
        if let Some(index) = self
            .providers
            .iter()
            .position(|item| item.id() == provider.id())
        {
            self.providers[index] = Box::new(provider);
        } else {
            self.providers.push(Box::new(provider));
        }
    }

    pub fn discover(&self) -> Vec<DiscoveredProvider> {
        self.discover_with(&|name: &str| {
            std::env::var(name).is_ok_and(|value| !value.trim().is_empty())
        })
    }

    pub fn discover_with(&self, environment: &dyn DiscoveryEnvironment) -> Vec<DiscoveredProvider> {
        self.providers
            .iter()
            .filter_map(|provider| {
                provider
                    .discover(environment)
                    .map(|config| DiscoveredProvider {
                        id: provider.id(),
                        config,
                    })
            })
            .collect()
    }
}

impl Default for DiscoveryRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register(DeepSeekDiscovery);
        registry
    }
}

pub struct DeepSeekDiscovery;

impl ProviderDiscovery for DeepSeekDiscovery {
    fn id(&self) -> &'static str {
        "deepseek"
    }

    fn discover(&self, environment: &dyn DiscoveryEnvironment) -> Option<Config> {
        let name = ["DEEPSEEK_API_KEY", "deepseek_api_key"]
            .into_iter()
            .find(|name| environment.has_nonempty(name))?;
        Some(Config {
            enabled: true,
            name: "DeepSeek".into(),
            base_url: "https://api.deepseek.com/anthropic".into(),
            protocol: crate::provider::Protocol::Anthropic,
            models_url: Some("https://api.deepseek.com/models".into()),
            model: "deepseek-flash".into(),
            api_key_env: Some(name.into()),
            ..Config::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_supports_both_environment_spellings_and_only_stores_the_name() {
        let registry = DiscoveryRegistry::default();
        assert!(registry.discover_with(&|_: &str| false).is_empty());
        let upper = registry.discover_with(&|_: &str| true);
        assert_eq!(upper[0].id, "deepseek");
        assert!(upper[0].config.protocol == crate::provider::Protocol::Anthropic);
        assert_eq!(
            upper[0].config.api_key_env.as_deref(),
            Some("DEEPSEEK_API_KEY")
        );
        let lower = registry.discover_with(&|name: &str| name == "deepseek_api_key");
        assert_eq!(
            lower[0].config.api_key_env.as_deref(),
            Some("deepseek_api_key")
        );
        assert!(
            serde_json::to_value(&lower[0].config)
                .unwrap()
                .get("api_key")
                .is_none()
        );
    }

    struct OtherDiscovery;
    impl ProviderDiscovery for OtherDiscovery {
        fn id(&self) -> &'static str {
            "other"
        }
        fn discover(&self, environment: &dyn DiscoveryEnvironment) -> Option<Config> {
            environment.has_nonempty("OTHER_KEY").then(|| Config {
                name: "Other".into(),
                api_key_env: Some("OTHER_KEY".into()),
                ..Config::default()
            })
        }
    }

    #[test]
    fn new_suppliers_register_without_changing_existing_detectors_or_order() {
        let mut registry = DiscoveryRegistry::default();
        registry.register(OtherDiscovery);
        registry.register(DeepSeekDiscovery);
        let found = registry.discover_with(&|_: &str| true);
        assert_eq!(
            found.iter().map(|provider| provider.id).collect::<Vec<_>>(),
            ["deepseek", "other"]
        );
        assert_eq!(found[1].config.api_key_env.as_deref(), Some("OTHER_KEY"));
        let only_other = registry.discover_with(&|name: &str| name == "OTHER_KEY");
        assert_eq!(only_other.len(), 1);
        assert_eq!(only_other[0].id, "other");
    }
}
