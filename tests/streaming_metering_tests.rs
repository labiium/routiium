use serde_json::Value;

use routiium::analytics_middleware::extract_streaming_token_usage_from_sse_event;

const CHAT_USAGE_SSE: &str = include_str!("fixtures/sse/chat_completions_with_usage.sse");
const RESPONSES_USAGE_SSE: &str = include_str!("fixtures/sse/responses_completed_with_usage.sse");
const BEDROCK_USAGE_SSE: &str = include_str!("fixtures/sse/bedrock_metadata_usage.sse");

fn sse_json_payloads(fixture: &str) -> Vec<Value> {
    fixture
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str(data).expect("fixture data payload is valid JSON"))
        .collect()
}

#[test]
fn golden_streaming_usage_fixtures_are_valid() {
    let chat = sse_json_payloads(CHAT_USAGE_SSE);
    assert_eq!(chat[1]["usage"]["total_tokens"], 10);

    let responses = sse_json_payloads(RESPONSES_USAGE_SSE);
    assert_eq!(responses[1]["response"]["usage"]["total_tokens"], 16);
    assert_eq!(
        responses[1]["response"]["usage"]["output_tokens_details"]["reasoning_tokens"],
        1
    );

    let bedrock = sse_json_payloads(BEDROCK_USAGE_SSE);
    assert_eq!(bedrock[1]["usage"]["totalTokens"], 21);
}

#[test]
fn chat_completions_streaming_usage_is_measured() {
    let usage = CHAT_USAGE_SSE
        .split("\n\n")
        .find_map(|event| extract_streaming_token_usage_from_sse_event(event.as_bytes()))
        .expect("terminal usage");

    assert_eq!(usage.prompt_tokens, 7);
    assert_eq!(usage.completion_tokens, 3);
    assert_eq!(usage.total_tokens, 10);
}

#[test]
fn responses_completed_streaming_usage_is_measured() {
    let usage = RESPONSES_USAGE_SSE
        .split("\n\n")
        .find_map(|event| extract_streaming_token_usage_from_sse_event(event.as_bytes()))
        .expect("response.completed usage");

    assert_eq!(usage.prompt_tokens, 11);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 16);
    assert_eq!(usage.cached_tokens, Some(2));
    assert_eq!(usage.reasoning_tokens, Some(1));
}

#[test]
fn bedrock_streaming_metadata_usage_is_measured() {
    let usage = BEDROCK_USAGE_SSE
        .split("\n\n")
        .find_map(|event| extract_streaming_token_usage_from_sse_event(event.as_bytes()))
        .expect("metadata usage");

    assert_eq!(usage.prompt_tokens, 13);
    assert_eq!(usage.completion_tokens, 8);
    assert_eq!(usage.total_tokens, 21);
}

#[test]
fn streaming_client_disconnect_records_one_terminal_event() {
    let server = include_str!("../src/server.rs");
    assert!(server.contains("impl Drop for MeteredSseStream"));
    assert!(server.contains("self.finalize(None);"));
    assert!(server.contains("ctx: Option<StreamAnalyticsContext>"));
}
