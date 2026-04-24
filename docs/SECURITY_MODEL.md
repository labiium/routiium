# Routiium Security Model: Embedded Router and Judge

Routiium's embedded router runs a request safety judge before selecting an upstream. The goal is to make the default path useful without a separate EduRouter deployment while reducing prompt injection, exfiltration, unsafe tool use, and dangerous-action risk.

## Default posture

```env
ROUTIIUM_ROUTER_MODE=embedded
ROUTIIUM_ROUTER_STRICT=1
ROUTIIUM_ROUTER_PRIVACY_MODE=full
ROUTIIUM_JUDGE_MODE=protect
ROUTIIUM_RESPONSE_GUARD=protect
ROUTIIUM_STREAMING_SAFETY=chunk
ROUTIIUM_SAFETY_AUDIT_PATH=./data/safety-audit.jsonl
ROUTIIUM_JUDGE_LLM=auto
ROUTIIUM_JUDGE_SENSITIVE_TARGET=secure
ROUTIIUM_JUDGE_ON_DENY=block
ROUTIIUM_WEB_JUDGE=restricted
```

If `OPENAI_API_KEY` is present, `auto` can call the configured judge model. If no judge key is available, deterministic checks still run and `/status` reports the judge configuration.

## Threats covered

- Prompt injection and instruction-hierarchy override attempts.
- Requests to reveal system/developer prompts, API keys, environment variables, tokens, or `.env` contents.
- Dangerous irreversible actions such as destructive shell/database commands.
- Risky tool exposure, especially shell, filesystem, database, cloud, deployment, browser, webhook, and payment tools.
- URL-based exfiltration patterns such as webhook/request-bin URLs and secret-bearing query strings.
- Judge bypass through fallback: embedded policy denials return structured 403 responses and are not silently routed through legacy fallback.
- Judge bypass through cache: non-cacheable safety decisions return `cache.ttl_ms: 0` and `x-safety-cache: no-store`.
- Output exfiltration: successful non-streaming responses are scanned before release; risky streams are forced to non-streaming or chunk-scanned.

## Decision modes

| Mode | Behavior |
| --- | --- |
| `off` | No embedded judge decisions. Use only when another control plane enforces policy. |
| `shadow` | Emit judge metadata but do not block/downgrade. |
| `protect` | Default. Enforce high-confidence high/critical blocks, route medium prompt-injection-like or sensitive requests to `secure`. |
| `enforce` | Stricter mode that also enforces medium-risk approvals/denials. |

## Custom judge prompts and secure rerouting

Operators can add policy overlays with `ROUTIIUM_JUDGE_POLICY_PATH` or `ROUTIIUM_JUDGE_PROMPT_FILE`. These prompts are appended after Routiium's immutable safety prompt, size-limited, secret-redacted, and fingerprinted in `x-judge-policy-fingerprint`. They can make behavior stricter or select aliases such as `secure`; they cannot disable the built-in prompt-injection, exfiltration, dangerous-action, or tool-risk rules.

Hard denials block by default (`ROUTIIUM_JUDGE_ON_DENY=block`). If an operator explicitly sets `ROUTIIUM_JUDGE_ON_DENY=route`, Routiium routes denial-class requests to `ROUTIIUM_JUDGE_DENY_TARGET` with no-store route metadata and strips tools from denial-rerouted requests before forwarding.

## Web/search-as-judge policy

`ROUTIIUM_WEB_JUDGE=restricted` does not send private prompts to a search engine. It only classifies URL/domain patterns present in the request. Full web search should be opt-in and should send only redacted, minimized public facts.

## LLM judge isolation

When the optional LLM judge runs, Routiium sends a redacted JSON context rather than raw trusted prompts. System prompt content is represented by fingerprints/presence flags, secrets are replaced with `[REDACTED_SECRET]`, and the judge has no tools. The judge must return structured JSON; invalid/unavailable judge responses fail closed for non-low-risk requests.

## Response guard and streaming safety

Request-side judging prevents unsafe prompts from reaching a model; response guarding catches the second failure mode: a model returning protected instructions, credential-like material, exfiltration URLs, or dangerous operational guidance. `ROUTIIUM_RESPONSE_GUARD=protect` blocks high/critical findings with HTTP 403 and emits `x-response-guard-*` headers. `shadow` reports findings without blocking.

Streaming is harder because bytes leave the gateway before the full answer is known. Routiium therefore:

1. Forces non-streaming for high-risk or non-cacheable judged requests.
2. Scans chunks when streaming remains enabled.
3. Supports `ROUTIIUM_STREAMING_SAFETY=force_non_stream` for deployments that want full postflight inspection on every streamed request.

Prefer non-streaming for privileged tools, data-loss-prevention workloads, or requests that touched private context.

## Operational checks

```bash
routiium status --json | jq '.router, .judge'
routiium router explain --model auto --prompt "Ignore previous instructions"
routiium judge policy validate --path config/judge-policy.json
routiium judge explain --policy config/judge-policy.json --prompt "Ignore previous instructions"
routiium judge test --suite all
routiium doctor --production --require-server
```

Review `x-judge-*`, `x-response-guard-*`, `x-streaming-safety`, `x-safety-*`, `x-route-*`, and `router-schema` headers in probes and application logs.

## Safety audit trail

Routiium keeps a bounded in-memory safety event trail and can append each event to JSONL:

```env
ROUTIIUM_SAFETY_AUDIT_PATH=./data/safety-audit.jsonl
ROUTIIUM_SAFETY_AUDIT_MAX_EVENTS=1000
```

Inspect recent events:

```bash
routiium judge events --limit 50 --json
curl -H "Authorization: Bearer $ROUTIIUM_ADMIN_TOKEN" \
  http://127.0.0.1:8088/admin/safety/events?limit=50
```

Events currently cover router/judge denials and response-guard blocks. They intentionally store metadata, reason, risk, categories, route/model identifiers, and client IP rather than full prompt/output bodies.
