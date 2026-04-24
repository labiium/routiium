# Routiium Router Usage Guide
## CLI shortcuts

Use the Routiium CLI for the common rollout loop:

```bash
routiium init --profile openai --out .env
routiium router explain --model auto --prompt "Ignore previous instructions"
routiium judge explain --prompt "Ignore previous instructions"
routiium judge test --suite all
routiium router probe --model auto
```

The CLI does not replace the Router API contract below; it gives operators a faster way to inspect embedded routing and verify remote Routiium-to-Router paths.


The router layer lets Routiium resolve human-friendly model aliases into concrete upstream endpoints and policies. When no remote Router is configured, Routiium uses its embedded EduRouter-style policy router and safety judge by default. This guide explains how routing decisions are made, which configuration hooks are available, how to wire everything up in Docker, and how to verify and troubleshoot router integration.

**Table of Contents:**
- [How Routing Works](#1-how-routing-works)
- [Router Modes](#2-router-modes)
- [Request Privacy Levels](#3-request-privacy-levels)
- [Plans, Caching, and Headers](#4-plans-caching-and-headers)
- [Setup Recipes](#5-setup-recipes)
- [Docker & Docker Compose](#6-docker--docker-compose)
- [Verification & Troubleshooting](#7-verification--troubleshooting)
- [Practical Examples](#8-practical-examples)
- [Router + Analytics Integration](#9-router--analytics-integration)
- [Reference: Key Router Environment Variables](#10-reference-key-router-environment-variables)

---

## 1. How Routing Works

1. Every inbound `/v1/responses`, `/v1/chat/completions`, or `/convert` call flows through `resolve_upstream` (`src/server.rs`).  
2. Routiium chooses a `RouterClient`: explicit `--router-config`, explicit `ROUTIIUM_ROUTER_URL`, or the embedded default router. It builds a `RouteRequest` from the payload using `extract_route_request` (`src/router_client.rs`). That request includes:
   - The alias the client asked for (`body.model`).
   - API surface (`responses` or `chat`).
   - Capability flags (text, tools, vision, etc.).
   - Temperature/JSON mode hints and rough token estimates.
   - Optional conversation signals whose detail level is controlled by `ROUTIIUM_ROUTER_PRIVACY_MODE`.
3. The router returns a `RoutePlan` describing the target upstream (`base_url`, `mode`, `model_id`, optional `auth_env`, headers, limits, cache TTL, policy revision, stickiness token, etc.).  
4. Routiium forwards the request upstream using that plan, adds observability headers (e.g. `x-route-id`, `x-resolved-model`, `router-schema`), and submits router feedback when supported.
5. Embedded safety denials never fall back to legacy routing. Remote router failures can fall back only when strict mode is disabled.

The Router contract is documented in detail in [`ROUTER_API_SPEC.md`](ROUTER_API_SPEC.md); `../examples/router_service.rs` is a runnable reference implementation.

---

## 2. Router Modes

| Mode | How to enable | When to use |
| ---- | ------------- | ----------- |
| **Embedded default router** | Default when no explicit router is configured (`ROUTIIUM_ROUTER_MODE=embedded`) | Safe-by-default single-binary deployments with aliases, scoring, and request judging. |
| **Local alias map** | `routiium --router-config=router_aliases.json` | Simple deployments where a static JSON map is sufficient. |
| **Remote HTTP router** | Set `ROUTIIUM_ROUTER_URL=https://router.yourdomain/` (optional `ROUTIIUM_ROUTER_TIMEOUT_MS`, `ROUTIIUM_ROUTER_MTLS`, etc.) | Dynamic policies, catalog metadata, and multi-tenant routing. |
| **Legacy prefix fallback** | Set `ROUTIIUM_ROUTER_MODE=off` and configure `ROUTIIUM_BACKENDS` | Emergency fallback or ultra-simple setups without embedded judge. |

`--router-config` takes precedence over `ROUTIIUM_ROUTER_URL`; both take precedence over the embedded router. Set `ROUTIIUM_ROUTER_MODE=off` to disable embedded routing intentionally.

---

## 3. Request Privacy Levels

`ROUTIIUM_ROUTER_PRIVACY_MODE` controls how much of the conversation is sent to the router:

| Value | Description |
| ----- | ----------- |
| `features` | Sends metadata only (modalities, tool usage, token estimates). |
| `summary` | Adds a short summary of the latest user message. |
| `full` | Includes the system prompt and the last five turns so routers can enforce richer policies. |

The router’s `RoutePlan.content_used` field (and the `X-Content-Used` response header) records what the router actually consumed for auditing. Embedded mode defaults to `full` because the judge runs in-process; remote mode should prefer `features` unless the remote policy needs content.

---

## 4. Plans, Caching, and Headers

- Each `RoutePlan` carries cache metadata (`cache.ttl_ms`, `cache.valid_until`, `cache.freeze_key`). Routiium also exposes `ROUTIIUM_CACHE_TTL_MS` to override the default 15 s cache horizon for remote routers.  
- Embedded judged requests set `cache.ttl_ms: 0` whenever content-sensitive safety decisions are involved. For remote per-request LLM judging, configure the Router to return `cache.ttl_ms: 0` (or set `ROUTIIUM_CACHE_TTL_MS=0`) so each routed request reaches the judge.
- Plans that include `stickiness.plan_token` cause Routiium to send that token back to the router on the next turn so multi-turn conversations stay on the same upstream.  
- Observability headers forwarded to clients:
  - `x-route-id`: Router-generated identifier (helps correlate downstream logs).
  - `x-resolved-model`: Actual upstream model ID.
  - `x-policy-rev` and `router-schema`: Policy metadata + schema version.
  - `x-content-used`: Privacy attestation from the router.
  - `x-judge-id`, `x-judge-mode`, `x-judge-verdict`, `x-judge-risk`, `x-judge-target`: judge observability.
  - `x-judge-action` and `x-judge-policy-fingerprint`: normalized judge action and policy overlay fingerprint.
  - `x-safety-policy-rev`, `x-safety-cache`: embedded/remote safety policy metadata.
  - `x-route-cache`: `hit`, `miss`, or `stale` when the router exposed cache hints.
- When strict mode is disabled (default), failed router lookups fall back to `ROUTIIUM_BACKENDS`. Enabling `ROUTIIUM_ROUTER_STRICT=1` preserves structured Router error statuses (for example `403 POLICY_DENY` or `409 NO_ROUTE`) so callers notice misconfigured aliases and policy denials immediately.

### Strict vs non-strict routing policy

| Setting | Router unavailable | Router policy denial | Judge enforcement guarantee | Recommended use |
| --- | --- | --- | --- | --- |
| `ROUTIIUM_ROUTER_STRICT=1` | Client receives structured router error | Client receives policy denial | Enforceable when Router returns zero-TTL judged plans | Judge/policy-sensitive production |
| unset / `0` | Routiium may fall back to legacy routing | Routiium may fall back where safe | Not guaranteed; judge may be bypassed by fallback | Development or best-effort routing |


### 4.1 Built-in and remote LLM-as-judge

Routiium now includes a built-in request judge in the embedded router. It runs deterministic checks for prompt injection, exfiltration, risky tools, dangerous actions, and suspicious URLs; its response guard scans outputs for prompt/secret leakage and dangerous guidance; when configured with a provider key, it can also call an isolated LLM judge with redacted context. Sensitive-but-allowable requests route to the built-in `secure` alias. Remote Router/EduRouter deployments can still return judge metadata through the same `RoutePlan.judge` field.

**Guarantee checklist: every routed request is judged**

- Keep embedded routing enabled, or point `ROUTIIUM_ROUTER_URL` at a judging Router.
- Use `ROUTIIUM_ROUTER_STRICT=1` so policy failures do not silently fall back.
- Use `ROUTIIUM_ROUTER_PRIVACY_MODE=full` only when the judge needs request content.
- Ensure content-sensitive judged decisions have `cache.ttl_ms: 0`.
- Review `x-judge-*`, `x-response-guard-*`, `x-streaming-safety`, and `x-safety-*` headers in probes.

**Bundled example Router profiles**

```bash
# Try safely: judge runs, but does not block or downgrade.
ROUTER_JUDGE_MODE=shadow
ROUTER_JUDGE_CONTEXT=full
ROUTER_JUDGE_FAILURE=allow

# Production enforce: judge can allow, downgrade, or deny.
ROUTER_JUDGE_MODE=enforce
ROUTER_JUDGE_CONTEXT=full
ROUTER_JUDGE_FAILURE=deny

# Soft-degrade enforce: judge outage routes to a safe model instead of failing.
ROUTER_JUDGE_MODE=enforce
ROUTER_JUDGE_FAILURE=safe_model
ROUTER_JUDGE_SAFE_MODEL=gpt-4o-mini-2024-07-18
```

Judge provider configuration for the example Router:

```bash
ROUTER_JUDGE_BASE_URL=https://api.openai.com/v1
ROUTER_JUDGE_MODEL=gpt-4o-mini
ROUTER_JUDGE_API_KEY_ENV=OPENAI_API_KEY
ROUTER_JUDGE_TIMEOUT_MS=800
```

Routiium side:

```bash
ROUTIIUM_ROUTER_URL=http://router:9090
ROUTIIUM_ROUTER_STRICT=1
ROUTIIUM_ROUTER_PRIVACY_MODE=full
ROUTIIUM_CACHE_TTL_MS=0
```

---

## 5. Setup Recipes

### 5.1 Local Alias Map

1. Copy `router_aliases.json.example` to `router_aliases.json` and edit each alias block:
   ```jsonc
   {
     "edu-fast": {
       "base_url": "https://api.openai.com/v1",
       "mode": "responses",
       "model_id": "gpt-4o-mini-2024-07-18",
       "auth_env": "OPENAI_API_KEY"
     }
   }
   ```
   - `mode` must be `responses` or `chat`.
   - `auth_env` tells Routiium which environment variable holds the provider key.
2. Launch Routiium with `--router-config=/path/to/router_aliases.json`.  
3. Hit `/status` and confirm `router` shows `local policy`.  

> Local alias maps are static; restart Routiium after editing the JSON file.

### 5.2 Remote Router Service

1. Run or deploy a Router that follows `ROUTER_API_SPEC.md`. You can start the built-in example locally:
   ```bash
   cargo run --example router_service
   ```
   This serves `/route/plan`, `/route/feedback`, and `/catalog/models` on `http://127.0.0.1:9090`.

2. Point Routiium at it:
   ```bash
   ROUTIIUM_ROUTER_URL=http://127.0.0.1:9090 \
   ROUTIIUM_ROUTER_TIMEOUT_MS=50 \
   ROUTIIUM_CACHE_TTL_MS=60000 \
   routiium --system-prompt-config=system_prompt.json
   ```

3. Optional env knobs:
   - `ROUTIIUM_ROUTER_STRICT=1` – fail the request if the router rejects an alias.
   - `ROUTIIUM_ROUTER_MTLS=1` – enable mutual TLS (expect OS-level certs).
   - `ROUTIIUM_ROUTER_TIMEOUT_MS` – per-request timeout (ms).
   - `ROUTIIUM_CACHE_TTL_MS` – maximum cache TTL (ms) for remote plans.
   - `ROUTIIUM_ROUTER_PRIVACY_MODE` – content sharing level (features, summary, full).

4. Verify the connection:
   ```bash
   # Check status endpoint
   curl http://localhost:8088/status | jq '.router'
   
   # Expected output:
   {
     "mode": "remote",
     "url": "http://127.0.0.1:9090",
     "strict": false,
     "cache_ttl_ms": 60000,
     "privacy_mode": "features"
   }
   ```

5. Test with a request:
   ```bash
   # Send a request using a router alias
   curl -X POST http://localhost:8088/v1/chat/completions \
     -H "Authorization: Bearer sk_test.abc123" \
     -H "Content-Type: application/json" \
     -d '{
       "model": "edu-fast",
       "messages": [{"role":"user","content":"Hello"}]
     }' -i
   
   # Check response headers for router metadata:
   # X-Route-Id: route_abc123xyz
   # X-Resolved-Model: gpt-4o-mini-2024-07-18
   # Router-Schema: 1.1
   ```

Use the response headers or `/status` endpoint to verify the connection. Router outages produce `WARN` logs; combine with strict mode to surface issues quickly.

---

## 6. Docker & Docker Compose

### 6.1 Local Alias Mode in Docker

1. Copy your alias file into the repo root (e.g. `router_aliases.json`).  
2. Mount it read-only and pass the flag via Compose:

```yaml
services:
  routiium:
    build: .
    env_file: .env
    command: ["--router-config=/app/router_aliases.json","--system-prompt-config=/app/system_prompt.json"]
    volumes:
      - routiium-data:/data
      - ./system_prompt.json:/app/system_prompt.json:ro
      - ./router_aliases.json:/app/router_aliases.json:ro
```

The container reads aliases at startup; restart it when you change the file.

### 6.2 Remote Router Mode in Docker

Add a router service (either your own implementation or the provided example) and point Routiium at it via env vars:

```yaml
services:
  router:
    build:
      context: .
      dockerfile: Dockerfile.router  # build your router image (example below)
    ports:
      - "9090:9090"

  routiium:
    build: .
    depends_on:
      - router
    env_file: .env
    environment:
      ROUTIIUM_ROUTER_URL: "http://router:9090"
      ROUTIIUM_ROUTER_TIMEOUT_MS: "50"
      ROUTIIUM_ROUTER_PRIVACY_MODE: "features"
      ROUTIIUM_ROUTER_STRICT: "1"
      ROUTIIUM_CACHE_TTL_MS: "60000"
    command: ["--system-prompt-config=/app/system_prompt.json"]
    volumes:
      - routiium-data:/data
      - ./system_prompt.json:/app/system_prompt.json:ro
```

To containerize the example router, you can reuse the Rust builder pattern:

```dockerfile
# Dockerfile.router
FROM rust:1.82-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY examples ./examples
RUN cargo build --release --example router_service

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /build/target/release/examples/router_service /usr/local/bin/router_service
EXPOSE 9090
ENTRYPOINT ["router_service"]
```

Expose the router on the same Docker network so Routiium can reach `http://router:9090`.

---

## 7. Verification & Troubleshooting

### 7.1 Basic Verification

**Check Router Status:**
```bash
curl http://localhost:8088/status | jq '.router'
```

Expected output (remote router):
```json
{
  "mode": "remote",
  "url": "http://router:9090",
  "strict": false,
  "cache_ttl_ms": 60000,
  "privacy_mode": "features"
}
```

Expected output (local aliases):
```json
{
  "mode": "local",
  "policy": "file:///app/router_aliases.json",
  "aliases_count": 5
}
```

**Inspect Response Headers:**
Every router-resolved request includes these headers:
```bash
curl -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_test.abc" \
  -H "Content-Type: application/json" \
  -d '{"model":"edu-fast","messages":[{"role":"user","content":"test"}]}' \
  -i | grep -i "x-route\|router-schema"

# Expected headers:
# X-Route-Id: route_01JQ2K2C7Y3X
# X-Resolved-Model: gpt-4o-mini-2024-07-18
# X-Policy-Rev: 42
# X-Content-Used: features_only
# X-Route-Cache: hit
# Router-Schema: 1.1
```

**Missing headers** indicate fallback to legacy routing was used.

### 7.2 Common Issues

#### Issue: "Router plan unavailable… falling back to legacy routing"

**Symptoms:**
- Logs show fallback warnings
- Response headers missing `X-Route-Id`
- Requests work but use `ROUTIIUM_BACKENDS` or global upstream

**Diagnosis:**
```bash
# 1. Check router connectivity
curl http://router:9090/capabilities

# 2. Test direct router API call
curl -X POST http://router:9090/route/plan \
  -H "Content-Type: application/json" \
  -d '{
    "schema_version":"1.1",
    "alias":"edu-fast",
    "api":"responses",
    "caps":["text"]
  }'

# 3. Check Routiium logs
docker logs routiium | grep -i router

# 4. Verify environment variables
docker exec routiium env | grep ROUTIIUM_ROUTER
```

**Solutions:**
- Verify `ROUTIIUM_ROUTER_URL` is reachable from Routiium container
- Check router service is running: `docker ps | grep router`
- Ensure network connectivity: add both to same Docker network
- Verify router schema compatibility (should be 1.1)
- Check router logs for errors

#### Issue: "Unknown model alias" or 404 from router

**Symptoms:**
- Router returns 404 or error
- Logs show "Router rejected alias"
- With `ROUTIIUM_ROUTER_STRICT=1`: Routiium preserves the Router's structured status/body, such as `404 ALIAS_UNKNOWN` or `409 NO_ROUTE`

**Diagnosis:**
```bash
# 1. List available models in router catalog
curl http://router:9090/catalog/models | jq '.models[].id'

# 2. Check what alias you're requesting
curl http://localhost:8088/analytics/events?limit=10 | \
  jq '.events[].request.model'

# 3. Query router directly with your alias
curl -X POST http://router:9090/route/plan \
  -H "Content-Type: application/json" \
  -d '{
    "schema_version":"1.1",
    "alias":"your-alias-here",
    "api":"responses",
    "caps":["text"]
  }' | jq
```

**Solutions:**
- Update router aliases to include the requested model
- Fix client code to use correct alias names
- For local router: edit `router_aliases.json` and restart
- For remote router: update router service configuration

#### Issue: High latency or timeout errors

**Symptoms:**
- Slow response times
- Timeout errors in logs
- `ROUTIIUM_ROUTER_TIMEOUT_MS` errors

**Diagnosis:**
```bash
# 1. Check router response time
time curl -X POST http://router:9090/route/plan \
  -H "Content-Type: application/json" \
  -d '{
    "schema_version":"1.1",
    "alias":"edu-fast",
    "api":"responses",
    "caps":["text"]
  }'

# 2. Check cache configuration
curl http://localhost:8088/status | jq '.router | {strict, cache_ttl_ms, privacy_mode}'

# 3. Monitor router performance
curl http://localhost:8088/analytics/aggregate | \
  jq '{avg_duration_ms, total_requests}'
```

**Solutions:**
- Increase `ROUTIIUM_ROUTER_TIMEOUT_MS` (default: 15ms, try 50-100ms)
- Increase `ROUTIIUM_CACHE_TTL_MS` to reduce router calls (default: 15000ms)
- Ensure router and Routiium are in same AZ/region
- Optimize router service performance
- Enable router plan caching with appropriate TTL

#### Issue: Cached plans not being used

**Symptoms:**
- High cache miss rate
- Every request shows `X-Route-Cache: miss`
- Poor performance despite caching enabled

**Diagnosis:**
```bash
# Check cache stats
curl http://localhost:8088/status | jq '.router'

# Verify cache headers in responses
curl -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_test.abc" \
  -H "Content-Type: application/json" \
  -d '{"model":"edu-fast","messages":[{"role":"user","content":"test"}]}' \
  -i | grep X-Route-Cache

# Make multiple identical requests
for i in {1..5}; do
  curl -X POST http://localhost:8088/v1/chat/completions \
    -H "Authorization: Bearer sk_test.abc" \
    -H "Content-Type: application/json" \
    -d '{"model":"edu-fast","messages":[{"role":"user","content":"test $i"}]}' \
    -i 2>&1 | grep X-Route-Cache
done
```

**Solutions:**
- Verify `ROUTIIUM_CACHE_TTL_MS` is set and reasonable (60000 = 1 minute)
- Check router returns valid `ttl_ms` in RoutePlan
- Ensure cache key factors are stable (same alias, api, basic params)
- Router plan `freeze_key` changes invalidate cache

#### Issue: Privacy mode not working as expected

**Symptoms:**
- Router receives more/less content than expected
- `X-Content-Used` header shows unexpected value

**Diagnosis:**
```bash
# Check current privacy mode
curl http://localhost:8088/status | jq '.router.privacy_mode'

# Test with different modes
ROUTIIUM_ROUTER_PRIVACY_MODE=full routiium &
curl -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_test.abc" \
  -H "Content-Type: application/json" \
  -d '{"model":"edu-fast","messages":[{"role":"user","content":"test"}]}' \
  -i | grep X-Content-Used
```

**Solutions:**
- Set `ROUTIIUM_ROUTER_PRIVACY_MODE` to desired level: `features`, `summary`, or `full`
- Restart Routiium after changing environment variables
- Verify router logs to see what content it receives
- Check `RouteRequest.content_attestation` in router logs

### 7.3 Monitoring Best Practices

**Enable Strict Mode in Staging:**
```bash
ROUTIIUM_ROUTER_STRICT=1
```
This makes routing failures explicit, helping catch configuration issues early.

**Disable Strict Mode in Production:**
```bash
# Unset or set to 0
ROUTIIUM_ROUTER_STRICT=0
```
Allows graceful fallback to legacy routing if router is unavailable.

**Monitor Router Health:**
```bash
# Add to monitoring script
curl -f http://router:9090/capabilities || alert "Router down"

# Check Routiium router policy posture
curl -s http://localhost:8088/status | \
  jq '.router | {strict, cache_ttl_ms, privacy_mode}'
```

**Alert on Fallback Usage:**
```bash
# Monitor logs for fallback warnings
docker logs routiium 2>&1 | grep -i "falling back to legacy routing" && \
  alert "Router fallback detected"
```

### 7.4 Debug Checklist

- [ ] Router service is running: `curl http://router:9090/capabilities`
- [ ] Routiium can reach router: test from Routiium container
- [ ] `ROUTIIUM_ROUTER_URL` is set correctly
- [ ] Router aliases are configured for requested models
- [ ] Response headers include `X-Route-Id` and `X-Resolved-Model`
- [ ] Cache hit ratio is reasonable (>50% for repeated requests)
- [ ] Privacy mode matches requirements
- [ ] Provider API keys are available (check `auth_env` in plans)
- [ ] Router schema version matches (1.1)
- [ ] No timeout errors in logs

---

## 8. Practical Examples

### 8.1 Testing Router Integration

**Basic alias resolution:**
```bash
# Request using alias
curl -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_test.abc123" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "edu-fast",
    "messages": [
      {"role": "user", "content": "What is HTTP/2?"}
    ]
  }' | jq

# Check resolved model in response headers
# X-Resolved-Model: gpt-4o-mini-2024-07-18
```

**Streaming with router:**
```bash
curl -N -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_test.abc123" \
  -H "Content-Type: application/json" \
  -H "Accept: text/event-stream" \
  -d '{
    "model": "edu-premium",
    "stream": true,
    "messages": [
      {"role": "user", "content": "Write a haiku about routing"}
    ]
  }'
```

**Using conversation stickiness:**
```bash
# First turn (router returns plan_token in response)
curl -X POST http://localhost:8088/v1/responses \
  -H "Authorization: Bearer sk_test.abc123" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "edu-fast",
    "conversation": {"id": "conv_123"},
    "input": [
      {"role": "user", "content": "Hello, remember this: X=42"}
    ]
  }' -i

# Note the X-Route-Id in response headers
# X-Route-Id: route_abc123xyz

# Second turn (uses stickiness to same upstream)
curl -X POST http://localhost:8088/v1/responses \
  -H "Authorization: Bearer sk_test.abc123" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "edu-fast",
    "conversation": {"id": "conv_123"},
    "input": [
      {"role": "user", "content": "What was X?"}
    ],
    "previous_response_id": "resp_from_first_turn"
  }' -i

# Should show same X-Resolved-Model as first turn
```

### 8.2 Router Catalog Queries

**List available models:**
```bash
curl http://router:9090/catalog/models | jq '.models[] | {
  id,
  provider,
  aliases,
  status,
  cost: .cost | {input_per_million, output_per_million}
}'
```

**Filter by capability:**
```bash
# Find models with vision support
curl http://router:9090/catalog/models | jq '.models[] | 
  select(.capabilities.modalities | contains(["image"])) | 
  {id, modalities: .capabilities.modalities}'
```

**Check model health:**
```bash
curl http://router:9090/catalog/models | jq '.models[] | 
  select(.status != "healthy") | 
  {id, status, status_reason}'
```

### 8.3 Privacy Mode Examples

**Features only (default):**
```bash
ROUTIIUM_ROUTER_PRIVACY_MODE=features

# Router receives only metadata: caps, token estimates, modalities
# No message content sent to router
```

**Summary mode:**
```bash
ROUTIIUM_ROUTER_PRIVACY_MODE=summary

# Router receives short summary of latest user message
# Useful for content-aware routing without full content
```

**Full mode:**
```bash
ROUTIIUM_ROUTER_PRIVACY_MODE=full

# Router receives system prompt and last 5 turns
# Use only when router needs full context for policy enforcement
```

**Verify privacy level:**
```bash
curl -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_test.abc" \
  -H "Content-Type: application/json" \
  -d '{"model":"edu-fast","messages":[{"role":"user","content":"test"}]}' \
  -i | grep X-Content-Used

# Expected: X-Content-Used: features_only
```

### 8.4 Cost-Aware Routing

**Router can enforce budget limits:**
```bash
# Router rejects if estimated cost exceeds budget
curl -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_test.abc" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "edu-premium",
    "messages": [
      {"role": "user", "content": "'"$(cat large_document.txt)"'"}
    ]
  }'

# Router may return 429 with retry_hint_ms if over budget
```

**Check cost in analytics:**
```bash
# After router-routed requests with cost tracking
curl http://localhost:8088/analytics/aggregate | jq '{
  total_cost,
  cost_by_model,
  avg_cost_per_request: (.total_cost / .total_requests)
}'
```

### 8.5 Testing Fallback Behavior

**Test with router down (strict mode disabled):**
```bash
# Stop router
docker stop router

# Request should fallback to ROUTIIUM_BACKENDS
curl -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_test.abc" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role":"user","content":"test"}]
  }' | jq

# Check logs for fallback message
docker logs routiium 2>&1 | tail -20 | grep "falling back"
```

**Test with strict mode enabled:**
```bash
# Enable strict mode
docker exec routiium sh -c 'export ROUTIIUM_ROUTER_STRICT=1'

# Request should fail instead of falling back when router is down
curl -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_test.abc" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "edu-fast",
    "messages": [{"role":"user","content":"test"}]
  }' | jq

# Expected: 502 Bad Gateway for connectivity failures, or the Router's structured
# status/body for reachable policy errors such as 403 POLICY_DENY.
```

---

## 9. Router + Analytics Integration

### 9.1 Tracking Router Usage

**Router resolution analytics:**
```bash
# Get models resolved by router
curl http://localhost:8088/analytics/events?limit=100 | jq '
  .events[] | 
  {
    requested: .request.model,
    resolved: (.routing.backend + "/" + .request.model),
    cache_status: .routing.cache_status
  }
'
```

**Cache configuration check:**
```bash
# Routiium exposes configured router cache TTL, not hit/miss counters.
curl http://localhost:8088/status | jq '.router | {cache_ttl_ms, strict, privacy_mode}'
```

Use request analytics and Router-side telemetry for cache hit-rate analysis.

### 9.2 Cost Analysis with Router

**Per-alias cost tracking:**
```bash
# Export analytics and group by original alias
curl "http://localhost:8088/analytics/export?format=csv" -o analytics.csv

# Analyze with awk/pandas
awk -F',' 'NR>1 {alias[$4]; cost[$4]+=$21} END {
  for (a in alias) print a, cost[a]
}' analytics.csv
```

**Router-driven cost optimization:**
```python
import requests
from collections import defaultdict

def analyze_router_cost_decisions():
    """Analyze if router is making cost-effective decisions"""
    resp = requests.get("http://localhost:8088/analytics/events?limit=1000")
    events = resp.json()['events']
    
    alias_costs = defaultdict(lambda: {'count': 0, 'total_cost': 0})
    
    for event in events:
        if event.get('cost'):
            alias = event['request']['model']
            alias_costs[alias]['count'] += 1
            alias_costs[alias]['total_cost'] += event['cost']['total_cost']
    
    print("Router Alias Cost Analysis:")
    print("=" * 60)
    for alias, stats in sorted(alias_costs.items()):
        avg_cost = stats['total_cost'] / stats['count']
        print(f"{alias:30} ${stats['total_cost']:8.4f} "
              f"({stats['count']:5} req, ${avg_cost:.6f}/req)")

analyze_router_cost_decisions()
```

### 9.3 Performance Monitoring

**Router latency impact:**
```bash
# Compare request durations with/without router
curl http://localhost:8088/analytics/aggregate | jq '{
  avg_duration_ms,
  avg_upstream_duration_ms,
  router_overhead_ms: (.avg_duration_ms - .avg_upstream_duration_ms)
}'
```

**Identify slow routes:**
```bash
curl http://localhost:8088/analytics/events?limit=500 | jq '
  .events[] | 
  select(.performance.duration_ms > 3000) |
  {
    model: .request.model,
    backend: .routing.backend,
    duration_ms: .performance.duration_ms
  }
'
```

---

## 10. Reference: Key Router Environment Variables

| Env var | Default | Purpose |
| ------- | ------- | ------- |
| `ROUTIIUM_ROUTER_URL` | unset | Base URL for the remote Router API (`http(s)://...`). |
| `ROUTIIUM_ROUTER_TIMEOUT_MS` | `15` | HTTP timeout (ms) for `/route/plan` & `/catalog/models`. |
| `ROUTIIUM_ROUTER_PRIVACY_MODE` | `features` | Controls how much conversation content is sent to the router (`features`, `summary`, `full`). |
| `ROUTIIUM_ROUTER_STRICT` | unset | When truthy (`1`, `true`, `yes`, `on`), fail client requests if routing fails. |
| `ROUTIIUM_ROUTER_MTLS` | unset | Enable mutual TLS for router calls (certs must already exist on the host). |
| `ROUTIIUM_CACHE_TTL_MS` | `15000` | Cache horizon for router plans when using `HttpRouterClient`. |
| `ROUTIIUM_BACKENDS` | unset | Semicolon-separated fallback rules (`prefix=edu,base=https://...,key_env=OPENAI_API_KEY,mode=responses`). |
| `ROUTER_JUDGE_MODE` | `off` | Example Router judge mode (`off`, `shadow`, `enforce`). |
| `ROUTER_JUDGE_CONTEXT` | `full` | Example Router judge context profile (`features`, `summary`, `full`). |
| `ROUTER_JUDGE_FAILURE` | `deny` | Example Router behavior when the judge is unavailable (`allow`, `deny`, `safe_model`). |
| `ROUTER_JUDGE_MODEL` | `gpt-4o-mini` | Example Router LLM judge model. |
| `ROUTER_JUDGE_SAFE_MODEL` | `gpt-4o-mini-2024-07-18` | Safe model used by `ROUTER_JUDGE_FAILURE=safe_model`. |

Keep provider keys (e.g., `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GROQ_API_KEY`) available in the environment so router plans referencing `auth_env` succeed.

---

With this configuration surface you can start with a static alias map, grow into a remote policy service, and still keep clear observability and fallback behaviour in Docker or bare-metal deployments.***
