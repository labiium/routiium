# Responses Parity Roadmap

This document lists the remaining gaps before Routiium can claim full parity with OpenAI’s Responses API when clients only speak the legacy Chat Completions surface. The checklist is grouped by feature area so contributors can pick up self-contained workstreams.

---

## 1. Stateful Conversations
- **Implicit conversation ids.** Persist a per-client conversation token (e.g., header or issued session id) so `/v1/chat/completions` callers do not need to append `conversation_id` and `previous_response_id` query parameters manually.
- **previous_response replay.** Track prior Responses ids and automatically pass `previous_response_id` to OpenAI when the same conversation token is reused.
- **Fallback storage.** Decide how long to cache conversation metadata (Redis, sled, in-memory TTL) and expose limits in `/status`.

## 2. Instructions/Input Modeling
- **Instructions extraction.** Split the first system message into a dedicated `instructions` field when converting to Responses so upstreams that rely on that separation (e.g., GPT-5, o-series) see canonical payloads.
- **Input array fidelity.** Reshape chat messages into `input: [{role, content:[parts...]}, …]` instead of stuffing them under `input.messages`, aligning with current Responses docs.
- **Multimodal parity.** Convert audio placeholders (`input_audio`) once the chat surface supports it, and add regression tests for `input_image`/`image_url` variations.

## 3. Responses-Only Parameters
- **Reasoning controls.** Surface `reasoning: { effort }` and `max_response_tokens` mapping when callers request reasoning-capable models.
- **Built-in tools.** Allow chat callers to request Responses-only tools (`web_search`, `mcp`, connectors) via an extension field or config so the gateway can attach them downstream.
- **Structured outputs.** Guarantee `response_format` objects satisfy the JSON Schema envelope (name/schema/strict) even when chat clients send the older `json_object` hint.

## 4. Tool Call Lifecycle
- **Typed tool outputs.** Accept Responses-style `function_call_output` objects from clients and weave them back into the conversation automatically before resuming upstream calls.
- **Tool call streaming.** When an upstream streams `function_call.arguments.delta`, forward the typed event to chat clients rather than collapsing it into plain text deltas.
- **Retry semantics.** Document and enforce how many times the proxy will re-invoke a model after a tool response, and expose policy via configuration.

## 5. Streaming Events & Compatibility
- **Typed SSE bridge.** Offer an opt-in mode where chat clients receive actual Responses event types (`response.output_text.delta`, `response.completed`, etc.) so they can migrate gradually without changing the endpoint.
- **Backpressure handling.** Implement buffering and heartbeat timeouts that match OpenAI’s Responses guidance (emit `: keep-alive` every N seconds).

## 6. Response Object Transformation
- **Full response envelope.** When routing through a Responses-capable upstream, forward the entire typed response (status, output array, usage, metadata) instead of only the chat-shaped projection. Provide a feature flag so clients can opt in per request.
- **Error normalization.** Map Responses error payloads (including `status`, `type`, `code`) to chat-style errors today, but also expose the raw payload in a response header for debugging.

## 7. Analytics & Observability
- **Reasoning token tracking.** Capture `usage.output_tokens_details.reasoning_tokens` and store it in the analytics backend alongside prompt/completion tokens.
- **Conversation diagnostics.** Emit structured logs whenever the gateway injects `conversation` or `previous_response_id` so operators can troubleshoot state hand-offs.
- **Tool metrics.** Record latency per tool call and tag whether it originated from Chat or Responses input.

## 8. Test Coverage & Tooling
- **Golden fixtures.** Add JSON golden files for Chat→Responses request conversion, Responses→Chat response translation, and SSE bridging to guard against regressions.
- **Integration suites.** Extend `python_tests` to cover stateful exchanges, reasoning models, structured outputs, and builtin tool invocations routed through `/v1/chat/completions`.
- **Load testing.** Validate sustained streaming throughput and memory usage when running mixed Chat/Responses workloads.

---

### Getting Involved

Each section above can be implemented independently. When contributing, please:
1. Open an issue referencing the bullet you plan to tackle.
2. Include new unit/integration tests that prove the gap is closed.
3. Update `API_REFERENCE.md` and relevant examples if you surface new request parameters or behaviors.

Once the outstanding bullets are addressed, Routiium will provide a truly seamless Responses experience to any Chat Completions client with zero application-side changes.
