use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AnalyticsError {
    #[error("Storage error: {0}")]
    Storage(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("Sled error: {0}")]
    Sled(String),
}

/// Comprehensive analytics event capturing all aspects of a request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsEvent {
    /// Unique event ID
    pub id: String,
    /// Unix timestamp (seconds)
    pub timestamp: u64,
    /// Request metadata
    pub request: RequestMetadata,
    /// Response metadata
    pub response: Option<ResponseMetadata>,
    /// Performance metrics
    pub performance: PerformanceMetrics,
    /// Authentication info
    pub auth: AuthMetadata,
    /// Backend routing info
    pub routing: RoutingMetadata,
    /// Detailed token usage
    pub token_usage: Option<TokenUsage>,
    /// Cost information
    pub cost: Option<CostInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetadata {
    /// Endpoint path (e.g., "/v1/chat/completions")
    pub endpoint: String,
    /// HTTP method
    pub method: String,
    /// Model requested
    pub model: Option<String>,
    /// Whether streaming was requested
    pub stream: bool,
    /// Request size in bytes
    pub size_bytes: usize,
    /// Number of messages in request
    pub message_count: Option<usize>,
    /// Total tokens in input (if available)
    pub input_tokens: Option<u64>,
    /// User agent
    pub user_agent: Option<String>,
    /// Client IP (if available)
    pub client_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMetadata {
    /// HTTP status code
    pub status_code: u16,
    /// Response size in bytes
    pub size_bytes: usize,
    /// Output tokens (if available from response)
    pub output_tokens: Option<u64>,
    /// Whether response was successful
    pub success: bool,
    /// Error message if failed
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Input/prompt tokens
    pub prompt_tokens: u64,
    /// Output/completion tokens
    pub completion_tokens: u64,
    /// Total tokens (prompt + completion)
    pub total_tokens: u64,
    /// Cached tokens (if applicable)
    pub cached_tokens: Option<u64>,
    /// Reasoning tokens (for o1/o3 models)
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostInfo {
    /// Cost for input tokens
    pub input_cost: f64,
    /// Cost for output tokens
    pub output_cost: f64,
    /// Cost for cached tokens (if applicable)
    pub cached_cost: Option<f64>,
    /// Total cost
    pub total_cost: f64,
    /// Currency (e.g., "USD")
    pub currency: String,
    /// Pricing model used for calculation
    pub pricing_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// Total request duration in milliseconds
    pub duration_ms: u64,
    /// Time to first byte (for streaming)
    pub ttfb_ms: Option<u64>,
    /// Upstream request duration
    pub upstream_duration_ms: Option<u64>,
    /// Tokens per second (output tokens / duration)
    pub tokens_per_second: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthMetadata {
    /// Whether request was authenticated
    pub authenticated: bool,
    /// API key ID (not the key itself)
    pub api_key_id: Option<String>,
    /// API key label
    pub api_key_label: Option<String>,
    /// Auth method (bearer, etc.)
    pub auth_method: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingMetadata {
    /// Backend that handled the request
    pub backend: String,
    /// Upstream mode (chat/responses)
    pub upstream_mode: String,
    /// Whether MCP was used
    pub mcp_enabled: bool,
    /// MCP servers invoked
    pub mcp_servers: Vec<String>,
    /// System prompt applied
    pub system_prompt_applied: bool,
}

/// Analytics aggregation for time-based queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsAggregation {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cached_tokens: u64,
    pub total_reasoning_tokens: u64,
    pub avg_duration_ms: f64,
    pub avg_tokens_per_second: Option<f64>,
    pub total_cost: f64,
    pub cost_by_model: std::collections::HashMap<String, f64>,
    pub models_used: std::collections::HashMap<String, u64>,
    pub endpoints_hit: std::collections::HashMap<String, u64>,
    pub backends_used: std::collections::HashMap<String, u64>,
    pub period_start: u64,
    pub period_end: u64,
}

pub(crate) struct JsonlBackend {
    path: PathBuf,
    writer: Arc<tokio::sync::Mutex<File>>,
    total_events: Arc<tokio::sync::RwLock<usize>>,
}

/// Storage backend for analytics
enum AnalyticsBackend {
    Redis(redis::Client),
    #[cfg(feature = "sled")]
    Sled(sled::Db),
    Memory(Arc<tokio::sync::RwLock<Vec<AnalyticsEvent>>>),
    Jsonl(JsonlBackend),
}

pub struct AnalyticsManager {
    backend: AnalyticsBackend,
    /// Maximum events to keep in memory mode
    max_events: usize,
    /// TTL for events in seconds (Redis/Sled)
    ttl_seconds: Option<u64>,
}

impl AnalyticsManager {
    /// Create with Redis backend
    pub fn new_redis(url: &str, ttl_seconds: Option<u64>) -> Result<Self, AnalyticsError> {
        let client = redis::Client::open(url)?;
        Ok(Self {
            backend: AnalyticsBackend::Redis(client),
            max_events: 10000,
            ttl_seconds,
        })
    }

    /// Create with Sled backend
    #[cfg(feature = "sled")]
    pub fn new_sled(path: &str, ttl_seconds: Option<u64>) -> Result<Self, AnalyticsError> {
        let db = sled::open(path).map_err(|e| AnalyticsError::Sled(e.to_string()))?;
        Ok(Self {
            backend: AnalyticsBackend::Sled(db),
            max_events: 100000,
            ttl_seconds,
        })
    }

    /// Create with JSONL backend
    pub fn new_jsonl<P: Into<PathBuf>>(path: P) -> Result<Self, AnalyticsError> {
        let path = path.into();

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| AnalyticsError::Storage(e.to_string()))?;
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| AnalyticsError::Storage(e.to_string()))?;

        let initial_count = count_jsonl_events(&path)?;

        Ok(Self {
            backend: AnalyticsBackend::Jsonl(JsonlBackend {
                path,
                writer: Arc::new(tokio::sync::Mutex::new(file)),
                total_events: Arc::new(tokio::sync::RwLock::new(initial_count)),
            }),
            max_events: usize::MAX,
            ttl_seconds: None,
        })
    }

    /// Create with in-memory backend
    pub fn new_memory(max_events: usize) -> Self {
        Self {
            backend: AnalyticsBackend::Memory(Arc::new(tokio::sync::RwLock::new(Vec::new()))),
            max_events,
            ttl_seconds: None,
        }
    }

    /// Create from environment configuration
    pub fn from_env() -> Result<Self, AnalyticsError> {
        // Check for Redis URL
        if let Ok(url) = std::env::var("ROUTIIUM_ANALYTICS_REDIS_URL") {
            let url = url.trim();
            if !url.is_empty() {
                let ttl = std::env::var("ROUTIIUM_ANALYTICS_TTL_SECONDS")
                    .ok()
                    .and_then(|s| s.parse().ok());
                return Self::new_redis(url, ttl);
            }
        }

        // Check for Sled path
        #[cfg(feature = "sled")]
        if let Ok(path) = std::env::var("ROUTIIUM_ANALYTICS_SLED_PATH") {
            let path = path.trim();
            if !path.is_empty() {
                let ttl = std::env::var("ROUTIIUM_ANALYTICS_TTL_SECONDS")
                    .ok()
                    .and_then(|s| s.parse().ok());
                return Self::new_sled(path, ttl);
            }
        }

        // Check for JSONL path
        if let Ok(path) = std::env::var("ROUTIIUM_ANALYTICS_JSONL_PATH") {
            let path = path.trim();
            if !path.is_empty() {
                return Self::new_jsonl(path);
            }
        }

        // Allow forcing the legacy in-memory backend
        let force_memory = std::env::var("ROUTIIUM_ANALYTICS_FORCE_MEMORY")
            .ok()
            .map(|v| {
                let normalized = v.trim().to_ascii_lowercase();
                normalized == "1" || normalized == "true" || normalized == "yes"
            })
            .unwrap_or(false);

        if force_memory {
            let max_events = std::env::var("ROUTIIUM_ANALYTICS_MAX_EVENTS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10000);

            return Ok(Self::new_memory(max_events));
        }

        // Default to JSONL file backend
        match Self::new_jsonl("data/analytics.jsonl") {
            Ok(manager) => Ok(manager),
            Err(err) => {
                // Final fallback to in-memory if JSONL initialization fails
                tracing::warn!("Failed to initialize JSONL analytics backend: {}", err);
                let max_events = std::env::var("ROUTIIUM_ANALYTICS_MAX_EVENTS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(10000);
                Ok(Self::new_memory(max_events))
            }
        }
    }

    /// Record an analytics event
    pub async fn record(&self, event: AnalyticsEvent) -> Result<(), AnalyticsError> {
        match &self.backend {
            AnalyticsBackend::Redis(client) => {
                let mut conn = client.get_connection()?;
                let key = format!("analytics:event:{}", event.id);
                let value = serde_json::to_string(&event)?;

                redis::cmd("SET")
                    .arg(&key)
                    .arg(value)
                    .query::<()>(&mut conn)?;

                // Set TTL if configured
                if let Some(ttl) = self.ttl_seconds {
                    redis::cmd("EXPIRE")
                        .arg(&key)
                        .arg(ttl)
                        .query::<()>(&mut conn)?;
                }

                // Add to sorted set by timestamp for range queries
                redis::cmd("ZADD")
                    .arg("analytics:events:by_time")
                    .arg(event.timestamp)
                    .arg(&event.id)
                    .query::<()>(&mut conn)?;

                // Index by model
                if let Some(ref model) = event.request.model {
                    redis::cmd("SADD")
                        .arg(format!("analytics:events:by_model:{}", model))
                        .arg(&event.id)
                        .query::<()>(&mut conn)?;
                }

                // Index by endpoint
                redis::cmd("SADD")
                    .arg(format!(
                        "analytics:events:by_endpoint:{}",
                        event.request.endpoint
                    ))
                    .arg(&event.id)
                    .query::<()>(&mut conn)?;

                Ok(())
            }
            #[cfg(feature = "sled")]
            AnalyticsBackend::Sled(db) => {
                let key = format!("event:{}", event.id);
                let value = serde_json::to_vec(&event)?;
                db.insert(key.as_bytes(), value)
                    .map_err(|e| AnalyticsError::Sled(e.to_string()))?;

                // Store timestamp index
                let ts_key = format!("ts:{}:{}", event.timestamp, event.id);
                db.insert(ts_key.as_bytes(), event.id.as_bytes())
                    .map_err(|e| AnalyticsError::Sled(e.to_string()))?;

                Ok(())
            }
            AnalyticsBackend::Memory(events) => {
                let mut events = events.write().await;
                events.push(event);

                // Trim to max size
                if events.len() > self.max_events {
                    let excess = events.len() - self.max_events;
                    events.drain(0..excess);
                }

                Ok(())
            }
            AnalyticsBackend::Jsonl(backend) => {
                let serialized = serde_json::to_string(&event)?;
                {
                    let mut file = backend.writer.lock().await;
                    file.write_all(serialized.as_bytes())
                        .map_err(|e| AnalyticsError::Storage(e.to_string()))?;
                    file.write_all(b"\n")
                        .map_err(|e| AnalyticsError::Storage(e.to_string()))?;
                    file.flush()
                        .map_err(|e| AnalyticsError::Storage(e.to_string()))?;
                }

                let mut count = backend.total_events.write().await;
                *count += 1;

                Ok(())
            }
        }
    }

    /// Query events by time range
    pub async fn query_range(
        &self,
        start_ts: u64,
        end_ts: u64,
        limit: Option<usize>,
    ) -> Result<Vec<AnalyticsEvent>, AnalyticsError> {
        match &self.backend {
            AnalyticsBackend::Redis(client) => {
                let mut conn = client.get_connection()?;

                // Get event IDs in time range
                let ids: Vec<String> = redis::cmd("ZRANGEBYSCORE")
                    .arg("analytics:events:by_time")
                    .arg(start_ts)
                    .arg(end_ts)
                    .query(&mut conn)?;

                let ids_to_fetch = if let Some(limit) = limit {
                    ids.into_iter().take(limit).collect::<Vec<_>>()
                } else {
                    ids
                };

                let mut events = Vec::new();
                for id in ids_to_fetch {
                    let key = format!("analytics:event:{}", id);
                    if let Ok(value) = redis::cmd("GET").arg(&key).query::<String>(&mut conn) {
                        if let Ok(event) = serde_json::from_str(&value) {
                            events.push(event);
                        }
                    }
                }

                Ok(events)
            }
            #[cfg(feature = "sled")]
            AnalyticsBackend::Sled(db) => {
                let mut events = Vec::new();
                let start_key = format!("ts:{}", start_ts);
                let end_key = format!("ts:{}", end_ts + 1);

                for (_, event_id_bytes) in
                    db.range(start_key.as_bytes()..end_key.as_bytes()).flatten()
                {
                    if let Ok(event_id) = String::from_utf8(event_id_bytes.to_vec()) {
                        let key = format!("event:{}", event_id);
                        if let Ok(Some(event_bytes)) = db.get(key.as_bytes()) {
                            if let Ok(event) = serde_json::from_slice(&event_bytes) {
                                events.push(event);
                                if let Some(limit) = limit {
                                    if events.len() >= limit {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                Ok(events)
            }
            AnalyticsBackend::Memory(events) => {
                let events = events.read().await;
                let mut filtered: Vec<_> = events
                    .iter()
                    .filter(|e| e.timestamp >= start_ts && e.timestamp <= end_ts)
                    .cloned()
                    .collect();

                if let Some(limit) = limit {
                    filtered.truncate(limit);
                }

                Ok(filtered)
            }
            AnalyticsBackend::Jsonl(backend) => {
                let file = match File::open(&backend.path) {
                    Ok(file) => file,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                    Err(err) => return Err(AnalyticsError::Storage(err.to_string())),
                };

                let reader = BufReader::new(file);
                let mut events = Vec::new();

                for line in reader.lines() {
                    let line = match line {
                        Ok(line) => line,
                        Err(err) => {
                            return Err(AnalyticsError::Storage(err.to_string()));
                        }
                    };

                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    match serde_json::from_str::<AnalyticsEvent>(trimmed) {
                        Ok(event) => {
                            if event.timestamp >= start_ts && event.timestamp <= end_ts {
                                events.push(event);
                                if let Some(limit) = limit {
                                    if events.len() >= limit {
                                        break;
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!("Skipping malformed analytics event: {}", err);
                        }
                    }
                }

                Ok(events)
            }
        }
    }

    /// Aggregate events in a time range
    pub async fn aggregate(
        &self,
        start_ts: u64,
        end_ts: u64,
    ) -> Result<AnalyticsAggregation, AnalyticsError> {
        let events = self.query_range(start_ts, end_ts, None).await?;

        let mut agg = AnalyticsAggregation {
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cached_tokens: 0,
            total_reasoning_tokens: 0,
            avg_duration_ms: 0.0,
            avg_tokens_per_second: None,
            total_cost: 0.0,
            cost_by_model: std::collections::HashMap::new(),
            models_used: std::collections::HashMap::new(),
            endpoints_hit: std::collections::HashMap::new(),
            backends_used: std::collections::HashMap::new(),
            period_start: start_ts,
            period_end: end_ts,
        };

        let mut total_duration = 0u64;
        let mut total_tps = 0.0;
        let mut tps_count = 0u64;

        for event in events {
            agg.total_requests += 1;

            if let Some(ref response) = event.response {
                if response.success {
                    agg.successful_requests += 1;
                } else {
                    agg.failed_requests += 1;
                }

                // Prefer detailed token_usage when present to avoid inconsistencies.
                if event.token_usage.is_none() {
                    if let Some(tokens) = response.output_tokens {
                        agg.total_output_tokens += tokens;
                    }
                }
            }

            // Aggregate token usage details
            if let Some(ref usage) = event.token_usage {
                agg.total_input_tokens += usage.prompt_tokens;
                agg.total_output_tokens += usage.completion_tokens;
                if let Some(cached) = usage.cached_tokens {
                    agg.total_cached_tokens += cached;
                }
                if let Some(reasoning) = usage.reasoning_tokens {
                    agg.total_reasoning_tokens += reasoning;
                }
            } else if let Some(tokens) = event.request.input_tokens {
                agg.total_input_tokens += tokens;
            }

            // Aggregate cost
            if let Some(ref cost) = event.cost {
                agg.total_cost += cost.total_cost;

                if let Some(ref model) = event.request.model {
                    *agg.cost_by_model.entry(model.clone()).or_insert(0.0) += cost.total_cost;
                }
            }

            // Aggregate tokens per second
            if let Some(tps) = event.performance.tokens_per_second {
                total_tps += tps;
                tps_count += 1;
            }

            if let Some(ref model) = event.request.model {
                *agg.models_used.entry(model.clone()).or_insert(0) += 1;
            }

            *agg.endpoints_hit
                .entry(event.request.endpoint.clone())
                .or_insert(0) += 1;

            *agg.backends_used
                .entry(event.routing.backend.clone())
                .or_insert(0) += 1;

            total_duration += event.performance.duration_ms;
        }

        if agg.total_requests > 0 {
            agg.avg_duration_ms = total_duration as f64 / agg.total_requests as f64;
        }

        if tps_count > 0 {
            agg.avg_tokens_per_second = Some(total_tps / tps_count as f64);
        }

        Ok(agg)
    }

    /// Get statistics about the analytics system
    pub async fn stats(&self) -> Result<AnalyticsStats, AnalyticsError> {
        match &self.backend {
            AnalyticsBackend::Redis(client) => {
                let mut conn = client.get_connection()?;
                let count: usize = redis::cmd("ZCARD")
                    .arg("analytics:events:by_time")
                    .query(&mut conn)
                    .unwrap_or(0);

                Ok(AnalyticsStats {
                    total_events: count,
                    backend_type: "redis".to_string(),
                    ttl_seconds: self.ttl_seconds,
                    max_events: None,
                })
            }
            #[cfg(feature = "sled")]
            AnalyticsBackend::Sled(db) => {
                let count = db.scan_prefix("event:").count();

                Ok(AnalyticsStats {
                    total_events: count,
                    backend_type: "sled".to_string(),
                    ttl_seconds: self.ttl_seconds,
                    max_events: Some(self.max_events),
                })
            }
            AnalyticsBackend::Memory(events) => {
                let count = events.read().await.len();

                Ok(AnalyticsStats {
                    total_events: count,
                    backend_type: "memory".to_string(),
                    ttl_seconds: None,
                    max_events: Some(self.max_events),
                })
            }
            AnalyticsBackend::Jsonl(backend) => {
                let count = *backend.total_events.read().await;

                Ok(AnalyticsStats {
                    total_events: count,
                    backend_type: "jsonl".to_string(),
                    ttl_seconds: None,
                    max_events: None,
                })
            }
        }
    }

    /// Clear all analytics data
    pub async fn clear(&self) -> Result<(), AnalyticsError> {
        match &self.backend {
            AnalyticsBackend::Redis(client) => {
                let mut conn = client.get_connection()?;

                // Delete all analytics keys
                let keys: Vec<String> = redis::cmd("KEYS").arg("analytics:*").query(&mut conn)?;

                if !keys.is_empty() {
                    redis::cmd("DEL").arg(&keys).query::<()>(&mut conn)?;
                }

                Ok(())
            }
            #[cfg(feature = "sled")]
            AnalyticsBackend::Sled(db) => {
                db.clear()
                    .map_err(|e| AnalyticsError::Sled(e.to_string()))?;
                Ok(())
            }
            AnalyticsBackend::Memory(events) => {
                events.write().await.clear();
                Ok(())
            }
            AnalyticsBackend::Jsonl(backend) => {
                {
                    let mut file = backend.writer.lock().await;
                    file.set_len(0)
                        .map_err(|e| AnalyticsError::Storage(e.to_string()))?;
                    file.seek(SeekFrom::Start(0))
                        .map_err(|e| AnalyticsError::Storage(e.to_string()))?;
                    file.flush()
                        .map_err(|e| AnalyticsError::Storage(e.to_string()))?;
                }

                let mut count = backend.total_events.write().await;
                *count = 0;

                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsStats {
    pub total_events: usize,
    pub backend_type: String,
    pub ttl_seconds: Option<u64>,
    pub max_events: Option<usize>,
}

fn count_jsonl_events(path: &Path) -> Result<usize, AnalyticsError> {
    match File::open(path) {
        Ok(file) => {
            let reader = BufReader::new(file);
            let mut count = 0usize;
            for line in reader.lines() {
                match line {
                    Ok(content) => {
                        if !content.trim().is_empty() {
                            count += 1;
                        }
                    }
                    Err(err) => {
                        return Err(AnalyticsError::Storage(err.to_string()));
                    }
                }
            }
            Ok(count)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(AnalyticsError::Storage(err.to_string())),
    }
}

/// Helper to create event ID
pub fn generate_event_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Helper to get current timestamp
pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event(
        id: &str,
        request_input_tokens: Option<u64>,
        response_output_tokens: Option<u64>,
        token_usage: Option<TokenUsage>,
    ) -> AnalyticsEvent {
        AnalyticsEvent {
            id: id.to_string(),
            timestamp: current_timestamp(),
            request: RequestMetadata {
                endpoint: "/v1/chat/completions".to_string(),
                method: "POST".to_string(),
                model: Some("alias-x".to_string()),
                stream: false,
                size_bytes: 123,
                message_count: Some(1),
                input_tokens: request_input_tokens,
                user_agent: None,
                client_ip: None,
            },
            response: Some(ResponseMetadata {
                status_code: 200,
                size_bytes: 456,
                output_tokens: response_output_tokens,
                success: true,
                error_message: None,
            }),
            performance: PerformanceMetrics {
                duration_ms: 10,
                ttfb_ms: None,
                upstream_duration_ms: None,
                tokens_per_second: None,
            },
            auth: AuthMetadata {
                authenticated: true,
                api_key_id: Some("key_1".to_string()),
                api_key_label: Some("test".to_string()),
                auth_method: Some("bearer".to_string()),
            },
            routing: RoutingMetadata {
                backend: "routing_config".to_string(),
                upstream_mode: "responses".to_string(),
                mcp_enabled: false,
                mcp_servers: Vec::new(),
                system_prompt_applied: false,
            },
            token_usage,
            cost: None,
        }
    }

    #[tokio::test]
    async fn aggregate_prefers_token_usage_when_present() {
        let manager = AnalyticsManager::new_memory(10);
        manager
            .record(sample_event(
                "evt_1",
                Some(999),
                Some(888),
                Some(TokenUsage {
                    prompt_tokens: 42,
                    completion_tokens: 17,
                    total_tokens: 59,
                    cached_tokens: Some(10),
                    reasoning_tokens: Some(2),
                }),
            ))
            .await
            .expect("record event");

        let agg = manager
            .aggregate(0, current_timestamp() + 60)
            .await
            .expect("aggregate");

        assert_eq!(agg.total_input_tokens, 42);
        assert_eq!(agg.total_output_tokens, 17);
        assert_eq!(agg.total_cached_tokens, 10);
        assert_eq!(agg.total_reasoning_tokens, 2);
    }

    #[tokio::test]
    async fn aggregate_falls_back_without_token_usage() {
        let manager = AnalyticsManager::new_memory(10);
        manager
            .record(sample_event("evt_2", Some(70), Some(30), None))
            .await
            .expect("record event");

        let agg = manager
            .aggregate(0, current_timestamp() + 60)
            .await
            .expect("aggregate");

        assert_eq!(agg.total_input_tokens, 70);
        assert_eq!(agg.total_output_tokens, 30);
        assert_eq!(agg.total_cached_tokens, 0);
        assert_eq!(agg.total_reasoning_tokens, 0);
    }
}
