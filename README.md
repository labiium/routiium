# Routiium

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

Routiium is an Actix-web service and Rust crate that exposes OpenAI-compatible `/v1/chat/completions` and `/v1/responses` endpoints while transparently translating payloads, streaming events, tools, routing decisions, and analytics on the fly. It lets existing Chat Completions clients tap into the modern Responses API (or any compatible upstream) without rewriting application code, while still benefiting from policy-aware multi-backend routing (documented in [`ROUTER_API_SPEC.md`](ROUTER_API_SPEC.md)) and full-stack observability via the analytics pipeline described in [`ANALYTICS.md`](ANALYTICS.md).

## What It Does

- Converts legacy Chat Completions requests, responses, and SSE chunks into the Responses API format (and back) while preserving tools, multimodal parts, logprobs, and token usage.
- Proxies `/v1/chat/completions` and `/v1/responses` to multiple upstream providers with per-model base URLs, custom headers, managed or passthrough auth, and automatic system prompt injection.
- Integrates with Router services (remote HTTP or local alias files) for policy-aware routing and falls back to legacy prefix rules defined via `ROUTIIUM_BACKENDS`.
- Issues, verifies, revokes, and expires first-party API keys (Redis, sled, or in-memory backends) so clients never see provider secrets.
- Pulls Model Context Protocol (MCP) tools into each request so clients automatically see the union of their declared tools plus any connected MCP servers.
- Records detailed analytics (request metadata, routing choices, auth state, token usage, per-request cost) using JSONL, Redis, Sled, or memory, and exposes query/export endpoints for operators.
- Ships with reloadable configuration for system prompts, MCP servers, and (experimental) routing metadata plus `/status` for automation.

## Quick Start

```bash
git clone https://github.com/labiium/routiium.git
cd routiium
# Provide whatever env vars you need (OPENAI_API_KEY, ROUTIIUM_BACKENDS, etc.)
cargo run --release -- \
  --mcp-config=mcp.json.example \
  --system-prompt-config=system_prompt.json.example \
  --router-config=router_aliases.json.example
```

Call the proxy (managed auth shown — use your issued `sk_<id>.<secret>` token):

```bash
curl -N http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_test.abcdef..." \
  -H "Content-Type: application/json" \
  -d '{
        "model":"gpt-4.1-nano",
        "messages":[{"role":"user","content":"Stream this"}],
        "stream": true
      }'
```

Need a container? The repo ships with a `Dockerfile`:

```bash
docker build -t routiium .
docker run --rm -p 8088:8088 \
  -e OPENAI_API_KEY=sk-your-upstream-key \
  routiium
```

### Docker Compose

The repository now ships with a `docker-compose.yml` that builds the local image, maps `8088`, and persists the sled database volume. Provide your upstream credentials in `.env` (Compose reads it automatically):

```bash
cp .env.example .env
echo "OPENAI_API_KEY=sk-your-upstream-key" >> .env
docker compose up --build
```

Override the public port by exporting `ROUTIIUM_PORT`, and pass additional routing knobs (e.g. `ROUTIIUM_BACKENDS`, `ROUTIIUM_ROUTER_URL`) via `.env` or your shell.

## CLI Flags

| Flag | Description |
| ---- | ----------- |
| `--keys-backend=redis://...|sled:<path>|memory` | Override the API key store (defaults to Redis via `ROUTIIUM_REDIS_URL`, else sled, else memory). |
| `--mcp-config=PATH` | Load Model Context Protocol server definitions (see `mcp.json.example`). |
| `--system-prompt-config=PATH` | Load system prompt injection rules (see `system_prompt.json.example`). |
| `--router-config=PATH` | Load a local alias/policy file consumed by the `LocalPolicyRouter` (`router_aliases.json.example`). |
| `--routing-config=PATH` | Load an experimental routing JSON that is surfaced in `/status` and reload endpoints (routing decisions still come from the Router client + `ROUTIIUM_BACKENDS`). |

## Environment Reference

Routiium loads `.env`, `.envfile`, or any path referenced via `ENV_FILE`, `ENVFILE`, or `DOTENV_PATH` before reading the rest of the environment.

### Server & HTTP

- `BIND_ADDR` – listen address (default `0.0.0.0:8088`).
- `RUST_LOG` – tracing filter, e.g. `info,tower_http=info`.
- `OPENAI_BASE_URL` – default upstream base URL (`https://api.openai.com/v1`).
- `OPENAI_API_KEY` – presence enables managed auth and serves as the fallback upstream bearer.
- `MODEL` – default model when the client omits `model`.
- `ROUTIIUM_UPSTREAM_MODE` – `responses` (default) or `chat`; `chat` rewrites upstream calls to `/v1/chat/completions` and converts payloads (handy for vLLM/Ollama).
- `ROUTIIUM_HTTP_TIMEOUT_SECONDS` – reqwest client timeout.
- `ROUTIIUM_NO_PROXY`, `ROUTIIUM_PROXY_URL`, `HTTP_PROXY`/`http_proxy`, `HTTPS_PROXY`/`https_proxy` – proxy controls.
- `CORS_ALLOWED_ORIGINS`, `CORS_ALLOWED_METHODS`, `CORS_ALLOWED_HEADERS`, `CORS_ALLOW_CREDENTIALS`, `CORS_MAX_AGE` – CORS policy knobs.

### Routing & Upstream Selection

- `ROUTIIUM_BACKENDS` – semicolon-separated rules (`prefix`, `base`/`base_url`, optional `key_env`, optional `mode=responses|chat`). Example:

  ```bash
  export OPENAI_API_KEY=sk-openai...
  export ANTHROPIC_API_KEY=sk-anthropic...
  export ROUTIIUM_BACKENDS="prefix=gpt-;base=https://api.openai.com/v1;key_env=OPENAI_API_KEY;mode=responses; prefix=claude-;base=https://api.anthropic.com/v1;key_env=ANTHROPIC_API_KEY;mode=responses; prefix=llama;base=http://localhost:11434/v1;mode=chat"
  ```

- `ROUTIIUM_ROUTER_URL` – enable the HTTP Router client (Schema 1.1). Helper env vars:
  - `ROUTIIUM_ROUTER_TIMEOUT_MS` – router request timeout (default 15 ms).
  - `ROUTIIUM_CACHE_TTL_MS` – plan cache TTL (default 15000 ms).
  - `ROUTIIUM_ROUTER_PRIVACY_MODE=features|summary|full` – how much request context is sent to the router.
  - `ROUTIIUM_ROUTER_STRICT` – when truthy, fail the request if the router rejects the alias (no legacy fallback).
  - `ROUTIIUM_ROUTER_MTLS` – set to enable mutual TLS (expects OS-level cert configuration).

### Authentication & Key Storage

- `ROUTIIUM_REDIS_URL` – use Redis for the API key store.
- `ROUTIIUM_REDIS_POOL_MAX` – r2d2 pool size for Redis (default 16).
- `ROUTIIUM_SLED_PATH` – path for the embedded sled database (default `./data/keys.db` when the `sled` feature is enabled).
- `ROUTIIUM_KEYS_REQUIRE_EXPIRATION`, `ROUTIIUM_KEYS_ALLOW_NO_EXPIRATION`, `ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS` – key issuance policy toggles.
- `ROUTIIUM_KEYS_DISABLE_CACHE` – set to `1/true` to skip the in-memory API key cache. By default Routiium eagerly loads and maintains every key in-process so sled-backed verification never blocks on disk I/O; disable the cache when multiple Routiium instances share the same external store and you prefer every verification to hit that backend directly.

### Analytics & Pricing

- `ROUTIIUM_ANALYTICS_REDIS_URL`, `ROUTIIUM_ANALYTICS_SLED_PATH`, `ROUTIIUM_ANALYTICS_JSONL_PATH` – choose the analytics backend (JSONL at `data/analytics.jsonl` is the default).
- `ROUTIIUM_ANALYTICS_TTL_SECONDS` – automatic expiration for Redis/Sled entries.
- `ROUTIIUM_ANALYTICS_FORCE_MEMORY`, `ROUTIIUM_ANALYTICS_MAX_EVENTS` – force the in-memory backend and cap retained events.
- `ROUTIIUM_PRICING_CONFIG` – path to custom pricing JSON (falls back to built-in OpenAI price cards).

## HTTP APIs

| Route | Description | Auth |
| ----- | ----------- | ---- |
| `GET /status` | Feature flags, config file paths, routing stats, analytics status. | None |
| `POST /convert` | Convert a Chat Completions payload into a Responses payload (applies system prompts, merges MCP tools, supports `conversation_id`). | None |
| `POST /v1/responses` | Native Responses proxy (handles system prompts, legacy tool formats, routing, analytics, streaming). | Managed or passthrough bearer |
| `POST /v1/chat/completions` | Native Chat Completions proxy with prompt injection and optional conversion of Responses-shaped upstream bodies. | Managed or passthrough bearer |
| `GET /keys` | List issued API keys (id, label, timestamps, scopes). | Protect via network ACLs |
| `POST /keys/generate` | Issue a new `sk_<id>.<secret>` token; body supports `label`, `ttl_seconds`, `expires_at`, `scopes`. | Protect via network ACLs |
| `POST /keys/revoke` | Revoke a key by id. | Protect via network ACLs |
| `POST /keys/set_expiration` | Set or clear expiration on an existing key. | Protect via network ACLs |
| `POST /reload/mcp` | Reload the MCP config and reconnect servers. | Typically internal |
| `POST /reload/system_prompt` | Reload the system prompt config. | Typically internal |
| `POST /reload/routing` | Reload the optional routing JSON (currently surfaces metadata only). | Typically internal |
| `POST /reload/all` | Reload MCP + system prompt configs. | Typically internal |
| `GET /analytics/stats` | Analytics backend stats (requires analytics enabled). | Internal |
| `GET /analytics/events` | Query raw analytics events (`start`, `end`, `limit`). | Internal |
| `GET /analytics/aggregate` | Aggregate metrics for a time window. | Internal |
| `GET /analytics/export` | Export events as JSON (`format=json`) or CSV (`format=csv`). | Internal |
| `POST /analytics/clear` | Wipe analytics storage. | Internal |

## Authentication Modes

1. **Managed mode** (recommended): set `OPENAI_API_KEY` (and any additional provider env vars referenced by routing rules). Clients call Routiium with internally issued tokens (`sk_<id>.<secret>`). The proxy validates them through `ApiKeyManager` before substituting provider secrets upstream.
2. **Passthrough mode**: leave `OPENAI_API_KEY` unset. Clients send their provider key in `Authorization: Bearer ...` and Routiium forwards it upstream unchanged (still applying conversion, routing, analytics, etc.).

Managed mode keeps a hot, in-process cache of every issued API key so sled-backed deployments never block on host filesystem latency during verification. The cache is warmed at startup and updated immediately on `generate`, `revoke`, and `set_expiration`. Set `ROUTIIUM_KEYS_DISABLE_CACHE=1` if you need every verification to go back to a shared store (e.g., Redis in multi-node setups).

## Multi-backend Routing & Router Integration

When resolving an upstream:

1. If `--router-config` or `ROUTIIUM_ROUTER_URL` is configured, Routiium asks the Router for a plan (Schema 1.1, see [`ROUTER_API_SPEC.md`](ROUTER_API_SPEC.md)). Plans return the upstream base URL, API mode (`responses` or `chat`), optional auth env var, stickiness tokens, headers, and policy metadata. Successful plans are cached for `ROUTIIUM_CACHE_TTL_MS` and surfaced to clients via headers like `x-route-id`, `x-resolved-model`, `router-schema`, and `x-policy-rev`.
2. If the Router rejects the alias (or is unavailable and `ROUTIIUM_ROUTER_STRICT` is not set), Routiium falls back to `ROUTIIUM_BACKENDS`, selecting the first rule whose `prefix` matches the requested model. `mode=chat` rewrites the upstream URL to `/v1/chat/completions` and converts payloads so you can front services such as vLLM or Ollama with a Responses surface.
3. If neither mechanism matches, the proxy uses `OPENAI_BASE_URL` and whichever `model` the client supplied (or the `MODEL` env fallback).

The optional `routing.json` loader (see `routing.json.example`) tracks richer policies for observability and `/status` output. Routing decisions today still use the Router client + `ROUTIIUM_BACKENDS`; the JSON file exists so you can version policies and inspect rule stats even before the full engine lands.

### Router Contract (Schema 1.1)

The Router integration follows the full Schema 1.1 contract captured in [`ROUTER_API_SPEC.md`](ROUTER_API_SPEC.md). Highlights:

- Every `RouteRequest`/`RoutePlan` exchanges `schema_version`, `request_id`, cache hints, and typed error metadata so upgrades remain safe.
- Budgets, estimates, and cost hints use **micro** units; routers can emit tokenizer hints, latency/cost targets, stickiness tokens, and prompt overlay metadata.
- Cache + stickiness semantics (`ttl_ms`, `valid_until`, `freeze_key`, `plan_token`) let Routiium deterministically reuse plans while `X-Route-Cache` and `Router-Schema` headers provide observability.
- Privacy controls (`privacy_mode`, `content_attestation`, `content_used`) make it explicit how much transcript content the router consumed.
- `RouteFeedback`, `plan_batch`, `prefetch`, and the catalog endpoints (`/catalog/models`) are part of the same spec; Routiium ships `examples/router_service.rs` as a runnable reference implementation.

If you are implementing a Router, start with that document—the server expects the exact fields, headers, and error codes described there and falls back gracefully only when `ROUTIIUM_ROUTER_STRICT` is disabled.

## System Prompts & MCP Tools

- **System prompts:** `--system-prompt-config` points to a JSON file with `global`, `per_model`, and `per_api` prompts plus an `injection_mode` (`prepend`, `append`, or `replace`). Prompts are applied to `/v1/responses`, `/v1/chat/completions`, and `/convert`, and you can hot-reload the file via `/reload/system_prompt`.
- **Model Context Protocol:** `--mcp-config` points to your MCP config (`mcp.json`). On boot Routiium spawns each MCP server, lists available tools, and merges them into every request so clients automatically see both their declared tools and MCP-provided ones. Tool names are prefixed with `serverName_` (`filesystem_read_directory`, `postgres_run_query`, etc.). Use `/reload/mcp` after editing the config.

## API Key Lifecycle

`ApiKeyManager` issues opaque tokens (`sk_<id>.<secret>`) whose secrets are never persisted (salted SHA-256 hashes only):

- Backends are auto-detected at runtime (`ROUTIIUM_REDIS_URL` → Redis, else sled through the default `sled` feature, else memory). Override with `--keys-backend`.
- Redis pool size is controlled through `ROUTIIUM_REDIS_POOL_MAX`.
- Expiration policy is governed by `ROUTIIUM_KEYS_REQUIRE_EXPIRATION`, `ROUTIIUM_KEYS_ALLOW_NO_EXPIRATION`, and `ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS`.
- `/keys`, `/keys/generate`, `/keys/revoke`, and `/keys/set_expiration` cover the full key lifecycle. Secure these endpoints via network ACLs, sidecars, or service mesh policy; Routiium does not implement a separate admin role.

Managed mode validates tokens on every call; passthrough mode skips the manager and forwards whatever bearer the client sent.

## Analytics & Pricing

Every request flows through `analytics_middleware`, which captures:

- Request metadata (endpoint, method, model, payload size, streaming flag, user agent, client IP).
- Response metadata (status, body size, error message, streaming duration).
- Auth metadata (key id + label when present, auth method).
- Routing metadata (backend string, upstream mode, whether MCP/system prompts were used).
- Token usage (prompt/completion/cached/reasoning tokens) and computed cost via `PricingConfig`.

Storage backends:

- JSONL (`data/analytics.jsonl` by default; override via `ROUTIIUM_ANALYTICS_JSONL_PATH`).
- Redis (`ROUTIIUM_ANALYTICS_REDIS_URL`, optional `ROUTIIUM_ANALYTICS_TTL_SECONDS`).
- Sled (`ROUTIIUM_ANALYTICS_SLED_PATH`, compiled in by default).
- Memory (`ROUTIIUM_ANALYTICS_FORCE_MEMORY=1`, optional `ROUTIIUM_ANALYTICS_MAX_EVENTS`).

Operators can inspect and manage analytics through `/analytics/stats`, `/analytics/events`, `/analytics/aggregate`, `/analytics/export?format=csv`, and `/analytics/clear`. Costs come from the built-in OpenAI price cards unless you point `ROUTIIUM_PRICING_CONFIG` at your own JSON (prefix matching is supported). See [ANALYTICS.md](ANALYTICS.md) for the complete data model.

## Operations & Observability

- **Status & reloads:** `GET /status` reports version info, enabled features, config paths, routing stats, and analytics state. `/reload/mcp`, `/reload/system_prompt`, `/reload/routing`, and `/reload/all` re-read their respective files without restarting the server.
- **Route headers:** When a Router plan is used Routiium forwards headers such as `x-route-id`, `router-schema`, `x-policy-rev`, and `x-resolved-model` so clients can trace which upstream handled the request.
- **Logging:** `init_tracing` discovers `.env`, `.envfile`, or whatever you point `ENV_FILE`/`ENVFILE`/`DOTENV_PATH` at, then configures `tracing-subscriber` based on `RUST_LOG`.
- **Proxies & CORS:** `build_http_client_from_env` honors `ROUTIIUM_NO_PROXY`, `ROUTIIUM_PROXY_URL`, `HTTP_PROXY`, and `HTTPS_PROXY`. `cors_config_from_env` applies the `CORS_*` knobs.
- **Docker:** The provided image defaults to `BIND_ADDR=0.0.0.0:8088` and `ROUTIIUM_SLED_PATH=/data/keys.db`; mount `/data` if you want persistent key storage.

## Additional Documentation & Examples

- [API_REFERENCE.md](API_REFERENCE.md) – exhaustive request/response documentation with curl snippets.
- [ANALYTICS.md](ANALYTICS.md) – analytics architecture, storage backends, API responses.
- [ROUTER_API_SPEC.md](ROUTER_API_SPEC.md) – Router schema 1.1 and implementation guide (see `examples/router_service.rs` for a runnable Router).
- `mcp.json.example`, `system_prompt.json.example`, `router_aliases.json.example` – starter configs for MCP servers, system prompts, and local router aliases.
- `routing.json.example` – example of the experimental routing metadata file surfaced via `/status`.

## Development

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo test
```

There is also a `python_tests/` directory with HTTP smoke tests; activate your preferred Python environment and run `pytest` if you modify the HTTP surface.

### Responses CLI

Need a manual multi-turn sanity check against the streaming Responses bridge? Use the lightweight helper:

```bash
ROUTIIUM_BASE=http://127.0.0.1:8088 \
python python_tests/chat_cli.py --model gpt-4.1-nano
```

The script loads `.env`, calls `ROUTIIUM_BASE/keys/generate` (unless you pass `ROUTIIUM_ACCESS_TOKEN`), then streams Responses API calls through `ROUTIIUM_BASE/v1/responses` using the official OpenAI SDK. Commands: `/reset` to clear the conversation, `/exit` to quit.

Need to automate the smoke test? `python_tests/run_chat_cli_e2e.sh` bootstraps the Python env, (optionally) builds Routiium, launches the proxy, feeds a canned prompt into `chat_cli.py`, and tears things down when finished:

```bash
cd python_tests
./run_chat_cli_e2e.sh --message "ping" --model gpt-4.1-nano
```

Use `--reuse-server` to point at an already running proxy or `--transcript logs/chat_cli.txt` to archive the captured conversation.

### Key generator CLI

Need to mint managed credentials without crafting curl payloads? Use the bundled helper:

```bash
ROUTIIUM_BASE=http://127.0.0.1:8088 \
python scripts/generate_api_key.py \
  --label demo-session \
  --ttl-seconds 86400 \
  --scope inference
```

The script loads `.env`, hits `/keys/generate`, and prints either a friendly summary or `--json` output so you can pipe the token elsewhere. Use `--expires-at 2024-12-31T23:59:59Z` to pin an exact cutoff or add multiple `--scope` flags to match your policy. The bearer string is shown only once—store it securely.
