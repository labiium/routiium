use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// System prompt configuration loaded from a JSON file
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SystemPromptConfig {
    /// Global system prompt applied to all requests (unless overridden)
    #[serde(default)]
    pub global: Option<String>,

    /// Model-specific system prompts that override the global prompt
    #[serde(default)]
    pub per_model: HashMap<String, String>,

    /// API-specific system prompts (e.g., "chat", "responses")
    #[serde(default)]
    pub per_api: HashMap<String, String>,

    /// Whether to prepend or append to existing system messages
    /// Options: "prepend", "append", "replace"
    #[serde(default = "default_injection_mode")]
    pub injection_mode: String,

    /// Whether system prompt injection is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_injection_mode() -> String {
    "prepend".to_string()
}

const fn default_enabled() -> bool {
    true
}

impl SystemPromptConfig {
    /// Load system prompt configuration from a JSON file
    ///
    /// # Errors
    /// Returns an error when the file cannot be read or parsed.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref()).with_context(|| {
            format!(
                "Failed to read system prompt config file: {}",
                path.as_ref().display()
            )
        })?;

        let config: Self = serde_json::from_str(&content)
            .with_context(|| "Failed to parse system prompt config JSON")?;

        Ok(config)
    }

    /// Get the appropriate system prompt for a given context
    /// Priority: `per_model` > `per_api` > global
    #[must_use]
    pub fn get_prompt(&self, model: Option<&str>, api: Option<&str>) -> Option<String> {
        if !self.enabled {
            return None;
        }

        // Check model-specific first
        if let Some(m) = model {
            if let Some(prompt) = self.per_model.get(m) {
                return Some(prompt.clone());
            }
        }

        // Check API-specific next
        if let Some(a) = api {
            if let Some(prompt) = self.per_api.get(a) {
                return Some(prompt.clone());
            }
        }

        // Fall back to global
        self.global.clone()
    }

    /// Create a default configuration with no prompts
    #[must_use]
    pub fn empty() -> Self {
        Self {
            global: None,
            per_model: HashMap::new(),
            per_api: HashMap::new(),
            injection_mode: default_injection_mode(),
            enabled: true,
        }
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "test fixtures are fixed")]
mod tests {
    use super::*;

    #[test]
    fn test_system_prompt_parsing() {
        let json = r#"
        {
            "global": "You are a helpful assistant.",
            "per_model": {
                "gpt-4": "You are an expert AI using GPT-4.",
                "claude-3": "You are Claude, an AI assistant."
            },
            "per_api": {
                "chat": "Chat API system prompt",
                "responses": "Responses API system prompt"
            },
            "injection_mode": "prepend",
            "enabled": true
        }
        "#;

        let config: SystemPromptConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.global,
            Some("You are a helpful assistant.".to_string())
        );
        assert_eq!(config.per_model.len(), 2);
        assert_eq!(config.per_api.len(), 2);
        assert_eq!(config.injection_mode, "prepend");
        assert!(config.enabled);
    }

    #[test]
    fn test_prompt_priority() {
        let config = SystemPromptConfig {
            global: Some("global".to_string()),
            per_model: {
                let mut map = HashMap::new();
                map.insert("gpt-4".to_string(), "model-specific".to_string());
                map
            },
            per_api: {
                let mut map = HashMap::new();
                map.insert("chat".to_string(), "api-specific".to_string());
                map
            },
            injection_mode: "prepend".to_string(),
            enabled: true,
        };

        // Model-specific should have highest priority
        assert_eq!(
            config.get_prompt(Some("gpt-4"), Some("chat")),
            Some("model-specific".to_string())
        );

        // API-specific should be next
        assert_eq!(
            config.get_prompt(Some("other-model"), Some("chat")),
            Some("api-specific".to_string())
        );

        // Global should be last
        assert_eq!(
            config.get_prompt(Some("other-model"), Some("other-api")),
            Some("global".to_string())
        );

        // Disabled config should return None
        let mut disabled_config = config;
        disabled_config.enabled = false;
        assert_eq!(
            disabled_config.get_prompt(Some("gpt-4"), Some("chat")),
            None
        );
    }
}
