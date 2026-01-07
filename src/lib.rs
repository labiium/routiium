#![forbid(unsafe_code)]
#![doc = r#"
Routiium

Translate OpenAI Chat Completions requests into Responses API payloads and proxy them to OpenAI's Responses endpoint.

Crate highlights
- Library: pure conversion via `to_responses_request(&ChatCompletionRequest, Option<String>)`.
- HTTP server (in `server`): `/convert` and `/proxy` (always available; proxy forwards to `OPENAI_BASE_URL`).
- Models: minimal but robust request models for Chat Completions and Responses APIs.

Modules
- `models`: Data structures for Chat and Responses.
- `conversion`: Mapping logic from Chat → Responses.
- `server`: Axum router/handlers (optional binary uses this).
- `util`: Shared helpers (tracing, env, SSE utilities).

Note: Keep the mapping rules aligned with OpenAI docs; the Responses API evolves over time.
"#]

pub mod analytics;
pub mod analytics_middleware;
pub mod auth;
pub mod bedrock;

pub mod conversion;
pub mod mcp_client;
pub mod mcp_config;
pub mod models;
pub mod pricing;
pub mod router_client;
pub mod routing_config;
pub mod server;
pub mod system_prompt_config;
pub mod util;

// Re-export the primary conversion function for ergonomic library use.
pub use crate::analytics::{AnalyticsEvent, AnalyticsManager, CostInfo, TokenUsage};
pub use crate::auth::{ApiKeyInfo, ApiKeyManager, GeneratedKey, Verification};
pub use crate::pricing::{ModelPricing, PricingConfig};
pub use crate::router_client::{
    CachedRouterClient, FallbackConfig, HttpRouterClient, HttpRouterConfig, LocalPolicyRouter,
    LocalRouter, PrivacyTier, RouteError, RouteFeedback, RouteHints, RouteLimits, RoutePlan,
    RouteRequest, RouterCache, RouterClient, RouterMode, UpstreamConfig,
    UpstreamMode as RouterUpstreamMode,
};
pub use crate::routing_config::{
    BackendConfig, LoadBalanceStrategy, MatchStrategy, ModelAlias, RequestTransform, ResolvedRoute,
    RoutingConfig, RoutingRule, RoutingStats, UpstreamMode as RoutingUpstreamMode,
};

pub use crate::conversion::{
    chat_to_responses_response, responses_chunk_to_chat_chunk, responses_to_chat_response,
    to_responses_request,
};

// Re-export model namespaces for convenience (downstream users can do `use routiium::chat`).
pub use crate::models::{chat, responses};
