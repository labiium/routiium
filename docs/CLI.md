# Routiium CLI Reference

Routiium uses a `clap`-based CLI. Run `routiium --help` for the top-level command list and `routiium <command> --help` for command-specific flags.

## `routiium serve`

Starts the HTTP gateway.

```bash
routiium serve
routiium serve --keys-backend sled:./data/keys.db
routiium serve --mcp-config mcp.json --system-prompt-config system_prompt.json
```

Existing root-level flags remain compatible, so `routiium --keys-backend=memory` behaves like `routiium serve --keys-backend memory`.

Common flags:

| Flag | Env fallback | Description |
| --- | --- | --- |
| `--keys-backend redis://...\|sled:<path>\|memory` | backend-specific env vars | API key store override. |
| `--mcp-config PATH` | `ROUTIIUM_MCP_CONFIG` | MCP server definitions. |
| `--system-prompt-config PATH` | `ROUTIIUM_SYSTEM_PROMPT_CONFIG` | System prompt rules. |
| `--routing-config PATH` | `ROUTIIUM_ROUTING_CONFIG` | Legacy routing config. |
| `--router-config PATH` | `ROUTIIUM_ROUTER_CONFIG` | Local policy router file. |
| `--rate-limit-config PATH` | `ROUTIIUM_RATE_LIMIT_CONFIG` | Rate limit policy file. |

## `routiium init`

Creates starter `.env` files for common profiles.

```bash
routiium init --profile openai --out .env
routiium init --profile vllm --out .env.local
routiium init --profile router --out .env.router
routiium init --profile judge --out .env.judge
routiium init --profile bedrock --out .env.bedrock --config-dir config
```

Profiles:

| Profile | Use when |
| --- | --- |
| `openai` | You want managed keys in front of OpenAI. |
| `vllm` | You have a local OpenAI-compatible server such as vLLM or Ollama. |
| `router` | You want remote routing through EduRouter or another Router service. |
| `judge` | You want router-side LLM-as-judge policy in shadow/enforce rollout. |
| `bedrock` | You want an AWS Bedrock-oriented starter config. |

`init` refuses to overwrite existing files unless `--force` is passed.

## `routiium doctor`

Checks local setup and optional live services.

```bash
routiium doctor --env-file .env
routiium doctor --url http://127.0.0.1:8088 --check-router
routiium doctor --require-server --url http://127.0.0.1:8088
routiium doctor --json
```

Doctor checks include env file presence, referenced config files, provider key/base URL hints, `/status`, optional remote router catalog reachability, and judge/cache compatibility. By default, an unreachable server is a warning so `doctor` can be used before `serve`; use `--require-server` for deployment readiness checks.

## `routiium status`

Fetches `/status` from a running Routiium server.

```bash
routiium status
routiium status --url http://localhost:8088 --json
```

## `routiium key`

Wraps the admin key HTTP API. If `ROUTIIUM_ADMIN_TOKEN` is set on the server, pass it with `--admin-token` or set the same env var locally.

```bash
routiium key create --label demo --ttl-seconds 86400
routiium key create --label ci --scope chat --scope models --json
routiium key list --active-only
routiium key list --label-prefix customer-
routiium key revoke <key-id>
```

The CLI intentionally talks to the running server instead of mutating the key database directly, so server-side auth, validation, rate-limit metadata, and storage choices remain authoritative.

## `routiium router probe`

Sends a minimal chat completion request through Routiium and prints status, routing-related headers, and the response body.

```bash
routiium router probe --model gpt-4.1-nano
routiium router probe --model safe-alias --api-key sk_<id>.<secret> --json
```

Use this after enabling `ROUTIIUM_ROUTER_URL`, local router config, or judge mode.

## `routiium judge profile`

Updates a local env file with judge rollout defaults.

```bash
routiium judge profile shadow --out .env
routiium judge profile enforce --out .env
routiium judge profile off --out .env
```

`shadow` logs/observes judge outcomes without blocking. `enforce` is for validated policies. Strict every-request judging requires `ROUTIIUM_CACHE_TTL_MS=0` in Routiium and zero-TTL judged plans from the Router.

## `routiium docs`

Prints the main documentation entry points.

```bash
routiium docs
routiium docs --json
```
