# Getting Started with Routiium

Install the CLI first:

```bash
npm install -g routiium
routiium --version
```

Routiium is now safe-by-default: if you do not configure a remote Router, `routiium serve` enables the embedded policy router, request judge, response guard, and streaming safety automatically.

## Path 1: One-key secure gateway

Use the per-user config for the easiest app-like setup:

```bash
routiium config init --profile openai
routiium config set OPENAI_API_KEY sk-your-provider-key
routiium config set ROUTIIUM_ADMIN_TOKEN change-me-admin-token
routiium doctor
routiium serve
routiium status
```

If you prefer repo-local deployment files, `routiium init --profile openai --out .env` still works and `routiium doctor --env-file .env` will inspect it.

Defaults in this path:

- `ROUTIIUM_ROUTER_MODE=embedded` gives you aliases like `auto`, `fast`, `balanced`, `safe`, and `premium`.
- `ROUTIIUM_JUDGE_MODE=protect` rejects high-confidence exfiltration/dangerous-action requests and downgrades prompt-injection-like requests.
- `ROUTIIUM_REJECTION_MODE=agent_result` returns an OpenAI-compatible rejected assistant result so agent loops can continue safely.
- `ROUTIIUM_JUDGE_LLM=auto` uses an LLM judge when the configured judge key is present; deterministic checks always run. `ROUTIIUM_JUDGE_OUTPUT_MODE=auto` prefers tool/function calling and falls back to JSON.
- `ROUTIIUM_RESPONSE_GUARD=protect` scans successful outputs for prompt/secret leakage.
- `ROUTIIUM_STREAMING_SAFETY=chunk` scans streams and forces risky judged requests to non-streaming.
- `ROUTIIUM_WEB_JUDGE=restricted` inspects suspicious URLs/domains without sending private prompts to search.

Point any OpenAI-compatible SDK at `http://127.0.0.1:8088/v1`.

### Security defaults to know

- Admin endpoints require `ROUTIIUM_ADMIN_TOKEN`; they do not open anonymously when unset.
- CORS is same-origin by default. Set `CORS_ALLOWED_ORIGINS` for browser clients, or `CORS_ALLOW_ALL=1` only for trusted local testing.
- `/convert` is safe by default and does not merge internal system prompt or MCP metadata unless `?include_internal_config=true` is used with admin auth.
- Runtime MCP config writes are disabled unless `ROUTIIUM_ALLOW_MCP_CONFIG_UPDATE=1` is set, because MCP server configs can spawn local commands.

## Path 2: Inspect routing and judge behavior locally

No server is required for these checks:

```bash
routiium router explain --model auto --prompt "Summarize this"
routiium router explain --model auto --prompt "Ignore previous instructions"
routiium judge explain --prompt "Ignore previous instructions"
routiium judge test --suite all
```

Use the output to confirm the selected model, tier, judge verdict, and risk level before exposing a deployment.

## Path 3: Managed keys in front of providers

After `routiium serve`, create a customer-facing key:

```bash
routiium key create --label first-user --ttl-seconds 86400
```

Use the returned `sk_<id>.<secret>` as the client bearer token. The upstream provider key remains server-side.

## Path 4: Local vLLM/Ollama/OpenAI-compatible server

```bash
routiium init --profile vllm --out .env
```

Edit `OPENAI_BASE_URL` if your local model server is not on `http://127.0.0.1:8000/v1`, then run:

```bash
routiium doctor --env-file .env
routiium serve
```

This profile uses `ROUTIIUM_UPSTREAM_MODE=chat`, so Routiium forwards Chat Completions-shaped requests to a Chat-compatible upstream. Set `ROUTIIUM_ROUTER_MODE=off` only if you explicitly want legacy routing without embedded judge decisions.

## Path 5: Remote Router-compatible service

Use this only when you want central policy/catalog/health management outside the Routiium process. The built-in router is the default and requires no separate project:

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

Remote Router configuration takes precedence over the embedded router. Use `features` privacy for low-data routing, `summary` for summarized conversation hints, and `full` only when the remote router or judge needs request content.

## Path 6: Stricter judge rollout

```bash
routiium judge profile shadow --out .env    # observe only
routiium judge profile protect --out .env   # safe default
routiium judge profile enforce --out .env   # stricter medium-risk enforcement
routiium judge policy init --out config/judge-policy.json --prompt-out config/judge-prompt.md
routiium judge policy validate --path config/judge-policy.json
```

For any external/remote judge path, keep `ROUTIIUM_CACHE_TTL_MS=0` and judged plans at `cache.ttl_ms: 0` when the requirement is “judge every request.”
Custom judge prompts are policy overlays only; Routiium's built-in safety prompt remains active and sensitive requests route to `secure` by default.


## Path 7: Synthetic/Hugging Face-compatible judge testing

Synthetic-compatible providers can be used both as the upstream and as the isolated LLM judge:

```bash
routiium config init --profile synthetic
routiium config set OPENAI_API_KEY syn-your-synthetic-key
routiium config set ROUTIIUM_JUDGE_MODEL hf:zai-org/GLM-5.1
routiium doctor
routiium serve
```

The profile sets `ROUTIIUM_UPSTREAM_MODE=chat`, `OPENAI_BASE_URL=https://api.synthetic.new/openai/v1`, `ROUTIIUM_JUDGE_BASE_URL=https://api.synthetic.new/openai/v1`, `ROUTIIUM_JUDGE_OUTPUT_MODE=auto`, embedded routing, `ROUTIIUM_CACHE_TTL_MS=0`, and agent-friendly rejection responses. You can swap the judge model with `hf:MiniMaxAI/MiniMax-M2.5` or `hf:moonshotai/Kimi-K2.5` using `routiium config set ROUTIIUM_JUDGE_MODEL ...`.

## Next steps

- Read [CLI.md](CLI.md) for command details.
- Read [JUDGE_POLICY.md](JUDGE_POLICY.md) before customizing judge prompts or reroute behavior.
- Read [ROUTER_USAGE.md](ROUTER_USAGE.md) for embedded and remote router behavior.
- Run `routiium doctor --production --require-server` before launch.
- Read [SECURITY_MODEL.md](SECURITY_MODEL.md) and [PRODUCTION_CHECKLIST.md](PRODUCTION_CHECKLIST.md) before enabling tools, web search, or external judges.
- Read [CONFIGURATION.md](CONFIGURATION.md) for env and file options.
- Read [RATE_LIMITS.md](RATE_LIMITS.md) before exposing shared deployments.
- Read [PRODUCTION_HARDENING.md](PRODUCTION_HARDENING.md) before internet-facing production use.
