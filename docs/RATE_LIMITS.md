# Rate Limiting

Routiium provides multi-bucket rate limiting, per-key concurrency control, and in-process analytics out of the box. This document describes the actual implementation.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Core Concepts](#2-core-concepts)
3. [Storage Backends](#3-storage-backends)
4. [Configuration](#4-configuration)
5. [Policy Resolution Hierarchy](#5-policy-resolution-hierarchy)
6. [Admin API](#6-admin-api)
7. [Response Headers](#7-response-headers)
8. [Concurrency Limiting](#8-concurrency-limiting)
9. [Analytics](#9-analytics)
10. [Auth Integration](#10-auth-integration)
11. [Configuration Examples](#11-configuration-examples)

---

## 1. Overview

The rate limiting system sits in `src/rate_limit.rs` and is wired into every proxy request handled by `src/server.rs`. When an API key is authenticated, `RateLimitManager::check_rate_limit` is called before the upstream request. All buckets in the resolved policy must pass — the first failure returns HTTP 429.

**Key features:**

- Multiple time-window buckets per policy (e.g. 500/day + 100/minute simultaneously)
- Fixed and sliding window algorithms
- Per-key policy assignments with a shared default
- Hot-reloadable JSON file configuration
- Dynamic admin API for live updates without restarts
- Emergency key blocking (in-memory fast-path + persistent store)
- Per-key in-process concurrency semaphores
- In-process analytics ring buffer

---

## 2. Core Concepts

### 2.1 Rate Limit Bucket

```rust
pub struct RateLimitBucket {
    pub name: String,           // "daily", "burst", "five_hour"
    pub requests: u64,          // Maximum requests in window
    pub window_seconds: u64,    // Window duration in seconds
    pub window_type: WindowType, // Fixed or Sliding (default: Fixed)
}
```

**WindowType:**

| Value | Behaviour | Best for |
|-------|-----------|----------|
| `Fixed` | Counter resets at fixed interval boundaries (e.g. on the hour) | Predictable billing periods |
| `Sliding` | Looks back `window_seconds` from now using a two-bucket weighted approximation | Smoother enforcement, prevents burst at boundaries |

### 2.2 Rate Limit Policy

A named collection of buckets. **All** buckets must allow the request; the first bucket that rejects causes a 429.

```rust
pub struct RateLimitPolicy {
    pub id: String,
    pub buckets: Vec<RateLimitBucket>,
}
```

### 2.3 Check Result

```rust
pub struct RateLimitCheckResult {
    pub allowed: bool,
    pub policy_id: String,          // "unlimited" when no policy resolves
    pub buckets: Vec<BucketStatus>,
    pub rejected_bucket: Option<BucketStatus>,
}

pub struct BucketStatus {
    pub name: String,
    pub limit: u64,
    pub remaining: u64,
    pub used: u64,
    pub allowed: bool,
    pub reset_at: u64,          // Unix timestamp when window resets
    pub window_seconds: u64,
}
```

---

## 3. Storage Backends

Three implementations of `RateLimitStore` are provided. Selection is automatic based on environment variables:

| Backend | Activation | Notes |
|---------|------------|-------|
| **Redis** | `ROUTIIUM_RATE_LIMIT_BACKEND=redis://...` or `ROUTIIUM_REDIS_URL` is set | Distributed; use in multi-node deployments |
| **Sled** | Default when no Redis URL is configured | Embedded persistent store; single-node only |
| **Memory** | `ROUTIIUM_RATE_LIMIT_BACKEND=memory` | In-process only; resets on restart; for tests/dev |

### 3.1 Redis

Atomic Lua scripts perform check-and-increment in a single round-trip. Both fixed and sliding windows are supported. Requires a pool (controlled by `ROUTIIUM_REDIS_POOL_MAX`, default 16).

Policy metadata (policy definitions, default policy ID, per-key assignments, emergency blocks) are stored as JSON in Redis keys, namespaced by `ROUTIIUM_REDIS_NAMESPACE` (default: none).

### 3.2 Sled

Embedded key-value store at `ROUTIIUM_RL_SLED_PATH` (default `./data/rate_limit.db`). Window counters are persisted; they survive restarts but are single-node.

### 3.3 Memory

All state lives in process. Zero dependencies. Suitable for testing and single-process dev environments. State is lost on restart.

---

## 4. Configuration

### 4.1 Enabling Rate Limiting

Rate limiting is **disabled by default** unless a backend or config file is provided. It activates when any of these are true:

- `ROUTIIUM_RATE_LIMIT_BACKEND` is set
- `ROUTIIUM_REDIS_URL` is set (Redis is used as the backend)
- `--rate-limit-config=PATH` is passed on the CLI
- `ROUTIIUM_RATE_LIMIT_CONFIG` environment variable is set

Set `ROUTIIUM_RATE_LIMIT_ENABLED=false` to explicitly disable even when the above apply.

### 4.2 Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `ROUTIIUM_RATE_LIMIT_ENABLED` | `true` when backend/config present | Set to `false` to disable entirely |
| `ROUTIIUM_RATE_LIMIT_BACKEND` | auto | `redis://host:port`, `sled:/path/to/db`, or `memory` |
| `ROUTIIUM_REDIS_URL` | — | Redis URL; used as RL backend when `ROUTIIUM_RATE_LIMIT_BACKEND` is unset |
| `ROUTIIUM_REDIS_POOL_MAX` | `16` | r2d2 pool size for Redis connections |
| `ROUTIIUM_RL_SLED_PATH` | `./data/rate_limit.db` | Sled database file path |
| `ROUTIIUM_RATE_LIMIT_CONFIG` | — | Path to rate limit JSON config file (overridden by `--rate-limit-config=`) |
| `ROUTIIUM_RATE_LIMIT_DAILY` | — | Quick daily limit; creates a sliding-window bucket |
| `ROUTIIUM_RATE_LIMIT_PER_MINUTE` | — | Quick per-minute limit; creates a sliding-window bucket |
| `ROUTIIUM_RATE_LIMIT_CUSTOM_REQUESTS` | — | Custom bucket request count (pair with `ROUTIIUM_RATE_LIMIT_CUSTOM_WINDOW_SECONDS`) |
| `ROUTIIUM_RATE_LIMIT_CUSTOM_WINDOW_SECONDS` | — | Custom bucket window in seconds |
| `ROUTIIUM_RATE_LIMIT_BUCKETS` | — | Full bucket definition as a JSON array (highest priority for env-based config) |

**Priority order for env-based default policy:**

1. `ROUTIIUM_RATE_LIMIT_BUCKETS` (JSON array, takes precedence over all other env vars)
2. `ROUTIIUM_RATE_LIMIT_DAILY` and/or `ROUTIIUM_RATE_LIMIT_PER_MINUTE`
3. `ROUTIIUM_RATE_LIMIT_CUSTOM_REQUESTS` + `ROUTIIUM_RATE_LIMIT_CUSTOM_WINDOW_SECONDS`

All env-defined buckets use the **Sliding** window type.

### 4.3 CLI Flag

```bash
routiium --rate-limit-config=rate_limits.json
```

The flag is equivalent to `ROUTIIUM_RATE_LIMIT_CONFIG` and takes precedence over it.

### 4.4 File-Based Configuration (Hot-Reloadable)

The JSON config file defines policies, per-key overrides, and a default policy. Reload it at runtime with `POST /admin/rate-limits/reload` — no restart required.

```json
{
  "version": "1.0",
  "default_policy": "free",
  "policies": {
    "free": {
      "buckets": [
        { "name": "daily",  "requests": 100, "window_seconds": 86400 },
        { "name": "burst",  "requests": 20,  "window_seconds": 60   }
      ],
      "concurrency": {
        "max_concurrent": 2,
        "max_queue_size": 0,
        "queue_timeout_ms": 30000,
        "strategy": "Reject"
      }
    },
    "pro": {
      "buckets": [
        { "name": "daily",  "requests": 5000, "window_seconds": 86400 },
        { "name": "minute", "requests": 500,  "window_seconds": 60   }
      ]
    },
    "enterprise": {
      "buckets": [
        { "name": "daily",      "requests": 100000, "window_seconds": 86400 },
        { "name": "minute",     "requests": 5000,   "window_seconds": 60   },
        { "name": "five_hour",  "requests": 25000,  "window_seconds": 18000 }
      ]
    }
  },
  "key_overrides": {
    "key-abc123": "pro",
    "key-def456": "enterprise"
  }
}
```

**`PolicyDef` fields:**

| Field | Type | Description |
|-------|------|-------------|
| `buckets` | `Vec<RateLimitBucket>` | Required. One or more time-window limits. |
| `concurrency` | `ConcurrencyConfig?` | Optional. Per-key concurrency settings. |

---

## 5. Policy Resolution Hierarchy

For each request, `RateLimitManager::resolve_policy` walks the following chain in order:

1. **In-memory emergency block** — fast-path check in the emergency blocks map. If the key is blocked, returns `BLOCKED` error immediately (results in HTTP 429).
2. **Persistent store block** — checks the backend for a stored block record. Same result if found.
3. **Per-key store assignment** — if the key has been explicitly assigned a policy via the admin API (`POST /admin/rate-limits/keys/{key_id}`), that policy is used.
4. **File config key override** — if the loaded JSON config has an entry in `key_overrides` for this key ID, that policy is used.
5. **Default policy from store** — if a default policy ID has been set via `POST /admin/rate-limits/default`, that policy is used.
6. **Default policy from file config** — if the JSON config has a `default_policy` field set, that policy is used.
7. **Unlimited** — if none of the above resolves, the request is allowed with no rate limiting applied.

Dynamic admin API assignments (steps 3 and 5) are persisted in the backend store and survive restarts. File config values (steps 4 and 6) are re-read on reload. Emergency blocks in step 1 are also persisted.

---

## 6. Admin API

All admin endpoints require `Authorization: Bearer <ROUTIIUM_ADMIN_TOKEN>` when that env var is set. Returns `HTTP 503` when rate limiting is not enabled.

### 6.1 Policy Management

| Method | Path | Body | Description |
|--------|------|------|-------------|
| `GET` | `/admin/rate-limits/policies` | — | List all policies |
| `POST` | `/admin/rate-limits/policies` | `RateLimitPolicy` JSON | Create a policy |
| `GET` | `/admin/rate-limits/policies/{id}` | — | Get a policy by ID |
| `PUT` | `/admin/rate-limits/policies/{id}` | `RateLimitPolicy` JSON | Replace a policy |
| `DELETE` | `/admin/rate-limits/policies/{id}` | — | Delete a policy |

**Policy JSON shape:**
```json
{
  "id": "standard",
  "buckets": [
    { "name": "daily",  "requests": 500,  "window_seconds": 86400, "window_type": "Sliding" },
    { "name": "minute", "requests": 100,  "window_seconds": 60,    "window_type": "Fixed"   }
  ]
}
```

`window_type` is optional and defaults to `"Fixed"`.

### 6.2 Default Policy

| Method | Path | Body | Description |
|--------|------|------|-------------|
| `GET` | `/admin/rate-limits/default` | — | Get the current default policy ID |
| `POST` | `/admin/rate-limits/default` | `{ "policy_id": "free" }` | Set the default policy |

### 6.3 Per-Key Assignment

| Method | Path | Body | Description |
|--------|------|------|-------------|
| `POST` | `/admin/rate-limits/keys/{key_id}` | `{ "policy_id": "pro" }` | Assign a policy to a key |
| `DELETE` | `/admin/rate-limits/keys/{key_id}` | — | Remove a key's policy (falls back to default) |
| `GET` | `/admin/rate-limits/keys/{key_id}/status` | — | Full rate limit status for a key |

**Status response:**
```json
{
  "key_id": "sk_abc.xyz",
  "blocked": false,
  "block": null,
  "policy_id": "pro",
  "assigned_policy_id": "pro",
  "policy": { "id": "pro", "buckets": [...] },
  "bucket_usage": [
    {
      "name": "daily",
      "limit": 5000,
      "remaining": 4850,
      "used": 150,
      "allowed": true,
      "reset_at": 1720137600,
      "window_seconds": 86400
    }
  ],
  "concurrency": {
    "active": 1,
    "max_concurrent": 5,
    "queued": 0,
    "max_queue_size": 10
  }
}
```

### 6.4 Emergency Blocks

Emergency blocks bypass the policy lookup and immediately reject the key. They are stored both in-memory (fast-path) and in the backend store (persistent across restarts).

| Method | Path | Body | Description |
|--------|------|------|-------------|
| `POST` | `/admin/rate-limits/emergency` | `EmergencyBlockBody` | Block a key |
| `GET` | `/admin/rate-limits/emergency` | — | List all active blocks |
| `DELETE` | `/admin/rate-limits/emergency/{key_id}` | — | Remove a block |

**EmergencyBlockBody:**
```json
{
  "key_id": "sk_abc.xyz",
  "duration_secs": 3600,
  "reason": "suspected abuse"
}
```

`duration_secs` is optional. Omit or set to `null` for a permanent block until manually removed.

### 6.5 Reload

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/admin/rate-limits/reload` | Re-read the rate limit config file from disk |

Returns `{ "reloaded": true }` on success, or `{ "reloaded": false, "error": "..." }` if no config path was configured.

### 6.6 Concurrency Status

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/admin/concurrency/keys/{key_id}` | Live concurrency counters for a key |

**Response:**
```json
{
  "key_id": "sk_abc.xyz",
  "active": 2,
  "queued": 1,
  "max_concurrent": 5,
  "max_queue_size": 10
}
```

Returns `{ "active": 0 }` when no concurrency config is in effect for the key.

### 6.7 Analytics

| Method | Path | Query Params | Description |
|--------|------|--------------|-------------|
| `GET` | `/admin/analytics/rate-limits` | `key_id`, `start`, `end`, `limit` | Query rate limit analytics events |

**Response:**
```json
{
  "events": [ ... ],
  "count": 42,
  "metrics": {
    "total_checks": 1000,
    "total_allowed": 850,
    "total_rejected": 150,
    "rejections_by_bucket": { "daily": 100, "minute": 50 },
    "concurrency_rejected": 5,
    "concurrency_queued": 20,
    "concurrency_queue_timeouts": 2
  },
  "start": null,
  "end": null
}
```

---

## 7. Response Headers

When rate limiting is active, these headers are added to the response:

| Header | Value | Example |
|--------|-------|---------|
| `X-RateLimit-Limit` | Bucket limit | `500` |
| `X-RateLimit-Remaining` | Remaining requests in window | `349` |
| `X-RateLimit-Reset` | Unix timestamp when window resets | `1720137600` |
| `X-RateLimit-Policy` | Policy ID applied | `pro` |
| `X-RateLimit-Policy-Id` | Policy ID applied (same value) | `pro` |

When the request is rejected (HTTP 429):

```json
{
  "error": {
    "message": "Rate limit exceeded: bucket 'daily' (used: 500/500, resets at 1720137600)",
    "type": "rate_limit_exceeded",
    "code": "rate_limit_exceeded"
  }
}
```

---

## 8. Concurrency Limiting

Concurrency limits cap the number of **simultaneously in-flight** requests per key (as opposed to rate limiting which caps requests per time window). They are enforced using per-key `tokio::sync::Semaphore` instances managed by `ConcurrencyManager`.

### 8.1 Configuration

```rust
pub struct ConcurrencyConfig {
    pub max_concurrent: u32,   // Max simultaneous in-flight requests (0 = unlimited)
    pub max_queue_size: u32,   // Max queued requests waiting for a slot (0 = disable queue)
    pub queue_timeout_ms: u64, // Max wait time in queue (default: 30000 ms)
    pub strategy: QueueStrategy,
}

pub enum QueueStrategy {
    Reject,        // Return 429 immediately when at capacity (default)
    QueueFifo,     // Queue requests FIFO; process as slots free
    QueuePriority, // Reserved for future priority queue support
}
```

Concurrency config is set per-policy in the JSON config file under the `concurrency` key of a `PolicyDef`. It is not currently exposed as a standalone admin endpoint — update the policy to change concurrency settings.

### 8.2 Acquire / Release

```
acquire(key_id, config) → ConcurrencyResult::Allowed(OwnedSemaphorePermit)
                         | ConcurrencyResult::Rejected { active, limit, queue_full }
```

The returned `OwnedSemaphorePermit` must be held for the duration of the upstream call. Dropping it releases the semaphore slot. Concurrency state is **in-process only** — it does not sync across multiple Routiium nodes.

### 8.3 Response on Rejection

When concurrency limit is reached and strategy is `Reject`, the response is HTTP 429:

```json
{
  "error": {
    "message": "Concurrency limit reached (active: 5/5, queue: disabled)",
    "type": "rate_limit_exceeded",
    "code": "concurrency_limit_exceeded"
  }
}
```

---

## 9. Analytics

Rate limit analytics are handled by an in-process `MetricsStore` — a thread-safe ring buffer (default capacity: 10,000 events) that requires no external dependencies.

### 9.1 Event Types

```rust
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
```

### 9.2 Event Schema

```rust
pub struct RateLimitAnalyticsEvent {
    pub id: String,                       // UUID
    pub timestamp: u64,                   // Milliseconds since Unix epoch
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
```

### 9.3 Per-Key Metrics

`MetricsStore` aggregates counters per key in `RateLimitMetrics`:

```rust
pub struct RateLimitMetrics {
    pub total_checks: u64,
    pub total_allowed: u64,
    pub total_rejected: u64,
    pub rejections_by_bucket: HashMap<String, u64>,
    pub concurrency_rejected: u64,
    pub concurrency_queued: u64,
    pub concurrency_queue_timeouts: u64,
}
```

Query via `GET /admin/analytics/rate-limits?key_id=<id>&limit=100`.

---

## 10. Auth Integration

`ApiKeyInfo` carries an optional `rate_limit_policy` field:

```rust
pub struct ApiKeyInfo {
    pub id: String,
    pub label: String,
    // ...
    pub rate_limit_policy: Option<String>, // policy ID override for this key
}
```

When a key is generated with `rate_limit_policy` set, that policy ID is used at step 3 of the [resolution hierarchy](#5-policy-resolution-hierarchy). This is an alternative to the `POST /admin/rate-limits/keys/{key_id}` endpoint — both write to the same per-key store slot.

---

## 11. Configuration Examples

### Example 1: Simple Daily Limit via Environment Variables

```bash
# Enable rate limiting with a sled backend
ROUTIIUM_RATE_LIMIT_BACKEND=sled:./data/rl.db
# 200 requests/day default limit
ROUTIIUM_RATE_LIMIT_DAILY=200
```

### Example 2: Daily + Burst via Environment Variables

```bash
ROUTIIUM_RATE_LIMIT_BACKEND=memory
ROUTIIUM_RATE_LIMIT_BUCKETS='[
  {"name":"daily","requests":500,"window_seconds":86400,"window_type":"Sliding"},
  {"name":"burst","requests":50,"window_seconds":60,"window_type":"Fixed"}
]'
```

### Example 3: Tiered Plans via Config File

```bash
ROUTIIUM_RATE_LIMIT_BACKEND=redis://localhost:6379
routiium --rate-limit-config=rate_limits.json
```

`rate_limits.json`:
```json
{
  "version": "1.0",
  "default_policy": "free",
  "policies": {
    "free": {
      "buckets": [
        { "name": "daily",  "requests": 100, "window_seconds": 86400 },
        { "name": "minute", "requests": 10,  "window_seconds": 60   }
      ],
      "concurrency": { "max_concurrent": 1, "max_queue_size": 0, "strategy": "Reject" }
    },
    "pro": {
      "buckets": [
        { "name": "daily",  "requests": 5000, "window_seconds": 86400 },
        { "name": "minute", "requests": 200,  "window_seconds": 60   }
      ],
      "concurrency": { "max_concurrent": 5, "max_queue_size": 10, "strategy": "QueueFifo" }
    }
  }
}
```

### Example 4: Custom 5-Hour Window

```json
{
  "id": "five-hour-plan",
  "buckets": [
    { "name": "five_hour", "requests": 1000, "window_seconds": 18000, "window_type": "Sliding" }
  ]
}
```

### Example 5: Live Policy Update

```bash
# Create a new policy
curl -X POST http://localhost:8088/admin/rate-limits/policies \
  -H "Authorization: Bearer $ROUTIIUM_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"id":"beta","buckets":[{"name":"daily","requests":2000,"window_seconds":86400}]}'

# Assign it to a key
curl -X POST http://localhost:8088/admin/rate-limits/keys/sk_abc123.xyz \
  -H "Authorization: Bearer $ROUTIIUM_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"policy_id":"beta"}'

# Check status
curl http://localhost:8088/admin/rate-limits/keys/sk_abc123.xyz/status \
  -H "Authorization: Bearer $ROUTIIUM_ADMIN_TOKEN"

# Emergency block
curl -X POST http://localhost:8088/admin/rate-limits/emergency \
  -H "Authorization: Bearer $ROUTIIUM_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"key_id":"sk_abc123.xyz","duration_secs":3600,"reason":"abuse detected"}'
```

---

## Notes

- **Multi-node deployments:** Use the Redis backend so rate limit counters are shared across all instances. Concurrency tracking (`ConcurrencyManager`) is always in-process — it does not distribute across nodes. For multi-node concurrency limits, implement an external counter or rely on rate limits alone.
- **No restarts required:** Policies, default policy, and key assignments can be changed live via the admin API. The file config can be hot-reloaded via `POST /admin/rate-limits/reload`.
- **Blocked keys return 429**, not 401, to avoid leaking block status to callers.
- **Unlimited keys:** If no policy resolves for a key (no assignment, no default), the key passes through with no rate limiting applied.
