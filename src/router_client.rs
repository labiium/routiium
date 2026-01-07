//! Router Client Module
//!
//! Provides a clean interface for delegating model selection and routing decisions
//! to a Router service (local or remote), enabling virtual model aliases and
//! sophisticated routing policies without embedding policy logic in Routiium.
//!
//! ## Architecture
//!
//! - `RouterClient` trait: Thin, fast decision interface
//! - `LocalRouter` trait: Embedded policy engine for single-binary deployments
//! - `HttpRouterClient`: Remote Router communication via HTTP
//! - `LocalPolicyRouter`: Simple embedded routing policies
//! - `RouterCache`: Decision caching with TTL and ETag validation
//!
//! ## Performance
//!
//! - Local routing: P50 ≤ 1-2ms, P95 ≤ 5ms
//! - Remote routing: P50 ≤ 3-5ms, P95 ≤ 15ms (same AZ)
//! - Cache hit: ~sub-100µs

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;

/// Mode for upstream API (Responses, Chat Completions, or Bedrock)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UpstreamMode {
    Responses,
    Chat,
    Bedrock,
}

impl Default for UpstreamMode {
    fn default() -> Self {
        Self::Responses
    }
}

/// Privacy mode for Router communication
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyMode {
    /// Only features/hashes, no raw content (default, safest)
    FeaturesOnly,
    /// Include short summaries of system prompt and last-K turns
    Summary,
    /// Include full system prompt and last-K messages (use sparingly)
    Full,
}

impl Default for PrivacyMode {
    fn default() -> Self {
        Self::FeaturesOnly
    }
}

/// Privacy/compliance tier hint
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyTier {
    /// Educational cloud OK
    EduCloudOk,
    /// On-premises only
    OnPremOnly,
    /// Public cloud with DPA
    PublicCloudDpa,
    /// No restrictions
    Unrestricted,
}

impl Default for PrivacyTier {
    fn default() -> Self {
        Self::Unrestricted
    }
}

/// Model capabilities metadata
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Capabilities {
    /// Supported modalities (e.g., ["text", "image", "audio"])
    #[serde(default)]
    pub modalities: Vec<String>,

    /// Maximum context window in tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u32>,

    /// Supports tool/function calling
    #[serde(default)]
    pub tools: bool,

    /// Supports JSON mode
    #[serde(default)]
    pub json_mode: bool,

    /// Supports prompt caching
    #[serde(default)]
    pub prompt_cache: bool,

    /// Supports logprobs
    #[serde(default)]
    pub logprobs: bool,

    /// Supports structured output
    #[serde(default)]
    pub structured_output: bool,
}

/// Provider-enforced rate limits and bounds
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelLimits {
    /// Transactions per second
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tps: Option<u32>,

    /// Requests per minute
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpm: Option<u32>,

    /// Requests per second burst capacity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rps_burst: Option<u32>,
}

/// Cost information for a model
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostCard {
    /// Currency code (e.g., "USD", "GBP")
    #[serde(default = "default_currency")]
    pub currency: String,

    /// Input tokens cost per million
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_per_million: Option<f64>,

    /// Output tokens cost per million
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_per_million: Option<f64>,

    /// Cached tokens cost per million
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_per_million: Option<f64>,

    /// Reasoning tokens cost per million
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_per_million: Option<f64>,

    /// Input tokens cost per million (minor units)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_per_million_micro: Option<u64>,

    /// Output tokens cost per million (minor units)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_per_million_micro: Option<u64>,

    /// Cached tokens cost per million (minor units)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_per_million_micro: Option<u64>,

    /// Reasoning tokens cost per million (minor units)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_per_million_micro: Option<u64>,
}

fn default_currency() -> String {
    "USD".to_string()
}

/// Service Level Objectives and actuals
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SLOs {
    /// Target P95 latency in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_p95_ms: Option<u64>,

    /// Recent observed metrics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent: Option<RecentMetrics>,
}

/// Recent observed metrics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecentMetrics {
    /// P50 latency in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p50_ms: Option<u64>,

    /// P95 latency in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p95_ms: Option<u64>,

    /// Error rate (0.0 to 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_rate: Option<f64>,

    /// Tokens per second throughput
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_sec: Option<f64>,
}

/// Model catalog entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogModel {
    /// Model identifier
    pub id: String,

    /// Provider name (e.g., "openai", "anthropic")
    pub provider: String,

    /// Regions where model can be served
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<Vec<String>>,

    /// Aliases and tags (e.g., ["tier:T1", "family:gpt-4o-mini"])
    #[serde(default)]
    pub aliases: Vec<String>,

    /// Model capabilities
    #[serde(default)]
    pub capabilities: Capabilities,

    /// Usage notes and recommendations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_notes: Option<String>,

    /// Cost information
    #[serde(default)]
    pub cost: CostCard,

    /// Service level objectives
    #[serde(default)]
    pub slos: SLOs,

    /// Provider rate limits and throughput bounds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<ModelLimits>,

    /// Policy tags (e.g., ["T1", "edu_safe", "offline_ok"])
    #[serde(default)]
    pub policy_tags: Vec<String>,

    /// Current status (e.g., "healthy", "degraded", "deprecated")
    #[serde(default = "default_status")]
    pub status: String,

    /// Additional status details (e.g., reason for degraded state)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,

    /// Deprecation date (ISO 8601)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecates_at: Option<String>,

    /// Router-level rate limiting policy identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rl_policy: Option<String>,

    /// Whether the model is deprecated or scheduled for removal
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<bool>,
}

fn default_status() -> String {
    "healthy".to_string()
}

/// Model catalog response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalog {
    /// Catalog revision
    pub revision: String,

    /// Models in catalog
    pub models: Vec<CatalogModel>,
}

/// Trace context propagated via W3C headers
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TraceContext {
    /// Traceparent header value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,

    /// Tracestate header value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracestate: Option<String>,
}

/// Spending limit expressed in minor units
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Budget {
    /// Amount in minor units (e.g., micro-USD)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_micro: Option<u64>,

    /// ISO currency code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

/// Request targets (latency, throughput, reliability)
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Targets {
    /// Target P95 latency in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "latency_p95_ms")]
    #[serde(alias = "latency_target_ms")]
    pub p95_latency_ms: Option<u64>,

    /// Minimum desired tokens/second
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_tokens_per_sec: Option<u32>,

    /// Desired reliability tier (e.g., "standard", "high")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reliability_tier: Option<String>,

    /// Legacy maximum cost in USD (deprecated)
    #[serde(default)]
    #[serde(skip_serializing)]
    #[serde(alias = "max_cost_usd")]
    pub legacy_max_cost_usd: Option<f64>,

    /// Legacy maximum cost in GBP (deprecated)
    #[serde(default)]
    #[serde(skip_serializing)]
    #[serde(alias = "max_cost_gbp")]
    pub legacy_max_cost_gbp: Option<f64>,
}

/// Geo hints for residency-aware routing
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GeoHint {
    /// Preferred region (e.g., "eu-west-1")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

/// Attestation of what content was included
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ContentAttestation {
    /// Level of content shared with router: none|summary|full
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<String>,
}

/// Token and output estimates
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Estimates {
    /// Estimated prompt tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,

    /// Maximum output tokens requested
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,

    /// Tokenizer identifier used for estimates
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokenizer_id: Option<String>,
}

/// Conversation signals for context-aware routing
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConversationSignals {
    /// Total number of turns in conversation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turns: Option<u32>,

    /// Seconds since last activity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_s: Option<u64>,

    /// Hash of system prompt (SHA-256)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,

    /// Hash of conversation history (SHA-256)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_fingerprint: Option<String>,

    /// Optional short summary (1-2 lines) - only with privacy_mode != FeaturesOnly
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,

    /// Full system prompt - only with privacy_mode = Full
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// Last K messages - only with privacy_mode = Full
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_messages: Option<Vec<serde_json::Value>>,
}

/// Organization/tenant context
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OrgContext {
    /// Tenant identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,

    /// Project/course identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,

    /// User role (e.g., "student", "teacher", "admin")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

    /// User identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

/// Tool/function call signal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSignal {
    /// Tool/function name
    pub name: String,

    /// Hash of JSON schema (SHA-256)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema_hash: Option<String>,
}

/// Request to Router for routing decision (v0.3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRequest {
    /// Schema version for compatibility negotiation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,

    /// Unique request identifier for traceability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,

    /// Distributed trace context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceContext>,

    /// Virtual model alias (e.g., "labiium-001")
    pub alias: String,

    /// API type being called ("responses" or "chat")
    #[serde(default)]
    pub api: String,

    /// Privacy mode for this request
    #[serde(default)]
    pub privacy_mode: PrivacyMode,

    /// Announcement of content inclusion for auditing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_attestation: Option<ContentAttestation>,

    /// Declared capabilities needed (e.g., ["text", "code", "vision", "tools"])
    #[serde(default)]
    pub caps: Vec<String>,

    /// Stream flag
    #[serde(default)]
    pub stream: bool,

    /// Request parameters (temperature, json_mode, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,

    /// Sticky routing plan token from previous decision
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_token: Option<String>,

    /// Request targets (latency, cost)
    #[serde(default)]
    pub targets: Targets,

    /// Budget limits in minor units
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,

    /// Token and output estimates
    #[serde(default)]
    pub estimates: Estimates,

    /// Conversation signals
    #[serde(default)]
    pub conversation: ConversationSignals,

    /// Organization/tenant context
    #[serde(default)]
    pub org: OrgContext,

    /// Data residency or latency hints
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geo: Option<GeoHint>,

    /// Tools requested
    #[serde(default)]
    pub tools: Vec<ToolSignal>,

    /// Request overrides (allow_premium, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overrides: Option<serde_json::Value>,

    // Legacy fields for backward compatibility
    /// Optional role/context (deprecated: use org.role)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

    /// Optional task type hint
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,

    /// Privacy/compliance tier (deprecated: use privacy_mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy: Option<PrivacyTier>,

    /// Additional hints
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<HashMap<String, serde_json::Value>>,
}

/// Upstream backend configuration from Router decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamConfig {
    /// Base URL (e.g., "https://api.openai.com/v1")
    pub base_url: String,

    /// API mode (responses or chat)
    pub mode: UpstreamMode,

    /// Actual model ID to use
    pub model_id: String,

    /// Environment variable name for API key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_env: Option<String>,

    /// Additional headers to send to provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

/// Resource limits from Router
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouteLimits {
    /// Maximum input tokens allowed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,

    /// Maximum output tokens allowed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,

    /// Request timeout in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// Prompt overlays from Router
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptOverlays {
    /// System prompt overlay to inject
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_overlay: Option<String>,

    /// Hash of overlay for cache validation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay_fingerprint: Option<String>,

    /// Size of overlay in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay_size_bytes: Option<u64>,

    /// Maximum overlay size allowed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_overlay_bytes: Option<u64>,
}

/// Routing hints (cost, latency, tier) - v0.3
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouteHints {
    /// Tier designation (e.g., "T1", "T2", "T3")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,

    /// Estimated cost in micro currency units
    #[serde(skip_serializing_if = "Option::is_none")]
    pub est_cost_micro: Option<u64>,

    /// Currency for cost estimate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,

    /// Estimated latency in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub est_latency_ms: Option<u64>,

    /// Provider name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,

    /// Relative penalty when selecting fallbacks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub penalty: Option<f32>,
}

/// Fallback backend configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackConfig {
    /// Base URL for fallback
    pub base_url: String,

    /// API mode
    pub mode: UpstreamMode,

    /// Model ID for fallback
    pub model_id: String,

    /// Reason for fallback (e.g., "openai_ratelimit")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Auth env var
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_env: Option<String>,

    /// Optional relative penalty score for ranking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub penalty: Option<f32>,
}

/// Cache control from Router
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    /// TTL in milliseconds
    pub ttl_ms: u64,

    /// ETag for validation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,

    /// Absolute expiry timestamp (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,

    /// Key used to freeze/invalidate a cached plan
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freeze_key: Option<String>,
}

/// Stickiness configuration to pin routing plan across turns
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Stickiness {
    /// Token to re-use an existing plan
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_token: Option<String>,

    /// Maximum number of turns this plan remains valid
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,

    /// Absolute expiry timestamp (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// Policy metadata returned with the plan
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyInfo {
    /// Policy revision identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,

    /// Unique policy ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Human-readable explanation for auditing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<String>,
}

/// Complete routing plan from Router (v0.3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutePlan {
    /// Schema version for compatibility negotiation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,

    /// Unique route ID for tracing
    pub route_id: String,

    /// Primary upstream configuration
    pub upstream: UpstreamConfig,

    /// Resource limits to enforce
    #[serde(default)]
    pub limits: RouteLimits,

    /// Prompt overlays from Router
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_overlays: Option<PromptOverlays>,

    /// Hints for cost/latency
    #[serde(default)]
    pub hints: RouteHints,

    /// Fallback backends (ordered by preference)
    #[serde(default)]
    pub fallbacks: Vec<FallbackConfig>,

    /// Cache control
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<CacheControl>,

    /// Policy revision
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_rev: Option<String>,

    /// Structured policy metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicyInfo>,

    /// Stickiness metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stickiness: Option<Stickiness>,

    /// Declared content usage level
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_used: Option<String>,
}

/// Token usage details for feedback
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsageDetails {
    /// Prompt tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,

    /// Completion tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u32>,

    /// Cached tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u32>,

    /// Reasoning tokens (for o1 models)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
}

/// Feedback to Router after request completion (v0.3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteFeedback {
    /// Route ID from plan
    pub route_id: String,

    /// Model ID actually used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,

    /// Success or failure
    pub success: bool,

    /// Duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Token usage details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsageDetails>,

    /// HTTP status code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,

    /// Errors encountered
    #[serde(default)]
    pub errors: Vec<String>,

    /// Actual cost in USD (deprecated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_cost_usd: Option<f64>,

    /// Actual cost in GBP (deprecated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_cost_gbp: Option<f64>,

    /// Actual cost in micro currency units
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_cost_micro: Option<u64>,

    /// Currency for actual cost
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,

    /// Upstream-specific error code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_error_code: Option<String>,

    /// Whether router-level rate limiting applied
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rl_applied: Option<bool>,

    /// Whether router cache was hit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit: Option<bool>,

    // Legacy fields for backward compatibility
    /// Input tokens (deprecated: use usage.prompt_tokens)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,

    /// Output tokens (deprecated: use usage.completion_tokens)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,

    /// Latency (deprecated: use duration_ms)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,

    /// Error message (deprecated: use errors array)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Router error types
#[derive(Debug, thiserror::Error)]
pub enum RouteError {
    #[error("Router timeout: {0}")]
    Timeout(String),

    #[error("Router unavailable: {0}")]
    Unavailable(String),

    #[error("No route found for alias: {0}")]
    NoRoute(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Router error: {0}")]
    RouterError(String),

    #[error("Network error: {0}")]
    NetworkError(String),
}

/// Main RouterClient trait - thin, fast decision interface
#[async_trait]
pub trait RouterClient: Send + Sync {
    /// Get routing plan for request
    async fn plan(&self, req: &RouteRequest) -> Result<RoutePlan, RouteError>;

    /// Send feedback after request (optional, async)
    fn feedback(&self, fb: &RouteFeedback) -> Result<(), RouteError> {
        let _ = fb;
        Ok(())
    }

    /// Get policy revision (for cache invalidation)
    fn policy_revision(&self) -> Option<String> {
        None
    }

    /// Get model catalog (optional, cached)
    async fn get_catalog(&self) -> Result<ModelCatalog, RouteError> {
        Err(RouteError::RouterError("Catalog not supported".to_string()))
    }
}

/// Local (embedded) policy engine trait
pub trait LocalRouter: Send + Sync {
    /// Local routing decision
    fn plan_local(&self, req: &RouteRequest) -> Result<RoutePlan, RouteError>;
}

/// Router mode configuration
#[derive(Clone)]
pub enum RouterMode {
    /// Use local embedded router
    Local(Arc<dyn LocalRouter>),
    /// Use remote HTTP router
    Remote(HttpRouterConfig),
    /// Hybrid: try local first, fall back to remote
    Hybrid {
        local: Arc<dyn LocalRouter>,
        remote: HttpRouterConfig,
    },
}

impl std::fmt::Debug for RouterMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouterMode::Local(_) => f.debug_tuple("Local").field(&"<LocalRouter>").finish(),
            RouterMode::Remote(config) => f.debug_tuple("Remote").field(config).finish(),
            RouterMode::Hybrid { remote, .. } => f
                .debug_struct("Hybrid")
                .field("local", &"<LocalRouter>")
                .field("remote", remote)
                .finish(),
        }
    }
}

/// HTTP Router client configuration
#[derive(Debug, Clone)]
pub struct HttpRouterConfig {
    /// Router base URL
    pub url: String,

    /// Request timeout in milliseconds
    pub timeout_ms: u64,

    /// Enable mTLS
    pub mtls: bool,

    /// HTTP client (shared)
    pub client: Option<reqwest::Client>,
}

impl Default for HttpRouterConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:9090".to_string(),
            timeout_ms: 15,
            mtls: false,
            client: None,
        }
    }
}

/// HTTP-based Router client (remote)
pub struct HttpRouterClient {
    config: HttpRouterConfig,
    client: reqwest::Client,
}

impl HttpRouterClient {
    /// Create new HTTP router client
    pub fn new(mut config: HttpRouterConfig) -> Result<Self> {
        let client = if let Some(c) = config.client.take() {
            c
        } else {
            reqwest::Client::builder()
                .timeout(Duration::from_millis(config.timeout_ms))
                .pool_idle_timeout(Duration::from_secs(60))
                .pool_max_idle_per_host(10)
                .build()?
        };

        Ok(Self { config, client })
    }

    /// Get model catalog from Router
    async fn get_catalog_async(&self) -> Result<ModelCatalog, RouteError> {
        let url = format!("{}/catalog/models", self.config.url.trim_end_matches('/'));

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| RouteError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(RouteError::RouterError(format!(
                "Catalog request failed: {}",
                status
            )));
        }

        let catalog: ModelCatalog = response
            .json()
            .await
            .map_err(|e| RouteError::RouterError(format!("Failed to parse catalog: {}", e)))?;

        Ok(catalog)
    }

    /// Make plan request to remote router
    async fn plan_async(&self, req: &RouteRequest) -> Result<RoutePlan, RouteError> {
        let url = format!("{}/route/plan", self.config.url.trim_end_matches('/'));

        let response = self
            .client
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(|e| RouteError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(RouteError::RouterError(format!(
                "Router returned {}: {}",
                status, body
            )));
        }

        let plan: RoutePlan = response
            .json()
            .await
            .map_err(|e| RouteError::RouterError(format!("Failed to parse plan: {}", e)))?;

        Ok(plan)
    }
}

#[async_trait]
impl RouterClient for HttpRouterClient {
    async fn plan(&self, req: &RouteRequest) -> Result<RoutePlan, RouteError> {
        self.plan_async(req).await.map_err(|e| match e {
            RouteError::NetworkError(_) => RouteError::Unavailable(e.to_string()),
            other => other,
        })
    }

    fn feedback(&self, fb: &RouteFeedback) -> Result<(), RouteError> {
        let runtime = Handle::try_current().ok();
        if let Some(rt) = runtime {
            // Spawn non-blocking
            let client = self.client.clone();
            let url = format!("{}/route/feedback", self.config.url.trim_end_matches('/'));
            let fb = fb.clone();
            rt.spawn(async move {
                let _ = client.post(&url).json(&fb).send().await;
            });
        }
        Ok(())
    }

    async fn get_catalog(&self) -> Result<ModelCatalog, RouteError> {
        self.get_catalog_async().await
    }
}

/// Simple local policy router for embedded deployments
pub struct LocalPolicyRouter {
    /// Static alias map
    aliases: HashMap<String, UpstreamConfig>,
}

impl LocalPolicyRouter {
    /// Create new local policy router with alias map
    pub fn new(aliases: HashMap<String, UpstreamConfig>) -> Self {
        Self { aliases }
    }

    /// Load from JSON file
    pub fn from_file(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let aliases: HashMap<String, UpstreamConfig> = serde_json::from_str(&content)?;
        Ok(Self::new(aliases))
    }

    /// Create empty router
    pub fn empty() -> Self {
        Self::new(HashMap::new())
    }
}

impl LocalRouter for LocalPolicyRouter {
    fn plan_local(&self, req: &RouteRequest) -> Result<RoutePlan, RouteError> {
        // Simple lookup in alias map
        let upstream = self
            .aliases
            .get(&req.alias)
            .ok_or_else(|| RouteError::NoRoute(req.alias.clone()))?
            .clone();

        // Generate route ID
        let uuid_str = uuid::Uuid::new_v4().simple().to_string();
        let route_id = format!("rte_{}", &uuid_str[..16]);

        // Simple plan with no fancy logic
        Ok(RoutePlan {
            schema_version: Some("1.1".to_string()),
            route_id,
            upstream,
            limits: RouteLimits {
                max_output_tokens: req.estimates.max_output_tokens,
                timeout_ms: req.targets.p95_latency_ms.or(Some(30000)),
                max_input_tokens: None,
            },
            prompt_overlays: None,
            hints: RouteHints::default(),
            fallbacks: vec![],
            cache: Some(CacheControl {
                ttl_ms: 15000,
                etag: None,
                valid_until: None,
                freeze_key: None,
            }),
            policy_rev: Some("local_v1".to_string()),
            policy: Some(PolicyInfo {
                revision: Some("local_v1".to_string()),
                id: Some("local_alias_policy".to_string()),
                explain: Some("Resolved via local alias map".to_string()),
            }),
            stickiness: None,
            content_used: req
                .content_attestation
                .as_ref()
                .and_then(|c| c.included.clone())
                .or_else(|| {
                    Some(
                        match req.privacy_mode {
                            PrivacyMode::FeaturesOnly => "none",
                            PrivacyMode::Summary => "summary",
                            PrivacyMode::Full => "full",
                        }
                        .to_string(),
                    )
                }),
        })
    }
}

#[async_trait]
impl RouterClient for LocalPolicyRouter {
    async fn plan(&self, req: &RouteRequest) -> Result<RoutePlan, RouteError> {
        self.plan_local(req)
    }
}

/// Cached routing decision
#[derive(Debug, Clone)]
struct CachedPlan {
    plan: RoutePlan,
    expires_at: Instant,
}

/// Decision cache with TTL and ETag validation
pub struct RouterCache {
    cache: Arc<Mutex<HashMap<String, CachedPlan>>>,
    default_ttl_ms: u64,
}

impl RouterCache {
    /// Create new router cache
    pub fn new(default_ttl_ms: u64) -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
            default_ttl_ms,
        }
    }

    /// Generate cache key from request
    fn cache_key(req: &RouteRequest) -> String {
        // Simple key: alias + api + major features
        let mut parts = vec![
            req.alias.clone(),
            req.api.clone(),
            req.stream.to_string(),
            req.caps.join(","),
        ];

        if let Some(token) = &req.plan_token {
            parts.push(token.clone());
        }

        parts.join("|")
    }

    /// Get cached plan if valid
    pub fn get(&self, req: &RouteRequest, policy_rev: Option<&str>) -> Option<RoutePlan> {
        let key = Self::cache_key(req);
        let mut cache = self.cache.lock().ok()?;

        if let Some(cached) = cache.get(&key) {
            // Check expiry
            if Instant::now() < cached.expires_at {
                // Check policy revision (invalidate if changed)
                if let Some(rev) = policy_rev {
                    if let Some(cached_rev) = &cached.plan.policy_rev {
                        if cached_rev != rev {
                            cache.remove(&key);
                            return None;
                        }
                    }
                }
                return Some(cached.plan.clone());
            } else {
                // Expired
                cache.remove(&key);
            }
        }

        None
    }

    /// Store plan in cache
    pub fn put(&self, req: &RouteRequest, plan: RoutePlan) {
        let key = Self::cache_key(req);
        let ttl_ms = plan
            .cache
            .as_ref()
            .map(|c| c.ttl_ms)
            .unwrap_or(self.default_ttl_ms);

        let expires_at = Instant::now() + Duration::from_millis(ttl_ms);

        let cached = CachedPlan { plan, expires_at };

        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key, cached);
        }
    }

    /// Clear all cached plans
    pub fn clear(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
    }

    /// Evict expired entries
    pub fn evict_expired(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            let now = Instant::now();
            cache.retain(|_, v| v.expires_at > now);
        }
    }
}

/// Router client with caching
pub struct CachedRouterClient {
    inner: Box<dyn RouterClient>,
    cache: RouterCache,
}

impl CachedRouterClient {
    /// Create new cached router client
    pub fn new(inner: Box<dyn RouterClient>, cache_ttl_ms: u64) -> Self {
        Self {
            inner,
            cache: RouterCache::new(cache_ttl_ms),
        }
    }

    /// Clear cache
    pub fn clear_cache(&self) {
        self.cache.clear();
    }
}

#[async_trait]
impl RouterClient for CachedRouterClient {
    async fn plan(&self, req: &RouteRequest) -> Result<RoutePlan, RouteError> {
        // Check cache first
        let policy_rev = self.inner.policy_revision();
        if let Some(cached_plan) = self.cache.get(req, policy_rev.as_deref()) {
            return Ok(cached_plan);
        }

        // Cache miss - ask router
        let plan = self.inner.plan(req).await?;

        // Store in cache
        self.cache.put(req, plan.clone());

        Ok(plan)
    }

    fn feedback(&self, fb: &RouteFeedback) -> Result<(), RouteError> {
        self.inner.feedback(fb)
    }

    fn policy_revision(&self) -> Option<String> {
        self.inner.policy_revision()
    }
}

/// Extract request features for routing decision (v0.3)
pub fn extract_route_request(
    model: &str,
    api: &str,
    payload: &serde_json::Value,
    privacy_mode: PrivacyMode,
) -> RouteRequest {
    // Extract capabilities from payload
    let mut caps = vec!["text".to_string()];

    // Check for vision
    if let Some(messages) = payload.get("messages").or_else(|| payload.get("input")) {
        if let Some(msgs) = messages.as_array() {
            for msg in msgs {
                if let Some(content) = msg.get("content") {
                    if content.is_array() {
                        caps.push("vision".to_string());
                        break;
                    }
                }
            }
        }
    }

    // Check for tools
    if payload.get("tools").is_some() {
        caps.push("tools".to_string());
    }

    // Estimate tokens (very rough)
    let prompt_tokens = estimate_tokens(payload);

    // Extract parameters
    let params = serde_json::json!({
        "temperature": payload.get("temperature"),
        "json_mode": payload.get("response_format").and_then(|v| v.get("type")).and_then(|v| v.as_str()) == Some("json_object"),
    });

    // Build conversation signals
    let conversation = build_conversation_signals(payload, privacy_mode);

    let uuid_str = uuid::Uuid::new_v4().simple().to_string();
    let request_id = format!("req_{}", &uuid_str[..12]);

    let content_attestation = match privacy_mode {
        PrivacyMode::FeaturesOnly => "none",
        PrivacyMode::Summary => "summary",
        PrivacyMode::Full => "full",
    }
    .to_string();

    RouteRequest {
        schema_version: Some("1.1".to_string()),
        request_id: Some(request_id),
        trace: None,
        alias: model.to_string(),
        api: api.to_string(),
        privacy_mode,
        content_attestation: Some(ContentAttestation {
            included: Some(content_attestation),
        }),
        caps,
        stream: payload
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        params: Some(params),
        plan_token: None,
        targets: Targets {
            p95_latency_ms: Some(30000),
            min_tokens_per_sec: None,
            reliability_tier: None,
            legacy_max_cost_usd: None,
            legacy_max_cost_gbp: None,
        },
        budget: None,
        estimates: Estimates {
            prompt_tokens: Some(prompt_tokens),
            max_output_tokens: payload
                .get("max_tokens")
                .or_else(|| payload.get("max_completion_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            tokenizer_id: Some("auto".to_string()),
        },
        conversation,
        org: OrgContext::default(),
        geo: None,
        tools: extract_tools(payload),
        overrides: None,
        // Legacy fields
        role: None,
        task: None,
        privacy: None,
        hints: None,
    }
}

/// Build conversation signals based on privacy mode
fn build_conversation_signals(
    payload: &serde_json::Value,
    privacy_mode: PrivacyMode,
) -> ConversationSignals {
    let messages = payload
        .get("messages")
        .or_else(|| payload.get("input"))
        .and_then(|v| v.as_array());

    let mut signals = ConversationSignals {
        turns: messages.map(|m| m.len() as u32),
        ..Default::default()
    };

    // Always compute fingerprints
    if let Some(msgs) = messages {
        // System prompt fingerprint
        if let Some(system_msg) = msgs.iter().find(|m| {
            m.get("role")
                .and_then(|r| r.as_str())
                .map(|r| r == "system")
                .unwrap_or(false)
        }) {
            if let Some(content) = system_msg.get("content").and_then(|c| c.as_str()) {
                signals.system_fingerprint = Some(format!("sha256:{}", simple_hash(content)));
            }
        }

        // History fingerprint
        let history_str = serde_json::to_string(msgs).unwrap_or_default();
        signals.history_fingerprint = Some(format!("sha256:{}", simple_hash(&history_str)));
    }

    // Add content based on privacy mode
    match privacy_mode {
        PrivacyMode::FeaturesOnly => {
            // No additional content
        }
        PrivacyMode::Summary => {
            // Add short summary
            if let Some(msgs) = messages {
                if let Some(last_user) = msgs.iter().rev().find(|m| {
                    m.get("role")
                        .and_then(|r| r.as_str())
                        .map(|r| r == "user")
                        .unwrap_or(false)
                }) {
                    if let Some(content) = last_user.get("content").and_then(|c| c.as_str()) {
                        // Create a very short summary (first 100 chars)
                        signals.summary = Some(
                            content
                                .chars()
                                .take(100)
                                .collect::<String>()
                                .trim()
                                .to_string(),
                        );
                    }
                }
            }
        }
        PrivacyMode::Full => {
            // Include full system prompt and recent messages
            if let Some(msgs) = messages {
                if let Some(system_msg) = msgs.iter().find(|m| {
                    m.get("role")
                        .and_then(|r| r.as_str())
                        .map(|r| r == "system")
                        .unwrap_or(false)
                }) {
                    if let Some(content) = system_msg.get("content").and_then(|c| c.as_str()) {
                        signals.system_prompt = Some(content.to_string());
                    }
                }

                // Last 5 messages
                let recent_count = 5.min(msgs.len());
                signals.recent_messages = Some(
                    msgs.iter()
                        .rev()
                        .take(recent_count)
                        .rev()
                        .cloned()
                        .collect(),
                );
            }
        }
    }

    signals
}

/// Extract tool signals from payload
fn extract_tools(payload: &serde_json::Value) -> Vec<ToolSignal> {
    let mut tools = Vec::new();

    if let Some(tool_array) = payload.get("tools").and_then(|t| t.as_array()) {
        for tool in tool_array {
            if let Some(name) = tool
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .or_else(|| tool.get("name").and_then(|n| n.as_str()))
            {
                let schema_hash = tool
                    .get("function")
                    .and_then(|f| f.get("parameters"))
                    .or_else(|| tool.get("parameters"))
                    .map(|s| format!("sha256:{}", simple_hash(&s.to_string())));

                tools.push(ToolSignal {
                    name: name.to_string(),
                    json_schema_hash: schema_hash,
                });
            }
        }
    }

    tools
}

/// Simple hash function (not cryptographically secure, just for fingerprinting)
fn simple_hash(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Rough token estimation (4 chars per token, improved)
fn estimate_tokens(payload: &serde_json::Value) -> u32 {
    let mut total = 0;

    // Count message content tokens
    if let Some(messages) = payload
        .get("messages")
        .or_else(|| payload.get("input"))
        .and_then(|v| v.as_array())
    {
        for msg in messages {
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                total += (content.len() / 4) as u32;
            }
        }
    }

    // Add overhead for message structure (~10 tokens per message)
    if let Some(msgs) = payload
        .get("messages")
        .or_else(|| payload.get("input"))
        .and_then(|v| v.as_array())
    {
        total += (msgs.len() as u32) * 10;
    }

    // Add overhead for tools
    if let Some(tools) = payload.get("tools").and_then(|t| t.as_array()) {
        total += (tools.len() as u32) * 50; // ~50 tokens per tool definition
    }

    total.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::StatusCode;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    mod router_stub {
        use crate as routiium;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/common/router_stub.rs"
        ));
    }
    use router_stub::{sample_plan, RouterResponseConfig, RouterStub};

    fn sample_route_request(alias: &str) -> RouteRequest {
        RouteRequest {
            schema_version: Some("1.1".to_string()),
            request_id: Some("req_sample".to_string()),
            trace: None,
            alias: alias.to_string(),
            api: "responses".to_string(),
            privacy_mode: PrivacyMode::FeaturesOnly,
            content_attestation: Some(ContentAttestation {
                included: Some("none".to_string()),
            }),
            caps: vec!["text".to_string()],
            stream: false,
            params: None,
            plan_token: None,
            targets: Targets {
                p95_latency_ms: Some(30000),
                min_tokens_per_sec: None,
                reliability_tier: None,
                legacy_max_cost_usd: None,
                legacy_max_cost_gbp: None,
            },
            budget: None,
            estimates: Estimates {
                prompt_tokens: Some(10),
                max_output_tokens: Some(32),
                tokenizer_id: Some("auto".to_string()),
            },
            conversation: ConversationSignals::default(),
            org: OrgContext::default(),
            geo: None,
            tools: vec![],
            overrides: None,
            role: None,
            task: None,
            privacy: None,
            hints: None,
        }
    }

    #[test]
    fn test_route_request_serialization() {
        let req = RouteRequest {
            schema_version: Some("1.1".to_string()),
            request_id: Some("req_test".to_string()),
            trace: Some(TraceContext {
                traceparent: Some("00-abc".to_string()),
                tracestate: Some("lab=1".to_string()),
            }),
            alias: "labiium-001".to_string(),
            api: "responses".to_string(),
            privacy_mode: PrivacyMode::FeaturesOnly,
            content_attestation: Some(ContentAttestation {
                included: Some("none".to_string()),
            }),
            caps: vec!["text".to_string()],
            stream: false,
            params: None,
            plan_token: Some("plan_token".to_string()),
            targets: Targets {
                p95_latency_ms: Some(3500),
                min_tokens_per_sec: Some(250),
                reliability_tier: Some("standard".to_string()),
                ..Default::default()
            },
            budget: Some(Budget {
                amount_micro: Some(5_000),
                currency: Some("USD".to_string()),
            }),
            estimates: Estimates {
                prompt_tokens: Some(1800),
                max_output_tokens: Some(512),
                tokenizer_id: Some("o200k_base".to_string()),
            },
            conversation: ConversationSignals::default(),
            org: OrgContext::default(),
            geo: Some(GeoHint {
                region: Some("eu-west-1".to_string()),
            }),
            tools: vec![],
            overrides: None,
            role: Some("student".to_string()),
            task: None,
            privacy: None,
            hints: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("labiium-001"));
        assert!(json.contains("responses"));
    }

    #[tokio::test]
    async fn test_local_router() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "test-model".to_string(),
            UpstreamConfig {
                base_url: "http://localhost:8000/v1".to_string(),
                mode: UpstreamMode::Chat,
                model_id: "llama3".to_string(),
                auth_env: None,
                headers: None,
            },
        );

        let router = LocalPolicyRouter::new(aliases);

        let req = RouteRequest {
            schema_version: Some("1.1".to_string()),
            request_id: Some("req_local".to_string()),
            trace: None,
            alias: "test-model".to_string(),
            api: "chat".to_string(),
            privacy_mode: PrivacyMode::FeaturesOnly,
            content_attestation: Some(ContentAttestation {
                included: Some("none".to_string()),
            }),
            caps: vec![],
            stream: false,
            params: None,
            plan_token: None,
            targets: Targets::default(),
            budget: None,
            estimates: Estimates {
                tokenizer_id: Some("auto".to_string()),
                ..Default::default()
            },
            conversation: ConversationSignals::default(),
            org: OrgContext::default(),
            geo: None,
            tools: vec![],
            overrides: None,
            role: None,
            task: None,
            privacy: None,
            hints: None,
        };

        let plan = router.plan(&req).await.unwrap();
        assert_eq!(plan.upstream.model_id, "llama3");
        assert_eq!(plan.upstream.mode, UpstreamMode::Chat);
        assert_eq!(plan.schema_version.as_deref(), Some("1.1"));
        assert_eq!(
            plan.policy.as_ref().and_then(|p| p.id.as_deref()),
            Some("local_alias_policy")
        );
        assert_eq!(plan.content_used.as_deref(), Some("none"));
    }

    #[tokio::test]
    async fn test_cache() {
        let router = LocalPolicyRouter::empty();
        let cached = CachedRouterClient::new(Box::new(router), 1000);

        let req = RouteRequest {
            schema_version: Some("1.1".to_string()),
            request_id: Some("req_cache".to_string()),
            trace: None,
            alias: "test".to_string(),
            api: "chat".to_string(),
            privacy_mode: PrivacyMode::FeaturesOnly,
            content_attestation: Some(ContentAttestation {
                included: Some("none".to_string()),
            }),
            caps: vec![],
            stream: false,
            params: None,
            plan_token: None,
            targets: Targets::default(),
            budget: None,
            estimates: Estimates {
                tokenizer_id: Some("auto".to_string()),
                ..Default::default()
            },
            conversation: ConversationSignals::default(),
            org: OrgContext::default(),
            geo: None,
            tools: vec![],
            overrides: None,
            role: None,
            task: None,
            privacy: None,
            hints: None,
        };

        // Should fail (no route)
        assert!(cached.plan(&req).await.is_err());
    }

    #[test]
    fn test_privacy_modes() {
        assert_eq!(PrivacyMode::default(), PrivacyMode::FeaturesOnly);

        let payload = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are a helpful assistant"},
                {"role": "user", "content": "Hello, world!"}
            ]
        });

        // Features only - no content
        let signals = build_conversation_signals(&payload, PrivacyMode::FeaturesOnly);
        assert!(signals.system_fingerprint.is_some());
        assert!(signals.summary.is_none());
        assert!(signals.system_prompt.is_none());

        // Summary - includes short summary
        let signals = build_conversation_signals(&payload, PrivacyMode::Summary);
        assert!(signals.summary.is_some());
        assert!(signals.system_prompt.is_none());

        // Full - includes everything
        let signals = build_conversation_signals(&payload, PrivacyMode::Full);
        assert!(signals.system_prompt.is_some());
        assert!(signals.recent_messages.is_some());
    }

    #[test]
    fn test_tool_extraction() {
        let payload = serde_json::json!({
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "parameters": {"type": "object", "properties": {}}
                    }
                }
            ]
        });

        let tools = extract_tools(&payload);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "get_weather");
        assert!(tools[0].json_schema_hash.is_some());
    }

    #[test]
    fn test_extract_route_request_detects_caps_and_params() {
        let payload = serde_json::json!({
            "messages": [
                {"role": "system", "content": "Be helpful"},
                {"role": "user", "content": [
                    {"type": "text", "text": "Describe the photo"},
                    {"type": "image", "image_url": "https://example.com/cat.png"}
                ]}
            ],
            "tools": [
                {
                    "function": {
                        "name": "lookup",
                        "parameters": {"type": "object"}
                    }
                }
            ],
            "stream": true,
            "temperature": 0.3,
            "max_tokens": 256
        });

        let req = extract_route_request(
            "alias-model",
            "responses",
            &payload,
            PrivacyMode::FeaturesOnly,
        );

        assert!(req.caps.contains(&"text".to_string()));
        assert!(req.caps.contains(&"vision".to_string()));
        assert!(req.caps.contains(&"tools".to_string()));
        assert!(req.stream);
        assert_eq!(req.estimates.max_output_tokens, Some(256));
        assert!(req.estimates.prompt_tokens.unwrap_or_default() > 0);
        assert_eq!(req.schema_version.as_deref(), Some("1.1"));
        assert!(req.request_id.as_ref().unwrap().starts_with("req_"));
        assert_eq!(
            req.content_attestation
                .as_ref()
                .and_then(|c| c.included.as_deref()),
            Some("none")
        );
        assert_eq!(req.targets.p95_latency_ms, Some(30000));
        assert_eq!(req.plan_token, None);
        assert!(req.trace.is_none());
        assert_eq!(req.budget, None);
        assert_eq!(req.geo, None);
        assert_eq!(req.estimates.tokenizer_id.as_deref(), Some("auto"));

        let params = req.params.expect("params must exist");
        assert_eq!(
            params.get("temperature").and_then(|v| v.as_f64()),
            Some(0.3)
        );
        assert_eq!(
            params.get("json_mode").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[derive(Clone)]
    struct CountingRouter {
        inner: Arc<CountingRouterInner>,
    }

    struct CountingRouterInner {
        calls: Mutex<u32>,
        policy_rev: Mutex<Option<String>>,
    }

    impl CountingRouter {
        fn new(policy_rev: Option<String>) -> Self {
            Self {
                inner: Arc::new(CountingRouterInner {
                    calls: Mutex::new(0),
                    policy_rev: Mutex::new(policy_rev),
                }),
            }
        }

        fn set_policy_revision(&self, rev: Option<&str>) {
            let mut guard = self.inner.policy_rev.lock().unwrap();
            *guard = rev.map(|r| r.to_string());
        }

        fn call_count(&self) -> u32 {
            *self.inner.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl RouterClient for CountingRouter {
        async fn plan(&self, _req: &RouteRequest) -> Result<RoutePlan, RouteError> {
            let call_number = {
                let mut guard = self.inner.calls.lock().unwrap();
                *guard += 1;
                *guard
            };

            let current_revision = self.policy_revision();

            Ok(RoutePlan {
                schema_version: Some("1.1".to_string()),
                route_id: format!("rte_test_{call_number}"),
                upstream: UpstreamConfig {
                    base_url: "https://primary.example.com/v1".to_string(),
                    mode: UpstreamMode::Responses,
                    model_id: "gpt-4o-mini".to_string(),
                    auth_env: Some("OPENAI_API_KEY".to_string()),
                    headers: None,
                },
                limits: RouteLimits::default(),
                prompt_overlays: None,
                hints: RouteHints {
                    currency: Some("USD".to_string()),
                    ..Default::default()
                },
                fallbacks: vec![],
                cache: Some(CacheControl {
                    ttl_ms: 10_000,
                    etag: None,
                    valid_until: None,
                    freeze_key: None,
                }),
                policy_rev: current_revision.clone(),
                policy: Some(PolicyInfo {
                    revision: current_revision,
                    id: Some("counting_policy".to_string()),
                    explain: Some("Test router plan".to_string()),
                }),
                stickiness: Some(Stickiness {
                    plan_token: Some(format!("plan_{call_number}")),
                    max_turns: Some(3),
                    expires_at: None,
                }),
                content_used: Some("none".to_string()),
            })
        }

        fn policy_revision(&self) -> Option<String> {
            self.inner.policy_rev.lock().unwrap().clone()
        }
    }

    #[tokio::test]
    async fn test_cached_router_client_uses_cache_until_policy_revision_changes() {
        let router = CountingRouter::new(Some("rev1".to_string()));
        let cached_client = CachedRouterClient::new(Box::new(router.clone()), 10_000);

        let req = RouteRequest {
            schema_version: Some("1.1".to_string()),
            request_id: Some("req_cache_test".to_string()),
            trace: None,
            alias: "alias".to_string(),
            api: "responses".to_string(),
            privacy_mode: PrivacyMode::FeaturesOnly,
            content_attestation: Some(ContentAttestation {
                included: Some("none".to_string()),
            }),
            caps: vec!["text".to_string()],
            stream: false,
            params: None,
            plan_token: None,
            targets: Targets::default(),
            budget: None,
            estimates: Estimates {
                tokenizer_id: Some("auto".to_string()),
                ..Default::default()
            },
            conversation: ConversationSignals::default(),
            org: OrgContext::default(),
            geo: None,
            tools: vec![],
            overrides: None,
            role: None,
            task: None,
            privacy: None,
            hints: None,
        };

        let first_plan = cached_client
            .plan(&req)
            .await
            .expect("first plan must succeed");
        assert_eq!(router.call_count(), 1);

        let second_plan = cached_client
            .plan(&req)
            .await
            .expect("cached plan must succeed");
        assert_eq!(router.call_count(), 1, "plan should come from cache");
        assert_eq!(first_plan.route_id, second_plan.route_id);
        assert_eq!(first_plan.schema_version.as_deref(), Some("1.1"));
        assert!(first_plan
            .stickiness
            .as_ref()
            .and_then(|s| s.plan_token.as_deref())
            .map(|s| s.starts_with("plan_"))
            .unwrap_or(false));

        // Changing policy revision should invalidate the cache
        router.set_policy_revision(Some("rev2"));
        let third_plan = cached_client
            .plan(&req)
            .await
            .expect("plan after policy change");
        assert_eq!(router.call_count(), 2);
        assert_ne!(third_plan.route_id, second_plan.route_id);
    }

    #[test]
    fn test_router_cache_uses_plan_token_in_cache_key() {
        let cache = RouterCache::new(30_000);

        let mut base_request = RouteRequest {
            schema_version: Some("1.1".to_string()),
            request_id: Some("req_cache_key".to_string()),
            trace: None,
            alias: "alias".to_string(),
            api: "responses".to_string(),
            privacy_mode: PrivacyMode::FeaturesOnly,
            content_attestation: Some(ContentAttestation {
                included: Some("none".to_string()),
            }),
            caps: vec!["text".to_string()],
            stream: false,
            params: None,
            plan_token: Some("plan_a".to_string()),
            targets: Targets::default(),
            budget: None,
            estimates: Estimates {
                tokenizer_id: Some("auto".to_string()),
                ..Default::default()
            },
            conversation: ConversationSignals::default(),
            org: OrgContext::default(),
            geo: None,
            tools: vec![],
            overrides: None,
            role: None,
            task: None,
            privacy: None,
            hints: None,
        };

        let sample_plan = RoutePlan {
            schema_version: Some("1.1".to_string()),
            route_id: "rte_cache_test".to_string(),
            upstream: UpstreamConfig {
                base_url: "https://primary.example.com/v1".to_string(),
                mode: UpstreamMode::Responses,
                model_id: "gpt-4o-mini".to_string(),
                auth_env: Some("OPENAI_API_KEY".to_string()),
                headers: None,
            },
            limits: RouteLimits::default(),
            prompt_overlays: None,
            hints: RouteHints {
                currency: Some("USD".to_string()),
                ..Default::default()
            },
            fallbacks: vec![],
            cache: Some(CacheControl {
                ttl_ms: 30_000,
                etag: None,
                valid_until: None,
                freeze_key: None,
            }),
            policy_rev: Some("rev1".to_string()),
            policy: Some(PolicyInfo {
                revision: Some("rev1".to_string()),
                id: Some("cache_policy".to_string()),
                explain: Some("Testing cache key behaviour".to_string()),
            }),
            stickiness: Some(Stickiness {
                plan_token: Some("plan_a".to_string()),
                max_turns: Some(3),
                expires_at: None,
            }),
            content_used: Some("none".to_string()),
        };

        cache.put(&base_request, sample_plan.clone());

        // Same plan token should hit cache
        assert!(cache.get(&base_request, Some("rev1")).is_some());

        // Different plan token should miss cache due to different key
        base_request.plan_token = Some("plan_b".to_string());
        assert!(cache.get(&base_request, Some("rev1")).is_none());
    }

    #[tokio::test]
    async fn http_router_client_plan_round_trip() {
        let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(sample_plan(
            "rte_http",
            "gpt-4o-mini",
        ))))
        .await;
        let config = HttpRouterConfig {
            url: router.url(),
            timeout_ms: 200,
            mtls: false,
            client: None,
        };
        let client = HttpRouterClient::new(config).expect("http router client");
        let req = sample_route_request("nano-basic");

        let plan = client.plan(&req).await.expect("router plan");
        assert_eq!(plan.route_id, "rte_http");
        assert_eq!(plan.upstream.model_id, "gpt-4o-mini");
        assert_eq!(router.calls(), 1);

        let captured = router.take_requests();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].alias, "nano-basic");
        assert_eq!(captured[0].schema_version.as_deref(), Some("1.1"));
    }

    #[tokio::test]
    async fn http_router_client_propagates_router_errors() {
        let router = RouterStub::start(RouterResponseConfig::Error {
            status: StatusCode::CONFLICT,
            body: json!({
                "error": {
                    "code": "ALIAS_UNKNOWN",
                    "message": "alias not found"
                }
            }),
        })
        .await;

        let config = HttpRouterConfig {
            url: router.url(),
            timeout_ms: 200,
            mtls: false,
            client: None,
        };
        let client = HttpRouterClient::new(config).expect("http router client");
        let req = sample_route_request("missing-alias");

        let err = client
            .plan(&req)
            .await
            .expect_err("router should return error");
        match err {
            RouteError::RouterError(msg) => {
                assert!(
                    msg.contains("409"),
                    "expected message to mention HTTP status: {msg}"
                )
            }
            other => panic!("unexpected error variant: {:?}", other),
        }
        assert_eq!(router.calls(), 1);
    }
}
