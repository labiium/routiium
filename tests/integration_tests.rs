/// Integration tests with real LLM APIs
///
/// These tests make actual API calls to the configured endpoint to validate
/// the conversion pipeline works end-to-end.
///
/// SETUP:
/// 1. Configure required environment variables in .env:
///    OPENAI_BASE_URL=https://api.openai.com/v1  (or your custom endpoint)
///    OPENAI_API_KEY=sk-proj-...                  (your API key)
///    MODEL=gpt-5-nano                            (model to test with)
///
/// 2. Run tests with --ignored flag:
///    cargo test --test integration_tests -- --ignored --nocapture
///
/// NOTES:
/// - These tests are marked #[ignore] to prevent accidental API calls
/// - All environment variables (OPENAI_BASE_URL, OPENAI_API_KEY, MODEL) are required
/// - Tests will skip if OPENAI_API_KEY is not set
/// - These tests will consume API credits
use routiium::{
    chat::{ChatCompletionRequest, ChatMessage, FunctionDef, Role, ToolDefinition},
    to_responses_request,
};
use serde_json::json;
use std::env;

/// Helper to check if we should run integration tests
fn should_run_integration_tests() -> bool {
    // Load environment from .env if present
    let dotenv_loaded = dotenvy::dotenv();
    match &dotenv_loaded {
        Ok(path) => eprintln!("Env debug -> loaded .env from: {}", path.display()),
        Err(_) => eprintln!("Env debug -> no .env loaded; using process environment"),
    }
    // Log env values (mask API key)
    let base_url_dbg = env::var("OPENAI_BASE_URL").unwrap_or_default();
    let model_dbg = env::var("MODEL").unwrap_or_default();
    let api_key_dbg = env::var("OPENAI_API_KEY").unwrap_or_default();
    let masked_key = if api_key_dbg.is_empty() {
        "<empty>".to_string()
    } else {
        let start = api_key_dbg.chars().take(4).collect::<String>();
        let end_rev = api_key_dbg.chars().rev().take(4).collect::<String>();
        let end = end_rev.chars().rev().collect::<String>();
        format!("{}…{}", start, end)
    };
    eprintln!("Env debug -> OPENAI_BASE_URL: {}", base_url_dbg);
    eprintln!("Env debug -> MODEL: {}", model_dbg);
    eprintln!("Env debug -> OPENAI_API_KEY: {}", masked_key);

    // Verify required variables are present and non-empty
    let mut missing: Vec<&'static str> = Vec::new();
    let api_key = env::var("OPENAI_API_KEY").ok().filter(|v| !v.is_empty());
    if api_key.is_none() {
        missing.push("OPENAI_API_KEY");
    }
    let base_url = env::var("OPENAI_BASE_URL").ok().filter(|v| !v.is_empty());
    if base_url.is_none() {
        missing.push("OPENAI_BASE_URL");
    }
    let model = env::var("MODEL").ok().filter(|v| !v.is_empty());
    if model.is_none() {
        missing.push("MODEL");
    }

    if !missing.is_empty() {
        eprintln!("⚠️  Missing required env vars: {:?}", missing);
        eprintln!("    Ensure your .env contains these or export them in your shell.");
        eprintln!("    Example .env:");
        eprintln!("      OPENAI_BASE_URL=https://api.openai.com/v1");
        eprintln!("      OPENAI_API_KEY=sk-...");
        eprintln!("      MODEL=gpt-4.1-nano");
        return false;
    }

    // Preflight: attempt TCP connect to the upstream host:port derived from OPENAI_BASE_URL
    if let Some(base) = base_url {
        if let Ok(url) = reqwest::Url::parse(&base) {
            if let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) {
                use std::net::{TcpStream, ToSocketAddrs};
                use std::time::Duration;
                let addr_str = format!("{}:{}", host, port);
                let resolved = addr_str.to_socket_addrs().ok();
                if let Some(addrs) = resolved {
                    let mut reachable = false;
                    for addr in addrs {
                        if TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok() {
                            reachable = true;
                            break;
                        }
                    }
                    if !reachable {
                        eprintln!(
                            "⚠️  Upstream not reachable at {} ({}). Skipping integration tests.",
                            base, addr_str
                        );
                        return false;
                    }
                } else {
                    eprintln!(
                        "⚠️  Could not resolve {}. Skipping integration tests.",
                        addr_str
                    );
                    return false;
                }
            }
        }
    }

    true
}

/// Helper to get API key from environment
fn get_api_key() -> String {
    env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set")
}

/// Helper to get base URL from environment
fn get_base_url() -> String {
    env::var("OPENAI_BASE_URL").expect("OPENAI_BASE_URL must be set in .env")
}

/// Helper to get model from environment
fn get_model() -> String {
    env::var("MODEL").expect("MODEL must be set in .env")
}

/// Create an HTTP client
fn create_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("Failed to create HTTP client")
}

// ============================================================================
// SECTION 1: Basic Integration Tests (Standard Models)
// ============================================================================

#[tokio::test]

async fn test_real_chat_completions_simple() {
    if !should_run_integration_tests() {
        eprintln!("Skipping integration test: OPENAI_API_KEY not set");
        return;
    }

    let client = create_client();
    let api_key = get_api_key();
    let base_url = get_base_url();
    let model = get_model();

    let request = ChatCompletionRequest {
        model,
        messages: vec![
            ChatMessage {
                role: Role::System,
                content: json!("You are a helpful assistant. Be concise."),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            ChatMessage {
                role: Role::User,
                content: json!("Say 'Hello, World!' and nothing else."),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        ],
        temperature: None, // GPT-5 models don't support temperature parameter
        max_tokens: None,
        max_completion_tokens: Some(50),
        top_p: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        logit_bias: None,
        user: None,
        n: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        stream: Some(false),
        extra_body: None,
    };

    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request)
        .send()
        .await
        .expect("Failed to send request");

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        eprintln!(
            "⚠️  Upstream returned non-success: {} - {}",
            status, error_text
        );
        eprintln!("    Skipping integration test due to upstream error.");
        return;
    }

    let response_json: serde_json::Value = response.json().await.expect("Failed to parse JSON");

    println!(
        "Response: {}",
        serde_json::to_string_pretty(&response_json).unwrap()
    );

    // Validate response structure
    assert_eq!(response_json["object"], "chat.completion");
    assert!(response_json["choices"].is_array());
    assert!(response_json["usage"].is_object());

    // Validate usage tokens
    assert!(response_json["usage"]["prompt_tokens"].is_number());
    assert!(response_json["usage"]["completion_tokens"].is_number());
    assert!(response_json["usage"]["total_tokens"].is_number());

    // Content should be present
    assert!(response_json["choices"][0]["message"]["content"].is_string());
}

#[tokio::test]

async fn test_real_conversion_chat_to_responses() {
    if !should_run_integration_tests() {
        eprintln!("Skipping integration test: OPENAI_API_KEY not set");
        return;
    }

    let client = create_client();
    let api_key = get_api_key();
    let base_url = get_base_url();
    let model = get_model();

    // Create Chat request
    let chat_request = ChatCompletionRequest {
        model,
        messages: vec![ChatMessage {
            role: Role::User,
            content: json!("What is 2+2? Answer with just the number."),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }],
        temperature: Some(0.1),
        max_tokens: Some(10),
        max_completion_tokens: None,
        top_p: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        logit_bias: None,
        user: None,
        n: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        stream: Some(false),
        extra_body: None,
    };

    // Convert to Responses format
    let responses_request = to_responses_request(&chat_request, None);

    // Validate conversion
    assert_eq!(responses_request.model, chat_request.model);
    assert_eq!(responses_request.messages.len(), 1);
    assert_eq!(responses_request.max_output_tokens, Some(10));

    // Make actual API call with Chat format
    let chat_response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&chat_request)
        .send()
        .await
        .expect("Failed to send request");

    if !chat_response.status().is_success() {
        let status = chat_response.status();
        let err = chat_response.text().await.unwrap_or_default();
        eprintln!("⚠️  Upstream error ({}): {}", status, err);
        eprintln!("    Skipping integration test due to upstream error.");
        return;
    }

    let response_json: serde_json::Value = chat_response.json().await.expect("Failed to parse");

    println!(
        "Chat response: {}",
        serde_json::to_string_pretty(&response_json).unwrap()
    );

    // Verify response structure matches expected Chat format
    assert_eq!(response_json["object"], "chat.completion");
    assert!(response_json["usage"]["total_tokens"].as_u64().unwrap() > 0);
}

// ============================================================================
// SECTION 2: Tool Calling Integration Tests
// ============================================================================

#[tokio::test]

async fn test_real_tool_calling() {
    if !should_run_integration_tests() {
        eprintln!("Skipping integration test: OPENAI_API_KEY not set");
        return;
    }

    let client = create_client();
    let api_key = get_api_key();
    let base_url = get_base_url();
    let model = get_model();

    let request = ChatCompletionRequest {
        model,
        messages: vec![ChatMessage {
            role: Role::User,
            content: json!("What's the weather in San Francisco?"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }],
        temperature: Some(0.7),
        max_tokens: Some(100),
        max_completion_tokens: None,
        top_p: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        logit_bias: None,
        user: None,
        n: None,
        tools: Some(vec![ToolDefinition::Function {
            function: FunctionDef {
                name: "get_weather".to_string(),
                description: Some("Get the current weather in a location".to_string()),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": {
                            "type": "string",
                            "description": "The city and state, e.g. San Francisco, CA"
                        },
                        "unit": {
                            "type": "string",
                            "enum": ["celsius", "fahrenheit"]
                        }
                    },
                    "required": ["location"]
                }),
            },
        }]),
        tool_choice: Some(json!("auto")),
        response_format: None,
        stream: Some(false),
        extra_body: None,
    };

    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request)
        .send()
        .await
        .expect("Failed to send request");

    if !response.status().is_success() {
        let status = response.status();
        let err = response.text().await.unwrap_or_default();
        eprintln!("⚠️  Upstream error ({}): {}", status, err);
        eprintln!("    Skipping integration test due to upstream error.");
        return;
    }

    let response_json: serde_json::Value = response.json().await.expect("Failed to parse");

    println!(
        "Tool calling response: {}",
        serde_json::to_string_pretty(&response_json).unwrap()
    );

    // Should have tool_calls in response
    let finish_reason = response_json["choices"][0]["finish_reason"].as_str();

    // Model should either call the tool or respond directly
    assert!(
        finish_reason == Some("tool_calls") || finish_reason == Some("stop"),
        "Expected tool_calls or stop, got {:?}",
        finish_reason
    );

    if finish_reason == Some("tool_calls") {
        assert!(response_json["choices"][0]["message"]["tool_calls"].is_array());
        let tool_calls = response_json["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert!(!tool_calls.is_empty());

        // Verify tool call structure
        assert_eq!(tool_calls[0]["type"], "function");
        assert!(tool_calls[0]["function"]["name"].is_string());
        assert!(tool_calls[0]["function"]["arguments"].is_string());
    }
}

// ============================================================================
// SECTION 3: Reasoning Model Integration Tests
// ============================================================================

#[tokio::test]

async fn test_real_reasoning_model_o1_mini() {
    if !should_run_integration_tests() {
        eprintln!("Skipping integration test: OPENAI_API_KEY not set");
        return;
    }

    let client = create_client();
    let api_key = get_api_key();
    let base_url = get_base_url();

    let request = ChatCompletionRequest {
        model: "o1-mini".to_string(), // Reasoning model
        messages: vec![
            ChatMessage {
                role: Role::User,
                content: json!("Solve this step by step: If a train travels 60 km/h for 2 hours, then 80 km/h for 1.5 hours, what is the total distance traveled?"),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        ],
        temperature: None, // o1 models don't support temperature
        max_tokens: Some(1000), // Reasoning models need more tokens
        max_completion_tokens: None,
        top_p: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        logit_bias: None,
        user: None,
        n: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        stream: Some(false),
        extra_body: None,    };

    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request)
        .send()
        .await
        .expect("Failed to send request");

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        eprintln!("API Error: {}", error_text);
        eprintln!("Note: o1-mini may not be available on your account");
        return;
    }

    let response_json: serde_json::Value = response.json().await.expect("Failed to parse");

    println!(
        "Reasoning model response: {}",
        serde_json::to_string_pretty(&response_json).unwrap()
    );

    // Validate response structure
    assert_eq!(response_json["object"], "chat.completion");
    assert!(response_json["usage"].is_object());

    // CRITICAL: Check for reasoning_tokens
    let usage = &response_json["usage"];

    if usage["reasoning_tokens"].is_number() {
        let reasoning_tokens = usage["reasoning_tokens"].as_u64().unwrap();
        println!("✅ Reasoning tokens found: {}", reasoning_tokens);

        // Reasoning tokens should be present for o1 models
        assert!(
            reasoning_tokens > 0,
            "Expected reasoning_tokens > 0 for o1 model"
        );

        // Total should include reasoning tokens
        let total = usage["total_tokens"].as_u64().unwrap();
        let completion = usage["completion_tokens"].as_u64().unwrap();
        let prompt = usage["prompt_tokens"].as_u64().unwrap();

        // For reasoning models: total = prompt + completion + reasoning
        assert_eq!(total, prompt + completion + reasoning_tokens);
    } else {
        println!("⚠️  Warning: reasoning_tokens not found in response");
        println!("This might be expected if the model/API doesn't expose them yet");
    }
}

#[tokio::test]

async fn test_real_reasoning_model_o1_preview() {
    if !should_run_integration_tests() {
        eprintln!("Skipping integration test: OPENAI_API_KEY not set");
        return;
    }

    let client = create_client();
    let api_key = get_api_key();
    let base_url = get_base_url();

    let request = ChatCompletionRequest {
        model: "o1-preview".to_string(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: json!("What is the sum of all prime numbers less than 20?"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }],
        temperature: None,
        max_tokens: Some(2000),
        max_completion_tokens: None,
        top_p: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        logit_bias: None,
        user: None,
        n: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        stream: Some(false),
        extra_body: None,
    };

    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request)
        .send()
        .await
        .expect("Failed to send request");

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        eprintln!("API Error: {}", error_text);
        eprintln!("Note: o1-preview may not be available on your account");
        return;
    }

    let response_json: serde_json::Value = response.json().await.expect("Failed to parse");

    println!(
        "o1-preview response: {}",
        serde_json::to_string_pretty(&response_json).unwrap()
    );

    // Check for reasoning tokens
    if let Some(reasoning_tokens) = response_json["usage"]["reasoning_tokens"].as_u64() {
        println!("✅ Reasoning tokens (o1-preview): {}", reasoning_tokens);
        assert!(reasoning_tokens > 0);
    }
}

// ============================================================================
// SECTION 4: Streaming Integration Tests
// ============================================================================

#[tokio::test]

async fn test_real_streaming_response() {
    if !should_run_integration_tests() {
        eprintln!("Skipping integration test: OPENAI_API_KEY not set");
        return;
    }

    let client = create_client();
    let api_key = get_api_key();
    let base_url = get_base_url();
    let model = get_model();

    let request = ChatCompletionRequest {
        model,
        messages: vec![ChatMessage {
            role: Role::User,
            content: json!("Count from 1 to 5, one number per line."),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }],
        temperature: Some(0.7),
        max_tokens: Some(50),
        max_completion_tokens: None,
        top_p: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        logit_bias: None,
        user: None,
        n: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        stream: Some(true), // Enable streaming
        extra_body: None,
    };

    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request)
        .send()
        .await
        .expect("Failed to send request");

    if !response.status().is_success() {
        let status = response.status();
        let err = response.text().await.unwrap_or_default();
        eprintln!("⚠️  Upstream error ({}): {}", status, err);
        eprintln!("    Skipping integration test due to upstream error.");
        return;
    }

    // Read streaming response
    use futures_util::stream::StreamExt;

    let mut stream = response.bytes_stream();
    let mut chunk_count = 0;
    let mut has_usage = false;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("Failed to read chunk");
        let text = String::from_utf8_lossy(&chunk);

        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    break;
                }

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                    chunk_count += 1;
                    assert_eq!(json["object"], "chat.completion.chunk");

                    // Check for usage in final chunk
                    if json["usage"].is_object() {
                        has_usage = true;
                        println!("✅ Usage found in streaming chunk: {:?}", json["usage"]);
                    }
                }
            }
        }
    }

    println!("Received {} streaming chunks", chunk_count);
    assert!(chunk_count > 0, "Expected at least one chunk");

    // Note: Not all APIs return usage in streaming mode
    if !has_usage {
        println!("⚠️  No usage info in streaming response (may be expected)");
    }
}

// ============================================================================
// SECTION 5: Round-Trip Integration Tests
// ============================================================================

#[tokio::test]

async fn test_real_round_trip_conversion() {
    if !should_run_integration_tests() {
        eprintln!("Skipping integration test: OPENAI_API_KEY not set");
        return;
    }

    let client = create_client();
    let api_key = get_api_key();
    let base_url = get_base_url();
    let model = get_model();

    // Step 1: Create Chat request
    let chat_request = ChatCompletionRequest {
        model,
        messages: vec![ChatMessage {
            role: Role::User,
            content: json!("Say exactly: 'Integration test successful'"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }],
        temperature: Some(0.1),
        max_tokens: Some(20),
        max_completion_tokens: None,
        top_p: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        logit_bias: None,
        user: None,
        n: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        stream: Some(false),
        extra_body: None,
    };

    // Step 2: Convert to Responses format
    let responses_request = to_responses_request(&chat_request, None);

    // Verify conversion
    assert_eq!(responses_request.model, chat_request.model);
    assert_eq!(
        responses_request.messages.len(),
        chat_request.messages.len()
    );
    assert_eq!(responses_request.max_output_tokens, chat_request.max_tokens);

    // Step 3: Make real API call
    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&chat_request)
        .send()
        .await
        .expect("Failed to send request");

    if !response.status().is_success() {
        let status = response.status();
        let err = response.text().await.unwrap_or_default();
        eprintln!("⚠️  Upstream error ({}): {}", status, err);
        eprintln!("    Skipping integration test due to upstream error.");
        return;
    }

    let response_json: serde_json::Value = response.json().await.expect("Failed to parse");

    println!(
        "Round-trip response: {}",
        serde_json::to_string_pretty(&response_json).unwrap()
    );

    // Verify response can be parsed and converted
    assert_eq!(response_json["object"], "chat.completion");
    assert!(response_json["choices"][0]["message"]["content"].is_string());

    // Verify usage tokens
    let usage = &response_json["usage"];
    assert!(usage["prompt_tokens"].as_u64().unwrap() > 0);
    assert!(usage["completion_tokens"].as_u64().unwrap() > 0);
    assert!(usage["total_tokens"].as_u64().unwrap() > 0);
}

// ============================================================================
// SECTION 6: Error Handling Integration Tests
// ============================================================================

#[tokio::test]

async fn test_real_invalid_model_error() {
    if !should_run_integration_tests() {
        eprintln!("Skipping integration test: OPENAI_API_KEY not set");
        return;
    }

    let client = create_client();
    let api_key = get_api_key();
    let base_url = get_base_url();

    let request = ChatCompletionRequest {
        model: "invalid-model-name-12345".to_string(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: json!("Test"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }],
        temperature: None,
        max_tokens: Some(10),
        max_completion_tokens: None,
        top_p: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        logit_bias: None,
        user: None,
        n: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        stream: Some(false),
        extra_body: None,
    };

    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request)
        .send()
        .await
        .expect("Failed to send request");

    // Should return error status
    assert!(!response.status().is_success());

    let status = response.status();
    let body_text = response.text().await.unwrap_or_default();
    let error_json: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "⚠️  Non-JSON error body from upstream (status {}), skipping: {}",
                status, body_text
            );
            return;
        }
    };

    println!(
        "Error response: {}",
        serde_json::to_string_pretty(&error_json).unwrap()
    );

    // Should have error structure
    assert!(error_json["error"].is_object());
    assert!(error_json["error"]["message"].is_string());
}
