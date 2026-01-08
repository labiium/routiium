use actix_web::http::header;
use actix_web::{web, HttpRequest, HttpResponse, HttpResponseBuilder, Responder};
use bytes::Bytes;
#[allow(unused_imports)]
use futures_util::TryStreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::conversion::{
    responses_chunk_to_chat_chunk, responses_to_chat_response, to_responses_request,
};
use crate::models::chat::ChatCompletionRequest;
use crate::models::responses;
use crate::router_client::{
    extract_route_request, PrivacyMode as RouterPrivacyMode, RouteError, RoutePlan,
    UpstreamMode as RouterUpstreamMode,
};
use crate::util::AppState;

use crate::util::error_response;
use tracing::warn;
/// Query parameters for conversion/proxy endpoints.
#[derive(Debug, Deserialize)]
pub struct ConvertQuery {
    /// Optional Responses conversation id to make the call stateful.
    pub conversation_id: Option<String>,
    /// Optional pointer to a previous Responses id (state chaining preview).
    pub previous_response_id: Option<String>,
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
}

fn router_privacy_mode_from_env() -> RouterPrivacyMode {
    match std::env::var("ROUTIIUM_ROUTER_PRIVACY_MODE")
        .unwrap_or_else(|_| "features".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
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

fn router_strict_mode() -> bool {
    matches!(
        std::env::var("ROUTIIUM_ROUTER_STRICT")
            .ok()
            .as_deref()
            .map(|v| v.trim().to_ascii_lowercase()),
        Some(ref v) if v == "1" || v == "true" || v == "yes" || v == "on"
    )
}

async fn resolve_upstream(
    state: &AppState,
    api: &str,
    body: &serde_json::Value,
) -> Result<UpstreamResolution, RouteError> {
    let requested_model = body.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let strict_mode = router_strict_mode();

    if let Some(router) = state.router_client.as_ref() {
        if !requested_model.is_empty() {
            let privacy_mode = router_privacy_mode_from_env();
            let route_request = extract_route_request(requested_model, api, body, privacy_mode);
            match router.plan(&route_request).await {
                Ok(plan) => {
                    let model_id = plan.upstream.model_id.clone();
                    return Ok(UpstreamResolution {
                        base_url: plan.upstream.base_url.clone(),
                        mode: map_router_mode(plan.upstream.mode),
                        key_env: plan.upstream.auth_env.clone(),
                        headers: plan.upstream.headers.clone(),
                        model_id,
                        plan: Some(plan),
                    });
                }
                Err(e) => {
                    if strict_mode {
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

    // Fallback to legacy prefix-based routing
    let mut base_url: Option<String> = None;
    let mut mode: Option<crate::util::UpstreamMode> = None;
    let mut key_env: Option<String> = None;

    if let Ok(cfg) = std::env::var("ROUTIIUM_BACKENDS") {
        if !requested_model.is_empty() {
            for rule_raw in cfg.split(';') {
                let r = rule_raw.trim();
                if r.is_empty() {
                    continue;
                }
                let mut prefix: Option<String> = None;
                let mut base: Option<String> = None;
                let mut key_env_local: Option<String> = None;
                let mut mode_local: Option<crate::util::UpstreamMode> = None;

                for kv in r.split([',', ';']) {
                    let p = kv.trim();
                    if p.is_empty() || !p.contains('=') {
                        continue;
                    }
                    let mut it = p.splitn(2, '=');
                    let k = it.next().unwrap_or("").trim().to_ascii_lowercase();
                    let v = it.next().unwrap_or("").trim().to_string();
                    if v.is_empty() {
                        continue;
                    }
                    match k.as_str() {
                        "prefix" => prefix = Some(v),
                        "base" | "base_url" => base = Some(v),
                        "key_env" | "api_key_env" => key_env_local = Some(v),
                        "mode" => {
                            let vv = v.to_ascii_lowercase();
                            mode_local = match vv.as_str() {
                                "chat" => Some(crate::util::UpstreamMode::Chat),
                                "bedrock" => Some(crate::util::UpstreamMode::Bedrock),
                                _ => Some(crate::util::UpstreamMode::Responses),
                            };
                        }
                        _ => {}
                    }
                }

                if let (Some(pfx), Some(bu)) = (prefix, base) {
                    if requested_model.starts_with(pfx.as_str()) {
                        base_url = Some(bu);
                        mode = mode_local;
                        key_env = key_env_local;
                        break;
                    }
                }
            }
        }
    }

    let resolved_model = if !requested_model.is_empty() {
        requested_model.to_string()
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
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

/// Passthrough for OpenAI Responses API (`/v1/responses`):
/// Accepts native Responses payload and forwards upstream without transformation.
/// Supports SSE when `stream: true`.
async fn responses_passthrough(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> impl Responder {
    let mut body = body.into_inner();

    // Apply system prompt injection if configured
    let system_prompt_guard = state.system_prompt_config.read().await;
    let model = body.get("model").and_then(|v| v.as_str());

    if let Some(prompt) = system_prompt_guard.get_prompt(model, Some("responses")) {
        // Inject system prompt into messages (Responses API uses "input" not "messages")
        if let Some(messages) = body.get_mut("input").and_then(|v| v.as_array_mut()) {
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

    // Determine managed (internal upstream key) vs passthrough mode
    let env_api_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let managed_mode = env_api_key.is_some();

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
                Some(crate::auth::Verification::Valid { .. }) => None,
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
            // No manager: accept and let routing pick env key
            None
        }
    } else {
        if client_bearer.is_none() {
            return error_response(
                http::StatusCode::UNAUTHORIZED,
                "Missing Authorization bearer",
            );
        }
        client_bearer.clone()
    };

    let stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let client = &state.http;
    let resolution = match resolve_upstream(&state, "responses", &body).await {
        Ok(res) => res,
        Err(err) => {
            return error_response(
                http::StatusCode::BAD_GATEWAY,
                &format!("Router error: {}", err),
            );
        }
    };

    let mut effective_body = body.clone();
    if let Some(obj) = effective_body.as_object_mut() {
        obj.insert(
            "model".to_string(),
            serde_json::json!(resolution.model_id.clone()),
        );
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
                        let mut builder = HttpResponse::Ok();
                        if let Some(plan) = resolution.plan.as_ref() {
                            insert_route_headers(&mut builder, plan, &resolution.model_id);
                        }
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
            let chat_req = crate::conversion::responses_json_to_chat_request(&stream_body);
            if let Ok(v) = serde_json::to_value(chat_req) {
                stream_body = v;
            }
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
                > = if let Some(plan) = resolution.plan.as_ref() {
                    if matches!(plan.upstream.mode, RouterUpstreamMode::Responses) {
                        Box::pin(ResponsesSseToChatSse::new(stream))
                    } else {
                        Box::pin(stream)
                    }
                } else {
                    Box::pin(stream)
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
                response.streaming(stream)
            }
            Err(e) => router_error_response(
                http::StatusCode::BAD_GATEWAY,
                &e.to_string(),
                resolution.plan.as_ref(),
                &resolution.model_id,
            ),
        }
    } else {
        let mut outbound_body = effective_body.clone();
        if matches!(resolution.mode, crate::util::UpstreamMode::Chat) {
            let chat_req = crate::conversion::responses_json_to_chat_request(&outbound_body);
            if let Ok(v) = serde_json::to_value(chat_req) {
                outbound_body = v;
            }
        }

        let real_url = format!("{}/{}", base, endpoint);
        let mut req = client
            .post(&real_url)
            .header("content-type", "application/json");
        req = apply_upstream_headers(req, &resolution.headers);
        if let Some(b) = eff_bearer {
            req = req.bearer_auth(b);
        }
        match req.json(&outbound_body).send().await {
            Ok(up) => {
                let status = up.status();
                let bytes = up.bytes().await.unwrap_or_default();
                let mut builder = HttpResponse::build(
                    actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap(),
                );
                if let Some(plan) = resolution.plan.as_ref() {
                    insert_route_headers(&mut builder, plan, &resolution.model_id);
                    if status.is_success()
                        && matches!(plan.upstream.mode, RouterUpstreamMode::Responses)
                    {
                        if let Ok(resp_obj) =
                            serde_json::from_slice::<responses::ResponsesResponse>(&bytes)
                        {
                            let chat_resp = responses_to_chat_response(&resp_obj);
                            if let Ok(body) = serde_json::to_vec(&chat_resp) {
                                builder.insert_header(("content-type", "application/json"));
                                return builder.body(body);
                            }
                        }
                    }
                }
                builder.body(bytes)
            }
            Err(e) => router_error_response(
                http::StatusCode::BAD_GATEWAY,
                &e.to_string(),
                resolution.plan.as_ref(),
                &resolution.model_id,
            ),
        }
    }
}

/// Configure Actix-web routes with AppState.
pub fn config_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("")
            .route("/status", web::get().to(status))
            .route("/convert", web::post().to(convert))
            .route(
                "/v1/chat/completions",
                web::post().to(chat_completions_passthrough),
            )
            .route("/v1/responses", web::post().to(responses_passthrough))
            .route("/keys", web::get().to(list_keys))
            .route("/keys/generate", web::post().to(generate_key))
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
            .route("/analytics/clear", web::post().to(analytics_clear)),
    );
}

/// Service status endpoint to expose feature flags and available routes.
async fn status(state: web::Data<AppState>) -> impl Responder {
    let proxy_enabled: bool = true;
    let routes = vec![
        "/status",
        "/convert",
        "/v1/chat/completions",
        "/v1/responses",
        "/keys",
        "/keys/generate",
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

    // Get analytics status
    let analytics_enabled = state.analytics.is_some();
    let analytics_stats = if let Some(mgr) = &state.analytics {
        mgr.stats().await.ok()
    } else {
        None
    };

    web::Json(serde_json::json!({
        "name": "routiium",
        "version": env!("CARGO_PKG_VERSION"),
        "proxy_enabled": proxy_enabled,
        "routes": routes,
        "features": {
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
            "analytics": {
                "enabled": analytics_enabled,
                "stats": analytics_stats
            }
        }
    }))
}

/// Convert a Chat Completions request into a Responses API request payload (JSON).
async fn convert(
    state: web::Data<AppState>,
    query: web::Query<ConvertQuery>,
    body: web::Json<ChatCompletionRequest>,
) -> impl Responder {
    let mcp_manager_guard = if let Some(mgr) = state.mcp_manager.as_ref() {
        Some(mgr.read().await)
    } else {
        None
    };

    let system_prompt_guard = state.system_prompt_config.read().await;

    let mut converted = crate::conversion::to_responses_request_with_mcp_and_prompt(
        &body,
        query.conversation_id.clone(),
        mcp_manager_guard.as_deref(),
        Some(&*system_prompt_guard),
    )
    .await;

    if let Some(prev) = query.previous_response_id.clone() {
        converted.previous_response_id = Some(prev);
    }

    web::Json(converted)
}

/// Direct passthrough for native Chat Completions requests (no translation).
async fn chat_completions_passthrough(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<ChatQuery>,
    body: web::Json<serde_json::Value>,
) -> impl Responder {
    let mut body = body.into_inner();
    let query = query.into_inner();
    let conversation_hint = query.conversation_id.filter(|s| !s.trim().is_empty());
    let previous_response_hint = query.previous_response_id.filter(|s| !s.trim().is_empty());

    // Apply system prompt injection if configured
    let system_prompt_guard = state.system_prompt_config.read().await;
    let model = body.get("model").and_then(|v| v.as_str());

    if let Some(prompt) = system_prompt_guard.get_prompt(model, Some("chat")) {
        // Deserialize to ChatCompletionRequest for injection
        if let Ok(mut req) = serde_json::from_value::<ChatCompletionRequest>(body.clone()) {
            crate::conversion::inject_system_prompt_chat(
                &mut req,
                &prompt,
                &system_prompt_guard.injection_mode,
            );
            if let Ok(modified) = serde_json::to_value(&req) {
                body = modified;
            }
        }
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

    // Determine managed (internal upstream key) vs passthrough mode
    let env_api_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let managed_mode = env_api_key.is_some();

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
                Some(crate::auth::Verification::Valid { .. }) => None,
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
            // No manager: accept and let routing pick env key
            None
        }
    } else {
        if client_bearer.is_none() {
            return error_response(
                http::StatusCode::UNAUTHORIZED,
                "Missing Authorization bearer",
            );
        }
        client_bearer.clone()
    };

    // Determine if streaming is requested
    let stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let client = &state.http;
    let resolution = match resolve_upstream(&state, "chat", &body).await {
        Ok(res) => res,
        Err(err) => {
            return error_response(
                http::StatusCode::BAD_GATEWAY,
                &format!("Router error: {}", err),
            );
        }
    };

    if let Some(obj) = body.as_object_mut() {
        obj.insert(
            "model".to_string(),
            serde_json::json!(resolution.model_id.clone()),
        );
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
                        let mut builder = HttpResponse::Ok();
                        if let Some(plan) = resolution.plan.as_ref() {
                            insert_route_headers(&mut builder, plan, &resolution.model_id);
                        }
                        return builder.json(chat_response);
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
                response.streaming(stream)
            }
            Err(e) => router_error_response(
                http::StatusCode::BAD_GATEWAY,
                &e.to_string(),
                resolution.plan.as_ref(),
                &resolution.model_id,
            ),
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
        let mut req = client
            .post(&real_url)
            .header("content-type", "application/json");
        req = apply_upstream_headers(req, &resolution.headers);
        if let Some(b) = eff_bearer {
            req = req.bearer_auth(b);
        }
        match req.json(&outbound_body).send().await {
            Ok(up) => {
                let status = up.status();
                let bytes = up.bytes().await.unwrap_or_default();
                let mut builder = HttpResponse::build(
                    actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap(),
                );
                if let Some(plan) = resolution.plan.as_ref() {
                    insert_route_headers(&mut builder, plan, &resolution.model_id);
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
                builder.body(bytes)
            }
            Err(e) => router_error_response(
                http::StatusCode::BAD_GATEWAY,
                &e.to_string(),
                resolution.plan.as_ref(),
                &resolution.model_id,
            ),
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

async fn generate_key(
    state: web::Data<AppState>,
    body: web::Json<GenerateKeyRequest>,
) -> impl Responder {
    let payload = body.into_inner();

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

    // Compute effective ttl_seconds from either expires_at or ttl_seconds or default
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Determine ttl based on precedence: expires_at > ttl_seconds > env default
    let ttl_seconds = if let Some(exp) = payload.expires_at {
        if exp <= now {
            return error_response(
                http::StatusCode::BAD_REQUEST,
                "expires_at must be in the future",
            );
        }
        Some(exp.saturating_sub(now))
    } else if let Some(ttl) = payload.ttl_seconds {
        Some(ttl)
    } else {
        default_ttl_secs
    };

    // If required, enforce at least some ttl
    if require_exp && ttl_seconds.is_none() {
        return error_response(
            http::StatusCode::BAD_REQUEST,
            "Expiration required: provide expires_at or ttl_seconds (or configure default TTL)",
        );
    }

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

async fn list_keys(state: web::Data<AppState>) -> impl Responder {
    match &state.api_keys {
        Some(mgr) => match mgr.list_keys() {
            Ok(items) => HttpResponse::Ok().json(items),
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
    body: web::Json<RevokeKeyRequest>,
) -> impl Responder {
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
    body: web::Json<SetExpirationRequest>,
) -> impl Responder {
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
async fn reload_mcp(state: web::Data<AppState>) -> impl Responder {
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
async fn reload_system_prompt(state: web::Data<AppState>) -> impl Responder {
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
async fn reload_routing(state: web::Data<AppState>) -> impl Responder {
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
async fn reload_all(state: web::Data<AppState>) -> impl Responder {
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
async fn analytics_stats(state: web::Data<AppState>) -> impl Responder {
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
    query: web::Query<AnalyticsEventsQuery>,
) -> impl Responder {
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
    query: web::Query<AnalyticsAggregateQuery>,
) -> impl Responder {
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
    query: web::Query<AnalyticsExportQuery>,
) -> impl Responder {
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
                        let mut csv_output = String::from(
                            "id,timestamp,endpoint,method,model,stream,status_code,success,duration_ms,tokens_per_second,prompt_tokens,completion_tokens,cached_tokens,reasoning_tokens,total_cost,backend,upstream_mode\n",
                        );

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

                            csv_output.push_str(&format!(
                                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                                event.id,
                                event.timestamp,
                                event.request.endpoint,
                                event.request.method,
                                model,
                                event.request.stream,
                                status,
                                success,
                                event.performance.duration_ms,
                                tps,
                                prompt_tokens,
                                completion_tokens,
                                cached_tokens,
                                reasoning_tokens,
                                cost,
                                event.routing.backend,
                                event.routing.upstream_mode
                            ));
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
async fn analytics_clear(state: web::Data<AppState>) -> impl Responder {
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
