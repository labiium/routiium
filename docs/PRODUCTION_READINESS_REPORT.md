# Routiium LLM Gateway Production Readiness Report

Status: production-ready for the audited OpenAI-compatible gateway profile, with the documented operational constraints below.

This report tracks the implementation state of the LLM gateway hardening plan captured in `.omx/plans/llm-gateway-implementation-plan.md` and `.omx/ultragoal/goals.json`.

## Completed

- Gateway contract and topology ADR exist in `docs/gateway-contract.md`.
- Package and crate identity now describe Routiium as an OpenAI-compatible LLM gateway instead of only a converter.
- Streaming usage fixtures cover Chat Completions, Responses `response.completed`, and Bedrock metadata events.
- Streaming analytics records response bytes, usage source, TTFB, upstream duration, and terminal events through a completion/drop finalizer.
- Analytics has explicit `usage_source`: `measured`, `estimated`, or `absent`.
- `/ready` reports analytics-store and upstream/router readiness.
- `/metrics` exposes low-cardinality Prometheus text counters for request count, 5xx errors, duration sum, and usage-source counts.
- `scripts/load/gateway_load.py` provides p50/p95/p99 latency and TTFB gates without third-party Python dependencies.
- `/analytics/governance` reports measured usage, unpriced measured events, and whether budget/token-limit reporting is authoritative for a selected window.
- `/ready` now has a Router-delegated governance mode: `ROUTIIUM_GOVERNANCE_MODE=router` fails closed unless Router is active, strict mode is enabled, and router plan caching is disabled.
- Opt-in exact-match response caching exists for non-streaming responses with explicit safe route metadata, request no-store handling, TTL bounds, admin invalidation, and measured cache-hit analytics.
- `RoutePlan::ordered_attempts()` projects primary plus ordered fallback hops.
- Non-streaming Chat and Responses handlers execute bounded fallback attempts for transport errors, 429, and 5xx before emitting client bytes.
- AWS SDK dependencies use the modern HTTPS client path; legacy Rustls AWS paths are excluded.
- Turso/libsql chat-history support is disabled in this production build because its current dependency graph pulls vulnerable Rustls packages.
- `cargo audit` exits successfully; remaining audit output is limited to allowed unmaintained warnings through the optional embedded `sled` backend.

## Production Constraints

- Local fleet-wide budget and token-limit enforcement is not implemented. Deployments may explicitly delegate enforcement to Router with `ROUTIIUM_GOVERNANCE_MODE=router`, but `/ready` requires strict Router behavior and disabled route-plan cache before accepting traffic.
- Exact-match response caching is in-memory per gateway process. Production deployments that need cross-node cache coherence still need an external cache design.
- Router failover analytics do not yet persist the full attempt list, fallback reasons, or terminal hop metadata beyond request logs and route headers.
- OpenAPI/API reference coverage for `/ready`, `/metrics`, and `/analytics/governance` is incomplete.
- Turso/libsql chat history returns a configuration error. Use `memory`, `jsonl`, `sqlite`, or `postgres` until libsql moves to a Rustls/webpki version that clears RustSec.

## Required For Wider Production Profiles

1. Add persisted failover attempt metadata, terminal provider, and route hop details without double-counting cost.
2. Keep streaming failover disabled after partial output; add explicit docs/tests for that guarantee if needed.
3. Add Redis-backed local budget/token enforcement, or operate with the tested fail-closed Router-delegated mode.
4. Decide whether per-process exact response caching is sufficient or whether production requires a shared external response cache.
5. Expand OpenAPI/API reference coverage for `/ready`, `/metrics`, `/analytics/governance`, and `/admin/cache/exact/clear`.
6. Re-enable Turso/libsql only after its dependency graph clears RustSec advisories.

## Current Verification Evidence

- `cargo fmt --check`
- `cargo test --test streaming_metering_tests`
- `cargo test --test admin_endpoints_tests`
- `cargo test --test router_integration`
- `cargo test route_plan_ordered_attempts_preserve_primary_and_fallbacks`
- `cargo test --no-run`
- `cargo check --all-features`
- `python3 -m py_compile scripts/load/gateway_load.py`
- `python3 scripts/load/gateway_load.py --help`
- `cargo audit`

This evidence proves the audited gateway profile compiles and is covered by focused tests. It does not prove every wider deployment profile, especially local distributed budget enforcement or Turso/libsql chat history.
