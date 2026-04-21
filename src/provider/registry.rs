use crate::provider::Provider;

/// Registry of available AI coding agent providers.
pub struct ProviderRegistry {
    providers: Vec<Box<dyn Provider>>,
    default_index: usize,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            default_index: 0,
        }
    }

    /// Register a provider. The first registered becomes the default.
    pub fn register(&mut self, provider: Box<dyn Provider>) {
        self.providers.push(provider);
    }

    /// Set the default provider by ID. Returns false if not found.
    pub fn set_default(&mut self, id: &str) -> bool {
        if let Some(idx) = self.providers.iter().position(|p| p.id() == id) {
            self.default_index = idx;
            true
        } else {
            false
        }
    }

    /// Look up a provider by its short ID.
    pub fn get_by_id(&self, id: &str) -> Option<&dyn Provider> {
        self.providers.iter().find(|p| p.id() == id).map(|p| &**p)
    }

    /// The default provider.
    #[allow(dead_code)]
    pub fn default_provider(&self) -> Option<&dyn Provider> {
        self.providers.get(self.default_index).map(|p| &**p)
    }

    /// Default provider index (for dialog pre-selection).
    pub fn default_index(&self) -> usize {
        self.default_index
    }

    /// All registered provider display names, in registration order.
    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.name()).collect()
    }

    /// All registered provider IDs, in registration order.
    pub fn provider_ids(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.id()).collect()
    }

    /// Number of registered providers.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::claude_code::ClaudeCodeProvider;

    #[test]
    fn register_and_lookup() {
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(ClaudeCodeProvider));

        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
        assert_eq!(registry.provider_names(), vec!["Claude Code"]);
        assert_eq!(registry.provider_ids(), vec!["claude-code"]);

        let provider = registry.get_by_id("claude-code").unwrap();
        assert_eq!(provider.name(), "Claude Code");
    }

    #[test]
    fn default_is_first_registered() {
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(ClaudeCodeProvider));

        let default = registry.default_provider().unwrap();
        assert_eq!(default.id(), "claude-code");
        assert_eq!(registry.default_index(), 0);
    }

    #[test]
    fn set_default_by_id() {
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(ClaudeCodeProvider));

        assert!(registry.set_default("claude-code"));
        assert!(!registry.set_default("nonexistent"));
    }

    #[test]
    fn empty_registry() {
        let registry = ProviderRegistry::new();
        assert!(registry.is_empty());
        assert!(registry.default_provider().is_none());
        assert!(registry.get_by_id("claude-code").is_none());
    }
}
