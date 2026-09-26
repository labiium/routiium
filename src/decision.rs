//! Provider-neutral client for typed semantic decisions.
//!
//! The public request and answer types are shared by routing, classification,
//! and safety consumers. Provider adapters own endpoint and error differences.

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::time::Duration;

const DEFAULT_TIMEOUT_MS: u64 = 1_000;
const MAX_REQUEST_BYTES: usize = 32 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_QUESTION_ID_BYTES: usize = 128;
const MAX_MODEL_ID_BYTES: usize = 256;
const MAX_ERROR_MESSAGE_CHARS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionProviderKind {
    OpenRouter,
    TypeSafe,
}

impl DecisionProviderKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::TypeSafe => "typesafe",
        }
    }

    const fn endpoint(self) -> &'static str {
        match self {
            Self::OpenRouter => "https://openrouter.ai/api/alpha/decisions",
            Self::TypeSafe => "https://api.typesafe.ai/v1/systemone",
        }
    }

    #[must_use]
    pub const fn default_api_key_env(self) -> &'static str {
        match self {
            Self::OpenRouter => "OPENROUTER_API_KEY",
            Self::TypeSafe => "TYPESAFE_API_KEY",
        }
    }

    /// Parse a provider name or alias.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` does not name a supported provider.
    pub fn parse(value: &str) -> Result<Self, DecisionError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openrouter" | "open_router" => Ok(Self::OpenRouter),
            "typesafe" | "type_safe" | "system_one" => Ok(Self::TypeSafe),
            _ => Err(DecisionError::InvalidRequest(format!(
                "unsupported decision provider `{}`",
                bounded_text(value, 64)
            ))),
        }
    }

    #[must_use]
    pub const fn default_model(self) -> &'static str {
        match self {
            Self::OpenRouter => "~typesafe/jev-latest",
            Self::TypeSafe => "jev-latest",
        }
    }
}

/// A boolean-like question whose answer is the probability of the proposition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoulQuestion {
    pub instructions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criteria: Option<BTreeMap<String, String>>,
}

/// A question that selects one key from a bounded set of described options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceQuestion {
    pub instructions: String,
    pub criteria: BTreeMap<String, String>,
}

/// A question that scores state against an ordered scale of two to ten levels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreQuestion {
    pub instructions: String,
    pub criteria: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionQuestion {
    Noul(NoulQuestion),
    Choice(ChoiceQuestion),
    Score(ScoreQuestion),
}

impl DecisionQuestion {
    #[must_use]
    pub fn noul(instructions: impl Into<String>) -> Self {
        Self::Noul(NoulQuestion {
            instructions: instructions.into(),
            criteria: None,
        })
    }

    #[must_use]
    pub fn choice(instructions: impl Into<String>, criteria: BTreeMap<String, String>) -> Self {
        Self::Choice(ChoiceQuestion {
            instructions: instructions.into(),
            criteria,
        })
    }

    #[must_use]
    pub fn score(instructions: impl Into<String>, criteria: Vec<String>) -> Self {
        Self::Score(ScoreQuestion {
            instructions: instructions.into(),
            criteria,
        })
    }

    fn validate(&self, id: &str) -> Result<(), DecisionError> {
        validate_identifier(id, "question", MAX_QUESTION_ID_BYTES)?;
        match self {
            Self::Noul(question) => {
                validate_instructions(&question.instructions)?;
                if let Some(criteria) = &question.criteria {
                    for key in ["true", "false"] {
                        if let Some(description) = criteria.get(key) {
                            validate_description(description)?;
                        }
                    }
                    if criteria.keys().any(|key| key != "true" && key != "false") {
                        return Err(DecisionError::InvalidRequest(format!(
                            "noul question {id} criteria may only contain `true` and `false`"
                        )));
                    }
                }
            }
            Self::Choice(question) => {
                validate_instructions(&question.instructions)?;
                if !(2..=255).contains(&question.criteria.len()) {
                    return Err(DecisionError::InvalidRequest(format!(
                        "choice question {id} must contain between 2 and 255 options"
                    )));
                }
                for (key, description) in &question.criteria {
                    validate_identifier(key, "choice option", MAX_QUESTION_ID_BYTES)?;
                    validate_description(description)?;
                }
            }
            Self::Score(question) => {
                validate_instructions(&question.instructions)?;
                if !(2..=10).contains(&question.criteria.len()) {
                    return Err(DecisionError::InvalidRequest(format!(
                        "score question {id} must contain between 2 and 10 levels"
                    )));
                }
                for description in &question.criteria {
                    validate_description(description)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionAnswer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        #[serde(default)]
        probabilities: BTreeMap<String, f64>,
        #[serde(default)]
        confidence: Option<f64>,
    },
    Score {
        score: f64,
        #[serde(default)]
        legend: BTreeMap<String, String>,
        #[serde(default)]
        probabilities: BTreeMap<String, f64>,
        #[serde(default)]
        confidence: Option<f64>,
    },
}

impl DecisionAnswer {
    fn validate_for(&self, question: &DecisionQuestion, id: &str) -> Result<(), DecisionError> {
        match (question, self) {
            (DecisionQuestion::Noul(_), Self::Noul { noul }) => {
                validate_probability(*noul, &format!("answer {id} noul"))?;
            }
            (
                DecisionQuestion::Choice(question),
                Self::Choice {
                    choice,
                    probabilities,
                    confidence,
                },
            ) => {
                if !question.criteria.contains_key(choice) {
                    return Err(DecisionError::InvalidResponse(format!(
                        "choice answer {id} selected an option outside its criteria"
                    )));
                }
                validate_probability_map(probabilities, &question.criteria, id)?;
                if let Some(confidence) = confidence {
                    validate_probability(*confidence, &format!("answer {id} confidence"))?;
                }
            }
            (
                DecisionQuestion::Score(question),
                Self::Score {
                    score,
                    legend,
                    probabilities,
                    confidence,
                },
            ) => {
                let max_index = question.criteria.len().saturating_sub(1);
                let max_index = u32::try_from(max_index).map_err(|error| {
                    DecisionError::InvalidResponse(format!(
                        "score answer {id} has an invalid criteria range: {error}"
                    ))
                })?;
                let max_score = f64::from(max_index);
                if !score.is_finite() || *score < 0.0 || *score > max_score {
                    return Err(DecisionError::InvalidResponse(format!(
                        "score answer {id} is outside its criteria range"
                    )));
                }
                for (key, description) in legend {
                    let Ok(index) = key.parse::<usize>() else {
                        return Err(DecisionError::InvalidResponse(format!(
                            "score answer {id} has an invalid legend key"
                        )));
                    };
                    if question.criteria.get(index) != Some(description) {
                        return Err(DecisionError::InvalidResponse(format!(
                            "score answer {id} legend does not match its criteria"
                        )));
                    }
                }
                for (key, value) in probabilities {
                    let Ok(index) = key.parse::<usize>() else {
                        return Err(DecisionError::InvalidResponse(format!(
                            "score answer {id} has an invalid probability key"
                        )));
                    };
                    if index >= question.criteria.len() {
                        return Err(DecisionError::InvalidResponse(format!(
                            "score answer {id} has a probability outside its criteria"
                        )));
                    }
                    validate_probability(*value, &format!("answer {id} probability"))?;
                }
                if let Some(confidence) = confidence {
                    validate_probability(*confidence, &format!("answer {id} confidence"))?;
                }
            }
            _ => {
                return Err(DecisionError::InvalidResponse(format!(
                    "answer {id} does not match the requested question type"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DecisionUsage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub model: String,
    pub state: Value,
    pub questions: BTreeMap<String, DecisionQuestion>,
}

impl DecisionRequest {
    /// Create a request and validate its model, questions, and encoded size.
    ///
    /// # Errors
    ///
    /// Returns an error if any request field is invalid or exceeds its limit.
    pub fn new(
        model: impl Into<String>,
        state: Value,
        questions: BTreeMap<String, DecisionQuestion>,
    ) -> Result<Self, DecisionError> {
        let request = Self {
            model: model.into(),
            state,
            questions,
        };
        request.validate()?;
        Ok(request)
    }

    /// Validate the request before sending it to a provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the model, questions, or serialized size is invalid.
    pub fn validate(&self) -> Result<(), DecisionError> {
        validate_identifier(&self.model, "model", MAX_MODEL_ID_BYTES)?;
        if self.questions.is_empty() {
            return Err(DecisionError::InvalidRequest(
                "decision request needs at least one question".to_string(),
            ));
        }
        for (id, question) in &self.questions {
            question.validate(id)?;
        }
        let encoded = serde_json::to_vec(self)
            .map_err(|error| DecisionError::InvalidRequest(error.to_string()))?;
        if encoded.len() > MAX_REQUEST_BYTES {
            return Err(DecisionError::InvalidRequest(format!(
                "decision request exceeds {MAX_REQUEST_BYTES} bytes"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionResponse {
    #[serde(default)]
    pub id: Option<String>,
    pub model: String,
    #[serde(default)]
    pub provider: Option<String>,
    pub usage: DecisionUsage,
    pub answers: BTreeMap<String, DecisionAnswer>,
}

impl DecisionResponse {
    /// Check that this response answers exactly the requested questions.
    ///
    /// # Errors
    ///
    /// Returns an error for missing, extra, malformed, or out-of-range answers.
    pub fn validate_for(&self, request: &DecisionRequest) -> Result<(), DecisionError> {
        validate_identifier(&self.model, "response model", MAX_MODEL_ID_BYTES)?;
        if self.answers.len() != request.questions.len() {
            return Err(DecisionError::InvalidResponse(
                "decision response answer count does not match the request".to_string(),
            ));
        }
        for (id, question) in &request.questions {
            let answer = self.answers.get(id).ok_or_else(|| {
                DecisionError::InvalidResponse(format!("decision response omitted answer {id}"))
            })?;
            answer.validate_for(question, id)?;
        }
        if let Some(cost) = self.usage.cost {
            if !cost.is_finite() || cost < 0.0 {
                return Err(DecisionError::InvalidResponse(
                    "decision response cost must be finite and non-negative".to_string(),
                ));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn choice(&self, question_id: &str) -> Option<(&str, Option<f64>)> {
        match self.answers.get(question_id)? {
            DecisionAnswer::Choice {
                choice, confidence, ..
            } => Some((choice, *confidence)),
            _ => None,
        }
    }

    #[must_use]
    pub fn noul_probability(&self, question_id: &str) -> Option<f64> {
        match self.answers.get(question_id)? {
            DecisionAnswer::Noul { noul } => Some(*noul),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum DecisionError {
    #[error("invalid decision request: {0}")]
    InvalidRequest(String),
    #[error("decision provider {provider} returned HTTP {status}: {message}")]
    Http {
        provider: &'static str,
        status: u16,
        message: String,
    },
    #[error("decision provider {provider} returned API error {code}: {message}")]
    Api {
        provider: &'static str,
        code: i64,
        message: String,
    },
    #[error("decision provider request timed out: {0}")]
    Timeout(String),
    #[error("decision provider network error: {0}")]
    Network(String),
    #[error("invalid decision provider response: {0}")]
    InvalidResponse(String),
}

#[async_trait]
pub trait DecisionProvider: Send + Sync {
    fn kind(&self) -> DecisionProviderKind;

    async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError>;
}

#[derive(Clone)]
pub struct DecisionClient {
    provider: std::sync::Arc<dyn DecisionProvider>,
}

impl DecisionClient {
    /// Create a provider client with a redirect-disabled HTTP client.
    ///
    /// # Errors
    ///
    /// Returns an error if the endpoint or API key is invalid, or the HTTP
    /// client cannot be initialized.
    pub fn new(config: DecisionClientConfig) -> Result<Self, DecisionError> {
        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| DecisionError::Network(error.to_string()))?;
        Self::with_http_client(config, http_client)
    }

    /// Create a provider client using an existing HTTP client.
    ///
    /// # Errors
    ///
    /// Returns an error if the endpoint or API key is invalid.
    pub fn with_http_client(
        config: DecisionClientConfig,
        http_client: reqwest::Client,
    ) -> Result<Self, DecisionError> {
        validate_endpoint(&config.endpoint)?;
        if config.api_key.trim().is_empty() {
            return Err(DecisionError::InvalidRequest(
                "decision provider API key must not be empty".to_string(),
            ));
        }
        let provider = HttpDecisionProvider {
            kind: config.kind,
            endpoint: config.endpoint,
            api_key: config.api_key,
            timeout_ms: config.timeout_ms.max(1),
            client: http_client,
        };
        Ok(Self {
            provider: std::sync::Arc::new(provider),
        })
    }

    #[must_use]
    pub fn from_provider(provider: std::sync::Arc<dyn DecisionProvider>) -> Self {
        Self { provider }
    }

    #[must_use]
    pub fn kind(&self) -> DecisionProviderKind {
        self.provider.kind()
    }

    /// Submit a typed request and validate the provider's response.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid request, transport/provider failure, or
    /// a response that does not match the requested question schema.
    pub async fn decide(
        &self,
        request: &DecisionRequest,
    ) -> Result<DecisionResponse, DecisionError> {
        self.provider.decide(request).await
    }
}

pub struct DecisionClientConfig {
    kind: DecisionProviderKind,
    api_key: String,
    endpoint: String,
    timeout_ms: u64,
}

impl DecisionClientConfig {
    #[must_use]
    pub fn new(kind: DecisionProviderKind, api_key: impl Into<String>) -> Self {
        Self {
            kind,
            api_key: api_key.into(),
            endpoint: kind.endpoint().to_string(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }

    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    #[must_use]
    pub const fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    #[must_use]
    pub const fn kind(&self) -> DecisionProviderKind {
        self.kind
    }

    /// Load the optional shared decision provider from Routiium environment.
    ///
    /// `ROUTIIUM_DECISION_PROVIDER` selects `openrouter` or `typesafe`. The key
    /// environment variable defaults to the provider's conventional variable.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported provider or when its configured API
    /// key is missing.
    pub fn from_env() -> Result<Option<Self>, DecisionError> {
        let Ok(provider) = std::env::var("ROUTIIUM_DECISION_PROVIDER") else {
            return Ok(None);
        };
        if provider.trim().is_empty() {
            return Ok(None);
        }
        let kind = DecisionProviderKind::parse(&provider)?;
        let api_key_env = std::env::var("ROUTIIUM_DECISION_API_KEY_ENV")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| kind.default_api_key_env().to_string());
        let api_key = std::env::var(&api_key_env)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                DecisionError::InvalidRequest(format!(
                    "decision provider key environment variable {api_key_env} is missing"
                ))
            })?;
        let endpoint = std::env::var("ROUTIIUM_DECISION_ENDPOINT")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| kind.endpoint().to_string());
        let timeout_ms = std::env::var("ROUTIIUM_DECISION_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .max(1);
        Ok(Some(Self {
            kind,
            api_key,
            endpoint,
            timeout_ms,
        }))
    }
}

struct HttpDecisionProvider {
    kind: DecisionProviderKind,
    endpoint: String,
    api_key: String,
    timeout_ms: u64,
    client: reqwest::Client,
}

#[async_trait]
impl DecisionProvider for HttpDecisionProvider {
    fn kind(&self) -> DecisionProviderKind {
        self.kind
    }

    async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        request.validate()?;
        let provider = self.kind.as_str();
        let body = serde_json::to_vec(request)
            .map_err(|error| DecisionError::InvalidRequest(error.to_string()))?;
        let response = self
            .client
            .post(&self.endpoint)
            .timeout(Duration::from_millis(self.timeout_ms))
            .bearer_auth(&self.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    DecisionError::Timeout(error.to_string())
                } else {
                    DecisionError::Network(error.to_string())
                }
            })?;
        let status = response.status();
        let mut stream = response.bytes_stream();
        let mut response_body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| DecisionError::Network(error.to_string()))?;
            if response_body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(DecisionError::InvalidResponse(format!(
                    "{provider} response exceeds {MAX_RESPONSE_BYTES} bytes"
                )));
            }
            response_body.extend_from_slice(&chunk);
        }
        let response_text = String::from_utf8(response_body).map_err(|error| {
            DecisionError::InvalidResponse(format!("provider returned non-UTF-8 JSON: {error}"))
        })?;
        let payload = serde_json::from_str::<Value>(&response_text).map_err(|error| {
            DecisionError::InvalidResponse(format!("provider returned invalid JSON: {error}"))
        })?;
        if !status.is_success() {
            return Err(http_error(
                provider,
                status.as_u16(),
                &payload,
                &self.api_key,
            ));
        }
        let normalized = normalize_envelope(payload, provider, &self.api_key)?;
        let decision = serde_json::from_value::<DecisionResponse>(normalized).map_err(|error| {
            DecisionError::InvalidResponse(format!("provider response schema mismatch: {error}"))
        })?;
        decision.validate_for(request)?;
        Ok(decision)
    }
}

fn normalize_envelope(
    mut payload: Value,
    provider: &'static str,
    api_key: &str,
) -> Result<Value, DecisionError> {
    if let Some(code) = payload.get("code").and_then(Value::as_i64) {
        if code != 0 {
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("provider returned an API error");
            return Err(DecisionError::Api {
                provider,
                code,
                message: redact_and_bound(message, api_key),
            });
        }
    }
    if let Some(data) = payload.get("data").filter(|value| value.is_object()) {
        payload = data.clone();
    }
    if let Some(error) = payload.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("provider returned an API error");
        return Err(DecisionError::Api {
            provider,
            code,
            message: redact_and_bound(message, api_key),
        });
    }
    Ok(payload)
}

fn http_error(
    provider: &'static str,
    status: u16,
    payload: &Value,
    api_key: &str,
) -> DecisionError {
    let message = payload
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("message").and_then(Value::as_str))
        .unwrap_or("provider returned an HTTP error");
    DecisionError::Http {
        provider,
        status,
        message: redact_and_bound(message, api_key),
    }
}

fn redact_and_bound(value: &str, api_key: &str) -> String {
    let redacted = if api_key.is_empty() {
        value.to_string()
    } else {
        value.replace(api_key, "[redacted]")
    };
    bounded_text(&redacted, MAX_ERROR_MESSAGE_CHARS)
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    let mut bounded = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        bounded.push('…');
    }
    bounded
}

fn validate_identifier(value: &str, name: &str, max_bytes: usize) -> Result<(), DecisionError> {
    if value.trim().is_empty() || value.len() > max_bytes {
        return Err(DecisionError::InvalidRequest(format!(
            "{name} must contain between 1 and {max_bytes} bytes"
        )));
    }
    Ok(())
}

fn validate_instructions(value: &str) -> Result<(), DecisionError> {
    if value.trim().is_empty() || value.len() > 4_096 {
        return Err(DecisionError::InvalidRequest(
            "question instructions must contain between 1 and 4096 bytes".to_string(),
        ));
    }
    Ok(())
}

fn validate_description(value: &str) -> Result<(), DecisionError> {
    if value.trim().is_empty() || value.len() > 2_048 {
        return Err(DecisionError::InvalidRequest(
            "question descriptions must contain between 1 and 2048 bytes".to_string(),
        ));
    }
    Ok(())
}

fn validate_probability(value: f64, name: &str) -> Result<(), DecisionError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(DecisionError::InvalidResponse(format!(
            "{name} must be a finite probability between 0 and 1"
        )));
    }
    Ok(())
}

fn validate_probability_map(
    probabilities: &BTreeMap<String, f64>,
    options: &BTreeMap<String, String>,
    id: &str,
) -> Result<(), DecisionError> {
    for (key, value) in probabilities {
        if !options.contains_key(key) {
            return Err(DecisionError::InvalidResponse(format!(
                "choice answer {id} has a probability for an unknown option"
            )));
        }
        validate_probability(*value, &format!("answer {id} probability"))?;
    }
    Ok(())
}

fn validate_endpoint(endpoint: &str) -> Result<(), DecisionError> {
    let url = reqwest::Url::parse(endpoint)
        .map_err(|error| DecisionError::InvalidRequest(format!("invalid endpoint: {error}")))?;
    let host = url.host_str().unwrap_or_default();
    let local_http =
        url.scheme() == "http" && (host == "localhost" || host == "127.0.0.1" || host == "[::1]");
    if url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || (!local_http && url.scheme() != "https")
    {
        return Err(DecisionError::InvalidRequest(
            "decision endpoint must use HTTPS, except for localhost test endpoints".to_string(),
        ));
    }
    Ok(())
}

/// Build a stable ordered question map without forcing callers to repeat map boilerplate.
#[must_use]
pub fn questions(
    entries: impl IntoIterator<Item = (String, DecisionQuestion)>,
) -> BTreeMap<String, DecisionQuestion> {
    entries.into_iter().collect()
}

/// Build a choice criteria map from identifier/description pairs.
#[must_use]
pub fn choice_criteria(
    entries: impl IntoIterator<Item = (String, String)>,
) -> BTreeMap<String, String> {
    entries.into_iter().collect()
}

/// Build a JSON object state while retaining the canonical JSON value type.
#[must_use]
pub fn state_object(entries: impl IntoIterator<Item = (String, Value)>) -> Value {
    Value::Object(entries.into_iter().collect::<Map<String, Value>>())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_used
    )]

    use super::*;
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::routing::post;
    use axum::{Json, Router};
    use std::sync::{Arc, Mutex};

    fn sample_request(model: &str) -> DecisionRequest {
        DecisionRequest::new(
            model,
            serde_json::json!({"message": "Help! My payouts have been failing for 3 days."}),
            questions([
                (
                    "urgent".to_string(),
                    DecisionQuestion::noul("Does this message convey urgency?"),
                ),
                (
                    "department".to_string(),
                    DecisionQuestion::choice(
                        "Which team should handle this?",
                        choice_criteria([
                            (
                                "billing".to_string(),
                                "Payments, invoicing, refunds".to_string(),
                            ),
                            (
                                "technical".to_string(),
                                "Bugs, outages, integrations".to_string(),
                            ),
                        ]),
                    ),
                ),
                (
                    "frustration".to_string(),
                    DecisionQuestion::score(
                        "How frustrated is the customer?",
                        vec![
                            "Calm".to_string(),
                            "Frustrated".to_string(),
                            "Very angry".to_string(),
                        ],
                    ),
                ),
            ]),
        )
        .expect("sample request is valid")
    }

    fn sample_response() -> Value {
        serde_json::json!({
            "id": "decision-test",
            "model": "typesafe/jev-1.13",
            "provider": "TypeSafe",
            "usage": {"input_tokens": 10, "output_tokens": 6, "cost": 0.001},
            "answers": {
                "urgent": {"type": "noul", "noul": 0.95},
                "department": {
                    "type": "choice",
                    "choice": "billing",
                    "probabilities": {"billing": 0.9, "technical": 0.1},
                    "confidence": 0.84
                },
                "frustration": {
                    "type": "score",
                    "score": 1.04,
                    "legend": {"0": "Calm", "1": "Frustrated", "2": "Very angry"},
                    "probabilities": {"0": 0.0, "1": 0.96, "2": 0.04},
                    "confidence": 0.96
                }
            }
        })
    }

    #[test]
    fn serializes_shared_typed_question_shapes() {
        let request = sample_request("~typesafe/jev-latest");
        let value = serde_json::to_value(request).expect("serializes");
        assert_eq!(value["questions"]["urgent"]["type"], "noul");
        assert_eq!(value["questions"]["department"]["type"], "choice");
        assert_eq!(value["questions"]["frustration"]["type"], "score");
    }

    #[test]
    fn validates_typed_answers_against_questions() {
        let request = sample_request("typesafe/jev-1.13");
        let response =
            serde_json::from_value::<DecisionResponse>(sample_response()).expect("response parses");
        response.validate_for(&request).expect("response matches");
        assert_eq!(response.noul_probability("urgent"), Some(0.95));
        assert_eq!(response.choice("department"), Some(("billing", Some(0.84))));

        let mut invalid = response;
        if let Some(DecisionAnswer::Choice { choice, .. }) = invalid.answers.get_mut("department") {
            *choice = "sales".to_string();
        }
        assert!(invalid.validate_for(&request).is_err());
    }

    #[test]
    fn rejects_invalid_score_scales_and_probabilities() {
        assert!(DecisionRequest::new(
            "jev",
            Value::Null,
            questions([(
                "score".to_string(),
                DecisionQuestion::score("score this", vec!["only one".to_string()]),
            )]),
        )
        .is_err());

        let request = sample_request("jev");
        let mut payload = sample_response();
        payload["answers"]["urgent"]["noul"] = serde_json::json!(1.1);
        let response = serde_json::from_value::<DecisionResponse>(payload).expect("parses");
        assert!(response.validate_for(&request).is_err());
    }

    #[derive(Clone, Default)]
    struct CapturedRequests(Arc<Mutex<Vec<(String, Value)>>>);

    async fn capture_decision(
        State(captured): State<CapturedRequests>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        captured
            .0
            .lock()
            .expect("request capture lock")
            .push((authorization, body));
        Json(sample_response())
    }

    async fn decision_auth_error() -> (axum::http::StatusCode, Json<Value>) {
        (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": {"code": 401, "message": "invalid decision-test-key"}
            })),
        )
    }

    #[tokio::test]
    #[allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic
    )]
    async fn provider_adapters_use_distinct_paths_and_the_same_body() {
        let captured = CapturedRequests::default();
        let app = Router::new()
            .route("/api/alpha/decisions", post(capture_decision))
            .route("/v1/systemone", post(capture_decision))
            .with_state(captured.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind provider stub");
        let address = listener.local_addr().expect("stub address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve provider stub");
        });

        let request = sample_request("typesafe/jev-1.13");
        for (kind, path) in [
            (DecisionProviderKind::OpenRouter, "/api/alpha/decisions"),
            (DecisionProviderKind::TypeSafe, "/v1/systemone"),
        ] {
            let endpoint = format!("http://{address}{path}");
            let client = DecisionClient::new(
                DecisionClientConfig::new(kind, "decision-test-key").with_endpoint(endpoint),
            )
            .expect("decision client");
            let response = client.decide(&request).await.expect("decision response");
            assert_eq!(response.choice("department"), Some(("billing", Some(0.84))));
        }

        server.abort();
        let calls = captured.0.lock().expect("request capture lock");
        assert_eq!(calls.len(), 2);
        for (authorization, body) in calls.iter() {
            assert_eq!(authorization, "Bearer decision-test-key");
            assert_eq!(body["model"], "typesafe/jev-1.13");
            assert!(body["state"].is_object());
            assert_eq!(body["questions"]["department"]["type"], "choice");
        }
    }

    #[tokio::test]
    #[allow(clippy::expect_used, clippy::unwrap_used)]
    async fn provider_errors_do_not_echo_the_api_key() {
        let app = Router::new().route("/api/alpha/decisions", post(decision_auth_error));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind error stub");
        let address = listener.local_addr().expect("error stub address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve error stub");
        });
        let client = DecisionClient::new(
            DecisionClientConfig::new(DecisionProviderKind::OpenRouter, "decision-test-key")
                .with_endpoint(format!("http://{address}/api/alpha/decisions")),
        )
        .expect("decision client");
        let error = client
            .decide(&sample_request("typesafe/jev-1.13"))
            .await
            .expect_err("provider should reject the key");
        server.abort();
        let message = error.to_string();
        assert!(!message.contains("decision-test-key"));
        assert!(message.contains("[redacted]"));
    }
}
