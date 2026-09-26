#![forbid(unsafe_code)]
#![doc = r"
Routiium

Run an OpenAI-compatible LLM gateway for routing, safety policy, analytics, and provider proxying.

Crate highlights
- Gateway: OpenAI-compatible `/v1` proxying with auth, rate limits, routing, judging, analytics, and MCP tools.
- Providers: OpenAI/Responses, OpenAI-compatible endpoints, AWS Bedrock, and router-selected upstreams.
- Library: request/response conversion helpers remain available for compatibility and tests.

Modules
- `models`: Data structures for Chat and Responses.
- `conversion`: Mapping logic between Chat Completions and Responses shapes.
- `server`: Actix Web gateway handlers and administrative endpoints.
- `router_client`: Local and remote routing policy clients.
- `analytics`: Request, usage, cost, and routing event storage.
- `util`: Shared helpers for tracing, environment parsing, and SSE utilities.

Note: Keep gateway contracts and provider mappings aligned with upstream API changes.
"]

pub mod analytics;
pub mod analytics_middleware;
pub mod app_config;
pub mod auth;
pub mod bedrock;
pub mod bedrock_mantle;
pub mod chat_history;
pub mod chat_history_jsonl;
pub mod chat_history_manager;
pub mod chat_history_memory;
pub mod rate_limit;

#[cfg(feature = "sqlite")]
pub mod chat_history_sqlite;

#[cfg(feature = "postgres")]
pub mod chat_history_postgres;

pub mod conversion;
pub mod decision;
pub mod jev;
pub mod judge_selector;
pub mod mcp_client;
pub mod mcp_config;
pub mod models;
pub mod pricing;
pub mod router_client;
pub mod routing_config;
pub mod safety_audit;
pub mod safety_judge;
pub mod server;
pub mod subscription_proxy;
pub mod system_prompt_config;
pub mod util;

// Re-export the primary conversion function for ergonomic library use.
pub use crate::analytics::{AnalyticsEvent, AnalyticsManager, CostInfo, TokenUsage};
pub use crate::auth::{ApiKeyInfo, ApiKeyManager, GeneratedKey, Verification};
pub use crate::chat_history::{
    ChatHistoryError, ChatHistoryStore, CompositeStore, Conversation, ConversationFilters,
    CostInfo as ChatCostInfo, MCPInfo, Message, MessageFilters, MessageRole, PrivacyLevel,
    RoutingInfo, StorageStats, TokenInfo,
};
pub use crate::chat_history_jsonl::JsonlChatHistoryStore;
pub use crate::chat_history_manager::{ChatHistoryConfig, ChatHistoryManager};
pub use crate::chat_history_memory::MemoryChatHistoryStore;
pub use crate::decision::{
    choice_criteria, questions, state_object, ChoiceQuestion, DecisionAnswer, DecisionClient,
    DecisionClientConfig, DecisionError, DecisionProvider, DecisionProviderKind, DecisionQuestion,
    DecisionRequest, DecisionResponse, DecisionUsage, NoulQuestion, ScoreQuestion,
};

#[cfg(feature = "sqlite")]
pub use crate::chat_history_sqlite::SqliteChatHistoryStore;

#[cfg(feature = "postgres")]
pub use crate::chat_history_postgres::PostgresChatHistoryStore;

pub use crate::jev::{
    JevCandidate, JevClient, JevConfig, JevError, JevModelRouteRequest, JevRouteDecision,
};
pub use crate::pricing::{ModelPricing, PricingConfig};
pub use crate::router_client::{
    CachedRouterClient, DecisionRouterClient, DecisionRouterConfig, EmbeddedDefaultRouter,
    FallbackConfig, HttpRouterClient, HttpRouterConfig, JevRouterClient, JudgeMetadata,
    LocalPolicyRouter, LocalRouter, PrivacyTier, RouteError, RouteFeedback, RouteHints,
    RouteLimits, RoutePlan, RouteRequest, RouterCache, RouterClient, RouterMode, UpstreamConfig,
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
