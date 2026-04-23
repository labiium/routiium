# Routiium

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

Routiium is a self-hosted, OpenAI-compatible LLM gateway. Put it between your apps and your model providers to add managed API keys, routing, rate limits, analytics, system prompts, MCP tools, and optional router-side LLM-as-judge policy without changing client SDKs.

```
Your App / OpenAI SDK
        │
        ▼
  Routiium :8088 ── auth, limits, routing, judge, analytics, tools
        │
        ├── OpenAI / Responses API
        ├── vLLM, Ollama, or any Chat-compatible endpoint
        ├── AWS Bedrock
        └── EduRouter / remote Router policy service
```

## Choose your path

### 1. Proxy an OpenAI-compatible app in minutes

```bash
git clone https://github.com/labiium/routiium.git && cd routiium
cargo run -- init --profile openai --out .env
# edit .env and set OPENAI_API_KEY
cargo run -- doctor --env-file .env
cargo run -- serve
```

Then point any OpenAI-compatible SDK at `http://127.0.0.1:8088/v1`.

### 2. Use a local vLLM/Ollama-style upstream

```bash
cargo run -- init --profile vllm --out .env
# edit OPENAI_BASE_URL if your local server is not http://127.0.0.1:8000/v1
cargo run -- serve
```

### 3. Route through EduRouter or another Router service

```bash
cargo run -- init --profile router --out .env
cargo run -- router probe --model gpt-4.1-nano
```

Remote router mode lets Routiium ask a policy service for the upstream target, cache hints, cost metadata, and routing decisions on each request.

### 4. Add LLM-as-judge policy

```bash
cargo run -- init --profile judge --out .env
# start your Router/EduRouter with matching ROUTER_JUDGE_* settings
cargo run -- router probe --model safe-alias
```

Start with `ROUTER_JUDGE_MODE=shadow` to observe verdicts. Move to `enforce` only after policy behavior is validated. For strict every-request judging, keep `ROUTIIUM_ROUTER_STRICT=1` and `ROUTIIUM_CACHE_TTL_MS=0`, and configure the Router to return zero-TTL judged plans.

## Day-one workflow

```bash
# 1) Start Routiium
cargo run -- serve

# 2) Create a managed customer key
cargo run -- key create --label demo --ttl-seconds 86400

# 3) Call the OpenAI-compatible API with the returned sk_<id>.<secret>
curl http://127.0.0.1:8088/v1/chat/completions \
  -H "Authorization: Bearer sk_<id>.<secret>" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4.1-nano","messages":[{"role":"user","content":"Hello"}]}'

# 4) Check runtime state
cargo run -- status
```

If `OPENAI_API_KEY` is configured on the server, Routiium runs in managed mode: clients use Routiium-issued keys and never see the upstream provider secret. If no server-side provider key is set, Routiium can run in passthrough mode and forward client bearer tokens upstream.

## CLI

Routiium now exposes a `clap`-based CLI:

| Command | Purpose |
| --- | --- |
| `routiium serve` | Run the HTTP gateway. Existing root flags still work as a compatibility alias. |
| `routiium init --profile <openai|vllm|router|judge|bedrock>` | Generate starter `.env` and profile config. |
| `routiium doctor` | Check env/config files, server health, and judge/router readiness. |
| `routiium status` | Fetch `/status` from a running server. |
| `routiium key create|list|revoke` | Manage API keys through the admin HTTP API. |
| `routiium router probe` | Send a small request and print routing-related response details. |
| `routiium judge profile shadow|enforce|off` | Update local env defaults for judge rollout. |
| `routiium docs` | Print the main docs entry points. |

Run `routiium --help` or see [docs/CLI.md](docs/CLI.md) for the full command reference.

## Core features

- **OpenAI-compatible proxy** for `/v1/chat/completions`, `/v1/responses`, and `/v1/models`.
- **Payload translation** between Chat Completions and Responses API shapes, including streaming, tools, multimodal content, logprobs, and usage metadata.
- **Managed API keys** with Redis, sled, or memory backends; hashed secrets; labels; expiration; scopes; revocation; and admin APIs.
- **Multi-backend routing** through prefix rules, routing JSON, local router config, or remote Router Schema 1.1 services.
- **LLM-as-judge integration** through router-side judge profiles with shadow/enforce rollout guidance.
- **Rate limits and concurrency controls** with hot-reloadable policies and emergency blocks.
- **Analytics and cost tracking** with JSONL, Redis, sled, or memory storage and CSV/JSON export.
- **System prompts and MCP tools** injected transparently into upstream requests.
- **Admin panel** for keys, limits, prompts, routing, analytics, and runtime state.

## Docker

```bash
docker build -t routiium .
docker run --rm -p 8088:8088 -e OPENAI_API_KEY=sk-your-key routiium
```

Or use the included Compose setup:

```bash
cp .env.example .env
# edit .env
docker compose up --build
```

## Admin panel

```bash
npm run admin:install
npm run admin:dev
```

Set `ROUTIIUM_ADMIN_TOKEN` on the server and enter the matching bearer in the panel header. The panel talks to live admin APIs; it is not a mock dashboard.

## Documentation

Start here:

- [Getting Started](docs/GETTING_STARTED.md) — profile-based onboarding recipes.
- [CLI Reference](docs/CLI.md) — all commands, flags, and examples.
- [Configuration](docs/CONFIGURATION.md) — env vars and config files.
- [API Reference](docs/API_REFERENCE.md) — HTTP routes and payload examples.
- [Router Usage](docs/ROUTER_USAGE.md) and [Router API Spec](docs/ROUTER_API_SPEC.md) — Router/EduRouter integration.
- [Rate Limits](docs/RATE_LIMITS.md), [Analytics](docs/ANALYTICS.md), [AWS Bedrock](docs/AWS_BEDROCK.md), and [Production Hardening](docs/PRODUCTION_HARDENING.md).

## Development

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo test
```

HTTP smoke tests live in `python_tests/` and can be run with `pytest`.
