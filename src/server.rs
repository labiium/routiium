use actix_web::http::header;
use actix_web::{web, HttpRequest, HttpResponse, HttpResponseBuilder, Responder};
use bytes::Bytes;
#[allow(unused_imports)]
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::conversion::{
    responses_chunk_to_chat_chunk, responses_to_chat_response, to_responses_request,
};
use crate::models::chat::{ChatCompletionRequest, Model, ModelsResponse};
use crate::models::responses;
use crate::router_client::{
    extract_route_request, PrivacyMode as RouterPrivacyMode, RouteError, RoutePlan,
    UpstreamMode as RouterUpstreamMode,
};
use crate::util::{env_bind_addr, managed_mode_from_env, upstream_mode_from_env, AppState};

use crate::util::error_response;
use tracing::warn;
/// Query parameters for conversion/proxy endpoints.
#[derive(Debug, Deserialize)]
pub struct ConvertQuery {
    /// Optional Responses conversation id to make the call stateful.
    pub conversation_id: Option<String>,
    /// Optional pointer to a previous Responses id (state chaining preview).
    pub previous_response_id: Option<String>,
    /// Include server-side system prompts and MCP tool metadata. Requires admin auth unless explicitly exposed.
    #[serde(default)]
    pub include_internal_config: bool,
}

/// Optional state hints accepted on `/v1/chat/completions`.
#[derive(Debug, Deserialize)]
pub struct ChatQuery {
    /// When provided, converted requests will set `conversation`.
    pub conversation_id: Option<String>,
    /// Forwarded as `previous_response_id` for Responses-compatible upstreams.
    pub previous_response_id: Option<String>,
}

#[derive(Clone)]
struct UpstreamResolution {
    base_url: String,
    mode: crate::util::UpstreamMode,
    key_env: Option<String>,
    headers: Option<HashMap<String, String>>,
    model_id: String,
    plan: Option<RoutePlan>,
    source: UpstreamSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpstreamSource {
    RouterPlan,
    RoutingConfig,
    BackendsEnv,
    Default,
}

fn mode_label(mode: crate::util::UpstreamMode) -> &'static str {
    match mode {
        crate::util::UpstreamMode::Responses => "responses",
        crate::util::UpstreamMode::Chat => "chat",
        crate::util::UpstreamMode::Bedrock => "bedrock",
    }
}

fn source_label(source: UpstreamSource) -> &'static str {
    match source {
        UpstreamSource::RouterPlan => "router",
        UpstreamSource::RoutingConfig => "routing_config",
        UpstreamSource::BackendsEnv => "backends_env",
        UpstreamSource::Default => "default",
    }
}

fn request_client_ip(req: &HttpRequest) -> String {
    req.connection_info()
        .realip_remote_addr()
        .unwrap_or("-")
        .to_string()
}

fn bearer_from_request(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            let s = s.trim();
            if s.len() >= 7 && s[..6].eq_ignore_ascii_case("bearer") {
                Some(s[6..].trim().to_string())
            } else {
                None
            }
        })
}

fn authorize_admin_request(
    req: &HttpRequest,
    expected_token: Option<&str>,
) -> Result<(), HttpResponse> {
    let Some(expected_token) = expected_token.map(str::trim).filter(|v| !v.is_empty()) else {
        if env_truthy("ROUTIIUM_INSECURE_ADMIN") {
            tracing::warn!(
                target: "routiium::auth",
                "Allowing admin request without ROUTIIUM_ADMIN_TOKEN because ROUTIIUM_INSECURE_ADMIN is enabled"
            );
            return Ok(());
        }

        return Err(HttpResponse::Unauthorized()
            .insert_header(("www-authenticate", "Bearer"))
            .json(serde_json::json!({
                "error": {
                    "message": "Admin token required. Set ROUTIIUM_ADMIN_TOKEN, or set ROUTIIUM_INSECURE_ADMIN=1 only for local throwaway development.",
                    "type": "invalid_request_error"
                }
            })));
    };

    let provided = bearer_from_request(req);
    if provided.as_deref() == Some(expected_token) {
        return Ok(());
    }

    Err(HttpResponse::Unauthorized()
        .insert_header(("www-authenticate", "Bearer"))
        .json(serde_json::json!({
            "error": {
                "message": "Unauthorized admin request",
                "type": "invalid_request_error"
            }
        })))
}

fn require_admin(req: &HttpRequest) -> Result<(), HttpResponse> {
    let expected_token = std::env::var("ROUTIIUM_ADMIN_TOKEN").ok();
    authorize_admin_request(req, expected_token.as_deref())
}

fn mcp_config_update_enabled() -> bool {
    env_truthy("ROUTIIUM_ALLOW_MCP_CONFIG_UPDATE")
}

fn managed_key_store_unavailable_response(api: &str, req: &HttpRequest) -> HttpResponse {
    tracing::error!(
        target: "routiium::auth",
        api = api,
        client = request_client_ip(req),
        "Managed auth is enabled but no API key manager is available"
    );
    error_response(
        http::StatusCode::SERVICE_UNAVAILABLE,
        "Managed auth is enabled but the API key store is unavailable",
    )
}

fn csv_escape_cell(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    let formula_prefix = matches!(
        value.as_bytes().first().copied(),
        Some(b'=') | Some(b'+') | Some(b'-') | Some(b'@')
    );
    let needs_quotes = formula_prefix
        || value.contains(',')
        || value.contains('"')
        || value.contains('\n')
        || value.contains('\r');

    if !needs_quotes {
        return value.to_string();
    }

    let mut escaped = String::with_capacity(value.len() + 3);
    escaped.push('"');
    if formula_prefix {
        escaped.push('\'');
    }
    for ch in value.chars() {
        if ch == '"' {
            escaped.push('"');
        }
        escaped.push(ch);
    }
    escaped.push('"');
    escaped
}

fn csv_row<I, S>(cells: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut row = cells
        .into_iter()
        .map(csv_escape_cell)
        .collect::<Vec<_>>()
        .join(",");
    row.push('\n');
    row
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_truthy(name: &str) -> bool {
    non_empty_env(name)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn write_pretty_json_file<T: Serialize>(path: &str, value: &T) -> std::io::Result<()> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let payload = serde_json::to_vec_pretty(value)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;
    std::fs::write(path, payload)
}

fn api_key_status(info: &crate::auth::ApiKeyInfo, now_secs: u64) -> &'static str {
    if info.revoked_at.is_some() {
        "revoked"
    } else if info.expires_at.map(|ts| ts <= now_secs).unwrap_or(false) {
        "expired"
    } else {
        "active"
    }
}

fn log_request_start(
    api: &str,
    req: &HttpRequest,
    requested_model: &str,
    resolution: &UpstreamResolution,
    stream: bool,
) {
    let route_id = resolution
        .plan
        .as_ref()
        .map(|plan| plan.route_id.as_str())
        .unwrap_or("-");
    tracing::info!(
        target: "routiium::request",
        api,
        client = request_client_ip(req),
        requested_model,
        resolved_model = %resolution.model_id,
        mode = mode_label(resolution.mode),
        source = source_label(resolution.source),
        stream,
        route_id,
        "request"
    );
}

fn router_privacy_mode_from_str(value: &str) -> RouterPrivacyMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "summary" => RouterPrivacyMode::Summary,
        "full" => RouterPrivacyMode::Full,
        _ => RouterPrivacyMode::FeaturesOnly,
    }
}

fn map_router_mode(mode: RouterUpstreamMode) -> crate::util::UpstreamMode {
    match mode {
        RouterUpstreamMode::Responses => crate::util::UpstreamMode::Responses,
        RouterUpstreamMode::Chat => crate::util::UpstreamMode::Chat,
        RouterUpstreamMode::Bedrock => crate::util::UpstreamMode::Bedrock,
    }
}

/// Record chat history for a conversation and messages
async fn record_chat_history(
    chat_history: &Option<std::sync::Arc<crate::chat_history_manager::ChatHistoryManager>>,
    conversation_id: Option<String>,
    requested_model: &str,
    request_body: &serde_json::Value,
    response_body: &serde_json::Value,
    resolution: &UpstreamResolution,
    usage: Option<&crate::analytics::TokenUsage>,
) {
    tracing::info!(
        "record_chat_history called with conversation_id: {:?}",
        conversation_id
    );
    if let Some(manager) = chat_history {
        tracing::info!("Chat history manager available, recording conversation");
        let conversation_id = conversation_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Create or update conversation
        let conversation = crate::chat_history::Conversation::new(conversation_id.clone());
        if let Err(e) = manager.record_conversation(&conversation).await {
            tracing::warn!("Failed to record conversation: {}", e);
        }

        // Extract user messages from request
        let privacy_level = manager.privacy_level();
        let mut messages_to_record = Vec::new();

        let set_routing_fields = |message: &mut crate::chat_history::Message| {
            if !requested_model.is_empty() {
                message.routing.requested_model = Some(requested_model.to_string());
            }
            message.routing.actual_model = Some(resolution.model_id.clone());
            message.routing.backend = Some(source_label(resolution.source).to_string());
            message.routing.backend_url = Some(resolution.base_url.clone());
            message.routing.upstream_mode = Some(mode_label(resolution.mode).to_string());
            message.routing.route_id = resolution.plan.as_ref().map(|p| p.route_id.clone());
        };

        // Record user/system messages from Chat Completions request payload.
        if let Some(messages) = request_body.get("messages").and_then(|v| v.as_array()) {
            for msg in messages {
                if let Some(role) = msg.get("role").and_then(|v| v.as_str()) {
                    if role == "user" || role == "system" {
                        let content = msg
                            .get("content")
                            .cloned()
                            .unwrap_or(serde_json::json!(null));
                        let role_enum = match role {
                            "system" => crate::chat_history::MessageRole::System,
                            _ => crate::chat_history::MessageRole::User,
                        };

                        let mut message = crate::chat_history::Message::new(
                            conversation_id.clone(),
                            role_enum,
                            content,
                            privacy_level,
                        );

                        set_routing_fields(&mut message);

                        messages_to_record.push(message);
                    }
                }
            }
        } else if let Some(inputs) = request_body.get("input").and_then(|v| v.as_array()) {
            // Record user/system messages from Responses API request payload.
            for input in inputs {
                if let Some(obj) = input.as_object() {
                    let role = obj
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("user")
                        .to_ascii_lowercase();
                    if role == "user" || role == "system" {
                        let role_enum = if role == "system" {
                            crate::chat_history::MessageRole::System
                        } else {
                            crate::chat_history::MessageRole::User
                        };
                        let content = obj
                            .get("content")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!(obj));
                        let mut message = crate::chat_history::Message::new(
                            conversation_id.clone(),
                            role_enum,
                            content,
                            privacy_level,
                        );
                        set_routing_fields(&mut message);
                        messages_to_record.push(message);
                    }
                } else if let Some(raw_text) = input.as_str() {
                    let mut message = crate::chat_history::Message::new(
                        conversation_id.clone(),
                        crate::chat_history::MessageRole::User,
                        serde_json::json!(raw_text),
                        privacy_level,
                    );
                    set_routing_fields(&mut message);
                    messages_to_record.push(message);
                }
            }
        }

        // Record assistant response from Chat Completions style payload.
        if let Some(choices) = response_body.get("choices").and_then(|v| v.as_array()) {
            for choice in choices {
                if let Some(msg) = choice.get("message") {
                    let content = msg
                        .get("content")
                        .cloned()
                        .unwrap_or(serde_json::json!(null));
                    let mut message = crate::chat_history::Message::new(
                        conversation_id.clone(),
                        crate::chat_history::MessageRole::Assistant,
                        content,
                        privacy_level,
                    );

                    set_routing_fields(&mut message);

                    // Add token usage
                    if let Some(usage) = usage {
                        message.tokens.input_tokens = Some(usage.prompt_tokens);
                        message.tokens.output_tokens = Some(usage.completion_tokens);
                        message.tokens.cached_tokens = usage.cached_tokens;
                        message.tokens.reasoning_tokens = usage.reasoning_tokens;
                    }

                    messages_to_record.push(message);
                }
            }
        } else {
            // Record assistant response from Responses API payloads.
            let mut recorded_from_output_items = false;
            if let Some(output_items) = response_body.get("output").and_then(|v| v.as_array()) {
                for item in output_items {
                    let content = item
                        .get("content")
                        .cloned()
                        .or_else(|| item.get("output_text").cloned());
                    if let Some(content) = content {
                        let mut message = crate::chat_history::Message::new(
                            conversation_id.clone(),
                            crate::chat_history::MessageRole::Assistant,
                            content,
                            privacy_level,
                        );
                        set_routing_fields(&mut message);
                        if let Some(usage) = usage {
                            message.tokens.input_tokens = Some(usage.prompt_tokens);
                            message.tokens.output_tokens = Some(usage.completion_tokens);
                            message.tokens.cached_tokens = usage.cached_tokens;
                            message.tokens.reasoning_tokens = usage.reasoning_tokens;
                        }
                        messages_to_record.push(message);
                        recorded_from_output_items = true;
                    }
                }
            }

            if !recorded_from_output_items {
                if let Some(output_text) = response_body.get("output_text").and_then(|v| v.as_str())
                {
                    let mut message = crate::chat_history::Message::new(
                        conversation_id.clone(),
                        crate::chat_history::MessageRole::Assistant,
                        serde_json::json!(output_text),
                        privacy_level,
                    );
                    set_routing_fields(&mut message);
                    if let Some(usage) = usage {
                        message.tokens.input_tokens = Some(usage.prompt_tokens);
                        message.tokens.output_tokens = Some(usage.completion_tokens);
                        message.tokens.cached_tokens = usage.cached_tokens;
                        message.tokens.reasoning_tokens = usage.reasoning_tokens;
                    }
                    messages_to_record.push(message);
                }
            }
        }

        // Record all messages
        if !messages_to_record.is_empty() {
            tracing::debug!("Attempting to record {} messages", messages_to_record.len());
            if let Err(e) = manager.record_messages(&messages_to_record).await {
                tracing::warn!("Failed to record messages: {}", e);
            } else {
                tracing::info!(
                    "Successfully recorded {} messages to chat history",
                    messages_to_record.len()
                );
            }
        } else {
            tracing::debug!("No messages to record");
        }
    } else {
        tracing::debug!("Chat history manager not available");
    }
}

fn request_message_count(body: &serde_json::Value) -> Option<usize> {
    body.get("messages")
        .and_then(|v| v.as_array())
        .map(|m| m.len())
        .or_else(|| {
            body.get("input")
                .and_then(|v| v.as_array())
                .map(|m| m.len())
        })
}

fn request_size_bytes(body: &serde_json::Value) -> usize {
    serde_json::to_string(body).map(|s| s.len()).unwrap_or(0)
}

fn extract_error_message(
    body: Option<&serde_json::Value>,
    fallback_bytes: &[u8],
) -> Option<String> {
    if let Some(value) = body {
        if let Some(message) = value
            .get("error")
            .and_then(|e| e.get("message").or_else(|| e.get("error")))
            .and_then(|v| v.as_str())
        {
            return Some(message.to_string());
        }
        if let Some(message) = value.get("message").and_then(|v| v.as_str()) {
            return Some(message.to_string());
        }
    }

    let text = String::from_utf8_lossy(fallback_bytes).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text.chars().take(240).collect())
    }
}

#[allow(clippy::too_many_arguments)]
async fn record_analytics_event(
    state: &AppState,
    req: &HttpRequest,
    request_body: &serde_json::Value,
    requested_model: &str,
    resolution: &UpstreamResolution,
    started_at: std::time::Instant,
    status_code: u16,
    response_size: usize,
    token_usage: Option<crate::analytics::TokenUsage>,
    authenticated: bool,
    api_key_id: Option<String>,
    api_key_label: Option<String>,
    system_prompt_applied: bool,
    error_message: Option<String>,
) {
    let Some(manager) = state.analytics.as_ref() else {
        return;
    };

    let duration_ms = started_at.elapsed().as_millis() as u64;
    let output_tokens = token_usage.as_ref().map(|u| u.completion_tokens);
    let tokens_per_second = output_tokens.and_then(|tokens| {
        if duration_ms > 0 {
            Some((tokens as f64 / duration_ms as f64) * 1000.0)
        } else {
            None
        }
    });

    let cost = token_usage.as_ref().and_then(|usage| {
        state.pricing.calculate_cost(
            &resolution.model_id,
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.cached_tokens,
            usage.reasoning_tokens,
        )
    });

    let event = crate::analytics::AnalyticsEvent {
        id: crate::analytics::generate_event_id(),
        timestamp: crate::analytics::current_timestamp(),
        request: crate::analytics::RequestMetadata {
            endpoint: req.path().to_string(),
            method: req.method().to_string(),
            model: if requested_model.is_empty() {
                None
            } else {
                Some(requested_model.to_string())
            },
            stream: request_body
                .get("stream")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            size_bytes: request_size_bytes(request_body),
            message_count: request_message_count(request_body),
            input_tokens: token_usage.as_ref().map(|u| u.prompt_tokens),
            user_agent: req
                .headers()
                .get("user-agent")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
            client_ip: Some(request_client_ip(req)),
        },
        response: Some(crate::analytics::ResponseMetadata {
            status_code,
            size_bytes: response_size,
            output_tokens,
            success: (200..300).contains(&status_code),
            error_message,
        }),
        performance: crate::analytics::PerformanceMetrics {
            duration_ms,
            ttfb_ms: None,
            upstream_duration_ms: None,
            tokens_per_second,
        },
        auth: crate::analytics::AuthMetadata {
            authenticated,
            api_key_id,
            api_key_label,
            auth_method: if authenticated {
                Some("bearer".to_string())
            } else {
                None
            },
        },
        routing: crate::analytics::RoutingMetadata {
            backend: source_label(resolution.source).to_string(),
            upstream_mode: mode_label(resolution.mode).to_string(),
            mcp_enabled: state.mcp_manager.is_some(),
            mcp_servers: Vec::new(),
            system_prompt_applied,
        },
        token_usage,
        cost,
    };

    if let Err(err) = manager.record(event).await {
        tracing::warn!("Failed to record analytics event: {}", err);
    }
}

async fn record_response_guard_event(
    state: &AppState,
    req: &HttpRequest,
    endpoint: &str,
    requested_model: &str,
    resolution: &UpstreamResolution,
    decision: &crate::safety_judge::ResponseGuardDecision,
) {
    state
        .safety_audit
        .record(crate::safety_audit::SafetyAuditEventBuilder {
            kind: "response_guard_block".to_string(),
            endpoint: endpoint.to_string(),
            client_ip: Some(request_client_ip(req)),
            requested_model: (!requested_model.is_empty()).then(|| requested_model.to_string()),
            resolved_model: Some(resolution.model_id.clone()),
            route_id: resolution.plan.as_ref().map(|plan| plan.route_id.clone()),
            action: None,
            target_alias: None,
            verdict: Some(decision.verdict.clone()),
            risk_level: Some(decision.risk_level.clone()),
            reason: Some(decision.reason.clone()),
            categories: decision.categories.clone(),
            policy_rev: Some(decision.policy_rev.clone()),
            policy_fingerprint: None,
        })
        .await;
}

async fn record_router_error_event(
    state: &AppState,
    req: &HttpRequest,
    endpoint: &str,
    requested_model: &str,
    err: &RouteError,
) {
    let (
        kind,
        action,
        target_alias,
        verdict,
        risk_level,
        reason,
        categories,
        policy_rev,
        policy_fingerprint,
    ) = match err {
        RouteError::Rejected {
            message,
            policy_rev,
            body,
            ..
        } => {
            let judge = body
                .as_ref()
                .and_then(|body| body.get("error"))
                .and_then(|error| error.get("judge"));
            let categories = judge
                .and_then(|judge| judge.get("categories"))
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (
                "router_rejected".to_string(),
                judge
                    .and_then(|judge| judge.get("action"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                judge
                    .and_then(|judge| judge.get("target"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                judge
                    .and_then(|judge| judge.get("verdict"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                judge
                    .and_then(|judge| judge.get("risk_level"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                Some(message.clone()),
                categories,
                policy_rev.clone(),
                judge
                    .and_then(|judge| judge.get("policy_fingerprint"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            )
        }
        other => (
            "router_error".to_string(),
            None,
            None,
            None,
            None,
            Some(other.to_string()),
            Vec::new(),
            None,
            None,
        ),
    };

    state
        .safety_audit
        .record(crate::safety_audit::SafetyAuditEventBuilder {
            kind,
            endpoint: endpoint.to_string(),
            client_ip: Some(request_client_ip(req)),
            requested_model: (!requested_model.is_empty()).then(|| requested_model.to_string()),
            resolved_model: None,
            route_id: None,
            action,
            target_alias,
            verdict,
            risk_level,
            reason,
            categories,
            policy_rev,
            policy_fingerprint,
        })
        .await;
}

fn judge_route_requires_tool_stripping(plan: &RoutePlan) -> bool {
    plan.judge
        .as_ref()
        .map(|judge| {
            judge.action.as_deref() == Some("route")
                && matches!(
                    judge.verdict.as_deref(),
                    Some("deny") | Some("needs_approval")
                )
        })
        .unwrap_or(false)
}

fn strip_high_risk_tools(body: &mut serde_json::Value) {
    if let Some(obj) = body.as_object_mut() {
        obj.remove("tools");
        obj.remove("tool_choice");
        obj.remove("parallel_tool_calls");
    }
}

async fn resolve_upstream(
    state: &AppState,
    api: &str,
    body: &mut serde_json::Value,
) -> Result<UpstreamResolution, RouteError> {
    let requested_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let strict_mode = state.router_strict || env_truthy("ROUTIIUM_ROUTER_STRICT");

    if let Some(router) = state.router_client.as_ref() {
        if !requested_model.is_empty() {
            let privacy_mode = router_privacy_mode_from_str(&state.router_privacy_mode);
            let route_request =
                extract_route_request(requested_model.as_str(), api, body, privacy_mode);
            match router.plan(&route_request).await {
                Ok(mut plan) => {
                    if judge_route_requires_tool_stripping(&plan) {
                        strip_high_risk_tools(body);
                        if let Some(judge) = plan.judge.as_mut() {
                            let mut categories = judge.categories.take().unwrap_or_default();
                            if !categories
                                .iter()
                                .any(|category| category == "tools_stripped")
                            {
                                categories.push("tools_stripped".to_string());
                            }
                            judge.categories = Some(categories);
                            judge.cacheable = Some(false);
                        }
                    }
                    let model_id = plan.upstream.model_id.clone();
                    return Ok(UpstreamResolution {
                        base_url: plan.upstream.base_url.clone(),
                        mode: map_router_mode(plan.upstream.mode),
                        key_env: plan.upstream.auth_env.clone(),
                        headers: plan.upstream.headers.clone(),
                        model_id,
                        plan: Some(plan),
                        source: UpstreamSource::RouterPlan,
                    });
                }
                Err(e) => {
                    if strict_mode || matches!(e, RouteError::Rejected { .. }) {
                        return Err(e);
                    }
                    tracing::debug!(
                        "Router plan unavailable for alias {} (api={}): {}; falling back to legacy routing",
                        requested_model,
                        api,
                        e
                    );
                }
            }
        }
    }

    // Try routing config first (if loaded and actually has a match/default)
    let routing_guard = state.routing_config.read().await;
    let resolved_alias = if !requested_model.is_empty() {
        routing_guard.resolve_alias(requested_model.as_str())
    } else {
        String::new()
    };
    let has_rule =
        !requested_model.is_empty() && routing_guard.find_rule(resolved_alias.as_str()).is_some();
    let has_default = routing_guard.default_backend.is_some();
    if has_rule || has_default {
        if let Ok(route) = routing_guard.resolve_route(requested_model.as_str()) {
            // Apply alias + transform chain from routing.json so runtime behavior matches config.
            let resolved_model = if !requested_model.is_empty() {
                match routing_guard.apply_transformations(requested_model.as_str(), body) {
                    Ok(model) => model,
                    Err(e) => {
                        if strict_mode {
                            return Err(RouteError::InvalidRequest(format!(
                                "routing transform failed: {}",
                                e
                            )));
                        }
                        tracing::warn!("routing transform failed, continuing: {}", e);
                        resolved_alias.clone()
                    }
                }
            } else {
                std::env::var("MODEL").unwrap_or_else(|_| "gpt-5-nano".to_string())
            };
            drop(routing_guard);
            // Convert routing_config::UpstreamMode to util::UpstreamMode
            let mode = match route.mode {
                crate::routing_config::UpstreamMode::Responses => {
                    crate::util::UpstreamMode::Responses
                }
                crate::routing_config::UpstreamMode::Chat => crate::util::UpstreamMode::Chat,
                crate::routing_config::UpstreamMode::Bedrock => crate::util::UpstreamMode::Bedrock,
            };
            return Ok(UpstreamResolution {
                base_url: route.base_url.clone(),
                mode,
                key_env: route.key_env.clone(),
                headers: None,
                model_id: resolved_model,
                plan: None,
                source: UpstreamSource::RoutingConfig,
            });
        }
    }
    drop(routing_guard);

    // Fallback to legacy prefix-based routing via ROUTIIUM_BACKENDS env var
    let mut base_url: Option<String> = None;
    let mut mode: Option<crate::util::UpstreamMode> = None;
    let mut key_env: Option<String> = None;
    let mut source = UpstreamSource::Default;
    if !requested_model.is_empty() {
        if let Some((bu, ke, m)) =
            crate::util::resolve_backend_for_model_name(requested_model.as_str())
        {
            base_url = Some(bu);
            key_env = ke;
            mode = Some(m);
            source = UpstreamSource::BackendsEnv;
        }
    }

    let resolved_model = if !requested_model.is_empty() {
        if !resolved_alias.is_empty() {
            resolved_alias
        } else {
            requested_model
        }
    } else {
        std::env::var("MODEL").unwrap_or_else(|_| "gpt-5-nano".to_string())
    };

    let fallback_mode = mode.unwrap_or_else(|| {
        if api.eq_ignore_ascii_case("chat") {
            crate::util::UpstreamMode::Chat
        } else {
            crate::util::upstream_mode_from_env()
        }
    });

    Ok(UpstreamResolution {
        base_url: base_url.unwrap_or_else(crate::util::openai_base_url),
        mode: fallback_mode,
        key_env,
        headers: None,
        model_id: resolved_model,
        plan: None,
        source,
    })
}

/// Add X-RateLimit-* headers to an in-progress response builder.
fn insert_rate_limit_headers(
    builder: &mut HttpResponseBuilder,
    result: &crate::rate_limit::RateLimitCheckResult,
) {
    for (k, v) in crate::rate_limit::RateLimitManager::rate_limit_headers(result) {
        builder.insert_header((k, v));
    }
}

/// Perform rate limit check and return 429 / block responses early.
/// Returns `Ok(Some(result))` when allowed, `Ok(None)` when no RL configured,
/// and an `Err` HttpResponse when the request should be rejected immediately.
async fn check_rate_limits_for_key(
    state: &AppState,
    key_id: Option<&str>,
    path: &str,
    model: Option<&str>,
) -> Result<Option<crate::rate_limit::RateLimitCheckResult>, HttpResponse> {
    let Some(ref rl_manager) = state.rate_limit_manager else {
        return Ok(None);
    };
    let Some(kid) = key_id else {
        return Ok(None);
    };
    match rl_manager.check_rate_limit(kid, path, model).await {
        Ok(result) => {
            if !result.allowed {
                let rejected = result.rejected_bucket.as_ref().unwrap();
                let now_s = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let retry_after = rejected.reset_at.saturating_sub(now_s);
                return Err(HttpResponse::TooManyRequests()
                    .insert_header(("Retry-After", retry_after.to_string()))
                    .insert_header(("X-RateLimit-Limit", rejected.limit.to_string()))
                    .insert_header(("X-RateLimit-Remaining", "0".to_string()))
                    .insert_header(("X-RateLimit-Reset", rejected.reset_at.to_string()))
                    .json(serde_json::json!({
                        "error": "rate_limit_exceeded",
                        "message": format!(
                            "Rate limit '{}' exceeded. Limit: {} per {} seconds.",
                            rejected.name, rejected.limit, rejected.window_seconds
                        ),
                        "limit": rejected.name,
                        "limit_value": rejected.limit,
                        "window_seconds": rejected.window_seconds,
                        "retry_after": retry_after,
                        "reset_at": rejected.reset_at,
                    })));
            }
            Ok(Some(result))
        }
        Err(e) => {
            let reason = e.to_string();
            if reason.starts_with("BLOCKED") {
                let msg = reason
                    .trim_start_matches("BLOCKED:")
                    .trim_start_matches("BLOCKED")
                    .trim();
                return Err(HttpResponse::TooManyRequests().json(serde_json::json!({
                    "error": "key_blocked",
                    "message": format!(
                        "API key is blocked: {}",
                        if msg.is_empty() { "abuse detected" } else { msg }
                    ),
                })));
            }
            // Store error is non-fatal; allow the request.
            tracing::warn!("Rate limit store error: {}", e);
            Ok(None)
        }
    }
}

fn insert_route_headers(builder: &mut HttpResponseBuilder, plan: &RoutePlan, resolved_model: &str) {
    builder.insert_header(("x-route-id", plan.route_id.clone()));
    builder.insert_header(("x-resolved-model", resolved_model.to_string()));
    if let Some(schema) = plan.schema_version.as_deref() {
        builder.insert_header(("router-schema", schema.to_string()));
    }
    if let Some(policy) = plan.policy_rev.as_deref() {
        builder.insert_header(("x-policy-rev", policy.to_string()));
    }
    if let Some(content_used) = plan.content_used.as_deref() {
        builder.insert_header(("x-content-used", content_used.to_string()));
    }
    if let Some(judge) = plan.judge.as_ref() {
        if let Some(id) = judge.id.as_deref() {
            builder.insert_header(("x-judge-id", id.to_string()));
        }
        if let Some(action) = judge.action.as_deref() {
            builder.insert_header(("x-judge-action", action.to_string()));
        }
        if let Some(mode) = judge.mode.as_deref() {
            builder.insert_header(("x-judge-mode", mode.to_string()));
        }
        if let Some(verdict) = judge.verdict.as_deref() {
            builder.insert_header(("x-judge-verdict", verdict.to_string()));
        }
        if let Some(risk_level) = judge.risk_level.as_deref() {
            builder.insert_header(("x-judge-risk", risk_level.to_string()));
        }
        if let Some(target) = judge.target.as_deref() {
            builder.insert_header(("x-judge-target", target.to_string()));
        }
        if let Some(policy_rev) = judge.policy_rev.as_deref() {
            builder.insert_header(("x-safety-policy-rev", policy_rev.to_string()));
        }
        if let Some(policy_fingerprint) = judge.policy_fingerprint.as_deref() {
            builder.insert_header(("x-judge-policy-fingerprint", policy_fingerprint.to_string()));
        }
        if let Some(cacheable) = judge.cacheable {
            builder.insert_header((
                "x-safety-cache",
                if cacheable { "cacheable" } else { "no-store" },
            ));
        }
    }
}

fn insert_response_guard_headers(
    builder: &mut HttpResponseBuilder,
    decision: &crate::safety_judge::ResponseGuardDecision,
) {
    builder.insert_header(("x-response-guard-id", decision.id.clone()));
    builder.insert_header(("x-response-guard-mode", decision.mode.clone()));
    builder.insert_header(("x-response-guard-verdict", decision.verdict.clone()));
    builder.insert_header(("x-response-guard-risk", decision.risk_level.clone()));
    builder.insert_header(("x-response-guard-blocked", decision.blocked.to_string()));
    builder.insert_header(("x-safety-policy-rev", decision.policy_rev.clone()));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RejectionMode {
    AgentResult,
    HttpError,
}

fn rejection_mode_from_env() -> RejectionMode {
    match std::env::var("ROUTIIUM_REJECTION_MODE")
        .unwrap_or_else(|_| "agent_result".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "http_error" | "http-error" | "error" | "strict" | "403" => RejectionMode::HttpError,
        _ => RejectionMode::AgentResult,
    }
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn short_uuid(prefix: &str) -> String {
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}{}", &uuid[..16])
}

fn insert_judge_headers_from_value(
    builder: &mut HttpResponseBuilder,
    judge: Option<&serde_json::Value>,
) {
    let Some(judge) = judge else {
        return;
    };
    for (field, header) in [
        ("id", "x-judge-id"),
        ("action", "x-judge-action"),
        ("mode", "x-judge-mode"),
        ("verdict", "x-judge-verdict"),
        ("risk_level", "x-judge-risk"),
        ("target", "x-judge-target"),
        ("policy_rev", "x-safety-policy-rev"),
        ("policy_fingerprint", "x-judge-policy-fingerprint"),
    ] {
        if let Some(value) = judge.get(field).and_then(|value| value.as_str()) {
            builder.insert_header((header, value.to_string()));
        }
    }
    if let Some(cacheable) = judge.get("cacheable").and_then(|value| value.as_bool()) {
        builder.insert_header((
            "x-safety-cache",
            if cacheable { "cacheable" } else { "no-store" },
        ));
    }
}

fn rejection_text(reason: &str, categories: &[String]) -> String {
    if categories.is_empty() {
        format!("Request rejected by Routiium safety policy: {reason}")
    } else {
        format!(
            "Request rejected by Routiium safety policy: {reason} Categories: {}.",
            categories.join(", ")
        )
    }
}

fn agent_rejection_body(
    api: &str,
    model: &str,
    text: &str,
    metadata: serde_json::Value,
) -> serde_json::Value {
    let model = if model.trim().is_empty() {
        "routiium-safety"
    } else {
        model
    };
    let created = unix_timestamp();
    match api {
        "chat" | "chat/completions" => serde_json::json!({
            "id": short_uuid("chatcmpl_rej_"),
            "object": "chat.completion",
            "created": created,
            "model": model,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": text
                },
                "finish_reason": "content_filter"
            }],
            "usage": {
                "prompt_tokens": 0,
                "completion_tokens": 0,
                "total_tokens": 0
            },
            "routiium_rejection": metadata
        }),
        _ => serde_json::json!({
            "id": short_uuid("resp_rej_"),
            "object": "response",
            "created": created,
            "model": model,
            "output_text": text,
            "output": [{
                "type": "assistant_message",
                "id": short_uuid("msg_rej_"),
                "content": text
            }],
            "usage": {
                "input_tokens": 0,
                "output_tokens": 0,
                "total_tokens": 0
            },
            "routiium_rejection": metadata
        }),
    }
}

fn response_guard_error_response(
    decision: &crate::safety_judge::ResponseGuardDecision,
    plan: Option<&RoutePlan>,
    resolved_model: &str,
    api: &str,
) -> HttpResponse {
    if rejection_mode_from_env() == RejectionMode::AgentResult {
        let mut builder = HttpResponse::Ok();
        if let Some(plan) = plan {
            insert_route_headers(&mut builder, plan, resolved_model);
        }
        insert_response_guard_headers(&mut builder, decision);
        builder.insert_header(("x-judge-action", "reject"));
        let metadata = serde_json::json!({
            "source": "response_guard",
            "action": "reject",
            "verdict": decision.verdict,
            "risk_level": decision.risk_level,
            "reason": decision.reason,
            "categories": decision.categories,
            "policy_rev": decision.policy_rev,
            "safety_event_id": decision.id,
        });
        let text = rejection_text(&decision.reason, &decision.categories);
        return builder.json(agent_rejection_body(api, resolved_model, &text, metadata));
    }

    let mut builder = HttpResponse::Forbidden();
    if let Some(plan) = plan {
        insert_route_headers(&mut builder, plan, resolved_model);
    }
    insert_response_guard_headers(&mut builder, decision);
    builder.json(serde_json::json!({
        "error": {
            "message": "Response blocked by Routiium response guard",
            "type": "safety_policy_error",
            "code": "response_guard_blocked",
            "safety_event_id": decision.id,
            "reason": decision.reason,
            "risk_level": decision.risk_level,
            "categories": decision.categories,
            "policy_rev": decision.policy_rev
        }
    }))
}

fn router_error_response(
    status: http::StatusCode,
    message: &str,
    plan: Option<&RoutePlan>,
    resolved_model: &str,
) -> HttpResponse {
    let mut builder =
        HttpResponse::build(actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap());
    if let Some(plan) = plan {
        insert_route_headers(&mut builder, plan, resolved_model);
    }
    let body = serde_json::json!({ "error": { "message": message } });
    builder.json(body)
}

fn router_plan_error_response(err: &RouteError) -> HttpResponse {
    match err {
        RouteError::Rejected {
            status,
            code,
            message,
            policy_rev,
            retry_hint_ms,
            body,
        } => {
            let status = actix_web::http::StatusCode::from_u16(*status)
                .unwrap_or(actix_web::http::StatusCode::BAD_GATEWAY);
            let mut builder = HttpResponse::build(status);
            if let Some(retry_hint_ms) = retry_hint_ms {
                builder.insert_header(("retry-after", retry_hint_ms.div_ceil(1000).to_string()));
            }
            if let Some(body) = body {
                builder.json(body)
            } else {
                let mut error = serde_json::Map::new();
                if let Some(code) = code {
                    error.insert("code".to_string(), serde_json::json!(code));
                }
                error.insert("message".to_string(), serde_json::json!(message));
                if let Some(policy_rev) = policy_rev {
                    error.insert("policy_rev".to_string(), serde_json::json!(policy_rev));
                }
                if let Some(retry_hint_ms) = retry_hint_ms {
                    error.insert(
                        "retry_hint_ms".to_string(),
                        serde_json::json!(retry_hint_ms),
                    );
                }
                builder.json(serde_json::json!({ "error": error }))
            }
        }
        RouteError::NoRoute(message) => HttpResponse::NotFound().json(serde_json::json!({
            "error": {
                "code": "NO_ROUTE",
                "message": message
            }
        })),
        RouteError::InvalidRequest(message) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": {
                "code": "INVALID_REQUEST",
                "message": message
            }
        })),
        RouteError::Timeout(message)
        | RouteError::Unavailable(message)
        | RouteError::NetworkError(message) => HttpResponse::BadGateway().json(serde_json::json!({
            "error": {
                "code": "ROUTER_UNAVAILABLE",
                "message": message
            }
        })),
        RouteError::RouterError(message) => HttpResponse::BadGateway().json(serde_json::json!({
            "error": {
                "code": "ROUTER_ERROR",
                "message": message
            }
        })),
    }
}

fn router_plan_rejection_response(
    err: &RouteError,
    api: &str,
    requested_model: &str,
) -> Option<HttpResponse> {
    if rejection_mode_from_env() == RejectionMode::HttpError {
        return None;
    }

    let RouteError::Rejected {
        status,
        code,
        message,
        policy_rev,
        body,
        ..
    } = err
    else {
        return None;
    };

    if *status != 403 {
        return None;
    }

    let body_error = body
        .as_ref()
        .and_then(|body| body.get("error"))
        .or(body.as_ref());
    let judge = body_error.and_then(|error| error.get("judge"));
    let code_value = code
        .as_deref()
        .or_else(|| {
            body_error
                .and_then(|error| error.get("code"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("POLICY_REJECT");
    if judge.is_none()
        && !matches!(
            code_value,
            "POLICY_DENY" | "POLICY_REJECT" | "APPROVAL_REQUIRED"
        )
    {
        return None;
    }

    let reason = body_error
        .and_then(|error| error.get("message"))
        .and_then(|value| value.as_str())
        .unwrap_or(message);
    let categories = judge
        .and_then(|judge| judge.get("categories"))
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let risk_level = judge
        .and_then(|judge| judge.get("risk_level"))
        .and_then(|value| value.as_str())
        .unwrap_or("high");
    let action = judge
        .and_then(|judge| judge.get("action"))
        .and_then(|value| value.as_str())
        .map(|action| {
            if action == "needs_approval" {
                "reject"
            } else {
                action
            }
        })
        .unwrap_or("reject");
    let verdict = judge
        .and_then(|judge| judge.get("verdict"))
        .and_then(|value| value.as_str())
        .map(|verdict| {
            if verdict == "needs_approval" {
                "deny"
            } else {
                verdict
            }
        })
        .unwrap_or("deny");
    let judge_id = judge
        .and_then(|judge| judge.get("id"))
        .and_then(|value| value.as_str());
    let policy_fingerprint = judge
        .and_then(|judge| judge.get("policy_fingerprint"))
        .and_then(|value| value.as_str());
    let policy_rev = policy_rev.as_deref().or_else(|| {
        judge
            .and_then(|judge| judge.get("policy_rev"))
            .and_then(|v| v.as_str())
    });

    let metadata = serde_json::json!({
        "source": "request_judge",
        "action": action,
        "verdict": verdict,
        "risk_level": risk_level,
        "reason": reason,
        "categories": categories,
        "policy_rev": policy_rev,
        "policy_fingerprint": policy_fingerprint,
        "judge_id": judge_id,
        "code": if code_value == "APPROVAL_REQUIRED" { "POLICY_REJECT" } else { code_value },
    });
    let text = rejection_text(reason, &categories);
    let mut builder = HttpResponse::Ok();
    insert_judge_headers_from_value(&mut builder, judge);
    builder.insert_header(("x-judge-action", action.to_string()));
    builder.insert_header(("x-judge-verdict", verdict.to_string()));
    builder.insert_header(("x-judge-risk", risk_level.to_string()));
    if let Some(policy_rev) = policy_rev {
        builder.insert_header(("x-safety-policy-rev", policy_rev.to_string()));
    }
    Some(builder.json(agent_rejection_body(api, requested_model, &text, metadata)))
}

fn extract_conversation_id(value: &serde_json::Value) -> Option<String> {
    use serde_json::Value;

    if let Some(conv) = value.get("conversation") {
        match conv {
            Value::String(s) if !s.trim().is_empty() => return Some(s.clone()),
            Value::Object(map) => {
                if let Some(id) = map.get("id").and_then(|v| v.as_str()) {
                    if !id.trim().is_empty() {
                        return Some(id.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    value
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            value
                .get("previous_response_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

fn extract_previous_response_id(value: &serde_json::Value) -> Option<String> {
    value
        .get("previous_response_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn convert_chat_payload_to_responses(
    payload: &serde_json::Value,
    conversation_hint: Option<String>,
    previous_response_hint: Option<String>,
) -> Option<serde_json::Value> {
    let mut conversation = conversation_hint.or_else(|| extract_conversation_id(payload));
    if let Some(conv) = conversation.as_ref() {
        if conv.trim().is_empty() {
            conversation = None;
        }
    }

    let mut previous_response =
        previous_response_hint.or_else(|| extract_previous_response_id(payload));
    if let Some(prev) = previous_response.as_ref() {
        if prev.trim().is_empty() {
            previous_response = None;
        }
    }

    serde_json::from_value::<ChatCompletionRequest>(payload.clone())
        .ok()
        .and_then(|req| {
            let mut responses_req = to_responses_request(&req, conversation);
            if let Some(prev) = previous_response {
                responses_req.previous_response_id = Some(prev);
            }
            serde_json::to_value(responses_req).ok()
        })
}

fn strip_responses_only_fields(payload: &mut serde_json::Value) {
    if let Some(obj) = payload.as_object_mut() {
        obj.remove("conversation");
        obj.remove("conversation_id");
        obj.remove("previous_response_id");
    }
}

fn collect_existing_tool_names(
    tools: &[serde_json::Value],
    prefer_nested_function_name: bool,
) -> HashSet<String> {
    let mut names = HashSet::new();

    for tool in tools {
        let Some(obj) = tool.as_object() else {
            continue;
        };

        let nested_name = obj
            .get("function")
            .and_then(|f| f.as_object())
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str());
        let direct_name = obj.get("name").and_then(|n| n.as_str());
        let selected = if prefer_nested_function_name {
            nested_name.or(direct_name)
        } else {
            direct_name.or(nested_name)
        };
        if let Some(name) = selected {
            names.insert(name.to_string());
        }
    }

    names
}

fn merge_mcp_tools_into_chat_payload(
    payload: &mut serde_json::Value,
    mcp_tools: &[crate::mcp_client::McpTool],
) {
    let Some(obj) = payload.as_object_mut() else {
        return;
    };
    let tools_value = obj
        .entry("tools".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let Some(tools_array) = tools_value.as_array_mut() else {
        return;
    };

    let mut existing_names = collect_existing_tool_names(tools_array, true);

    for tool in mcp_tools {
        let combined_name = format!("{}_{}", tool.server_name, tool.name);
        if !existing_names.insert(combined_name.clone()) {
            continue;
        }

        tools_array.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": combined_name,
                "description": tool.description.as_deref().unwrap_or("MCP tool"),
                "parameters": tool.input_schema.clone()
            }
        }));
    }
}

fn merge_mcp_tools_into_responses_payload(
    payload: &mut serde_json::Value,
    mcp_tools: &[crate::mcp_client::McpTool],
) {
    let Some(obj) = payload.as_object_mut() else {
        return;
    };
    let tools_value = obj
        .entry("tools".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let Some(tools_array) = tools_value.as_array_mut() else {
        return;
    };

    let mut existing_names = collect_existing_tool_names(tools_array, false);

    for tool in mcp_tools {
        let combined_name = format!("{}_{}", tool.server_name, tool.name);
        if !existing_names.insert(combined_name.clone()) {
            continue;
        }

        tools_array.push(serde_json::json!({
            "type": "function",
            "name": combined_name,
            "description": tool.description.as_deref().unwrap_or("MCP tool"),
            "parameters": tool.input_schema.clone()
        }));
    }
}

fn inject_system_prompt_chat_json(payload: &mut serde_json::Value, prompt: &str, mode: &str) {
    let Some(messages) = payload.get_mut("messages").and_then(|v| v.as_array_mut()) else {
        return;
    };

    let is_system = |msg: &serde_json::Value| {
        msg.get("role")
            .and_then(|v| v.as_str())
            .map(|role| role == "system")
            .unwrap_or(false)
    };

    let system_message = serde_json::json!({
        "role": "system",
        "content": prompt
    });

    match mode {
        "append" => {
            let mut last_system_pos: Option<usize> = None;
            for (idx, msg) in messages.iter().enumerate() {
                if is_system(msg) {
                    last_system_pos = Some(idx);
                }
            }
            if let Some(pos) = last_system_pos {
                messages.insert(pos + 1, system_message);
            } else {
                messages.push(system_message);
            }
        }
        "replace" => {
            messages.retain(|m| !is_system(m));
            messages.insert(0, system_message);
        }
        _ => {
            messages.insert(0, system_message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_history::{MessageFilters, MessageRole, PrivacyLevel};
    use crate::chat_history_manager::{ChatHistoryConfig, ChatHistoryManager};
    use actix_web::test::TestRequest;
    use serde_json::json;
    use std::sync::Arc;

    struct EnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn new(keys: &[&'static str]) -> Self {
            let saved = keys
                .iter()
                .map(|key| (*key, std::env::var(key).ok()))
                .collect();
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn extract_conversation_supports_object_form() {
        let payload = json!({
            "conversation": { "id": "conv-body" },
            "previous_response_id": "resp-body"
        });
        assert_eq!(
            extract_conversation_id(&payload),
            Some("conv-body".to_string())
        );
    }

    #[test]
    fn convert_chat_payload_prefers_query_hints() {
        let payload = json!({
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "Hi"}],
            "conversation": {"id": "conv-body"},
            "previous_response_id": "resp-body"
        });
        let converted = convert_chat_payload_to_responses(
            &payload,
            Some("conv-query".into()),
            Some("resp-query".into()),
        )
        .expect("conversion succeeds");
        assert_eq!(converted["conversation"], "conv-query");
        assert_eq!(converted["previous_response_id"], "resp-query");
    }

    #[test]
    fn strip_responses_fields_removes_all_supported_keys() {
        let mut payload = json!({
            "model": "gpt-4o-mini",
            "messages": [],
            "conversation": {"id": "conv"},
            "conversation_id": "conv",
            "previous_response_id": "resp"
        });
        strip_responses_only_fields(&mut payload);
        assert!(payload.get("conversation").is_none());
        assert!(payload.get("conversation_id").is_none());
        assert!(payload.get("previous_response_id").is_none());
    }

    #[test]
    fn merge_mcp_tools_into_chat_payload_dedupes_existing_names() {
        let mut payload = json!({
            "model": "gpt-4o-mini",
            "messages": [{"role":"user","content":"hi"}],
            "tools": [
                {"type":"function","function":{"name":"local_tool","description":"local","parameters":{"type":"object","properties":{}}}},
                {"type":"function","function":{"name":"mock_echo","description":"existing","parameters":{"type":"object","properties":{}}}}
            ]
        });

        let mcp_tools = vec![
            crate::mcp_client::McpTool {
                server_name: "mock".to_string(),
                name: "echo".to_string(),
                description: Some("Echo text".to_string()),
                input_schema: json!({"type":"object","properties":{"text":{"type":"string"}}}),
            },
            crate::mcp_client::McpTool {
                server_name: "mock".to_string(),
                name: "sum".to_string(),
                description: Some("Sum values".to_string()),
                input_schema: json!({"type":"object","properties":{"a":{"type":"number"},"b":{"type":"number"}}}),
            },
        ];

        merge_mcp_tools_into_chat_payload(&mut payload, &mcp_tools);

        let tools = payload["tools"].as_array().expect("tools array");
        let names: Vec<String> = tools
            .iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect();

        assert!(names.contains(&"local_tool".to_string()));
        assert!(names.contains(&"mock_echo".to_string()));
        assert!(names.contains(&"mock_sum".to_string()));
        assert_eq!(
            names.iter().filter(|n| n.as_str() == "mock_echo").count(),
            1
        );
    }

    #[test]
    fn merge_mcp_tools_into_responses_payload_dedupes_existing_names() {
        let mut payload = json!({
            "model": "gpt-4.1-nano",
            "input": [{"role":"user","content":"hi"}],
            "tools": [
                {"type":"function","name":"local_tool","description":"local","parameters":{"type":"object","properties":{}}},
                {"type":"function","name":"mock_echo","description":"existing","parameters":{"type":"object","properties":{}}}
            ]
        });

        let mcp_tools = vec![
            crate::mcp_client::McpTool {
                server_name: "mock".to_string(),
                name: "echo".to_string(),
                description: Some("Echo text".to_string()),
                input_schema: json!({"type":"object","properties":{"text":{"type":"string"}}}),
            },
            crate::mcp_client::McpTool {
                server_name: "mock".to_string(),
                name: "sum".to_string(),
                description: Some("Sum values".to_string()),
                input_schema: json!({"type":"object","properties":{"a":{"type":"number"},"b":{"type":"number"}}}),
            },
        ];

        merge_mcp_tools_into_responses_payload(&mut payload, &mcp_tools);

        let tools = payload["tools"].as_array().expect("tools array");
        let names: Vec<String> = tools
            .iter()
            .filter_map(|t| {
                t.get("name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect();

        assert!(names.contains(&"local_tool".to_string()));
        assert!(names.contains(&"mock_echo".to_string()));
        assert!(names.contains(&"mock_sum".to_string()));
        assert_eq!(
            names.iter().filter(|n| n.as_str() == "mock_echo").count(),
            1
        );
    }

    #[test]
    fn admin_auth_rejects_when_token_unset_by_default() {
        let req = TestRequest::default().to_http_request();
        let _env = EnvGuard::new(&["ROUTIIUM_INSECURE_ADMIN"]);
        std::env::remove_var("ROUTIIUM_INSECURE_ADMIN");
        assert!(authorize_admin_request(&req, None).is_err());
    }

    #[test]
    fn admin_auth_allows_without_token_only_when_explicitly_insecure() {
        let _env = EnvGuard::new(&["ROUTIIUM_INSECURE_ADMIN"]);
        std::env::set_var("ROUTIIUM_INSECURE_ADMIN", "1");
        let req = TestRequest::default().to_http_request();
        assert!(authorize_admin_request(&req, None).is_ok());
    }

    #[test]
    fn admin_auth_requires_matching_bearer() {
        let req = TestRequest::default().to_http_request();
        assert!(authorize_admin_request(&req, Some("secret-token")).is_err());

        let bad = TestRequest::default()
            .insert_header(("Authorization", "Bearer wrong-token"))
            .to_http_request();
        assert!(authorize_admin_request(&bad, Some("secret-token")).is_err());

        let good = TestRequest::default()
            .insert_header(("Authorization", "Bearer secret-token"))
            .to_http_request();
        assert!(authorize_admin_request(&good, Some("secret-token")).is_ok());
    }

    #[test]
    fn csv_cells_escape_delimiters_quotes_and_formula_prefixes() {
        assert_eq!(csv_escape_cell("simple"), "simple");
        assert_eq!(csv_escape_cell("hello,world"), "\"hello,world\"");
        assert_eq!(csv_escape_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_escape_cell("=SUM(A1:A2)"), "\"'=SUM(A1:A2)\"");
        assert_eq!(
            csv_row(["id", "model,with,comma", "+formula"]),
            "id,\"model,with,comma\",\"'+formula\"\n"
        );
    }

    async fn test_history_manager() -> Arc<ChatHistoryManager> {
        let config = ChatHistoryConfig {
            enabled: true,
            primary_backend: "memory".to_string(),
            sink_backends: Vec::new(),
            privacy_level: PrivacyLevel::Full,
            ttl_seconds: 3600,
            strict: false,
            jsonl_path: None,
            memory_max_messages: Some(1000),
            sqlite_url: None,
            postgres_url: None,
            turso_url: None,
            turso_auth_token: None,
        };
        Arc::new(
            ChatHistoryManager::new(config)
                .await
                .expect("create chat history manager"),
        )
    }

    #[actix_web::test]
    async fn record_chat_history_tracks_requested_and_actual_models() {
        let manager = test_history_manager().await;
        let chat_history = Some(manager.clone());
        let resolution = UpstreamResolution {
            base_url: "https://upstream.example/v1".to_string(),
            mode: crate::util::UpstreamMode::Responses,
            key_env: None,
            headers: None,
            model_id: "gpt-4o-mini-2024-07-18".to_string(),
            plan: None,
            source: UpstreamSource::RoutingConfig,
        };
        let request_body = json!({
            "model": "alias-x",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let response_body = json!({
            "choices": [{
                "message": { "role": "assistant", "content": "Hi there" }
            }]
        });
        let usage = crate::analytics::TokenUsage {
            prompt_tokens: 12,
            completion_tokens: 7,
            total_tokens: 19,
            cached_tokens: Some(5),
            reasoning_tokens: Some(1),
        };

        record_chat_history(
            &chat_history,
            Some("conv-alias-model".to_string()),
            "alias-x",
            &request_body,
            &response_body,
            &resolution,
            Some(&usage),
        )
        .await;

        let messages = manager
            .list_messages(&MessageFilters {
                conversation_id: Some("conv-alias-model".to_string()),
                ..Default::default()
            })
            .await
            .expect("messages");
        assert_eq!(messages.len(), 2);

        for msg in &messages {
            assert_eq!(msg.routing.requested_model.as_deref(), Some("alias-x"));
            assert_eq!(
                msg.routing.actual_model.as_deref(),
                Some("gpt-4o-mini-2024-07-18")
            );
        }

        let assistant = messages
            .iter()
            .find(|m| m.role == MessageRole::Assistant)
            .expect("assistant message");
        assert_eq!(assistant.tokens.input_tokens, Some(12));
        assert_eq!(assistant.tokens.output_tokens, Some(7));
        assert_eq!(assistant.tokens.cached_tokens, Some(5));
        assert_eq!(assistant.tokens.reasoning_tokens, Some(1));
    }

    #[actix_web::test]
    async fn record_chat_history_supports_responses_shape() {
        let manager = test_history_manager().await;
        let chat_history = Some(manager.clone());
        let resolution = UpstreamResolution {
            base_url: "https://upstream.example/v1".to_string(),
            mode: crate::util::UpstreamMode::Responses,
            key_env: None,
            headers: None,
            model_id: "gpt-4o-mini".to_string(),
            plan: None,
            source: UpstreamSource::RouterPlan,
        };
        let request_body = json!({
            "model": "alias-responses",
            "input": [{"role": "user", "content": "Ping"}]
        });
        let response_body = json!({
            "output_text": "Pong",
            "usage": {"input_tokens": 9, "output_tokens": 4}
        });
        let usage = crate::analytics_middleware::extract_token_usage(&response_body)
            .expect("responses usage extraction");

        record_chat_history(
            &chat_history,
            Some("conv-responses-shape".to_string()),
            "alias-responses",
            &request_body,
            &response_body,
            &resolution,
            Some(&usage),
        )
        .await;

        let messages = manager
            .list_messages(&MessageFilters {
                conversation_id: Some("conv-responses-shape".to_string()),
                ..Default::default()
            })
            .await
            .expect("messages");

        assert!(
            messages.iter().any(|m| m.role == MessageRole::User),
            "expected a user message from input[]"
        );
        assert!(
            messages.iter().any(|m| m.role == MessageRole::Assistant),
            "expected an assistant message from output_text"
        );
    }
}

fn apply_upstream_headers(
    builder: reqwest::RequestBuilder,
    headers: &Option<HashMap<String, String>>,
) -> reqwest::RequestBuilder {
    if let Some(map) = headers {
        let mut builder = builder;
        for (key, value) in map {
            match (
                reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                (Ok(name), Ok(val)) => {
                    builder = builder.header(name, val);
                }
                _ => warn!("Skipping invalid upstream header from router: {}", key),
            }
        }
        builder
    } else {
        builder
    }
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|idx| idx + 1)
        .unwrap_or(start);
    &bytes[start..end]
}

struct ResponsesSseToChatSse<S> {
    inner: S,
    buffer: Vec<u8>,
    done: bool,
    is_first_chunk: bool,
}

impl<S> ResponsesSseToChatSse<S>
where
    S: futures_util::stream::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    fn new(inner: S) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            done: false,
            is_first_chunk: true,
        }
    }

    fn next_event(&mut self) -> Option<Vec<u8>> {
        if let Some(pos) = self.buffer.windows(2).position(|window| window == b"\n\n") {
            let mut event = self.buffer.drain(..pos + 2).collect::<Vec<u8>>();
            let original_event = event.clone();
            // Remove trailing delimiter from event slice for parsing
            event.truncate(event.len().saturating_sub(2));

            let mut other_lines = Vec::new();
            let mut data_segments: Vec<Vec<u8>> = Vec::new();
            for line in event.split(|&b| b == b'\n') {
                let line = if let Some(stripped) = line.strip_suffix(b"\r") {
                    stripped
                } else {
                    line
                };
                if line.starts_with(b"data:") {
                    let payload = trim_ascii(&line[5..]);
                    if !payload.is_empty() {
                        data_segments.push(payload.to_vec());
                    }
                } else if !line.is_empty() {
                    other_lines.push(line.to_vec());
                }
            }

            if data_segments.is_empty() {
                return Some(original_event);
            }

            let data_payload = data_segments.join(&b'\n');
            if trim_ascii(&data_payload) == b"[DONE]" {
                let mut out = Vec::new();
                for line in &other_lines {
                    out.extend_from_slice(line);
                    out.push(b'\n');
                }
                out.extend_from_slice(b"data: [DONE]\n\n");
                return Some(out);
            }

            match serde_json::from_slice::<responses::ResponsesChunk>(&data_payload) {
                Ok(chunk) => {
                    let chat_chunk = responses_chunk_to_chat_chunk(&chunk, self.is_first_chunk);
                    self.is_first_chunk = false;
                    match serde_json::to_vec(&chat_chunk) {
                        Ok(json) => {
                            let mut out = Vec::new();
                            for line in &other_lines {
                                out.extend_from_slice(line);
                                out.push(b'\n');
                            }
                            out.extend_from_slice(b"data: ");
                            out.extend_from_slice(&json);
                            out.extend_from_slice(b"\n\n");
                            Some(out)
                        }
                        Err(_) => Some(original_event),
                    }
                }
                Err(_) => Some(original_event),
            }
        } else {
            None
        }
    }
}

impl<S> futures_util::stream::Stream for ResponsesSseToChatSse<S>
where
    S: futures_util::stream::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            if let Some(event) = this.next_event() {
                return Poll::Ready(Some(Ok(Bytes::from(event))));
            }

            if this.done {
                if this.buffer.is_empty() {
                    return Poll::Ready(None);
                } else {
                    // Emit remaining buffer even if it lacks terminator
                    let remaining = std::mem::take(&mut this.buffer);
                    return Poll::Ready(Some(Ok(Bytes::from(remaining))));
                }
            }

            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    this.buffer.extend_from_slice(&chunk);
                }
                Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err))),
                Poll::Ready(None) => {
                    this.done = true;
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

struct SafetySseGuard<S> {
    inner: S,
    rolling_text: String,
    emitted_block: bool,
}

impl<S> SafetySseGuard<S>
where
    S: futures_util::stream::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    fn new(inner: S) -> Self {
        Self {
            inner,
            rolling_text: String::new(),
            emitted_block: false,
        }
    }
}

impl<S> futures_util::stream::Stream for SafetySseGuard<S>
where
    S: futures_util::stream::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.emitted_block {
            return Poll::Ready(None);
        }

        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                let text = String::from_utf8_lossy(&chunk);
                this.rolling_text.push_str(&text);
                if this.rolling_text.len() > 24_000 {
                    this.rolling_text = this
                        .rolling_text
                        .chars()
                        .rev()
                        .take(16_000)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                }

                let decision = crate::safety_judge::guard_response_text(&this.rolling_text);
                if decision.should_block() {
                    this.emitted_block = true;
                    tracing::warn!(
                        target: "routiium::safety",
                        guard_id = %decision.id,
                        risk = %decision.risk_level,
                        categories = ?decision.categories,
                        "streaming response blocked by response guard"
                    );
                    let payload = serde_json::json!({
                        "error": {
                            "message": "Streaming response blocked by Routiium response guard",
                            "type": "safety_policy_error",
                            "code": "response_guard_blocked",
                            "safety_event_id": decision.id,
                            "reason": decision.reason,
                            "risk_level": decision.risk_level,
                            "categories": decision.categories,
                            "policy_rev": decision.policy_rev
                        }
                    });
                    let event = format!("event: error\ndata: {}\n\n", payload);
                    Poll::Ready(Some(Ok(Bytes::from(event))))
                } else {
                    Poll::Ready(Some(Ok(chunk)))
                }
            }
            other => other,
        }
    }
}

/// Passthrough for OpenAI Responses API (`/v1/responses`):
/// Accepts native Responses payload and forwards upstream without transformation.
/// Supports SSE when `stream: true`.
async fn responses_passthrough(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> impl Responder {
    let mut body = body.into_inner();
    let started_at = std::time::Instant::now();
    let requested_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let conversation_hint = extract_conversation_id(&body).filter(|s| !s.trim().is_empty());
    let mut system_prompt_applied = false;

    // Apply system prompt injection if configured
    let system_prompt_guard = state.system_prompt_config.read().await;
    let model = body.get("model").and_then(|v| v.as_str());

    if let Some(prompt) = system_prompt_guard.get_prompt(model, Some("responses")) {
        // Inject system prompt into messages (Responses API uses "input" not "messages")
        if let Some(messages) = body.get_mut("input").and_then(|v| v.as_array_mut()) {
            system_prompt_applied = true;
            let system_msg = serde_json::json!({
                "role": "system",
                "content": prompt
            });

            match system_prompt_guard.injection_mode.as_str() {
                "append" => {
                    let last_system_pos = messages
                        .iter()
                        .rposition(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"));
                    if let Some(pos) = last_system_pos {
                        messages.insert(pos + 1, system_msg);
                    } else {
                        messages.push(system_msg);
                    }
                }
                "replace" => {
                    messages.retain(|m| m.get("role").and_then(|r| r.as_str()) != Some("system"));
                    messages.insert(0, system_msg);
                }
                _ => {
                    // Default: prepend
                    messages.insert(0, system_msg);
                }
            }
        }
    }
    drop(system_prompt_guard);

    // Convert Chat API-formatted tools to Responses API flat format
    // The Python SDK sends tools in Chat API format (nested function object),
    // but OpenAI Responses API expects flat format (name/description/parameters at top level)
    if let Some(tools) = body.get_mut("tools").and_then(|t| t.as_array_mut()) {
        for tool in tools.iter_mut() {
            if let Some(obj) = tool.as_object_mut() {
                // Check if it's Chat API format: {"type": "function", "function": {...}}
                if let Some(function) = obj.get("function").and_then(|f| f.as_object()) {
                    // Extract all fields from nested function object first (before mutating obj)
                    let name = function.get("name").cloned();
                    let desc = function.get("description").cloned();
                    let params = function.get("parameters").cloned();

                    // Now insert into the top level
                    if let Some(n) = name {
                        obj.insert("name".to_string(), n);
                    }
                    if let Some(d) = desc {
                        obj.insert("description".to_string(), d);
                    }
                    if let Some(p) = params {
                        obj.insert("parameters".to_string(), p);
                    }
                    // Remove the nested function object
                    obj.remove("function");
                }
            }
        }
    }

    if let Some(mgr) = state.mcp_manager.as_ref() {
        let manager = mgr.read().await;
        match manager.list_all_tools().await {
            Ok(mcp_tools) => merge_mcp_tools_into_responses_payload(&mut body, &mcp_tools),
            Err(err) => warn!("Failed to fetch MCP tools for /v1/responses: {}", err),
        }
    }

    // Determine managed (internal upstream key) vs passthrough mode
    let managed_mode = managed_mode_from_env();
    let authenticated: bool;
    let mut api_key_id: Option<String> = None;
    let mut api_key_label: Option<String> = None;

    // Extract client bearer
    let client_bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            let s = s.trim();
            if s.len() >= 7 && s[..6].eq_ignore_ascii_case("bearer") {
                Some(s[6..].trim().to_string())
            } else {
                None
            }
        });

    // Resolve upstream bearer (managed mode validates client token but defers provider key selection to routing)
    let upstream_bearer = if managed_mode {
        if let Some(manager) = &state.api_keys {
            match client_bearer.as_deref().map(|tok| manager.verify(tok)) {
                Some(crate::auth::Verification::Valid { id, label, .. }) => {
                    authenticated = true;
                    api_key_id = Some(id);
                    api_key_label = label;
                    None
                }
                Some(crate::auth::Verification::Revoked { .. }) => {
                    tracing::warn!(
                        target: "routiium::auth",
                        api = "responses",
                        client = request_client_ip(&req),
                        "API key revoked"
                    );
                    return error_response(http::StatusCode::UNAUTHORIZED, "API key revoked");
                }
                Some(crate::auth::Verification::Expired { .. }) => {
                    tracing::warn!(
                        target: "routiium::auth",
                        api = "responses",
                        client = request_client_ip(&req),
                        "API key expired"
                    );
                    return error_response(http::StatusCode::UNAUTHORIZED, "API key expired");
                }
                Some(_) => {
                    tracing::warn!(
                        target: "routiium::auth",
                        api = "responses",
                        client = request_client_ip(&req),
                        "Invalid API key"
                    );
                    return error_response(http::StatusCode::UNAUTHORIZED, "Invalid API key");
                }
                None => {
                    tracing::warn!(
                        target: "routiium::auth",
                        api = "responses",
                        client = request_client_ip(&req),
                        "Missing Authorization bearer"
                    );
                    return error_response(
                        http::StatusCode::UNAUTHORIZED,
                        "Missing Authorization bearer",
                    );
                }
            }
        } else {
            return managed_key_store_unavailable_response("responses", &req);
        }
    } else {
        if client_bearer.is_none() {
            tracing::warn!(
                target: "routiium::auth",
                api = "responses",
                client = request_client_ip(&req),
                "Missing Authorization bearer"
            );
            return error_response(
                http::StatusCode::UNAUTHORIZED,
                "Missing Authorization bearer",
            );
        }
        authenticated = true;
        client_bearer.clone()
    };

    // Rate limiting check (after auth, before upstream)
    let rl_result = match check_rate_limits_for_key(
        &state,
        api_key_id.as_deref(),
        req.path(),
        body.get("model").and_then(|v| v.as_str()),
    )
    .await
    {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let mut stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let client = &state.http;
    let resolution = match resolve_upstream(&state, "responses", &mut body).await {
        Ok(res) => res,
        Err(err) => {
            record_router_error_event(&state, &req, "responses", &requested_model, &err).await;
            if let Some(response) =
                router_plan_rejection_response(&err, "responses", &requested_model)
            {
                return response;
            }
            return router_plan_error_response(&err);
        }
    };

    let streaming_safety =
        if stream && crate::safety_judge::should_force_non_stream(resolution.plan.as_ref()) {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("stream".to_string(), serde_json::json!(false));
            }
            stream = false;
            Some("forced_non_stream")
        } else if stream {
            Some(crate::safety_judge::streaming_safety_mode_from_env().as_str())
        } else {
            None
        };

    log_request_start("responses", &req, &requested_model, &resolution, stream);

    let mut effective_body = body.clone();
    if let Some(obj) = effective_body.as_object_mut() {
        obj.insert(
            "model".to_string(),
            serde_json::json!(resolution.model_id.clone()),
        );
        if streaming_safety == Some("forced_non_stream") {
            obj.insert("stream".to_string(), serde_json::json!(false));
        }
    }

    let mut eff_bearer = upstream_bearer.clone();
    if eff_bearer.is_none() {
        if let Some(key_env) = resolution.key_env.as_deref() {
            if let Ok(v) = std::env::var(key_env) {
                if !v.is_empty() {
                    eff_bearer = Some(v);
                }
            }
        }
    }
    if eff_bearer.is_none() {
        if let Ok(v) = std::env::var("OPENAI_API_KEY") {
            if !v.is_empty() {
                eff_bearer = Some(v);
            }
        }
    }

    // Handle Bedrock mode separately (uses AWS SDK instead of HTTP)
    if matches!(resolution.mode, crate::util::UpstreamMode::Bedrock) {
        // Convert to Chat format first
        let chat_req = crate::conversion::responses_json_to_chat_request(&effective_body);

        // Convert to Bedrock format
        let (_content_type, bedrock_body) = match crate::bedrock::chat_to_bedrock_request(&chat_req)
        {
            Ok(result) => result,
            Err(e) => {
                return error_response(
                    http::StatusCode::BAD_REQUEST,
                    &format!("Failed to convert to Bedrock format: {}", e),
                );
            }
        };

        // Extract region from base_url or use default
        let region = resolution.base_url.split('.').nth(1).unwrap_or("us-east-1");

        // Invoke Bedrock model (non-streaming for now)
        match crate::bedrock::invoke_bedrock_model(&resolution.model_id, bedrock_body, region).await
        {
            Ok(bedrock_response) => {
                // Convert Bedrock response to Chat Completions format
                match crate::bedrock::bedrock_to_chat_response(
                    bedrock_response,
                    &resolution.model_id,
                    None,
                ) {
                    Ok(chat_response) => {
                        // Convert Chat to Responses format
                        let responses_response =
                            crate::conversion::chat_to_responses_response(&chat_response);
                        let response_json =
                            serde_json::to_value(&responses_response).unwrap_or_default();
                        let token_usage =
                            crate::analytics_middleware::extract_token_usage(&response_json);
                        let response_bytes =
                            serde_json::to_vec(&responses_response).unwrap_or_default();
                        let response_guard =
                            crate::safety_judge::guard_response_bytes(&response_bytes);
                        if response_guard.should_block() {
                            tracing::warn!(
                                target: "routiium::safety",
                                guard_id = %response_guard.id,
                                risk = %response_guard.risk_level,
                                categories = ?response_guard.categories,
                                "bedrock responses output blocked by response guard"
                            );
                            record_analytics_event(
                                &state,
                                &req,
                                &body,
                                &requested_model,
                                &resolution,
                                started_at,
                                403,
                                0,
                                None,
                                authenticated,
                                api_key_id.clone(),
                                api_key_label.clone(),
                                system_prompt_applied,
                                Some(response_guard.reason.clone()),
                            )
                            .await;
                            record_response_guard_event(
                                &state,
                                &req,
                                "responses",
                                &requested_model,
                                &resolution,
                                &response_guard,
                            )
                            .await;
                            return response_guard_error_response(
                                &response_guard,
                                resolution.plan.as_ref(),
                                &resolution.model_id,
                                "responses",
                            );
                        }

                        record_chat_history(
                            &state.chat_history,
                            conversation_hint.clone(),
                            &requested_model,
                            &body,
                            &response_json,
                            &resolution,
                            token_usage.as_ref(),
                        )
                        .await;

                        record_analytics_event(
                            &state,
                            &req,
                            &body,
                            &requested_model,
                            &resolution,
                            started_at,
                            200,
                            serde_json::to_vec(&responses_response)
                                .map(|v| v.len())
                                .unwrap_or(0),
                            token_usage,
                            authenticated,
                            api_key_id.clone(),
                            api_key_label.clone(),
                            system_prompt_applied,
                            None,
                        )
                        .await;

                        let mut builder = HttpResponse::Ok();
                        if let Some(plan) = resolution.plan.as_ref() {
                            insert_route_headers(&mut builder, plan, &resolution.model_id);
                        }
                        insert_response_guard_headers(&mut builder, &response_guard);
                        return builder.json(responses_response);
                    }
                    Err(e) => {
                        return error_response(
                            http::StatusCode::INTERNAL_SERVER_ERROR,
                            &format!("Failed to convert Bedrock response: {}", e),
                        );
                    }
                }
            }
            Err(e) => {
                return error_response(
                    http::StatusCode::BAD_GATEWAY,
                    &format!("Bedrock invocation failed: {}", e),
                );
            }
        }
    }

    let endpoint = match resolution.mode {
        crate::util::UpstreamMode::Responses => "responses",
        crate::util::UpstreamMode::Chat => "chat/completions",
        crate::util::UpstreamMode::Bedrock => "bedrock/invoke", // Shouldn't reach here
    };
    let base = resolution.base_url.trim_end_matches('/');

    if stream {
        use bytes::Bytes;
        use futures_util::TryStreamExt;

        let mut stream_body = effective_body.clone();
        if matches!(resolution.mode, crate::util::UpstreamMode::Chat) {
            stream_body = crate::conversion::responses_json_to_chat_value(&stream_body);
        }

        let real_url = format!("{}/{}", base, endpoint);
        let mut rb = client
            .post(&real_url)
            .header("accept", "text/event-stream")
            .header("content-type", "application/json")
            .header("connection", "close")
            .json(&stream_body);
        rb = apply_upstream_headers(rb, &resolution.headers);
        if let Some(b) = eff_bearer.clone() {
            rb = rb.bearer_auth(b);
        }

        match rb.send().await {
            Ok(up) => {
                let status = up.status();
                if !status.is_success() {
                    let bytes = up.bytes().await.unwrap_or_default();
                    let response_json = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
                    let token_usage = response_json
                        .as_ref()
                        .and_then(crate::analytics_middleware::extract_token_usage);
                    let error_message = extract_error_message(response_json.as_ref(), &bytes);
                    record_analytics_event(
                        &state,
                        &req,
                        &body,
                        &requested_model,
                        &resolution,
                        started_at,
                        status.as_u16(),
                        bytes.len(),
                        token_usage,
                        authenticated,
                        api_key_id.clone(),
                        api_key_label.clone(),
                        system_prompt_applied,
                        error_message,
                    )
                    .await;

                    let mut builder = HttpResponse::build(
                        actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap(),
                    );
                    if let Some(plan) = resolution.plan.as_ref() {
                        insert_route_headers(&mut builder, plan, &resolution.model_id);
                    }
                    return builder.body(bytes);
                }

                let upstream_ct = up.headers().get("content-type").cloned();
                let base_stream = up
                    .bytes_stream()
                    .map_err(|e| std::io::Error::other(e.to_string()))
                    .map_ok(Bytes::from);

                let stream: Pin<
                    Box<
                        dyn futures_util::stream::Stream<Item = Result<Bytes, std::io::Error>>
                            + Send,
                    >,
                > = if let Some(plan) = resolution.plan.as_ref() {
                    if matches!(plan.upstream.mode, RouterUpstreamMode::Responses) {
                        Box::pin(ResponsesSseToChatSse::new(base_stream))
                    } else {
                        Box::pin(base_stream)
                    }
                } else {
                    Box::pin(base_stream)
                };

                let stream: Pin<
                    Box<
                        dyn futures_util::stream::Stream<Item = Result<Bytes, std::io::Error>>
                            + Send,
                    >,
                > = match crate::safety_judge::streaming_safety_mode_from_env() {
                    crate::safety_judge::StreamingSafetyMode::Off => stream,
                    _ => Box::pin(SafetySseGuard::new(stream)),
                };

                let mut response = HttpResponse::Ok();
                if let Some(ct) = upstream_ct {
                    if let Ok(ct_str) = ct.to_str() {
                        response.insert_header(("content-type", ct_str));
                    } else {
                        response.insert_header(("content-type", "text/event-stream"));
                    }
                } else {
                    response.insert_header(("content-type", "text/event-stream"));
                }
                response
                    .insert_header(("cache-control", "no-cache"))
                    .insert_header(("connection", "keep-alive"));
                if let Some(plan) = resolution.plan.as_ref() {
                    insert_route_headers(&mut response, plan, &resolution.model_id);
                }
                if let Some(value) = streaming_safety {
                    response.insert_header(("x-streaming-safety", value));
                }
                if let Some(ref rl) = rl_result {
                    insert_rate_limit_headers(&mut response, rl);
                }

                record_analytics_event(
                    &state,
                    &req,
                    &body,
                    &requested_model,
                    &resolution,
                    started_at,
                    status.as_u16(),
                    0,
                    None,
                    authenticated,
                    api_key_id.clone(),
                    api_key_label.clone(),
                    system_prompt_applied,
                    None,
                )
                .await;

                response.streaming(stream)
            }
            Err(e) => {
                record_analytics_event(
                    &state,
                    &req,
                    &body,
                    &requested_model,
                    &resolution,
                    started_at,
                    502,
                    0,
                    None,
                    authenticated,
                    api_key_id.clone(),
                    api_key_label.clone(),
                    system_prompt_applied,
                    Some(e.to_string()),
                )
                .await;
                router_error_response(
                    http::StatusCode::BAD_GATEWAY,
                    &e.to_string(),
                    resolution.plan.as_ref(),
                    &resolution.model_id,
                )
            }
        }
    } else {
        let mut outbound_body = effective_body.clone();
        if matches!(resolution.mode, crate::util::UpstreamMode::Chat) {
            outbound_body = crate::conversion::responses_json_to_chat_value(&outbound_body);
        }

        let real_url = format!("{}/{}", base, endpoint);
        let mut upstream_req = client
            .post(&real_url)
            .header("content-type", "application/json");
        upstream_req = apply_upstream_headers(upstream_req, &resolution.headers);
        if let Some(b) = eff_bearer {
            upstream_req = upstream_req.bearer_auth(b);
        }
        match upstream_req.json(&outbound_body).send().await {
            Ok(up) => {
                let status = up.status();
                let bytes = up.bytes().await.unwrap_or_default();
                let response_json = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
                let token_usage = response_json
                    .as_ref()
                    .and_then(crate::analytics_middleware::extract_token_usage);

                let response_guard = if status.is_success() {
                    let decision = crate::safety_judge::guard_response_bytes(&bytes);
                    if decision.should_block() {
                        tracing::warn!(
                            target: "routiium::safety",
                            guard_id = %decision.id,
                            risk = %decision.risk_level,
                            categories = ?decision.categories,
                            "responses output blocked by response guard"
                        );
                        record_analytics_event(
                            &state,
                            &req,
                            &body,
                            &requested_model,
                            &resolution,
                            started_at,
                            403,
                            0,
                            None,
                            authenticated,
                            api_key_id.clone(),
                            api_key_label.clone(),
                            system_prompt_applied,
                            Some(decision.reason.clone()),
                        )
                        .await;
                        record_response_guard_event(
                            &state,
                            &req,
                            "responses",
                            &requested_model,
                            &resolution,
                            &decision,
                        )
                        .await;
                        return response_guard_error_response(
                            &decision,
                            resolution.plan.as_ref(),
                            &resolution.model_id,
                            "responses",
                        );
                    }
                    Some(decision)
                } else {
                    None
                };

                if status.is_success() {
                    if let Some(ref response_json) = response_json {
                        record_chat_history(
                            &state.chat_history,
                            conversation_hint.clone(),
                            &requested_model,
                            &body,
                            response_json,
                            &resolution,
                            token_usage.as_ref(),
                        )
                        .await;
                    }
                }

                let error_message = if status.is_success() {
                    None
                } else {
                    extract_error_message(response_json.as_ref(), &bytes)
                };
                record_analytics_event(
                    &state,
                    &req,
                    &body,
                    &requested_model,
                    &resolution,
                    started_at,
                    status.as_u16(),
                    bytes.len(),
                    token_usage.clone(),
                    authenticated,
                    api_key_id.clone(),
                    api_key_label.clone(),
                    system_prompt_applied,
                    error_message,
                )
                .await;

                let mut builder = HttpResponse::build(
                    actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap(),
                );
                if let Some(plan) = resolution.plan.as_ref() {
                    insert_route_headers(&mut builder, plan, &resolution.model_id);
                    if let Some(decision) = response_guard.as_ref() {
                        insert_response_guard_headers(&mut builder, decision);
                    }
                    if let Some(value) = streaming_safety {
                        builder.insert_header(("x-streaming-safety", value));
                    }
                    if status.is_success()
                        && matches!(plan.upstream.mode, RouterUpstreamMode::Responses)
                    {
                        if let Ok(resp_obj) =
                            serde_json::from_slice::<responses::ResponsesResponse>(&bytes)
                        {
                            let chat_resp = responses_to_chat_response(&resp_obj);
                            if let Ok(body) = serde_json::to_vec(&chat_resp) {
                                builder.insert_header(("content-type", "application/json"));
                                if let Some(ref rl) = rl_result {
                                    insert_rate_limit_headers(&mut builder, rl);
                                }
                                return builder.body(body);
                            }
                        }
                    }
                }
                if resolution.plan.is_none() {
                    if let Some(decision) = response_guard.as_ref() {
                        insert_response_guard_headers(&mut builder, decision);
                    }
                    if let Some(value) = streaming_safety {
                        builder.insert_header(("x-streaming-safety", value));
                    }
                }
                if let Some(ref rl) = rl_result {
                    insert_rate_limit_headers(&mut builder, rl);
                }
                builder.body(bytes)
            }
            Err(e) => {
                record_analytics_event(
                    &state,
                    &req,
                    &body,
                    &requested_model,
                    &resolution,
                    started_at,
                    502,
                    0,
                    None,
                    authenticated,
                    api_key_id,
                    api_key_label,
                    system_prompt_applied,
                    Some(e.to_string()),
                )
                .await;
                router_error_response(
                    http::StatusCode::BAD_GATEWAY,
                    &e.to_string(),
                    resolution.plan.as_ref(),
                    &resolution.model_id,
                )
            }
        }
    }
}

async fn admin_panel_state(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut keys = match &state.api_keys {
        Some(mgr) => mgr.list_keys().unwrap_or_default(),
        None => Vec::new(),
    };

    if let Some(rl_mgr) = state.rate_limit_manager.as_ref() {
        for key in &mut keys {
            key.rate_limit_policy = rl_mgr.get_key_policy_id(&key.id).await.ok().flatten();
        }
    }
    keys.sort_by_key(|key| std::cmp::Reverse(key.created_at));

    let active_key_count = keys
        .iter()
        .filter(|key| api_key_status(key, now_secs) == "active")
        .count();

    let principal_window_start = now_secs.saturating_sub(30 * 24 * 60 * 60);
    let principal_events = if let Some(mgr) = &state.analytics {
        mgr.query_range(principal_window_start, now_secs, Some(5000))
            .await
            .ok()
    } else {
        None
    };

    let mut principal_stats: HashMap<String, (u64, u64, HashSet<String>)> = HashMap::new();
    if let Some(events) = &principal_events {
        for event in events {
            if !event.auth.authenticated {
                continue;
            }
            let Some(api_key_id) = event.auth.api_key_id.as_ref() else {
                continue;
            };
            let entry = principal_stats
                .entry(api_key_id.clone())
                .or_insert_with(|| (0, 0, HashSet::new()));
            entry.0 += 1;
            entry.1 = entry.1.max(event.timestamp);
            if let Some(model) = event.request.model.as_ref() {
                if !model.is_empty() {
                    entry.2.insert(model.clone());
                }
            }
        }
    }

    let mut principal_items: Vec<serde_json::Value> = Vec::new();
    let known_key_ids: HashSet<String> = keys.iter().map(|key| key.id.clone()).collect();
    for key in &keys {
        let (request_count, last_seen, models) = principal_stats
            .remove(&key.id)
            .unwrap_or_else(|| (0, 0, HashSet::new()));
        let mut models_used: Vec<String> = models.into_iter().collect();
        models_used.sort();
        principal_items.push(serde_json::json!({
            "id": key.id,
            "label": key.label,
            "status": api_key_status(key, now_secs),
            "created_at": key.created_at,
            "expires_at": key.expires_at,
            "revoked_at": key.revoked_at,
            "scopes": key.scopes,
            "rate_limit_policy": key.rate_limit_policy,
            "request_count_30d": request_count,
            "last_seen_at": (last_seen > 0).then_some(last_seen),
            "models_used": models_used
        }));
    }

    for (key_id, (request_count, last_seen, models)) in principal_stats {
        if known_key_ids.contains(&key_id) {
            continue;
        }
        let mut models_used: Vec<String> = models.into_iter().collect();
        models_used.sort();
        principal_items.push(serde_json::json!({
            "id": key_id,
            "label": serde_json::Value::Null,
            "status": "observed_only",
            "created_at": serde_json::Value::Null,
            "expires_at": serde_json::Value::Null,
            "revoked_at": serde_json::Value::Null,
            "scopes": serde_json::Value::Null,
            "rate_limit_policy": serde_json::Value::Null,
            "request_count_30d": request_count,
            "last_seen_at": (last_seen > 0).then_some(last_seen),
            "models_used": models_used
        }));
    }

    principal_items.sort_by(|a, b| {
        let b_requests = b
            .get("request_count_30d")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let a_requests = a
            .get("request_count_30d")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        b_requests.cmp(&a_requests)
    });

    let analytics_stats = if let Some(mgr) = &state.analytics {
        mgr.stats()
            .await
            .ok()
            .and_then(|stats| serde_json::to_value(stats).ok())
    } else {
        None
    };

    let chat_history_stats = if let Some(mgr) = &state.chat_history {
        mgr.stats()
            .await
            .ok()
            .and_then(|stats| serde_json::to_value(stats).ok())
    } else {
        None
    };
    let chat_history_health = if let Some(mgr) = &state.chat_history {
        mgr.health().await.ok()
    } else {
        None
    };
    let chat_history_config = crate::chat_history_manager::ChatHistoryConfig::from_env();

    let system_prompt_guard = state.system_prompt_config.read().await;
    let system_prompt_config = system_prompt_guard.clone();
    let system_prompt_summary = serde_json::json!({
        "global_configured": system_prompt_config.global.is_some(),
        "per_model_count": system_prompt_config.per_model.len(),
        "per_api_count": system_prompt_config.per_api.len(),
        "injection_mode": system_prompt_config.injection_mode,
        "enabled": system_prompt_config.enabled
    });
    drop(system_prompt_guard);

    let mcp_config = state
        .mcp_config_path
        .as_ref()
        .and_then(|path| crate::mcp_config::McpConfig::load_from_file(path).ok());
    let (mcp_connected_servers, mcp_tools) = if let Some(manager_arc) = state.mcp_manager.as_ref() {
        let manager = manager_arc.read().await;
        let connected_servers = manager.connected_servers();
        let tools = manager
            .list_all_tools()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|tool| {
                serde_json::json!({
                    "server_name": tool.server_name,
                    "name": tool.name,
                    "combined_name": format!("{}_{}", tool.server_name, tool.name),
                    "description": tool.description,
                    "input_schema": tool.input_schema
                })
            })
            .collect::<Vec<_>>();
        (connected_servers, tools)
    } else {
        (Vec::new(), Vec::new())
    };

    let routing_guard = state.routing_config.read().await;
    let routing_config = routing_guard.clone();
    let routing_stats = routing_guard.stats();
    let mut routing_bedrock_backends = Vec::new();
    for rule in &routing_guard.rules {
        for backend in &rule.backends {
            if backend.mode == crate::routing_config::UpstreamMode::Bedrock {
                routing_bedrock_backends.push(serde_json::json!({
                    "rule_id": rule.id,
                    "description": rule.description,
                    "base_url": backend.base_url,
                    "key_env": backend.key_env,
                    "weight": backend.weight,
                    "timeout_seconds": backend.timeout_seconds
                }));
            }
        }
    }
    if let Some(default_backend) = routing_guard.default_backend.as_ref() {
        if default_backend.mode == crate::routing_config::UpstreamMode::Bedrock {
            routing_bedrock_backends.push(serde_json::json!({
                "rule_id": serde_json::Value::Null,
                "description": "default_backend",
                "base_url": default_backend.base_url,
                "key_env": default_backend.key_env
            }));
        }
    }
    drop(routing_guard);

    let router_catalog = if let Some(router) = &state.router_client {
        router.get_catalog().await.ok()
    } else {
        None
    };
    let router_catalog_models = router_catalog
        .as_ref()
        .map(|catalog| {
            catalog
                .models
                .iter()
                .map(|model| {
                    serde_json::json!({
                        "id": model.id,
                        "provider": model.provider,
                        "aliases": model.aliases,
                        "status": model.status,
                        "policy_tags": model.policy_tags,
                        "region": model.region
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let bedrock_catalog_models = router_catalog
        .as_ref()
        .map(|catalog| {
            catalog
                .models
                .iter()
                .filter(|model| model.provider.eq_ignore_ascii_case("bedrock"))
                .map(|model| {
                    serde_json::json!({
                        "id": model.id,
                        "provider": model.provider,
                        "aliases": model.aliases,
                        "region": model.region,
                        "status": model.status
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut pricing_models = state
        .pricing
        .models
        .iter()
        .map(|(model, pricing)| {
            serde_json::json!({
                "model": model,
                "input_per_million": pricing.input_per_million,
                "output_per_million": pricing.output_per_million,
                "cached_per_million": pricing.cached_per_million,
                "reasoning_per_million": pricing.reasoning_per_million
            })
        })
        .collect::<Vec<_>>();
    pricing_models.sort_by(|a, b| {
        let a_model = a
            .get("model")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let b_model = b
            .get("model")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        a_model.cmp(b_model)
    });

    let (rate_limit_policies, rate_limit_default_policy_id, emergency_blocks) =
        if let Some(mgr) = &state.rate_limit_manager {
            (
                mgr.list_policies().await.unwrap_or_default(),
                mgr.get_default_policy_id().await.ok().flatten(),
                mgr.list_emergency_blocks().await.unwrap_or_default(),
            )
        } else {
            (Vec::new(), None, Vec::new())
        };

    let upstream_mode = match upstream_mode_from_env() {
        crate::util::UpstreamMode::Responses => "responses",
        crate::util::UpstreamMode::Chat => "chat",
        crate::util::UpstreamMode::Bedrock => "bedrock",
    };
    let aws_region = non_empty_env("AWS_REGION").or_else(|| non_empty_env("AWS_DEFAULT_REGION"));
    let aws_profile = non_empty_env("AWS_PROFILE");
    let bedrock_credentials_source = if non_empty_env("AWS_ACCESS_KEY_ID").is_some()
        && non_empty_env("AWS_SECRET_ACCESS_KEY").is_some()
    {
        "static"
    } else if aws_profile.is_some() {
        "profile"
    } else if non_empty_env("AWS_WEB_IDENTITY_TOKEN_FILE").is_some() {
        "web_identity"
    } else if aws_region.is_some() {
        "default_provider_chain"
    } else {
        "not_configured"
    };

    HttpResponse::Ok().json(serde_json::json!({
        "overview": {
            "generated_at": now_secs,
            "health": "ok",
            "bind_addr": env_bind_addr(),
            "admin_token_configured": non_empty_env("ROUTIIUM_ADMIN_TOKEN").is_some(),
            "api_keys": {
                "total": keys.len(),
                "active": active_key_count
            },
            "rate_limits": {
                "enabled": state.rate_limit_manager.is_some(),
                "policies": rate_limit_policies.len(),
                "default_policy_id": rate_limit_default_policy_id,
                "emergency_blocks": emergency_blocks.len()
            },
            "safety_audit": {
                "max_events": state.safety_audit.max_events(),
                "jsonl_path": state.safety_audit.jsonl_path()
            },
            "analytics": {
                "enabled": state.analytics.is_some(),
                "stats": analytics_stats
            },
            "chat_history": {
                "enabled": state.chat_history.is_some(),
                "health": chat_history_health,
                "stats": chat_history_stats
            }
        },
        "system_prompt": {
            "config_path": state.system_prompt_config_path,
            "reloadable": state.system_prompt_config_path.is_some(),
            "config": system_prompt_config,
            "summary": system_prompt_summary
        },
        "mcp": {
            "enabled": state.mcp_manager.is_some(),
            "config_path": state.mcp_config_path,
            "reloadable": state.mcp_config_path.is_some(),
            "config": mcp_config,
            "configured_servers": mcp_config.as_ref().map(|cfg| cfg.server_names()).unwrap_or_default(),
            "connected_servers": mcp_connected_servers,
            "tools": mcp_tools
        },
        "routing": {
            "config_path": state.routing_config_path,
            "reloadable": state.routing_config_path.is_some(),
            "config": routing_config,
            "stats": routing_stats,
            "router": {
                "enabled": state.router_client.is_some(),
                "mode": if state.router_config_path.is_some() {
                    "local"
                } else if state.router_url.is_some() {
                    "remote"
                } else if state.router_client.is_some() {
                    "configured"
                } else {
                    "none"
                },
                "config_path": state.router_config_path,
                "url": state.router_url,
                "catalog_revision": router_catalog.as_ref().map(|catalog| catalog.revision.clone()),
                "catalog_models": router_catalog_models
            }
        },
        "pricing": {
            "config_path": state.pricing_config_path,
            "source": if state.pricing_config_path.is_some() { "file" } else { "built_in" },
            "models_count": pricing_models.len(),
            "default_pricing": state.pricing.default.as_ref().map(|pricing| serde_json::json!({
                "input_per_million": pricing.input_per_million,
                "output_per_million": pricing.output_per_million,
                "cached_per_million": pricing.cached_per_million,
                "reasoning_per_million": pricing.reasoning_per_million
            })),
            "models": pricing_models
        },
        "settings": {
            "auth": {
                "mode": if managed_mode_from_env() { "managed" } else { "passthrough" },
                "managed_override": non_empty_env("ROUTIIUM_MANAGED_MODE"),
                "admin_token_configured": non_empty_env("ROUTIIUM_ADMIN_TOKEN").is_some(),
                "key_store_available": state.api_keys.is_some()
            },
            "server": {
                "bind_addr": env_bind_addr(),
                "upstream_mode": upstream_mode,
                "http_timeout_seconds": non_empty_env("ROUTIIUM_HTTP_TIMEOUT_SECONDS"),
                "proxy_url_configured": non_empty_env("ROUTIIUM_PROXY_URL").is_some(),
                "no_proxy": env_truthy("ROUTIIUM_NO_PROXY")
            },
            "cors": {
                "allow_all": env_truthy("CORS_ALLOW_ALL"),
                "allowed_origins": non_empty_env("CORS_ALLOWED_ORIGINS"),
                "allowed_methods": non_empty_env("CORS_ALLOWED_METHODS")
            },
            "analytics": {
                "enabled": state.analytics.is_some(),
                "backend": non_empty_env("ROUTIIUM_ANALYTICS_BACKEND"),
                "path": non_empty_env("ROUTIIUM_ANALYTICS_PATH")
            },
            "chat_history": {
                "enabled": chat_history_config.enabled,
                "primary_backend": chat_history_config.primary_backend,
                "sink_backends": chat_history_config.sink_backends,
                "privacy_level": format!("{:?}", chat_history_config.privacy_level).to_ascii_lowercase(),
                "ttl_seconds": chat_history_config.ttl_seconds,
                "strict": chat_history_config.strict,
                "jsonl_path": chat_history_config.jsonl_path,
                "sqlite_url_configured": chat_history_config.sqlite_url.is_some(),
                "postgres_url_configured": chat_history_config.postgres_url.is_some(),
                "turso_url_configured": chat_history_config.turso_url.is_some()
            },
            "rate_limits": {
                "enabled": state.rate_limit_manager.is_some(),
                "backend": non_empty_env("ROUTIIUM_RATE_LIMIT_BACKEND"),
                "config_path": non_empty_env("ROUTIIUM_RATE_LIMIT_CONFIG")
            },
            "safety": {
                "judge_mode": crate::safety_judge::SafetyJudgeConfig::from_env().mode.as_str(),
                "response_guard_mode": crate::safety_judge::response_guard_mode_from_env().as_str(),
                "streaming_safety": crate::safety_judge::streaming_safety_mode_from_env().as_str()
            }
        },
        "bedrock": {
            "enabled": matches!(upstream_mode_from_env(), crate::util::UpstreamMode::Bedrock) || !routing_bedrock_backends.is_empty() || !bedrock_catalog_models.is_empty(),
            "default_upstream_mode": upstream_mode,
            "aws_region": aws_region,
            "aws_profile": aws_profile,
            "credentials_source": bedrock_credentials_source,
            "routing_backends": routing_bedrock_backends,
            "router_catalog_models": bedrock_catalog_models
        },
        "principals": {
            "kind": "api_keys",
            "note": "Routiium does not maintain a first-class user directory. This panel derives principals from API keys and recent authenticated traffic.",
            "items": principal_items,
            "sample_window_start": principal_window_start,
            "sample_window_end": now_secs,
            "sample_limit": principal_events.as_ref().map(|events| events.len()).unwrap_or(0)
        },
        "rate_limits": {
            "policies": rate_limit_policies,
            "default_policy_id": rate_limit_default_policy_id,
            "emergency_blocks": emergency_blocks
        }
    }))
}

#[derive(Debug, Deserialize)]
struct SafetyEventsQuery {
    limit: Option<usize>,
}

async fn admin_safety_events(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<SafetyEventsQuery>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }

    let limit = query.limit.unwrap_or(100).clamp(1, 1_000);
    let events = state.safety_audit.recent(limit).await;
    HttpResponse::Ok().json(serde_json::json!({
        "events": events,
        "count": events.len(),
        "limit": limit,
        "jsonl_path": state.safety_audit.jsonl_path(),
        "max_events": state.safety_audit.max_events()
    }))
}

async fn admin_panel_update_system_prompts(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<crate::system_prompt_config::SystemPromptConfig>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }

    let Some(path) = state.system_prompt_config_path.as_ref() else {
        return error_response(
            http::StatusCode::BAD_REQUEST,
            "System prompt config is not file-backed",
        );
    };

    let config = body.into_inner();
    if let Err(err) = write_pretty_json_file(path, &config) {
        return error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to write system prompt config: {}", err),
        );
    }

    let mut guard = state.system_prompt_config.write().await;
    *guard = config.clone();
    drop(guard);

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "config_path": path,
        "config": config
    }))
}

async fn admin_panel_update_mcp(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<crate::mcp_config::McpConfig>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }

    let Some(path) = state.mcp_config_path.as_ref() else {
        return error_response(
            http::StatusCode::BAD_REQUEST,
            "MCP config is not file-backed",
        );
    };
    let Some(manager_arc) = state.mcp_manager.as_ref() else {
        return error_response(
            http::StatusCode::BAD_REQUEST,
            "MCP manager is unavailable at runtime",
        );
    };

    if !mcp_config_update_enabled() {
        return error_response(
            http::StatusCode::FORBIDDEN,
            "MCP config updates are disabled by default because MCP commands are a privileged execution surface. Set ROUTIIUM_ALLOW_MCP_CONFIG_UPDATE=1 to enable this endpoint for a trusted admin deployment.",
        );
    }

    let config = body.into_inner();
    if let Err(err) = write_pretty_json_file(path, &config) {
        return error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to write MCP config: {}", err),
        );
    }

    let new_manager = match crate::mcp_client::McpClientManager::new(config.clone()).await {
        Ok(manager) => manager,
        Err(err) => {
            return error_response(
                http::StatusCode::BAD_REQUEST,
                &format!("Updated MCP config could not be initialized: {}", err),
            )
        }
    };

    let mut manager_guard = manager_arc.write().await;
    *manager_guard = new_manager;
    drop(manager_guard);

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "config_path": path,
        "configured_servers": config.server_names()
    }))
}

async fn admin_panel_update_routing(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<crate::routing_config::RoutingConfig>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }

    let Some(path) = state.routing_config_path.as_ref() else {
        return error_response(
            http::StatusCode::BAD_REQUEST,
            "Routing config is not file-backed",
        );
    };

    let config = body.into_inner();
    if let Err(err) = write_pretty_json_file(path, &config) {
        return error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to write routing config: {}", err),
        );
    }

    let loaded = match crate::routing_config::RoutingConfig::load_from_file(path) {
        Ok(config) => config,
        Err(err) => {
            return error_response(
                http::StatusCode::BAD_REQUEST,
                &format!("Updated routing config could not be loaded: {}", err),
            )
        }
    };
    let stats = loaded.stats();

    let mut guard = state.routing_config.write().await;
    *guard = loaded.clone();
    drop(guard);

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "config_path": path,
        "stats": stats,
        "config": loaded
    }))
}

/// Configure Actix-web routes with AppState.
pub fn config_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("")
            .route("/health", web::get().to(health))
            .route("/status", web::get().to(status))
            .route("/convert", web::post().to(convert))
            .route(
                "/v1/chat/completions",
                web::post().to(chat_completions_passthrough),
            )
            .route("/models", web::get().to(list_models))
            .route("/v1/models", web::get().to(list_models))
            .route("/v1/responses", web::post().to(responses_passthrough))
            .route("/keys", web::get().to(list_keys))
            .route("/keys/generate", web::post().to(generate_key))
            .route("/keys/generate_batch", web::post().to(generate_key_batch))
            .route("/keys/revoke", web::post().to(revoke_key))
            .route("/keys/set_expiration", web::post().to(set_key_expiration))
            .route("/reload/mcp", web::post().to(reload_mcp))
            .route(
                "/reload/system_prompt",
                web::post().to(reload_system_prompt),
            )
            .route("/reload/routing", web::post().to(reload_routing))
            .route("/reload/all", web::post().to(reload_all))
            .route("/analytics/stats", web::get().to(analytics_stats))
            .route("/analytics/events", web::get().to(analytics_events))
            .route("/analytics/aggregate", web::get().to(analytics_aggregate))
            .route("/analytics/export", web::get().to(analytics_export))
            .route("/analytics/clear", web::post().to(analytics_clear))
            .route("/admin/safety/events", web::get().to(admin_safety_events))
            .route("/chat_history/stats", web::get().to(chat_history_stats))
            .route(
                "/chat_history/conversations",
                web::get().to(chat_history_conversations),
            )
            .route(
                "/chat_history/conversations/{id}",
                web::get().to(chat_history_conversation),
            )
            .route(
                "/chat_history/messages",
                web::get().to(chat_history_messages),
            )
            .route(
                "/chat_history/conversations/{id}",
                web::delete().to(chat_history_delete_conversation),
            )
            .route("/chat_history/clear", web::post().to(chat_history_clear))
            // Rate limit admin endpoints
            .route(
                "/admin/rate-limits/policies",
                web::get().to(rl_list_policies),
            )
            .route(
                "/admin/rate-limits/policies",
                web::post().to(rl_create_policy),
            )
            .route(
                "/admin/rate-limits/policies/{id}",
                web::get().to(rl_get_policy),
            )
            .route(
                "/admin/rate-limits/policies/{id}",
                web::put().to(rl_update_policy),
            )
            .route(
                "/admin/rate-limits/policies/{id}",
                web::delete().to(rl_delete_policy),
            )
            .route(
                "/admin/rate-limits/keys/{key_id}/status",
                web::get().to(rl_key_status),
            )
            .route(
                "/admin/rate-limits/keys/{key_id}",
                web::post().to(rl_assign_key_policy),
            )
            .route(
                "/admin/rate-limits/keys/{key_id}",
                web::delete().to(rl_remove_key_policy),
            )
            .route(
                "/admin/rate-limits/default",
                web::get().to(rl_get_default_policy),
            )
            .route(
                "/admin/rate-limits/default",
                web::post().to(rl_set_default_policy),
            )
            .route(
                "/admin/rate-limits/emergency",
                web::post().to(rl_emergency_block),
            )
            .route(
                "/admin/rate-limits/emergency",
                web::get().to(rl_list_emergency_blocks),
            )
            .route(
                "/admin/rate-limits/emergency/{key_id}",
                web::delete().to(rl_remove_emergency_block),
            )
            .route(
                "/admin/concurrency/keys/{key_id}",
                web::get().to(rl_concurrency_status),
            )
            .route(
                "/admin/rate-limits/reload",
                web::post().to(rl_reload_config),
            )
            .route("/admin/analytics/rate-limits", web::get().to(rl_analytics))
            .route("/admin/panel/state", web::get().to(admin_panel_state))
            .route(
                "/admin/panel/system-prompts",
                web::put().to(admin_panel_update_system_prompts),
            )
            .route("/admin/panel/mcp", web::put().to(admin_panel_update_mcp))
            .route(
                "/admin/panel/routing",
                web::put().to(admin_panel_update_routing),
            ),
    );
}

/// Liveness endpoint for orchestrator probes.
async fn health() -> impl Responder {
    web::Json(serde_json::json!({
        "status": "ok"
    }))
}

/// Service status endpoint to expose feature flags and available routes.
async fn status(state: web::Data<AppState>) -> impl Responder {
    let proxy_enabled: bool = true;
    let routes = vec![
        "/health",
        "/status",
        "/convert",
        "/v1/chat/completions",
        "/models",
        "/v1/models",
        "/v1/responses",
        "/keys",
        "/keys/generate",
        "/keys/generate_batch",
        "/keys/revoke",
        "/keys/set_expiration",
        "/reload/mcp",
        "/reload/system_prompt",
        "/reload/routing",
        "/reload/all",
        "/analytics/stats",
        "/analytics/events",
        "/analytics/aggregate",
        "/analytics/export",
        "/analytics/clear",
        "/admin/safety/events",
        "/chat_history/stats",
        "/chat_history/conversations",
        "/chat_history/conversations/{id}",
        "/chat_history/messages",
        "/chat_history/clear",
        "/admin/panel/state",
        "/admin/panel/system-prompts",
        "/admin/panel/mcp",
        "/admin/panel/routing",
    ];

    // Get current configuration status
    let mcp_enabled = state.mcp_manager.is_some();
    let mcp_config_path = state.mcp_config_path.as_deref();
    let system_prompt_config_path = state.system_prompt_config_path.as_deref();
    let routing_config_path = state.routing_config_path.as_deref();

    let system_prompt_guard = state.system_prompt_config.read().await;
    let system_prompt_enabled = system_prompt_guard.enabled;
    drop(system_prompt_guard);

    // Get routing status
    let routing_guard = state.routing_config.read().await;
    let routing_stats = routing_guard.stats();
    drop(routing_guard);

    let router_enabled = state.router_client.is_some();
    let router_url = state.router_url.as_deref();
    let router_config_path = state.router_config_path.as_deref();
    let router_mode = if state.router_config_path.is_some() {
        "local"
    } else if state.router_url.is_some() {
        "remote"
    } else if router_enabled {
        "embedded"
    } else {
        "none"
    };
    let router_policy = state
        .router_config_path
        .as_ref()
        .map(|p| format!("file://{}", p));
    let pricing_config_path = state.pricing_config_path.as_deref();
    let safety_config = crate::safety_judge::SafetyJudgeConfig::from_env();
    let response_guard_mode = crate::safety_judge::response_guard_mode_from_env();
    let streaming_safety_mode = crate::safety_judge::streaming_safety_mode_from_env();
    let judge_key_present = std::env::var(&safety_config.llm_api_key_env)
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    // Get analytics status
    let analytics_enabled = state.analytics.is_some();
    let analytics_stats = if let Some(mgr) = &state.analytics {
        mgr.stats().await.ok()
    } else {
        None
    };

    let managed_override = std::env::var("ROUTIIUM_MANAGED_MODE")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let managed_mode = managed_mode_from_env();
    let openai_key_present = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some();
    let auth_mode = if managed_mode {
        "managed"
    } else {
        "passthrough"
    };

    web::Json(serde_json::json!({
        "name": "routiium",
        "version": env!("CARGO_PKG_VERSION"),
        "proxy_enabled": proxy_enabled,
        "routes": routes,
        "router": {
            "enabled": router_enabled,
            "mode": router_mode,
            "policy": router_policy,
            "url": router_url,
            "strict": state.router_strict,
            "cache_ttl_ms": state.router_cache_ttl_ms,
            "privacy_mode": state.router_privacy_mode
        },
        "judge": {
            "enabled": safety_config.mode != crate::safety_judge::SafetyMode::Off,
            "mode": safety_config.mode.as_str(),
            "llm_enabled": safety_config.llm_enabled,
            "llm_model": safety_config.llm_model,
            "llm_api_key_env": safety_config.llm_api_key_env,
            "llm_key_present": judge_key_present,
            "safe_target": safety_config.safe_target,
            "sensitive_target": safety_config.sensitive_target,
            "deny_target": safety_config.deny_target,
            "on_deny": safety_config.on_deny.as_str(),
            "rejection_mode": match rejection_mode_from_env() {
                RejectionMode::AgentResult => "agent_result",
                RejectionMode::HttpError => "http_error",
            },
            "policy_fingerprint": safety_config.policy_fingerprint,
            "web_judge": safety_config.web_judge.as_str(),
            "response_guard_mode": response_guard_mode.as_str(),
            "streaming_safety": streaming_safety_mode.as_str()
        },
        "features": {
            "auth": {
                "mode": auth_mode,
                "managed": managed_mode,
                "managed_override": managed_override,
                "openai_key_present": openai_key_present,
                "key_store_available": state.api_keys.is_some()
            },
            "mcp": {
                "enabled": mcp_enabled,
                "config_path": mcp_config_path,
                "reloadable": mcp_config_path.is_some()
            },
            "system_prompt": {
                "enabled": system_prompt_enabled,
                "config_path": system_prompt_config_path,
                "reloadable": system_prompt_config_path.is_some()
            },
            "routing": {
                "enabled": routing_config_path.is_some(),
                "config_path": routing_config_path,
                "reloadable": routing_config_path.is_some(),
                "stats": routing_stats
            },
            "router": {
                "enabled": router_enabled,
                "mode": router_mode,
                "config_path": router_config_path,
                "url": router_url,
                "strict": state.router_strict,
                "cache_ttl_ms": state.router_cache_ttl_ms,
                "privacy_mode": state.router_privacy_mode
            },
            "analytics": {
                "enabled": analytics_enabled,
                "stats": analytics_stats
            },
            "pricing": {
                "enabled": true,
                "config_path": pricing_config_path,
                "models_count": state.pricing.models.len(),
                "source": if pricing_config_path.is_some() { "file" } else { "built_in" }
            }
        }
    }))
}

/// List available models in OpenAI-compatible format.
async fn list_models(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    // Apply same authentication logic as other endpoints
    let managed_mode = managed_mode_from_env();

    // Extract client bearer token
    let client_bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            let s = s.trim();
            if s.len() >= 7 && s[..6].eq_ignore_ascii_case("bearer") {
                Some(s[6..].trim().to_string())
            } else {
                None
            }
        });

    // Authentication check (same as other endpoints)
    if managed_mode {
        if let Some(manager) = &state.api_keys {
            match client_bearer.as_deref().map(|tok| manager.verify(tok)) {
                Some(crate::auth::Verification::Valid { .. }) => {
                    // Valid token, continue
                }
                Some(crate::auth::Verification::Revoked { .. }) => {
                    return error_response(http::StatusCode::UNAUTHORIZED, "API key revoked");
                }
                Some(crate::auth::Verification::Expired { .. }) => {
                    return error_response(http::StatusCode::UNAUTHORIZED, "API key expired");
                }
                Some(_) => {
                    return error_response(http::StatusCode::UNAUTHORIZED, "Invalid API key");
                }
                None => {
                    return error_response(
                        http::StatusCode::UNAUTHORIZED,
                        "Missing Authorization bearer",
                    );
                }
            }
        } else {
            return managed_key_store_unavailable_response("models", &req);
        }
    } else if client_bearer.is_none() {
        return error_response(
            http::StatusCode::UNAUTHORIZED,
            "Missing Authorization bearer",
        );
    }

    // Try to get models from Router catalog if available
    let models = if let Some(router_client) = &state.router_client {
        match router_client.get_catalog().await {
            Ok(catalog) => {
                // Convert Router catalog models to OpenAI format
                catalog
                    .models
                    .into_iter()
                    .map(|catalog_model| Model {
                        id: catalog_model.id.clone(),
                        object: "model".to_string(),
                        created: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        owned_by: catalog_model.provider,
                    })
                    .collect()
            }
            Err(_) => {
                // Fallback to static model list
                get_default_models()
            }
        }
    } else {
        // No Router client, use static model list
        get_default_models()
    };

    let response = ModelsResponse {
        object: "list".to_string(),
        data: models,
    };

    HttpResponse::Ok().json(response)
}

/// Get default model list when Router catalog is unavailable
fn get_default_models() -> Vec<Model> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    vec![
        Model {
            id: "gpt-4o".to_string(),
            object: "model".to_string(),
            created: now,
            owned_by: "openai".to_string(),
        },
        Model {
            id: "gpt-4o-2024-11-20".to_string(),
            object: "model".to_string(),
            created: now,
            owned_by: "openai".to_string(),
        },
        Model {
            id: "gpt-4o-mini".to_string(),
            object: "model".to_string(),
            created: now,
            owned_by: "openai".to_string(),
        },
        Model {
            id: "gpt-4o-mini-2024-07-18".to_string(),
            object: "model".to_string(),
            created: now,
            owned_by: "openai".to_string(),
        },
        Model {
            id: "gpt-4-turbo".to_string(),
            object: "model".to_string(),
            created: now,
            owned_by: "openai".to_string(),
        },
        Model {
            id: "gpt-4-turbo-2024-04-09".to_string(),
            object: "model".to_string(),
            created: now,
            owned_by: "openai".to_string(),
        },
        Model {
            id: "gpt-3.5-turbo".to_string(),
            object: "model".to_string(),
            created: now,
            owned_by: "openai".to_string(),
        },
        Model {
            id: "gpt-3.5-turbo-0125".to_string(),
            object: "model".to_string(),
            created: now,
            owned_by: "openai".to_string(),
        },
    ]
}

/// Convert a Chat Completions request into a Responses API request payload (JSON).
async fn convert(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<ConvertQuery>,
    body: web::Json<ChatCompletionRequest>,
) -> impl Responder {
    let mut converted = if query.include_internal_config {
        if !env_truthy("ROUTIIUM_PUBLIC_CONVERT_INTERNAL_CONFIG") {
            if let Err(resp) = require_admin(&req) {
                return resp;
            }
        }

        let mcp_manager_guard = if let Some(mgr) = state.mcp_manager.as_ref() {
            Some(mgr.read().await)
        } else {
            None
        };

        let system_prompt_guard = state.system_prompt_config.read().await;

        crate::conversion::to_responses_request_with_mcp_and_prompt(
            &body,
            query.conversation_id.clone(),
            mcp_manager_guard.as_deref(),
            Some(&*system_prompt_guard),
        )
        .await
    } else {
        crate::conversion::to_responses_request(&body, query.conversation_id.clone())
    };

    if let Some(prev) = query.previous_response_id.clone() {
        converted.previous_response_id = Some(prev);
    }

    HttpResponse::Ok().json(converted)
}

/// Direct passthrough for native Chat Completions requests (no translation).
async fn chat_completions_passthrough(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<ChatQuery>,
    body: web::Json<serde_json::Value>,
) -> impl Responder {
    let mut body = body.into_inner();
    let started_at = std::time::Instant::now();
    let requested_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let query = query.into_inner();
    let conversation_hint = query.conversation_id.filter(|s| !s.trim().is_empty());
    let previous_response_hint = query.previous_response_id.filter(|s| !s.trim().is_empty());
    let mut system_prompt_applied = false;

    // Apply system prompt injection if configured
    let system_prompt_guard = state.system_prompt_config.read().await;
    let model = body.get("model").and_then(|v| v.as_str());

    if let Some(prompt) = system_prompt_guard.get_prompt(model, Some("chat")) {
        system_prompt_applied = true;
        inject_system_prompt_chat_json(&mut body, &prompt, &system_prompt_guard.injection_mode);
    }
    drop(system_prompt_guard);

    // Remove explicit null content fields (tool call responses don't require them)
    if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        for message in messages {
            if let Some(obj) = message.as_object_mut() {
                if obj
                    .get("content")
                    .map(|value| value.is_null())
                    .unwrap_or(false)
                {
                    obj.insert(
                        "content".to_string(),
                        serde_json::Value::String(String::new()),
                    );
                }
            }
        }
    }

    if let Some(mgr) = state.mcp_manager.as_ref() {
        let manager = mgr.read().await;
        match manager.list_all_tools().await {
            Ok(mcp_tools) => merge_mcp_tools_into_chat_payload(&mut body, &mcp_tools),
            Err(err) => warn!(
                "Failed to fetch MCP tools for /v1/chat/completions: {}",
                err
            ),
        }
    }

    // Determine managed (internal upstream key) vs passthrough mode
    let managed_mode = managed_mode_from_env();
    let authenticated: bool;
    let mut api_key_id: Option<String> = None;
    let mut api_key_label: Option<String> = None;

    // Extract client bearer (could be internal access token or upstream key)
    let client_bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            let s = s.trim();
            if s.len() >= 7 && s[..6].eq_ignore_ascii_case("bearer") {
                Some(s[6..].trim().to_string())
            } else {
                None
            }
        });

    // Resolve upstream bearer (managed mode validates client token but defers provider key selection to routing)
    let upstream_bearer = if managed_mode {
        if let Some(manager) = &state.api_keys {
            match client_bearer.as_deref().map(|tok| manager.verify(tok)) {
                Some(crate::auth::Verification::Valid { id, label, .. }) => {
                    authenticated = true;
                    api_key_id = Some(id);
                    api_key_label = label;
                    None
                }
                Some(crate::auth::Verification::Revoked { .. }) => {
                    tracing::warn!(
                        target: "routiium::auth",
                        api = "chat",
                        client = request_client_ip(&req),
                        "API key revoked"
                    );
                    return error_response(http::StatusCode::UNAUTHORIZED, "API key revoked");
                }
                Some(crate::auth::Verification::Expired { .. }) => {
                    tracing::warn!(
                        target: "routiium::auth",
                        api = "chat",
                        client = request_client_ip(&req),
                        "API key expired"
                    );
                    return error_response(http::StatusCode::UNAUTHORIZED, "API key expired");
                }
                Some(_) => {
                    tracing::warn!(
                        target: "routiium::auth",
                        api = "chat",
                        client = request_client_ip(&req),
                        "Invalid API key"
                    );
                    return error_response(http::StatusCode::UNAUTHORIZED, "Invalid API key");
                }
                None => {
                    tracing::warn!(
                        target: "routiium::auth",
                        api = "chat",
                        client = request_client_ip(&req),
                        "Missing Authorization bearer"
                    );
                    return error_response(
                        http::StatusCode::UNAUTHORIZED,
                        "Missing Authorization bearer",
                    );
                }
            }
        } else {
            return managed_key_store_unavailable_response("chat", &req);
        }
    } else {
        if client_bearer.is_none() {
            tracing::warn!(
                target: "routiium::auth",
                api = "chat",
                client = request_client_ip(&req),
                "Missing Authorization bearer"
            );
            return error_response(
                http::StatusCode::UNAUTHORIZED,
                "Missing Authorization bearer",
            );
        }
        authenticated = true;
        client_bearer.clone()
    };

    // Rate limiting check (after auth, before upstream)
    let rl_result = match check_rate_limits_for_key(
        &state,
        api_key_id.as_deref(),
        req.path(),
        body.get("model").and_then(|v| v.as_str()),
    )
    .await
    {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    // Determine if streaming is requested
    let mut stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let client = &state.http;
    let resolution = match resolve_upstream(&state, "chat", &mut body).await {
        Ok(res) => res,
        Err(err) => {
            record_router_error_event(&state, &req, "chat", &requested_model, &err).await;
            if let Some(response) = router_plan_rejection_response(&err, "chat", &requested_model) {
                return response;
            }
            return router_plan_error_response(&err);
        }
    };

    let streaming_safety =
        if stream && crate::safety_judge::should_force_non_stream(resolution.plan.as_ref()) {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("stream".to_string(), serde_json::json!(false));
            }
            stream = false;
            Some("forced_non_stream")
        } else if stream {
            Some(crate::safety_judge::streaming_safety_mode_from_env().as_str())
        } else {
            None
        };

    log_request_start("chat", &req, &requested_model, &resolution, stream);

    if let Some(obj) = body.as_object_mut() {
        obj.insert(
            "model".to_string(),
            serde_json::json!(resolution.model_id.clone()),
        );
        if streaming_safety == Some("forced_non_stream") {
            obj.insert("stream".to_string(), serde_json::json!(false));
        }
    }

    let mut eff_bearer = upstream_bearer.clone();
    if eff_bearer.is_none() {
        if let Some(key_env) = resolution.key_env.as_deref() {
            if let Ok(v) = std::env::var(key_env) {
                if !v.is_empty() {
                    eff_bearer = Some(v);
                }
            }
        }
    }
    if eff_bearer.is_none() {
        if let Ok(v) = std::env::var("OPENAI_API_KEY") {
            if !v.is_empty() {
                eff_bearer = Some(v);
            }
        }
    }

    // Handle Bedrock mode separately (uses AWS SDK instead of HTTP)
    if matches!(resolution.mode, crate::util::UpstreamMode::Bedrock) {
        // Parse body as Chat Completions request
        let chat_req = match serde_json::from_value::<ChatCompletionRequest>(body.clone()) {
            Ok(req) => req,
            Err(e) => {
                return error_response(
                    http::StatusCode::BAD_REQUEST,
                    &format!("Invalid chat request: {}", e),
                );
            }
        };

        // Convert to Bedrock format
        let (_content_type, bedrock_body) = match crate::bedrock::chat_to_bedrock_request(&chat_req)
        {
            Ok(result) => result,
            Err(e) => {
                return error_response(
                    http::StatusCode::BAD_REQUEST,
                    &format!("Failed to convert to Bedrock format: {}", e),
                );
            }
        };

        // Extract region from base_url or use default
        let region = resolution.base_url.split('.').nth(1).unwrap_or("us-east-1");

        if stream {
            use bytes::Bytes;
            use futures_util::StreamExt;

            let provider =
                match crate::bedrock::BedrockProvider::from_model_id(&resolution.model_id) {
                    Ok(provider) => provider,
                    Err(err) => {
                        return error_response(
                            http::StatusCode::BAD_REQUEST,
                            &format!("Failed to resolve Bedrock provider: {}", err),
                        );
                    }
                };

            match crate::bedrock::invoke_bedrock_model_streaming(
                &resolution.model_id,
                bedrock_body,
                region,
            )
            .await
            {
                Ok(stream) => {
                    let model_id = resolution.model_id.clone();
                    let mapped = stream.map(move |event| match event {
                        Ok(evt) => {
                            if let Some(chunk) = evt.chunk {
                                match crate::bedrock::bedrock_chunk_to_sse(
                                    &chunk, &model_id, &provider,
                                ) {
                                    Ok(sse) => Ok(Bytes::from(sse)),
                                    Err(err) => Err(std::io::Error::other(err.to_string())),
                                }
                            } else if evt.done {
                                Ok(Bytes::from("data: [DONE]\n\n"))
                            } else {
                                Ok(Bytes::from(""))
                            }
                        }
                        Err(err) => Err(std::io::Error::other(err.to_string())),
                    });
                    let mapped = match crate::safety_judge::streaming_safety_mode_from_env() {
                        crate::safety_judge::StreamingSafetyMode::Off => mapped.boxed(),
                        _ => SafetySseGuard::new(mapped.boxed()).boxed(),
                    };

                    let mut response = HttpResponse::Ok();
                    if let Some(plan) = resolution.plan.as_ref() {
                        insert_route_headers(&mut response, plan, &resolution.model_id);
                    }
                    if let Some(value) = streaming_safety {
                        response.insert_header(("x-streaming-safety", value));
                    }

                    record_analytics_event(
                        &state,
                        &req,
                        &body,
                        &requested_model,
                        &resolution,
                        started_at,
                        200,
                        0,
                        None,
                        authenticated,
                        api_key_id.clone(),
                        api_key_label.clone(),
                        system_prompt_applied,
                        None,
                    )
                    .await;

                    return response
                        .insert_header(("content-type", "text/event-stream"))
                        .insert_header(("cache-control", "no-cache"))
                        .insert_header(("connection", "keep-alive"))
                        .streaming(mapped);
                }
                Err(err) => {
                    record_analytics_event(
                        &state,
                        &req,
                        &body,
                        &requested_model,
                        &resolution,
                        started_at,
                        502,
                        0,
                        None,
                        authenticated,
                        api_key_id.clone(),
                        api_key_label.clone(),
                        system_prompt_applied,
                        Some(err.to_string()),
                    )
                    .await;
                    return error_response(
                        http::StatusCode::BAD_GATEWAY,
                        &format!("Bedrock streaming invocation failed: {}", err),
                    );
                }
            }
        }

        match crate::bedrock::invoke_bedrock_model(&resolution.model_id, bedrock_body, region).await
        {
            Ok(bedrock_response) => match crate::bedrock::bedrock_to_chat_response(
                bedrock_response,
                &resolution.model_id,
                None,
            ) {
                Ok(chat_response) => {
                    // Record chat history for Bedrock responses
                    let response_json = serde_json::to_value(&chat_response).unwrap_or_default();
                    let token_usage =
                        crate::analytics_middleware::extract_token_usage(&response_json);
                    let response_bytes = serde_json::to_vec(&chat_response).unwrap_or_default();
                    let response_guard = crate::safety_judge::guard_response_bytes(&response_bytes);
                    if response_guard.should_block() {
                        tracing::warn!(
                            target: "routiium::safety",
                            guard_id = %response_guard.id,
                            risk = %response_guard.risk_level,
                            categories = ?response_guard.categories,
                            "bedrock chat output blocked by response guard"
                        );
                        record_analytics_event(
                            &state,
                            &req,
                            &body,
                            &requested_model,
                            &resolution,
                            started_at,
                            403,
                            0,
                            None,
                            authenticated,
                            api_key_id.clone(),
                            api_key_label.clone(),
                            system_prompt_applied,
                            Some(response_guard.reason.clone()),
                        )
                        .await;
                        record_response_guard_event(
                            &state,
                            &req,
                            "chat",
                            &requested_model,
                            &resolution,
                            &response_guard,
                        )
                        .await;
                        return response_guard_error_response(
                            &response_guard,
                            resolution.plan.as_ref(),
                            &resolution.model_id,
                            "chat",
                        );
                    }
                    record_chat_history(
                        &state.chat_history,
                        conversation_hint.clone(),
                        &requested_model,
                        &body,
                        &response_json,
                        &resolution,
                        token_usage.as_ref(),
                    )
                    .await;

                    record_analytics_event(
                        &state,
                        &req,
                        &body,
                        &requested_model,
                        &resolution,
                        started_at,
                        200,
                        serde_json::to_vec(&chat_response)
                            .map(|v| v.len())
                            .unwrap_or(0),
                        token_usage,
                        authenticated,
                        api_key_id.clone(),
                        api_key_label.clone(),
                        system_prompt_applied,
                        None,
                    )
                    .await;

                    let mut builder = HttpResponse::Ok();
                    if let Some(plan) = resolution.plan.as_ref() {
                        insert_route_headers(&mut builder, plan, &resolution.model_id);
                    }
                    insert_response_guard_headers(&mut builder, &response_guard);
                    if let Some(value) = streaming_safety {
                        builder.insert_header(("x-streaming-safety", value));
                    }
                    return builder.json(chat_response);
                }
                Err(e) => {
                    return error_response(
                        http::StatusCode::INTERNAL_SERVER_ERROR,
                        &format!("Failed to convert Bedrock response: {}", e),
                    );
                }
            },
            Err(e) => {
                record_analytics_event(
                    &state,
                    &req,
                    &body,
                    &requested_model,
                    &resolution,
                    started_at,
                    502,
                    0,
                    None,
                    authenticated,
                    api_key_id.clone(),
                    api_key_label.clone(),
                    system_prompt_applied,
                    Some(e.to_string()),
                )
                .await;
                return error_response(
                    http::StatusCode::BAD_GATEWAY,
                    &format!("Bedrock invocation failed: {}", e),
                );
            }
        }
    }

    let endpoint = match resolution.mode {
        crate::util::UpstreamMode::Responses => "responses",
        crate::util::UpstreamMode::Chat => "chat/completions",
        crate::util::UpstreamMode::Bedrock => "bedrock/invoke", // Shouldn't reach here
    };
    let base = resolution.base_url.trim_end_matches('/');

    let expects_responses = resolution
        .plan
        .as_ref()
        .map(|plan| matches!(plan.upstream.mode, RouterUpstreamMode::Responses))
        .unwrap_or_else(|| matches!(resolution.mode, crate::util::UpstreamMode::Responses));

    if stream {
        use bytes::Bytes;
        use futures_util::{stream::StreamExt, TryStreamExt};

        let mut outbound_body = body.clone();
        if expects_responses {
            if let Some(converted) = convert_chat_payload_to_responses(
                &outbound_body,
                conversation_hint.clone(),
                previous_response_hint.clone(),
            ) {
                outbound_body = converted;
            } else {
                warn!("Failed to convert chat payload to Responses request for router plan");
            }
        } else {
            strip_responses_only_fields(&mut outbound_body);
        }

        let real_url = format!("{}/{}", base, endpoint);
        let mut rb = client
            .post(&real_url)
            .header("accept", "text/event-stream")
            .header("content-type", "application/json")
            .header("connection", "close")
            .json(&outbound_body);
        rb = apply_upstream_headers(rb, &resolution.headers);
        if let Some(b) = eff_bearer.clone() {
            rb = rb.bearer_auth(b);
        }
        match rb.send().await {
            Ok(up) => {
                let status = up.status();
                if !status.is_success() {
                    let bytes = up.bytes().await.unwrap_or_default();
                    let response_json = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
                    let token_usage = response_json
                        .as_ref()
                        .and_then(crate::analytics_middleware::extract_token_usage);
                    let error_message = extract_error_message(response_json.as_ref(), &bytes);
                    record_analytics_event(
                        &state,
                        &req,
                        &body,
                        &requested_model,
                        &resolution,
                        started_at,
                        status.as_u16(),
                        bytes.len(),
                        token_usage,
                        authenticated,
                        api_key_id.clone(),
                        api_key_label.clone(),
                        system_prompt_applied,
                        error_message,
                    )
                    .await;

                    let mut builder = HttpResponse::build(
                        actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap(),
                    );
                    if let Some(plan) = resolution.plan.as_ref() {
                        insert_route_headers(&mut builder, plan, &resolution.model_id);
                    }
                    return builder.body(bytes);
                }
                let upstream_ct = up.headers().get("content-type").cloned();
                let base_stream = up
                    .bytes_stream()
                    .map_err(|e| std::io::Error::other(e.to_string()))
                    .map_ok(Bytes::from);

                let stream = if expects_responses {
                    ResponsesSseToChatSse::new(base_stream).boxed()
                } else {
                    base_stream.boxed()
                };
                let stream = match crate::safety_judge::streaming_safety_mode_from_env() {
                    crate::safety_judge::StreamingSafetyMode::Off => stream,
                    _ => SafetySseGuard::new(stream).boxed(),
                };

                let mut response = HttpResponse::Ok();
                if let Some(ct) = upstream_ct {
                    if let Ok(ct_str) = ct.to_str() {
                        response.insert_header(("content-type", ct_str));
                    } else {
                        response.insert_header(("content-type", "text/event-stream"));
                    }
                } else {
                    response.insert_header(("content-type", "text/event-stream"));
                }

                response
                    .insert_header(("cache-control", "no-cache"))
                    .insert_header(("connection", "keep-alive"));
                if let Some(plan) = resolution.plan.as_ref() {
                    insert_route_headers(&mut response, plan, &resolution.model_id);
                }
                if let Some(value) = streaming_safety {
                    response.insert_header(("x-streaming-safety", value));
                }
                if let Some(ref rl) = rl_result {
                    insert_rate_limit_headers(&mut response, rl);
                }

                record_analytics_event(
                    &state,
                    &req,
                    &body,
                    &requested_model,
                    &resolution,
                    started_at,
                    status.as_u16(),
                    0,
                    None,
                    authenticated,
                    api_key_id.clone(),
                    api_key_label.clone(),
                    system_prompt_applied,
                    None,
                )
                .await;

                response.streaming(stream)
            }
            Err(e) => {
                record_analytics_event(
                    &state,
                    &req,
                    &body,
                    &requested_model,
                    &resolution,
                    started_at,
                    502,
                    0,
                    None,
                    authenticated,
                    api_key_id.clone(),
                    api_key_label.clone(),
                    system_prompt_applied,
                    Some(e.to_string()),
                )
                .await;
                router_error_response(
                    http::StatusCode::BAD_GATEWAY,
                    &e.to_string(),
                    resolution.plan.as_ref(),
                    &resolution.model_id,
                )
            }
        }
    } else {
        let mut outbound_body = body.clone();
        if expects_responses {
            if let Some(converted) = convert_chat_payload_to_responses(
                &outbound_body,
                conversation_hint.clone(),
                previous_response_hint.clone(),
            ) {
                outbound_body = converted;
            } else {
                warn!("Failed to convert chat payload to Responses request for router plan");
            }
        } else {
            strip_responses_only_fields(&mut outbound_body);
        }

        let real_url = format!("{}/{}", base, endpoint);
        let mut upstream_req = client
            .post(&real_url)
            .header("content-type", "application/json");
        upstream_req = apply_upstream_headers(upstream_req, &resolution.headers);
        if let Some(b) = eff_bearer {
            upstream_req = upstream_req.bearer_auth(b);
        }
        match upstream_req.json(&outbound_body).send().await {
            Ok(up) => {
                let status = up.status();
                let bytes = up.bytes().await.unwrap_or_default();
                let response_json = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
                let token_usage = response_json
                    .as_ref()
                    .and_then(crate::analytics_middleware::extract_token_usage);

                let response_guard = if status.is_success() {
                    let decision = crate::safety_judge::guard_response_bytes(&bytes);
                    if decision.should_block() {
                        tracing::warn!(
                            target: "routiium::safety",
                            guard_id = %decision.id,
                            risk = %decision.risk_level,
                            categories = ?decision.categories,
                            "chat output blocked by response guard"
                        );
                        record_analytics_event(
                            &state,
                            &req,
                            &body,
                            &requested_model,
                            &resolution,
                            started_at,
                            403,
                            0,
                            None,
                            authenticated,
                            api_key_id.clone(),
                            api_key_label.clone(),
                            system_prompt_applied,
                            Some(decision.reason.clone()),
                        )
                        .await;
                        record_response_guard_event(
                            &state,
                            &req,
                            "chat",
                            &requested_model,
                            &resolution,
                            &decision,
                        )
                        .await;
                        return response_guard_error_response(
                            &decision,
                            resolution.plan.as_ref(),
                            &resolution.model_id,
                            "chat",
                        );
                    }
                    Some(decision)
                } else {
                    None
                };

                // Record chat history for successful non-streaming responses
                // This must happen BEFORE any early returns below
                if status.is_success() {
                    if let Some(ref response_json) = response_json {
                        record_chat_history(
                            &state.chat_history,
                            conversation_hint.clone(),
                            &requested_model,
                            &body,
                            response_json,
                            &resolution,
                            token_usage.as_ref(),
                        )
                        .await;
                    }
                }

                let error_message = if status.is_success() {
                    None
                } else {
                    extract_error_message(response_json.as_ref(), &bytes)
                };
                record_analytics_event(
                    &state,
                    &req,
                    &body,
                    &requested_model,
                    &resolution,
                    started_at,
                    status.as_u16(),
                    bytes.len(),
                    token_usage.clone(),
                    authenticated,
                    api_key_id.clone(),
                    api_key_label.clone(),
                    system_prompt_applied,
                    error_message,
                )
                .await;

                let mut builder = HttpResponse::build(
                    actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap(),
                );
                if let Some(plan) = resolution.plan.as_ref() {
                    insert_route_headers(&mut builder, plan, &resolution.model_id);
                }
                if let Some(decision) = response_guard.as_ref() {
                    insert_response_guard_headers(&mut builder, decision);
                }
                if let Some(value) = streaming_safety {
                    builder.insert_header(("x-streaming-safety", value));
                }
                if status.is_success() && expects_responses {
                    match serde_json::from_slice::<responses::ResponsesResponse>(&bytes) {
                        Ok(resp_obj) => {
                            let chat_resp = responses_to_chat_response(&resp_obj);
                            match serde_json::to_vec(&chat_resp) {
                                Ok(body) => {
                                    builder.insert_header(("content-type", "application/json"));
                                    return builder.body(body);
                                }
                                Err(err) => tracing::debug!(
                                    "Failed to serialize chat conversion payload: {}",
                                    err
                                ),
                            }
                        }
                        Err(err) => tracing::debug!(
                            "Failed to parse Responses payload for chat conversion: {}",
                            err
                        ),
                    }
                }

                // For Gemini models, parse thought tags from the response
                if status.is_success()
                    && !expects_responses
                    && resolution.model_id.starts_with("gemini-")
                {
                    if let Ok(mut chat_resp) = serde_json::from_slice::<
                        crate::models::chat::ChatCompletionResponse,
                    >(&bytes)
                    {
                        // Parse thought tags from each choice's message content
                        for choice in &mut chat_resp.choices {
                            if let Some(content) = choice.message.content.clone() {
                                let (actual_content, reasoning) =
                                    crate::conversion::parse_thought_tags(&content);
                                choice.message.content = if actual_content.is_empty() {
                                    None
                                } else {
                                    Some(actual_content)
                                };
                                choice.message.reasoning = reasoning;

                                // Estimate reasoning tokens if we have reasoning content
                                if let (Some(reasoning_text), Some(ref mut usage)) =
                                    (choice.message.reasoning.as_ref(), chat_resp.usage.as_mut())
                                {
                                    // Rough estimate: 1 token per 4 characters
                                    let reasoning_tokens =
                                        (reasoning_text.len() as u64).div_ceil(4);
                                    usage.reasoning_tokens = Some(reasoning_tokens);
                                }
                            }
                        }

                        match serde_json::to_vec(&chat_resp) {
                            Ok(body) => {
                                builder.insert_header(("content-type", "application/json"));
                                if let Some(ref rl) = rl_result {
                                    insert_rate_limit_headers(&mut builder, rl);
                                }
                                return builder.body(body);
                            }
                            Err(err) => tracing::debug!(
                                "Failed to serialize Gemini thought-parsed payload: {}",
                                err
                            ),
                        }
                    }
                }

                if let Some(ref rl) = rl_result {
                    insert_rate_limit_headers(&mut builder, rl);
                }
                builder.body(bytes)
            }
            Err(e) => {
                record_analytics_event(
                    &state,
                    &req,
                    &body,
                    &requested_model,
                    &resolution,
                    started_at,
                    502,
                    0,
                    None,
                    authenticated,
                    api_key_id,
                    api_key_label,
                    system_prompt_applied,
                    Some(e.to_string()),
                )
                .await;
                router_error_response(
                    http::StatusCode::BAD_GATEWAY,
                    &e.to_string(),
                    resolution.plan.as_ref(),
                    &resolution.model_id,
                )
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct GenerateKeyRequest {
    label: Option<String>,
    ttl_seconds: Option<u64>,
    expires_at: Option<u64>,
    scopes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct GenerateKeyBatchRequest {
    labels: Vec<String>,
    label_prefix: Option<String>,
    ttl_seconds: Option<u64>,
    expires_at: Option<u64>,
    scopes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
struct ListKeysQuery {
    label: Option<String>,
    label_prefix: Option<String>,
    include_revoked: Option<bool>,
}

fn resolve_ttl_seconds(
    expires_at: Option<u64>,
    ttl_seconds: Option<u64>,
) -> Result<Option<u64>, HttpResponse> {
    // Env flag to require expiration at creation
    let require_exp = std::env::var("ROUTIIUM_KEYS_REQUIRE_EXPIRATION")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false);

    // Optional default TTL (seconds) from env
    let default_ttl_secs: Option<u64> = std::env::var("ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Determine ttl based on precedence: expires_at > ttl_seconds > env default
    let ttl_seconds = if let Some(exp) = expires_at {
        if exp <= now {
            return Err(error_response(
                http::StatusCode::BAD_REQUEST,
                "expires_at must be in the future",
            ));
        }
        Some(exp.saturating_sub(now))
    } else if let Some(ttl) = ttl_seconds {
        Some(ttl)
    } else {
        default_ttl_secs
    };

    if require_exp && ttl_seconds.is_none() {
        return Err(error_response(
            http::StatusCode::BAD_REQUEST,
            "Expiration required: provide expires_at or ttl_seconds (or configure default TTL)",
        ));
    }

    Ok(ttl_seconds)
}

async fn generate_key(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<GenerateKeyRequest>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let payload = body.into_inner();
    let ttl_seconds = match resolve_ttl_seconds(payload.expires_at, payload.ttl_seconds) {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    match &state.api_keys {
        Some(mgr) => match mgr.generate_key(
            payload.label,
            ttl_seconds.map(std::time::Duration::from_secs),
            payload.scopes,
        ) {
            Ok(gen) => HttpResponse::Ok().json(gen),
            Err(e) => error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("failed to generate key: {}", e),
            ),
        },
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "API key manager unavailable",
        ),
    }
}

async fn generate_key_batch(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<GenerateKeyBatchRequest>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let payload = body.into_inner();

    if payload.labels.is_empty() {
        return error_response(
            http::StatusCode::BAD_REQUEST,
            "labels must include at least one entry",
        );
    }

    let ttl_seconds = match resolve_ttl_seconds(payload.expires_at, payload.ttl_seconds) {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let prefix = payload.label_prefix.unwrap_or_default();
    let labels: Vec<String> = payload
        .labels
        .into_iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .map(|l| format!("{}{}", prefix, l))
        .collect();

    if labels.is_empty() {
        return error_response(
            http::StatusCode::BAD_REQUEST,
            "labels must include at least one non-empty entry",
        );
    }

    match &state.api_keys {
        Some(mgr) => {
            let mut out = Vec::with_capacity(labels.len());
            for label in labels {
                match mgr.generate_key(
                    Some(label),
                    ttl_seconds.map(std::time::Duration::from_secs),
                    payload.scopes.clone(),
                ) {
                    Ok(gen) => out.push(gen),
                    Err(e) => {
                        return error_response(
                            http::StatusCode::INTERNAL_SERVER_ERROR,
                            &format!("failed to generate key: {}", e),
                        );
                    }
                }
            }
            HttpResponse::Ok().json(out)
        }
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "API key manager unavailable",
        ),
    }
}

async fn list_keys(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<ListKeysQuery>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    match &state.api_keys {
        Some(mgr) => match mgr.list_keys() {
            Ok(items) => {
                let label = query
                    .label
                    .as_ref()
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty());
                let label_prefix = query
                    .label_prefix
                    .as_ref()
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty());
                let include_revoked = query.include_revoked.unwrap_or(true);

                let mut filtered: Vec<_> = items
                    .into_iter()
                    .filter(|item| {
                        if !include_revoked && item.revoked_at.is_some() {
                            return false;
                        }
                        if let Some(label) = label {
                            return item.label.as_deref() == Some(label);
                        }
                        if let Some(prefix) = label_prefix {
                            return item
                                .label
                                .as_ref()
                                .map(|val| val.starts_with(prefix))
                                .unwrap_or(false);
                        }
                        true
                    })
                    .collect();

                if let Some(rl_mgr) = state.rate_limit_manager.as_ref() {
                    for item in &mut filtered {
                        item.rate_limit_policy =
                            rl_mgr.get_key_policy_id(&item.id).await.ok().flatten();
                    }
                }

                HttpResponse::Ok().json(filtered)
            }
            Err(e) => error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("failed to list keys: {}", e),
            ),
        },
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "API key manager unavailable",
        ),
    }
}

#[derive(Debug, Deserialize)]
struct RevokeKeyRequest {
    id: String,
}

async fn revoke_key(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<RevokeKeyRequest>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let payload = body.into_inner();

    match &state.api_keys {
        Some(mgr) => {
            match mgr.revoke(&payload.id) {
                Ok(true) => HttpResponse::Ok()
                    .json(serde_json::json!({ "revoked": true, "id": payload.id })),
                Ok(false) => HttpResponse::Ok()
                    .json(serde_json::json!({ "revoked": false, "id": payload.id })),
                Err(e) => error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("failed to revoke: {}", e),
                ),
            }
        }
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "API key manager unavailable",
        ),
    }
}

#[derive(Debug, Deserialize)]
struct SetExpirationRequest {
    id: String,
    expires_at: Option<u64>,
    ttl_seconds: Option<u64>,
}

async fn set_key_expiration(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<SetExpirationRequest>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let payload = body.into_inner();

    let new_exp = if let Some(at) = payload.expires_at {
        Some(at)
    } else if let Some(ttl) = payload.ttl_seconds {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Some(now.saturating_add(ttl))
    } else {
        None
    };

    match &state.api_keys {
        Some(mgr) => match mgr.set_expiration(&payload.id, new_exp) {
            Ok(true) => HttpResponse::Ok().json(
                serde_json::json!({ "updated": true, "id": payload.id, "expires_at": new_exp }),
            ),
            Ok(false) => {
                HttpResponse::Ok().json(serde_json::json!({ "updated": false, "id": payload.id }))
            }
            Err(e) => error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("failed to set expiration: {}", e),
            ),
        },
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "API key manager unavailable",
        ),
    }
}

/// Reload MCP configuration from file at runtime
async fn reload_mcp(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let config_path = match &state.mcp_config_path {
        Some(path) => path.clone(),
        None => {
            return error_response(
                http::StatusCode::BAD_REQUEST,
                "No MCP config path configured - cannot reload",
            );
        }
    };

    tracing::info!("Reloading MCP configuration from: {}", config_path);

    // Load new config
    let config = match crate::mcp_config::McpConfig::load_from_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load MCP config: {}", e);
            return error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to load MCP config: {}", e),
            );
        }
    };

    // Create new MCP client manager
    let new_manager = match crate::mcp_client::McpClientManager::new(config).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to initialize MCP client manager: {}", e);
            return error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to initialize MCP manager: {}", e),
            );
        }
    };

    // Get connected servers for response
    let connected_servers = new_manager.connected_servers();
    let server_count = connected_servers.len();

    // Replace the manager
    if let Some(manager_arc) = &state.mcp_manager {
        let mut manager_guard = manager_arc.write().await;
        *manager_guard = new_manager;
        tracing::info!(
            "MCP configuration reloaded successfully with {} servers",
            server_count
        );

        HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "message": "MCP configuration reloaded",
            "servers": connected_servers,
            "count": server_count
        }))
    } else {
        error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "MCP manager not initialized",
        )
    }
}

/// Reload system prompt configuration from file at runtime
async fn reload_system_prompt(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let config_path = match &state.system_prompt_config_path {
        Some(path) => path.clone(),
        None => {
            return error_response(
                http::StatusCode::BAD_REQUEST,
                "No system prompt config path configured - cannot reload",
            );
        }
    };

    tracing::info!(
        "Reloading system prompt configuration from: {}",
        config_path
    );

    // Load new config
    let config = match crate::system_prompt_config::SystemPromptConfig::load_from_file(&config_path)
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load system prompt config: {}", e);
            return error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to load system prompt config: {}", e),
            );
        }
    };

    // Replace the config
    let mut config_guard = state.system_prompt_config.write().await;
    *config_guard = config.clone();

    tracing::info!("System prompt configuration reloaded successfully");

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "System prompt configuration reloaded",
        "enabled": config.enabled,
        "has_global": config.global.is_some(),
        "per_model_count": config.per_model.len(),
        "per_api_count": config.per_api.len(),
        "injection_mode": config.injection_mode
    }))
}

/// Reload routing configuration from file at runtime
async fn reload_routing(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let config_path = match &state.routing_config_path {
        Some(path) => path.clone(),
        None => {
            return error_response(
                http::StatusCode::BAD_REQUEST,
                "No routing config path configured - cannot reload",
            );
        }
    };

    tracing::info!("Reloading routing configuration from: {}", config_path);

    // Load new config
    let config = match crate::routing_config::RoutingConfig::load_from_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load routing config: {}", e);
            return error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to load routing config: {}", e),
            );
        }
    };

    // Replace the config
    let mut config_guard = state.routing_config.write().await;
    *config_guard = config.clone();

    tracing::info!("Routing configuration reloaded successfully");

    let stats = config.stats();

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "Routing configuration reloaded",
        "stats": stats
    }))
}

/// Reload both MCP and system prompt configurations
async fn reload_all(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mut results = serde_json::json!({
        "mcp": { "success": false, "message": "Not attempted" },
        "system_prompt": { "success": false, "message": "Not attempted" },
        "routing": { "success": false, "message": "Not attempted" }
    });

    // Reload MCP if path is configured
    if let Some(mcp_path) = &state.mcp_config_path {
        tracing::info!("Reloading MCP configuration from: {}", mcp_path);

        match crate::mcp_config::McpConfig::load_from_file(mcp_path) {
            Ok(config) => match crate::mcp_client::McpClientManager::new(config).await {
                Ok(new_manager) => {
                    let connected_servers = new_manager.connected_servers();
                    let server_count = connected_servers.len();

                    if let Some(manager_arc) = &state.mcp_manager {
                        let mut manager_guard = manager_arc.write().await;
                        *manager_guard = new_manager;

                        results["mcp"] = serde_json::json!({
                            "success": true,
                            "message": "MCP configuration reloaded",
                            "servers": connected_servers,
                            "count": server_count
                        });

                        tracing::info!("MCP configuration reloaded successfully");
                    } else {
                        results["mcp"] = serde_json::json!({
                            "success": false,
                            "message": "MCP manager not initialized"
                        });
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to initialize MCP client manager: {}", e);
                    results["mcp"] = serde_json::json!({
                        "success": false,
                        "message": format!("Failed to initialize MCP manager: {}", e)
                    });
                }
            },
            Err(e) => {
                tracing::error!("Failed to load MCP config: {}", e);
                results["mcp"] = serde_json::json!({
                    "success": false,
                    "message": format!("Failed to load MCP config: {}", e)
                });
            }
        }
    } else {
        results["mcp"] = serde_json::json!({
            "success": false,
            "message": "No MCP config path configured"
        });
    }

    // Reload system prompt if path is configured
    if let Some(prompt_path) = &state.system_prompt_config_path {
        tracing::info!(
            "Reloading system prompt configuration from: {}",
            prompt_path
        );

        match crate::system_prompt_config::SystemPromptConfig::load_from_file(prompt_path) {
            Ok(config) => {
                let mut config_guard = state.system_prompt_config.write().await;
                *config_guard = config.clone();

                results["system_prompt"] = serde_json::json!({
                    "success": true,
                    "message": "System prompt configuration reloaded",
                    "enabled": config.enabled,
                    "has_global": config.global.is_some(),
                    "per_model_count": config.per_model.len(),
                    "per_api_count": config.per_api.len(),
                    "injection_mode": config.injection_mode
                });

                tracing::info!("System prompt configuration reloaded successfully");
            }
            Err(e) => {
                tracing::error!("Failed to load system prompt config: {}", e);
                results["system_prompt"] = serde_json::json!({
                    "success": false,
                    "message": format!("Failed to load system prompt config: {}", e)
                });
            }
        }
    } else {
        results["system_prompt"] = serde_json::json!({
            "success": false,
            "message": "No system prompt config path configured"
        });
    }

    // Reload routing if path is configured
    if let Some(routing_path) = &state.routing_config_path {
        tracing::info!("Reloading routing configuration from: {}", routing_path);

        match crate::routing_config::RoutingConfig::load_from_file(routing_path) {
            Ok(config) => {
                let mut config_guard = state.routing_config.write().await;
                *config_guard = config.clone();
                let stats = config.stats();

                results["routing"] = serde_json::json!({
                    "success": true,
                    "message": "Routing configuration reloaded",
                    "stats": stats
                });

                tracing::info!("Routing configuration reloaded successfully");
            }
            Err(e) => {
                results["routing"] = serde_json::json!({
                    "success": false,
                    "message": format!("Failed to reload routing config: {}", e)
                });
                tracing::error!("Failed to reload routing config: {}", e);
            }
        }
    }

    HttpResponse::Ok().json(results)
}

/// Analytics endpoints
///
/// Get analytics statistics
async fn analytics_stats(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    match &state.analytics {
        Some(mgr) => match mgr.stats().await {
            Ok(stats) => HttpResponse::Ok().json(stats),
            Err(e) => error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to get analytics stats: {}", e),
            ),
        },
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "Analytics not enabled",
        ),
    }
}

#[derive(Debug, Deserialize)]
struct AnalyticsEventsQuery {
    /// Start timestamp (unix seconds)
    start: Option<u64>,
    /// End timestamp (unix seconds)
    end: Option<u64>,
    /// Maximum number of events to return
    limit: Option<usize>,
}

/// Query analytics events
async fn analytics_events(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<AnalyticsEventsQuery>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    match &state.analytics {
        Some(mgr) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let start = query.start.unwrap_or(now.saturating_sub(3600)); // Default: last hour
            let end = query.end.unwrap_or(now);

            match mgr.query_range(start, end, query.limit).await {
                Ok(events) => HttpResponse::Ok().json(serde_json::json!({
                    "events": events,
                    "count": events.len(),
                    "start": start,
                    "end": end
                })),
                Err(e) => error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("Failed to query events: {}", e),
                ),
            }
        }
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "Analytics not enabled",
        ),
    }
}

#[derive(Debug, Deserialize)]
struct AnalyticsAggregateQuery {
    /// Start timestamp (unix seconds)
    start: Option<u64>,
    /// End timestamp (unix seconds)
    end: Option<u64>,
}

/// Get aggregated analytics
async fn analytics_aggregate(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<AnalyticsAggregateQuery>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    match &state.analytics {
        Some(mgr) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let start = query.start.unwrap_or(now.saturating_sub(3600)); // Default: last hour
            let end = query.end.unwrap_or(now);

            match mgr.aggregate(start, end).await {
                Ok(agg) => HttpResponse::Ok().json(agg),
                Err(e) => error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("Failed to aggregate analytics: {}", e),
                ),
            }
        }
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "Analytics not enabled",
        ),
    }
}

#[derive(Debug, Deserialize)]
struct AnalyticsExportQuery {
    /// Start timestamp (unix seconds)
    start: Option<u64>,
    /// End timestamp (unix seconds)
    end: Option<u64>,
    /// Export format (json, csv)
    format: Option<String>,
}

/// Export analytics data
async fn analytics_export(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<AnalyticsExportQuery>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    match &state.analytics {
        Some(mgr) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let start = query.start.unwrap_or(now.saturating_sub(86400)); // Default: last 24h
            let end = query.end.unwrap_or(now);
            let format = query.format.as_deref().unwrap_or("json");

            match mgr.query_range(start, end, None).await {
                Ok(events) => match format {
                    "csv" => {
                        // Generate CSV export
                        let mut csv_output = csv_row([
                            "id",
                            "timestamp",
                            "endpoint",
                            "method",
                            "model",
                            "stream",
                            "status_code",
                            "success",
                            "duration_ms",
                            "tokens_per_second",
                            "prompt_tokens",
                            "completion_tokens",
                            "cached_tokens",
                            "reasoning_tokens",
                            "total_cost",
                            "backend",
                            "upstream_mode",
                        ]);

                        for event in events {
                            let model = event.request.model.as_deref().unwrap_or("");
                            let status =
                                event.response.as_ref().map(|r| r.status_code).unwrap_or(0);
                            let success =
                                event.response.as_ref().map(|r| r.success).unwrap_or(false);

                            let (prompt_tokens, completion_tokens, cached_tokens, reasoning_tokens) =
                                if let Some(ref usage) = event.token_usage {
                                    (
                                        usage.prompt_tokens,
                                        usage.completion_tokens,
                                        usage.cached_tokens.unwrap_or(0),
                                        usage.reasoning_tokens.unwrap_or(0),
                                    )
                                } else {
                                    let input = event.request.input_tokens.unwrap_or(0);
                                    let output = event
                                        .response
                                        .as_ref()
                                        .and_then(|r| r.output_tokens)
                                        .unwrap_or(0);
                                    (input, output, 0, 0)
                                };

                            let tps = event
                                .performance
                                .tokens_per_second
                                .map(|t| format!("{:.2}", t))
                                .unwrap_or_else(|| "".to_string());

                            let cost = event
                                .cost
                                .as_ref()
                                .map(|c| format!("{:.6}", c.total_cost))
                                .unwrap_or_else(|| "".to_string());

                            csv_output.push_str(&csv_row([
                                event.id,
                                event.timestamp.to_string(),
                                event.request.endpoint,
                                event.request.method,
                                model.to_string(),
                                event.request.stream.to_string(),
                                status.to_string(),
                                success.to_string(),
                                event.performance.duration_ms.to_string(),
                                tps,
                                prompt_tokens.to_string(),
                                completion_tokens.to_string(),
                                cached_tokens.to_string(),
                                reasoning_tokens.to_string(),
                                cost,
                                event.routing.backend,
                                event.routing.upstream_mode,
                            ]));
                        }

                        HttpResponse::Ok()
                            .insert_header(("content-type", "text/csv"))
                            .insert_header((
                                "content-disposition",
                                format!(
                                    "attachment; filename=\"analytics_{}_to_{}.csv\"",
                                    start, end
                                ),
                            ))
                            .body(csv_output)
                    }
                    _ => {
                        // Default JSON export
                        HttpResponse::Ok()
                            .insert_header(("content-type", "application/json"))
                            .insert_header((
                                "content-disposition",
                                format!(
                                    "attachment; filename=\"analytics_{}_to_{}.json\"",
                                    start, end
                                ),
                            ))
                            .json(serde_json::json!({
                                "events": events,
                                "count": events.len(),
                                "period": {
                                    "start": start,
                                    "end": end
                                }
                            }))
                    }
                },
                Err(e) => error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("Failed to export analytics: {}", e),
                ),
            }
        }
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "Analytics not enabled",
        ),
    }
}

/// Clear all analytics data
async fn analytics_clear(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    match &state.analytics {
        Some(mgr) => match mgr.clear().await {
            Ok(_) => HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "Analytics data cleared"
            })),
            Err(e) => error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to clear analytics: {}", e),
            ),
        },
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "Analytics not enabled",
        ),
    }
}

/// Query parameters for chat history endpoints
#[derive(Debug, Deserialize)]
struct ChatHistoryConversationsQuery {
    /// Start timestamp (unix seconds)
    start: Option<u64>,
    /// End timestamp (unix seconds)
    end: Option<u64>,
    /// Maximum number of conversations to return
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ChatHistoryMessagesQuery {
    /// Filter by conversation ID
    conversation_id: Option<String>,
    /// Maximum number of messages to return
    limit: Option<usize>,
}

/// Get chat history stats
async fn chat_history_stats(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    match &state.chat_history {
        Some(mgr) => match mgr.stats().await {
            Ok(stats) => HttpResponse::Ok().json(serde_json::json!({
                "total_conversations": stats.total_conversations,
                "total_messages": stats.total_messages,
                "backend_type": stats.backend_type,
                "storage_path": stats.storage_path,
            })),
            Err(e) => error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to get chat history stats: {}", e),
            ),
        },
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "Chat history not enabled",
        ),
    }
}

/// Query chat conversations
async fn chat_history_conversations(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<ChatHistoryConversationsQuery>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    use crate::chat_history::ConversationFilters;

    match &state.chat_history {
        Some(mgr) => {
            let filters = ConversationFilters {
                start_time: query.start,
                end_time: query.end,
                limit: query.limit,
            };

            match mgr.list_conversations(&filters).await {
                Ok(conversations) => HttpResponse::Ok().json(serde_json::json!({
                    "conversations": conversations,
                    "count": conversations.len(),
                })),
                Err(e) => error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("Failed to query conversations: {}", e),
                ),
            }
        }
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "Chat history not enabled",
        ),
    }
}

/// Get a specific conversation
async fn chat_history_conversation(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let conversation_id = path.into_inner();

    match &state.chat_history {
        Some(mgr) => match mgr.get_conversation(&conversation_id).await {
            Ok(conversation) => HttpResponse::Ok().json(conversation),
            Err(e) => error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to get conversation: {}", e),
            ),
        },
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "Chat history not enabled",
        ),
    }
}

/// Query chat messages
async fn chat_history_messages(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<ChatHistoryMessagesQuery>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    use crate::chat_history::MessageFilters;

    match &state.chat_history {
        Some(mgr) => match query.conversation_id.as_ref() {
            Some(conv_id) => {
                let filters = MessageFilters {
                    conversation_id: Some(conv_id.clone()),
                    limit: query.limit,
                    ..Default::default()
                };

                match mgr.list_messages(&filters).await {
                    Ok(messages) => {
                        let message_count = messages.len();
                        HttpResponse::Ok().json(serde_json::json!({
                            "messages": messages,
                            "count": message_count,
                            "conversation_id": conv_id
                        }))
                    }
                    Err(e) => error_response(
                        http::StatusCode::INTERNAL_SERVER_ERROR,
                        &format!("Failed to query messages: {}", e),
                    ),
                }
            }
            None => error_response(
                http::StatusCode::BAD_REQUEST,
                "conversation_id parameter is required",
            ),
        },
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "Chat history not enabled",
        ),
    }
}

/// Delete a conversation
async fn chat_history_delete_conversation(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let conversation_id = path.into_inner();

    match &state.chat_history {
        Some(mgr) => match mgr.delete_conversation(&conversation_id).await {
            Ok(_) => HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "Conversation deleted",
                "conversation_id": conversation_id
            })),
            Err(e) => error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to delete conversation: {}", e),
            ),
        },
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "Chat history not enabled",
        ),
    }
}

/// Clear all chat history
async fn chat_history_clear(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    match &state.chat_history {
        Some(mgr) => match mgr.clear().await {
            Ok(_) => HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "Chat history cleared"
            })),
            Err(e) => error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to clear chat history: {}", e),
            ),
        },
        None => error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "Chat history not enabled",
        ),
    }
}

// ============================================================================
// Rate Limit Admin Endpoints
// ============================================================================

/// Helper: get rl manager or return 503
macro_rules! require_rl {
    ($state:expr) => {
        match &$state.rate_limit_manager {
            Some(m) => m.clone(),
            None => {
                return error_response(
                    http::StatusCode::SERVICE_UNAVAILABLE,
                    "Rate limiting not enabled",
                )
            }
        }
    };
}

/// List all rate limit policies.
async fn rl_list_policies(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    match mgr.list_policies().await {
        Ok(policies) => {
            let count = policies.len();
            HttpResponse::Ok().json(serde_json::json!({
                "policies": policies,
                "count": count
            }))
        }
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to list policies: {}", e),
        ),
    }
}

/// Create a new rate limit policy.
async fn rl_create_policy(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<crate::rate_limit::RateLimitPolicy>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let policy = body.into_inner();
    let policy_id = policy.id.clone();
    match mgr.create_policy(policy).await {
        Ok(()) => HttpResponse::Created().json(serde_json::json!({
            "success": true,
            "policy_id": policy_id
        })),
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to create policy: {}", e),
        ),
    }
}

/// Get a specific rate limit policy.
async fn rl_get_policy(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let id = path.into_inner();
    match mgr.get_policy(&id).await {
        Ok(Some(policy)) => HttpResponse::Ok().json(policy),
        Ok(None) => error_response(
            http::StatusCode::NOT_FOUND,
            &format!("Policy '{}' not found", id),
        ),
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to get policy: {}", e),
        ),
    }
}

/// Update an existing rate limit policy.
async fn rl_update_policy(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<crate::rate_limit::RateLimitPolicy>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let id = path.into_inner();
    let mut policy = body.into_inner();
    policy.id = id.clone(); // ensure consistent id
    match mgr.update_policy(policy).await {
        Ok(true) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "policy_id": id
        })),
        Ok(false) => error_response(
            http::StatusCode::NOT_FOUND,
            &format!("Policy '{}' not found", id),
        ),
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to update policy: {}", e),
        ),
    }
}

/// Delete a rate limit policy.
async fn rl_delete_policy(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let id = path.into_inner();
    match mgr.delete_policy(&id).await {
        Ok(true) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "message": format!("Policy '{}' deleted", id)
        })),
        Ok(false) => error_response(
            http::StatusCode::NOT_FOUND,
            &format!("Policy '{}' not found", id),
        ),
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to delete policy: {}", e),
        ),
    }
}

/// Get rate limit status for a specific key.
async fn rl_key_status(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let key_id = path.into_inner();

    // Resolve policy
    let policy = match mgr.resolve_policy(&key_id).await {
        Ok(p) => p,
        Err(e) => {
            return error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to resolve policy: {}", e),
            )
        }
    };

    // Check block status
    let block = mgr.get_block(&key_id).await;

    // Concurrency status
    let concurrency_status = mgr.concurrency.get_status(&key_id);

    // Policy assignment
    let assigned_policy_id = mgr.get_key_policy_id(&key_id).await.ok().flatten();

    // Current usage per bucket
    let (policy_id_label, usage) = match mgr.get_current_usage(&key_id).await {
        Ok(u) => u,
        Err(_) => ("unlimited".to_string(), vec![]),
    };

    HttpResponse::Ok().json(serde_json::json!({
        "key_id": key_id,
        "blocked": block.is_some(),
        "block": block,
        "policy": policy,
        "policy_id": policy_id_label,
        "assigned_policy_id": assigned_policy_id,
        "bucket_usage": usage,
        "concurrency": concurrency_status.map(|(active, queued, max_concurrent, max_queue_size)| serde_json::json!({
            "active": active,
            "max_concurrent": max_concurrent,
            "queued": queued,
            "max_queue_size": max_queue_size
        }))
    }))
}

/// Assign a rate limit policy to a key.
#[derive(Debug, Deserialize)]
struct AssignPolicyBody {
    policy_id: String,
}

async fn rl_assign_key_policy(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<AssignPolicyBody>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let key_id = path.into_inner();
    match mgr.assign_key_policy(&key_id, &body.policy_id).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "key_id": key_id,
            "policy_id": body.policy_id
        })),
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to assign policy: {}", e),
        ),
    }
}

/// Remove a key's policy assignment (falls back to default).
async fn rl_remove_key_policy(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let key_id = path.into_inner();
    match mgr.remove_key_policy(&key_id).await {
        Ok(true) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "key_id": key_id,
            "message": "Policy assignment removed"
        })),
        Ok(false) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "key_id": key_id,
            "message": "No policy was assigned"
        })),
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to remove policy assignment: {}", e),
        ),
    }
}

/// Get the default policy.
async fn rl_get_default_policy(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let id = match mgr.get_default_policy_id().await {
        Ok(id) => id,
        Err(e) => {
            return error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to get default policy: {}", e),
            )
        }
    };
    let policy = match id.as_deref() {
        Some(pid) => match mgr.get_policy(pid).await {
            Ok(p) => p,
            Err(e) => {
                return error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("Failed to resolve default policy: {}", e),
                )
            }
        },
        None => None,
    };
    HttpResponse::Ok().json(serde_json::json!({
        "default_policy_id": id,
        "policy": policy
    }))
}

/// Set the default rate limit policy.
#[derive(Debug, Deserialize)]
struct SetDefaultPolicyBody {
    policy_id: String,
}

async fn rl_set_default_policy(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<SetDefaultPolicyBody>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    match mgr.set_default_policy(&body.policy_id).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "default_policy_id": body.policy_id
        })),
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to set default policy: {}", e),
        ),
    }
}

/// Emergency block a key.
#[derive(Debug, Deserialize)]
struct EmergencyBlockBody {
    key_id: String,
    duration_secs: Option<u64>,
    reason: Option<String>,
}

async fn rl_emergency_block(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<EmergencyBlockBody>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let duration_secs = body.duration_secs.unwrap_or(3600); // 1 hour default
    let reason = body
        .reason
        .clone()
        .unwrap_or_else(|| "Emergency block by admin".to_string());
    match mgr
        .set_emergency_block(&body.key_id, Some(duration_secs), &reason)
        .await
    {
        Ok(()) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "key_id": body.key_id,
                "until_secs": now + duration_secs,
                "reason": reason
            }))
        }
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to set emergency block: {}", e),
        ),
    }
}

/// Remove an emergency block.
async fn rl_remove_emergency_block(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let key_id = path.into_inner();
    match mgr.remove_emergency_block(&key_id).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "key_id": key_id,
            "message": "Emergency block removed"
        })),
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to remove emergency block: {}", e),
        ),
    }
}

/// List all emergency blocks.
async fn rl_list_emergency_blocks(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    match mgr.list_emergency_blocks().await {
        Ok(blocks) => {
            let count = blocks.len();
            HttpResponse::Ok().json(serde_json::json!({
                "blocks": blocks,
                "count": count
            }))
        }
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to list emergency blocks: {}", e),
        ),
    }
}

/// Get concurrency status for a key.
async fn rl_concurrency_status(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let key_id = path.into_inner();
    let status = mgr.concurrency.get_status(&key_id);
    HttpResponse::Ok().json(serde_json::json!({
        "key_id": key_id,
        "concurrency": status.map(|(active, queued, max_concurrent, max_queue_size)| serde_json::json!({
            "active": active,
            "max_concurrent": max_concurrent,
            "queued": queued,
            "max_queue_size": max_queue_size
        }))
    }))
}

/// Reload rate limit file config.
async fn rl_reload_config(state: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    match mgr.reload_file_config().await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "message": "Rate limit config reloaded"
        })),
        Err(e) => error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to reload config: {}", e),
        ),
    }
}

/// Query rate limit analytics and per-key metrics.
#[derive(Debug, Deserialize)]
struct RlAnalyticsQuery {
    key_id: Option<String>,
    start: Option<u64>,
    end: Option<u64>,
    limit: Option<usize>,
}

async fn rl_analytics(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<RlAnalyticsQuery>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req) {
        return resp;
    }
    let mgr = require_rl!(state);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let start = query.start.unwrap_or(now.saturating_sub(3600));
    let end = query.end.unwrap_or(now);
    let limit = query.limit.unwrap_or(200);

    let events = mgr.metrics.get_events(query.key_id.as_deref(), limit, 0);

    let metrics = if let Some(ref kid) = query.key_id {
        mgr.metrics
            .get_metrics(kid)
            .map(|m| serde_json::to_value(&m).unwrap_or_default())
            .unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::to_value(mgr.metrics.get_all_metrics()).unwrap_or_default()
    };

    let count = events.len();
    HttpResponse::Ok().json(serde_json::json!({
        "events": events,
        "count": count,
        "metrics": metrics,
        "start": start,
        "end": end
    }))
}
