use routiium::conversion::{inject_system_prompt, inject_system_prompt_chat};
use routiium::models::chat::{ChatCompletionRequest, ChatMessage, Role};
use routiium::models::responses::ResponsesMessage;
use routiium::system_prompt_config::SystemPromptConfig;
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_system_prompt_config_empty() {
    let config = SystemPromptConfig::empty();
    assert!(config.global.is_none());
    assert_eq!(config.per_model.len(), 0);
    assert_eq!(config.per_api.len(), 0);
    assert_eq!(config.injection_mode, "prepend");
    assert!(config.enabled);
}

#[test]
fn test_system_prompt_config_priority() {
    let mut per_model = HashMap::new();
    per_model.insert("gpt-4".to_string(), "model-specific".to_string());

    let mut per_api = HashMap::new();
    per_api.insert("chat".to_string(), "api-specific".to_string());

    let config = SystemPromptConfig {
        global: Some("global".to_string()),
        per_model,
        per_api,
        injection_mode: "prepend".to_string(),
        enabled: true,
    };

    // Model-specific should have highest priority
    assert_eq!(
        config.get_prompt(Some("gpt-4"), Some("chat")),
        Some("model-specific".to_string())
    );

    // API-specific should be next
    assert_eq!(
        config.get_prompt(Some("other-model"), Some("chat")),
        Some("api-specific".to_string())
    );

    // Global should be last
    assert_eq!(
        config.get_prompt(Some("other-model"), Some("other-api")),
        Some("global".to_string())
    );

    // None when model and API don't match
    assert_eq!(
        config.get_prompt(Some("unknown"), Some("unknown")),
        Some("global".to_string())
    );
}

#[test]
fn test_system_prompt_config_disabled() {
    let config = SystemPromptConfig {
        global: Some("global".to_string()),
        per_model: HashMap::new(),
        per_api: HashMap::new(),
        injection_mode: "prepend".to_string(),
        enabled: false,
    };

    assert_eq!(config.get_prompt(Some("gpt-4"), Some("chat")), None);
}

#[test]
fn test_inject_system_prompt_prepend() {
    let mut messages = vec![
        ResponsesMessage {
            role: "user".to_string(),
            content: json!("Hello"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        },
        ResponsesMessage {
            role: "assistant".to_string(),
            content: json!("Hi there!"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        },
    ];

    inject_system_prompt(&mut messages, "You are helpful", "prepend");

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[0].content, json!("You are helpful"));
    assert_eq!(messages[1].role, "user");
}

#[test]
fn test_inject_system_prompt_append() {
    let mut messages = vec![
        ResponsesMessage {
            role: "system".to_string(),
            content: json!("Original system message"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        },
        ResponsesMessage {
            role: "user".to_string(),
            content: json!("Hello"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        },
    ];

    inject_system_prompt(&mut messages, "Additional instructions", "append");

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[0].content, json!("Original system message"));
    assert_eq!(messages[1].role, "system");
    assert_eq!(messages[1].content, json!("Additional instructions"));
    assert_eq!(messages[2].role, "user");
}

#[test]
fn test_inject_system_prompt_replace() {
    let mut messages = vec![
        ResponsesMessage {
            role: "system".to_string(),
            content: json!("Old system message"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        },
        ResponsesMessage {
            role: "user".to_string(),
            content: json!("Hello"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        },
        ResponsesMessage {
            role: "system".to_string(),
            content: json!("Another old system message"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        },
    ];

    inject_system_prompt(&mut messages, "New system message", "replace");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[0].content, json!("New system message"));
    assert_eq!(messages[1].role, "user");
}

#[test]
fn test_inject_system_prompt_chat_prepend() {
    let mut req = ChatCompletionRequest {
        model: "gpt-4".to_string(),
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

    inject_system_prompt_chat(&mut req, "You are helpful", "prepend");

    assert_eq!(req.messages.len(), 2);
    assert!(matches!(req.messages[0].role, Role::System));
    assert_eq!(req.messages[0].content, json!("You are helpful"));
    assert!(matches!(req.messages[1].role, Role::User));
}

#[test]
fn test_inject_system_prompt_chat_replace() {
    let mut req = ChatCompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![
            ChatMessage {
                role: Role::System,
                content: json!("Old system message"),
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
            ChatMessage {
                role: Role::System,
                content: json!("Another old"),
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

    inject_system_prompt_chat(&mut req, "New system message", "replace");

    assert_eq!(req.messages.len(), 2);
    assert!(matches!(req.messages[0].role, Role::System));
    assert_eq!(req.messages[0].content, json!("New system message"));
    assert!(matches!(req.messages[1].role, Role::User));
}

#[test]
fn test_inject_system_prompt_chat_append() {
    let mut req = ChatCompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![
            ChatMessage {
                role: Role::System,
                content: json!("Original system"),
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

    inject_system_prompt_chat(&mut req, "Additional system", "append");

    assert_eq!(req.messages.len(), 3);
    assert!(matches!(req.messages[0].role, Role::System));
    assert_eq!(req.messages[0].content, json!("Original system"));
    assert!(matches!(req.messages[1].role, Role::System));
    assert_eq!(req.messages[1].content, json!("Additional system"));
    assert!(matches!(req.messages[2].role, Role::User));
}
