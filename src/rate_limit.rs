//! Rate limiting and concurrency control for Routiium.
//!
//! Supports:
//! - Multiple rate limit buckets per API key (e.g. 500/day + 100/minute)
//! - Fixed and sliding window algorithms
//! - File-based, Sled, and Redis storage backends
//! - Dynamic admin API for policy management
//! - Per-key policy overrides and emergency blocks
//! - Concurrent request limiting with optional queue support
//! - In-process analytics event recording

#![forbid(unsafe_code)]

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::info;

// ============================================================================
// Core Types
// ============================================================================

/// Window type for rate limit buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
pub enum WindowType {
    /// Counter resets at fixed interval boundaries (e.g. every hour on the hour).
    #[default]
    Fixed,
    /// Sliding window: looks back `window_seconds` from now (two-bucket approximation).
    Sliding,
}

/// A single rate limit constraint with a specific time window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitBucket {
    /// Human-readable name, e.g. "daily", "burst", "five_hour".
    pub name: String,
    /// Maximum requests allowed within the window.
    pub requests: u64,
    /// Window duration in seconds (e.g. 86400 for 1 day).
    pub window_seconds: u64,
    /// Window algorithm.
    #[serde(default)]
    pub window_type: WindowType,
}

/// A collection of rate limit buckets forming a policy.
/// All buckets must pass for a request to be allowed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitPolicy {
    pub id: String,
    pub buckets: Vec<RateLimitBucket>,
}

/// Queue strategy for concurrency control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum QueueStrategy {
    /// Return 429 immediately when at capacity.
    #[default]
    Reject,
    /// Queue requests FIFO; process as slots free up.
    QueueFifo,
    /// Queue with priority (future: by tier/scope; currently behaves like Reject).
    QueuePriority,
}

/// Concurrency (in-flight request) limits for a policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcurrencyConfig {
    /// Maximum simultaneous in-flight requests per key.
    pub max_concurrent: u32,
    /// Maximum queued requests when at capacity (0 = disable queuing).
    #[serde(default)]
    pub max_queue_size: u32,
    /// Maximum time (ms) to wait in the queue before rejecting.
    #[serde(default = "default_queue_timeout_ms")]
    pub queue_timeout_ms: u64,
    /// What to do when at capacity.
    #[serde(default)]
    pub strategy: QueueStrategy,
}

fn default_queue_timeout_ms() -> u64 {
    30_000
}

/// Combined rate limit + concurrency policy for an API key.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RequestPolicy {
    pub rate_limits: Option<RateLimitPolicy>,
    pub concurrency: Option<ConcurrencyConfig>,
}

// ============================================================================
// File-based configuration
// ============================================================================

/// Policy definition as found in the JSON config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDef {
    pub buckets: Vec<RateLimitBucket>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<ConcurrencyConfig>,
}

/// JSON config file structure for rate limits (hot-reloadable).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RateLimitFileConfig {
    #[serde(default = "default_version")]
    pub version: String,
    /// ID of the policy used when no per-key mapping exists.
    pub default_policy: Option<String>,
    /// Named policies.
    #[serde(default)]
    pub policies: HashMap<String, PolicyDef>,
    /// key_id → policy_id overrides.
    #[serde(default)]
    pub key_overrides: HashMap<String, String>,
}

fn default_version() -> String {
    "1.0".to_string()
}

impl RateLimitFileConfig {
    pub fn load_from_file(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }

    pub fn empty() -> Self {
        Self::default()
    }

    /// Convert to a flat list of `RateLimitPolicy` objects.
    pub fn to_policies(&self) -> Vec<RateLimitPolicy> {
        self.policies
            .iter()
            .map(|(id, def)| RateLimitPolicy {
                id: id.clone(),
                buckets: def.buckets.clone(),
            })
            .collect()
    }
}

// ============================================================================
// Check / status types
// ============================================================================

/// Status of a single rate limit bucket after a check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketStatus {
    pub name: String,
    pub limit: u64,
    pub remaining: u64,
    pub used: u64,
    pub allowed: bool,
    /// Unix timestamp (seconds) when this window resets.
    pub reset_at: u64,
    pub window_seconds: u64,
}

/// Result of checking all buckets in a policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitCheckResult {
    pub allowed: bool,
    pub policy_id: String,
    pub buckets: Vec<BucketStatus>,
    /// If rejected, which bucket caused it (first one that exceeded).
    pub rejected_bucket: Option<BucketStatus>,
}

/// Information about an active emergency block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockInfo {
    pub key_id: String,
    /// Unix timestamp when block expires; 0 = permanent.
    pub until_secs: u64,
    pub reason: String,
}

// ============================================================================
// Analytics types
// ============================================================================

/// Event types for rate limiting analytics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RateLimitEventType {
    RateLimitChecked,
    RateLimitAllowed,
    RateLimitRejected,
    RateLimitWarning,
    ConcurrencyAcquired,
    ConcurrencyReleased,
    ConcurrencyQueued,
    ConcurrencyDequeued,
    ConcurrencyQueueTimeout,
    ConcurrencyRejected,
}

/// Analytics event recorded for every rate limit decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitAnalyticsEvent {
    pub id: String,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    pub api_key_id: String,
    pub event_type: RateLimitEventType,
    pub policy_id: Option<String>,
    pub bucket_name: Option<String>,
    pub limit: Option<u64>,
    pub remaining: Option<u64>,
    pub window_seconds: Option<u64>,
    pub active_requests: Option<u32>,
    pub max_concurrent: Option<u32>,
    pub queue_position: Option<u32>,
    pub queue_size: Option<u32>,
    pub wait_time_ms: Option<u64>,
    pub endpoint: String,
    pub model: Option<String>,
    pub request_size_bytes: Option<usize>,
    pub duration_ms: Option<u64>,
}

// ============================================================================
// In-memory metrics snapshot
// ============================================================================

/// Aggregate metrics for a single API key (admin endpoint use).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RateLimitMetrics {
    pub total_checks: u64,
    pub total_allowed: u64,
    pub total_rejected: u64,
    pub rejections_by_bucket: HashMap<String, u64>,
    pub concurrency_rejected: u64,
    pub concurrency_queued: u64,
    pub concurrency_queue_timeouts: u64,
}

// ============================================================================
// Storage trait
// ============================================================================

/// Async storage trait for rate limit counters and policy configuration.
#[async_trait]
pub trait RateLimitStore: Send + Sync {
    // --- Counter operations ---

    /// Atomically check whether the request is allowed and, if so, increment
    /// the counter. Returns a `BucketStatus` reflecting the state after the op.
    async fn check_and_increment(
        &self,
        key_id: &str,
        bucket: &RateLimitBucket,
        now_secs: u64,
    ) -> Result<BucketStatus>;

    /// Read current usage without incrementing (for status/admin endpoints).
    async fn get_bucket_status(
        &self,
        key_id: &str,
        bucket: &RateLimitBucket,
        now_secs: u64,
    ) -> Result<BucketStatus>;

    // --- Policy CRUD ---

    async fn save_policy(&self, policy: &RateLimitPolicy) -> Result<()>;
    async fn get_policy(&self, id: &str) -> Result<Option<RateLimitPolicy>>;
    async fn delete_policy(&self, id: &str) -> Result<bool>;
    async fn list_policies(&self) -> Result<Vec<RateLimitPolicy>>;

    // --- Key-to-policy mapping ---

    async fn set_key_policy(&self, key_id: &str, policy_id: &str) -> Result<()>;
    async fn get_key_policy(&self, key_id: &str) -> Result<Option<String>>;
    async fn remove_key_policy(&self, key_id: &str) -> Result<bool>;

    // --- Default policy ---

    async fn set_default_policy_id(&self, policy_id: &str) -> Result<()>;
    async fn get_default_policy_id(&self) -> Result<Option<String>>;

    // --- Emergency blocks ---

    async fn block_key(&self, key_id: &str, until_secs: u64, reason: &str) -> Result<()>;
    async fn unblock_key(&self, key_id: &str) -> Result<()>;
    async fn get_block(&self, key_id: &str) -> Result<Option<BlockInfo>>;
    async fn list_blocks(&self) -> Result<Vec<BlockInfo>>;
}

// ============================================================================
// Shared helpers
// ============================================================================

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn window_start_for(now_secs: u64, window_seconds: u64) -> u64 {
    if window_seconds == 0 {
        return 0;
    }
    now_secs - (now_secs % window_seconds)
}

fn counter_key(key_id: &str, bucket_name: &str, ws: u64) -> String {
    format!("rl:c:{}:{}:{}", key_id, bucket_name, ws)
}

// ============================================================================
// Memory store
// ============================================================================

struct MemoryInner {
    /// counter_key → (count, window_start, expiry_secs)
    counters: HashMap<String, (u64, u64, u64)>,
    policies: HashMap<String, RateLimitPolicy>,
    key_policies: HashMap<String, String>,
    default_policy: Option<String>,
    blocks: HashMap<String, BlockInfo>,
}

impl MemoryInner {
    fn new() -> Self {
        Self {
            counters: HashMap::new(),
            policies: HashMap::new(),
            key_policies: HashMap::new(),
            default_policy: None,
            blocks: HashMap::new(),
        }
    }

    fn get_count(&self, ck: &str, expected_ws: u64, now_secs: u64) -> u64 {
        match self.counters.get(ck) {
            Some(&(count, ws, expiry)) if expiry > now_secs && ws == expected_ws => count,
            _ => 0,
        }
    }

    /// Atomic check-and-increment for a fixed window.
    fn increment_fixed(
        &mut self,
        ck: &str,
        ws: u64,
        window_seconds: u64,
        limit: u64,
        now_secs: u64,
    ) -> (bool, u64) {
        let expiry = ws + window_seconds;
        let entry = self
            .counters
            .entry(ck.to_string())
            .or_insert((0, ws, expiry));

        // Reset if window has shifted or expired.
        if entry.2 <= now_secs || entry.1 != ws {
            *entry = (0, ws, expiry);
        }

        let new_count = entry.0 + 1;
        if new_count <= limit {
            entry.0 = new_count;
            (true, new_count)
        } else {
            (false, entry.0)
        }
    }

    /// Atomic check-and-increment for a sliding window (two-bucket approximation).
    #[allow(clippy::too_many_arguments)]
    fn increment_sliding(
        &mut self,
        curr_ck: &str,
        prev_ck: &str,
        ws: u64,
        prev_ws: u64,
        window_seconds: u64,
        limit: u64,
        now_secs: u64,
    ) -> (bool, u64) {
        let elapsed = now_secs - ws;
        let prev_weight = 1.0 - (elapsed as f64 / window_seconds as f64);

        let prev_count = self.get_count(prev_ck, prev_ws, now_secs);
        let curr_count = self.get_count(curr_ck, ws, now_secs);

        let estimated_with = (prev_count as f64 * prev_weight + (curr_count + 1) as f64) as u64;

        if estimated_with <= limit {
            let expiry = ws + window_seconds * 2;
            let entry = self
                .counters
                .entry(curr_ck.to_string())
                .or_insert((0, ws, expiry));
            if entry.1 != ws || entry.2 <= now_secs {
                *entry = (0, ws, expiry);
            }
            entry.0 += 1;
            (true, estimated_with)
        } else {
            let estimated = (prev_count as f64 * prev_weight + curr_count as f64) as u64;
            (false, estimated)
        }
    }

    fn get_count_sliding(
        &self,
        curr_ck: &str,
        prev_ck: &str,
        ws: u64,
        prev_ws: u64,
        window_seconds: u64,
        now_secs: u64,
    ) -> u64 {
        let elapsed = now_secs - ws;
        let prev_weight = 1.0 - (elapsed as f64 / window_seconds as f64);
        let prev_count = self.get_count(prev_ck, prev_ws, now_secs);
        let curr_count = self.get_count(curr_ck, ws, now_secs);
        (prev_count as f64 * prev_weight + curr_count as f64) as u64
    }
}

/// Thread-safe, in-process rate limit store (no persistence; best for tests).
pub struct MemoryRateLimitStore {
    inner: Mutex<MemoryInner>,
}

impl MemoryRateLimitStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MemoryInner::new()),
        }
    }
}

impl Default for MemoryRateLimitStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RateLimitStore for MemoryRateLimitStore {
    async fn check_and_increment(
        &self,
        key_id: &str,
        bucket: &RateLimitBucket,
        now_secs: u64,
    ) -> Result<BucketStatus> {
        let mut inner = self.inner.lock().map_err(|_| anyhow!("lock poisoned"))?;
        let ws = window_start_for(now_secs, bucket.window_seconds);
        let reset_at = ws + bucket.window_seconds;

        let (allowed, used) = match bucket.window_type {
            WindowType::Fixed => {
                let ck = counter_key(key_id, &bucket.name, ws);
                inner.increment_fixed(&ck, ws, bucket.window_seconds, bucket.requests, now_secs)
            }
            WindowType::Sliding => {
                let prev_ws = ws.saturating_sub(bucket.window_seconds);
                let curr_ck = counter_key(key_id, &bucket.name, ws);
                let prev_ck = counter_key(key_id, &bucket.name, prev_ws);
                inner.increment_sliding(
                    &curr_ck,
                    &prev_ck,
                    ws,
                    prev_ws,
                    bucket.window_seconds,
                    bucket.requests,
                    now_secs,
                )
            }
        };

        let remaining = used.min(bucket.requests).pipe(|u| bucket.requests - u);
        Ok(BucketStatus {
            name: bucket.name.clone(),
            limit: bucket.requests,
            remaining,
            used,
            allowed,
            reset_at,
            window_seconds: bucket.window_seconds,
        })
    }

    async fn get_bucket_status(
        &self,
        key_id: &str,
        bucket: &RateLimitBucket,
        now_secs: u64,
    ) -> Result<BucketStatus> {
        let inner = self.inner.lock().map_err(|_| anyhow!("lock poisoned"))?;
        let ws = window_start_for(now_secs, bucket.window_seconds);
        let reset_at = ws + bucket.window_seconds;

        let used = match bucket.window_type {
            WindowType::Fixed => {
                let ck = counter_key(key_id, &bucket.name, ws);
                inner.get_count(&ck, ws, now_secs)
            }
            WindowType::Sliding => {
                let prev_ws = ws.saturating_sub(bucket.window_seconds);
                let curr_ck = counter_key(key_id, &bucket.name, ws);
                let prev_ck = counter_key(key_id, &bucket.name, prev_ws);
                inner.get_count_sliding(
                    &curr_ck,
                    &prev_ck,
                    ws,
                    prev_ws,
                    bucket.window_seconds,
                    now_secs,
                )
            }
        };

        let remaining = bucket.requests.saturating_sub(used);
        Ok(BucketStatus {
            name: bucket.name.clone(),
            limit: bucket.requests,
            remaining,
            used,
            allowed: used < bucket.requests,
            reset_at,
            window_seconds: bucket.window_seconds,
        })
    }

    async fn save_policy(&self, policy: &RateLimitPolicy) -> Result<()> {
        self.inner
            .lock()
            .map_err(|_| anyhow!("lock poisoned"))?
            .policies
            .insert(policy.id.clone(), policy.clone());
        Ok(())
    }

    async fn get_policy(&self, id: &str) -> Result<Option<RateLimitPolicy>> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| anyhow!("lock poisoned"))?
            .policies
            .get(id)
            .cloned())
    }

    async fn delete_policy(&self, id: &str) -> Result<bool> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| anyhow!("lock poisoned"))?
            .policies
            .remove(id)
            .is_some())
    }

    async fn list_policies(&self) -> Result<Vec<RateLimitPolicy>> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| anyhow!("lock poisoned"))?
            .policies
            .values()
            .cloned()
            .collect())
    }

    async fn set_key_policy(&self, key_id: &str, policy_id: &str) -> Result<()> {
        self.inner
            .lock()
            .map_err(|_| anyhow!("lock poisoned"))?
            .key_policies
            .insert(key_id.to_string(), policy_id.to_string());
        Ok(())
    }

    async fn get_key_policy(&self, key_id: &str) -> Result<Option<String>> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| anyhow!("lock poisoned"))?
            .key_policies
            .get(key_id)
            .cloned())
    }

    async fn remove_key_policy(&self, key_id: &str) -> Result<bool> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| anyhow!("lock poisoned"))?
            .key_policies
            .remove(key_id)
            .is_some())
    }

    async fn set_default_policy_id(&self, policy_id: &str) -> Result<()> {
        self.inner
            .lock()
            .map_err(|_| anyhow!("lock poisoned"))?
            .default_policy = Some(policy_id.to_string());
        Ok(())
    }

    async fn get_default_policy_id(&self) -> Result<Option<String>> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| anyhow!("lock poisoned"))?
            .default_policy
            .clone())
    }

    async fn block_key(&self, key_id: &str, until_secs: u64, reason: &str) -> Result<()> {
        self.inner
            .lock()
            .map_err(|_| anyhow!("lock poisoned"))?
            .blocks
            .insert(
                key_id.to_string(),
                BlockInfo {
                    key_id: key_id.to_string(),
                    until_secs,
                    reason: reason.to_string(),
                },
            );
        Ok(())
    }

    async fn unblock_key(&self, key_id: &str) -> Result<()> {
        self.inner
            .lock()
            .map_err(|_| anyhow!("lock poisoned"))?
            .blocks
            .remove(key_id);
        Ok(())
    }

    async fn get_block(&self, key_id: &str) -> Result<Option<BlockInfo>> {
        let mut inner = self.inner.lock().map_err(|_| anyhow!("lock poisoned"))?;
        let now = now_secs();
        if let Some(block) = inner.blocks.get(key_id) {
            if block.until_secs == 0 || block.until_secs > now {
                return Ok(Some(block.clone()));
            }
            inner.blocks.remove(key_id); // auto-expire
        }
        Ok(None)
    }

    async fn list_blocks(&self) -> Result<Vec<BlockInfo>> {
        let mut inner = self.inner.lock().map_err(|_| anyhow!("lock poisoned"))?;
        let now = now_secs();
        inner
            .blocks
            .retain(|_, b| b.until_secs == 0 || b.until_secs > now);
        Ok(inner.blocks.values().cloned().collect())
    }
}

// ============================================================================
// Sled store (persistent, embedded, single-node)
// ============================================================================

/// Counter value stored in sled: (count, window_start, expiry_secs).
type SledCounter = (u64, u64, u64);

/// Persistent embedded rate limit store backed by sled.
pub struct SledRateLimitStore {
    counters: sled::Tree,
    policies: sled::Tree,
    key_policies: sled::Tree,
    meta: sled::Tree,
    blocks: sled::Tree,
    _db: sled::Db,
}

impl SledRateLimitStore {
    pub fn open_default() -> Result<Self> {
        let path = std::env::var("ROUTIIUM_RL_SLED_PATH")
            .unwrap_or_else(|_| "./data/rate_limit.db".to_string());
        Self::open_path(&path)
    }

    pub fn open_path(path: &str) -> Result<Self> {
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let db = sled::open(path)?;
        Ok(Self {
            counters: db.open_tree("rl_counters")?,
            policies: db.open_tree("rl_policies")?,
            key_policies: db.open_tree("rl_key_policies")?,
            meta: db.open_tree("rl_meta")?,
            blocks: db.open_tree("rl_blocks")?,
            _db: db,
        })
    }
}

#[async_trait]
impl RateLimitStore for SledRateLimitStore {
    async fn check_and_increment(
        &self,
        key_id: &str,
        bucket: &RateLimitBucket,
        now_secs: u64,
    ) -> Result<BucketStatus> {
        let ws = window_start_for(now_secs, bucket.window_seconds);
        let reset_at = ws + bucket.window_seconds;
        let limit = bucket.requests;
        let window_seconds = bucket.window_seconds;
        let window_type = bucket.window_type;
        let bucket_name = bucket.name.clone();
        let key_id = key_id.to_string();

        let counters = self.counters.clone();

        let (allowed, used) = tokio::task::spawn_blocking(move || -> Result<(bool, u64)> {
            match window_type {
                WindowType::Fixed => {
                    let ck = counter_key(&key_id, &bucket_name, ws);
                    let raw_key = ck.as_bytes().to_vec();
                    loop {
                        let current_iv = counters.get(&raw_key)?;
                        let (count, cur_ws, expiry): SledCounter = match &current_iv {
                            Some(iv) => serde_json::from_slice(iv)?,
                            None => (0, ws, ws + window_seconds),
                        };
                        let (count, cur_ws, expiry) = if expiry <= now_secs || cur_ws != ws {
                            (0u64, ws, ws + window_seconds)
                        } else {
                            (count, cur_ws, expiry)
                        };
                        let new_count = count + 1;
                        if new_count > limit {
                            return Ok((false, count));
                        }
                        let new_val =
                            serde_json::to_vec::<SledCounter>(&(new_count, cur_ws, expiry))?;
                        let old_val = current_iv.as_ref().map(|iv| iv.to_vec());
                        match counters.compare_and_swap(&raw_key, old_val, Some(new_val))? {
                            Ok(()) => return Ok((true, new_count)),
                            Err(_) => continue,
                        }
                    }
                }
                WindowType::Sliding => {
                    let prev_ws = ws.saturating_sub(window_seconds);
                    let prev_ck = counter_key(&key_id, &bucket_name, prev_ws);
                    let curr_ck = counter_key(&key_id, &bucket_name, ws);

                    // Read previous count
                    let prev_count: u64 = if let Some(iv) = counters.get(prev_ck.as_bytes())? {
                        let (count, pws, exp): SledCounter = serde_json::from_slice(&iv)?;
                        if exp > now_secs && pws == prev_ws {
                            count
                        } else {
                            0
                        }
                    } else {
                        0
                    };

                    let elapsed = now_secs - ws;
                    let prev_weight = 1.0 - (elapsed as f64 / window_seconds as f64);
                    let raw_curr = curr_ck.as_bytes().to_vec();

                    loop {
                        let current_iv = counters.get(&raw_curr)?;
                        let (curr_count, cur_ws, expiry): SledCounter = match &current_iv {
                            Some(iv) => serde_json::from_slice(iv)?,
                            None => (0, ws, ws + window_seconds * 2),
                        };
                        let (curr_count, cur_ws, expiry) = if expiry <= now_secs || cur_ws != ws {
                            (0u64, ws, ws + window_seconds * 2)
                        } else {
                            (curr_count, cur_ws, expiry)
                        };
                        let estimated =
                            (prev_count as f64 * prev_weight + (curr_count + 1) as f64) as u64;
                        if estimated > limit {
                            let curr_est =
                                (prev_count as f64 * prev_weight + curr_count as f64) as u64;
                            return Ok((false, curr_est));
                        }
                        let new_count = curr_count + 1;
                        let new_val =
                            serde_json::to_vec::<SledCounter>(&(new_count, cur_ws, expiry))?;
                        let old_val = current_iv.as_ref().map(|iv| iv.to_vec());
                        match counters.compare_and_swap(&raw_curr, old_val, Some(new_val))? {
                            Ok(()) => return Ok((true, estimated)),
                            Err(_) => continue,
                        }
                    }
                }
            }
        })
        .await??;

        let remaining = limit.saturating_sub(used);
        Ok(BucketStatus {
            name: bucket.name.clone(),
            limit,
            remaining,
            used,
            allowed,
            reset_at,
            window_seconds,
        })
    }

    async fn get_bucket_status(
        &self,
        key_id: &str,
        bucket: &RateLimitBucket,
        now_secs: u64,
    ) -> Result<BucketStatus> {
        let ws = window_start_for(now_secs, bucket.window_seconds);
        let reset_at = ws + bucket.window_seconds;
        let limit = bucket.requests;
        let window_seconds = bucket.window_seconds;
        let window_type = bucket.window_type;
        let bucket_name = bucket.name.clone();
        let key_id = key_id.to_string();
        let counters = self.counters.clone();

        let used = tokio::task::spawn_blocking(move || -> Result<u64> {
            match window_type {
                WindowType::Fixed => {
                    let ck = counter_key(&key_id, &bucket_name, ws);
                    if let Some(iv) = counters.get(ck.as_bytes())? {
                        let (count, cws, exp): SledCounter = serde_json::from_slice(&iv)?;
                        if exp > now_secs && cws == ws {
                            return Ok(count);
                        }
                    }
                    Ok(0)
                }
                WindowType::Sliding => {
                    let prev_ws = ws.saturating_sub(window_seconds);
                    let curr_ck = counter_key(&key_id, &bucket_name, ws);
                    let prev_ck = counter_key(&key_id, &bucket_name, prev_ws);
                    let curr_count: u64 = if let Some(iv) = counters.get(curr_ck.as_bytes())? {
                        let (c, cws, exp): SledCounter = serde_json::from_slice(&iv)?;
                        if exp > now_secs && cws == ws {
                            c
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    let prev_count: u64 = if let Some(iv) = counters.get(prev_ck.as_bytes())? {
                        let (c, pws, exp): SledCounter = serde_json::from_slice(&iv)?;
                        if exp > now_secs && pws == prev_ws {
                            c
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    let elapsed = now_secs - ws;
                    let prev_weight = 1.0 - (elapsed as f64 / window_seconds as f64);
                    Ok((prev_count as f64 * prev_weight + curr_count as f64) as u64)
                }
            }
        })
        .await??;

        let remaining = limit.saturating_sub(used);
        Ok(BucketStatus {
            name: bucket.name.clone(),
            limit,
            remaining,
            used,
            allowed: used < limit,
            reset_at,
            window_seconds,
        })
    }

    async fn save_policy(&self, policy: &RateLimitPolicy) -> Result<()> {
        let key = format!("rl:p:{}", policy.id).into_bytes();
        let val = serde_json::to_vec(policy)?;
        let tree = self.policies.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            tree.insert(key, val)?;
            tree.flush()?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn get_policy(&self, id: &str) -> Result<Option<RateLimitPolicy>> {
        let key = format!("rl:p:{}", id).into_bytes();
        let tree = self.policies.clone();
        tokio::task::spawn_blocking(move || -> Result<Option<RateLimitPolicy>> {
            Ok(tree
                .get(key)?
                .map(|iv| serde_json::from_slice(&iv))
                .transpose()?)
        })
        .await?
    }

    async fn delete_policy(&self, id: &str) -> Result<bool> {
        let key = format!("rl:p:{}", id).into_bytes();
        let tree = self.policies.clone();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let existed = tree.remove(key)?.is_some();
            tree.flush()?;
            Ok(existed)
        })
        .await?
    }

    async fn list_policies(&self) -> Result<Vec<RateLimitPolicy>> {
        let tree = self.policies.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<RateLimitPolicy>> {
            let mut out = Vec::new();
            for item in tree.iter() {
                let (_, v) = item?;
                if let Ok(p) = serde_json::from_slice::<RateLimitPolicy>(&v) {
                    out.push(p);
                }
            }
            Ok(out)
        })
        .await?
    }

    async fn set_key_policy(&self, key_id: &str, policy_id: &str) -> Result<()> {
        let key = format!("rl:k:{}", key_id).into_bytes();
        let val = policy_id.as_bytes().to_vec();
        let tree = self.key_policies.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            tree.insert(key, val)?;
            tree.flush()?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn get_key_policy(&self, key_id: &str) -> Result<Option<String>> {
        let key = format!("rl:k:{}", key_id).into_bytes();
        let tree = self.key_policies.clone();
        tokio::task::spawn_blocking(move || -> Result<Option<String>> {
            Ok(tree
                .get(key)?
                .map(|iv| String::from_utf8_lossy(&iv).to_string()))
        })
        .await?
    }

    async fn remove_key_policy(&self, key_id: &str) -> Result<bool> {
        let key = format!("rl:k:{}", key_id).into_bytes();
        let tree = self.key_policies.clone();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let existed = tree.remove(key)?.is_some();
            tree.flush()?;
            Ok(existed)
        })
        .await?
    }

    async fn set_default_policy_id(&self, policy_id: &str) -> Result<()> {
        let val = policy_id.as_bytes().to_vec();
        let tree = self.meta.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            tree.insert(b"rl:default", val)?;
            tree.flush()?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn get_default_policy_id(&self) -> Result<Option<String>> {
        let tree = self.meta.clone();
        tokio::task::spawn_blocking(move || -> Result<Option<String>> {
            Ok(tree
                .get(b"rl:default")?
                .map(|iv| String::from_utf8_lossy(&iv).to_string()))
        })
        .await?
    }

    async fn block_key(&self, key_id: &str, until_secs: u64, reason: &str) -> Result<()> {
        let block = BlockInfo {
            key_id: key_id.to_string(),
            until_secs,
            reason: reason.to_string(),
        };
        let key = format!("rl:block:{}", key_id).into_bytes();
        let val = serde_json::to_vec(&block)?;
        let tree = self.blocks.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            tree.insert(key, val)?;
            tree.flush()?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn unblock_key(&self, key_id: &str) -> Result<()> {
        let key = format!("rl:block:{}", key_id).into_bytes();
        let tree = self.blocks.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            tree.remove(key)?;
            tree.flush()?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn get_block(&self, key_id: &str) -> Result<Option<BlockInfo>> {
        let key_str = format!("rl:block:{}", key_id);
        let key_bytes = key_str.as_bytes().to_vec();
        let tree = self.blocks.clone();
        let del_tree = self.blocks.clone();
        let now = now_secs();
        tokio::task::spawn_blocking(move || -> Result<Option<BlockInfo>> {
            if let Some(iv) = tree.get(&key_bytes)? {
                let block: BlockInfo = serde_json::from_slice(&iv)?;
                if block.until_secs == 0 || block.until_secs > now {
                    return Ok(Some(block));
                }
                del_tree.remove(&key_bytes)?;
            }
            Ok(None)
        })
        .await?
    }

    async fn list_blocks(&self) -> Result<Vec<BlockInfo>> {
        let tree = self.blocks.clone();
        let now = now_secs();
        tokio::task::spawn_blocking(move || -> Result<Vec<BlockInfo>> {
            let mut out = Vec::new();
            let mut to_remove: Vec<sled::IVec> = Vec::new();
            for item in tree.iter() {
                let (k, v) = item?;
                if let Ok(block) = serde_json::from_slice::<BlockInfo>(&v) {
                    if block.until_secs == 0 || block.until_secs > now {
                        out.push(block);
                    } else {
                        to_remove.push(k);
                    }
                }
            }
            for k in to_remove {
                tree.remove(k)?;
            }
            Ok(out)
        })
        .await?
    }
}

// ============================================================================
// Redis store
// ============================================================================

struct RlRedisManager {
    client: redis::Client,
}

impl r2d2::ManageConnection for RlRedisManager {
    type Connection = redis::Connection;
    type Error = redis::RedisError;

    fn connect(&self) -> std::result::Result<Self::Connection, Self::Error> {
        self.client.get_connection()
    }

    fn is_valid(&self, conn: &mut Self::Connection) -> std::result::Result<(), Self::Error> {
        let _: String = redis::cmd("PING").query(conn)?;
        Ok(())
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        false
    }
}

/// Distributed rate limit store backed by Redis.
pub struct RedisRateLimitStore {
    pool: r2d2::Pool<RlRedisManager>,
    ns: String,
}

impl RedisRateLimitStore {
    pub fn connect(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)?;
        let max_size = std::env::var("ROUTIIUM_REDIS_POOL_MAX")
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(16);
        let pool = r2d2::Pool::builder()
            .max_size(max_size)
            .build(RlRedisManager { client })?;
        Ok(Self {
            pool,
            ns: "routiium".to_string(),
        })
    }

    pub fn connect_default() -> Result<Self> {
        let url = std::env::var("ROUTIIUM_REDIS_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
        Self::connect(&url)
    }

    fn ns(&self, rest: &str) -> String {
        format!("{}:{}", self.ns, rest)
    }
}

/// Lua script for atomic fixed-window increment.
/// KEYS[1] = counter key
/// ARGV[1] = window TTL seconds, ARGV[2] = limit
/// Returns [allowed (1/0), current_count]
const FIXED_WINDOW_LUA: &str = r#"
local current = tonumber(redis.call('GET', KEYS[1])) or 0
local limit = tonumber(ARGV[2])
if current >= limit then
    return {0, current}
end
local new = redis.call('INCR', KEYS[1])
if new == 1 then
    redis.call('EXPIRE', KEYS[1], tonumber(ARGV[1]))
end
if new > limit then
    redis.call('DECR', KEYS[1])
    return {0, new - 1}
end
return {1, new}
"#;

/// Lua script for sliding window (two-bucket approximation).
/// KEYS[1] = current window key, KEYS[2] = previous window key
/// ARGV[1] = TTL (2×window_seconds), ARGV[2] = limit, ARGV[3] = prev_weight×1000
/// Returns [allowed (1/0), estimated_count]
const SLIDING_WINDOW_LUA: &str = r#"
local prev_count = tonumber(redis.call('GET', KEYS[2])) or 0
local curr_count = tonumber(redis.call('GET', KEYS[1])) or 0
local prev_weight = tonumber(ARGV[3]) / 1000
local limit = tonumber(ARGV[2])
local estimated_with = math.floor(prev_count * prev_weight + (curr_count + 1))
if estimated_with > limit then
    local estimated = math.floor(prev_count * prev_weight + curr_count)
    return {0, estimated}
end
local new_curr = redis.call('INCR', KEYS[1])
if new_curr == 1 then
    redis.call('EXPIRE', KEYS[1], tonumber(ARGV[1]))
end
return {1, estimated_with}
"#;

#[async_trait]
impl RateLimitStore for RedisRateLimitStore {
    async fn check_and_increment(
        &self,
        key_id: &str,
        bucket: &RateLimitBucket,
        now_secs: u64,
    ) -> Result<BucketStatus> {
        let ws = window_start_for(now_secs, bucket.window_seconds);
        let reset_at = ws + bucket.window_seconds;
        let limit = bucket.requests;
        let window_seconds = bucket.window_seconds;
        let window_type = bucket.window_type;
        let pool = self.pool.clone();
        let ns = self.ns.clone();
        let key_id = key_id.to_string();
        let bucket_name = bucket.name.clone();

        let (allowed, used) = tokio::task::spawn_blocking(move || -> Result<(bool, u64)> {
            let mut conn = pool.get()?;
            let curr_key = format!("{}:rl:c:{}:{}:{}", ns, key_id, bucket_name, ws);
            match window_type {
                WindowType::Fixed => {
                    let result: Vec<i64> = redis::Script::new(FIXED_WINDOW_LUA)
                        .key(&curr_key)
                        .arg(window_seconds)
                        .arg(limit)
                        .invoke(&mut *conn)?;
                    let allowed = result.first().copied().unwrap_or(0) != 0;
                    let count = result.get(1).copied().unwrap_or(0).max(0) as u64;
                    Ok((allowed, count))
                }
                WindowType::Sliding => {
                    let prev_ws = ws.saturating_sub(window_seconds);
                    let prev_key = format!("{}:rl:c:{}:{}:{}", ns, key_id, bucket_name, prev_ws);
                    let elapsed = now_secs - ws;
                    let prev_weight = 1.0 - (elapsed as f64 / window_seconds as f64);
                    let prev_weight_int = (prev_weight * 1000.0) as i64;
                    let result: Vec<i64> = redis::Script::new(SLIDING_WINDOW_LUA)
                        .key(&curr_key)
                        .key(&prev_key)
                        .arg(window_seconds * 2)
                        .arg(limit)
                        .arg(prev_weight_int)
                        .invoke(&mut *conn)?;
                    let allowed = result.first().copied().unwrap_or(0) != 0;
                    let count = result.get(1).copied().unwrap_or(0).max(0) as u64;
                    Ok((allowed, count))
                }
            }
        })
        .await??;

        let remaining = limit.saturating_sub(used);
        Ok(BucketStatus {
            name: bucket.name.clone(),
            limit,
            remaining,
            used,
            allowed,
            reset_at,
            window_seconds,
        })
    }

    async fn get_bucket_status(
        &self,
        key_id: &str,
        bucket: &RateLimitBucket,
        now_secs: u64,
    ) -> Result<BucketStatus> {
        let ws = window_start_for(now_secs, bucket.window_seconds);
        let reset_at = ws + bucket.window_seconds;
        let limit = bucket.requests;
        let window_seconds = bucket.window_seconds;
        let window_type = bucket.window_type;
        let pool = self.pool.clone();
        let ns = self.ns.clone();
        let key_id = key_id.to_string();
        let bucket_name = bucket.name.clone();

        let used = tokio::task::spawn_blocking(move || -> Result<u64> {
            let mut conn = pool.get()?;
            let curr_key = format!("{}:rl:c:{}:{}:{}", ns, key_id, bucket_name, ws);
            match window_type {
                WindowType::Fixed => {
                    let count: Option<u64> = redis::cmd("GET").arg(&curr_key).query(&mut *conn)?;
                    Ok(count.unwrap_or(0))
                }
                WindowType::Sliding => {
                    let prev_ws = ws.saturating_sub(window_seconds);
                    let prev_key = format!("{}:rl:c:{}:{}:{}", ns, key_id, bucket_name, prev_ws);
                    let elapsed = now_secs - ws;
                    let prev_weight = 1.0 - (elapsed as f64 / window_seconds as f64);
                    let curr: u64 = redis::cmd("GET")
                        .arg(&curr_key)
                        .query::<Option<u64>>(&mut *conn)?
                        .unwrap_or(0);
                    let prev: u64 = redis::cmd("GET")
                        .arg(&prev_key)
                        .query::<Option<u64>>(&mut *conn)?
                        .unwrap_or(0);
                    Ok((prev as f64 * prev_weight + curr as f64) as u64)
                }
            }
        })
        .await??;

        let remaining = limit.saturating_sub(used);
        Ok(BucketStatus {
            name: bucket.name.clone(),
            limit,
            remaining,
            used,
            allowed: used < limit,
            reset_at,
            window_seconds,
        })
    }

    async fn save_policy(&self, policy: &RateLimitPolicy) -> Result<()> {
        let key = self.ns(&format!("rl:p:{}", policy.id));
        let val = serde_json::to_string(policy)?;
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut conn = pool.get()?;
            let _: () = redis::cmd("SET").arg(&key).arg(&val).query(&mut *conn)?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn get_policy(&self, id: &str) -> Result<Option<RateLimitPolicy>> {
        let key = self.ns(&format!("rl:p:{}", id));
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<Option<RateLimitPolicy>> {
            let mut conn = pool.get()?;
            let val: Option<String> = redis::cmd("GET").arg(&key).query(&mut *conn)?;
            val.map(|s| serde_json::from_str(&s).map_err(Into::into))
                .transpose()
        })
        .await?
    }

    async fn delete_policy(&self, id: &str) -> Result<bool> {
        let key = self.ns(&format!("rl:p:{}", id));
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let mut conn = pool.get()?;
            let n: i64 = redis::cmd("DEL").arg(&key).query(&mut *conn)?;
            Ok(n > 0)
        })
        .await?
    }

    async fn list_policies(&self) -> Result<Vec<RateLimitPolicy>> {
        let pattern = self.ns("rl:p:*");
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<RateLimitPolicy>> {
            let mut conn = pool.get()?;
            let keys: Vec<String> = redis::cmd("KEYS").arg(&pattern).query(&mut *conn)?;
            let mut out = Vec::new();
            for k in keys {
                if let Ok(Some(s)) = redis::cmd("GET")
                    .arg(&k)
                    .query::<Option<String>>(&mut *conn)
                {
                    if let Ok(p) = serde_json::from_str::<RateLimitPolicy>(&s) {
                        out.push(p);
                    }
                }
            }
            Ok(out)
        })
        .await?
    }

    async fn set_key_policy(&self, key_id: &str, policy_id: &str) -> Result<()> {
        let key = self.ns(&format!("rl:k:{}", key_id));
        let val = policy_id.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut conn = pool.get()?;
            let _: () = redis::cmd("SET").arg(&key).arg(&val).query(&mut *conn)?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn get_key_policy(&self, key_id: &str) -> Result<Option<String>> {
        let key = self.ns(&format!("rl:k:{}", key_id));
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<Option<String>> {
            let mut conn = pool.get()?;
            Ok(redis::cmd("GET").arg(&key).query(&mut *conn)?)
        })
        .await?
    }

    async fn remove_key_policy(&self, key_id: &str) -> Result<bool> {
        let key = self.ns(&format!("rl:k:{}", key_id));
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let mut conn = pool.get()?;
            let n: i64 = redis::cmd("DEL").arg(&key).query(&mut *conn)?;
            Ok(n > 0)
        })
        .await?
    }

    async fn set_default_policy_id(&self, policy_id: &str) -> Result<()> {
        let key = self.ns("rl:default");
        let val = policy_id.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut conn = pool.get()?;
            let _: () = redis::cmd("SET").arg(&key).arg(&val).query(&mut *conn)?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn get_default_policy_id(&self) -> Result<Option<String>> {
        let key = self.ns("rl:default");
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<Option<String>> {
            let mut conn = pool.get()?;
            Ok(redis::cmd("GET").arg(&key).query(&mut *conn)?)
        })
        .await?
    }

    async fn block_key(&self, key_id: &str, until_secs: u64, reason: &str) -> Result<()> {
        let block = BlockInfo {
            key_id: key_id.to_string(),
            until_secs,
            reason: reason.to_string(),
        };
        let key = self.ns(&format!("rl:block:{}", key_id));
        let val = serde_json::to_string(&block)?;
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut conn = pool.get()?;
            let _: () = redis::cmd("SET").arg(&key).arg(&val).query(&mut *conn)?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn unblock_key(&self, key_id: &str) -> Result<()> {
        let key = self.ns(&format!("rl:block:{}", key_id));
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut conn = pool.get()?;
            let _: () = redis::cmd("DEL").arg(&key).query(&mut *conn)?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn get_block(&self, key_id: &str) -> Result<Option<BlockInfo>> {
        let key = self.ns(&format!("rl:block:{}", key_id));
        let pool = self.pool.clone();
        let now = now_secs();
        tokio::task::spawn_blocking(move || -> Result<Option<BlockInfo>> {
            let mut conn = pool.get()?;
            let val: Option<String> = redis::cmd("GET").arg(&key).query(&mut *conn)?;
            match val {
                Some(s) => {
                    let block: BlockInfo = serde_json::from_str(&s)?;
                    if block.until_secs == 0 || block.until_secs > now {
                        Ok(Some(block))
                    } else {
                        let _: () = redis::cmd("DEL").arg(&key).query(&mut *conn)?;
                        Ok(None)
                    }
                }
                None => Ok(None),
            }
        })
        .await?
    }

    async fn list_blocks(&self) -> Result<Vec<BlockInfo>> {
        let pattern = self.ns("rl:block:*");
        let pool = self.pool.clone();
        let now = now_secs();
        tokio::task::spawn_blocking(move || -> Result<Vec<BlockInfo>> {
            let mut conn = pool.get()?;
            let keys: Vec<String> = redis::cmd("KEYS").arg(&pattern).query(&mut *conn)?;
            let mut out = Vec::new();
            for k in keys {
                if let Ok(Some(s)) = redis::cmd("GET")
                    .arg(&k)
                    .query::<Option<String>>(&mut *conn)
                {
                    if let Ok(block) = serde_json::from_str::<BlockInfo>(&s) {
                        if block.until_secs == 0 || block.until_secs > now {
                            out.push(block);
                        }
                    }
                }
            }
            Ok(out)
        })
        .await?
    }
}

// ============================================================================
// Concurrency Manager
// ============================================================================

struct ConcurrencyEntry {
    semaphore: Arc<Semaphore>,
    config: ConcurrencyConfig,
    active_count: Arc<AtomicI32>,
    queued_count: Arc<AtomicI32>,
}

/// Manages per-key concurrency limits using in-process semaphores.
pub struct ConcurrencyManager {
    entries: RwLock<HashMap<String, Arc<ConcurrencyEntry>>>,
}

/// Result of attempting to acquire a concurrency slot.
pub enum ConcurrencyResult {
    /// Slot acquired; hold the permit until the request completes.
    Allowed(OwnedSemaphorePermit),
    /// Request was rejected due to capacity or queue overflow.
    Rejected {
        active: u32,
        limit: u32,
        queue_full: bool,
    },
}

impl ConcurrencyManager {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    fn get_or_create(&self, key_id: &str, config: &ConcurrencyConfig) -> Arc<ConcurrencyEntry> {
        {
            let r = self.entries.read().unwrap();
            if let Some(e) = r.get(key_id) {
                return e.clone();
            }
        }
        let mut w = self.entries.write().unwrap();
        if let Some(e) = w.get(key_id) {
            return e.clone();
        }
        let entry = Arc::new(ConcurrencyEntry {
            semaphore: Arc::new(Semaphore::new(config.max_concurrent as usize)),
            config: config.clone(),
            active_count: Arc::new(AtomicI32::new(0)),
            queued_count: Arc::new(AtomicI32::new(0)),
        });
        w.insert(key_id.to_string(), entry.clone());
        entry
    }

    /// Try to acquire a concurrency slot for `key_id`.
    pub async fn acquire(
        &self,
        key_id: &str,
        config: &ConcurrencyConfig,
    ) -> Result<ConcurrencyResult> {
        let entry = self.get_or_create(key_id, config);
        let active = entry.active_count.load(Ordering::Relaxed).max(0) as u32;

        match config.strategy {
            QueueStrategy::Reject | QueueStrategy::QueuePriority => {
                match entry.semaphore.clone().try_acquire_owned() {
                    Ok(permit) => {
                        entry.active_count.fetch_add(1, Ordering::Relaxed);
                        Ok(ConcurrencyResult::Allowed(permit))
                    }
                    Err(_) => Ok(ConcurrencyResult::Rejected {
                        active,
                        limit: config.max_concurrent,
                        queue_full: false,
                    }),
                }
            }
            QueueStrategy::QueueFifo => {
                // Fast path: try immediate acquisition.
                if let Ok(permit) = entry.semaphore.clone().try_acquire_owned() {
                    entry.active_count.fetch_add(1, Ordering::Relaxed);
                    return Ok(ConcurrencyResult::Allowed(permit));
                }

                // Check queue capacity.
                let queued = entry.queued_count.load(Ordering::Relaxed).max(0) as u32;
                if config.max_queue_size > 0 && queued >= config.max_queue_size {
                    return Ok(ConcurrencyResult::Rejected {
                        active,
                        limit: config.max_concurrent,
                        queue_full: true,
                    });
                }

                entry.queued_count.fetch_add(1, Ordering::Relaxed);
                let entry_clone = entry.clone();
                let timeout = Duration::from_millis(config.queue_timeout_ms);

                let result =
                    tokio::time::timeout(timeout, entry.semaphore.clone().acquire_owned()).await;

                entry_clone.queued_count.fetch_sub(1, Ordering::Relaxed);

                match result {
                    Ok(Ok(permit)) => {
                        entry_clone.active_count.fetch_add(1, Ordering::Relaxed);
                        Ok(ConcurrencyResult::Allowed(permit))
                    }
                    Ok(Err(_)) => Err(anyhow!("concurrency semaphore closed")),
                    Err(_) => Ok(ConcurrencyResult::Rejected {
                        active,
                        limit: config.max_concurrent,
                        queue_full: false,
                    }),
                }
            }
        }
    }

    /// Decrement the active counter when a request completes (permit is dropped by caller).
    pub fn release(&self, key_id: &str) {
        if let Ok(r) = self.entries.read() {
            if let Some(entry) = r.get(key_id) {
                let prev = entry.active_count.fetch_sub(1, Ordering::Relaxed);
                if prev < 0 {
                    entry.active_count.store(0, Ordering::Relaxed);
                }
            }
        }
    }

    /// Returns `(active, queued, max_concurrent, max_queue_size)` or `None` if unknown.
    pub fn get_status(&self, key_id: &str) -> Option<(u32, u32, u32, u32)> {
        let r = self.entries.read().unwrap();
        r.get(key_id).map(|e| {
            let active = e.active_count.load(Ordering::Relaxed).max(0) as u32;
            let queued = e.queued_count.load(Ordering::Relaxed).max(0) as u32;
            (
                active,
                queued,
                e.config.max_concurrent,
                e.config.max_queue_size,
            )
        })
    }

    /// Reconfigure concurrency limits for a key (resets the semaphore).
    pub fn update_config(&self, key_id: &str, config: ConcurrencyConfig) {
        let mut w = self.entries.write().unwrap();
        w.insert(
            key_id.to_string(),
            Arc::new(ConcurrencyEntry {
                semaphore: Arc::new(Semaphore::new(config.max_concurrent as usize)),
                config,
                active_count: Arc::new(AtomicI32::new(0)),
                queued_count: Arc::new(AtomicI32::new(0)),
            }),
        );
    }
}

impl Default for ConcurrencyManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// In-memory metrics store
// ============================================================================

struct MetricsInner {
    metrics: HashMap<String, RateLimitMetrics>,
    events: Vec<RateLimitAnalyticsEvent>,
    max_events: usize,
}

/// Lightweight in-process metrics store (ring buffer of events + counters).
pub struct MetricsStore {
    inner: Mutex<MetricsInner>,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MetricsInner {
                metrics: HashMap::new(),
                events: Vec::new(),
                max_events: 10_000,
            }),
        }
    }

    pub fn record(&self, event: RateLimitAnalyticsEvent) {
        if let Ok(mut inner) = self.inner.lock() {
            let m = inner.metrics.entry(event.api_key_id.clone()).or_default();
            match event.event_type {
                RateLimitEventType::RateLimitChecked => m.total_checks += 1,
                RateLimitEventType::RateLimitAllowed => m.total_allowed += 1,
                RateLimitEventType::RateLimitRejected => {
                    m.total_rejected += 1;
                    if let Some(ref bucket) = event.bucket_name {
                        *m.rejections_by_bucket.entry(bucket.clone()).or_insert(0) += 1;
                    }
                }
                RateLimitEventType::ConcurrencyRejected => m.concurrency_rejected += 1,
                RateLimitEventType::ConcurrencyQueued => m.concurrency_queued += 1,
                RateLimitEventType::ConcurrencyQueueTimeout => m.concurrency_queue_timeouts += 1,
                _ => {}
            }
            inner.events.push(event);
            if inner.events.len() > inner.max_events {
                let drain = inner.events.len() - inner.max_events;
                inner.events.drain(0..drain);
            }
        }
    }

    pub fn get_metrics(&self, key_id: &str) -> Option<RateLimitMetrics> {
        self.inner.lock().ok()?.metrics.get(key_id).cloned()
    }

    pub fn get_all_metrics(&self) -> HashMap<String, RateLimitMetrics> {
        self.inner
            .lock()
            .map(|i| i.metrics.clone())
            .unwrap_or_default()
    }

    pub fn get_events(
        &self,
        key_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Vec<RateLimitAnalyticsEvent> {
        let inner = match self.inner.lock() {
            Ok(i) => i,
            Err(_) => return vec![],
        };
        match key_id {
            Some(kid) => inner
                .events
                .iter()
                .filter(|e| e.api_key_id == kid)
                .skip(offset)
                .take(limit)
                .cloned()
                .collect(),
            None => inner
                .events
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect(),
        }
    }

    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.metrics.clear();
            inner.events.clear();
        }
    }
}

impl Default for MetricsStore {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Rate Limit Manager
// ============================================================================

/// Central manager orchestrating rate limiting, concurrency, and analytics.
pub struct RateLimitManager {
    store: Arc<dyn RateLimitStore>,
    pub concurrency: Arc<ConcurrencyManager>,
    pub metrics: Arc<MetricsStore>,
    /// Hot-reloadable file config.
    file_config: Arc<RwLock<Option<RateLimitFileConfig>>>,
    /// Path to config file for reload support.
    pub config_path: Option<String>,
    /// Local policy cache (avoids store round-trips for hot paths).
    policy_cache: Arc<RwLock<HashMap<String, RateLimitPolicy>>>,
    /// In-memory emergency blocks (highest priority, survives store failures).
    emergency_blocks: Arc<RwLock<HashMap<String, BlockInfo>>>,
}

impl RateLimitManager {
    /// Create a manager with the given persistent store.
    pub fn new(store: Arc<dyn RateLimitStore>) -> Self {
        Self {
            store,
            concurrency: Arc::new(ConcurrencyManager::new()),
            metrics: Arc::new(MetricsStore::new()),
            file_config: Arc::new(RwLock::new(None)),
            config_path: None,
            policy_cache: Arc::new(RwLock::new(HashMap::new())),
            emergency_blocks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn with_config_path(mut self, path: Option<String>) -> Self {
        self.config_path = path;
        self
    }

    /// Initialise from environment variables; falls back through Redis → Sled → Memory.
    pub fn from_env() -> Result<Self> {
        let backend = std::env::var("ROUTIIUM_RATE_LIMIT_BACKEND").ok();
        let store: Arc<dyn RateLimitStore> = if let Some(ref b) = backend {
            let b = b.trim();
            if b.starts_with("redis://") || b.starts_with("rediss://") {
                Arc::new(RedisRateLimitStore::connect(b)?)
            } else if b == "memory" {
                Arc::new(MemoryRateLimitStore::new())
            } else {
                Arc::new(SledRateLimitStore::open_default()?)
            }
        } else if let Ok(redis_url) = std::env::var("ROUTIIUM_REDIS_URL") {
            match RedisRateLimitStore::connect(&redis_url) {
                Ok(s) => Arc::new(s),
                Err(_) => Arc::new(SledRateLimitStore::open_default()?),
            }
        } else {
            Arc::new(SledRateLimitStore::open_default()?)
        };
        Ok(Self::new(store))
    }

    // ---- File config -------------------------------------------------------

    /// Load a JSON config file and populate the store.
    pub async fn load_file_config(&self, path: &str) -> Result<()> {
        let config = RateLimitFileConfig::load_from_file(path)?;

        for (id, def) in &config.policies {
            let policy = RateLimitPolicy {
                id: id.clone(),
                buckets: def.buckets.clone(),
            };
            self.store.save_policy(&policy).await?;
            self.policy_cache
                .write()
                .unwrap()
                .insert(id.clone(), policy);
        }

        if let Some(ref default_id) = config.default_policy {
            self.store.set_default_policy_id(default_id).await?;
        }

        for (key_id, policy_id) in &config.key_overrides {
            self.store.set_key_policy(key_id, policy_id).await?;
        }

        *self.file_config.write().unwrap() = Some(config);
        info!("Rate limit config loaded from {}", path);
        Ok(())
    }

    /// Hot-reload the config file (if a path was provided).
    pub async fn reload_file_config(&self) -> Result<()> {
        if let Some(ref path) = self.config_path.clone() {
            // Invalidate cache first.
            self.policy_cache.write().unwrap().clear();
            self.load_file_config(path).await?;
        }
        Ok(())
    }

    // ---- Policy resolution -------------------------------------------------

    /// Resolve the effective policy for `key_id`, following priority:
    /// 1. Emergency block  → returns `Err("BLOCKED:<reason>")`
    /// 2. Per-key store mapping
    /// 3. File config key override
    /// 4. Default from store
    /// 5. Default from file config
    /// 6. `None` (unlimited)
    pub async fn resolve_policy(&self, key_id: &str) -> Result<Option<RateLimitPolicy>> {
        // 1. In-memory emergency blocks (fastest path)
        {
            let blocks = self.emergency_blocks.read().unwrap();
            if let Some(block) = blocks.get(key_id) {
                let n = now_secs();
                if block.until_secs == 0 || block.until_secs > n {
                    return Err(anyhow!("BLOCKED:{}", block.reason));
                }
            }
        }

        // 2. Persistent block check
        if self.store.get_block(key_id).await?.is_some() {
            return Err(anyhow!("BLOCKED"));
        }

        // 3. Per-key mapping in store
        if let Some(policy_id) = self.store.get_key_policy(key_id).await? {
            if let Some(p) = self.get_cached_policy(&policy_id).await? {
                return Ok(Some(p));
            }
        }

        // 4. File config key override
        let file_key_override = {
            let fc = self.file_config.read().unwrap();
            fc.as_ref()
                .and_then(|cfg| cfg.key_overrides.get(key_id).cloned())
        };
        if let Some(policy_id) = file_key_override {
            if let Some(p) = self.get_cached_policy(&policy_id).await? {
                return Ok(Some(p));
            }
        }

        // 5. Default from store
        if let Some(default_id) = self.store.get_default_policy_id().await? {
            if let Some(p) = self.get_cached_policy(&default_id).await? {
                return Ok(Some(p));
            }
        }

        // 6. Default from file config
        let file_default_id = {
            let fc = self.file_config.read().unwrap();
            fc.as_ref().and_then(|cfg| cfg.default_policy.clone())
        };
        if let Some(default_id) = file_default_id {
            if let Some(p) = self.get_cached_policy(&default_id).await? {
                return Ok(Some(p));
            }
        }

        Ok(None)
    }

    async fn get_cached_policy(&self, policy_id: &str) -> Result<Option<RateLimitPolicy>> {
        {
            let cache = self.policy_cache.read().unwrap();
            if let Some(p) = cache.get(policy_id) {
                return Ok(Some(p.clone()));
            }
        }
        let p = self.store.get_policy(policy_id).await?;
        if let Some(ref policy) = p {
            self.policy_cache
                .write()
                .unwrap()
                .insert(policy_id.to_string(), policy.clone());
        }
        Ok(p)
    }

    // ---- Rate limit check --------------------------------------------------

    /// Check rate limits for `key_id`. Increments counters if allowed.
    /// Returns `Err` if the key is blocked, `Ok(result)` otherwise.
    pub async fn check_rate_limit(
        &self,
        key_id: &str,
        endpoint: &str,
        model: Option<&str>,
    ) -> Result<RateLimitCheckResult> {
        let ts = now_secs();
        let policy = self.resolve_policy(key_id).await?;

        let Some(policy) = policy else {
            // Unlimited — record minimal event.
            self.record_event(RateLimitAnalyticsEvent {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: ts * 1000,
                api_key_id: key_id.to_string(),
                event_type: RateLimitEventType::RateLimitAllowed,
                policy_id: None,
                bucket_name: None,
                limit: None,
                remaining: None,
                window_seconds: None,
                active_requests: None,
                max_concurrent: None,
                queue_position: None,
                queue_size: None,
                wait_time_ms: None,
                endpoint: endpoint.to_string(),
                model: model.map(str::to_string),
                request_size_bytes: None,
                duration_ms: None,
            });
            return Ok(RateLimitCheckResult {
                allowed: true,
                policy_id: "unlimited".to_string(),
                buckets: vec![],
                rejected_bucket: None,
            });
        };

        let policy_id = policy.id.clone();

        self.record_event(RateLimitAnalyticsEvent {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: ts * 1000,
            api_key_id: key_id.to_string(),
            event_type: RateLimitEventType::RateLimitChecked,
            policy_id: Some(policy_id.clone()),
            bucket_name: None,
            limit: None,
            remaining: None,
            window_seconds: None,
            active_requests: None,
            max_concurrent: None,
            queue_position: None,
            queue_size: None,
            wait_time_ms: None,
            endpoint: endpoint.to_string(),
            model: model.map(str::to_string),
            request_size_bytes: None,
            duration_ms: None,
        });

        let mut bucket_statuses = Vec::new();
        let mut rejected_bucket: Option<BucketStatus> = None;

        for bucket in &policy.buckets {
            let status = self.store.check_and_increment(key_id, bucket, ts).await?;

            // Warn at 80% usage.
            if status.allowed && bucket.requests > 0 {
                let pct = (status.used * 100) / bucket.requests;
                if pct >= 80 {
                    self.record_event(RateLimitAnalyticsEvent {
                        id: uuid::Uuid::new_v4().to_string(),
                        timestamp: ts * 1000,
                        api_key_id: key_id.to_string(),
                        event_type: RateLimitEventType::RateLimitWarning,
                        policy_id: Some(policy_id.clone()),
                        bucket_name: Some(bucket.name.clone()),
                        limit: Some(bucket.requests),
                        remaining: Some(status.remaining),
                        window_seconds: Some(bucket.window_seconds),
                        active_requests: None,
                        max_concurrent: None,
                        queue_position: None,
                        queue_size: None,
                        wait_time_ms: None,
                        endpoint: endpoint.to_string(),
                        model: model.map(str::to_string),
                        request_size_bytes: None,
                        duration_ms: None,
                    });
                }
            }

            if !status.allowed && rejected_bucket.is_none() {
                rejected_bucket = Some(status.clone());
            }
            bucket_statuses.push(status);
        }

        let allowed = rejected_bucket.is_none();
        let event_type = if allowed {
            RateLimitEventType::RateLimitAllowed
        } else {
            RateLimitEventType::RateLimitRejected
        };

        self.record_event(RateLimitAnalyticsEvent {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: ts * 1000,
            api_key_id: key_id.to_string(),
            event_type,
            policy_id: Some(policy_id.clone()),
            bucket_name: rejected_bucket.as_ref().map(|b| b.name.clone()),
            limit: bucket_statuses.first().map(|b| b.limit),
            remaining: bucket_statuses.first().map(|b| b.remaining),
            window_seconds: bucket_statuses.first().map(|b| b.window_seconds),
            active_requests: None,
            max_concurrent: None,
            queue_position: None,
            queue_size: None,
            wait_time_ms: None,
            endpoint: endpoint.to_string(),
            model: model.map(str::to_string),
            request_size_bytes: None,
            duration_ms: None,
        });

        Ok(RateLimitCheckResult {
            allowed,
            policy_id,
            buckets: bucket_statuses,
            rejected_bucket,
        })
    }

    /// Read current usage without incrementing (for status endpoints).
    pub async fn get_current_usage(&self, key_id: &str) -> Result<(String, Vec<BucketStatus>)> {
        let ts = now_secs();
        let policy = self.resolve_policy(key_id).await.unwrap_or_default();
        let Some(policy) = policy else {
            return Ok(("unlimited".to_string(), vec![]));
        };
        let mut statuses = Vec::new();
        for bucket in &policy.buckets {
            let s = self.store.get_bucket_status(key_id, bucket, ts).await?;
            statuses.push(s);
        }
        Ok((policy.id, statuses))
    }

    // ---- Emergency blocks --------------------------------------------------

    /// Set an emergency block (in-memory + persistent).
    /// `duration_seconds = None` means permanent.
    pub async fn set_emergency_block(
        &self,
        key_id: &str,
        duration_seconds: Option<u64>,
        reason: &str,
    ) -> Result<()> {
        let until_secs = duration_seconds.map(|d| now_secs() + d).unwrap_or(0);
        let block = BlockInfo {
            key_id: key_id.to_string(),
            until_secs,
            reason: reason.to_string(),
        };
        self.emergency_blocks
            .write()
            .unwrap()
            .insert(key_id.to_string(), block);
        self.store.block_key(key_id, until_secs, reason).await?;
        info!(
            "Emergency block set for key {} (until {})",
            key_id, until_secs
        );
        Ok(())
    }

    pub async fn remove_emergency_block(&self, key_id: &str) -> Result<()> {
        self.emergency_blocks.write().unwrap().remove(key_id);
        self.store.unblock_key(key_id).await?;
        Ok(())
    }

    pub async fn list_emergency_blocks(&self) -> Result<Vec<BlockInfo>> {
        self.store.list_blocks().await
    }

    /// Check whether a key is currently blocked (in-memory or persistent).
    pub async fn get_block(&self, key_id: &str) -> Option<BlockInfo> {
        // Fast in-memory path first.
        {
            let blocks = self.emergency_blocks.read().unwrap();
            if let Some(b) = blocks.get(key_id) {
                let n = now_secs();
                if b.until_secs == 0 || b.until_secs > n {
                    return Some(b.clone());
                }
            }
        }
        // Fall back to persistent store.
        self.store.get_block(key_id).await.ok().flatten()
    }

    // ---- Policy CRUD -------------------------------------------------------

    pub async fn create_policy(&self, policy: RateLimitPolicy) -> Result<()> {
        self.store.save_policy(&policy).await?;
        self.policy_cache
            .write()
            .unwrap()
            .insert(policy.id.clone(), policy);
        Ok(())
    }

    pub async fn update_policy(&self, policy: RateLimitPolicy) -> Result<bool> {
        let existed = self.store.get_policy(&policy.id).await?.is_some();
        self.store.save_policy(&policy).await?;
        self.policy_cache
            .write()
            .unwrap()
            .insert(policy.id.clone(), policy);
        Ok(existed)
    }

    pub async fn delete_policy(&self, id: &str) -> Result<bool> {
        let deleted = self.store.delete_policy(id).await?;
        self.policy_cache.write().unwrap().remove(id);
        Ok(deleted)
    }

    pub async fn get_policy(&self, id: &str) -> Result<Option<RateLimitPolicy>> {
        self.store.get_policy(id).await
    }

    pub async fn list_policies(&self) -> Result<Vec<RateLimitPolicy>> {
        self.store.list_policies().await
    }

    pub async fn assign_key_policy(&self, key_id: &str, policy_id: &str) -> Result<()> {
        self.store.set_key_policy(key_id, policy_id).await
    }

    pub async fn remove_key_policy(&self, key_id: &str) -> Result<bool> {
        self.store.remove_key_policy(key_id).await
    }

    pub async fn get_key_policy_id(&self, key_id: &str) -> Result<Option<String>> {
        self.store.get_key_policy(key_id).await
    }

    pub async fn set_default_policy(&self, policy_id: &str) -> Result<()> {
        self.store.set_default_policy_id(policy_id).await
    }

    pub async fn get_default_policy_id(&self) -> Result<Option<String>> {
        self.store.get_default_policy_id().await
    }

    // ---- Response headers --------------------------------------------------

    /// Build standard rate limit response headers from a check result.
    pub fn rate_limit_headers(result: &RateLimitCheckResult) -> Vec<(String, String)> {
        let mut headers = Vec::new();

        // Primary header uses the most-restrictive bucket (smallest remaining).
        if let Some(primary) = result.buckets.iter().min_by_key(|b| b.remaining) {
            headers.push(("X-RateLimit-Limit".to_string(), primary.limit.to_string()));
            headers.push((
                "X-RateLimit-Remaining".to_string(),
                primary.remaining.to_string(),
            ));
            headers.push((
                "X-RateLimit-Reset".to_string(),
                primary.reset_at.to_string(),
            ));
        }

        let policy_str = result
            .buckets
            .iter()
            .map(|b| format!("{};w={}", b.limit, b.window_seconds))
            .collect::<Vec<_>>()
            .join(", ");
        if !policy_str.is_empty() {
            headers.push(("X-RateLimit-Policy".to_string(), policy_str));
        }

        headers.push((
            "X-RateLimit-Policy-Id".to_string(),
            result.policy_id.clone(),
        ));

        headers
    }

    // ---- Internal analytics ------------------------------------------------

    fn record_event(&self, event: RateLimitAnalyticsEvent) {
        self.metrics.record(event);
    }
}

// ============================================================================
// Extension trait for saturating subtraction helper
// ============================================================================

trait Pipe: Sized {
    fn pipe<F, R>(self, f: F) -> R
    where
        F: FnOnce(Self) -> R;
}

impl<T> Pipe for T {
    fn pipe<F, R>(self, f: F) -> R
    where
        F: FnOnce(Self) -> R,
    {
        f(self)
    }
}

// ============================================================================
// Environment variable helpers
// ============================================================================

/// Build a default `RateLimitPolicy` from environment variables (optional).
///
/// Reads:
/// - `ROUTIIUM_RATE_LIMIT_BUCKETS` — JSON array of buckets (highest priority)
/// - `ROUTIIUM_RATE_LIMIT_DAILY` — daily request limit
/// - `ROUTIIUM_RATE_LIMIT_PER_MINUTE` — per-minute request limit
/// - `ROUTIIUM_RATE_LIMIT_CUSTOM_REQUESTS` + `ROUTIIUM_RATE_LIMIT_CUSTOM_WINDOW_SECONDS`
pub fn default_policy_from_env() -> Option<RateLimitPolicy> {
    if let Ok(json) = std::env::var("ROUTIIUM_RATE_LIMIT_BUCKETS") {
        if let Ok(buckets) = serde_json::from_str::<Vec<RateLimitBucket>>(&json) {
            if !buckets.is_empty() {
                return Some(RateLimitPolicy {
                    id: "default".to_string(),
                    buckets,
                });
            }
        }
    }

    let daily = std::env::var("ROUTIIUM_RATE_LIMIT_DAILY")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    let per_minute = std::env::var("ROUTIIUM_RATE_LIMIT_PER_MINUTE")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    let custom_req = std::env::var("ROUTIIUM_RATE_LIMIT_CUSTOM_REQUESTS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    let custom_win = std::env::var("ROUTIIUM_RATE_LIMIT_CUSTOM_WINDOW_SECONDS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());

    let mut buckets = Vec::new();
    if let Some(d) = daily {
        buckets.push(RateLimitBucket {
            name: "daily".to_string(),
            requests: d,
            window_seconds: 86400,
            window_type: WindowType::Sliding,
        });
    }
    if let Some(pm) = per_minute {
        buckets.push(RateLimitBucket {
            name: "per_minute".to_string(),
            requests: pm,
            window_seconds: 60,
            window_type: WindowType::Sliding,
        });
    }
    if let (Some(r), Some(w)) = (custom_req, custom_win) {
        buckets.push(RateLimitBucket {
            name: "custom".to_string(),
            requests: r,
            window_seconds: w,
            window_type: WindowType::Sliding,
        });
    }

    if buckets.is_empty() {
        None
    } else {
        Some(RateLimitPolicy {
            id: "default".to_string(),
            buckets,
        })
    }
}
