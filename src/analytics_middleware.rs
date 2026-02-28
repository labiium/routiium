use crate::analytics::{
    current_timestamp, generate_event_id, AnalyticsEvent, AnalyticsManager, AuthMetadata,
    PerformanceMetrics, RequestMetadata, ResponseMetadata, RoutingMetadata,
};
use actix_web::{
    body::MessageBody,
    dev::{ServiceRequest, ServiceResponse},
    middleware::Next,
    Error, HttpMessage,
};
use std::time::Instant;

/// Extension to store analytics context during request processing
#[derive(Clone)]
pub struct AnalyticsContext {
    pub event_id: String,
    pub start_time: Instant,
    pub timestamp: u64,
    pub request_metadata: RequestMetadata,
    pub auth_metadata: AuthMetadata,
    pub routing_metadata: RoutingMetadata,
}

/// Summary of the outcome we're about to record for analytics.
pub struct AnalyticsOutcome {
    pub status_code: u16,
    pub response_size: usize,
    pub success: bool,
    pub error_message: Option<String>,
    pub output_tokens: Option<u64>,
    pub token_usage: Option<crate::analytics::TokenUsage>,
}

impl AnalyticsContext {
    pub fn new(req: &ServiceRequest) -> Self {
        let start_time = Instant::now();
        let timestamp = current_timestamp();
        let event_id = generate_event_id();

        // Extract request metadata
        let endpoint = req.path().to_string();
        let method = req.method().to_string();
        let user_agent = req
            .headers()
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        // Try to extract client IP from various headers
        let client_ip = req
            .headers()
            .get("x-forwarded-for")
            .or_else(|| req.headers().get("x-real-ip"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
            .or_else(|| req.peer_addr().map(|addr| addr.ip().to_string()));

        let request_metadata = RequestMetadata {
            endpoint,
            method,
            model: None, // Will be populated from request body
            stream: false,
            size_bytes: 0,
            message_count: None,
            input_tokens: None,
            user_agent,
            client_ip,
        };

        let auth_metadata = AuthMetadata {
            authenticated: false,
            api_key_id: None,
            api_key_label: None,
            auth_method: None,
        };

        let routing_metadata = RoutingMetadata {
            backend: "unknown".to_string(),
            upstream_mode: "unknown".to_string(),
            mcp_enabled: false,
            mcp_servers: Vec::new(),
            system_prompt_applied: false,
        };

        Self {
            event_id,
            start_time,
            timestamp,
            request_metadata,
            auth_metadata,
            routing_metadata,
        }
    }

    pub fn with_model(mut self, model: Option<String>) -> Self {
        self.request_metadata.model = model;
        self
    }

    pub fn with_stream(mut self, stream: bool) -> Self {
        self.request_metadata.stream = stream;
        self
    }

    pub fn with_size(mut self, size_bytes: usize) -> Self {
        self.request_metadata.size_bytes = size_bytes;
        self
    }

    pub fn with_message_count(mut self, count: usize) -> Self {
        self.request_metadata.message_count = Some(count);
        self
    }

    pub fn with_input_tokens(mut self, tokens: u64) -> Self {
        self.request_metadata.input_tokens = Some(tokens);
        self
    }

    pub fn with_auth(
        mut self,
        authenticated: bool,
        key_id: Option<String>,
        label: Option<String>,
    ) -> Self {
        self.auth_metadata.authenticated = authenticated;
        self.auth_metadata.api_key_id = key_id;
        self.auth_metadata.api_key_label = label;
        if authenticated {
            self.auth_metadata.auth_method = Some("bearer".to_string());
        }
        self
    }

    pub fn with_routing(
        mut self,
        backend: String,
        upstream_mode: String,
        mcp_enabled: bool,
        mcp_servers: Vec<String>,
        system_prompt_applied: bool,
    ) -> Self {
        self.routing_metadata.backend = backend;
        self.routing_metadata.upstream_mode = upstream_mode;
        self.routing_metadata.mcp_enabled = mcp_enabled;
        self.routing_metadata.mcp_servers = mcp_servers;
        self.routing_metadata.system_prompt_applied = system_prompt_applied;
        self
    }

    pub async fn finalize_and_record(
        self,
        manager: &AnalyticsManager,
        pricing: &crate::pricing::PricingConfig,
        outcome: AnalyticsOutcome,
    ) {
        let duration_ms = self.start_time.elapsed().as_millis() as u64;

        let response_metadata = ResponseMetadata {
            status_code: outcome.status_code,
            size_bytes: outcome.response_size,
            output_tokens: outcome.output_tokens,
            success: outcome.success,
            error_message: outcome.error_message,
        };

        // Calculate tokens per second if we have output tokens
        let tokens_per_second = if let Some(tokens) = outcome.output_tokens {
            if duration_ms > 0 {
                Some((tokens as f64 / duration_ms as f64) * 1000.0)
            } else {
                None
            }
        } else {
            None
        };

        let performance_metrics = PerformanceMetrics {
            duration_ms,
            ttfb_ms: None, // Could be set for streaming
            upstream_duration_ms: None,
            tokens_per_second,
        };

        // Calculate cost if we have token usage and model
        let cost = if let (Some(ref usage), Some(ref model)) =
            (&outcome.token_usage, &self.request_metadata.model)
        {
            pricing.calculate_cost(
                model,
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.cached_tokens,
                usage.reasoning_tokens,
            )
        } else {
            None
        };

        let event = AnalyticsEvent {
            id: self.event_id,
            timestamp: self.timestamp,
            request: self.request_metadata,
            response: Some(response_metadata),
            performance: performance_metrics,
            auth: self.auth_metadata,
            routing: self.routing_metadata,
            token_usage: outcome.token_usage,
            cost,
        };

        if let Err(e) = manager.record(event).await {
            tracing::error!("Failed to record analytics event: {}", e);
        }
    }
}

/// Middleware to automatically capture analytics for all requests
pub async fn analytics_middleware(
    req: ServiceRequest,
    next: Next<impl MessageBody>,
) -> Result<ServiceResponse<impl MessageBody>, Error> {
    // Create analytics context
    let ctx = AnalyticsContext::new(&req);

    // Store context in request extensions for handlers to update
    req.extensions_mut().insert(ctx.clone());

    // Process request
    let res = next.call(req).await?;

    // Note: Final recording happens in individual handlers where we have more context
    // This middleware just sets up the initial context

    Ok(res)
}

/// Helper to extract and update analytics context from request body
pub fn update_context_from_body(ctx: &mut AnalyticsContext, body: &serde_json::Value) {
    // Extract model
    if let Some(model) = body.get("model").and_then(|v| v.as_str()) {
        ctx.request_metadata.model = Some(model.to_string());
    }

    // Extract stream flag
    if let Some(stream) = body.get("stream").and_then(|v| v.as_bool()) {
        ctx.request_metadata.stream = stream;
    }

    // Count messages
    if let Some(messages) = body.get("messages").and_then(|v| v.as_array()) {
        ctx.request_metadata.message_count = Some(messages.len());
    }

    // Estimate body size
    if let Ok(json_str) = serde_json::to_string(body) {
        ctx.request_metadata.size_bytes = json_str.len();
    }
}

/// Extract token usage from OpenAI-style response
pub fn extract_token_usage(
    response_body: &serde_json::Value,
) -> Option<crate::analytics::TokenUsage> {
    let usage = response_body.get("usage")?;

    // Chat Completions schema: prompt_tokens/completion_tokens
    // Responses schema: input_tokens/output_tokens
    let prompt_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(|v| v.as_u64())?;
    let completion_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(|v| v.as_u64())?;
    let total_tokens = usage
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(prompt_tokens + completion_tokens);

    let cached_tokens = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .or_else(|| usage.get("cached_tokens").and_then(|v| v.as_u64()));

    let reasoning_tokens = usage
        .get("completion_tokens_details")
        .and_then(|d| d.get("reasoning_tokens"))
        .and_then(|v| v.as_u64())
        .or_else(|| usage.get("reasoning_tokens").and_then(|v| v.as_u64()));

    Some(crate::analytics::TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cached_tokens,
        reasoning_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::extract_token_usage;
    use serde_json::json;

    #[test]
    fn extract_usage_chat_schema() {
        let payload = json!({
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 8,
                "total_tokens": 20,
                "prompt_tokens_details": { "cached_tokens": 4 },
                "completion_tokens_details": { "reasoning_tokens": 2 }
            }
        });

        let usage = extract_token_usage(&payload).expect("usage");
        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 8);
        assert_eq!(usage.total_tokens, 20);
        assert_eq!(usage.cached_tokens, Some(4));
        assert_eq!(usage.reasoning_tokens, Some(2));
    }

    #[test]
    fn extract_usage_responses_schema() {
        let payload = json!({
            "usage": {
                "input_tokens": 30,
                "output_tokens": 10,
                "cached_tokens": 6,
                "reasoning_tokens": 1
            }
        });

        let usage = extract_token_usage(&payload).expect("usage");
        assert_eq!(usage.prompt_tokens, 30);
        assert_eq!(usage.completion_tokens, 10);
        assert_eq!(usage.total_tokens, 40);
        assert_eq!(usage.cached_tokens, Some(6));
        assert_eq!(usage.reasoning_tokens, Some(1));
    }
}
