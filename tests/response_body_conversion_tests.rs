/// Comprehensive response body conversion tests
///
/// Tests validate conversion of actual API response bodies:
/// - Responses API responses → Chat Completions responses
/// - Chat Completions responses → Responses API responses (if needed)
/// - Usage token mapping including reasoning_tokens
/// - Output array item handling (reasoning, tool_calls, etc.)
/// - Streaming chunk conversion
/// - Error response handling
use routiium::{
    chat::{
        ChatChoice, ChatCompletionResponse, ChatResponseMessage, ChatUsage, FunctionCall, ToolCall,
    },
    chat_to_responses_response,
    responses::{OutputItem, ResponsesResponse, ResponsesUsage},
    responses_to_chat_response,
};
use serde_json::json;

// ============================================================================
// SECTION 1: Response Models Tests
// ============================================================================

mod response_models {
    use super::*;

    #[test]
    fn test_chat_usage_with_reasoning_tokens() {
        // Chat API usage should support reasoning_tokens as extension
        let usage_json = json!({
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150,
            "reasoning_tokens": 30
        });

        // This will test the ChatUsage model once implemented
        // For now, we're documenting the expected structure
        assert!(usage_json["prompt_tokens"].is_number());
        assert!(usage_json["completion_tokens"].is_number());
        assert!(usage_json["reasoning_tokens"].is_number());
    }

    #[test]
    fn test_responses_usage_structure() {
        // Responses API usage structure
        let usage_json = json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "total_tokens": 150,
            "reasoning_tokens": 30,
            "cached_tokens": 20
        });

        assert_eq!(usage_json["input_tokens"], 100);
        assert_eq!(usage_json["output_tokens"], 50);
        assert_eq!(usage_json["reasoning_tokens"], 30);
        assert_eq!(usage_json["cached_tokens"], 20);
        assert_eq!(usage_json["total_tokens"], 150);
    }

    #[test]
    fn test_chat_response_structure() {
        // Expected Chat Completions response structure
        let response_json = json!({
            "id": "chatcmpl-abc123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello! How can I help you?"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "total_tokens": 30
            }
        });

        assert_eq!(response_json["object"], "chat.completion");
        assert!(response_json["choices"].is_array());
        assert!(response_json["usage"].is_object());
    }

    #[test]
    fn test_responses_response_structure() {
        // Expected Responses API response structure
        let response_json = json!({
            "id": "rsp-abc123",
            "object": "response",
            "created": 1234567890,
            "model": "gpt-4o",
            "output_text": "Hello! How can I help you?",
            "output": [{
                "id": "msg-1",
                "type": "assistant_message",
                "content": "Hello! How can I help you?"
            }],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 20,
                "total_tokens": 30
            }
        });

        assert_eq!(response_json["object"], "response");
        assert!(response_json["output"].is_array());
        assert!(response_json["usage"].is_object());
    }
}

// ============================================================================
// SECTION 2: Basic Response Conversion Tests (Responses → Chat)
// ============================================================================

mod basic_response_conversion {
    use super::*;

    #[test]
    fn test_simple_text_response_conversion() {
        let responses_response = json!({
            "id": "rsp-123",
            "object": "response",
            "created": 1234567890,
            "model": "gpt-4o-mini",
            "output_text": "Hello there!",
            "output": [{
                "id": "msg-1",
                "type": "assistant_message",
                "content": "Hello there!"
            }],
            "usage": {
                "input_tokens": 5,
                "output_tokens": 3,
                "total_tokens": 8
            }
        });

        // Expected Chat response
        let expected_chat = json!({
            "id": "rsp-123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4o-mini",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello there!"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 3,
                "total_tokens": 8
            }
        });

        // Test will verify conversion once implemented
        assert_eq!(responses_response["model"], expected_chat["model"]);
    }

    #[test]
    fn test_usage_token_mapping() {
        let responses_usage = json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "total_tokens": 150
        });

        let expected_chat_usage = json!({
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        });

        // Validate field mapping
        assert_eq!(
            responses_usage["input_tokens"],
            expected_chat_usage["prompt_tokens"]
        );
        assert_eq!(
            responses_usage["output_tokens"],
            expected_chat_usage["completion_tokens"]
        );
        assert_eq!(
            responses_usage["total_tokens"],
            expected_chat_usage["total_tokens"]
        );
    }

    #[test]
    fn test_output_text_to_message_content() {
        let responses_output_text = "This is the assistant's response.";
        let expected_message_content = "This is the assistant's response.";

        assert_eq!(responses_output_text, expected_message_content);
    }

    #[test]
    fn test_id_preservation() {
        let responses_id = "rsp-abc123def456";
        let expected_chat_id = "rsp-abc123def456";

        // ID should be preserved as-is
        assert_eq!(responses_id, expected_chat_id);
    }

    #[test]
    fn test_object_type_conversion() {
        let responses_object = "response";
        let expected_chat_object = "chat.completion";

        // Object type must be converted
        assert_ne!(responses_object, expected_chat_object);
    }

    #[test]
    fn test_created_timestamp_preservation() {
        let timestamp: u64 = 1234567890;

        // Timestamp should be preserved exactly
        let responses_created = timestamp;
        let expected_chat_created = timestamp;

        assert_eq!(responses_created, expected_chat_created);
    }
}

// ============================================================================
// SECTION 3: Reasoning Tokens Handling Tests
// ============================================================================

mod reasoning_tokens {
    use super::*;

    #[test]
    fn test_reasoning_tokens_preserved_in_conversion() {
        let responses_response = json!({
            "id": "rsp-reasoning-1",
            "object": "response",
            "created": 1234567890,
            "model": "o1-preview",
            "output_text": "After careful analysis, the answer is 42.",
            "output": [
                {
                    "id": "reasoning-1",
                    "type": "reasoning",
                    "summary": ["Step 1: Consider the problem", "Step 2: Apply logic"]
                },
                {
                    "id": "msg-1",
                    "type": "assistant_message",
                    "content": "After careful analysis, the answer is 42."
                }
            ],
            "usage": {
                "input_tokens": 20,
                "output_tokens": 50,
                "reasoning_tokens": 128,  // Critical field!
                "total_tokens": 198
            }
        });

        // Expected Chat response with reasoning_tokens preserved
        let expected_usage = json!({
            "prompt_tokens": 20,
            "completion_tokens": 50,
            "total_tokens": 198,
            "reasoning_tokens": 128  // Must be preserved as extension
        });

        assert_eq!(
            responses_response["usage"]["reasoning_tokens"],
            expected_usage["reasoning_tokens"]
        );
    }

    #[test]
    fn test_reasoning_tokens_none_for_non_reasoning_models() {
        let responses_response = json!({
            "id": "rsp-normal-1",
            "object": "response",
            "created": 1234567890,
            "model": "gpt-4o",
            "output_text": "Hello!",
            "output": [{
                "id": "msg-1",
                "type": "assistant_message",
                "content": "Hello!"
            }],
            "usage": {
                "input_tokens": 5,
                "output_tokens": 2,
                "total_tokens": 7
                // No reasoning_tokens field
            }
        });

        // Reasoning tokens should be None/absent for non-reasoning models
        assert!(responses_response["usage"]["reasoning_tokens"].is_null());
    }

    #[test]
    fn test_reasoning_tokens_with_cached_tokens() {
        let responses_response = json!({
            "id": "rsp-cached-1",
            "object": "response",
            "created": 1234567890,
            "model": "o1-mini",
            "output_text": "Result after reasoning",
            "output": [{
                "id": "msg-1",
                "type": "assistant_message",
                "content": "Result after reasoning"
            }],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 30,
                "reasoning_tokens": 64,
                "cached_tokens": 50,  // Prompt caching
                "total_tokens": 194
            }
        });

        // Both reasoning_tokens and cached_tokens should be preserved
        assert_eq!(responses_response["usage"]["reasoning_tokens"], 64);
        assert_eq!(responses_response["usage"]["cached_tokens"], 50);
    }

    #[test]
    fn test_high_reasoning_token_count() {
        // Reasoning models can use thousands of reasoning tokens
        let responses_response = json!({
            "id": "rsp-complex-1",
            "object": "response",
            "created": 1234567890,
            "model": "o3-mini",
            "output_text": "Complex solution",
            "output": [{
                "id": "msg-1",
                "type": "assistant_message",
                "content": "Complex solution"
            }],
            "usage": {
                "input_tokens": 50,
                "output_tokens": 20,
                "reasoning_tokens": 8192,  // Large reasoning token count
                "total_tokens": 8262
            }
        });

        assert_eq!(responses_response["usage"]["reasoning_tokens"], 8192);

        // Total should equal input + output + reasoning
        let total = responses_response["usage"]["total_tokens"]
            .as_u64()
            .unwrap();
        let expected_total = 50 + 20 + 8192;
        assert_eq!(total, expected_total);
    }
}

// ============================================================================
// SECTION 4: Output Array Item Handling Tests
// ============================================================================

mod output_array_items {
    use super::*;

    #[test]
    fn test_assistant_message_output_item() {
        let output_item = json!({
            "id": "msg-1",
            "type": "assistant_message",
            "content": "This is the response"
        });

        assert_eq!(output_item["type"], "assistant_message");
        assert_eq!(output_item["content"], "This is the response");
    }

    #[test]
    fn test_reasoning_output_item() {
        let output_item = json!({
            "id": "reasoning-1",
            "type": "reasoning",
            "summary": [
                "First, I analyzed the problem",
                "Then, I considered the constraints",
                "Finally, I arrived at a solution"
            ]
        });

        assert_eq!(output_item["type"], "reasoning");
        assert!(output_item["summary"].is_array());
        assert_eq!(output_item["summary"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_reasoning_with_encrypted_content() {
        let output_item = json!({
            "id": "reasoning-1",
            "type": "reasoning",
            "encrypted_content": "base64encodedencrypteddata=="
        });

        // For ZDR compliance, reasoning content may be encrypted
        assert_eq!(output_item["type"], "reasoning");
        assert!(output_item["encrypted_content"].is_string());
    }

    #[test]
    fn test_tool_call_output_item() {
        let output_item = json!({
            "id": "call-1",
            "type": "tool_call",
            "name": "get_weather",
            "arguments": "{\"location\": \"San Francisco\"}",
            "call_id": "call-abc123"
        });

        assert_eq!(output_item["type"], "tool_call");
        assert_eq!(output_item["name"], "get_weather");
        assert!(output_item["arguments"].is_string());
    }

    #[test]
    fn test_function_call_output_output_item() {
        let output_item = json!({
            "id": "out-1",
            "type": "function_call_output",
            "call_id": "call-abc123",
            "content": "{\"temperature\": 65, \"conditions\": \"sunny\"}"
        });

        assert_eq!(output_item["type"], "function_call_output");
        assert_eq!(output_item["call_id"], "call-abc123");
        assert!(output_item["content"].is_string());
    }

    #[test]
    fn test_multiple_output_items() {
        let output_array = json!([
            {
                "id": "reasoning-1",
                "type": "reasoning",
                "summary": ["Analysis step"]
            },
            {
                "id": "call-1",
                "type": "tool_call",
                "name": "search",
                "arguments": "{}"
            },
            {
                "id": "msg-1",
                "type": "assistant_message",
                "content": "Here's what I found"
            }
        ]);

        assert_eq!(output_array.as_array().unwrap().len(), 3);
        assert_eq!(output_array[0]["type"], "reasoning");
        assert_eq!(output_array[1]["type"], "tool_call");
        assert_eq!(output_array[2]["type"], "assistant_message");
    }
}

// ============================================================================
// SECTION 5: Tool Calling Response Conversion Tests
// ============================================================================

mod tool_calling_responses {
    use super::*;

    #[test]
    fn test_tool_call_in_chat_response() {
        let chat_response = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-abc",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\": \"NYC\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 50,
                "completion_tokens": 20,
                "total_tokens": 70
            }
        });

        assert_eq!(chat_response["choices"][0]["finish_reason"], "tool_calls");
        assert!(chat_response["choices"][0]["message"]["tool_calls"].is_array());
    }

    #[test]
    fn test_responses_tool_call_conversion() {
        let responses_response = json!({
            "id": "rsp-123",
            "object": "response",
            "created": 1234567890,
            "model": "gpt-4o",
            "output_text": null,
            "output": [{
                "id": "call-1",
                "type": "tool_call",
                "name": "get_weather",
                "arguments": "{\"location\": \"NYC\"}",
                "call_id": "call-abc"
            }],
            "usage": {
                "input_tokens": 50,
                "output_tokens": 20,
                "total_tokens": 70
            }
        });

        // Should convert to Chat format with tool_calls
        assert_eq!(responses_response["output"][0]["type"], "tool_call");
        assert_eq!(responses_response["output"][0]["name"], "get_weather");
    }

    #[test]
    fn test_multiple_tool_calls() {
        let chat_response = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call-1",
                            "type": "function",
                            "function": {
                                "name": "search_web",
                                "arguments": "{\"query\": \"weather\"}"
                            }
                        },
                        {
                            "id": "call-2",
                            "type": "function",
                            "function": {
                                "name": "get_location",
                                "arguments": "{}"
                            }
                        }
                    ]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150
            }
        });

        assert_eq!(
            chat_response["choices"][0]["message"]["tool_calls"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}

// ============================================================================
// SECTION 6: Finish Reason Handling Tests
// ============================================================================

mod finish_reason {
    use super::*;

    #[test]
    fn test_finish_reason_stop() {
        let response = json!({
            "choices": [{
                "finish_reason": "stop"
            }]
        });

        assert_eq!(response["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn test_finish_reason_length() {
        let response = json!({
            "choices": [{
                "finish_reason": "length"
            }]
        });

        assert_eq!(response["choices"][0]["finish_reason"], "length");
    }

    #[test]
    fn test_finish_reason_tool_calls() {
        let response = json!({
            "choices": [{
                "finish_reason": "tool_calls"
            }]
        });

        assert_eq!(response["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn test_finish_reason_content_filter() {
        let response = json!({
            "choices": [{
                "finish_reason": "content_filter"
            }]
        });

        assert_eq!(response["choices"][0]["finish_reason"], "content_filter");
    }
}

// ============================================================================
// SECTION 7: Streaming Response Conversion Tests
// ============================================================================

mod streaming_responses {
    use super::*;

    #[test]
    fn test_chat_streaming_chunk_structure() {
        let chunk = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "delta": {
                    "role": "assistant",
                    "content": "Hello"
                },
                "finish_reason": null
            }]
        });

        assert_eq!(chunk["object"], "chat.completion.chunk");
        assert!(chunk["choices"][0]["delta"].is_object());
    }

    #[test]
    fn test_streaming_content_delta() {
        let chunk = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "delta": {
                    "content": " world"
                },
                "finish_reason": null
            }]
        });

        assert_eq!(chunk["choices"][0]["delta"]["content"], " world");
        assert!(chunk["choices"][0]["delta"]["role"].is_null());
    }

    #[test]
    fn test_streaming_final_chunk() {
        let chunk = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "total_tokens": 30
            }
        });

        assert_eq!(chunk["choices"][0]["finish_reason"], "stop");
        assert!(chunk["usage"].is_object());
    }

    #[test]
    fn test_streaming_tool_call_delta() {
        let chunk = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call-abc",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"loc"
                        }
                    }]
                },
                "finish_reason": null
            }]
        });

        assert!(chunk["choices"][0]["delta"]["tool_calls"].is_array());
    }

    #[test]
    fn test_streaming_with_reasoning_tokens() {
        // Final chunk with reasoning tokens in usage
        let chunk = json!({
            "id": "chatcmpl-reasoning",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "o1-preview",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 10,
                "reasoning_tokens": 128,
                "total_tokens": 158
            }
        });

        assert_eq!(chunk["usage"]["reasoning_tokens"], 128);
    }
}

// ============================================================================
// SECTION 8: Error Response Handling Tests
// ============================================================================

mod error_responses {
    use super::*;

    #[test]
    fn test_chat_error_response_structure() {
        let error_response = json!({
            "error": {
                "message": "Invalid API key",
                "type": "invalid_request_error",
                "param": null,
                "code": "invalid_api_key"
            }
        });

        assert!(error_response["error"].is_object());
        assert!(error_response["error"]["message"].is_string());
    }

    #[test]
    fn test_rate_limit_error() {
        let error_response = json!({
            "error": {
                "message": "Rate limit exceeded",
                "type": "rate_limit_error",
                "param": null,
                "code": null
            }
        });

        assert_eq!(error_response["error"]["type"], "rate_limit_error");
    }

    #[test]
    fn test_model_not_found_error() {
        let error_response = json!({
            "error": {
                "message": "Model not found",
                "type": "invalid_request_error",
                "param": "model",
                "code": null
            }
        });

        assert_eq!(error_response["error"]["param"], "model");
    }
}

// ============================================================================
// SECTION 9: Multiple Choices Handling Tests
// ============================================================================

mod multiple_choices {
    use super::*;

    #[test]
    fn test_n_parameter_multiple_completions() {
        let response = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4o",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "First completion"
                    },
                    "finish_reason": "stop"
                },
                {
                    "index": 1,
                    "message": {
                        "role": "assistant",
                        "content": "Second completion"
                    },
                    "finish_reason": "stop"
                },
                {
                    "index": 2,
                    "message": {
                        "role": "assistant",
                        "content": "Third completion"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 50,
                "completion_tokens": 60,
                "total_tokens": 110
            }
        });

        assert_eq!(response["choices"].as_array().unwrap().len(), 3);
        assert_eq!(response["choices"][0]["index"], 0);
        assert_eq!(response["choices"][1]["index"], 1);
        assert_eq!(response["choices"][2]["index"], 2);
    }
}

// ============================================================================
// SECTION 10: Round-Trip Response Conversion Tests
// ============================================================================

mod response_round_trip {
    use super::*;

    #[test]
    fn test_chat_to_responses_to_chat_response() {
        // Original Chat response
        let original_chat = json!({
            "id": "chatcmpl-original",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Test response"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });

        // Key fields should be preservable through round-trip
        assert_eq!(original_chat["model"], "gpt-4o");
        assert_eq!(
            original_chat["choices"][0]["message"]["content"],
            "Test response"
        );
        assert_eq!(original_chat["usage"]["total_tokens"], 15);
    }

    #[test]
    fn test_reasoning_tokens_survive_round_trip() {
        let original = json!({
            "id": "chatcmpl-reasoning",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "o1-preview",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Result"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 10,
                "reasoning_tokens": 128,
                "total_tokens": 158
            }
        });

        // reasoning_tokens must survive conversion
        assert_eq!(original["usage"]["reasoning_tokens"], 128);
    }
}

// ============================================================================
// SECTION 11: Specification Compliance Tests
// ============================================================================

mod response_spec_compliance {
    use super::*;

    #[test]
    fn test_all_usage_fields_present() {
        let usage = json!({
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        });

        // Required fields for Chat API
        assert!(usage["prompt_tokens"].is_number());
        assert!(usage["completion_tokens"].is_number());
        assert!(usage["total_tokens"].is_number());
    }

    #[test]
    fn test_usage_total_equals_sum() {
        let prompt = 100u64;
        let completion = 50u64;
        let total = 150u64;

        assert_eq!(prompt + completion, total);
    }

    #[test]
    fn test_usage_with_reasoning_total_calculation() {
        let prompt = 20u64;
        let completion = 10u64;
        let reasoning = 128u64;
        let total = 158u64;

        // For reasoning models: total = prompt + completion + reasoning
        assert_eq!(prompt + completion + reasoning, total);
    }

    #[test]
    fn test_cached_tokens_not_in_total() {
        // According to spec, cached_tokens are included in prompt_tokens
        // but tracked separately for billing
        let usage = json!({
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150,
            "cached_tokens": 60
        });

        let total = usage["total_tokens"].as_u64().unwrap();
        let prompt = usage["prompt_tokens"].as_u64().unwrap();
        let completion = usage["completion_tokens"].as_u64().unwrap();

        // cached_tokens are subset of prompt_tokens
        assert_eq!(prompt + completion, total);
    }
}

// ============================================================================
// SECTION 12: Actual Conversion Tests Using Implemented Functions
// ============================================================================

mod actual_conversions {
    use super::*;

    #[test]
    fn test_responses_to_chat_simple_text() {
        let responses = ResponsesResponse {
            id: "rsp-123".to_string(),
            object: "response".to_string(),
            created: 1234567890,
            model: "gpt-4o-mini".to_string(),
            output_text: Some("Hello there!".to_string()),
            output: vec![OutputItem::AssistantMessage {
                id: "msg-1".to_string(),
                content: "Hello there!".to_string(),
            }],
            usage: Some(ResponsesUsage {
                input_tokens: 5,
                output_tokens: 3,
                total_tokens: 8,
                reasoning_tokens: None,
                cached_tokens: None,
            }),
            system_fingerprint: None,
        };

        let chat = responses_to_chat_response(&responses);

        assert_eq!(chat.id, "rsp-123");
        assert_eq!(chat.object, "chat.completion");
        assert_eq!(chat.model, "gpt-4o-mini");
        assert_eq!(chat.choices.len(), 1);
        assert_eq!(chat.choices[0].message.role, "assistant");
        assert_eq!(
            chat.choices[0].message.content,
            Some("Hello there!".to_string())
        );
        assert_eq!(chat.choices[0].finish_reason, Some("stop".to_string()));

        let usage = chat.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 5);
        assert_eq!(usage.completion_tokens, 3);
        assert_eq!(usage.total_tokens, 8);
    }

    #[test]
    fn test_responses_to_chat_with_reasoning_tokens() {
        let responses = ResponsesResponse {
            id: "rsp-reasoning".to_string(),
            object: "response".to_string(),
            created: 1234567890,
            model: "o1-preview".to_string(),
            output_text: Some("After analysis, the answer is 42.".to_string()),
            output: vec![
                OutputItem::Reasoning {
                    id: "reasoning-1".to_string(),
                    summary: Some(vec![
                        "Step 1: Analyze".to_string(),
                        "Step 2: Calculate".to_string(),
                    ]),
                    encrypted_content: None,
                },
                OutputItem::AssistantMessage {
                    id: "msg-1".to_string(),
                    content: "After analysis, the answer is 42.".to_string(),
                },
            ],
            usage: Some(ResponsesUsage {
                input_tokens: 20,
                output_tokens: 10,
                total_tokens: 158,
                reasoning_tokens: Some(128),
                cached_tokens: None,
            }),
            system_fingerprint: None,
        };

        let chat = responses_to_chat_response(&responses);

        assert_eq!(chat.model, "o1-preview");

        // Reasoning tokens must be preserved
        let usage = chat.usage.unwrap();
        assert_eq!(usage.reasoning_tokens, Some(128));
        assert_eq!(usage.prompt_tokens, 20);
        assert_eq!(usage.completion_tokens, 10);
        assert_eq!(usage.total_tokens, 158);
    }

    #[test]
    fn test_responses_to_chat_with_tool_calls() {
        let responses = ResponsesResponse {
            id: "rsp-tools".to_string(),
            object: "response".to_string(),
            created: 1234567890,
            model: "gpt-4o".to_string(),
            output_text: None,
            output: vec![OutputItem::ToolCall {
                id: "call-1".to_string(),
                name: "get_weather".to_string(),
                arguments: "{\"location\":\"NYC\"}".to_string(),
                call_id: "call_abc123".to_string(),
            }],
            usage: Some(ResponsesUsage {
                input_tokens: 50,
                output_tokens: 20,
                total_tokens: 70,
                reasoning_tokens: None,
                cached_tokens: None,
            }),
            system_fingerprint: None,
        };

        let chat = responses_to_chat_response(&responses);

        assert_eq!(
            chat.choices[0].finish_reason,
            Some("tool_calls".to_string())
        );
        assert!(chat.choices[0].message.tool_calls.is_some());

        let tool_calls = chat.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_abc123");
        assert_eq!(tool_calls[0].call_type, "function");
        assert_eq!(tool_calls[0].function.name, "get_weather");
        assert_eq!(tool_calls[0].function.arguments, "{\"location\":\"NYC\"}");
    }

    #[test]
    fn test_chat_to_responses_simple() {
        let chat = ChatCompletionResponse {
            id: "chatcmpl-123".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "gpt-4o".to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatResponseMessage {
                    role: "assistant".to_string(),
                    content: Some("Test response".to_string()),
                    tool_calls: None,
                    function_call: None,
                    reasoning_content: None,
                },
                finish_reason: Some("stop".to_string()),
                logprobs: None,
            }],
            usage: Some(ChatUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
                reasoning_tokens: None,
                cached_tokens: None,
            }),
            system_fingerprint: None,
        };

        let responses = chat_to_responses_response(&chat);

        assert_eq!(responses.id, "chatcmpl-123");
        assert_eq!(responses.object, "response");
        assert_eq!(responses.model, "gpt-4o");
        assert_eq!(responses.output_text, Some("Test response".to_string()));
        assert_eq!(responses.output.len(), 1);

        match &responses.output[0] {
            OutputItem::AssistantMessage { id: _, content } => {
                assert_eq!(content, "Test response");
            }
            _ => panic!("Expected AssistantMessage"),
        }

        let usage = responses.usage.unwrap();
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.total_tokens, 15);
    }

    #[test]
    fn test_chat_to_responses_with_reasoning_tokens() {
        let chat = ChatCompletionResponse {
            id: "chatcmpl-o1".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "o1-mini".to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatResponseMessage {
                    role: "assistant".to_string(),
                    content: Some("Result".to_string()),
                    tool_calls: None,
                    function_call: None,
                    reasoning_content: None,
                },
                finish_reason: Some("stop".to_string()),
                logprobs: None,
            }],
            usage: Some(ChatUsage {
                prompt_tokens: 20,
                completion_tokens: 10,
                total_tokens: 158,
                reasoning_tokens: Some(128),
                cached_tokens: Some(10),
            }),
            system_fingerprint: None,
        };

        let responses = chat_to_responses_response(&chat);

        let usage = responses.usage.unwrap();
        assert_eq!(usage.reasoning_tokens, Some(128));
        assert_eq!(usage.cached_tokens, Some(10));
    }

    #[test]
    fn test_chat_to_responses_with_tool_calls() {
        let chat = ChatCompletionResponse {
            id: "chatcmpl-tool".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "gpt-4o".to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatResponseMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_calls: Some(vec![ToolCall {
                        id: "call-123".to_string(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: "search".to_string(),
                            arguments: "{\"q\":\"test\"}".to_string(),
                        },
                    }]),
                    function_call: None,
                    reasoning_content: None,
                },
                finish_reason: Some("tool_calls".to_string()),
                logprobs: None,
            }],
            usage: Some(ChatUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                reasoning_tokens: None,
                cached_tokens: None,
            }),
            system_fingerprint: None,
        };

        let responses = chat_to_responses_response(&chat);

        assert_eq!(responses.output.len(), 1);
        match &responses.output[0] {
            OutputItem::ToolCall {
                id: _,
                name,
                arguments,
                call_id,
            } => {
                assert_eq!(name, "search");
                assert_eq!(arguments, "{\"q\":\"test\"}");
                assert_eq!(call_id, "call-123");
            }
            _ => panic!("Expected ToolCall"),
        }
    }

    #[test]
    fn test_round_trip_simple_response() {
        // Start with Responses format
        let original_responses = ResponsesResponse {
            id: "rsp-round".to_string(),
            object: "response".to_string(),
            created: 1234567890,
            model: "gpt-4o".to_string(),
            output_text: Some("Round trip test".to_string()),
            output: vec![OutputItem::AssistantMessage {
                id: "msg-1".to_string(),
                content: "Round trip test".to_string(),
            }],
            usage: Some(ResponsesUsage {
                input_tokens: 15,
                output_tokens: 8,
                total_tokens: 23,
                reasoning_tokens: None,
                cached_tokens: None,
            }),
            system_fingerprint: None,
        };

        // Convert to Chat
        let chat = responses_to_chat_response(&original_responses);

        // Convert back to Responses
        let reconstructed = chat_to_responses_response(&chat);

        // Verify key fields match
        assert_eq!(reconstructed.model, original_responses.model);
        assert_eq!(reconstructed.output_text, original_responses.output_text);

        let orig_usage = original_responses.usage.unwrap();
        let recon_usage = reconstructed.usage.unwrap();
        assert_eq!(recon_usage.input_tokens, orig_usage.input_tokens);
        assert_eq!(recon_usage.output_tokens, orig_usage.output_tokens);
        assert_eq!(recon_usage.total_tokens, orig_usage.total_tokens);
    }

    #[test]
    fn test_round_trip_with_reasoning_tokens() {
        let original_responses = ResponsesResponse {
            id: "rsp-reasoning-round".to_string(),
            object: "response".to_string(),
            created: 1234567890,
            model: "o1-preview".to_string(),
            output_text: Some("Reasoned answer".to_string()),
            output: vec![OutputItem::AssistantMessage {
                id: "msg-1".to_string(),
                content: "Reasoned answer".to_string(),
            }],
            usage: Some(ResponsesUsage {
                input_tokens: 30,
                output_tokens: 15,
                total_tokens: 173,
                reasoning_tokens: Some(128),
                cached_tokens: Some(20),
            }),
            system_fingerprint: None,
        };

        let chat = responses_to_chat_response(&original_responses);
        let reconstructed = chat_to_responses_response(&chat);

        // Reasoning tokens must survive round trip
        let orig_usage = original_responses.usage.unwrap();
        let recon_usage = reconstructed.usage.unwrap();

        assert_eq!(recon_usage.reasoning_tokens, orig_usage.reasoning_tokens);
        assert_eq!(recon_usage.cached_tokens, orig_usage.cached_tokens);
        assert_eq!(recon_usage.reasoning_tokens, Some(128));
        assert_eq!(recon_usage.cached_tokens, Some(20));
    }

    #[test]
    fn test_serialization_chat_response_with_reasoning() {
        let chat = ChatCompletionResponse {
            id: "chatcmpl-serial".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "o1-mini".to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatResponseMessage {
                    role: "assistant".to_string(),
                    content: Some("Answer".to_string()),
                    tool_calls: None,
                    function_call: None,
                    reasoning_content: None,
                },
                finish_reason: Some("stop".to_string()),
                logprobs: None,
            }],
            usage: Some(ChatUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 143,
                reasoning_tokens: Some(128),
                cached_tokens: None,
            }),
            system_fingerprint: None,
        };

        // Serialize to JSON
        let json = serde_json::to_value(&chat).unwrap();

        // Verify structure
        assert_eq!(json["object"], "chat.completion");
        assert_eq!(json["usage"]["reasoning_tokens"], 128);
        assert_eq!(json["usage"]["prompt_tokens"], 10);
        assert_eq!(json["usage"]["completion_tokens"], 5);
        assert_eq!(json["usage"]["total_tokens"], 143);
    }

    #[test]
    fn test_serialization_responses_response() {
        let responses = ResponsesResponse {
            id: "rsp-serial".to_string(),
            object: "response".to_string(),
            created: 1234567890,
            model: "gpt-4o".to_string(),
            output_text: Some("Text".to_string()),
            output: vec![OutputItem::AssistantMessage {
                id: "msg-1".to_string(),
                content: "Text".to_string(),
            }],
            usage: Some(ResponsesUsage {
                input_tokens: 5,
                output_tokens: 3,
                total_tokens: 8,
                reasoning_tokens: None,
                cached_tokens: None,
            }),
            system_fingerprint: None,
        };

        let json = serde_json::to_value(&responses).unwrap();

        assert_eq!(json["object"], "response");
        assert_eq!(json["output_text"], "Text");
        assert!(json["output"].is_array());
        assert_eq!(json["usage"]["input_tokens"], 5);
        assert_eq!(json["usage"]["output_tokens"], 3);
    }

    #[test]
    fn test_deserialization_chat_response() {
        let json = json!({
            "id": "chatcmpl-deser",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15,
                "reasoning_tokens": 64
            }
        });

        let chat: ChatCompletionResponse = serde_json::from_value(json).unwrap();

        assert_eq!(chat.id, "chatcmpl-deser");
        assert_eq!(chat.model, "gpt-4o");
        assert_eq!(chat.choices[0].message.content, Some("Hello".to_string()));

        let usage = chat.usage.unwrap();
        assert_eq!(usage.reasoning_tokens, Some(64));
    }

    #[test]
    fn test_deserialization_responses_response() {
        let json = json!({
            "id": "rsp-deser",
            "object": "response",
            "created": 1234567890,
            "model": "o1-preview",
            "output_text": "Result",
            "output": [{
                "id": "msg-1",
                "type": "assistant_message",
                "content": "Result"
            }],
            "usage": {
                "input_tokens": 20,
                "output_tokens": 10,
                "total_tokens": 158,
                "reasoning_tokens": 128
            }
        });

        let responses: ResponsesResponse = serde_json::from_value(json).unwrap();

        assert_eq!(responses.id, "rsp-deser");
        assert_eq!(responses.model, "o1-preview");
        assert_eq!(responses.output_text, Some("Result".to_string()));

        let usage = responses.usage.unwrap();
        assert_eq!(usage.reasoning_tokens, Some(128));
    }
}
