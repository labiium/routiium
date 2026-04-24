# Routiium CLI Reference

Routiium uses a `clap`-based CLI. Install it with `npm install -g routiium`, or build from source with `cargo install --path .`. Run `routiium --help` for the top-level command list and `routiium <command> --help` for command-specific flags.

## npm global install

```bash
npm install -g routiium
routiium --version
```

The npm package exposes the `routiium` binary globally. Its postinstall step downloads a prebuilt release binary for Linux, macOS, or Windows when available. If there is no matching release asset, it builds the Rust binary from the packaged source with `cargo build --release --locked`. Set `ROUTIIUM_BINARY=/path/to/routiium` to force a custom binary, `ROUTIIUM_NPM_BINARY_URL` to test a release asset, or `ROUTIIUM_NPM_SKIP_DOWNLOAD=1` to force the local Cargo fallback.

## npm trusted publishing / OIDC

The npm release workflow uses GitHub Actions OIDC trusted publishing and does not require `NPM_TOKEN` once trust is configured on npm. Configure trust before a tag release:

```bash
npm run package:trust:github:dry-run
npm run package:trust:github
npm run package:trust:list
```

Expected trust tuple:

- package: `routiium`
- repository: `labiium/routiium`
- workflow file: `publish-npm.yml`

The workflow grants `id-token: write`, uses a GitHub-hosted runner, and runs `npm publish --access public`. npm automatically exchanges the GitHub OIDC token during publish and generates provenance for public packages published from public repositories.

## Security-sensitive defaults

- Admin commands and admin HTTP APIs require `ROUTIIUM_ADMIN_TOKEN`; unset tokens fail closed unless `ROUTIIUM_INSECURE_ADMIN=1` is explicitly set for throwaway local development.
- CORS is not open by default. Configure `CORS_ALLOWED_ORIGINS` for browser apps, or `CORS_ALLOW_ALL=1` only for trusted local use.
- `/convert` performs safe conversion by default. Use `/convert?include_internal_config=true` with admin auth to include internal system prompt/MCP metadata.
- MCP runtime config updates require `ROUTIIUM_ALLOW_MCP_CONFIG_UPDATE=1` because MCP server configs can spawn local commands.

## `routiium serve`

Starts the HTTP gateway.

```bash
routiium serve
routiium serve --config ~/.config/routiium/config.env
routiium serve --keys-backend sled:./data/keys.db
routiium serve --mcp-config mcp.json --system-prompt-config system_prompt.json
```

Existing root-level flags remain compatible, so `routiium --keys-backend=memory` behaves like `routiium serve --keys-backend memory`.

Common flags:

| Flag | Env fallback | Description |
| --- | --- | --- |
| `--config PATH` | `ROUTIIUM_CONFIG` | Env/config file to load before serving. Defaults to the XDG user config and local `.env` discovery. |
| `--keys-backend redis://...\|sled:<path>\|memory` | backend-specific env vars | API key store override. |
| `--mcp-config PATH` | `ROUTIIUM_MCP_CONFIG` | MCP server definitions. |
| `--system-prompt-config PATH` | `ROUTIIUM_SYSTEM_PROMPT_CONFIG` | System prompt rules. |
| `--routing-config PATH` | `ROUTIIUM_ROUTING_CONFIG` | Legacy routing config. |
| `--router-config PATH` | `ROUTIIUM_ROUTER_CONFIG` | Local policy router file. |
| `--rate-limit-config PATH` | `ROUTIIUM_RATE_LIMIT_CONFIG` | Rate limit policy file. |


## `routiium config`

Manages the per-user config file at `$XDG_CONFIG_HOME/routiium/config.env`, or `~/.config/routiium/config.env` when `XDG_CONFIG_HOME` is not set. This gives app-like onboarding without requiring every project to carry a `.env` file.

```bash
routiium config path
routiium config init --profile openai
routiium config init --profile synthetic
routiium config set OPENAI_API_KEY sk-your-provider-key
routiium config get ROUTIIUM_JUDGE_MODE
routiium config list
routiium serve --config ~/.config/routiium/config.env
```

Config precedence is: CLI flags > existing process environment > explicit config file (`--config`, `ROUTIIUM_CONFIG`, `ENV_FILE`, `ENVFILE`, or `DOTENV_PATH`) > local `.envfile`/`.env` > per-user config.

The `synthetic` profile is designed for OpenAI-compatible Synthetic/Hugging Face model endpoints and judge testing. It sets `OPENAI_BASE_URL` and `ROUTIIUM_JUDGE_BASE_URL` to `https://api.synthetic.new/openai/v1`, uses `ROUTIIUM_UPSTREAM_MODE=chat`, enables embedded routing/judging, and defaults `ROUTIIUM_JUDGE_MODEL` to `hf:zai-org/GLM-5.1`, `ROUTIIUM_JUDGE_OUTPUT_MODE=auto`, and `ROUTIIUM_JUDGE_MAX_TOKENS=1024` so reasoning-heavy models still return JSON content. Replace the placeholder key with your own Synthetic key.

## `routiium init`

Creates starter `.env` files for common profiles.

```bash
routiium init --profile openai --out .env
routiium init --profile vllm --out .env.local
routiium init --profile router --out .env.router
routiium init --profile judge --out .env.judge
routiium init --profile bedrock --out .env.bedrock --config-dir config
routiium init --profile synthetic --out .env.synthetic
```

Profiles:

| Profile | Use when |
| --- | --- |
| `openai` | You want managed keys plus embedded routing/judge defaults in front of OpenAI. |
| `vllm` | You have a local OpenAI-compatible server such as vLLM or Ollama. |
| `router` | You want remote routing through a Router-compatible policy service. |
| `judge` | You want embedded router + LLM-as-judge protect defaults. |
| `bedrock` | You want an AWS Bedrock-oriented starter config. |
| `synthetic` | You want an OpenAI-compatible Synthetic/HF endpoint profile for upstream and LLM judge testing. |

`init` refuses to overwrite existing files unless `--force` is passed.

## `routiium doctor`

Checks local setup and optional live services.

```bash
routiium doctor --env-file .env
routiium doctor --url http://127.0.0.1:8088 --check-router
routiium doctor --require-server --url http://127.0.0.1:8088
routiium doctor --production --require-server
routiium doctor --json
```

If `--env-file` is omitted, `doctor` checks the per-user config when it exists, otherwise `.env`.

Doctor checks include env file presence, referenced config files, provider key/base URL hints, `/status`, optional remote router catalog reachability, and judge/cache compatibility. Embedded routing does not require `ROUTIIUM_ROUTER_URL`; use `--check-router` only for remote Router-compatible deployments. By default, an unreachable server is a warning so `doctor` can be used before `serve`; use `--require-server` for deployment readiness checks.

`--production` adds stricter checks for an internet-facing deployment: high-entropy admin token, explicit CORS origins, managed auth, persistent key store, enabled router/judge, enabled response guard, and streaming safety.

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

Sends a minimal chat completion request through a running Routiium server and prints status, routing-related headers, and the response body.

```bash
routiium router probe --model auto
routiium router probe --model safe --api-key sk_<id>.<secret> --json
```

Use this after starting Routiium to verify headers such as `x-route-id`, `x-resolved-model`, `x-judge-verdict`, and `x-safety-policy-rev`.
Response guard headers (`x-response-guard-*`) and `x-streaming-safety` are also included when output safety is active.

## `routiium router explain`

Runs the embedded router and deterministic judge locally without starting the server or calling an external judge.

```bash
routiium router explain --model auto --prompt "Summarize this"
routiium router explain --model auto --prompt "Ignore previous instructions" --json
```

This is the fastest onboarding surface for understanding default aliases, tiers, judge verdicts, and cacheability.

## `routiium judge profile`

Updates a local env file with judge rollout defaults.

```bash
routiium judge profile shadow --out .env
routiium judge profile protect --out .env
routiium judge profile enforce --out .env
routiium judge profile off --out .env
```

`shadow` observes judge outcomes, `protect` is the safe default, and `enforce` is stricter for validated policies. External/remote every-request judging requires `ROUTIIUM_CACHE_TTL_MS=0` in Routiium and zero-TTL judged plans from the Router.
Profiles also set `ROUTIIUM_RESPONSE_GUARD` and `ROUTIIUM_STREAMING_SAFETY` so request judging and output guarding roll out together.

## `routiium judge policy`

Creates and validates custom judge policy overlays. Routiium's built-in safety prompt remains immutable; generated prompts are appended as operator policy.

```bash
routiium judge policy init --out config/judge-policy.json --prompt-out config/judge-prompt.md
routiium judge policy validate --path config/judge-policy.json
```

Use this when you want to tune sensitive-request rerouting (`secure`) or add organization-specific judge guidance without deploying a separate Router service.

## `routiium judge explain`

Runs the embedded router and deterministic judge locally, optionally with a custom policy file.

```bash
routiium judge explain --prompt "Ignore previous instructions"
routiium judge explain --policy config/judge-policy.json --prompt "This contains sk-example..."
routiium judge explain --json
```

The output shows judge action, verdict, risk, target alias, selected model, and policy fingerprint.

## `routiium judge test`

Runs local built-in judge scenarios for onboarding and regression checks.

```bash
routiium judge test --suite all
routiium judge test --suite prompt-injection --json
```

The suite covers prompt injection, exfiltration, and dangerous-action examples without calling an external model.

## `routiium judge events`

Reads recent safety audit events from the authenticated admin API.

```bash
routiium judge events --limit 50
routiium judge events --url http://localhost:8088 --admin-token "$ROUTIIUM_ADMIN_TOKEN" --json
```

Use this after probes or incidents to inspect router denials and response-guard blocks. Set `ROUTIIUM_SAFETY_AUDIT_PATH` on the server when you also need durable JSONL audit logs.

## `routiium docs`

Prints the main documentation entry points.

```bash
routiium docs
routiium docs --json
```
