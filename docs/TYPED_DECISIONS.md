# Jev and typed-decision models

Jev-style models return **typed decisions**, not conversational assistant text.
Routiium supports their `model` / `state` / `questions` contract through
OpenRouter Decisions and TypeSafe System One. Use them to select an approved
upstream model, judge a request, or classify application data through the Rust
`DecisionClient`.

**The decision model and the model answering your user are different.** A client
still sends `model: "auto"` to Routiium's Chat Completions or Responses API.
Routiium asks Jev to choose a catalog candidate, then forwards the original
request to that candidate. Do not put a Jev decision-model ID in a normal chat
request expecting an assistant completion.

## Provider and model IDs

| Setting | OpenRouter | TypeSafe direct |
| --- | --- | --- |
| Provider value | `openrouter` | `typesafe` |
| Default key variable | `OPENROUTER_API_KEY` | `TYPESAFE_API_KEY` |
| Default decision model | `~typesafe/jev-latest` | `jev-latest` |
| Decision endpoint | `https://openrouter.ai/api/alpha/decisions` | `https://api.typesafe.ai/v1/systemone` |

These are Routiium's defaults, not a guarantee of provider availability. Model
IDs are provider-specific; an OpenRouter ID is not interchangeable with a
TypeSafe ID. For reproducibility, set an exact model version supported by your
provider/account instead of a `latest` alias. The quick start below pins
`typesafe/jev-1.13`, the version shown in OpenRouter's Jev documentation.
For TypeSafe direct, obtain a supported versioned ID from its model documentation;
do not infer it by stripping the OpenRouter namespace. Check availability for
your account before deploying.

Other decision models can be used if they support the same typed contract and
answer schema. Routiium does not turn arbitrary chat models into decision models.

## 1. Route with Jev through OpenRouter

Start with a working Routiium upstream configuration. Keep its provider keys,
Router catalog, and managed client-key setup: an OpenRouter decision key does
**not** configure or authenticate the upstream that generates the final answer.
See [Getting Started](GETTING_STARTED.md) for the base setup.

Add these settings to your local `.env` (or your deployment's secret/config
manager):

```dotenv
# Set OPENROUTER_API_KEY privately; do not commit its value.
ROUTIIUM_DECISION_PROVIDER=openrouter
ROUTIIUM_DECISION_API_KEY_ENV=OPENROUTER_API_KEY
ROUTIIUM_DECISION_MODEL=typesafe/jev-1.13
ROUTIIUM_DECISION_PRIVACY_MODE=features
ROUTIIUM_DECISION_TIMEOUT_MS=1000
ROUTIIUM_DECISION_STRICT=0
```

`ROUTIIUM_DECISION_API_KEY_ENV` is the **name** of the variable holding the key,
not the key itself. If your secret is stored under another name, point this
setting at that name. Routing and judging have separate key-variable settings.

For a temporary Bash session, load a key without placing its literal value in
shell history:

```bash
read -rsp 'OpenRouter API key: ' OPENROUTER_API_KEY; printf '\n'
export OPENROUTER_API_KEY
routiium serve
```

Do not use `set -x` around credential handling. Keep `.env` private and out of
Git; use a secret manager in production.

From another terminal, send a normal gateway request using your **Routiium
managed key**, not the OpenRouter provider key. Create one with
`routiium key create --label jev-demo --ttl-seconds 86400` after configuring
admin authentication, then enter the returned key at the prompt below:

```bash
read -rsp 'Routiium client key: ' ROUTIIUM_CLIENT_KEY; printf '\n'
printf 'header = "Authorization: Bearer %s"\n' "$ROUTIIUM_CLIENT_KEY" | \
  curl --config - -sS http://127.0.0.1:8088/v1/chat/completions \
    -H 'Content-Type: application/json' \
    -d '{
      "model": "auto",
      "messages": [{"role": "user", "content": "Explain why leaves are green."}]
    }'
unset ROUTIIUM_CLIENT_KEY
```

The header is passed through stdin, not as a literal credential in shell history
or curl's process arguments. Keep shell tracing disabled. Never paste provider
keys into prompts, request JSON, screenshots, or debug logs.

### What the routing decision does

1. Routiium runs a local safety gate; risky requests may skip remote model
   selection and follow the normal safety-aware Router path instead.
2. It gets healthy, non-deprecated candidates from the Router catalog, filtering
   for known capability/context requirements.
3. It asks a typed `choice` question named `selected_model`.
4. It validates the answer against the supplied candidates and asks the wrapped
   Router to resolve the final upstream plan and credentials.

The default decision-routed aliases are `auto`, `routiium-auto`,
`openai-multimodal`, and `default`. To change them:

```dotenv
ROUTIIUM_DECISION_ROUTE_ALIASES=auto,balanced
```

Other aliases or explicit model IDs bypass this selection unless listed.
A catalog with fewer than two eligible candidates cannot make a remote choice:
it falls back by default, or returns an error with decision strict mode enabled.
Catalog metadata is cached for 60 seconds by default; this is not a cache of
assistant responses.

### Fallback and privacy

- `ROUTIIUM_DECISION_STRICT=0` (default): decision-time errors fall back to the
  normal Router. A successful chat response alone does **not** prove Jev was used.
- `ROUTIIUM_DECISION_STRICT=1`: errors in the enabled decision-routing path are
  returned instead of falling back. This does not override safety bypasses or
  make startup configuration errors fatal; check startup logs too.
- `features` (default): no user prompt content is sent for model selection;
  request features and candidate metadata still go to the decision provider.
- `summary`: opt into bounded, redacted summary context.
- `full`: opt into bounded, redacted recent user content, not an unlimited raw
  transcript. Local safety checks still apply.

`ROUTIIUM_DECISION_PRIVACY_MODE` controls **routing decisions**, not the request
judge or final upstream request. `summary` and `full` can only include context
already collected by `ROUTIIUM_ROUTER_PRIVACY_MODE`: if the Router is set to
`features`, changing decision privacy alone does not restore prompt content.
Redaction is best-effort, not a guarantee that sensitive information is safe to
send externally.

## 2. Use Jev as the request judge

Judging is independent of decision routing. To use OpenRouter for the optional
external LLM judge, add:

```dotenv
ROUTIIUM_JUDGE_MODE=protect
ROUTIIUM_JUDGE_LLM=auto
ROUTIIUM_JUDGE_DECISION_PROVIDER=openrouter
ROUTIIUM_JUDGE_API_KEY_ENV=OPENROUTER_API_KEY
ROUTIIUM_JUDGE_DECISION_MODEL=typesafe/jev-1.13
ROUTIIUM_JUDGE_TIMEOUT_MS=800
```

The judge asks typed questions for verdict (`allow`, `downgrade`, `deny`), risk
level, human approval, and safety categories. Deterministic safety checks and
local policy merging remain active; this does not replace the response guard.

Unlike routing with `features`, the judge sends bounded, redacted request
context and safety policy to the selected external provider. Enable it only
when that data flow is acceptable. `ROUTIIUM_JUDGE_LLM=off` disables external
LLM judging while retaining the configured deterministic checks.

Set the judge's decision model explicitly when changing providers. Otherwise,
`ROUTIIUM_JUDGE_MODEL` / legacy `ROUTER_JUDGE_MODEL` can override the provider's
default and accidentally supply a chat-model ID. The routing model setting
`ROUTIIUM_DECISION_MODEL` does not configure the judge model.

See [Judge Policy](JUDGE_POLICY.md), [Security Model](SECURITY_MODEL.md), and the
[judge environment reference](CONFIGURATION.md#built-in-judge-and-remote-judge-profiles)
for confidence thresholds, failure behavior, and policy controls.

## 3. TypeSafe direct and the legacy Jevai path

For TypeSafe's typed API, change the routing settings to:

```dotenv
ROUTIIUM_DECISION_PROVIDER=typesafe
ROUTIIUM_DECISION_API_KEY_ENV=TYPESAFE_API_KEY
ROUTIIUM_DECISION_MODEL=jev-latest
```

Set `TYPESAFE_API_KEY` privately. For a TypeSafe judge, separately change
`ROUTIIUM_JUDGE_DECISION_PROVIDER`, `ROUTIIUM_JUDGE_API_KEY_ENV`, and
`ROUTIIUM_JUDGE_DECISION_MODEL` to the corresponding TypeSafe values.

The older `JEV_API_KEY` / `ROUTIIUM_JEV_*` integration is **different**: it calls
Jevai's specialized `/api/v1/decisions/model-route` endpoint, not OpenRouter
Decisions. It is considered only when `ROUTIIUM_DECISION_PROVIDER` is unset.
Do not supply an OpenRouter key as `JEV_API_KEY`. For new integrations, prefer
the provider-neutral settings above.

## 4. Application classification (Rust API)

Routiium exports `routiium::decision::DecisionClient`, `DecisionClientConfig`,
`DecisionRequest`, and `DecisionQuestion`. Construct a client for your provider
and send a request with application state and named questions:

```json
{
  "model": "typesafe/jev-1.13",
  "state": {"message": "My invoice was charged twice."},
  "questions": {
    "department": {
      "type": "choice",
      "instructions": "Which team should handle this message?",
      "criteria": {
        "billing": "Payments, invoices, and refunds",
        "technical": "Bugs, outages, and integrations"
      }
    }
  }
}
```

This is the **decision-provider request shape**, not a payload for Routiium's
`/v1/chat/completions`. The gateway does not expose a generic public
`/v1/decisions` endpoint; use the Rust client or the provider's decision API.

Supported question types:

- `noul`: a probability from 0 to 1 for a boolean-like proposition.
- `choice`: select one of 2–255 named options. Probability maps and confidence
  are accepted but may be absent in a compatible provider response.
- `score`: a possibly fractional position on a zero-based ordered scale of
  2–10 descriptions (for three descriptions, the valid range is 0–2).

The Rust client validates requests, limits serialized requests to 32 KiB and
responses to 64 KiB, and validates typed answers. These are JSON byte limits,
not the provider's token context limit. Classifier probabilities are not safety guarantees.
Do not send private application data without reviewing the provider's policies.

## Verification and troubleshooting

- On startup, look for `Typed-decision model routing enabled for aliases:` with
  the expected provider/model. `Unable to initialize decision provider` or
  `Invalid typed-decision configuration` means it was not enabled.
- `routiium router explain` is useful for local Router inspection, but is not
  proof that a live external decision request succeeded. Exercise the running
  gateway with a configured alias and inspect route headers/logs.
- Inspect `x-resolved-model` / `x-route-id` for the final selected upstream;
  these identify the answering model, not the Jev decision model.
- Missing key: check the variable named by `ROUTIIUM_DECISION_API_KEY_ENV`.
- Authentication/model errors: confirm the key belongs to the selected provider
  and the model ID is supported there. Chat Completions base URLs do not replace
  the full decision endpoint.
- Unexpected fallback: check timeouts, eligible catalog candidates, aliases,
  the local safety gate, and `ROUTIIUM_DECISION_STRICT`.
- To override endpoints, use `ROUTIIUM_DECISION_ENDPOINT` for routing or
  `ROUTIIUM_JUDGE_DECISION_ENDPOINT` for judging. Credentials are sent to that
  endpoint; use only a trusted HTTPS destination (local HTTP tests are allowed).
- Mock protocol/redaction tests can be rerun with
  `cargo test --lib --locked decision::tests` and
  `cargo test --lib --locked jev`. These tests do not verify live provider
  availability.

See [Configuration](CONFIGURATION.md#routing-and-router-service) for the full
routing environment reference and [Router Usage](ROUTER_USAGE.md) for catalog
and upstream configuration.

### Provider references

- [OpenRouter Jev guide](https://openrouter.ai/docs/guides/community/jev)
- [OpenRouter Decisions API reference](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-request)
- [TypeSafe documentation](https://docs.typesafe.ai/)

Provider docs describe the external service; Routiium's narrower request shape,
size bounds, routing safety gate, and fallback behavior are defined above.
