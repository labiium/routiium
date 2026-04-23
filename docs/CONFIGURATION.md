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
| `ROUTIIUM_BACKENDS` | unset | Semicolon-separated legacy backend rules. |
| `ROUTIIUM_ROUTER_URL` | unset | Remote Router/EduRouter URL. |
| `ROUTIIUM_ROUTER_TIMEOUT_MS` | `15` | Remote router request timeout in milliseconds. |
| `ROUTIIUM_CACHE_TTL_MS` | `15000` | Local route-plan cache horizon. |
| `ROUTIIUM_ROUTER_PRIVACY_MODE` | `features` | `features`, `summary`, or `full`. |
| `ROUTIIUM_ROUTER_STRICT` | unset | Preserve router errors instead of falling back. |
| `ROUTIIUM_ROUTER_MTLS` | unset | Enable mTLS support in the HTTP router client. |

Use `routiium router probe --model <alias>` after changing router settings.

## LLM-as-judge profiles

These variables are consumed by the example Router/EduRouter side of the deployment, while Routiium enforces strict remote routing and cache behavior.

| Variable | Typical value | Description |
| --- | --- | --- |
| `ROUTER_JUDGE_MODE` | `shadow` then `enforce` | Judge rollout mode. |
| `ROUTER_JUDGE_CONTEXT` | `full` | Context sent to the judge. |
| `ROUTER_JUDGE_FAILURE` | `allow`, `deny`, or `safe_model` | Failure policy. |
| `ROUTER_JUDGE_MODEL` | `gpt-4o-mini` | Judge model. |
| `ROUTER_JUDGE_SAFE_MODEL` | model id | Safe fallback when using `safe_model`. |
| `ROUTER_JUDGE_API_KEY_ENV` | `OPENAI_API_KEY` | Env var holding the judge provider key. |
| `ROUTER_JUDGE_TIMEOUT_MS` | `800` | Judge timeout. |

For every-request judging, set `ROUTIIUM_CACHE_TTL_MS=0` and configure the Router to return zero-TTL plans for judged aliases.

## Rate limits, analytics, and proxy controls

See the dedicated guides for full details:

- [RATE_LIMITS.md](RATE_LIMITS.md)
- [ANALYTICS.md](ANALYTICS.md)
- [PRODUCTION_HARDENING.md](PRODUCTION_HARDENING.md)
