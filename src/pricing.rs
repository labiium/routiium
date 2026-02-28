use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Pricing information for a specific model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    /// Price per 1M input tokens (in USD)
    pub input_per_million: f64,
    /// Price per 1M output tokens (in USD)
    pub output_per_million: f64,
    /// Price per 1M cached tokens (optional, in USD)
    pub cached_per_million: Option<f64>,
    /// Price per 1M reasoning tokens (optional, for o1/o3, in USD)
    pub reasoning_per_million: Option<f64>,
}

impl ModelPricing {
    pub fn calculate_cost(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
    ) -> (f64, f64, Option<f64>, f64) {
        // `cached_tokens` are a subset of input tokens in OpenAI-compatible usage payloads.
        // Charge uncached input at normal input price and cached input at cached price
        // (when configured) to avoid double billing.
        let cached_token_count = cached_tokens.unwrap_or(0);
        let uncached_input_tokens = input_tokens.saturating_sub(cached_token_count);

        let input_cost = (uncached_input_tokens as f64 / 1_000_000.0) * self.input_per_million;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * self.output_per_million;

        let cached_cost = self
            .cached_per_million
            .map(|price| (cached_token_count as f64 / 1_000_000.0) * price);

        let reasoning_cost =
            if let (Some(tokens), Some(price)) = (reasoning_tokens, self.reasoning_per_million) {
                Some((tokens as f64 / 1_000_000.0) * price)
            } else {
                None
            };

        let total =
            input_cost + output_cost + cached_cost.unwrap_or(0.0) + reasoning_cost.unwrap_or(0.0);

        (input_cost, output_cost, cached_cost, total)
    }
}

/// Pricing configuration manager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingConfig {
    /// Map of model name to pricing
    pub models: HashMap<String, ModelPricing>,
    /// Default pricing for unknown models
    pub default: Option<ModelPricing>,
}

impl PricingConfig {
    /// Create with OpenAI default pricing (as of 2024)
    pub fn openai_defaults() -> Self {
        let mut models = HashMap::new();

        // GPT-4o models
        models.insert(
            "gpt-4o".to_string(),
            ModelPricing {
                input_per_million: 2.50,
                output_per_million: 10.00,
                cached_per_million: Some(1.25), // 50% discount
                reasoning_per_million: None,
            },
        );
        models.insert(
            "gpt-4o-2024-11-20".to_string(),
            ModelPricing {
                input_per_million: 2.50,
                output_per_million: 10.00,
                cached_per_million: Some(1.25),
                reasoning_per_million: None,
            },
        );

        // GPT-4o mini
        models.insert(
            "gpt-4o-mini".to_string(),
            ModelPricing {
                input_per_million: 0.150,
                output_per_million: 0.600,
                cached_per_million: Some(0.075),
                reasoning_per_million: None,
            },
        );
        models.insert(
            "gpt-4o-mini-2024-07-18".to_string(),
            ModelPricing {
                input_per_million: 0.150,
                output_per_million: 0.600,
                cached_per_million: Some(0.075),
                reasoning_per_million: None,
            },
        );

        // o1 models
        models.insert(
            "o1".to_string(),
            ModelPricing {
                input_per_million: 15.00,
                output_per_million: 60.00,
                cached_per_million: Some(7.50),
                reasoning_per_million: None, // Included in output
            },
        );
        models.insert(
            "o1-preview".to_string(),
            ModelPricing {
                input_per_million: 15.00,
                output_per_million: 60.00,
                cached_per_million: Some(7.50),
                reasoning_per_million: None,
            },
        );
        models.insert(
            "o1-mini".to_string(),
            ModelPricing {
                input_per_million: 3.00,
                output_per_million: 12.00,
                cached_per_million: Some(1.50),
                reasoning_per_million: None,
            },
        );

        // GPT-4 Turbo
        models.insert(
            "gpt-4-turbo".to_string(),
            ModelPricing {
                input_per_million: 10.00,
                output_per_million: 30.00,
                cached_per_million: None,
                reasoning_per_million: None,
            },
        );
        models.insert(
            "gpt-4-turbo-2024-04-09".to_string(),
            ModelPricing {
                input_per_million: 10.00,
                output_per_million: 30.00,
                cached_per_million: None,
                reasoning_per_million: None,
            },
        );

        // GPT-3.5 Turbo
        models.insert(
            "gpt-3.5-turbo".to_string(),
            ModelPricing {
                input_per_million: 0.50,
                output_per_million: 1.50,
                cached_per_million: None,
                reasoning_per_million: None,
            },
        );

        Self {
            models,
            default: Some(ModelPricing {
                input_per_million: 2.50,
                output_per_million: 10.00,
                cached_per_million: Some(1.25),
                reasoning_per_million: None,
            }),
        }
    }

    /// Load from JSON file
    pub fn load_from_file(path: &str) -> Result<Self, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Get pricing for a model, with prefix matching and fallback
    pub fn get_pricing(&self, model: &str) -> Option<&ModelPricing> {
        // Exact match first
        if let Some(pricing) = self.models.get(model) {
            return Some(pricing);
        }

        // Prefix match (e.g., "gpt-4o-2024-08-06" matches "gpt-4o")
        for (key, pricing) in &self.models {
            if model.starts_with(key) {
                return Some(pricing);
            }
        }

        // Default fallback
        self.default.as_ref()
    }

    /// Calculate cost for a request
    pub fn calculate_cost(
        &self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
    ) -> Option<crate::analytics::CostInfo> {
        let pricing = self.get_pricing(model)?;
        let (input_cost, output_cost, cached_cost, total_cost) =
            pricing.calculate_cost(input_tokens, output_tokens, cached_tokens, reasoning_tokens);

        Some(crate::analytics::CostInfo {
            input_cost,
            output_cost,
            cached_cost,
            total_cost,
            currency: "USD".to_string(),
            pricing_model: Some(model.to_string()),
        })
    }
}

impl Default for PricingConfig {
    fn default() -> Self {
        Self::openai_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpt4o_pricing() {
        let config = PricingConfig::openai_defaults();
        let cost = config
            .calculate_cost("gpt-4o", 1_000_000, 1_000_000, None, None)
            .unwrap();

        assert_eq!(cost.input_cost, 2.50);
        assert_eq!(cost.output_cost, 10.00);
        assert_eq!(cost.total_cost, 12.50);
    }

    #[test]
    fn test_gpt4o_mini_pricing() {
        let config = PricingConfig::openai_defaults();
        let cost = config
            .calculate_cost("gpt-4o-mini", 1_000_000, 1_000_000, None, None)
            .unwrap();

        assert_eq!(cost.input_cost, 0.150);
        assert_eq!(cost.output_cost, 0.600);
        assert_eq!(cost.total_cost, 0.750);
    }

    #[test]
    fn test_cached_tokens() {
        let config = PricingConfig::openai_defaults();
        let cost = config
            .calculate_cost("gpt-4o", 1_000_000, 1_000_000, Some(1_000_000), None)
            .unwrap();

        // Input tokens are fully cached, so base input cost should be 0 and
        // cost is charged via cached pricing only.
        assert_eq!(cost.input_cost, 0.0);
        assert_eq!(cost.output_cost, 10.00);
        assert_eq!(cost.cached_cost, Some(1.25));
        assert_eq!(cost.total_cost, 11.25);
    }

    #[test]
    fn test_partially_cached_tokens() {
        let config = PricingConfig::openai_defaults();
        let cost = config
            .calculate_cost("gpt-4o", 1_000_000, 0, Some(400_000), None)
            .unwrap();

        // 600k uncached @ $2.50/M = $1.50
        assert!((cost.input_cost - 1.5).abs() < 1e-9);
        // 400k cached @ $1.25/M = $0.50
        assert_eq!(cost.cached_cost, Some(0.5));
        assert!((cost.total_cost - 2.0).abs() < 1e-9);
    }

    #[test]
    fn test_prefix_matching() {
        let config = PricingConfig::openai_defaults();
        let cost = config
            .calculate_cost("gpt-4o-2024-08-06", 1_000_000, 1_000_000, None, None)
            .unwrap();

        assert_eq!(cost.input_cost, 2.50);
        assert_eq!(cost.output_cost, 10.00);
    }
}
