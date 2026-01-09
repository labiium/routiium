use crate::models::chat;
use crate::models::responses as resp;
use serde_json::{Map, Value};

// ============================================================================
// Response Body Conversion Functions
// ============================================================================

/// Convert a Responses API response to a Chat Completions response.
///
/// This conversion maps:
/// - output_text → choices[0].message.content
/// - usage.input_tokens → usage.prompt_tokens
/// - usage.output_tokens → usage.completion_tokens
/// - usage.reasoning_tokens → usage.reasoning_tokens (preserved)
/// - output array items → appropriate Chat format
pub fn responses_to_chat_response(
    responses_response: &resp::ResponsesResponse,
) -> chat::ChatCompletionResponse {
    // Extract primary message content
    let mut content = responses_response.output_text.clone();

    if content.is_none() {
        let mut text_parts: Vec<String> = Vec::new();
        for item in &responses_response.output {
            match item {
                resp::OutputItem::AssistantMessage { content: text, .. } => {
                    if !text.is_empty() {
                        text_parts.push(text.clone());
                    }
                }
                resp::OutputItem::FunctionCallOutput { content: text, .. } => {
                    if !text.is_empty() {
                        text_parts.push(text.clone());
                    }
                }
                _ => {}
            }
        }

        if !text_parts.is_empty() {
            content = Some(text_parts.join(
                "
",
            ));
        }
    }

    // Check if there are tool calls in the output
    let mut tool_calls: Vec<chat::ToolCall> = Vec::new();
    let mut finish_reason = "stop";

    for item in &responses_response.output {
        if let resp::OutputItem::ToolCall {
            id: _,
            name,
            arguments,
            call_id,
        } = item
        {
            tool_calls.push(chat::ToolCall {
                id: call_id.clone(),
                call_type: "function".to_string(),
                function: chat::FunctionCall {
                    name: name.clone(),
                    arguments: arguments.clone(),
                },
            });
            finish_reason = "tool_calls";
        }
    }

    if content.is_none() && !tool_calls.is_empty() {
        content = None;
    } else if content.is_none() {
        content = Some(String::new());
    }

    // Build the assistant message
    let message = chat::ChatResponseMessage {
        role: "assistant".to_string(),
        content,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        function_call: None,
    };
    // Create single choice
    let choice = chat::ChatChoice {
        index: 0,
        message,
        finish_reason: Some(finish_reason.to_string()),
        logprobs: None,
    };

    // Convert usage
    let usage = responses_response.usage.as_ref().map(|u| chat::ChatUsage {
        prompt_tokens: u.input_tokens,
        completion_tokens: u.output_tokens,
        total_tokens: u.total_tokens,
        reasoning_tokens: u.reasoning_tokens,
        cached_tokens: u.cached_tokens,
    });

    chat::ChatCompletionResponse {
        id: responses_response.id.clone(),
        object: "chat.completion".to_string(),
        created: responses_response.created,
        model: responses_response.model.clone(),
        choices: vec![choice],
        usage,
        system_fingerprint: responses_response.system_fingerprint.clone(),
    }
}

/// Convert a Chat Completions response to a Responses API response.
///
/// This is useful for testing round-trip conversions.
pub fn chat_to_responses_response(
    chat_response: &chat::ChatCompletionResponse,
) -> resp::ResponsesResponse {
    let mut output_items: Vec<resp::OutputItem> = Vec::new();
    let mut output_text: Option<String> = None;

    // Process first choice (primary response)
    if let Some(choice) = chat_response.choices.first() {
        // Add assistant message if content exists
        if let Some(ref content) = choice.message.content {
            output_text = Some(content.clone());
            output_items.push(resp::OutputItem::AssistantMessage {
                id: format!("msg-{}", chat_response.id),
                content: content.clone(),
            });
        }

        // Add tool calls if present
        if let Some(ref tool_calls) = choice.message.tool_calls {
            for (idx, tool_call) in tool_calls.iter().enumerate() {
                output_items.push(resp::OutputItem::ToolCall {
                    id: format!("call-{}", idx),
                    name: tool_call.function.name.clone(),
                    arguments: tool_call.function.arguments.clone(),
                    call_id: tool_call.id.clone(),
                });
            }
        }
    }

    // Convert usage
    let usage = chat_response.usage.as_ref().map(|u| resp::ResponsesUsage {
        input_tokens: u.prompt_tokens,
        output_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
        reasoning_tokens: u.reasoning_tokens,
        cached_tokens: u.cached_tokens,
    });

    resp::ResponsesResponse {
        id: chat_response.id.clone(),
        object: "response".to_string(),
        created: chat_response.created,
        model: chat_response.model.clone(),
        output_text,
        output: output_items,
        usage,
        system_fingerprint: chat_response.system_fingerprint.clone(),
    }
}

/// Convert a Responses API streaming chunk to a Chat Completions chunk.
pub fn responses_chunk_to_chat_chunk(
    responses_chunk: &resp::ResponsesChunk,
    is_first: bool,
) -> chat::ChatCompletionChunk {
    let mut delta_content = responses_chunk.output_text_delta.clone();
    let mut tool_call_deltas: Vec<chat::ToolCallDelta> = Vec::new();

    if let Some(output_deltas) = &responses_chunk.output_deltas {
        for (idx, item) in output_deltas.iter().enumerate() {
            match item {
                resp::OutputItem::ToolCall {
                    call_id,
                    name,
                    arguments,
                    ..
                } => {
                    tool_call_deltas.push(chat::ToolCallDelta {
                        index: idx as u32,
                        id: Some(call_id.clone()),
                        call_type: Some("function".to_string()),
                        function: Some(chat::FunctionCallDelta {
                            name: Some(name.clone()),
                            arguments: Some(arguments.clone()),
                        }),
                    });
                }
                resp::OutputItem::AssistantMessage { content, .. }
                | resp::OutputItem::FunctionCallOutput { content, .. } => {
                    if delta_content.is_none() {
                        delta_content = Some(content.clone());
                    }
                }
                _ => {}
            }
        }
    }

    let delta = chat::ChatDelta {
        role: if is_first {
            Some("assistant".to_string())
        } else {
            None
        },
        content: delta_content,
        tool_calls: if tool_call_deltas.is_empty() {
            None
        } else {
            Some(tool_call_deltas)
        },
    };

    let choice = chat::ChatStreamChoice {
        index: 0,
        delta,
        finish_reason: None,
    };

    let usage = responses_chunk.usage.as_ref().map(|u| chat::ChatUsage {
        prompt_tokens: u.input_tokens,
        completion_tokens: u.output_tokens,
        total_tokens: u.total_tokens,
        reasoning_tokens: u.reasoning_tokens,
        cached_tokens: u.cached_tokens,
    });

    chat::ChatCompletionChunk {
        id: responses_chunk.id.clone(),
        object: "chat.completion.chunk".to_string(),
        created: responses_chunk.created,
        model: responses_chunk.model.clone(),
        choices: vec![choice],
        usage,
    }
}

// ============================================================================
// Existing Request Conversion Functions
// ============================================================================

pub fn responses_json_to_chat_request(v: &serde_json::Value) -> chat::ChatCompletionRequest {
    let model = v
        .get("model")
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string();

    // Prefer top-level "messages" (Chat API); fall back to "input.messages" (Responses API)
    let messages_val = v
        .get("messages")
        .cloned()
        .or_else(|| {
            v.get("input")
                .and_then(|input| input.get("messages"))
                .cloned()
        })
        .or_else(|| v.get("input").cloned()) // Also support "input" as array directly
        .unwrap_or_else(|| serde_json::Value::Array(vec![]));

    let mut messages: Vec<chat::ChatMessage> = Vec::new();
    if let serde_json::Value::Array(arr) = messages_val {
        for m in arr {
            let role_str = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let role = match role_str {
                "system" => chat::Role::System,
                "user" => chat::Role::User,
                "assistant" => chat::Role::Assistant,
                "tool" => chat::Role::Tool,
                "function" => chat::Role::Function,
                _ => chat::Role::User,
            };
            let content = m.get("content").cloned().unwrap_or(serde_json::Value::Null);
            let name = m
                .get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string());
            let tool_call_id = m
                .get("tool_call_id")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string());
            let tool_calls = m.get("tool_calls").and_then(|tc| {
                tc.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|val| {
                            serde_json::from_value::<chat::ToolCall>(val.clone()).ok()
                        })
                        .collect::<Vec<_>>()
                })
            });
            messages.push(chat::ChatMessage {
                role,
                content,
                name,
                tool_call_id,
                tool_calls: tool_calls.filter(|v| !v.is_empty()),
            });
        }
    }

    // Decoding/sampling
    let temperature = v.get("temperature").and_then(|x| x.as_f64());
    let top_p = v.get("top_p").and_then(|x| x.as_f64());
    let max_tokens = v
        .get("max_output_tokens")
        .and_then(|x| x.as_u64())
        .map(|n| n as u32);
    let stop = v.get("stop").cloned();
    let presence_penalty = v.get("presence_penalty").and_then(|x| x.as_f64());
    let frequency_penalty = v.get("frequency_penalty").and_then(|x| x.as_f64());
    let logit_bias = v
        .get("logit_bias")
        .and_then(|lb| lb.as_object())
        .map(|obj| {
            let mut map = std::collections::HashMap::<String, f64>::new();
            for (k, val) in obj {
                if let Some(f) = val.as_f64() {
                    map.insert(k.clone(), f);
                }
            }
            map
        });
    let user = v
        .get("user")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let n = v.get("n").and_then(|x| x.as_u64()).map(|u| u as u32);

    // Tools
    let tools = v.get("tools").and_then(|t| t.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|tdef| {
                let ttype = tdef.get("type").and_then(|s| s.as_str()).unwrap_or("");
                if ttype == "function" {
                    if let Some(fun) = tdef.get("function") {
                        let name = fun
                            .get("name")
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string())?;
                        let description = fun
                            .get("description")
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string());
                        let parameters = fun
                            .get("parameters")
                            .cloned()
                            .unwrap_or(serde_json::json!({}));
                        Some(chat::ToolDefinition::Function {
                            function: chat::FunctionDef {
                                name,
                                description,
                                parameters,
                            },
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    });

    let tool_choice = v.get("tool_choice").cloned();

    // Response format
    let response_format = v
        .get("response_format")
        .and_then(|rf| rf.as_object())
        .map(|obj| {
            let kind = obj
                .get("type")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let mut extra = std::collections::HashMap::new();
            for (k, val) in obj {
                if k != "type" {
                    extra.insert(k.clone(), val.clone());
                }
            }
            chat::ResponseFormat { kind, extra }
        });

    // Streaming flag
    let stream = v.get("stream").and_then(|x| x.as_bool());

    chat::ChatCompletionRequest {
        model,
        messages,
        temperature,
        top_p,
        max_tokens,
        max_completion_tokens: None,
        stop,
        presence_penalty,
        frequency_penalty,
        logit_bias,
        user,
        n,
        tools,
        tool_choice,
        response_format,
        stream,
    }
}

fn merge_response_extras_into_chat(src: &Value, dest: &mut Value) {
    let Some(src_obj) = src.as_object() else {
        return;
    };
    let Some(dest_obj) = dest.as_object_mut() else {
        return;
    };

    const EXCLUDED_KEYS: [&str; 19] = [
        "model",
        "messages",
        "input",
        "temperature",
        "top_p",
        "max_output_tokens",
        "stop",
        "presence_penalty",
        "frequency_penalty",
        "logit_bias",
        "user",
        "n",
        "tools",
        "tool_choice",
        "response_format",
        "stream",
        "conversation",
        "conversation_id",
        "previous_response_id",
    ];

    for (key, value) in src_obj {
        if dest_obj.contains_key(key) {
            continue;
        }
        if EXCLUDED_KEYS.contains(&key.as_str()) {
            continue;
        }
        dest_obj.insert(key.clone(), value.clone());
    }
}

/// Convert a Responses-shaped JSON payload into a Chat Completions JSON payload,
/// preserving non-standard top-level fields for passthrough backends (e.g., vLLM).
pub fn responses_json_to_chat_value(v: &Value) -> Value {
    let chat_req = responses_json_to_chat_request(v);
    let mut chat_val = serde_json::to_value(chat_req).unwrap_or_else(|_| Value::Object(Map::new()));
    merge_response_extras_into_chat(v, &mut chat_val);
    chat_val
}

#[cfg(test)]
mod tests_responses_to_chat {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_responses_to_chat_basic() {
        let v = json!({
            "model": "gpt-4o-mini",
            "messages": [
                {"role":"system","content":"You are helpful."},
                {"role":"user","content":"Hi"},
                {"role":"assistant","content":"Hello"}
            ],
            "max_output_tokens": 123,
            "tools": [{
                "type":"function",
                "function": {
                    "name":"lookup",
                    "description":"Lookup a value",
                    "parameters":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}
                }
            }],
            "tool_choice": {"type":"function","function":{"name":"lookup"}},
            "response_format": {"type":"json_object","schema":{"type":"object"}},
            "stream": false
        });
        let out = responses_json_to_chat_request(&v);
        assert_eq!(out.model, "gpt-4o-mini");
        assert_eq!(out.messages.len(), 3);
        assert_eq!(out.max_tokens, Some(123));
        assert!(out.tools.as_ref().unwrap().len() == 1);
        assert!(out.tool_choice.is_some());
        assert!(out.response_format.is_some());
        assert_eq!(out.stream, Some(false));
    }

    #[test]
    fn falls_back_to_input_messages() {
        let v = json!({
            "model": "gpt-4o-mini",
            "input": {
                "messages": [
                    {"role":"user","content":"From input.messages"}
                ]
            }
        });
        let out = responses_json_to_chat_request(&v);
        assert_eq!(out.messages.len(), 1);
        assert_eq!(super::role_to_string(&out.messages[0].role), "user");
    }

    #[test]
    fn converts_multimodal_content() {
        use super::map_message_content;

        // Test Chat format to Responses format conversion
        let chat_content = json!([
            {"type": "text", "text": "Describe this image"},
            {"type": "image_url", "image_url": {"url": "https://example.com/image.jpg", "detail": "high"}}
        ]);

        let responses_content = map_message_content(&chat_content);

        // Verify conversion
        let arr = responses_content.as_array().unwrap();
        assert_eq!(arr.len(), 2);

        // Check text conversion
        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[0]["text"], "Describe this image");

        // Check image conversion - URL should be flattened
        assert_eq!(arr[1]["type"], "input_image");
        assert_eq!(arr[1]["image_url"], "https://example.com/image.jpg");
        assert_eq!(arr[1]["detail"], "high");
    }

    #[test]
    fn preserves_simple_text_content() {
        use super::map_message_content;

        // Simple string content should pass through unchanged
        let content = json!("Hello world");
        let result = map_message_content(&content);
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn converts_base64_image() {
        use super::map_message_content;

        let chat_content = json!([
            {"type": "text", "text": "What's in this image?"},
            {
                "type": "image_url",
                "image_url": {
                    "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
                }
            }
        ]);

        let responses_content = map_message_content(&chat_content);
        let arr = responses_content.as_array().unwrap();

        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[1]["type"], "input_image");
        assert!(arr[1]["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn converts_multiple_images() {
        use super::map_message_content;

        let chat_content = json!([
            {"type": "text", "text": "Compare these images"},
            {"type": "image_url", "image_url": {"url": "https://example.com/img1.jpg", "detail": "low"}},
            {"type": "image_url", "image_url": {"url": "https://example.com/img2.jpg", "detail": "high"}},
            {"type": "text", "text": "Which is better?"}
        ]);

        let responses_content = map_message_content(&chat_content);
        let arr = responses_content.as_array().unwrap();

        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[1]["type"], "input_image");
        assert_eq!(arr[1]["detail"], "low");
        assert_eq!(arr[2]["type"], "input_image");
        assert_eq!(arr[2]["detail"], "high");
        assert_eq!(arr[3]["type"], "input_text");
    }

    #[test]
    fn converts_image_without_detail() {
        use super::map_message_content;

        // Detail parameter is optional
        let chat_content = json!([
            {
                "type": "image_url",
                "image_url": {
                    "url": "https://example.com/image.jpg"
                }
            }
        ]);

        let responses_content = map_message_content(&chat_content);
        let arr = responses_content.as_array().unwrap();

        assert_eq!(arr[0]["type"], "input_image");
        assert_eq!(arr[0]["image_url"], "https://example.com/image.jpg");
        // detail should not be present if not in source
        assert!(arr[0].get("detail").is_none() || arr[0]["detail"].is_null());
    }

    #[test]
    fn preserves_unknown_content_types() {
        use super::map_message_content;

        // Unknown types should pass through unchanged
        let chat_content = json!([
            {"type": "custom_type", "data": "some value"}
        ]);

        let responses_content = map_message_content(&chat_content);
        let arr = responses_content.as_array().unwrap();

        assert_eq!(arr[0]["type"], "custom_type");
        assert_eq!(arr[0]["data"], "some value");
    }

    #[test]
    fn handles_empty_content_array() {
        use super::map_message_content;

        let chat_content = json!([]);
        let responses_content = map_message_content(&chat_content);

        assert_eq!(responses_content.as_array().unwrap().len(), 0);
    }

    #[test]
    fn converts_vision_with_tools_request() {
        // Test full request conversion with both vision and tools
        let req = chat::ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![chat::ChatMessage {
                role: chat::Role::User,
                content: json!([
                    {"type": "text", "text": "Analyze this image and use tools if needed"},
                    {"type": "image_url", "image_url": {"url": "https://example.com/chart.png", "detail": "high"}}
                ]),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: Some(500),
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: Some(vec![chat::ToolDefinition::Function {
                function: chat::FunctionDef {
                    name: "analyze_data".into(),
                    description: Some("Analyze data from image".into()),
                    parameters: json!({"type": "object", "properties": {}}),
                },
            }]),
            tool_choice: Some(json!("auto")),
            response_format: None,
            stream: None,
        };

        let out = super::to_responses_request(&req, None);

        // Verify messages converted correctly
        assert_eq!(out.messages.len(), 1);
        let content = &out.messages[0].content;
        assert!(content.is_array());

        let arr = content.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[1]["type"], "input_image");
        assert_eq!(arr[1]["image_url"], "https://example.com/chart.png");
        assert_eq!(arr[1]["detail"], "high");

        // Verify tools are present
        assert!(out.tools.is_some());
        assert_eq!(out.tool_choice, Some(json!("auto")));
    }

    #[test]
    fn converts_reasoning_model_multimodal() {
        // Test multimodal with reasoning models (o1, o3, gpt-5)
        let req = chat::ChatCompletionRequest {
            model: "o1-preview".into(),
            messages: vec![chat::ChatMessage {
                role: chat::Role::User,
                content: json!([
                    {"type": "text", "text": "Think step by step about this image"},
                    {"type": "image_url", "image_url": {"url": "https://example.com/problem.jpg"}}
                ]),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: Some(4096),
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
        };

        let out = super::to_responses_request(&req, None);

        assert_eq!(out.model, "o1-preview");
        assert_eq!(out.max_output_tokens, Some(4096));

        let content = &out.messages[0].content;
        let arr = content.as_array().unwrap();
        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[1]["type"], "input_image");
    }
}

/// Convert an OpenAI Chat Completions request into a Responses API request (wrapping chat messages under `input.messages`).
///
/// Mapping highlights:
/// - messages: forwarded 1:1 under `input.messages` (Responses chat-form).
///   Role mapping: system|user|assistant|tool; legacy "function" is mapped to "tool".
/// - max_tokens -> max_output_tokens (Responses naming).
/// - tools (function) and tool_choice: forwarded preserving JSON schema.
/// - response_format: forwarded as an object; `{ "type": <kind>, ...extras }`.
/// - stream: forwarded verbatim (used by the proxy to request SSE).
/// - conversation: optional Responses-side conversation identifier for stateful calls.
pub fn to_responses_request(
    src: &chat::ChatCompletionRequest,
    conversation: Option<String>,
) -> resp::ResponsesRequest {
    let messages = map_messages(&src.messages);

    let tools = src
        .tools
        .as_ref()
        .map(|ts| ts.iter().map(map_tool).collect::<Vec<_>>());

    let response_format = src.response_format.as_ref().map(map_response_format);

    let mut max_output_tokens = src.max_completion_tokens.or(src.max_tokens);
    if let Some(max_tokens) = max_output_tokens {
        if max_tokens < 16 {
            max_output_tokens = Some(16);
        }
    }

    let tool_choice = src.tool_choice.as_ref().map(map_tool_choice_for_responses);

    resp::ResponsesRequest {
        model: src.model.clone(),
        messages,
        // Sampling / decoding
        temperature: src.temperature,
        top_p: src.top_p,
        // Prefer max_completion_tokens (newer parameter) over max_tokens
        max_output_tokens,
        stop: src.stop.clone(),
        presence_penalty: src.presence_penalty,
        frequency_penalty: src.frequency_penalty,
        logit_bias: src.logit_bias.clone(),
        user: src.user.clone(),
        n: src.n,
        // Tools
        tools,
        tool_choice,
        // Output shaping
        response_format,
        // Streaming
        stream: src.stream,
        // Stateful conversation id (optional)
        conversation,
        previous_response_id: None,
    }
}

/// Convert an OpenAI Chat Completions request into a Responses API request with MCP tools merged in.
pub async fn to_responses_request_with_mcp(
    src: &chat::ChatCompletionRequest,
    conversation: Option<String>,
    mcp_manager: Option<&crate::mcp_client::McpClientManager>,
) -> resp::ResponsesRequest {
    let mut request = to_responses_request(src, conversation);

    // Add MCP tools if manager is available
    if let Some(manager) = mcp_manager {
        if let Ok(mcp_tools) = manager.list_all_tools().await {
            let mcp_tool_definitions: Vec<resp::ResponsesToolDefinition> = mcp_tools
                .iter()
                .map(|tool| {
                    // Convert MCP tool to Responses tool definition (flat structure)
                    resp::ResponsesToolDefinition::Function {
                        name: format!("{}_{}", tool.server_name, tool.name),
                        description: tool.description.clone(),
                        parameters: tool.input_schema.clone(),
                    }
                })
                .collect();

            // Merge with existing tools
            let mut all_tools = request.tools.unwrap_or_default();
            all_tools.extend(mcp_tool_definitions);
            request.tools = if all_tools.is_empty() {
                None
            } else {
                Some(all_tools)
            };
        }
    }

    request
}

/// Convert an OpenAI Chat Completions request with MCP tools and system prompt injection
pub async fn to_responses_request_with_mcp_and_prompt(
    src: &chat::ChatCompletionRequest,
    conversation: Option<String>,
    mcp_manager: Option<&crate::mcp_client::McpClientManager>,
    system_prompt_config: Option<&crate::system_prompt_config::SystemPromptConfig>,
) -> resp::ResponsesRequest {
    let mut request = to_responses_request(src, conversation);

    // Add MCP tools if manager is available
    if let Some(manager) = mcp_manager {
        if let Ok(mcp_tools) = manager.list_all_tools().await {
            let mcp_tool_definitions: Vec<resp::ResponsesToolDefinition> = mcp_tools
                .iter()
                .map(|tool| {
                    // Convert MCP tool to Responses tool definition (flat structure)
                    resp::ResponsesToolDefinition::Function {
                        name: format!("{}_{}", tool.server_name, tool.name),
                        description: tool.description.clone(),
                        parameters: tool.input_schema.clone(),
                    }
                })
                .collect();

            // Merge with existing tools
            let mut all_tools = request.tools.unwrap_or_default();
            all_tools.extend(mcp_tool_definitions);
            request.tools = if all_tools.is_empty() {
                None
            } else {
                Some(all_tools)
            };
        }
    }

    // Inject system prompt if configured
    if let Some(config) = system_prompt_config {
        if let Some(prompt) = config.get_prompt(Some(&request.model), Some("responses")) {
            inject_system_prompt(&mut request.messages, &prompt, &config.injection_mode);
        }
    }

    request
}

/// Inject system prompt into Chat Completions request
pub fn inject_system_prompt_chat(req: &mut chat::ChatCompletionRequest, prompt: &str, mode: &str) {
    let system_message = chat::ChatMessage {
        role: chat::Role::System,
        content: serde_json::Value::String(prompt.to_string()),
        name: None,
        tool_call_id: None,
        tool_calls: None,
    };

    match mode {
        "append" => {
            // Find last system message position or append at end
            let last_system_pos = req
                .messages
                .iter()
                .rposition(|m| matches!(m.role, chat::Role::System));

            if let Some(pos) = last_system_pos {
                req.messages.insert(pos + 1, system_message);
            } else {
                req.messages.push(system_message);
            }
        }
        "replace" => {
            // Remove all existing system messages and prepend new one
            req.messages
                .retain(|m| !matches!(m.role, chat::Role::System));
            req.messages.insert(0, system_message);
        }
        _ => {
            // Default: prepend
            req.messages.insert(0, system_message);
        }
    }
}

/// Inject system prompt into Responses messages
pub fn inject_system_prompt(messages: &mut Vec<resp::ResponsesMessage>, prompt: &str, mode: &str) {
    let system_message = resp::ResponsesMessage {
        role: "system".to_string(),
        content: serde_json::Value::String(prompt.to_string()),
        name: None,
        tool_call_id: None,
        tool_calls: None,
    };

    match mode {
        "append" => {
            // Find last system message position or append at end
            let last_system_pos = messages.iter().rposition(|m| m.role == "system");

            if let Some(pos) = last_system_pos {
                messages.insert(pos + 1, system_message);
            } else {
                messages.push(system_message);
            }
        }
        "replace" => {
            // Remove all existing system messages and prepend new one
            messages.retain(|m| m.role != "system");
            messages.insert(0, system_message);
        }
        _ => {
            // Default: prepend
            messages.insert(0, system_message);
        }
    }
}

fn map_messages(src: &[chat::ChatMessage]) -> Vec<resp::ResponsesMessage> {
    src.iter()
        .map(|m| {
            let tool_calls_raw = m.tool_calls.as_ref().map(|calls| {
                calls
                    .iter()
                    .filter_map(|call| serde_json::to_value(call).ok())
                    .collect::<Vec<_>>()
            });

            let mut content_value = map_message_content(&m.content);
            let has_tool_calls = tool_calls_raw
                .as_ref()
                .is_some_and(|items| !items.is_empty());

            if content_value.is_null() && matches!(m.role, chat::Role::Assistant) && has_tool_calls
            {
                content_value = serde_json::Value::String(String::new());
            }

            let tool_calls = tool_calls_raw
                .as_ref()
                .filter(|items| !items.is_empty())
                .cloned();

            resp::ResponsesMessage {
                role: role_to_string(&m.role).to_string(),
                content: content_value,
                name: m.name.clone(),
                tool_call_id: m.tool_call_id.clone(),
                tool_calls,
            }
        })
        .collect()
}

/// Convert Chat API content format to Responses API content format.
///
/// Chat API uses:
/// - Simple string for text
/// - Array with `{"type": "text", "text": "..."}` and `{"type": "image_url", "image_url": {"url": "...", "detail": "..."}}`
///
/// Responses API uses:
/// - Simple string for text (unchanged)
/// - Array with `{"type": "input_text", "text": "..."}` and `{"type": "input_image", "image_url": "...", "detail": "..."}`
fn map_message_content(content: &serde_json::Value) -> serde_json::Value {
    use serde_json::{Map, Value};

    match content {
        // Simple string content - pass through unchanged
        Value::String(_) => content.clone(),

        // Array content - may need transformation for multimodal
        Value::Array(parts) => {
            let transformed_parts: Vec<Value> = parts
                .iter()
                .map(|part| {
                    if let Some(obj) = part.as_object() {
                        let mut new_obj = Map::new();

                        // Get the type field
                        if let Some(Value::String(type_str)) = obj.get("type") {
                            match type_str.as_str() {
                                // Convert Chat "text" to Responses "input_text"
                                "text" => {
                                    new_obj.insert(
                                        "type".to_string(),
                                        Value::String("input_text".to_string()),
                                    );
                                    if let Some(text) = obj.get("text") {
                                        new_obj.insert("text".to_string(), text.clone());
                                    }
                                }

                                // Convert Chat "image_url" to Responses "input_image"
                                "image_url" => {
                                    new_obj.insert(
                                        "type".to_string(),
                                        Value::String("input_image".to_string()),
                                    );

                                    // Flatten the nested image_url structure
                                    if let Some(Value::Object(image_url_obj)) = obj.get("image_url")
                                    {
                                        if let Some(url) = image_url_obj.get("url") {
                                            new_obj.insert("image_url".to_string(), url.clone());
                                        }
                                        if let Some(detail) = image_url_obj.get("detail") {
                                            new_obj.insert("detail".to_string(), detail.clone());
                                        }
                                    }
                                }

                                // Pass through other types unchanged (future compatibility)
                                _ => {
                                    return part.clone();
                                }
                            }
                            Value::Object(new_obj)
                        } else {
                            // No type field - pass through unchanged
                            part.clone()
                        }
                    } else {
                        // Not an object - pass through unchanged
                        part.clone()
                    }
                })
                .collect();

            Value::Array(transformed_parts)
        }

        // Other types (null, object, numbers) - pass through unchanged
        Value::Null => Value::Null,
        _ => content.clone(),
    }
}

fn role_to_string(role: &chat::Role) -> &'static str {
    match role {
        chat::Role::System => "system",
        chat::Role::User => "user",
        chat::Role::Assistant => "assistant",
        chat::Role::Tool => "tool",
        // Legacy Chat role; Responses expects "tool" for tool outputs.
        chat::Role::Function => "tool",
    }
}

fn map_tool(t: &chat::ToolDefinition) -> resp::ResponsesToolDefinition {
    match t {
        chat::ToolDefinition::Function { function } => resp::ResponsesToolDefinition::Function {
            name: function.name.clone(),
            description: function.description.clone(),
            parameters: function.parameters.clone(),
        },
    }
}

fn map_tool_choice_for_responses(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::{Map, Value};

    match value {
        Value::String(_) => value.clone(),
        Value::Object(obj) => {
            if let Some(Value::String(kind)) = obj.get("type") {
                if kind == "function" {
                    if let Some(function) = obj.get("function").and_then(|f| f.as_object()) {
                        if let Some(Value::String(name)) = function.get("name") {
                            let mut out = Map::new();
                            out.insert("type".to_string(), Value::String("function".to_string()));
                            out.insert("name".to_string(), Value::String(name.clone()));
                            if let Some(arguments) = function.get("arguments") {
                                out.insert("arguments".to_string(), arguments.clone());
                            }
                            return Value::Object(out);
                        }
                    }
                }
            }
            value.clone()
        }
        _ => value.clone(),
    }
}

fn map_response_format(rf: &chat::ResponseFormat) -> Value {
    // Build an object: { "type": rf.kind, ...rf.extra } with "type" from kind
    let mut obj = Map::<String, Value>::new();
    obj.insert("type".to_string(), Value::String(rf.kind.clone()));
    for (k, v) in rf.extra.iter() {
        // Guard against accidental override of "type" inside extras.
        if k != "type" {
            obj.insert(k.clone(), v.clone());
        }
    }
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::chat::{
        ChatCompletionRequest, ChatMessage, FunctionDef, ResponseFormat, Role, ToolDefinition,
    };
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn maps_basic_fields() {
        let req = ChatCompletionRequest {
            model: "gpt-4o-mini".into(),
            messages: vec![
                ChatMessage {
                    role: Role::System,
                    content: json!("You are helpful."),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                ChatMessage {
                    role: Role::User,
                    content: json!("Hello"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
            ],
            temperature: Some(0.3),
            top_p: Some(0.95),
            max_tokens: Some(128),
            max_completion_tokens: None,
            stop: None,
            presence_penalty: Some(0.0),
            frequency_penalty: Some(0.0),
            logit_bias: None,
            user: Some("unit".into()),
            n: Some(1),
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: Some(false),
        };

        let out = to_responses_request(&req, Some("conv-xyz".into()));
        assert_eq!(out.model, "gpt-4o-mini");
        assert_eq!(out.messages.len(), 2);
        assert_eq!(out.messages[0].role, "system");
        assert_eq!(out.messages[1].role, "user");
        assert_eq!(out.max_output_tokens, Some(128));
        assert_eq!(out.temperature, Some(0.3));
        assert_eq!(out.top_p, Some(0.95));
        assert_eq!(out.conversation.as_deref(), Some("conv-xyz"));
        assert_eq!(out.stream, Some(false));
    }

    #[test]
    fn maps_tools_and_response_format() {
        let mut extra = HashMap::new();
        extra.insert(
            "schema".into(),
            json!({"type":"object","properties":{"x":{"type":"string"}}}),
        );

        let req = ChatCompletionRequest {
            model: "gpt-4o-mini".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Call a tool"),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: Some(vec![ToolDefinition::Function {
                function: FunctionDef {
                    name: "lookup".into(),
                    description: Some("Lookup a value".into()),
                    parameters: json!({
                        "type": "object",
                        "properties": { "key": { "type": "string" } },
                        "required": ["key"]
                    }),
                },
            }]),
            tool_choice: Some(json!({"type":"function","function":{"name":"lookup"}})),
            response_format: Some(ResponseFormat {
                kind: "json_object".into(),
                extra,
            }),
            stream: Some(true),
        };

        let out = to_responses_request(&req, None);
        assert!(out.tools.is_some());
        let tools = out.tools.unwrap();
        assert_eq!(tools.len(), 1);
        #[allow(irrefutable_let_patterns)]
        if let resp::ResponsesToolDefinition::Function {
            name, description, ..
        } = &tools[0]
        {
            assert_eq!(name, "lookup");
            assert!(description.as_deref() == Some("Lookup a value"));
        } else {
            panic!("expected function tool");
        }

        let rf = out.response_format.expect("response_format missing");
        assert_eq!(rf.get("type").and_then(|v| v.as_str()), Some("json_object"));
        assert!(rf.get("schema").is_some());
        assert_eq!(out.stream, Some(true));
    }

    #[test]
    fn maps_function_role_to_tool() {
        let req = ChatCompletionRequest {
            model: "gpt-4o-mini".into(),
            messages: vec![ChatMessage {
                role: Role::Function,
                content: json!("result"),
                name: Some("fn".into()),
                tool_call_id: Some("t1".into()),
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.messages.len(), 1);
        assert_eq!(out.messages[0].role, "tool");
        assert_eq!(out.messages[0].name.as_deref(), Some("fn"));
        assert_eq!(out.messages[0].tool_call_id.as_deref(), Some("t1"));
    }
}
