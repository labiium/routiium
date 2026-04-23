# Getting Started with Routiium

This guide is organized by the job you want Routiium to do first. Each path starts with `routiium init`, checks setup with `routiium doctor`, and then runs the same OpenAI-compatible server.

## Path 1: Managed keys in front of OpenAI

```bash
routiium init --profile openai --out .env
```

Edit `.env`:

```env
OPENAI_API_KEY=sk-your-provider-key
ROUTIIUM_ADMIN_TOKEN=change-me-admin-token
```

Then run:

```bash
routiium doctor --env-file .env
routiium serve
routiium key create --label first-user --ttl-seconds 86400
```

Use the returned `sk_<id>.<secret>` as the client bearer token and set your OpenAI SDK base URL to `http://127.0.0.1:8088/v1`.

## Path 2: Local vLLM/Ollama/OpenAI-compatible server

```bash
routiium init --profile vllm --out .env
```

Edit `OPENAI_BASE_URL` if your local model server is not on `http://127.0.0.1:8000/v1`, then run:

```bash
routiium doctor --env-file .env
routiium serve
```

This profile uses `ROUTIIUM_UPSTREAM_MODE=chat`, so Routiium forwards Chat Completions-shaped requests to a Chat-compatible upstream.

## Path 3: Remote Router or EduRouter

```bash
routiium init --profile router --out .env
```

Edit:

```env
ROUTIIUM_ROUTER_URL=http://127.0.0.1:9090
ROUTIIUM_ROUTER_STRICT=1
ROUTIIUM_ROUTER_PRIVACY_MODE=features
```

Run:

```bash
routiium serve
routiium router probe --model your-router-alias
```

Use `features` privacy for low-data routing, `summary` for summarized conversation hints, and `full` only when the router or judge needs request content.

## Path 4: LLM-as-judge rollout

```bash
routiium init --profile judge --out .env
```

The judge profile sets Routiium-side defaults for strict remote routing and zero local route-cache TTL:

```env
ROUTIIUM_ROUTER_STRICT=1
ROUTIIUM_ROUTER_PRIVACY_MODE=full
ROUTIIUM_CACHE_TTL_MS=0
ROUTER_JUDGE_MODE=shadow
```

Recommended rollout:

1. Start with `shadow` and review router verdict telemetry.
2. Probe representative aliases with `routiium router probe --model <alias>`.
3. Switch to `routiium judge profile enforce --out .env` only after false positives are understood.
4. Keep router-side judged plans at `cache.ttl_ms: 0` when the requirement is “judge every request.”

## Path 5: AWS Bedrock starter

```bash
routiium init --profile bedrock --out .env --config-dir config
```

Edit AWS region/model details and ensure normal AWS credentials are available in the environment or instance role. See [AWS_BEDROCK.md](AWS_BEDROCK.md) for provider-specific details.

## Next steps

- Read [CLI.md](CLI.md) for command details.
- Read [CONFIGURATION.md](CONFIGURATION.md) for env and file options.
- Read [RATE_LIMITS.md](RATE_LIMITS.md) before exposing shared deployments.
- Read [PRODUCTION_HARDENING.md](PRODUCTION_HARDENING.md) before internet-facing production use.
