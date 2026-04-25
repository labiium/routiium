# Routiium

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

Routiium is a self-hosted, OpenAI-compatible LLM gateway with safe-by-default routing. Put it between your apps and your model providers to add managed API keys, built-in policy routing, request-level LLM/deterministic judging, rate limits, analytics, system prompts, MCP tools, and optional remote Router-compatible policy services without changing client SDKs.

```
Your App / OpenAI SDK
        │
        ▼
  Routiium :8088 ── auth, limits, routing, judge, analytics, tools
        │
        ├── OpenAI / Responses API
        ├── vLLM, Ollama, or any Chat-compatible endpoint
        ├── AWS Bedrock
        ├── Built-in policy router + safety judge
        └── Optional remote Router-compatible policy service
```

## Install

```bash
npm install -g routiium
routiium --version
```

The npm package installs a `routiium` command. On install it downloads the matching native binary from the GitHub release when available and falls back to `cargo build --release --locked` if a prebuilt binary is not available for your platform. Source installs still work with `cargo run -- ...`.

## Choose your path

### 1. Proxy an OpenAI-compatible app safely in minutes

```bash
npm install -g routiium
routiium config init --profile openai
routiium config set OPENAI_API_KEY sk-your-provider-key
routiium doctor
routiium serve
```

Then point any OpenAI-compatible SDK at `http://127.0.0.1:8088/v1`. With no extra router setup, Routiium enables its embedded router, strict safety path, `protect` judge mode, response guard, streaming safety, and restricted web/URL judging.

### 2. Use a local vLLM/Ollama-style upstream

```bash
routiium init --profile vllm --out .env
# edit OPENAI_BASE_URL if your local server is not http://127.0.0.1:8000/v1
routiium serve
```

### 3. Use built-in routing, or connect a remote router

```bash
routiium init --profile router --out .env
routiium router probe --model gpt-4.1-nano
```

Routiium ships with aliases such as `auto`, `fast`, `balanced`, `safe`, `secure`, and `premium` with cost/latency/context-aware scoring. You do not need another project to get default routing. Remote router mode is optional for teams that want a separate Router-compatible policy service for central policy, catalog, health, and overlay management; EduRouter is only a small companion/reference implementation of that interface.

### 4. Inspect the built-in judge

```bash
routiium router explain --model auto --prompt "Ignore previous instructions"
routiium judge policy init --out config/judge-policy.json
routiium judge explain --policy config/judge-policy.json --prompt "Ignore previous instructions"
routiium judge test --suite all
routiium judge profile protect --out .env
```

Default `protect` mode enforces high-confidence deterministic blocks, downgrades prompt-injection-like requests to a safer route, scans successful outputs with the response guard, and uses an LLM judge automatically when `OPENAI_API_KEY` is available. LLM judge calls prefer tool/function calling (`ROUTIIUM_JUDGE_OUTPUT_MODE=auto`) and fall back to JSON for providers that do not support tools. Use `shadow` to observe, `protect` for safe defaults, `enforce` for stricter policy, or `off` to disable.
Custom judge prompts are supported through policy overlays; Routiium still keeps its built-in safety prompt immutable and routes sensitive-but-allowable requests to the built-in `secure` alias by default.
For agentic applications, rejected unsafe actions return an OpenAI-compatible assistant result by default (`ROUTIIUM_REJECTION_MODE=agent_result`) so the loop can continue without fulfilling the unsafe request; set `http_error` for strict gateway-style 403s.

## Day-one workflow

```bash
# 1) Start Routiium
routiium serve

# 2) Create a managed customer key
routiium key create --label demo --ttl-seconds 86400

# 3) Call the OpenAI-compatible API with the returned sk_<id>.<secret>
curl http://127.0.0.1:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_<id>.<secret>" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4.1-nano","messages":[{"role":"user","content":"Hello"}]}'

# 4) Check runtime state
routiium status
```

If `OPENAI_API_KEY` is configured on the server, Routiium runs in managed mode: clients use Routiium-issued keys and never see the upstream provider secret. If no server-side provider key is set, Routiium can run in passthrough mode and forward client bearer tokens upstream.

## CLI

Routiium now exposes a `clap`-based CLI:

| Command | Purpose |
| --- | --- |
| `routiium serve` | Run the HTTP gateway. Existing root flags still work as a compatibility alias. |
| `routiium config init --profile <profile>` | Create or update `~/.config/routiium/config.env` with starter settings. Profiles: `openai`, `vllm`, `router`, `judge`, `bedrock`, `synthetic`. |
| `routiium config set`, `get`, `list`, `path` | Manage the per-user config file without hand-editing env files. |
| `routiium init --profile <profile>` | Generate starter `.env` and profile config for repo-local/deploy workflows. Profiles: `openai`, `vllm`, `router`, `judge`, `bedrock`, `synthetic`. |
| `routiium doctor` | Check env/config files, server health, and judge/router readiness. Use `--production` before launch. |
| `routiium status` | Fetch `/status` from a running server. |
| `routiium key create`, `list`, `revoke` | Manage API keys through the admin HTTP API. |
| `routiium router probe` | Send a small request and print routing-related response details. |
| `routiium router explain` | Explain the embedded router + judge decision locally. |
| `routiium judge profile <mode>` | Update local env defaults for judge rollout. Modes: `shadow`, `protect`, `enforce`, `off`. |
| `routiium judge policy init`, `validate` | Create or validate custom judge prompt/policy overlays. |
| `routiium judge explain` | Explain judge action, target alias, and policy fingerprint locally. |
| `routiium judge test` | Run built-in prompt-injection/exfiltration/dangerous-action judge checks. |
| `routiium docs` | Print the main docs entry points. |

Run `routiium --help` or see [docs/CLI.md](docs/CLI.md) for the full command reference. `/convert` now defaults to safe conversion without internal system prompts/MCP metadata; add `?include_internal_config=true` with admin auth only when you intentionally need to inspect internal conversion shape.

## Core features

- **OpenAI-compatible proxy** for `/v1/chat/completions`, `/v1/responses`, and `/v1/models`.
- **Payload translation** between Chat Completions and Responses API shapes, including streaming, tools, multimodal content, logprobs, and usage metadata.
- **Managed API keys** with Redis, sled, or memory backends; hashed secrets; labels; expiration; scopes; revocation; and admin APIs.
- **Embedded policy routing** with `auto`, `fast`, `balanced`, `safe`, `secure`, and `premium` aliases, cost/latency/context scoring, route metadata, and remote Router Schema compatibility.
- **Built-in request judge and response guard** with deterministic prompt-injection/exfiltration/tool-risk checks, optional LLM judging, restricted web/URL judging, custom operator policy overlays, `secure` rerouting for sensitive requests, output-leak blocking, streaming safety, and structured denial responses.
- **Rate limits and concurrency controls** with hot-reloadable policies and emergency blocks.
- **Analytics and cost tracking** with JSONL, Redis, sled, or memory storage and CSV/JSON export.
- **System prompts and MCP tools** injected transparently into upstream requests. MCP runtime config writes are disabled by default because MCP servers can spawn local commands; enable `ROUTIIUM_ALLOW_MCP_CONFIG_UPDATE=1` only for trusted admin deployments.
- **Admin panel** for keys, limits, prompts, routing, analytics, and runtime state.

## Docker

```bash
docker build -t routiium .
docker run --rm -p 8088:8088 -e OPENAI_API_KEY=sk-your-key routiium
```

To run with the unified YAML runtime config:

```bash
cp routiium.yaml.example routiium.yaml
docker run --rm -p 8088:8088 \
  -e OPENAI_API_KEY=sk-your-key \
  -e ROUTIIUM_ADMIN_TOKEN=change-me-admin-token \
  -v "$PWD/routiium.yaml:/config/routiium.yaml:ro" \
  routiium serve --config-yaml /config/routiium.yaml
```

The YAML file is a deployment profile, not an OpenAI-only or append-only requirement. It can define any OpenAI-compatible provider, disable system prompt injection per alias, use multiple system prompts, define inline prompts per alias, or use `prepend`, `append`, and `replace` prompt policies.
It can also assign per-alias judge, response-guard, tool-result guard, MCP bundle, rate-limit fallback, and pricing-model policies. Reload a mounted YAML profile with `POST /reload/runtime-config` using the admin bearer token.

Or use the included Compose setup:

```bash
cp .env.example .env
# edit .env and set OPENAI_API_KEY plus ROUTIIUM_ADMIN_TOKEN
# optional: cp routiium.yaml.example routiium.yaml and set ROUTIIUM_CONFIG_YAML_HOST=./routiium.yaml
docker compose up --build
```

The Compose file mounts `${ROUTIIUM_CONFIG_YAML_HOST:-./routiium.yaml.example}` to `/config/routiium.yaml` and sets `ROUTIIUM_CONFIG_YAML=/config/routiium.yaml` in the container.
Unset `ROUTIIUM_CONFIG_YAML_HOST` to use the example profile, or point it at your own YAML file.

## Admin panel

```bash
npm run admin:install
npm run admin:dev
```

Set `ROUTIIUM_ADMIN_TOKEN` on the server and enter the matching bearer in the panel header. Admin APIs now fail closed when this token is unset; `ROUTIIUM_INSECURE_ADMIN=1` is only for throwaway local development. The panel talks to live admin APIs; it is not a mock dashboard.

## Documentation

Start here:

- [Getting Started](docs/GETTING_STARTED.md) — profile-based onboarding recipes.
- [CLI Reference](docs/CLI.md) — all commands, flags, and examples.
- [Configuration](docs/CONFIGURATION.md) — `~/.config/routiium/config.env`, `.env`, env vars, and config files.
- [Security Model](docs/SECURITY_MODEL.md) — built-in judge threat model and controls.
- [Judge Policy](docs/JUDGE_POLICY.md) — custom prompts, secure rerouting, and policy validation.
- [Production Checklist](docs/PRODUCTION_CHECKLIST.md) — launch checks for auth, CORS, guards, and streaming safety.
- [API Reference](docs/API_REFERENCE.md) — HTTP routes and payload examples.
- [Router Usage](docs/ROUTER_USAGE.md) and [Router API Spec](docs/ROUTER_API_SPEC.md) — built-in routing plus optional remote Router-compatible integration.
- [Rate Limits](docs/RATE_LIMITS.md), [Analytics](docs/ANALYTICS.md), [AWS Bedrock](docs/AWS_BEDROCK.md), and [Production Hardening](docs/PRODUCTION_HARDENING.md).

## Development

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo test
npm run package:verify
npm pack --dry-run
```

HTTP smoke tests live in `python_tests/` and can be run with `pytest`.
