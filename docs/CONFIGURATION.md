# Routiium Configuration

Routiium reads process environment variables, `.env`/`.envfile`, and selected CLI flags. CLI flags are best for local experiments; env and config files are better for deploys.

## Server basics

| Variable | Default | Description |
| --- | --- | --- |
| `BIND_ADDR` | `0.0.0.0:8088` | HTTP listen address. |
| `OPENAI_API_KEY` | unset | Server-side upstream provider key; enables managed mode by default. |
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | Default upstream base URL. |
| `MODEL` | unset | Default model when clients omit `model`. |
| `ROUTIIUM_MANAGED_MODE` | auto | Force managed or passthrough auth mode. |
| `ROUTIIUM_UPSTREAM_MODE` | `responses` | `responses`, `chat`, or provider-specific modes. |
| `ROUTIIUM_HTTP_TIMEOUT_SECONDS` | unset | Upstream request timeout. |
| `RUST_LOG` | `info,tower_http=info` | Tracing filter. |

## CLI-backed config files

| CLI flag | Env var | Description |
| --- | --- | --- |
| `--mcp-config PATH` | `ROUTIIUM_MCP_CONFIG` | MCP server definitions. |
| `--system-prompt-config PATH` | `ROUTIIUM_SYSTEM_PROMPT_CONFIG` | System prompt injection rules. |
| `--routing-config PATH` | `ROUTIIUM_ROUTING_CONFIG` | Legacy routing config with reload support. |
| `--router-config PATH` | `ROUTIIUM_ROUTER_CONFIG` | Local policy router file. |
| `--rate-limit-config PATH` | `ROUTIIUM_RATE_LIMIT_CONFIG` | Rate limit policy file. |

## API keys and admin

| Variable | Default | Description |
| --- | --- | --- |
| `ROUTIIUM_REDIS_URL` | unset | Redis key-store URL. |
| `ROUTIIUM_SLED_PATH` | `./data/keys.db` | Sled key-store path. |
| `ROUTIIUM_ADMIN_TOKEN` | unset | Bearer token for protected admin endpoints. |
| `ROUTIIUM_KEYS_REQUIRE_EXPIRATION` | unset | Require TTL/expiration on new keys. |
| `ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS` | unset | Default TTL for new keys. |
| `ROUTIIUM_KEYS_DISABLE_CACHE` | unset | Disable in-process key cache. |

`routiium key create|list|revoke` uses the HTTP admin API and therefore follows the same admin-token policy as direct HTTP calls.

## Routing and Router service

| Variable | Default | Description |
| --- | --- | --- |
| `ROUTIIUM_BACKENDS` | unset | Semicolon-separated legacy backend rules used only when embedded/remote routing is disabled or allowed to fall back. |
| `ROUTIIUM_ROUTER_MODE` | `embedded` | `embedded` for built-in router/judge; `off`/`legacy` to disable it. |
| `ROUTIIUM_ROUTER_URL` | unset | Remote Router/EduRouter URL. Takes precedence over embedded routing. |
| `ROUTIIUM_ROUTER_TIMEOUT_MS` | `15` | Remote router request timeout in milliseconds. |
| `ROUTIIUM_CACHE_TTL_MS` | `15000` remote, `0` embedded status | Local route-plan cache horizon for cached remote clients. |
| `ROUTIIUM_ROUTER_PRIVACY_MODE` | `full` embedded, `features` remote profile | `features`, `summary`, or `full`. |
| `ROUTIIUM_ROUTER_STRICT` | `1` embedded, unset otherwise | Preserve router errors instead of falling back. |
| `ROUTIIUM_ROUTER_MTLS` | unset | Enable mTLS support in the HTTP router client. |

Use `routiium router explain --model auto` before starting a server and `routiium router probe --model <alias>` after changing router settings.

## Built-in judge and remote judge profiles

Routiium consumes the `ROUTIIUM_JUDGE_*` variables for the embedded judge. The legacy `ROUTER_JUDGE_*` variables are still accepted by the example remote Router.

| Variable | Typical value | Description |
| --- | --- | --- |
| `ROUTIIUM_JUDGE_MODE` | `protect` | `off`, `shadow`, `protect`, or `enforce`. |
| `ROUTIIUM_JUDGE_LLM` | `auto` | `auto` uses the LLM judge when the configured key is present; `off` uses deterministic checks only. |
| `ROUTIIUM_JUDGE_MODEL` | `gpt-5-nano` | Judge model for optional external LLM judging. |
| `ROUTIIUM_JUDGE_BASE_URL` | `https://api.openai.com/v1` | Judge provider base URL. |
| `ROUTIIUM_JUDGE_SAFE_TARGET` | `safe` | Embedded route target for downgrades. |
| `ROUTIIUM_JUDGE_SENSITIVE_TARGET` | `secure` | Embedded route target for sensitive-but-allowable requests such as prompt injection or secrets in prompts. |
| `ROUTIIUM_JUDGE_DENY_TARGET` | `secure` | Embedded route target when deny rerouting is explicitly enabled. |
| `ROUTIIUM_JUDGE_ON_DENY` | `block` | `block` hard-denies dangerous requests; `route` explicitly reroutes denials to `ROUTIIUM_JUDGE_DENY_TARGET`. |
| `ROUTIIUM_REJECTION_MODE` | `agent_result` | `agent_result` returns OpenAI-compatible rejected assistant results for agent loops; `http_error` returns strict HTTP 403 policy errors. |
| `ROUTIIUM_JUDGE_POLICY_PATH` | unset | Optional JSON policy overlay for custom prompts and judge route targets. |
| `ROUTIIUM_JUDGE_PROMPT_FILE` | unset | Optional operator prompt file appended after Routiium's immutable safety prompt. |
| `ROUTIIUM_JUDGE_API_KEY_ENV` | `OPENAI_API_KEY` | Env var holding the judge provider key. |
| `ROUTIIUM_JUDGE_TIMEOUT_MS` | `800` | Judge timeout. |
| `ROUTIIUM_WEB_JUDGE` | `restricted` | `off`, `restricted`, or `full`; restricted does URL/domain checks without sending private prompts to search. |
| `ROUTIIUM_RESPONSE_GUARD` | inherits judge mode | `off`, `shadow`, `protect`, or `enforce`; scans successful outputs for prompt/secret leakage and dangerous-action guidance. |
| `ROUTIIUM_STREAMING_SAFETY` | `chunk` | `off`, `chunk`, `buffer`, or `force_non_stream`; risky judged streams are forced to non-streaming so the response guard can inspect the whole body. |
| `ROUTIIUM_SAFETY_AUDIT_PATH` | unset | Optional JSONL file for router denials and response-guard blocks. |
| `ROUTIIUM_SAFETY_AUDIT_MAX_EVENTS` | `1000` | In-memory recent safety-event retention for `/admin/safety/events`. |

For every-request external/remote judging, set `ROUTIIUM_CACHE_TTL_MS=0` and configure the Router to return zero-TTL plans for judged aliases.
Use `routiium judge policy init`, `routiium judge policy validate`, and `routiium judge explain` to create and inspect custom judge policy overlays. Operator prompts can make policy stricter but cannot replace Routiium's built-in safety prompt.

Response-guard decisions are returned in `x-response-guard-*` headers. In `protect`/`enforce`, blocked outputs return HTTP 403 with `code=response_guard_blocked`.
Operators can inspect recent safety events with `routiium judge events` or `GET /admin/safety/events`.

## Rate limits, analytics, and proxy controls

See the dedicated guides for full details:

- [RATE_LIMITS.md](RATE_LIMITS.md)
- [ANALYTICS.md](ANALYTICS.md)
- [PRODUCTION_HARDENING.md](PRODUCTION_HARDENING.md)
