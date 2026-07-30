# Gateway Contract

This document is the production contract for Routiium as an OpenAI-compatible LLM gateway. It defines the request accounting and topology invariants that new proxying, observability, failover, and governance work must preserve.

## Request Accounting Invariants

Every terminal gateway request emits exactly one analytics event. Terminal means the request reached a final gateway outcome: upstream success, upstream error, policy rejection, rate-limit rejection, gateway validation error, or client disconnect after the gateway accepted the request.

Successful non-streaming and streaming requests both record:

- request size
- response size
- HTTP status
- endpoint or route
- authenticated principal when available
- model and terminal provider when known
- time to first byte
- upstream duration
- token usage when provider data exists
- usage source: `measured`, `estimated`, or `absent`

Streaming requests are not exempt from usage accounting. If a provider emits terminal usage, Routiium records it as `measured`. If usage must be inferred after an incomplete or provider-silent stream, Routiium records `estimated`. If no defensible usage value exists, Routiium records `absent`. Missing usage is an observable state, not a silent zero.

Client disconnects after request acceptance still emit exactly one analytics event. The event must include the bytes and timing observed before disconnect and must set usage source to `estimated` or `absent` unless measured provider usage was already received.

Retries and fallback attempts record attempt metadata without double-counting cost. A logical request has one terminal analytics event; retry/fallback details are attached as attempts or route hops.

## Topology ADR

Status: accepted for production planning.

Routiium supports single-instance development and production deployments, but production governance features are fleet-safe only when they use a shared atomic store.

Memory and local Sled storage are acceptable for development, tests, local demos, and explicitly single-node deployments. They are not acceptable sources of truth for production budget enforcement, token limits, circuit state, or shared rate-limit counters across more than one gateway process.

Redis or another shared atomic store is required before enabling production budget or token-limit enforcement across a fleet. Operators may still run analytics sinks such as JSONL or local stores for observability, but those sinks must not be treated as fleet-wide enforcement state.

## Phase Gates

Observability can ship in layers. RED metrics, readiness, and error counters may ship before token and cost metrics are fully correct. Token, cost, budget, and cache decisions require correct usage-source handling first.

Provider expansion is deferred until the shared gateway path is honest and measurable. New providers must satisfy the same request accounting invariants as existing OpenAI-compatible and Bedrock paths.
