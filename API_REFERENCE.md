# Routiium – API Reference

Routiium is an HTTP service that:
- Converts OpenAI Chat Completions requests into the modern OpenAI Responses API payloads.
- Proxies both Chat Completions and Responses requests to one or more upstream providers.
- Optionally injects system prompts per model/API at runtime.
- Provides API key issuance/validation (managed mode), analytics collection with cost tracking, and runtime config reloads.
- Supports intelligent routing via Router integration with virtual model aliases and policy enforcement.

This document details all HTTP routes, expected authentication, parameters, and examples.

Base URL: http://localhost:PORT (default PORT configured by your deployment)

Content-Type: application/json unless otherwise specified


## Authentication

Routiium supports two modes:

1) Managed mode (recommended)
- Condition: Server has OPENAI_API_KEY set or uses Router-based authentication.
- Client sends an internal access key using Authorization: Bearer sk_<id>.<secret>.
- The proxy validates the token (issue/revoke/expire via the "Keys" endpoints) and substitutes the upstream provider key (OPENAI_API_KEY or per-backend key_env).
- Use the Keys endpoints to issue/revoke client tokens.
- Provides full analytics, cost tracking, and usage monitoring capabilities.

2) Passthrough mode
- Condition: OPENAI_API_KEY is NOT set on the server.
- Client sends their provider API key directly using Authorization: Bearer <provider_api_key>.
- The proxy forwards that upstream unchanged.
- Limited analytics capabilities (no per-user tracking).

Common headers:
- Authorization: Bearer <token>
- Content-Type: application/json
- For streaming: Accept: text/event-stream and include "stream": true in body.

Error responses:
- Status: appropriate 4xx/5xx
- Body: {"error":{"message":"human-readable error"}}

Observability headers (when Router is enabled):
- X-Route-Id: Unique route identifier for correlation
- X-Resolved-Model: Actual upstream model used
- X-Policy-Rev: Policy revision from Router
- X-Content-Used: Privacy attestation (what content Router consumed)
- X-Route-Cache: Cache status (hit, miss, stale)
- Router-Schema: Router API schema version


## Routing and Multi-backend

Routiium supports two routing modes:

### 1. Router-based Routing (Recommended)
Use a Router service to enable virtual model aliases, policy enforcement, and intelligent routing decisions.

Configuration:
```bash
ROUTIIUM_ROUTER_URL=http://router:9090
ROUTIIUM_ROUTER_TIMEOUT_MS=50
ROUTIIUM_ROUTER_PRIVACY_MODE=features  # features, summary, or full
ROUTIIUM_ROUTER_STRICT=1               # Fail on router errors
ROUTIIUM_CACHE_TTL_MS=60000            # Plan cache TTL
```

Benefits:
- Virtual model aliases (e.g., "edu-fast" → "gpt-4o-mini")
- Cost-aware routing and budgeting
- Policy enforcement (privacy tiers, rate limits)
- Multi-turn conversation stickiness
- Dynamic catalog updates without restarts

### 2. Legacy Prefix-based Routing
Route by model prefix with static configuration:

- ROUTIIUM_BACKENDS rules (semicolon-separated):
  - prefix=<model_prefix>
  - base|base_url=<upstream_base_url>
  - key_env|api_key_env=<ENV_VAR_WITH_API_KEY> (optional)
  - mode=responses|chat|bedrock (optional; default from env)

Example:
```bash
ROUTIIUM_BACKENDS="gpt-4o,base=https://api.openai.com/v1,mode=responses;local-,base=http://localhost:8000/v1,key_env=LOCAL_API_KEY,mode=chat"
```

Fallback: When Router is unavailable and ROUTIIUM_ROUTER_STRICT is not set, the system falls back to legacy routing.


## System Prompt Injection

If a system prompt config is loaded, Routiium can inject system prompts:
- For /v1/responses: injects a {"role":"system","content":"..."} message into messages based on injection_mode: prepend (default), append, or replace.
- For /v1/chat/completions: injects a system message by re-serializing the chat payload.

Configuration supports:
- Global system prompts
- Per-model overrides
- Per-API overrides
- Injection modes: prepend, append, replace

Configuration is hot-reloadable (see Reload endpoints).

## Cost Tracking and Pricing

When ROUTIIUM_PRICING_CONFIG is set, Routiium tracks costs for all requests:
- Calculates costs based on token usage and model pricing
- Supports input, output, cached, and reasoning tokens
- Tracks costs per model, per user, and per time period
- Integrates with analytics for cost reports

Pricing config format:
```json
{
  "models": {
    "gpt-4o": {
      "input_per_million": 2.50,
      "output_per_million": 10.00,
      "cached_per_million": 1.25,
      "reasoning_per_million": null
    }
  }
}
```


# Endpoints

The service registers these routes:

- GET /status
- POST /convert
- POST /v1/chat/completions
- POST /v1/responses
- GET /keys
- POST /keys/generate
- POST /keys/revoke
- POST /keys/set_expiration
- POST /reload/mcp
- POST /reload/system_prompt
- POST /reload/routing
- POST /reload/all
- GET /analytics/stats
- GET /analytics/events
- GET /analytics/aggregate
- GET /analytics/export
- POST /analytics/clear
- GET /chat_history/:conversation_id
- DELETE /chat_history/:conversation_id


## GET /status

Returns runtime status, discovered routes, and feature flags.

Auth: None

Response:
- name, version
- routes: list of available routes
- features.mcp: {enabled, config_path, reloadable}
- features.system_prompt: {enabled, config_path, reloadable}
- features.analytics: {enabled, stats?}

Example:
```
curl -s http://localhost:PORT/status | jq
```

Example response:
```json
{
  "name": "routiium",
  "version": "x.y.z",
  "proxy_enabled": true,
  "routes": ["/status", "/convert", "/v1/chat/completions", "/v1/responses", "..."],
  "features": {
    "mcp": { "enabled": true, "config_path": "mcp.json", "reloadable": true },
    "system_prompt": { "enabled": true, "config_path": "system_prompt.json", "reloadable": true },
    "analytics": { 
      "enabled": true, 
      "backend": "jsonl",
      "stats": { 
        "total_events": 123,
        "total_cost": 12.45,
        "total_input_tokens": 50000,
        "total_output_tokens": 25000
      } 
    },
    "router": {
      "mode": "remote",
      "url": "http://router:9090",
      "strict": true,
      "cache_hits": 42,
      "cache_misses": 18
    },
    "pricing": {
      "enabled": true,
      "config_path": "pricing.json",
      "models_count": 15
    }
  }
}
```


## POST /convert

Converts a Chat Completions request into an OpenAI Responses API payload. No network call is performed.

Auth: None

Query parameters:
- conversation_id (optional): If provided, used in conversion to make the call stateful for the Responses API.
- previous_response_id (optional): Injects `previous_response_id` into the resulting Responses payload for state-linked turns.

Body:
- A valid Chat Completions JSON request.

Response:
- Converted Responses-shaped JSON.

Example:
```
curl -s -X POST "http://localhost:PORT/convert?conversation_id=abc123" \
  -H "Content-Type: application/json" \
  -d '{
    "model":"gpt-4o-mini",
    "messages":[{"role":"user","content":"Hello"}]
  }' | jq
```


## POST /v1/chat/completions

Pass-through for native Chat Completions requests. Optionally injects system prompts.

Auth:
- Managed mode: Authorization: Bearer sk_<id>.<secret> (validated; upstream API key supplied by server).
- Passthrough mode: Authorization: Bearer <provider_api_key> (forwarded upstream).

Query parameters:
- `conversation_id` (optional) – forces the converted Responses payload to include this conversation id when the routed upstream expects `/v1/responses`.
- `previous_response_id` (optional) – forwarded as `previous_response_id` for stateful Responses calls.

Body:
- Standard Chat Completions JSON.
- stream (bool, optional): When true, the proxy streams Server-Sent Events.

Streaming:
- Set "stream": true
- Optionally set Accept: text/event-stream
- The proxy streams upstream tokens/events back to the client.

Example (managed mode):
Example (streaming):
```
curl -N -X POST http://localhost:PORT/v1/chat/completions \
  -H "Authorization: Bearer sk_abc.def" \
  -H "Content-Type: application/json" \
  -d '{
    "model":"gpt-4o-mini",
    "stream": true,
    "messages":[{"role":"user","content":"Tell me a joke"}]
  }'
```

Response headers (with Router):
```
HTTP/1.1 200 OK
X-Route-Id: route_abc123xyz
X-Resolved-Model: gpt-4o-mini-2024-07-18
X-Policy-Rev: 42
X-Content-Used: features_only
Router-Schema: 1.1
```

Example (passthrough mode):
```
curl -s -X POST http://localhost:PORT/v1/chat/completions \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model":"gpt-4o-mini",
    "messages":[{"role":"user","content":"Explain HTTP/2"}]
  }'
```


## POST /v1/responses

Pass-through for native OpenAI Responses API requests. Optionally injects system prompts. Supports multi-backend routing, and when the configured backend mode=chat for a matched model prefix, non-stream requests are translated to Chat Completions upstream.

Auth:
- Managed mode: Authorization: Bearer sk_<id>.<secret>
- Passthrough mode: Authorization: Bearer <provider_api_key>

Body:
- Standard Responses API payload (e.g., model, input/messages, tools, conversation, stream, etc.)

Streaming:
- Set "stream": true
- The proxy uses SSE to stream upstream events.

Example (non-stream):
```
curl -s -X POST http://localhost:PORT/v1/responses \
  -H "Authorization: Bearer sk_abc.def" \
  -H "Content-Type: application/json" \
  -d '{
    "model":"gpt-4o",
    "input":[{"role":"user","content":"Summarize this in bullet points"}],
    "stream": false
  }' | jq
```

Example (streaming):
```
curl -N -X POST http://localhost:PORT/v1/responses \
  -H "Authorization: Bearer sk_abc.def" \
  -H "Content-Type: application/json" \
  -H "Accept: text/event-stream" \
  -d '{
    "model":"gpt-4o",
    "input":[{"role":"user","content":"Write a short poem"}],
    "stream": true
  }'
```


## Keys – API key management (Managed Mode)

These endpoints manage internal access tokens that clients use in managed mode. There is no separate admin auth here; deploy behind a trusted network boundary or enforce ACLs at your reverse proxy.

Shared types (typical):
- GeneratedKey: { id, token, created_at, expires_at?, label?, scopes? }
- ApiKeyInfo: { id, label?, created_at, expires_at?, revoked_at?, scopes? }

Environment variables:
- ROUTIIUM_KEYS_REQUIRE_EXPIRATION: "1|true|yes|on" to require expiration when generating.
- ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS: default TTL in seconds when not provided in request.

### GET /keys

Lists known keys (no tokens/secrets, only metadata).

Auth: None (protect via network ACL)

Response: Array<ApiKeyInfo>

Example:
```
curl -s http://localhost:PORT/keys | jq
```

### POST /keys/generate

Creates a new client access key.

Auth: None (protect via network ACL)

Body:
- label (string, optional)
- ttl_seconds (u64, optional)
- expires_at (unix seconds, optional; takes precedence over ttl_seconds)
- scopes (array<string>, optional)

Responses:
- 200 OK + GeneratedKey on success
- 400 if expiration is required by policy and not provided

Example:
```
curl -s -X POST http://localhost:PORT/keys/generate \
  -H "Content-Type: application/json" \
  -d '{
    "label":"demo",
    "ttl_seconds": 86400,
    "scopes": ["inference"]
  }' | jq
```

### POST /keys/revoke

Revokes a key by id.

Auth: None (protect via network ACL)

Body:
- id (string) – the key id to revoke

Response:
- {"revoked": true|false, "id": "<id>"}

Example:
```
curl -s -X POST http://localhost:PORT/keys/revoke \
  -H "Content-Type: application/json" \
  -d '{"id":"<key-id>"}' | jq
```

### POST /keys/set_expiration

Sets or clears expiration for a key.

Auth: None (protect via network ACL)

Body:
- id (string)
- expires_at (unix seconds, optional)
- ttl_seconds (u64, optional) – if provided, new expiration = now + ttl_seconds
  - Precedence: expires_at > ttl_seconds. If neither is present, clears expiration.

Response:
- {"updated": true|false, "id": "<id>", "expires_at": <unix|null>}

Example:
```
curl -s -X POST http://localhost:PORT/keys/set_expiration \
  -H "Content-Type: application/json" \
  -d '{"id":"<key-id>", "ttl_seconds": 604800}' | jq
```


## Reload – Runtime configuration reloads

### POST /reload/mcp

Reloads the MCP configuration file and reconnects servers.

Auth: None (protect via network ACL)

Prerequisite: The server must have been started with an MCP config path.

Response (success):
```json
{
  "success": true,
  "message": "MCP configuration reloaded",
  "servers": [{"name":"...","status":"..."}],
  "count": 2
}
```

### POST /reload/system_prompt

Reloads system prompt configuration.

Auth: None (protect via network ACL)

Prerequisite: The server must have been started with a system prompt config path.

Response (success):
```json
{
  "success": true,
  "message": "System prompt configuration reloaded",
  "enabled": true,
  "has_global": true,
  "per_model_count": 2,
  "per_api_count": 2,
  "injection_mode": "prepend"
}
```

### POST /reload/routing

Reloads routing configuration (local alias map or ROUTIIUM_BACKENDS).

Auth: None (protect via network ACL)

Response (success):
```json
{
  "success": true,
  "message": "Routing configuration reloaded",
  "backends_count": 3
}
```

### POST /reload/all

Reloads both MCP and system prompt configurations (when configured).

Auth: None (protect via network ACL)

Response (example):
```json
{
  "mcp": {
    "success": true,
    "message": "MCP configuration reloaded",
    "servers": [],
    "count": 0
  },
  "system_prompt": {
    "success": true,
    "message": "System prompt configuration reloaded",
    "enabled": true,
    "has_global": true,
    "per_model_count": 1,
    "per_api_count": 2,
    "injection_mode": "prepend"
  },
  "routing": {
    "success": true,
    "message": "Routing configuration reloaded",
    "backends_count": 3
  }
}
```


## Analytics

If analytics initializes successfully from the environment, these endpoints are enabled.

Storage backends:
- **JSONL** (default): Append-only log at `data/analytics.jsonl`
- **Redis**: Production-ready with TTL and indexing
- **Sled**: Embedded database for single-server deployments
- **Memory**: Development only, data lost on restart

Configuration:
```bash
# JSONL (default)
ROUTIIUM_ANALYTICS_JSONL_PATH=./data/analytics.jsonl

# Redis (recommended for production)
ROUTIIUM_ANALYTICS_REDIS_URL=redis://localhost:6379
ROUTIIUM_ANALYTICS_TTL_SECONDS=2592000  # 30 days

# Sled
ROUTIIUM_ANALYTICS_SLED_PATH=./analytics.db
ROUTIIUM_ANALYTICS_TTL_SECONDS=2592000

# Memory (dev only)
ROUTIIUM_ANALYTICS_FORCE_MEMORY=true
ROUTIIUM_ANALYTICS_MAX_EVENTS=10000
```

Notes:
- Time parameters are Unix seconds.
- Defaults:
  - events/aggregate default to the last 1 hour if not specified
  - export defaults to last 24 hours and "json" format
- Cost tracking requires ROUTIIUM_PRICING_CONFIG to be set

### GET /analytics/stats

High-level analytics stats.

Auth: None (protect via network ACL)

Response: Stats JSON with cost tracking information

Example:
```
curl -s http://localhost:PORT/analytics/stats | jq
```

Example response:
```json
{
  "total_events": 1542,
  "backend_type": "jsonl",
  "ttl_seconds": null,
  "max_events": null,
  "total_cost": 45.67,
  "total_input_tokens": 150000,
  "total_output_tokens": 75000,
  "total_cached_tokens": 25000,
  "total_reasoning_tokens": 0,
  "avg_tokens_per_second": 325.4
}
```

### GET /analytics/events

Query raw events in a time range.

Auth: None (protect via network ACL)

Query parameters:
- start (u64, optional) – default now - 3600
- end (u64, optional) – default now
- limit (u64, optional) – maximum number of events

Response:
```json
{
  "events": [
    {
      "id": "evt_abc123",
      "timestamp": 1730000100,
      "request": {
        "endpoint": "/v1/chat/completions",
        "method": "POST",
        "model": "gpt-4o-mini",
        "stream": false,
        "input_tokens": 50
      },
      "response": {
        "status_code": 200,
        "success": true,
        "output_tokens": 120
      },
      "performance": {
        "duration_ms": 1247,
        "tokens_per_second": 96.3
      },
      "token_usage": {
        "prompt_tokens": 50,
        "completion_tokens": 120,
        "total_tokens": 170,
        "cached_tokens": 20,
        "reasoning_tokens": null
      },
      "cost": {
        "input_cost": 0.0000075,
        "output_cost": 0.000072,
        "cached_cost": 0.0000015,
        "total_cost": 0.0000810,
        "currency": "USD",
        "pricing_model": "gpt-4o-mini"
      },
      "routing": {
        "backend": "openai",
        "upstream_mode": "chat",
        "system_prompt_applied": true
      }
    }
  ],
  "count": 1,
  "start": 1730000000,
  "end": 1730003600
}
```

Example:
```
curl -s "http://localhost:PORT/analytics/events?start=1730000000&end=1730007200&limit=100" | jq
```

### GET /analytics/aggregate

Aggregated metrics over a time range.

Auth: None (protect via network ACL)

Query parameters:
- start (u64, optional) – default now - 3600
- end (u64, optional) – default now

Response: Aggregates JSON (counts, token totals, duration averages, cost totals, model breakdowns, etc.)

Example:
```
curl -s "http://localhost:PORT/analytics/aggregate?start=1730000000&end=1730007200" | jq
```

Example response:
```json
{
  "total_requests": 1542,
  "successful_requests": 1523,
  "failed_requests": 19,
  "total_input_tokens": 45230,
  "total_output_tokens": 89441,
  "total_cached_tokens": 12500,
  "total_reasoning_tokens": 0,
  "avg_duration_ms": 1247.3,
  "avg_tokens_per_second": 325.4,
  "total_cost": 45.67,
  "cost_by_model": {
    "gpt-4o": 32.50,
    "gpt-4o-mini": 13.17
  },
  "models_used": {
    "gpt-4o": 892,
    "gpt-4o-mini": 650
  },
  "endpoints_hit": {
    "/v1/chat/completions": 892,
    "/v1/responses": 650
  },
  "backends_used": {
    "openai": 1542
  },
  "period_start": 1730000000,
  "period_end": 1730007200
}
```

### GET /analytics/export

Export events for a time range.

Auth: None (protect via network ACL)

Query parameters:
- start (u64, optional) – default now - 86400
- end (u64, optional) – default now
- format (string, optional) – "json" (default) or "csv"

Responses:
- JSON: application/json attachment (complete event data)
- CSV: text/csv attachment with header row (flattened data)

CSV columns include:
- id, timestamp, endpoint, method, model, stream
- status_code, success, duration_ms
- input_tokens, output_tokens, cached_tokens, reasoning_tokens
- input_cost, output_cost, cached_cost, total_cost
- backend, upstream_mode, tokens_per_second
- api_key_id, api_key_label

Examples:
```
curl -s -OJ "http://localhost:PORT/analytics/export?format=json"
curl -s -OJ "http://localhost:PORT/analytics/export?format=csv&start=1730000000&end=1730086400"
```

### POST /analytics/clear

Clears all analytics data.

Auth: None (protect via network ACL)

Response:
```json
{ "success": true, "message": "Analytics data cleared" }
```

Example:
```
curl -s -X POST http://localhost:PORT/analytics/clear | jq
```


# Practical Examples

## Convert only (no upstream call)
```
curl -s -X POST "http://localhost:PORT/convert" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role":"user", "content":"Summarize HTTP/1.1 vs HTTP/2"}]
  }' | jq
```

## Responses API with state and streaming
```
curl -N -X POST http://localhost:PORT/v1/responses \
  -H "Authorization: Bearer sk_abc.def" \
  -H "Content-Type: application/json" \
  -H "Accept: text/event-stream" \
  -d '{
    "model": "gpt-4o",
    "conversation": {"id":"conv_123"},
    "input": [{"role":"user","content":"Stream me a limerick about routers"}],
    "stream": true
  }'
```

## Chat Completions with Router alias and cost tracking
```
curl -s -X POST http://localhost:PORT/v1/chat/completions \
  -H "Authorization: Bearer sk_abc.def" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "edu-fast",
    "messages": [{"role":"user","content":"Write a haiku about latencies"}]
  }' | jq
```

Response includes cost information when pricing is configured:
```json
{
  "id": "chatcmpl-abc123",
  "model": "gpt-4o-mini-2024-07-18",
  "usage": {
    "prompt_tokens": 15,
    "completion_tokens": 20,
    "total_tokens": 35,
    "prompt_tokens_details": {
      "cached_tokens": 10
    }
  }
}
```

Check response headers:
```
X-Route-Id: route_xyz789
X-Resolved-Model: gpt-4o-mini-2024-07-18
X-Policy-Rev: 42
```


# Status Codes

- 200 OK – Success
- 400 Bad Request – Invalid input (e.g., malformed JSON, invalid parameters)
- 401 Unauthorized – Missing/invalid/revoked/expired token
- 502 Bad Gateway – Upstream error or connectivity issue
- 503 Service Unavailable – Dependent component unavailable (e.g., key manager or analytics disabled)


# Environment Variables (selected)

- OPENAI_API_KEY – Enables managed mode; used as default upstream key if not overridden by routing.
- ROUTIIUM_BACKENDS – Multi-backend routing config; see "Routing and Multi-backend".
- ROUTIIUM_KEYS_REQUIRE_EXPIRATION – Require expiration when generating keys ("1|true|yes|on").
- ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS – Default TTL for key generation.
- ROUTIIUM_PRICING_CONFIG – Optional pricing JSON file to enable cost tracking aligned with your provider list.
- ROUTIIUM_ROUTER_URL – Base URL for remote Router API (enables Router-based routing).
- ROUTIIUM_ROUTER_TIMEOUT_MS – HTTP timeout for Router calls (default: 15ms).
- ROUTIIUM_ROUTER_PRIVACY_MODE – Content sharing level: features, summary, or full (default: features).
- ROUTIIUM_ROUTER_STRICT – Fail requests if routing fails (1|true|yes|on).
- ROUTIIUM_CACHE_TTL_MS – Cache horizon for Router plans (default: 15000ms).
- ROUTIIUM_ANALYTICS_JSONL_PATH – Path to JSONL analytics log (default: ./data/analytics.jsonl).
- ROUTIIUM_ANALYTICS_REDIS_URL – Redis URL for analytics storage.
- ROUTIIUM_ANALYTICS_SLED_PATH – Sled database path for analytics.
- ROUTIIUM_ANALYTICS_TTL_SECONDS – TTL for analytics events in Redis/Sled.
- ROUTIIUM_ANALYTICS_FORCE_MEMORY – Use in-memory analytics (dev only).
- ROUTIIUM_ANALYTICS_MAX_EVENTS – Max events in memory mode.

Notes:
- In managed mode, an Authorization bearer is mandatory and is validated; the upstream provider key is selected by routing (key_env if configured, else OPENAI_API_KEY).
- In passthrough mode, the client must send a valid upstream provider key as the bearer.


# Compatibility

- Works with OpenAI native endpoints.
- For local backends (vLLM, Ollama, etc.) that expose Chat Completions only, set mode=chat for their model prefixes; non-stream Responses POSTs get translated upstream automatically.


# Change Log

See README.md and release notes for additions to endpoints and behavior.
