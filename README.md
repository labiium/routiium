# Routiium

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

You have applications calling OpenAI's Chat Completions API. You want to issue API keys to customers, route traffic across providers, enforce rate limits, track per-request costs, and inject tools and system prompts — all without changing your application code.

**Routiium is a self-hosted LLM reverse proxy.** Drop it between your clients and any LLM provider. Your existing Chat Completions code keeps working while Routiium handles auth, routing, limiting, and observability transparently.

```
Your App (any OpenAI-compatible SDK)
               │
       ┌───────┴───────┐
       │   Routiium    │  ← auth, rate limits, system prompts,
       │   :8088       │    MCP tools, analytics, cost tracking
       └───┬───┬───┬───┘
           │   │   │
     OpenAI  Anthropic  AWS Bedrock / vLLM / Ollama
```

## Quick Start

```bash
git clone https://github.com/labiium/routiium.git && cd routiium

# Set your upstream provider key
export OPENAI_API_KEY=sk-your-key

cargo run --release
```

Routiium is now proxying on `localhost:8088`. Try it:

```bash
# Generate a managed API key
curl -s http://localhost:8088/keys/generate \
  -H "Content-Type: application/json" \
  -d '{"label":"my-first-key"}' | jq .

# Use the returned sk_<id>.<secret> token to call the proxy
curl -N http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_<id>.<secret>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4.1-nano",
    "messages": [{"role": "user", "content": "Hello!"}],
    "stream": true
  }'
```

Your client never sees `OPENAI_API_KEY`. Routiium validates the issued token, substitutes the real key upstream, records analytics, and returns the response.

### Docker

```bash
docker build -t routiium .
docker run --rm -p 8088:8088 -e OPENAI_API_KEY=sk-your-key routiium
```

Or with the included `docker-compose.yml` (hardened: read-only rootfs, dropped capabilities, no-new-privileges):

```bash
cp .env.example .env       # add your provider keys
docker compose up --build
```

## Why Routiium

### "I want to give customers API keys without exposing my provider secrets"

Routiium issues opaque tokens (`sk_<id>.<secret>`) and validates them on every request. Provider keys stay on the server. You control issuance, revocation, expiration, and scopes through a REST API — no redeploy needed.

### "I want to route `gpt-4` to OpenAI but `claude-3` to Anthropic from the same client"

Configure `ROUTIIUM_BACKENDS` with prefix rules and Routiium selects the right upstream automatically. Clients send any model name; the proxy resolves the provider, swaps in the correct auth, and converts payloads if needed (Chat Completions, Responses API, or Bedrock SigV4).

```bash
export ROUTIIUM_BACKENDS="prefix=gpt-,base=https://api.openai.com/v1,key_env=OPENAI_API_KEY; \
  prefix=claude-,base=https://api.anthropic.com/v1,key_env=ANTHROPIC_API_KEY; \
  prefix=anthropic.,base=https://bedrock-runtime.us-east-1.amazonaws.com,mode=bedrock"
```

### "I want to know how much each customer is spending"

Every request is logged with token usage, model, routing decision, auth identity, and computed cost. Query the analytics API or export as CSV. Bring your own pricing cards or use the built-in OpenAI defaults.

### "I need per-customer rate limits without client-side logic"

Define policies with daily, per-minute, or custom-window buckets. Assign policies to keys via the admin API. Emergency-block abusive keys instantly. Everything is hot-reloadable — no restarts.

### "I want every request to include my company's system prompt and database tools"

System prompt injection (global, per-model, or per-API-mode) is applied transparently. MCP (Model Context Protocol) servers are spawned at boot and their tools are merged into every request — clients see the union of their declared tools plus any MCP-provided ones.

## Features

### Multi-Backend Routing

Route requests to any combination of providers based on model prefix, local policy files, or an external Router service (Schema 1.1 with caching, stickiness, and cost hints). Upstream modes:
- **Responses** (default) — native OpenAI Responses API
- **Chat** — rewrite to `/v1/chat/completions` for vLLM, Ollama, or any Chat-compatible endpoint
- **Bedrock** — AWS SigV4 signing for Claude, Llama, Titan, and other Bedrock models

See [docs/ROUTER_API_SPEC.md](docs/ROUTER_API_SPEC.md) and [docs/AWS_BEDROCK.md](docs/AWS_BEDROCK.md).

### Managed Authentication

- Tokens: `sk_<id>.<secret>` — secrets are never stored (salted SHA-256 hashes only)
- Backends: Redis, sled (embedded), or in-memory — auto-detected or overridden via `--keys-backend`
- In-process key cache for single-digit-microsecond verification (disable with `ROUTIIUM_KEYS_DISABLE_CACHE=1` for multi-node setups sharing Redis)
- Passthrough mode available: leave `OPENAI_API_KEY` unset and clients forward their own provider keys

### Rate Limiting

- Multi-bucket policies: daily, per-minute, custom time windows (fixed or sliding)
- In-process concurrency semaphores
- Emergency key blocking (instant 429, with optional duration and reason)
- Per-key policy assignment or global default
- Hot-reloadable config file and full admin API — no restarts

See [docs/RATE_LIMITS.md](docs/RATE_LIMITS.md).

### Analytics and Cost Tracking

Every request records: endpoint, model, status, auth identity, routing decision, token usage (prompt/completion/cached/reasoning), and computed cost.

- Storage: JSONL (default), Redis, Sled, or in-memory
- Query: `/analytics/events`, `/analytics/aggregate`, `/analytics/export?format=csv`
- Pricing: built-in OpenAI cards or custom JSON via `ROUTIIUM_PRICING_CONFIG`

See [docs/ANALYTICS.md](docs/ANALYTICS.md).

### System Prompts

JSON config with `global`, `per_model`, and `per_api` prompts. Injection modes: `prepend`, `append`, or `replace`. Hot-reloadable via `/reload/system_prompt`.

### Model Context Protocol (MCP)

Point `--mcp-config` at your MCP config and Routiium spawns each server, discovers its tools, and merges them into every request. Tool names are namespaced (`serverName_toolName`). Hot-reload via `/reload/mcp`.

### Payload Translation

Bidirectional conversion between Chat Completions and Responses API formats, preserving tools, multimodal content, streaming SSE events, logprobs, reasoning tokens, and token usage.

## Admin Panel

A React + Vite dashboard backed by Routiium's live admin APIs (not a mock). Run it from the repo root:

```bash
npm run admin:install && npm run admin:dev
```

The panel provides:
- API key management (generate, revoke, set expiration)
- Rate limit policy CRUD, per-key assignment, emergency blocks
- System prompt, MCP, and routing config editing with hot-apply
- Analytics dashboard with export
- Chat history inspection
- Read-only views of pricing, Bedrock detection, and environment settings

Set `ROUTIIUM_ADMIN_TOKEN` on the server and enter the matching bearer in the panel header.

## Repo Layout

```
src/           Rust server and library code
tests/         Integration and behavior tests
docs/          Protocol, analytics, hardening, and rate-limit references
apps/admin/    Admin panel (Vite + React)
```

## CLI Flags

| Flag | Description |
| ---- | ----------- |
| `--keys-backend=redis://...\|sled:<path>\|memory` | Override the API key store. |
| `--mcp-config=PATH` | Load MCP server definitions. |
| `--system-prompt-config=PATH` | Load system prompt injection rules. |
| `--router-config=PATH` | Load a local alias/policy file for routing. |
| `--routing-config=PATH` | Load routing JSON with runtime reload support. |
| `--rate-limit-config=PATH` | Load rate limit policy config. |

## Environment Variables

### Server

| Variable | Default | Description |
| -------- | ------- | ----------- |
| `BIND_ADDR` | `0.0.0.0:8088` | Listen address. |
| `OPENAI_API_KEY` | — | Enables managed auth; used as fallback upstream bearer. |
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | Default upstream base URL. |
| `MODEL` | — | Default model when the client omits `model`. |
| `ROUTIIUM_MANAGED_MODE` | auto | `managed\|force\|true` or `passthrough\|false`. |
| `ROUTIIUM_UPSTREAM_MODE` | `responses` | `responses` or `chat` (for vLLM/Ollama). |
| `ROUTIIUM_HTTP_TIMEOUT_SECONDS` | — | Upstream request timeout. |
| `RUST_LOG` | — | Tracing filter (e.g. `info,tower_http=info`). |

### Routing

| Variable | Default | Description |
| -------- | ------- | ----------- |
| `ROUTIIUM_BACKENDS` | — | Semicolon-separated backend rules (`prefix`, `base`, `key_env`, `mode`). |
| `ROUTIIUM_ROUTER_URL` | — | Enable HTTP Router client (Schema 1.1). |
| `ROUTIIUM_ROUTER_TIMEOUT_MS` | `15` | Router request timeout. |
| `ROUTIIUM_CACHE_TTL_MS` | `15000` | Router plan cache TTL. |
| `ROUTIIUM_ROUTER_PRIVACY_MODE` | `features` | `features\|summary\|full` — controls content sent to router. |
| `ROUTIIUM_ROUTER_STRICT` | — | Fail if router rejects (no legacy fallback). |

### Auth and Keys

| Variable | Default | Description |
| -------- | ------- | ----------- |
| `ROUTIIUM_REDIS_URL` | — | Redis URL for key store. |
| `ROUTIIUM_SLED_PATH` | `./data/keys.db` | Sled database path. |
| `ROUTIIUM_ADMIN_TOKEN` | — | Admin bearer token for protected endpoints. |
| `ROUTIIUM_KEYS_REQUIRE_EXPIRATION` | — | Require TTL on new keys. |
| `ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS` | — | Default TTL for new keys. |
| `ROUTIIUM_KEYS_DISABLE_CACHE` | — | Skip in-memory key cache. |

### Rate Limiting

| Variable | Default | Description |
| -------- | ------- | ----------- |
| `ROUTIIUM_RATE_LIMIT_BACKEND` | — | `redis://...`, `sled:/path`, or `memory`. |
| `ROUTIIUM_RATE_LIMIT_ENABLED` | `true` | Set `false` to disable. |
| `ROUTIIUM_RATE_LIMIT_DAILY` | — | Daily request limit (sliding window). |
| `ROUTIIUM_RATE_LIMIT_PER_MINUTE` | — | Per-minute request limit. |
| `ROUTIIUM_RATE_LIMIT_CONFIG` | — | Path to rate limit JSON config. |

### Analytics

| Variable | Default | Description |
| -------- | ------- | ----------- |
| `ROUTIIUM_ANALYTICS_JSONL_PATH` | `data/analytics.jsonl` | JSONL analytics file. |
| `ROUTIIUM_ANALYTICS_REDIS_URL` | — | Redis URL for analytics. |
| `ROUTIIUM_ANALYTICS_SLED_PATH` | — | Sled path for analytics. |
| `ROUTIIUM_ANALYTICS_TTL_SECONDS` | — | Auto-expire analytics entries. |
| `ROUTIIUM_PRICING_CONFIG` | — | Custom pricing JSON path. |

### Proxy and CORS

| Variable | Description |
| -------- | ----------- |
| `ROUTIIUM_NO_PROXY`, `ROUTIIUM_PROXY_URL`, `HTTP_PROXY`, `HTTPS_PROXY` | Proxy controls. |
| `CORS_ALLOWED_ORIGINS`, `CORS_ALLOWED_METHODS`, `CORS_ALLOWED_HEADERS` | CORS policy. |
| `CORS_ALLOW_CREDENTIALS`, `CORS_MAX_AGE` | CORS policy (cont.). |

Routiium also loads `.env`, `.envfile`, or any path set via `ENV_FILE` / `ENVFILE` / `DOTENV_PATH`.

## HTTP API Reference

### Proxy Endpoints

| Route | Auth |
| ----- | ---- |
| `GET /health` | None |
| `GET /status` | None |
| `GET /v1/models` | Bearer |
| `POST /v1/chat/completions` | Bearer |
| `POST /v1/responses` | Bearer |
| `POST /convert` | None |

### Key Management

| Route | Description |
| ----- | ----------- |
| `GET /keys` | List keys (supports `label`, `label_prefix`, `include_revoked`). |
| `POST /keys/generate` | Issue a new token (`label`, `ttl_seconds`, `expires_at`, `scopes`). |
| `POST /keys/generate_batch` | Issue multiple keys (`labels` array). |
| `POST /keys/revoke` | Revoke by id. |
| `POST /keys/set_expiration` | Set or clear expiration. |

### Admin (requires `ROUTIIUM_ADMIN_TOKEN`)

| Route | Description |
| ----- | ----------- |
| `GET/POST/PUT/DELETE /admin/rate-limits/policies[/{id}]` | Rate limit policy CRUD. |
| `GET/POST /admin/rate-limits/default` | Get/set default policy. |
| `GET/POST/DELETE /admin/rate-limits/keys/{key_id}` | Per-key policy assignment. |
| `GET/POST/DELETE /admin/rate-limits/emergency[/{key_id}]` | Emergency blocks. |
| `POST /admin/rate-limits/reload` | Hot-reload rate limit config. |
| `GET /admin/concurrency/keys/{key_id}` | Live concurrency counters. |
| `GET /admin/analytics/rate-limits` | Rate limit event metrics. |
| `GET /admin/panel/state` | Full runtime snapshot for the admin panel. |
| `PUT /admin/panel/system-prompts` | Persist and hot-apply system prompts. |
| `PUT /admin/panel/mcp` | Persist and reconnect MCP config. |
| `PUT /admin/panel/routing` | Persist and hot-apply routing config. |

### Analytics

| Route | Description |
| ----- | ----------- |
| `GET /analytics/stats` | Backend stats. |
| `GET /analytics/events` | Query events (`start`, `end`, `limit`). |
| `GET /analytics/aggregate` | Aggregated metrics. |
| `GET /analytics/export` | Export as JSON or CSV. |
| `POST /analytics/clear` | Wipe storage. |

### Config Reload

| Route | Description |
| ----- | ----------- |
| `POST /reload/mcp` | Reload MCP config. |
| `POST /reload/system_prompt` | Reload system prompts. |
| `POST /reload/routing` | Reload routing config. |
| `POST /reload/all` | Reload all configs. |

Full request/response documentation with curl examples: [docs/API_REFERENCE.md](docs/API_REFERENCE.md).

## Documentation

| Document | Description |
| -------- | ----------- |
| [docs/API_REFERENCE.md](docs/API_REFERENCE.md) | Full API documentation with curl snippets. |
| [docs/ANALYTICS.md](docs/ANALYTICS.md) | Analytics architecture and data model. |
| [docs/RATE_LIMITS.md](docs/RATE_LIMITS.md) | Rate limiting reference. |
| [docs/AWS_BEDROCK.md](docs/AWS_BEDROCK.md) | AWS Bedrock integration guide. |
| [docs/ROUTER_API_SPEC.md](docs/ROUTER_API_SPEC.md) | Router Schema 1.1 specification. |
| [docs/PRODUCTION_HARDENING.md](docs/PRODUCTION_HARDENING.md) | Deployment hardening checklist. |

## Development

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo test
```

HTTP smoke tests live in `python_tests/` — run with `pytest`.

### Responses CLI

Manual multi-turn sanity check against the streaming proxy:

```bash
ROUTIIUM_BASE=http://127.0.0.1:8088 python python_tests/chat_cli.py --model gpt-4.1-nano
```

### Key Generator CLI

Mint managed credentials from the command line:

```bash
ROUTIIUM_BASE=http://127.0.0.1:8088 python scripts/generate_api_key.py --label demo --ttl-seconds 86400
```
