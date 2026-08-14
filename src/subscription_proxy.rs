//! Native pass-through endpoints for first-party coding clients.
//!
//! The client remains responsible for signing in and refreshing its subscription
//! credentials. Routiium forwards those credentials for the lifetime of one
//! request, but deliberately never persists them.

use actix_web::http::header;
use actix_web::{web, HttpRequest, HttpResponse};
use bytes::{Bytes, BytesMut};
use futures_util::{Stream, StreamExt};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::analytics::TokenUsage;
use crate::util::AppState;

static LOG_WRITE_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum SubscriptionProvider {
    ClaudeCode,
    Codex,
}

impl SubscriptionProvider {
    const fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
        }
    }

    const fn upstream_env(self) -> &'static str {
        match self {
            Self::ClaudeCode => "ROUTIIUM_CLAUDE_SUBSCRIPTION_BASE_URL",
            Self::Codex => "ROUTIIUM_CODEX_SUBSCRIPTION_BASE_URL",
        }
    }

    const fn default_upstream(self) -> Option<&'static str> {
        match self {
            Self::ClaudeCode => Some("https://api.anthropic.com"),
            // ChatGPT/Codex subscription endpoints are not a public Platform API
            // contract, so deployments must choose their authorized upstream.
            Self::Codex => None,
        }
    }

    const fn suffix(self) -> &'static str {
        match self {
            Self::ClaudeCode => "/v1/messages",
            Self::Codex => "/responses",
        }
    }
}

#[derive(Debug, Serialize)]
struct CapturedBody {
    format: &'static str,
    content: serde_json::Value,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
    original_bytes: usize,
}

#[derive(Debug, Serialize)]
struct SubscriptionUsageRecord {
    schema_version: u8,
    id: String,
    timestamp_ms: u128,
    provider: SubscriptionProvider,
    user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id_hash: Option<String>,
    method: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    stream: bool,
    request_headers: BTreeMap<String, String>,
    request: CapturedBody,
    status_code: u16,
    response_headers: BTreeMap<String, String>,
    response: CapturedBody,
    response_bytes: usize,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport_error: Option<String>,
}

struct LogContext {
    provider: SubscriptionProvider,
    user_id: String,
    account_id_hash: Option<String>,
    method: String,
    path: String,
    query: Option<String>,
    model: Option<String>,
    stream: bool,
    request_headers: BTreeMap<String, String>,
    request_body: Bytes,
    status_code: u16,
    response_headers: BTreeMap<String, String>,
    started_at: Instant,
}

impl LogContext {
    fn finish(
        self,
        response_body: &[u8],
        response_original_bytes: usize,
        response_truncated: bool,
        transport_error: Option<String>,
    ) {
        let record = SubscriptionUsageRecord {
            schema_version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            provider: self.provider,
            user_id: self.user_id,
            account_id_hash: self.account_id_hash,
            method: self.method,
            path: self.path,
            query: self.query,
            model: self.model,
            stream: self.stream,
            request_headers: self.request_headers,
            request: capture_body(&self.request_body),
            status_code: self.status_code,
            response_headers: self.response_headers,
            response: capture_body_with_metadata(
                response_body,
                response_original_bytes,
                response_truncated,
            ),
            response_bytes: response_original_bytes,
            duration_ms: u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            token_usage: extract_usage(response_body, self.stream),
            transport_error,
        };
        if let Err(error) = append_usage_record(&record) {
            tracing::error!(error = %error, "failed to persist subscription usage record");
        }
    }
}

struct LoggedResponseStream {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    context: Option<LogContext>,
    captured: Vec<u8>,
    response_bytes: usize,
    capture_limit: Option<usize>,
    truncated: bool,
}

impl LoggedResponseStream {
    fn finish(&mut self, error: Option<String>) {
        if let Some(context) = self.context.take() {
            if self.truncated {
                tracing::warn!(
                    provider = context.provider.label(),
                    "subscription response log was truncated by ROUTIIUM_SUBSCRIPTION_LOG_MAX_BODY_BYTES"
                );
            }
            context.finish(&self.captured, self.response_bytes, self.truncated, error);
        }
    }
}

impl Stream for LoggedResponseStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                self.response_bytes = self.response_bytes.saturating_add(bytes.len());
                let remaining = self.capture_limit.map_or(bytes.len(), |limit| {
                    limit.saturating_sub(self.captured.len())
                });
                let take = remaining.min(bytes.len());
                self.captured
                    .extend_from_slice(bytes.get(..take).unwrap_or_default());
                self.truncated |= take < bytes.len();
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(error))) => {
                let message = error.to_string();
                self.finish(Some(message.clone()));
                Poll::Ready(Some(Err(std::io::Error::other(message))))
            }
            Poll::Ready(None) => {
                self.finish(None);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for LoggedResponseStream {
    fn drop(&mut self) {
        self.finish(Some(
            "downstream disconnected before the upstream stream completed".into(),
        ));
    }
}

#[expect(
    clippy::future_not_send,
    reason = "Actix handlers use the framework's request payload and response types"
)]
pub async fn claude_messages(
    state: web::Data<AppState>,
    request: HttpRequest,
    payload: web::Payload,
) -> HttpResponse {
    let body = match read_request_body(payload).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    proxy_subscription(state, request, body, SubscriptionProvider::ClaudeCode).await
}

#[expect(
    clippy::future_not_send,
    reason = "Actix handlers use the framework's request payload and response types"
)]
pub async fn codex_responses(
    state: web::Data<AppState>,
    request: HttpRequest,
    payload: web::Payload,
) -> HttpResponse {
    let body = match read_request_body(payload).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    proxy_subscription(state, request, body, SubscriptionProvider::Codex).await
}

#[expect(
    clippy::future_not_send,
    reason = "Actix payload streams are framework-owned and not Send"
)]
async fn read_request_body(mut payload: web::Payload) -> Result<Bytes, HttpResponse> {
    let limit = std::env::var("ROUTIIUM_SUBSCRIPTION_MAX_REQUEST_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(64 * 1024 * 1024);
    let mut body = BytesMut::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk.map_err(|error| {
            HttpResponse::BadRequest().json(serde_json::json!({
                "error": { "message": format!("failed to read request body: {error}") }
            }))
        })?;
        if limit > 0 && body.len().saturating_add(chunk.len()) > limit {
            return Err(HttpResponse::PayloadTooLarge().json(serde_json::json!({
                "error": {
                    "message": format!(
                        "subscription request exceeds ROUTIIUM_SUBSCRIPTION_MAX_REQUEST_BYTES ({limit})"
                    )
                }
            })));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

#[expect(
    clippy::future_not_send,
    clippy::too_many_lines,
    reason = "Actix proxy handler preserves streaming request/response lifecycle"
)]
async fn proxy_subscription(
    state: web::Data<AppState>,
    request: HttpRequest,
    body: Bytes,
    provider: SubscriptionProvider,
) -> HttpResponse {
    let started_at = Instant::now();
    let credential = request
        .headers()
        .get(header::AUTHORIZATION)
        .or_else(|| request.headers().get("x-api-key"))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let Some(credential) = credential else {
        return HttpResponse::Unauthorized().json(serde_json::json!({
            "error": {
                "message": "The coding client did not provide subscription credentials"
            }
        }));
    };

    let target = match subscription_target(provider, request.query_string()) {
        Ok(target) => target,
        Err(message) => {
            return HttpResponse::ServiceUnavailable().json(serde_json::json!({
                "error": { "message": message }
            }));
        }
    };

    let request_json = serde_json::from_slice::<serde_json::Value>(&body).ok();
    let model = request_json
        .as_ref()
        .and_then(|value| value.get("model"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let stream = request_json
        .as_ref()
        .and_then(|value| value.get("stream"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let user_id = request
        .headers()
        .get("x-routiium-user-id")
        .and_then(|value| value.to_str().ok())
        .map(sanitize_user_id)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| credential_fingerprint(provider, credential));
    let account_id_hash = request
        .headers()
        .get("chatgpt-account-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| secret_fingerprint("account", value));

    let mut upstream = state.http.post(target).body(body.clone());
    for (name, value) in request.headers() {
        if should_forward_request_header(name.as_str()) {
            upstream = upstream.header(name.as_str(), value.as_bytes());
        }
    }

    let upstream = match upstream.send().await {
        Ok(response) => response,
        Err(error) => {
            let context = LogContext {
                provider,
                user_id,
                account_id_hash,
                method: request.method().to_string(),
                path: request.path().to_string(),
                query: nonempty(request.query_string()),
                model,
                stream,
                request_headers: safe_request_headers(&request),
                request_body: body,
                status_code: 502,
                response_headers: BTreeMap::new(),
                started_at,
            };
            context.finish(&[], 0, false, Some(error.to_string()));
            return HttpResponse::BadGateway().json(serde_json::json!({
                "error": { "message": format!("subscription upstream request failed: {error}") }
            }));
        }
    };

    let status = upstream.status();
    let is_event_stream = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"));
    let response_headers = safe_response_headers(upstream.headers());
    let mut builder = HttpResponse::build(
        actix_web::http::StatusCode::from_u16(status.as_u16())
            .unwrap_or(actix_web::http::StatusCode::BAD_GATEWAY),
    );
    copy_response_headers(&mut builder, upstream.headers());

    let context = LogContext {
        provider,
        user_id,
        account_id_hash,
        method: request.method().to_string(),
        path: request.path().to_string(),
        query: nonempty(request.query_string()),
        model,
        stream: stream || is_event_stream,
        request_headers: safe_request_headers(&request),
        request_body: body,
        status_code: status.as_u16(),
        response_headers,
        started_at,
    };

    if is_event_stream {
        let logged_stream = LoggedResponseStream {
            inner: Box::pin(upstream.bytes_stream()),
            context: Some(context),
            captured: Vec::new(),
            response_bytes: 0,
            capture_limit: capture_limit(),
            truncated: false,
        };
        builder.streaming(logged_stream)
    } else {
        match upstream.bytes().await {
            Ok(bytes) => {
                context.finish(&bytes, bytes.len(), false, None);
                builder.body(bytes)
            }
            Err(error) => {
                context.finish(&[], 0, false, Some(error.to_string()));
                HttpResponse::BadGateway().json(serde_json::json!({
                    "error": { "message": format!("subscription upstream response failed: {error}") }
                }))
            }
        }
    }
}

fn subscription_target(provider: SubscriptionProvider, query: &str) -> Result<String, String> {
    let base = std::env::var(provider.upstream_env())
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| provider.default_upstream().map(str::to_owned))
        .ok_or_else(|| {
            format!(
                "{} must be configured with an upstream authorized for this deployment",
                provider.upstream_env()
            )
        })?;
    let mut target = format!("{}{}", base.trim_end_matches('/'), provider.suffix());
    if !query.is_empty() {
        target.push('?');
        target.push_str(query);
    }
    let url = reqwest::Url::parse(&target)
        .map_err(|error| format!("invalid {}: {error}", provider.upstream_env()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "{} must use http or https",
            provider.upstream_env()
        ));
    }
    Ok(target)
}

fn should_forward_request_header(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "content-length"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "x-routiium-user-id"
    ) && !name.to_ascii_lowercase().starts_with("x-forwarded-")
}

fn copy_response_headers(
    builder: &mut actix_web::HttpResponseBuilder,
    headers: &reqwest::header::HeaderMap,
) {
    for (name, value) in headers {
        if !matches!(
            name.as_str().to_ascii_lowercase().as_str(),
            "content-length"
                | "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
                | "set-cookie"
        ) {
            if let Ok(value) = value.to_str() {
                builder.append_header((name.as_str(), value));
            }
        }
    }
}

fn safe_request_headers(request: &HttpRequest) -> BTreeMap<String, String> {
    const SAFE: &[&str] = &[
        "accept",
        "content-type",
        "anthropic-version",
        "anthropic-beta",
        "originator",
        "user-agent",
        "x-stainless-lang",
        "x-stainless-package-version",
    ];
    request
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str().to_ascii_lowercase();
            if SAFE.contains(&name.as_str()) {
                value.to_str().ok().map(|value| (name, value.to_string()))
            } else {
                None
            }
        })
        .collect()
}

fn safe_response_headers(headers: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    const SAFE: &[&str] = &[
        "content-type",
        "request-id",
        "x-request-id",
        "openai-processing-ms",
        "anthropic-ratelimit-requests-limit",
        "anthropic-ratelimit-requests-remaining",
        "anthropic-ratelimit-tokens-limit",
        "anthropic-ratelimit-tokens-remaining",
    ];
    headers
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str().to_ascii_lowercase();
            if SAFE.contains(&name.as_str()) {
                value.to_str().ok().map(|value| (name, value.to_string()))
            } else {
                None
            }
        })
        .collect()
}

fn capture_body(body: &[u8]) -> CapturedBody {
    let original_bytes = body.len();
    let (body, truncated) = match capture_limit() {
        Some(limit) if body.len() > limit => (body.get(..limit).unwrap_or_default(), true),
        _ => (body, false),
    };
    capture_body_with_metadata(body, original_bytes, truncated)
}

fn capture_body_with_metadata(body: &[u8], original_bytes: usize, truncated: bool) -> CapturedBody {
    if let Ok(json) = serde_json::from_slice(body) {
        return CapturedBody {
            format: "json",
            content: json,
            truncated,
            original_bytes,
        };
    }
    if let Ok(text) = std::str::from_utf8(body) {
        return CapturedBody {
            format: "text",
            content: serde_json::Value::String(text.to_string()),
            truncated,
            original_bytes,
        };
    }
    {
        use base64::Engine;
        CapturedBody {
            format: "base64",
            content: serde_json::Value::String(
                base64::engine::general_purpose::STANDARD.encode(body),
            ),
            truncated,
            original_bytes,
        }
    }
}

fn extract_usage(body: &[u8], stream: bool) -> Option<TokenUsage> {
    if !stream {
        return serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .as_ref()
            .and_then(usage_from_value);
    }

    let mut aggregate: Option<TokenUsage> = None;
    for line in body.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(data) = line.strip_prefix(b"data:") else {
            continue;
        };
        let data = trim_ascii(data);
        if data.is_empty() || data == b"[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) else {
            continue;
        };
        merge_usage(&mut aggregate, usage_from_value(&value));
    }
    aggregate
}

fn usage_from_value(value: &serde_json::Value) -> Option<TokenUsage> {
    let usage = value
        .get("usage")
        .or_else(|| value.get("message").and_then(|value| value.get("usage")))
        .or_else(|| value.get("response").and_then(|value| value.get("usage")))?;
    let input = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(serde_json::Value::as_u64);
    let output = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(serde_json::Value::as_u64);
    if input.is_none() && output.is_none() {
        return None;
    }
    let prompt_tokens = input.unwrap_or(0);
    let completion_tokens = output.unwrap_or(0);
    let cached_tokens = usage
        .get("cache_read_input_tokens")
        .or_else(|| usage.get("cached_tokens"))
        .or_else(|| {
            usage
                .get("input_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
        })
        .and_then(serde_json::Value::as_u64);
    Some(TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: usage
            .get("total_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| prompt_tokens.saturating_add(completion_tokens)),
        cached_tokens,
        reasoning_tokens: usage
            .get("reasoning_tokens")
            .or_else(|| {
                usage
                    .get("output_tokens_details")
                    .and_then(|details| details.get("reasoning_tokens"))
            })
            .and_then(serde_json::Value::as_u64),
    })
}

fn merge_usage(current: &mut Option<TokenUsage>, next: Option<TokenUsage>) {
    let Some(next) = next else {
        return;
    };
    if let Some(current) = current {
        current.prompt_tokens = current.prompt_tokens.max(next.prompt_tokens);
        current.completion_tokens = current.completion_tokens.max(next.completion_tokens);
        current.total_tokens = current
            .total_tokens
            .max(
                current
                    .prompt_tokens
                    .saturating_add(current.completion_tokens),
            )
            .max(next.total_tokens);
        current.cached_tokens = match (current.cached_tokens, next.cached_tokens) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (left, right) => left.or(right),
        };
        current.reasoning_tokens = match (current.reasoning_tokens, next.reasoning_tokens) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (left, right) => left.or(right),
        };
    } else {
        *current = Some(next);
    }
}

fn append_usage_record(record: &SubscriptionUsageRecord) -> Result<(), String> {
    let path = std::env::var("ROUTIIUM_SUBSCRIPTION_LOG_PATH")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "./data/subscription_usage.jsonl".to_string());
    if let Some(parent) = Path::new(&path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("creating {}: {error}", parent.display()))?;
    }
    let line = serde_json::to_vec(record).map_err(|error| error.to_string())?;
    let _guard = LOG_WRITE_LOCK
        .lock()
        .map_err(|_| "subscription log lock poisoned".to_string())?;
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| format!("opening {path}: {error}"))?;
    file.write_all(&line)
        .and_then(|()| file.write_all(b"\n"))
        .map_err(|error| format!("writing {path}: {error}"))
}

fn capture_limit() -> Option<usize> {
    std::env::var("ROUTIIUM_SUBSCRIPTION_LOG_MAX_BODY_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
}

fn credential_fingerprint(provider: SubscriptionProvider, credential: &str) -> String {
    secret_fingerprint(provider.label(), credential)
}

fn secret_fingerprint(namespace: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update([0]);
    hasher.update(value.as_bytes());
    let encoded = hex::encode(hasher.finalize());
    format!("anon_{}", encoded.get(..24).unwrap_or(&encoded))
}

fn sanitize_user_id(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "_-.@".contains(*character))
        .take(128)
        .collect()
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |position| position.saturating_add(1));
    bytes.get(start..end).unwrap_or_default()
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "tests use assertions and fixed fixtures where failures should abort"
)]
mod tests {
    use super::*;
    use actix_web::{test as actix_test, App};
    use axum::{routing::post, Router};
    use tokio::net::TcpListener;

    static TEST_ENV_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    #[test]
    fn credential_fingerprint_is_stable_and_provider_scoped() {
        let first = credential_fingerprint(SubscriptionProvider::Codex, "Bearer secret");
        let second = credential_fingerprint(SubscriptionProvider::Codex, "Bearer secret");
        let claude = credential_fingerprint(SubscriptionProvider::ClaudeCode, "Bearer secret");
        assert_eq!(first, second);
        assert_ne!(first, claude);
        assert!(!first.contains("secret"));
    }

    #[test]
    fn auth_and_internal_headers_are_never_forwarded_incorrectly() {
        assert!(should_forward_request_header("authorization"));
        assert!(should_forward_request_header("anthropic-beta"));
        assert!(!should_forward_request_header("proxy-authorization"));
        assert!(!should_forward_request_header("x-routiium-user-id"));
        assert!(!should_forward_request_header("x-forwarded-for"));
    }

    #[test]
    fn extracts_anthropic_non_stream_usage() {
        let body =
            br#"{"usage":{"input_tokens":12,"output_tokens":7,"cache_read_input_tokens":4}}"#;
        let usage = extract_usage(body, false).expect("usage");
        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 7);
        assert_eq!(usage.total_tokens, 19);
        assert_eq!(usage.cached_tokens, Some(4));
    }

    #[test]
    fn extracts_nested_responses_usage_details() {
        let body = br#"{"response":{"usage":{"input_tokens":12,"output_tokens":7,"input_tokens_details":{"cached_tokens":4},"output_tokens_details":{"reasoning_tokens":3}}}}"#;
        let usage = extract_usage(body, false).expect("usage");
        assert_eq!(usage.cached_tokens, Some(4));
        assert_eq!(usage.reasoning_tokens, Some(3));
    }

    #[test]
    fn combines_anthropic_stream_usage() {
        let body = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":1}}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":9}}\n\n";
        let usage = extract_usage(body, true).expect("usage");
        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 9);
        assert_eq!(usage.total_tokens, 21);
    }

    #[test]
    fn invalid_user_characters_are_removed() {
        assert_eq!(sanitize_user_id("user one/../../x"), "userone....x");
    }

    #[actix_rt::test]
    async fn claude_route_forwards_native_request_and_persists_redacted_log() {
        async fn upstream(
            headers: axum::http::HeaderMap,
            body: axum::body::Bytes,
        ) -> axum::response::Response {
            assert_eq!(
                headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer top-secret")
            );
            assert_eq!(
                headers
                    .get("anthropic-version")
                    .and_then(|value| value.to_str().ok()),
                Some("2023-06-01")
            );
            assert!(headers.get("x-routiium-user-id").is_none());
            let request: serde_json::Value = serde_json::from_slice(&body).expect("request JSON");
            assert_eq!(request["model"], "claude-test");
            axum::response::Response::builder()
                .status(200)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"id":"msg_1","content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":8,"output_tokens":3}}"#,
                ))
                .expect("upstream response")
        }

        let _env_guard = TEST_ENV_LOCK.lock().await;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind upstream stub");
        let address = listener.local_addr().expect("stub address");
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/v1/messages", post(upstream))
                    .into_make_service(),
            )
            .await
            .expect("run upstream stub");
        });

        let temp = tempfile::tempdir().expect("temporary log directory");
        let log_path = temp.path().join("usage.jsonl");
        std::env::set_var(
            "ROUTIIUM_CLAUDE_SUBSCRIPTION_BASE_URL",
            format!("http://{address}"),
        );
        std::env::set_var("ROUTIIUM_SUBSCRIPTION_LOG_PATH", &log_path);

        let state = AppState {
            http: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("HTTP client"),
            ..AppState::default()
        };
        let app = actix_test::init_service(App::new().app_data(web::Data::new(state)).route(
            "/subscriptions/claude/v1/messages",
            web::post().to(claude_messages),
        ))
        .await;
        let request_body =
            br#"{"model":"claude-test","messages":[{"role":"user","content":"hi"}]}"#;
        let request = actix_test::TestRequest::post()
            .uri("/subscriptions/claude/v1/messages?beta=true")
            .insert_header(("authorization", "Bearer top-secret"))
            .insert_header(("anthropic-version", "2023-06-01"))
            .insert_header(("x-routiium-user-id", "user-123"))
            .set_payload(request_body.as_slice())
            .to_request();
        let response = actix_test::call_service(&app, request).await;
        assert!(response.status().is_success());
        let response_body = actix_test::read_body(response).await;
        assert!(String::from_utf8_lossy(&response_body).contains("hello"));

        let log = std::fs::read_to_string(&log_path).expect("usage log");
        assert!(!log.contains("top-secret"));
        let record: serde_json::Value =
            serde_json::from_str(log.trim()).expect("usage record JSON");
        assert_eq!(record["provider"], "claude_code");
        assert_eq!(record["user_id"], "user-123");
        assert_eq!(record["request"]["content"]["model"], "claude-test");
        assert_eq!(record["response"]["content"]["id"], "msg_1");
        assert_eq!(record["token_usage"]["prompt_tokens"], 8);
        assert_eq!(record["token_usage"]["completion_tokens"], 3);

        std::env::remove_var("ROUTIIUM_CLAUDE_SUBSCRIPTION_BASE_URL");
        std::env::remove_var("ROUTIIUM_SUBSCRIPTION_LOG_PATH");
    }

    #[actix_rt::test]
    async fn codex_route_forwards_account_context_and_persists_usage() {
        async fn upstream(
            headers: axum::http::HeaderMap,
            body: axum::body::Bytes,
        ) -> axum::response::Response {
            assert_eq!(
                headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer codex-secret")
            );
            assert_eq!(
                headers
                    .get("chatgpt-account-id")
                    .and_then(|value| value.to_str().ok()),
                Some("account-123")
            );
            let request: serde_json::Value = serde_json::from_slice(&body).expect("request JSON");
            assert_eq!(request["model"], "codex-test");
            axum::response::Response::builder()
                .status(200)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"id":"resp_1","output":[],"usage":{"input_tokens":4,"output_tokens":2}}"#,
                ))
                .expect("upstream response")
        }

        let _env_guard = TEST_ENV_LOCK.lock().await;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind upstream stub");
        let address = listener.local_addr().expect("stub address");
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/responses", post(upstream))
                    .into_make_service(),
            )
            .await
            .expect("run upstream stub");
        });

        let temp = tempfile::tempdir().expect("temporary log directory");
        let log_path = temp.path().join("usage.jsonl");
        std::env::set_var(
            "ROUTIIUM_CODEX_SUBSCRIPTION_BASE_URL",
            format!("http://{address}"),
        );
        std::env::set_var("ROUTIIUM_SUBSCRIPTION_LOG_PATH", &log_path);

        let state = AppState {
            http: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("HTTP client"),
            ..AppState::default()
        };
        let app = actix_test::init_service(App::new().app_data(web::Data::new(state)).route(
            "/subscriptions/codex/responses",
            web::post().to(codex_responses),
        ))
        .await;
        let request_body = br#"{"model":"codex-test","input":"hello"}"#;
        let request = actix_test::TestRequest::post()
            .uri("/subscriptions/codex/responses")
            .insert_header(("authorization", "Bearer codex-secret"))
            .insert_header(("chatgpt-account-id", "account-123"))
            .set_payload(request_body.as_slice())
            .to_request();
        let response = actix_test::call_service(&app, request).await;
        assert!(response.status().is_success());
        let response_body = actix_test::read_body(response).await;
        assert!(String::from_utf8_lossy(&response_body).contains("resp_1"));

        let log = std::fs::read_to_string(&log_path).expect("usage log");
        assert!(!log.contains("codex-secret"));
        assert!(!log.contains("account-123"));
        let record: serde_json::Value =
            serde_json::from_str(log.trim()).expect("usage record JSON");
        assert_eq!(record["provider"], "codex");
        assert!(record["account_id_hash"]
            .as_str()
            .unwrap()
            .starts_with("anon_"));
        assert_eq!(record["token_usage"]["prompt_tokens"], 4);
        assert_eq!(record["token_usage"]["completion_tokens"], 2);

        std::env::remove_var("ROUTIIUM_CODEX_SUBSCRIPTION_BASE_URL");
        std::env::remove_var("ROUTIIUM_SUBSCRIPTION_LOG_PATH");
    }
}
