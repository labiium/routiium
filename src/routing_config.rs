//! Routing Configuration Module
//!
//! Provides comprehensive model routing with:
//! - Model aliasing (virtual model names → real models)
//! - Multiple match strategies (exact, prefix, regex, glob)
//! - Priority-based routing with fallback chains
//! - Request transformations (model rewriting, parameter injection)
//! - Backend pools with load balancing
//! - Runtime reloadable configuration

use anyhow::{anyhow, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Match strategy for routing rules
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MatchStrategy {
    /// Exact model name match
    Exact { model: String },
    /// Model name starts with prefix
    Prefix { prefix: String },
    /// Regex pattern match
    Regex { pattern: String },
    /// Glob pattern match (e.g., "gpt-4*", "claude-3-*-20240229")
    Glob { pattern: String },
    /// Match any model (catch-all)
    Any,
}

/// Load balancing strategy for backend pools
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalanceStrategy {
    /// Use first available backend
    #[default]
    First,
    /// Round-robin across backends
    RoundRobin,
    /// Random selection
    Random,
    /// Weighted random (requires weights in backend config)
    Weighted,
}

/// Upstream mode (Responses API or Chat Completions API)
/// Upstream mode (responses, chat, or bedrock)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpstreamMode {
    #[default]
    Responses,
    Chat,
    Bedrock,
}

/// Backend configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    /// Base URL for the backend (e.g., "https://api.openai.com/v1")
    pub base_url: String,

    /// Environment variable name containing the API key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_env: Option<String>,

    /// API mode (responses or chat)
    #[serde(default)]
    pub mode: UpstreamMode,

    /// Optional weight for weighted load balancing (default: 1)
    #[serde(default = "default_weight")]
    pub weight: u32,

    /// Optional timeout in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,

    /// Optional health check endpoint
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_check_url: Option<String>,
}

fn default_weight() -> u32 {
    1
}

/// Request transformation configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RequestTransform {
    /// Rewrite the model name to a different value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewrite_model: Option<String>,

    /// Add or override request parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_parameters: Option<HashMap<String, serde_json::Value>>,

    /// Remove specific parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remove_parameters: Option<Vec<String>>,

    /// Override temperature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_temperature: Option<f32>,

    /// Override max_tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_max_tokens: Option<u32>,
}

/// Routing rule with priority and transformations
#[derive(Debug, Serialize, Deserialize)]
pub struct RoutingRule {
    /// Unique identifier for the rule
    pub id: String,

    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Match strategy
    pub match_strategy: MatchStrategy,

    /// Backend(s) to route to (supports multiple for load balancing)
    pub backends: Vec<BackendConfig>,

    /// Load balancing strategy (default: first)
    #[serde(default)]
    pub load_balance: LoadBalanceStrategy,

    /// Priority (higher = checked first, default: 0)
    #[serde(default)]
    pub priority: i32,

    /// Request transformations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<RequestTransform>,

    /// Whether this rule is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Internal counter for round-robin (not serialized)
    #[serde(skip)]
    counter: AtomicUsize,

    /// Compiled regex pattern (not serialized)
    #[serde(skip)]
    regex_pattern: Option<Regex>,
}

fn default_true() -> bool {
    true
}

impl RoutingRule {
    /// Check if this rule matches the given model
    pub fn matches(&self, model: &str) -> bool {
        if !self.enabled {
            return false;
        }

        match &self.match_strategy {
            MatchStrategy::Exact { model: target } => model == target,
            MatchStrategy::Prefix { prefix } => model.starts_with(prefix),
            MatchStrategy::Regex { .. } => {
                if let Some(re) = &self.regex_pattern {
                    re.is_match(model)
                } else {
                    false
                }
            }
            MatchStrategy::Glob { pattern } => glob_match(pattern, model),
            MatchStrategy::Any => true,
        }
    }

    /// Select a backend based on load balancing strategy
    pub fn select_backend(&self) -> Option<&BackendConfig> {
        if self.backends.is_empty() {
            return None;
        }

        match self.load_balance {
            LoadBalanceStrategy::First => self.backends.first(),
            LoadBalanceStrategy::RoundRobin => {
                let idx = self.counter.fetch_add(1, Ordering::Relaxed) % self.backends.len();
                self.backends.get(idx)
            }
            LoadBalanceStrategy::Random => {
                let idx = rand::random::<usize>() % self.backends.len();
                self.backends.get(idx)
            }
            LoadBalanceStrategy::Weighted => {
                // Weighted random selection
                let total_weight: u32 = self.backends.iter().map(|b| b.weight).sum();
                if total_weight == 0 {
                    return self.backends.first();
                }

                let mut rand_val = (rand::random::<f64>() * total_weight as f64) as u32;
                for backend in &self.backends {
                    if rand_val < backend.weight {
                        return Some(backend);
                    }
                    rand_val -= backend.weight;
                }
                self.backends.last()
            }
        }
    }

    /// Apply transformations to the request body
    pub fn apply_transform(&self, body: &mut serde_json::Value) -> Result<()> {
        let Some(transform) = &self.transform else {
            return Ok(());
        };

        // Rewrite model name
        if let Some(new_model) = &transform.rewrite_model {
            if let Some(obj) = body.as_object_mut() {
                obj.insert(
                    "model".to_string(),
                    serde_json::Value::String(new_model.clone()),
                );
            }
        }

        // Add/override parameters
        if let Some(params) = &transform.add_parameters {
            if let Some(obj) = body.as_object_mut() {
                for (key, value) in params {
                    obj.insert(key.clone(), value.clone());
                }
            }
        }

        // Remove parameters
        if let Some(remove_keys) = &transform.remove_parameters {
            if let Some(obj) = body.as_object_mut() {
                for key in remove_keys {
                    obj.remove(key);
                }
            }
        }

        // Override temperature
        if let Some(temp) = transform.override_temperature {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("temperature".to_string(), serde_json::json!(temp));
            }
        }

        // Override max_tokens
        if let Some(max_tok) = transform.override_max_tokens {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("max_tokens".to_string(), serde_json::json!(max_tok));
            }
        }

        Ok(())
    }
}

impl Clone for RoutingRule {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            description: self.description.clone(),
            match_strategy: self.match_strategy.clone(),
            backends: self.backends.clone(),
            load_balance: self.load_balance.clone(),
            priority: self.priority,
            transform: self.transform.clone(),
            enabled: self.enabled,
            counter: AtomicUsize::new(0),
            regex_pattern: self.regex_pattern.clone(),
        }
    }
}

/// Model alias configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAlias {
    /// Virtual/dummy model name
    pub alias: String,

    /// Target real model name
    pub target: String,

    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Whether this alias is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Default fallback backend configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultBackend {
    pub base_url: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_env: Option<String>,

    #[serde(default)]
    pub mode: UpstreamMode,
}

/// Main routing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Model aliases (virtual name → real name mappings)
    #[serde(default)]
    pub aliases: Vec<ModelAlias>,

    /// Routing rules (sorted by priority)
    pub rules: Vec<RoutingRule>,

    /// Default/fallback backend
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_backend: Option<DefaultBackend>,

    /// Whether to allow unmatched models to pass through
    #[serde(default = "default_true")]
    pub allow_passthrough: bool,
}

impl RoutingConfig {
    /// Load routing config from file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: RoutingConfig = serde_json::from_str(&content)?;

        // Sort rules by priority (highest first)
        config.rules.sort_by(|a, b| b.priority.cmp(&a.priority));

        // Compile regex patterns
        for rule in &mut config.rules {
            if let MatchStrategy::Regex { pattern } = &rule.match_strategy {
                match Regex::new(pattern) {
                    Ok(re) => rule.regex_pattern = Some(re),
                    Err(e) => {
                        tracing::warn!("Invalid regex in rule {}: {} - {}", rule.id, pattern, e);
                    }
                }
            }
        }

        Ok(config)
    }

    /// Create an empty routing config
    pub fn empty() -> Self {
        Self {
            aliases: vec![],
            rules: vec![],
            default_backend: None,
            allow_passthrough: true,
        }
    }

    /// Resolve model name through aliases
    pub fn resolve_alias(&self, model: &str) -> String {
        for alias in &self.aliases {
            if alias.enabled && alias.alias == model {
                return alias.target.clone();
            }
        }
        model.to_string()
    }

    /// Find matching routing rule for a model
    pub fn find_rule(&self, model: &str) -> Option<&RoutingRule> {
        // First resolve aliases
        let resolved_model = self.resolve_alias(model);

        // Find first matching rule (already sorted by priority)
        self.rules.iter().find(|rule| rule.matches(&resolved_model))
    }

    /// Get statistics about routing configuration
    pub fn stats(&self) -> RoutingStats {
        let total_rules = self.rules.len();
        let enabled_rules = self.rules.iter().filter(|r| r.enabled).count();
        let total_aliases = self.aliases.len();
        let enabled_aliases = self.aliases.iter().filter(|a| a.enabled).count();
        let total_backends: usize = self.rules.iter().map(|r| r.backends.len()).sum();

        let mut match_strategies = HashMap::new();
        for rule in &self.rules {
            if rule.enabled {
                let strategy_name = match &rule.match_strategy {
                    MatchStrategy::Exact { .. } => "exact",
                    MatchStrategy::Prefix { .. } => "prefix",
                    MatchStrategy::Regex { .. } => "regex",
                    MatchStrategy::Glob { .. } => "glob",
                    MatchStrategy::Any => "any",
                };
                *match_strategies
                    .entry(strategy_name.to_string())
                    .or_insert(0) += 1;
            }
        }

        RoutingStats {
            total_rules,
            enabled_rules,
            total_aliases,
            enabled_aliases,
            total_backends,
            has_default_backend: self.default_backend.is_some(),
            allow_passthrough: self.allow_passthrough,
            match_strategies,
        }
    }
}

/// Routing statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingStats {
    pub total_rules: usize,
    pub enabled_rules: usize,
    pub total_aliases: usize,
    pub enabled_aliases: usize,
    pub total_backends: usize,
    pub has_default_backend: bool,
    pub allow_passthrough: bool,
    pub match_strategies: HashMap<String, usize>,
}

/// Resolved backend information
#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    pub base_url: String,
    pub key_env: Option<String>,
    pub mode: UpstreamMode,
    pub timeout_seconds: Option<u64>,
    pub rule_id: Option<String>,
}

impl RoutingConfig {
    /// Resolve routing for a model and get backend configuration
    pub fn resolve_route(&self, model: &str) -> Result<ResolvedRoute> {
        // Resolve through alias first
        let resolved_model = self.resolve_alias(model);

        // Find matching rule
        if let Some(rule) = self.find_rule(&resolved_model) {
            if let Some(backend) = rule.select_backend() {
                return Ok(ResolvedRoute {
                    base_url: backend.base_url.clone(),
                    key_env: backend.key_env.clone(),
                    mode: backend.mode,
                    timeout_seconds: backend.timeout_seconds,
                    rule_id: Some(rule.id.clone()),
                });
            }
        }

        // Fall back to default backend
        if let Some(default) = &self.default_backend {
            return Ok(ResolvedRoute {
                base_url: default.base_url.clone(),
                key_env: default.key_env.clone(),
                mode: default.mode,
                timeout_seconds: None,
                rule_id: None,
            });
        }

        // No match and no default
        if !self.allow_passthrough {
            return Err(anyhow!("No routing rule found for model: {}", model));
        }

        // Passthrough with environment defaults
        Ok(ResolvedRoute {
            base_url: std::env::var("OPENAI_API_BASE")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
            key_env: Some("OPENAI_API_KEY".to_string()),
            mode: UpstreamMode::Responses,
            timeout_seconds: None,
            rule_id: None,
        })
    }

    /// Apply transformations to a request for a specific model
    pub fn apply_transformations(
        &self,
        model: &str,
        body: &mut serde_json::Value,
    ) -> Result<String> {
        // Resolve alias
        let resolved_model = self.resolve_alias(model);

        // Update model in body if alias was resolved
        if resolved_model != model {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("model".to_string(), serde_json::json!(resolved_model));
            }
        }

        // Find and apply rule transformations
        if let Some(rule) = self.find_rule(&resolved_model) {
            rule.apply_transform(body)?;
        }

        // Return final model name from body
        Ok(body
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(&resolved_model)
            .to_string())
    }
}

/// Simple glob pattern matching (supports * wildcard)
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('*').collect();

    if pattern_parts.len() == 1 {
        // No wildcards - exact match
        return pattern == text;
    }

    let mut text_pos = 0;

    for (i, part) in pattern_parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        if i == 0 {
            // First part - must match at start
            if !text.starts_with(part) {
                return false;
            }
            text_pos = part.len();
        } else if i == pattern_parts.len() - 1 {
            // Last part - must match at end
            if !text.ends_with(part) {
                return false;
            }
            // Verify it's after our current position
            if let Some(pos) = text[text_pos..].find(part) {
                text_pos += pos + part.len();
            } else {
                return false;
            }
        } else {
            // Middle part - find anywhere after current position
            if let Some(pos) = text[text_pos..].find(part) {
                text_pos += pos + part.len();
            } else {
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn test_glob_match() {
        assert!(glob_match("gpt-4*", "gpt-4"));
        assert!(glob_match("gpt-4*", "gpt-4-turbo"));
        assert!(glob_match("*-20240229", "claude-3-opus-20240229"));
        assert!(glob_match("gpt-*-preview", "gpt-4-turbo-preview"));
        assert!(!glob_match("gpt-4*", "gpt-3.5-turbo"));
    }

    #[test]
    fn test_model_alias() {
        let config = RoutingConfig {
            aliases: vec![ModelAlias {
                alias: "my-model".to_string(),
                target: "gpt-4-turbo".to_string(),
                description: None,
                enabled: true,
            }],
            rules: vec![],
            default_backend: None,
            allow_passthrough: true,
        };

        assert_eq!(config.resolve_alias("my-model"), "gpt-4-turbo");
        assert_eq!(config.resolve_alias("other-model"), "other-model");
    }

    #[test]
    fn test_routing_match() {
        let rule = RoutingRule {
            id: "test".to_string(),
            description: None,
            match_strategy: MatchStrategy::Prefix {
                prefix: "gpt-4".to_string(),
            },
            backends: vec![],
            load_balance: LoadBalanceStrategy::First,
            priority: 0,
            transform: None,
            enabled: true,
            counter: AtomicUsize::new(0),
            regex_pattern: None,
        };

        assert!(rule.matches("gpt-4-turbo"));
        assert!(!rule.matches("gpt-3.5-turbo"));
    }

    #[test]
    fn test_resolve_route_with_priority_and_default() {
        let alias = ModelAlias {
            alias: "friendly".to_string(),
            target: "gpt-4o".to_string(),
            description: Some("Alias for GPT-4o".to_string()),
            enabled: true,
        };

        let high_rule = RoutingRule {
            id: "high".to_string(),
            description: None,
            match_strategy: MatchStrategy::Exact {
                model: "gpt-4o".to_string(),
            },
            backends: vec![BackendConfig {
                base_url: "https://primary.example.com/v1".to_string(),
                key_env: Some("PRIMARY_KEY".to_string()),
                mode: UpstreamMode::Responses,
                weight: 1,
                timeout_seconds: Some(45),
                health_check_url: None,
            }],
            load_balance: LoadBalanceStrategy::First,
            priority: 100,
            transform: None,
            enabled: true,
            counter: AtomicUsize::new(0),
            regex_pattern: None,
        };

        let low_rule = RoutingRule {
            id: "low".to_string(),
            description: None,
            match_strategy: MatchStrategy::Prefix {
                prefix: "gpt-".to_string(),
            },
            backends: vec![BackendConfig {
                base_url: "https://secondary.example.com/v1".to_string(),
                key_env: Some("SECONDARY_KEY".to_string()),
                mode: UpstreamMode::Responses,
                weight: 1,
                timeout_seconds: Some(60),
                health_check_url: None,
            }],
            load_balance: LoadBalanceStrategy::First,
            priority: 50,
            transform: None,
            enabled: true,
            counter: AtomicUsize::new(0),
            regex_pattern: None,
        };

        let config = RoutingConfig {
            aliases: vec![alias],
            rules: vec![high_rule, low_rule],
            default_backend: Some(DefaultBackend {
                base_url: "https://default.example.com/v1".to_string(),
                key_env: Some("DEFAULT_KEY".to_string()),
                mode: UpstreamMode::Responses,
            }),
            allow_passthrough: false,
        };

        // Alias should resolve to high priority rule
        let resolved = config.resolve_route("friendly").unwrap();
        assert_eq!(resolved.base_url, "https://primary.example.com/v1");
        assert_eq!(resolved.key_env.as_deref(), Some("PRIMARY_KEY"));
        assert_eq!(resolved.rule_id.as_deref(), Some("high"));

        // Unknown model should fall back to default backend
        let default_route = config.resolve_route("unknown-model").unwrap();
        assert_eq!(default_route.base_url, "https://default.example.com/v1");
        assert_eq!(default_route.key_env.as_deref(), Some("DEFAULT_KEY"));
        assert!(default_route.rule_id.is_none());
    }

    #[test]
    fn test_apply_transformations_rewrites_and_updates_body() {
        let mut add_parameters = HashMap::new();
        add_parameters.insert("top_p".to_string(), json!(0.9));

        let transform = RequestTransform {
            rewrite_model: Some("llama-3.1-8b".to_string()),
            add_parameters: Some(add_parameters),
            remove_parameters: Some(vec!["metadata".to_string()]),
            override_temperature: Some(0.75),
            override_max_tokens: Some(2048),
        };

        let rule = RoutingRule {
            id: "transform".to_string(),
            description: None,
            match_strategy: MatchStrategy::Exact {
                model: "llama-alias".to_string(),
            },
            backends: vec![BackendConfig {
                base_url: "http://localhost:11434/v1".to_string(),
                key_env: None,
                mode: UpstreamMode::Chat,
                weight: 1,
                timeout_seconds: None,
                health_check_url: None,
            }],
            load_balance: LoadBalanceStrategy::First,
            priority: 10,
            transform: Some(transform),
            enabled: true,
            counter: AtomicUsize::new(0),
            regex_pattern: None,
        };

        let config = RoutingConfig {
            aliases: vec![ModelAlias {
                alias: "teaching-helper".to_string(),
                target: "llama-alias".to_string(),
                description: None,
                enabled: true,
            }],
            rules: vec![rule],
            default_backend: None,
            allow_passthrough: true,
        };

        let mut body = json!({"model": "teaching-helper", "metadata": "remove-me"});
        let final_model = config
            .apply_transformations("teaching-helper", &mut body)
            .unwrap();

        assert_eq!(final_model, "llama-3.1-8b");
        assert_eq!(
            body.get("model").and_then(|v| v.as_str()),
            Some("llama-3.1-8b")
        );
        assert_eq!(body.get("top_p").and_then(|v| v.as_f64()), Some(0.9));
        assert_eq!(body.get("temperature").and_then(|v| v.as_f64()), Some(0.75));
        assert_eq!(body.get("max_tokens").and_then(|v| v.as_u64()), Some(2048));
        assert!(body.get("metadata").is_none());
    }

    #[test]
    fn test_round_robin_backend_selection() {
        let rule = RoutingRule {
            id: "round".to_string(),
            description: None,
            match_strategy: MatchStrategy::Any,
            backends: vec![
                BackendConfig {
                    base_url: "https://backend-a.example.com/v1".to_string(),
                    key_env: Some("A_KEY".to_string()),
                    mode: UpstreamMode::Responses,
                    weight: 1,
                    timeout_seconds: None,
                    health_check_url: None,
                },
                BackendConfig {
                    base_url: "https://backend-b.example.com/v1".to_string(),
                    key_env: Some("B_KEY".to_string()),
                    mode: UpstreamMode::Responses,
                    weight: 1,
                    timeout_seconds: None,
                    health_check_url: None,
                },
            ],
            load_balance: LoadBalanceStrategy::RoundRobin,
            priority: 0,
            transform: None,
            enabled: true,
            counter: AtomicUsize::new(0),
            regex_pattern: None,
        };

        let first = rule.select_backend().unwrap().base_url.clone();
        let second = rule.select_backend().unwrap().base_url.clone();
        let third = rule.select_backend().unwrap().base_url.clone();

        assert_eq!(first, "https://backend-a.example.com/v1");
        assert_eq!(second, "https://backend-b.example.com/v1");
        assert_eq!(third, "https://backend-a.example.com/v1");
    }
}
