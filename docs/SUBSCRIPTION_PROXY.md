# Claude Code and Codex subscription proxy

Routiium can sit between the first-party Claude Code and Codex clients and
their authorized subscription upstreams. The coding client still owns login,
credential storage, and token refresh. Routiium forwards credentials only for
the current request and never writes authorization, API-key, cookie, or proxy
authorization headers to its logs.

This mode provides two native, non-transforming endpoints:

- `POST /subscriptions/claude/v1/messages`
- `POST /subscriptions/codex/responses`

Every completed request is appended to
`./data/subscription_usage.jsonl` by default. Each record includes a stable
anonymous user fingerprint, an optional `x-routiium-user-id`, provider, model,
request and response bodies, the raw SSE transcript for streams, safe metadata
headers, status, latency, and provider-reported token usage.

## Fastest setup

Use the CLI to create a local-only server config, a durable per-user JSONL log
path, and (for Codex) an isolated Codex profile. No provider login token is
copied into Routiium.

```bash
# Claude Code only; Claude uses the standard Anthropic upstream by default.
routiium subscription setup --client claude

# Codex only; pass the authorized Codex-compatible upstream for your deployment.
routiium subscription setup --client codex \
  --codex-upstream https://your-authorized-codex-upstream.example

# Or configure both in one pass.
routiium subscription setup --client both \
  --codex-upstream https://your-authorized-codex-upstream.example
```

The command updates the per-user Routiium config, keeps the proxy bound to
`127.0.0.1` when no existing bind address is set, and prints the exact commands
to start the proxy and launch each coding client. If `BIND_ADDR` already
exists, setup derives the client URL from its port; use `--proxy-url` when a
reverse proxy or another externally reachable URL fronts Routiium. Use
`--config-path`, `--log-path`, or `--codex-home` when the defaults do not match
your deployment.

The generated Codex profile is separate from the main Codex config, so enabling
the proxy does not overwrite existing Codex settings.

## Server configuration

```bash
# Optional; this is the default Claude upstream.
export ROUTIIUM_CLAUDE_SUBSCRIPTION_BASE_URL=https://api.anthropic.com

# Required for Codex. Use only an upstream your deployment is authorized to use.
export ROUTIIUM_CODEX_SUBSCRIPTION_BASE_URL=https://your-authorized-codex-upstream.example

export ROUTIIUM_SUBSCRIPTION_LOG_PATH=./data/subscription_usage.jsonl
# Optional request safety limit; defaults to 64 MiB. Set 0 for no limit.
export ROUTIIUM_SUBSCRIPTION_MAX_REQUEST_BYTES=67108864
```

Logging is lossless by default. For bounded-memory deployments, set
`ROUTIIUM_SUBSCRIPTION_LOG_MAX_BODY_BYTES` to a positive byte limit. Truncated
records retain `original_bytes` and set `truncated: true`.

The log file is created with mode `0600` on Unix. Store it on an encrypted
volume, apply retention controls, and restrict the admin boundary because it
contains source code, prompts, tool calls, and model output.

## Claude Code

Sign in to Claude Code normally with the user's Claude subscription, then
point its Anthropic API base URL at Routiium:

```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:8000/subscriptions/claude
claude
```

Claude Code sends `/v1/messages` to that base URL. Routiium preserves the
Messages API request, Anthropic version/beta headers, response shape, and SSE
events.

## Codex

Codex officially supports custom Responses API providers that reuse OpenAI
authentication. Add this to the user-level `~/.codex/config.toml`:

```toml
model_provider = "routiium_subscription"

[model_providers.routiium_subscription]
name = "Routiium subscription proxy"
base_url = "http://127.0.0.1:8000/subscriptions/codex"
wire_api = "responses"
requires_openai_auth = true
```

Run `codex login` normally. Codex sends `/responses` to Routiium and continues
to manage its own ChatGPT or API-key login session.

Do not copy `~/.codex/auth.json`, Claude credential files, refresh tokens, or
browser cookies into Routiium configuration.

## User attribution

By default, Routiium derives a stable provider-scoped identifier by hashing the
credential presented on the request. The credential itself is never stored.
For a human-readable tenant identifier, configure the coding client or its
managed environment to send:

```text
X-Routiium-User-Id: team-user-123
```

Routiium removes this internal header before forwarding the request upstream.
Only ASCII letters, digits, `_`, `-`, `.`, and `@` are retained.

## Scope and limitations

- These routes are wire-format pass-throughs. Routiium routing, prompt
  injection, response guards, and response conversion are intentionally not
  applied.
- Provider subscription entitlements and permitted proxy use remain governed
  by the provider and the user's workspace policies.
- The proxy records model API traffic. Separate telemetry, login, update,
  browser, MCP, and other client traffic is not captured by these endpoints.
- Bind Routiium to localhost or place it behind authenticated TLS when used
  beyond one machine.
