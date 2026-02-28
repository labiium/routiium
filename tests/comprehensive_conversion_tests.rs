/// Comprehensive conversion tests based on OpenAI Chat Completions and Responses API specification.
///
/// This test suite validates 100% correctness of conversion including:
/// - Request conversion (Chat → Responses)
/// - Response conversion (Responses → Chat)
/// - Reasoning tokens handling
/// - Streaming mode
/// - Tool/function calling
/// - Edge cases and validation
/// - Round-trip conversions
use routiium::models::chat::{
    ChatCompletionRequest, ChatMessage, FunctionDef, ResponseFormat, Role, ToolDefinition,
};
use routiium::models::responses::ResponsesToolDefinition;
use routiium::to_responses_request;
use serde_json::{json, Value};
use std::collections::HashMap;

// ============================================================================
// SECTION 1: Request Conversion Tests (Chat → Responses)
// ============================================================================

mod request_conversion {
    use super::*;

    #[test]
    fn test_model_mapping() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Hello"),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.model, "gpt-4o", "Model name must be preserved exactly");
    }

    #[test]
    fn test_messages_array_mapping() {
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
                    content: json!("Hi"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                ChatMessage {
                    role: Role::Assistant,
                    content: json!("Hello! How can I help?"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
            ],
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
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        assert_eq!(out.messages.len(), 3, "All messages must be preserved");
        assert_eq!(out.messages[0].role, "system");
        assert_eq!(out.messages[1].role, "user");
        assert_eq!(out.messages[2].role, "assistant");

        // Content must be preserved exactly
        assert_eq!(out.messages[0].content, json!("You are helpful."));
        assert_eq!(out.messages[1].content, json!("Hi"));
        assert_eq!(out.messages[2].content, json!("Hello! How can I help?"));
    }

    #[test]
    fn test_max_tokens_to_max_output_tokens() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: Some(1024),
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
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(
            out.max_output_tokens,
            Some(1024),
            "max_tokens must map to max_output_tokens"
        );
    }

    #[test]
    fn test_streaming_flag_preserved() {
        let test_cases = vec![
            (Some(true), Some(true)),
            (Some(false), Some(false)),
            (None, None),
        ];

        for (input_stream, expected_stream) in test_cases {
            let req = ChatCompletionRequest {
                model: "gpt-4o".into(),
                messages: vec![ChatMessage {
                    role: Role::User,
                    content: json!("Test"),
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
                tools: None,
                tool_choice: None,
                response_format: None,
                stream: input_stream,
                extra_body: None,
            };

            let out = to_responses_request(&req, None);
            assert_eq!(
                out.stream, expected_stream,
                "stream flag must be preserved exactly"
            );
        }
    }

    #[test]
    fn test_temperature_and_top_p_preserved() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: Some(0.7),
            top_p: Some(0.9),
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
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.temperature, Some(0.7));
        assert_eq!(out.top_p, Some(0.9));
    }

    #[test]
    fn test_stop_sequences_string_and_array() {
        // Test single string stop
        let req1 = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_completion_tokens: None,
            stop: Some(json!("\n\n")),
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out1 = to_responses_request(&req1, None);
        assert_eq!(out1.stop, Some(json!("\n\n")));

        // Test array of strings
        let req2 = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_completion_tokens: None,
            stop: Some(json!(["END", "STOP", "---"])),
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out2 = to_responses_request(&req2, None);
        assert_eq!(out2.stop, Some(json!(["END", "STOP", "---"])));
    }

    #[test]
    fn test_user_field_preserved() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
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
            user: Some("user-12345".into()),
            n: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.user, Some("user-12345".into()));
    }

    #[test]
    fn test_n_parameter_preserved() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
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
            n: Some(3),
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.n, Some(3));
    }

    #[test]
    fn test_penalties_preserved() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_completion_tokens: None,
            stop: None,
            presence_penalty: Some(0.5),
            frequency_penalty: Some(0.3),
            logit_bias: None,
            user: None,
            n: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.presence_penalty, Some(0.5));
        assert_eq!(out.frequency_penalty, Some(0.3));
    }

    #[test]
    fn test_logit_bias_preserved() {
        let mut bias = HashMap::new();
        bias.insert("50256".into(), -100.0);
        bias.insert("1234".into(), 50.0);

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
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
            logit_bias: Some(bias.clone()),
            user: None,
            n: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.logit_bias, Some(bias));
    }
}

// ============================================================================
// SECTION 2: Role Mapping Tests
// ============================================================================

mod role_mapping {
    use super::*;

    #[test]
    fn test_all_role_mappings() {
        let roles = vec![
            (Role::System, "system"),
            (Role::User, "user"),
            (Role::Assistant, "assistant"),
            (Role::Tool, "tool"),
            (Role::Function, "tool"), // Legacy function → tool
        ];

        for (chat_role, expected_responses_role) in roles {
            let req = ChatCompletionRequest {
                model: "gpt-4o".into(),
                messages: vec![ChatMessage {
                    role: chat_role.clone(),
                    content: json!("test"),
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
                tools: None,
                tool_choice: None,
                response_format: None,
                stream: None,
                extra_body: None,
            };

            let out = to_responses_request(&req, None);
            assert_eq!(
                out.messages[0].role, expected_responses_role,
                "Role {:?} must map to {}",
                chat_role, expected_responses_role
            );
        }
    }

    #[test]
    fn test_function_role_converts_to_tool_with_metadata() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::Function,
                content: json!({"result": "success"}),
                name: Some("my_function".into()),
                tool_call_id: Some("call-abc123".into()),
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
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.messages[0].role, "tool");
        assert_eq!(out.messages[0].name, Some("my_function".into()));
        assert_eq!(out.messages[0].tool_call_id, Some("call-abc123".into()));
    }

    #[test]
    fn test_tool_message_metadata_preserved() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::Tool,
                content: json!("Tool result"),
                name: Some("search_tool".into()),
                tool_call_id: Some("call-xyz".into()),
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
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.messages[0].name, Some("search_tool".into()));
        assert_eq!(out.messages[0].tool_call_id, Some("call-xyz".into()));
    }
}

// ============================================================================
// SECTION 3: Tool/Function Calling Conversion Tests
// ============================================================================

mod tool_conversion {
    use super::*;

    #[test]
    fn test_single_function_tool_conversion() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Use the lookup tool"),
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
                    description: Some("Look up information".into()),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "The search query"
                            }
                        },
                        "required": ["query"]
                    }),
                },
            }]),
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        assert!(out.tools.is_some());
        let tools = out.tools.unwrap();
        assert_eq!(tools.len(), 1);

        match &tools[0] {
            ResponsesToolDefinition::Function {
                name,
                description,
                parameters,
            } => {
                assert_eq!(name, "lookup");
                assert_eq!(description, &Some("Look up information".into()));

                // Validate parameters schema is preserved
                assert_eq!(parameters["type"], "object");
                assert!(parameters["properties"].is_object());
                assert_eq!(parameters["required"], json!(["query"]));
            }
        }
    }

    #[test]
    fn test_multiple_tools_conversion() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Use tools"),
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
            tools: Some(vec![
                ToolDefinition::Function {
                    function: FunctionDef {
                        name: "search".into(),
                        description: Some("Search the web".into()),
                        parameters: json!({"type": "object", "properties": {}}),
                    },
                },
                ToolDefinition::Function {
                    function: FunctionDef {
                        name: "calculate".into(),
                        description: Some("Perform calculations".into()),
                        parameters: json!({"type": "object", "properties": {}}),
                    },
                },
                ToolDefinition::Function {
                    function: FunctionDef {
                        name: "weather".into(),
                        description: None,
                        parameters: json!({"type": "object"}),
                    },
                },
            ]),
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        let tools = out.tools.unwrap();
        assert_eq!(tools.len(), 3, "All tools must be converted");

        let names: Vec<String> = tools
            .iter()
            .map(|t| match t {
                ResponsesToolDefinition::Function { name, .. } => name.clone(),
            })
            .collect();

        assert_eq!(names, vec!["search", "calculate", "weather"]);
    }

    #[test]
    fn test_tool_choice_preserved() {
        let tool_choice_cases = vec![
            json!("auto"),
            json!("none"),
            json!({"type": "function", "function": {"name": "lookup"}}),
        ];

        for tool_choice in tool_choice_cases {
            let req = ChatCompletionRequest {
                model: "gpt-4o".into(),
                messages: vec![ChatMessage {
                    role: Role::User,
                    content: json!("Test"),
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
                        description: None,
                        parameters: json!({}),
                    },
                }]),
                tool_choice: Some(tool_choice.clone()),
                response_format: None,
                stream: None,
                extra_body: None,
            };

            let out = to_responses_request(&req, None);
            match (&tool_choice, out.tool_choice.clone()) {
                (Value::String(_), Some(actual)) => assert_eq!(
                    actual,
                    tool_choice.clone(),
                    "string tool_choice should be forwarded unchanged"
                ),
                (Value::Object(orig), Some(Value::Object(out_map))) => {
                    let is_function = orig
                        .get("type")
                        .and_then(Value::as_str)
                        .map(|s| s == "function")
                        .unwrap_or(false);

                    if is_function {
                        let function = orig
                            .get("function")
                            .and_then(Value::as_object)
                            .expect("function tool choice must include object");
                        let fn_name = function
                            .get("name")
                            .and_then(Value::as_str)
                            .expect("function tool choice must include name");

                        assert_eq!(
                            out_map.get("type"),
                            Some(&Value::String("function".to_string())),
                            "tool_choice type should remain function"
                        );
                        assert_eq!(
                            out_map.get("name"),
                            Some(&Value::String(fn_name.to_string())),
                            "tool_choice function name should be flattened"
                        );

                        match function.get("arguments") {
                            Some(args) => assert_eq!(
                                out_map.get("arguments"),
                                Some(args),
                                "tool_choice function arguments should be preserved"
                            ),
                            None => assert!(
                                !out_map.contains_key("arguments"),
                                "tool_choice function should not introduce arguments"
                            ),
                        }
                    } else {
                        assert_eq!(
                            Value::Object(out_map.clone()),
                            tool_choice.clone(),
                            "non-function tool_choice object should be unchanged"
                        );
                    }
                }
                (expected, actual) => panic!(
                    "Unexpected tool_choice mapping: {:?} -> {:?}",
                    expected, actual
                ),
            }
        }
    }

    #[test]
    fn test_empty_tools_array_handling() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
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
            tools: Some(vec![]),
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        // Empty tools array should be preserved
        assert!(out.tools.is_some());
        assert_eq!(out.tools.unwrap().len(), 0);
    }

    #[test]
    fn test_complex_tool_parameters_schema() {
        let complex_schema = json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "object",
                    "properties": {
                        "city": {"type": "string"},
                        "country": {"type": "string"}
                    },
                    "required": ["city"]
                },
                "units": {
                    "type": "string",
                    "enum": ["celsius", "fahrenheit"]
                },
                "days": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10
                }
            },
            "required": ["location"]
        });

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Get weather"),
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
                    name: "get_weather".into(),
                    description: Some("Get weather forecast".into()),
                    parameters: complex_schema.clone(),
                },
            }]),
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        let tools = out.tools.unwrap();
        match &tools[0] {
            ResponsesToolDefinition::Function { parameters, .. } => {
                assert_eq!(parameters, &complex_schema);
            }
        }
    }
}

// ============================================================================
// SECTION 4: Response Format / Structured Output Tests
// ============================================================================

mod response_format {
    use super::*;

    #[test]
    fn test_json_object_response_format() {
        let mut extra = HashMap::new();
        extra.insert(
            "schema".into(),
            json!({
                "type": "object",
                "properties": {
                    "answer": {"type": "string"}
                },
                "required": ["answer"]
            }),
        );

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Return JSON"),
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
            tools: None,
            tool_choice: None,
            response_format: Some(ResponseFormat {
                kind: "json_object".into(),
                extra,
            }),
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        let rf = out
            .response_format
            .expect("response_format must be present");
        assert_eq!(rf["type"], "json_object");
        assert!(rf["schema"].is_object());
        assert_eq!(rf["schema"]["type"], "object");
    }

    #[test]
    fn test_response_format_type_field_not_overridden() {
        let mut extra = HashMap::new();
        extra.insert("type".into(), json!("should_be_ignored"));
        extra.insert("schema".into(), json!({"type": "object"}));

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
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
            tools: None,
            tool_choice: None,
            response_format: Some(ResponseFormat {
                kind: "json_schema".into(),
                extra,
            }),
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        let rf = out.response_format.unwrap();
        assert_eq!(
            rf["type"], "json_schema",
            "Type field must come from 'kind', not extras"
        );
    }

    #[test]
    fn test_response_format_with_no_extras() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
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
            tools: None,
            tool_choice: None,
            response_format: Some(ResponseFormat {
                kind: "text".into(),
                extra: HashMap::new(),
            }),
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        let rf = out.response_format.unwrap();
        assert_eq!(rf["type"], "text");
        // Should only have "type" field
        assert_eq!(rf.as_object().unwrap().len(), 1);
    }
}

// ============================================================================
// SECTION 5: Multimodal Content Tests
// ============================================================================

mod multimodal_content {
    use super::*;

    #[test]
    fn test_text_only_content() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Simple text message"),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.messages[0].content, json!("Simple text message"));
    }

    #[test]
    fn test_multimodal_content_array() {
        let content = json!([
            {
                "type": "text",
                "text": "What's in this image?"
            },
            {
                "type": "image_url",
                "image_url": {
                    "url": "https://example.com/image.jpg",
                    "detail": "high"
                }
            }
        ]);

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: content.clone(),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        // Verify content is converted to Responses API format
        let converted_content = &out.messages[0].content;
        assert!(converted_content.is_array());

        let arr = converted_content.as_array().unwrap();
        assert_eq!(arr.len(), 2);

        // Check text conversion: "text" → "input_text"
        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[0]["text"], "What's in this image?");

        // Check image conversion: "image_url" → "input_image" with flattened URL
        assert_eq!(arr[1]["type"], "input_image");
        assert_eq!(arr[1]["image_url"], "https://example.com/image.jpg");
        assert_eq!(arr[1]["detail"], "high");
    }

    #[test]
    fn test_vision_url_format_conversion() {
        // Test that nested image_url.url is flattened to image_url in Responses API
        let content = json!([
            {
                "type": "image_url",
                "image_url": {
                    "url": "https://upload.wikimedia.org/wikipedia/commons/thumb/d/dd/Gfp-wisconsin-madison-the-nature-boardwalk.jpg/2560px-Gfp-wisconsin-madison-the-nature-boardwalk.jpg"
                }
            }
        ]);

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content,
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        let arr = out.messages[0].content.as_array().unwrap();

        // Verify URL is flattened
        assert_eq!(arr[0]["type"], "input_image");
        assert!(arr[0]["image_url"].is_string());
        assert!(arr[0]["image_url"].as_str().unwrap().contains("wikipedia"));
        // Verify nested structure is gone
        assert!(arr[0].get("image_url").unwrap().get("url").is_none());
    }

    #[test]
    fn test_base64_vision_content() {
        let content = json!([
            {
                "type": "text",
                "text": "Describe this base64 image"
            },
            {
                "type": "image_url",
                "image_url": {
                    "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
                }
            }
        ]);

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content,
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        let arr = out.messages[0].content.as_array().unwrap();

        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[1]["type"], "input_image");
        assert!(arr[1]["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn test_mixed_content_messages() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![
                ChatMessage {
                    role: Role::User,
                    content: json!("Plain text"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                ChatMessage {
                    role: Role::User,
                    content: json!([
                        {"type": "text", "text": "Analyze this"},
                        {"type": "image_url", "image_url": {"url": "https://example.com/img.png"}}
                    ]),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
            ],
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
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.messages[0].content, json!("Plain text"));
        assert!(out.messages[1].content.is_array());
        assert_eq!(out.messages[1].content.as_array().unwrap().len(), 2);
    }
}

// ============================================================================
// SECTION 6: Edge Cases and Validation Tests
// ============================================================================

mod edge_cases {
    use super::*;

    #[test]
    fn test_empty_messages_array() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![],
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
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.messages.len(), 0);
    }

    #[test]
    fn test_null_content() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::Assistant,
                content: json!(null),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.messages[0].content, json!(null));
    }

    #[test]
    fn test_conversation_id_parameter() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out1 = to_responses_request(&req, None);
        assert!(out1.conversation.is_none());

        let out2 = to_responses_request(&req, Some("conv-12345".into()));
        assert_eq!(out2.conversation, Some("conv-12345".into()));
    }

    #[test]
    fn test_all_optional_fields_none() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Minimal request"),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        // Should serialize without optional fields
        let serialized = serde_json::to_value(&out).unwrap();

        // Required fields must be present
        assert!(serialized["model"].is_string());
        // In Responses API format, messages are serialized as "input" array
        assert!(serialized["input"].is_array());

        // Optional fields should not be serialized when None
        assert!(
            !serialized.as_object().unwrap().contains_key("temperature")
                || serialized["temperature"].is_null()
        );
    }

    #[test]
    fn test_extreme_parameter_values() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: Some(2.0), // Max temperature
            top_p: Some(1.0),       // Max top_p
            max_tokens: Some(4096), // Large token count
            max_completion_tokens: None,
            stop: None,
            presence_penalty: Some(2.0),   // Max penalty
            frequency_penalty: Some(-2.0), // Min penalty
            logit_bias: None,
            user: None,
            n: Some(10), // Multiple completions
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.temperature, Some(2.0));
        assert_eq!(out.top_p, Some(1.0));
        assert_eq!(out.max_output_tokens, Some(4096));
        assert_eq!(out.presence_penalty, Some(2.0));
        assert_eq!(out.frequency_penalty, Some(-2.0));
        assert_eq!(out.n, Some(10));
    }

    #[test]
    fn test_very_long_message_history() {
        let mut messages = Vec::new();
        for i in 0..100 {
            messages.push(ChatMessage {
                role: if i % 2 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                },
                content: json!(format!("Message {}", i)),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            });
        }

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages,
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
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        assert_eq!(out.messages.len(), 100);

        // Verify order is preserved
        for i in 0..100 {
            assert_eq!(out.messages[i].content, json!(format!("Message {}", i)));
        }
    }
}

// ============================================================================
// SECTION 7: Serialization Tests
// ============================================================================

mod serialization {
    use super::*;

    #[test]
    fn test_responses_request_serializes_to_valid_json() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
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
            temperature: Some(0.7),
            top_p: None,
            max_tokens: Some(100),
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
            stream: Some(false),
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        let serialized = serde_json::to_value(&out);

        assert!(serialized.is_ok(), "Serialization must succeed");

        let json = serialized.unwrap();
        assert_eq!(json["model"], "gpt-4o");
        // Messages are serialized as "input" array in Responses API format
        assert!(json["input"].is_array());
        assert_eq!(json["input"].as_array().unwrap().len(), 2);
        assert_eq!(json["temperature"], 0.7);
        assert_eq!(json["max_output_tokens"], 100);
        assert_eq!(json["stream"], false);
    }

    #[test]
    fn test_serialization_omits_none_fields() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        let json = serde_json::to_value(&out).unwrap();
        let obj = json.as_object().unwrap();

        // Should only contain model and input (messages serialized as "input" in Responses API)
        assert!(obj.contains_key("model"));
        assert!(obj.contains_key("input"));

        // These should not be present when None
        assert!(!obj.contains_key("temperature"));
        assert!(!obj.contains_key("top_p"));
        assert!(!obj.contains_key("max_output_tokens"));
        assert!(!obj.contains_key("stop"));
        assert!(!obj.contains_key("tools"));
        assert!(!obj.contains_key("tool_choice"));
        assert!(!obj.contains_key("response_format"));
        assert!(!obj.contains_key("stream"));
    }

    #[test]
    fn test_top_level_messages_structure() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Hello"),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        let json = serde_json::to_value(&out).unwrap();

        // In Responses API format, messages are serialized as "input" array
        assert!(json["input"].is_array());
        assert!(json.as_object().unwrap().contains_key("input"));
    }
}

// ============================================================================
// SECTION 8: Response Conversion Tests (Responses → Chat)
// ============================================================================

mod response_conversion {
    use super::*;
    use routiium::conversion::responses_json_to_chat_request;

    #[test]
    fn test_basic_responses_to_chat_conversion() {
        let responses_json = json!({
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello"}
            ],
            "max_output_tokens": 200,
            "temperature": 0.7
        });

        let chat_req = responses_json_to_chat_request(&responses_json);

        assert_eq!(chat_req.model, "gpt-4o-mini");
        assert_eq!(chat_req.messages.len(), 2);
        assert_eq!(chat_req.max_tokens, Some(200));
        assert_eq!(chat_req.temperature, Some(0.7));
    }

    #[test]
    fn test_max_output_tokens_to_max_tokens() {
        let responses_json = json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "Test"}],
            "max_output_tokens": 512
        });

        let chat_req = responses_json_to_chat_request(&responses_json);
        assert_eq!(
            chat_req.max_tokens,
            Some(512),
            "max_output_tokens must map back to max_tokens"
        );
    }

    #[test]
    fn test_all_role_mappings_responses_to_chat() {
        let responses_json = json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": "usr"},
                {"role": "assistant", "content": "ast"},
                {"role": "tool", "content": "tol", "name": "tool1", "tool_call_id": "call1"}
            ]
        });

        let chat_req = responses_json_to_chat_request(&responses_json);

        assert_eq!(chat_req.messages.len(), 4);
        assert!(matches!(chat_req.messages[0].role, Role::System));
        assert!(matches!(chat_req.messages[1].role, Role::User));
        assert!(matches!(chat_req.messages[2].role, Role::Assistant));
        assert!(matches!(chat_req.messages[3].role, Role::Tool));
        assert_eq!(chat_req.messages[3].name, Some("tool1".into()));
        assert_eq!(chat_req.messages[3].tool_call_id, Some("call1".into()));
    }

    #[test]
    fn test_tools_conversion_responses_to_chat() {
        let responses_json = json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "Test"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "search",
                    "description": "Search the web",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": {"type": "string"}
                        },
                        "required": ["query"]
                    }
                }
            }],
            "tool_choice": {"type": "function", "function": {"name": "search"}}
        });

        let chat_req = responses_json_to_chat_request(&responses_json);

        assert!(chat_req.tools.is_some());
        let tools = chat_req.tools.unwrap();
        assert_eq!(tools.len(), 1);

        match &tools[0] {
            ToolDefinition::Function { function } => {
                assert_eq!(function.name, "search");
                assert_eq!(function.description, Some("Search the web".into()));
            }
        }

        assert_eq!(
            chat_req.tool_choice,
            Some(json!({"type": "function", "function": {"name": "search"}}))
        );
    }

    #[test]
    fn test_response_format_conversion_responses_to_chat() {
        let responses_json = json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "Test"}],
            "response_format": {
                "type": "json_object",
                "schema": {"type": "object", "properties": {}}
            }
        });

        let chat_req = responses_json_to_chat_request(&responses_json);

        assert!(chat_req.response_format.is_some());
        let rf = chat_req.response_format.unwrap();
        assert_eq!(rf.kind, "json_object");
        assert!(rf.extra.contains_key("schema"));
    }

    #[test]
    fn test_fallback_to_input_messages() {
        let responses_json = json!({
            "model": "gpt-4o",
            "input": {
                "messages": [
                    {"role": "user", "content": "From input.messages"}
                ]
            }
        });

        let chat_req = responses_json_to_chat_request(&responses_json);
        assert_eq!(chat_req.messages.len(), 1);
        assert_eq!(chat_req.messages[0].content, json!("From input.messages"));
    }

    #[test]
    fn test_prefers_top_level_messages_over_input() {
        let responses_json = json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "Top level"}
            ],
            "input": {
                "messages": [
                    {"role": "user", "content": "Nested"}
                ]
            }
        });

        let chat_req = responses_json_to_chat_request(&responses_json);
        assert_eq!(chat_req.messages.len(), 1);
        assert_eq!(
            chat_req.messages[0].content,
            json!("Top level"),
            "Top-level messages should take precedence"
        );
    }

    #[test]
    fn test_all_sampling_parameters_responses_to_chat() {
        let responses_json = json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "Test"}],
            "temperature": 0.8,
            "top_p": 0.95,
            "presence_penalty": 0.5,
            "frequency_penalty": 0.3,
            "logit_bias": {"123": 10.0, "456": -10.0},
            "user": "test-user",
            "n": 2,
            "stop": ["END", "STOP"],
            "stream": true
        });

        let chat_req = responses_json_to_chat_request(&responses_json);

        assert_eq!(chat_req.temperature, Some(0.8));
        assert_eq!(chat_req.top_p, Some(0.95));
        assert_eq!(chat_req.presence_penalty, Some(0.5));
        assert_eq!(chat_req.frequency_penalty, Some(0.3));
        assert!(chat_req.logit_bias.is_some());
        assert_eq!(chat_req.user, Some("test-user".into()));
        assert_eq!(chat_req.n, Some(2));
        assert_eq!(chat_req.stop, Some(json!(["END", "STOP"])));
        assert_eq!(chat_req.stream, Some(true));
    }
}

// ============================================================================
// SECTION 9: Round-Trip Conversion Tests
// ============================================================================

mod round_trip {
    use super::*;
    use routiium::conversion::responses_json_to_chat_request;

    #[test]
    fn test_basic_round_trip() {
        let original = ChatCompletionRequest {
            model: "gpt-4o".into(),
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
            temperature: Some(0.7),
            top_p: Some(0.9),
            max_tokens: Some(100),
            max_completion_tokens: None,
            stop: Some(json!("\n")),
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: Some("test".into()),
            n: Some(1),
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: Some(false),
            extra_body: None,
        };

        // Convert to Responses
        let responses_req = to_responses_request(&original, None);

        // Serialize and deserialize
        let responses_json = serde_json::to_value(&responses_req).unwrap();

        // Convert back to Chat
        let reconstructed = responses_json_to_chat_request(&responses_json);

        // Verify key fields match
        assert_eq!(reconstructed.model, original.model);
        assert_eq!(reconstructed.messages.len(), original.messages.len());
        assert_eq!(reconstructed.temperature, original.temperature);
        assert_eq!(reconstructed.top_p, original.top_p);
        assert_eq!(reconstructed.max_tokens, original.max_tokens);
        assert_eq!(reconstructed.stop, original.stop);
        assert_eq!(reconstructed.user, original.user);
        assert_eq!(reconstructed.n, original.n);
        assert_eq!(reconstructed.stream, original.stream);
    }

    #[test]
    fn test_round_trip_with_tools() {
        let original = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Use tool"),
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
                    description: Some("Lookup data".into()),
                    parameters: json!({"type": "object", "properties": {}}),
                },
            }]),
            tool_choice: Some(json!("auto")),
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let responses_req = to_responses_request(&original, None);

        // Verify tools are present in the Responses format
        assert!(responses_req.tools.is_some());
        assert_eq!(responses_req.tools.as_ref().unwrap().len(), 1);
        assert_eq!(responses_req.tool_choice, Some(json!("auto")));

        // Verify serialization includes tools
        let responses_json = serde_json::to_value(&responses_req).unwrap();
        assert!(responses_json["tools"].is_array());
        assert_eq!(responses_json["tools"].as_array().unwrap().len(), 1);
        assert_eq!(responses_json["tool_choice"], "auto");

        // Verify basic round-trip structure
        let reconstructed = responses_json_to_chat_request(&responses_json);
        assert_eq!(reconstructed.model, original.model);
        assert_eq!(reconstructed.messages.len(), 1);
    }

    #[test]
    fn test_round_trip_multimodal_content() {
        let content = json!([
            {"type": "text", "text": "Describe"},
            {"type": "image_url", "image_url": {"url": "https://example.com/img.jpg"}}
        ]);

        let original = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: content.clone(),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let responses_req = to_responses_request(&original, None);

        // Verify content was converted to Responses API format
        let msg_content = &responses_req.messages[0].content;
        let arr = msg_content.as_array().unwrap();
        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[0]["text"], "Describe");
        assert_eq!(arr[1]["type"], "input_image");
        assert_eq!(arr[1]["image_url"], "https://example.com/img.jpg");

        // Serialize and convert back
        let responses_json = serde_json::to_value(&responses_req).unwrap();
        let reconstructed = responses_json_to_chat_request(&responses_json);

        // Verify basic structure preserved (content format may differ)
        assert_eq!(reconstructed.model, original.model);
        assert_eq!(reconstructed.messages.len(), 1);
        assert!(reconstructed.messages[0].content.is_array());
    }
}

// ============================================================================
// SECTION 10: Reasoning Model Tests (for o1, o3, GPT-5, etc.)
// ============================================================================

mod reasoning_models {
    use super::*;

    #[test]
    fn test_reasoning_model_identification() {
        let reasoning_models = vec!["o1-preview", "o1-mini", "o3-mini", "gpt-5", "gpt-5-turbo"];

        for model_name in reasoning_models {
            let req = ChatCompletionRequest {
                model: model_name.into(),
                messages: vec![ChatMessage {
                    role: Role::User,
                    content: json!("Solve this complex problem"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                }],
                temperature: None,
                top_p: None,
                max_tokens: Some(2000),
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
                extra_body: None,
            };

            let out = to_responses_request(&req, None);

            // For reasoning models, max_output_tokens is critical
            assert_eq!(
                out.max_output_tokens,
                Some(2000),
                "Reasoning models must have max_output_tokens set correctly"
            );
        }
    }

    #[test]
    fn test_reasoning_model_supports_all_parameters() {
        let req = ChatCompletionRequest {
            model: "o1-preview".into(),
            messages: vec![
                ChatMessage {
                    role: Role::System,
                    content: json!("Think step by step."),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                ChatMessage {
                    role: Role::User,
                    content: json!("Complex reasoning task"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
            ],
            temperature: Some(1.0),
            top_p: Some(1.0),
            max_tokens: Some(4096),
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: Some(1),
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: Some(false),
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        // All parameters should be preserved for reasoning models
        assert_eq!(out.model, "o1-preview");
        assert_eq!(out.max_output_tokens, Some(4096));
        assert_eq!(out.temperature, Some(1.0));
        assert_eq!(out.stream, Some(false));
    }
}

// ============================================================================
// SECTION 11: Specification Compliance Tests
// ============================================================================

mod specification_compliance {
    use super::*;

    /// Test that messages array is preserved in order
    #[test]
    fn test_message_order_preservation() {
        let messages = [
            ("system", "You are helpful"),
            ("user", "First question"),
            ("assistant", "First answer"),
            ("user", "Second question"),
            ("assistant", "Second answer"),
            ("user", "Third question"),
        ];

        let chat_messages: Vec<ChatMessage> = messages
            .iter()
            .map(|(role_str, content)| {
                let role = match *role_str {
                    "system" => Role::System,
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    _ => Role::User,
                };
                ChatMessage {
                    role,
                    content: json!(content),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                }
            })
            .collect();

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: chat_messages,
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
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        assert_eq!(out.messages.len(), messages.len());
        for (i, (expected_role, expected_content)) in messages.iter().enumerate() {
            assert_eq!(out.messages[i].role, *expected_role);
            assert_eq!(out.messages[i].content, json!(expected_content));
        }
    }

    /// Test that no fields are dropped during conversion
    #[test]
    fn test_no_field_loss() {
        let mut logit_bias = HashMap::new();
        logit_bias.insert("100".into(), 5.0);

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Test"),
                name: Some("user1".into()),
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: Some(0.5),
            top_p: Some(0.8),
            max_tokens: Some(256),
            max_completion_tokens: None,
            stop: Some(json!(["STOP"])),
            presence_penalty: Some(0.1),
            frequency_penalty: Some(0.2),
            logit_bias: Some(logit_bias.clone()),
            user: Some("test-user".into()),
            n: Some(1),
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: Some(true),
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        // Verify every field is preserved
        assert_eq!(out.model, "gpt-4o");
        assert_eq!(out.messages.len(), 1);
        assert_eq!(out.messages[0].name, Some("user1".into()));
        assert_eq!(out.temperature, Some(0.5));
        assert_eq!(out.top_p, Some(0.8));
        assert_eq!(out.max_output_tokens, Some(256));
        assert_eq!(out.stop, Some(json!(["STOP"])));
        assert_eq!(out.presence_penalty, Some(0.1));
        assert_eq!(out.frequency_penalty, Some(0.2));
        assert_eq!(out.logit_bias, Some(logit_bias));
        assert_eq!(out.user, Some("test-user".into()));
        assert_eq!(out.n, Some(1));
        assert_eq!(out.stream, Some(true));
    }

    /// Test JSON Schema validation for structured outputs
    #[test]
    fn test_json_schema_structured_output() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 0},
                "email": {"type": "string", "format": "email"},
                "address": {
                    "type": "object",
                    "properties": {
                        "street": {"type": "string"},
                        "city": {"type": "string"}
                    },
                    "required": ["city"]
                }
            },
            "required": ["name", "email"]
        });

        let mut extra = HashMap::new();
        extra.insert("schema".into(), schema.clone());

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Extract person data"),
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
            tools: None,
            tool_choice: None,
            response_format: Some(ResponseFormat {
                kind: "json_schema".into(),
                extra,
            }),
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        let rf = out.response_format.unwrap();

        assert_eq!(rf["type"], "json_schema");
        assert_eq!(rf["schema"], schema);
    }

    /// Test that conversation parameter enables stateful mode
    #[test]
    fn test_conversation_stateful_mode() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Continue our conversation"),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, Some("conv-abc-123".into()));

        assert_eq!(out.conversation, Some("conv-abc-123".into()));
    }
}

// ============================================================================
// SECTION 12: Vision and Multimodal Comprehensive Tests
// ============================================================================

mod vision_comprehensive {
    use super::*;

    #[test]
    fn test_vision_with_multiple_tools() {
        // Matches Python test: test_responses_endpoint_vision_with_tools
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!([
                    {"type": "text", "text": "Analyze this chart and extract data"},
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
            tools: Some(vec![
                ToolDefinition::Function {
                    function: FunctionDef {
                        name: "extract_data".into(),
                        description: Some("Extract data from image".into()),
                        parameters: json!({
                            "type": "object",
                            "properties": {
                                "data_type": {"type": "string"}
                            }
                        }),
                    },
                },
                ToolDefinition::Function {
                    function: FunctionDef {
                        name: "analyze_trend".into(),
                        description: Some("Analyze data trend".into()),
                        parameters: json!({"type": "object", "properties": {}}),
                    },
                },
            ]),
            tool_choice: Some(json!("auto")),
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        // Verify vision content converted correctly
        let arr = out.messages[0].content.as_array().unwrap();
        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[1]["type"], "input_image");
        assert_eq!(arr[1]["image_url"], "https://example.com/chart.png");
        assert_eq!(arr[1]["detail"], "high");

        // Verify both tools present
        assert!(out.tools.is_some());
        let tools = out.tools.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(out.tool_choice, Some(json!("auto")));
    }

    #[test]
    fn test_vision_streaming_request() {
        // Matches Python test: test_responses_endpoint_vision_streaming
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!([
                    {"type": "text", "text": "Describe this image in detail"},
                    {"type": "image_url", "image_url": {"url": "https://example.com/photo.jpg"}}
                ]),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: Some(300),
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
            stream: Some(true), // Enable streaming
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        // Verify streaming enabled
        assert_eq!(out.stream, Some(true));

        // Verify vision content
        let arr = out.messages[0].content.as_array().unwrap();
        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[1]["type"], "input_image");
    }

    #[test]
    fn test_vision_with_reasoning_model() {
        // Test vision with reasoning models (gpt-5-nano, o1, etc.)
        let req = ChatCompletionRequest {
            model: "gpt-5-nano".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!([
                    {"type": "text", "text": "Think step by step about this problem"},
                    {"type": "image_url", "image_url": {"url": "https://example.com/math-problem.jpg"}}
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
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        assert_eq!(out.model, "gpt-5-nano");
        assert_eq!(out.max_output_tokens, Some(4096));

        // Verify multimodal content converted
        let arr = out.messages[0].content.as_array().unwrap();
        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[1]["type"], "input_image");
    }

    #[test]
    fn test_multiple_images_in_single_message() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!([
                    {"type": "text", "text": "Compare these images"},
                    {"type": "image_url", "image_url": {"url": "https://example.com/img1.jpg", "detail": "low"}},
                    {"type": "image_url", "image_url": {"url": "https://example.com/img2.jpg", "detail": "high"}},
                    {"type": "text", "text": "What are the key differences?"}
                ]),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);
        let arr = out.messages[0].content.as_array().unwrap();

        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0]["type"], "input_text");
        assert_eq!(arr[1]["type"], "input_image");
        assert_eq!(arr[1]["detail"], "low");
        assert_eq!(arr[2]["type"], "input_image");
        assert_eq!(arr[2]["detail"], "high");
        assert_eq!(arr[3]["type"], "input_text");
    }

    #[test]
    fn test_vision_base64_with_tools() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!([
                    {"type": "text", "text": "Extract text from this image"},
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
                        }
                    }
                ]),
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
                    name: "ocr_extract".into(),
                    description: Some("Extract text via OCR".into()),
                    parameters: json!({"type": "object"}),
                },
            }]),
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        // Verify base64 image preserved
        let arr = out.messages[0].content.as_array().unwrap();
        assert_eq!(arr[1]["type"], "input_image");
        assert!(arr[1]["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));

        // Verify tool present
        assert!(out.tools.is_some());
    }
}

// ============================================================================
// SECTION 13: Complex Real-World Scenarios
// ============================================================================

mod real_world_scenarios {
    use super::*;

    /// Test a complete function calling flow
    #[test]
    fn test_complete_function_calling_flow() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![
                ChatMessage {
                    role: Role::System,
                    content: json!("You are a helpful assistant with access to tools."),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                ChatMessage {
                    role: Role::User,
                    content: json!("What's the weather in San Francisco?"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                ChatMessage {
                    role: Role::Assistant,
                    content: json!(null),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                ChatMessage {
                    role: Role::Tool,
                    content: json!("{\"temperature\": 65, \"conditions\": \"sunny\"}"),
                    name: Some("get_weather".into()),
                    tool_call_id: Some("call_abc123".into()),
                    tool_calls: None,
                },
            ],
            temperature: Some(0.7),
            top_p: None,
            max_tokens: Some(150),
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: Some(vec![ToolDefinition::Function {
                function: FunctionDef {
                    name: "get_weather".into(),
                    description: Some("Get current weather for a location".into()),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "location": {"type": "string"}
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

        let out = to_responses_request(&req, None);

        assert_eq!(out.messages.len(), 4);
        assert_eq!(out.messages[3].role, "tool");
        assert_eq!(out.messages[3].name, Some("get_weather".into()));
        assert_eq!(out.messages[3].tool_call_id, Some("call_abc123".into()));
        assert!(out.tools.is_some());
        assert_eq!(out.tool_choice, Some(json!("auto")));
    }

    /// Test multimodal input with function calling
    #[test]
    fn test_multimodal_with_tools() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!([
                    {"type": "text", "text": "Analyze this chart and tell me the trend"},
                    {"type": "image_url", "image_url": {"url": "https://example.com/chart.png"}}
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
            tools: Some(vec![ToolDefinition::Function {
                function: FunctionDef {
                    name: "analyze_trend".into(),
                    description: Some("Analyze data trend".into()),
                    parameters: json!({"type": "object", "properties": {}}),
                },
            }]),
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        assert!(out.messages[0].content.is_array());
        assert!(out.tools.is_some());
    }

    /// Test structured output with tools
    #[test]
    fn test_structured_output_with_tools() {
        let mut extra = HashMap::new();
        extra.insert(
            "schema".into(),
            json!({
                "type": "object",
                "properties": {
                    "summary": {"type": "string"},
                    "key_points": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["summary"]
            }),
        );

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("Summarize this document"),
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
                    name: "search_docs".into(),
                    description: None,
                    parameters: json!({"type": "object"}),
                },
            }]),
            tool_choice: None,
            response_format: Some(ResponseFormat {
                kind: "json_object".into(),
                extra,
            }),
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        assert!(out.tools.is_some());
        assert!(out.response_format.is_some());
        let rf = out.response_format.unwrap();
        assert_eq!(rf["type"], "json_object");
        assert!(rf["schema"].is_object());
    }

    /// Test long conversation with alternating roles
    #[test]
    fn test_long_conversation() {
        let mut messages = vec![ChatMessage {
            role: Role::System,
            content: json!("You are a coding assistant."),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }];

        // Add 20 user/assistant exchanges
        for i in 0..20 {
            messages.push(ChatMessage {
                role: Role::User,
                content: json!(format!("User message {}", i)),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            });
            messages.push(ChatMessage {
                role: Role::Assistant,
                content: json!(format!("Assistant response {}", i)),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            });
        }

        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages,
            temperature: Some(0.7),
            top_p: None,
            max_tokens: Some(2000),
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: Some("developer-123".into()),
            n: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
            extra_body: None,
        };

        let out = to_responses_request(&req, None);

        // 1 system + 40 user/assistant messages
        assert_eq!(out.messages.len(), 41);
        assert_eq!(out.messages[0].role, "system");

        // Verify alternating pattern
        for i in 1..=20 {
            assert_eq!(out.messages[i * 2 - 1].role, "user");
            assert_eq!(out.messages[i * 2].role, "assistant");
        }
    }
}
