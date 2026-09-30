//! Optional Jev model-routing client.
//!
//! Jev makes a typed choice from a caller-provided set of downstream models.
//! Routiium keeps the execution boundary local: Jev can select a catalog entry,
//! but the normal `RouterClient` still resolves and authorizes the selected route.

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://www.jevai.org";
const DEFAULT_TIMEOUT_MS: u64 = 750;
const DEFAULT_CATALOG_TTL_MS: u64 = 60_000;
const MAX_REQUEST_BYTES: usize = 32 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_TASK_BYTES: usize = 12 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 256;
const DEFAULT_ROUTE_ALIASES: &[&str] = &["auto", "routiium-auto", "openai-multimodal", "default"];

/// Candidate presented to Jev for model selection.
#[derive(Debug, Clone, Serialize)]
pub struct JevCandidate {
    /// Stable identifier that must be present in the Router catalog.
    pub id: String,

    /// Human-readable capabilities and policy description.
    pub description: String,

    /// Coarse cost hint, such as `low`, `medium`, or `high`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<String>,

    /// Coarse latency hint, such as `low`, `medium`, or `high`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency: Option<String>,
}

/// Request payload for Jev's model-routing preset.
#[derive(Debug, Clone, Serialize)]
pub struct JevModelRouteRequest {
    /// Bounded task description used for the routing decision.
    pub task: String,

    /// Approved downstream candidates.
    pub candidates: Vec<JevCandidate>,

    /// Ordered priorities for the decision.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub priorities: Vec<String>,

    /// Hard constraints the selected model must satisfy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,

    /// Consequence level for the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stakes: Option<String>,
}

/// Parsed Jev model-routing decision.
#[derive(Debug, Clone, PartialEq)]
pub struct JevRouteDecision {
    /// Selected candidate identifier as returned by Jev.
    pub selected_id: String,

    /// Optional calibrated confidence from Jev.
    pub confidence: Option<f64>,

    /// Optional per-candidate probabilities from Jev.
    pub probabilities: HashMap<String, f64>,
}

/// Jev client configuration.
#[derive(Clone)]
pub struct JevConfig {
    /// Jev API base URL.
    pub base_url: String,

    /// Bearer token used for the Jev API.
    pub api_key: String,

    /// Request timeout in milliseconds.
    pub timeout_ms: u64,

    /// Model aliases that should be handed to Jev.
    pub route_aliases: Vec<String>,

    /// Whether Jev failures should be returned instead of using the normal router.
    pub fail_closed: bool,

    /// How long the Router catalog may be reused by Jev.
    pub catalog_ttl_ms: u64,
}

impl JevConfig {
    /// Load optional Jev routing configuration from the environment.
    ///
    /// Jev is enabled when a `ROUTIIUM_JEV_API_KEY`/`JEV_API_KEY` is present.
    /// Routiium owns the default aliases; no enablement or alias environment
    /// variables are required.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("ROUTIIUM_JEV_API_KEY_ENV")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .and_then(|name| first_env_value(&[name.as_str()]))
            .or_else(|| first_env_value(&["ROUTIIUM_JEV_API_KEY", "JEV_API_KEY"]))?;
        let base_url = first_env_value(&["ROUTIIUM_JEV_BASE_URL"])
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        if base_url.is_empty() {
            return None;
        }

        Some(Self {
            base_url,
            api_key,
            timeout_ms: env_u64("ROUTIIUM_JEV_TIMEOUT_MS", DEFAULT_TIMEOUT_MS).max(1),
            route_aliases: DEFAULT_ROUTE_ALIASES
                .iter()
                .map(|alias| (*alias).to_string())
                .collect(),
            fail_closed: env_truthy("ROUTIIUM_JEV_STRICT"),
            catalog_ttl_ms: env_u64("ROUTIIUM_JEV_CATALOG_TTL_MS", DEFAULT_CATALOG_TTL_MS),
        })
    }
}

/// HTTP client for Jev's model-routing decision endpoint.
#[derive(Clone)]
pub struct JevClient {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
}

impl JevClient {
    /// Construct a Jev client from explicit configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying HTTP client cannot be built.
    pub fn new(config: &JevConfig) -> Result<Self> {
        if config.api_key.trim().is_empty() {
            return Err(anyhow!("Jev API key must not be empty"));
        }
        validate_base_url(&config.base_url)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms.max(1)))
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(4)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            endpoint: format!(
                "{}/api/v1/decisions/model-route",
                config.base_url.trim_end_matches('/')
            ),
            api_key: config.api_key.clone(),
        })
    }

    /// Construct a client when Jev has been configured in the environment.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let config = JevConfig::from_env()?;
        match Self::new(&config) {
            Ok(client) => Some(client),
            Err(error) => {
                tracing::warn!("Unable to initialize Jev routing client: {error}");
                None
            }
        }
    }

    /// Ask Jev to select one of the supplied candidates.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, non-success API responses,
    /// malformed envelopes, or a response without a selected candidate.
    pub async fn route(
        &self,
        request: &JevModelRouteRequest,
    ) -> std::result::Result<JevRouteDecision, JevError> {
        if request.candidates.len() < 2 {
            return Err(JevError::InvalidRequest(
                "Jev model routing requires at least two candidates".to_string(),
            ));
        }

        let body = crate::decision::serialize_bounded_json(request, MAX_REQUEST_BYTES)
            .map_err(|error| JevError::InvalidRequest(error.to_string()))?;

        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    JevError::Timeout(error.to_string())
                } else {
                    JevError::Network(error.to_string())
                }
            })?;

        let status = response.status();
        let mut response_stream = response.bytes_stream();
        let mut response_body = Vec::new();
        while let Some(chunk) = response_stream.next().await {
            let chunk = chunk.map_err(|error| JevError::Network(error.to_string()))?;
            if response_body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(JevError::InvalidResponse(format!(
                    "Jev response exceeds {MAX_RESPONSE_BYTES} bytes"
                )));
            }
            response_body.extend_from_slice(&chunk);
        }
        let response_text = String::from_utf8(response_body).map_err(|error| {
            JevError::InvalidResponse(format!("Jev returned non-UTF-8 JSON: {error}"))
        })?;
        let payload = serde_json::from_str::<Value>(&response_text).map_err(|error| {
            JevError::InvalidResponse(format!("Jev returned invalid JSON: {error}"))
        })?;

        if !status.is_success() {
            return Err(JevError::Http {
                status: status.as_u16(),
                message: response_message(&payload, &response_text),
            });
        }

        let code = parse_code(payload.get("code")).ok_or_else(|| {
            JevError::InvalidResponse("Jev response omitted a numeric code".to_string())
        })?;
        if code != 0 {
            return Err(JevError::Api {
                code,
                message: response_message(&payload, &response_text),
            });
        }

        let data = payload
            .get("data")
            .ok_or_else(|| JevError::InvalidResponse("Jev response omitted data".to_string()))?;
        let selected_id = extract_selection(data).ok_or_else(|| {
            JevError::InvalidResponse("Jev response omitted a selected candidate".to_string())
        })?;
        if selected_id.len() > MAX_IDENTIFIER_BYTES {
            return Err(JevError::InvalidResponse(format!(
                "Jev selected candidate exceeds {MAX_IDENTIFIER_BYTES} bytes"
            )));
        }
        let confidence = parse_probability(data.get("confidence"), "confidence")?;
        let probabilities = parse_probabilities(data.get("probabilities"))?;

        Ok(JevRouteDecision {
            selected_id,
            confidence,
            probabilities,
        })
    }
}

/// Failure returned by the Jev HTTP/API boundary.
#[derive(Debug, thiserror::Error)]
pub enum JevError {
    #[error("Jev request timed out: {0}")]
    Timeout(String),

    #[error("Jev network error: {0}")]
    Network(String),

    #[error("Jev HTTP error ({status}): {message}")]
    Http { status: u16, message: String },

    #[error("Jev API error ({code}): {message}")]
    Api { code: i64, message: String },

    #[error("invalid Jev request: {0}")]
    InvalidRequest(String),

    #[error("invalid Jev response: {0}")]
    InvalidResponse(String),
}

fn extract_selection(data: &Value) -> Option<String> {
    if let Some(value) = data
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(value.to_string());
    }

    let object = data.as_object()?;
    for key in [
        "decision",
        "selected",
        "selected_id",
        "selected_candidate",
        "model",
        "candidate",
        "route",
    ] {
        if let Some(value) = object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
    }

    for key in ["answer", "answers"] {
        let Some(value) = object.get(key) else {
            continue;
        };
        if let Some(selection) = extract_answer_selection(value) {
            return Some(selection);
        }
    }
    None
}

fn extract_answer_selection(value: &Value) -> Option<String> {
    if let Some(selection) = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(selection.to_string());
    }
    if let Some(object) = value.as_object() {
        for key in ["choice", "decision", "selected", "selected_id", "value"] {
            if let Some(selection) = object
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(selection.to_string());
            }
        }
        for nested in object.values() {
            if let Some(selection) = extract_answer_selection(nested) {
                return Some(selection);
            }
        }
    }
    None
}

fn response_message(payload: &Value, fallback: &str) -> String {
    payload
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
        })
        .or_else(|| payload.get("error").and_then(Value::as_str))
        .map_or_else(
            || truncate_text(fallback, 512),
            |message| truncate_text(message, 512),
        )
}

fn parse_probability(
    value: Option<&Value>,
    field: &str,
) -> std::result::Result<Option<f64>, JevError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let probability = value.as_f64().ok_or_else(|| {
        JevError::InvalidResponse(format!("Jev response field {field} is not numeric"))
    })?;
    if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
        return Err(JevError::InvalidResponse(format!(
            "Jev response field {field} is outside the range 0..=1"
        )));
    }
    Ok(Some(probability))
}

fn parse_probabilities(
    value: Option<&Value>,
) -> std::result::Result<HashMap<String, f64>, JevError> {
    let Some(value) = value else {
        return Ok(HashMap::new());
    };
    let values = value.as_object().ok_or_else(|| {
        JevError::InvalidResponse("Jev response probabilities must be an object".to_string())
    })?;
    values
        .iter()
        .map(|(key, value)| {
            let probability =
                parse_probability(Some(value), "probabilities value")?.ok_or_else(|| {
                    JevError::InvalidResponse("Jev probability was omitted".to_string())
                })?;
            if key.len() > MAX_IDENTIFIER_BYTES {
                return Err(JevError::InvalidResponse(format!(
                    "Jev probability candidate exceeds {MAX_IDENTIFIER_BYTES} bytes"
                )));
            }
            Ok((key.clone(), probability))
        })
        .collect()
}

fn validate_base_url(base_url: &str) -> Result<()> {
    let url =
        reqwest::Url::parse(base_url).map_err(|error| anyhow!("invalid Jev base URL: {error}"))?;
    if url.host_str().is_none() || url.query().is_some() || url.fragment().is_some() {
        return Err(anyhow!(
            "Jev base URL must include a host and cannot include a query or fragment"
        ));
    }
    if url.scheme() == "https" {
        return Ok(());
    }
    let local_http = url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]"));
    if local_http {
        Ok(())
    } else {
        Err(anyhow!(
            "Jev base URL must use HTTPS; HTTP is allowed only for localhost test endpoints"
        ))
    }
}

fn parse_code(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
    })
}

fn truncate_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end: usize = 0;
    for character in value.chars() {
        let next = end.saturating_add(character.len_utf8());
        if next > max_bytes {
            break;
        }
        end = next;
    }
    value.get(..end).unwrap_or_default().to_string()
}

/// Bound a task description before it is sent to the external decision API.
#[must_use]
pub fn bound_task(value: &str) -> String {
    truncate_text(value, MAX_TASK_BYTES)
}

fn first_env_value(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Clone)]
    struct TestState {
        seen_auth: Arc<Mutex<Option<String>>>,
        seen_body: Arc<Mutex<Option<Value>>>,
    }

    async fn jev_route_stub(
        State(state): State<TestState>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        *state
            .seen_auth
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        *state
            .seen_body
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(body);
        Json(json!({
            "code": 0,
            "message": "ok",
            "data": {
                "decision": "fast-model",
                "confidence": 0.88,
                "probabilities": {"fast-model": 0.88}
            }
        }))
    }

    #[test]
    fn extracts_documented_decision_shape() {
        let data = json!({
            "decision": "reasoning-model",
            "confidence": 0.91,
            "probabilities": {"fast-model": 0.04, "reasoning-model": 0.91}
        });
        assert_eq!(extract_selection(&data).as_deref(), Some("reasoning-model"));
    }

    #[test]
    fn extracts_nested_answers_shape() {
        let data = json!({
            "answers": {"backend": {"choice": "balanced"}}
        });
        assert_eq!(extract_selection(&data).as_deref(), Some("balanced"));
    }

    #[test]
    fn bounds_task_without_splitting_utf8() {
        let task = "x".repeat(MAX_TASK_BYTES.saturating_add(100));
        assert_eq!(bound_task(&task).len(), MAX_TASK_BYTES);

        let unicode = "é".repeat(MAX_TASK_BYTES);
        assert!(bound_task(&unicode).len() <= MAX_TASK_BYTES);
    }

    #[test]
    fn rejects_invalid_probability_values() {
        assert!(parse_probability(Some(&json!(1.1)), "confidence").is_err());
        assert!(parse_probability(Some(&json!(-0.1)), "confidence").is_err());
        assert!(parse_probabilities(Some(&json!([0.5]))).is_err());
    }

    #[test]
    fn rejects_non_local_http_base_urls() {
        let config = JevConfig {
            base_url: "http://jev.example.test".to_string(),
            api_key: "jev-test-key".to_string(),
            timeout_ms: 1_000,
            route_aliases: vec!["auto".to_string()],
            fail_closed: false,
            catalog_ttl_ms: 1_000,
        };
        assert!(JevClient::new(&config).is_err());
    }

    #[tokio::test]
    async fn posts_to_model_route_with_bearer_and_parses_envelope(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let seen_auth = Arc::new(Mutex::new(None));
        let seen_body = Arc::new(Mutex::new(None));
        let state = TestState {
            seen_auth: Arc::clone(&seen_auth),
            seen_body: Arc::clone(&seen_body),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/api/v1/decisions/model-route", post(jev_route_stub))
                    .with_state(state)
                    .into_make_service(),
            )
            .await
        });

        let config = JevConfig {
            base_url: format!("http://{address}"),
            api_key: "jev-test-key".to_string(),
            timeout_ms: 1_000,
            route_aliases: vec!["auto".to_string()],
            fail_closed: false,
            catalog_ttl_ms: 1_000,
        };
        let client = JevClient::new(&config)?;
        let decision = client
            .route(&JevModelRouteRequest {
                task: "Choose a model".to_string(),
                candidates: vec![
                    JevCandidate {
                        id: "fast-model".to_string(),
                        description: "Fast".to_string(),
                        cost: None,
                        latency: None,
                    },
                    JevCandidate {
                        id: "deep-model".to_string(),
                        description: "Deep".to_string(),
                        cost: None,
                        latency: None,
                    },
                ],
                priorities: vec!["quality".to_string()],
                constraints: Vec::new(),
                stakes: Some("low".to_string()),
            })
            .await?;

        server.abort();
        let server_result = server.await;
        if !server_result.is_err_and(|error| error.is_cancelled()) {
            return Err(std::io::Error::other("Jev stub server was not cancelled").into());
        }

        if decision.selected_id != "fast-model" {
            return Err(std::io::Error::other("Jev selected an unexpected model").into());
        }
        let authorization = seen_auth
            .lock()
            .map_err(|_| std::io::Error::other("poisoned Jev auth capture mutex"))?
            .clone();
        if authorization.as_deref() != Some("Bearer jev-test-key") {
            return Err(std::io::Error::other("Jev stub observed unexpected authorization").into());
        }
        let task = {
            let body = seen_body
                .lock()
                .map_err(|_| std::io::Error::other("poisoned Jev body capture mutex"))?;
            body.as_ref()
                .and_then(|value| value.get("task"))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        if task.as_deref() != Some("Choose a model") {
            return Err(std::io::Error::other("Jev stub observed an unexpected task").into());
        }
        Ok(())
    }
}
