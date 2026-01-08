//! Comprehensive integration tests for chat history system
//!
//! Tests all aspects of chat history including:
//! - Core data models
//! - Multiple storage backends
//! - Composite store
//! - Manager integration
//! - Routing metadata capture
//! - Privacy levels
//! - Query filters

use routiium::chat_history::{
    ChatHistoryStore, Conversation, ConversationFilters, CostInfo as ChatCostInfo, MCPInfo,
    Message, MessageFilters, MessageRole, PrivacyLevel, RoutingInfo, TokenInfo,
};
use routiium::chat_history_jsonl::JsonlChatHistoryStore;
use routiium::chat_history_manager::{ChatHistoryConfig, ChatHistoryManager};
use routiium::chat_history_memory::MemoryChatHistoryStore;
use routiium::CompositeStore;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test]
async fn test_conversation_lifecycle() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    let mut conv = Conversation::new("conv_test_001".to_string());
    conv.title = Some("Test Conversation".to_string());
    conv.metadata
        .insert("user_id".to_string(), json!("user_123"));

    store.record_conversation(&conv).await.unwrap();

    let retrieved = store.get_conversation("conv_test_001").await.unwrap();
    assert_eq!(retrieved.conversation_id, "conv_test_001");
    assert_eq!(retrieved.title, Some("Test Conversation".to_string()));
    assert_eq!(retrieved.metadata.get("user_id"), Some(&json!("user_123")));

    // Touch and verify update
    std::thread::sleep(std::time::Duration::from_millis(10));
    let mut updated = retrieved.clone();
    updated.touch();
    store.record_conversation(&updated).await.unwrap();

    let after_touch = store.get_conversation("conv_test_001").await.unwrap();
    assert!(after_touch.last_seen_at >= conv.last_seen_at);
}

#[tokio::test]
async fn test_message_with_full_routing_info() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    let routing = RoutingInfo {
        requested_model: Some("gpt-4".to_string()),
        actual_model: Some("claude-3-opus-20240229".to_string()),
        backend: Some("anthropic".to_string()),
        backend_url: Some("https://api.anthropic.com/v1".to_string()),
        upstream_mode: Some("chat".to_string()),
        route_id: Some("rule_anthropic_fallback".to_string()),
        transformations_applied: Some(json!({
            "rewrite_model": "claude-3-opus-20240229",
            "override_temperature": 0.7
        })),
    };

    let mcp = MCPInfo {
        mcp_enabled: true,
        mcp_servers: vec!["github".to_string(), "filesystem".to_string()],
        system_prompt_applied: true,
    };

    let tokens = TokenInfo {
        input_tokens: Some(150),
        output_tokens: Some(300),
        cached_tokens: Some(50),
        reasoning_tokens: Some(100),
    };

    let cost = ChatCostInfo {
        input_cost: 0.015,
        output_cost: 0.045,
        cached_cost: Some(0.005),
        total_cost: 0.065,
        currency: "USD".to_string(),
    };

    let msg = Message::new(
        "conv_test_001".to_string(),
        MessageRole::Assistant,
        json!({"text": "Hello, how can I help?"}),
        PrivacyLevel::Full,
    )
    .with_request_id("req_abc123".to_string())
    .with_routing(routing)
    .with_mcp(mcp)
    .with_tokens(tokens)
    .with_cost(cost);

    store.record_message(&msg).await.unwrap();

    let filters = MessageFilters {
        conversation_id: Some("conv_test_001".to_string()),
        ..Default::default()
    };

    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 1);

    let retrieved = &messages[0];
    assert_eq!(retrieved.request_id, Some("req_abc123".to_string()));
    assert_eq!(retrieved.routing.requested_model, Some("gpt-4".to_string()));
    assert_eq!(
        retrieved.routing.actual_model,
        Some("claude-3-opus-20240229".to_string())
    );
    assert_eq!(retrieved.routing.backend, Some("anthropic".to_string()));
    assert_eq!(
        retrieved.routing.route_id,
        Some("rule_anthropic_fallback".to_string())
    );
    assert!(retrieved.routing.transformations_applied.is_some());
    assert!(retrieved.mcp.mcp_enabled);
    assert_eq!(retrieved.mcp.mcp_servers.len(), 2);
    assert!(retrieved.mcp.system_prompt_applied);
    assert_eq!(retrieved.tokens.input_tokens, Some(150));
    assert_eq!(retrieved.tokens.output_tokens, Some(300));
    assert_eq!(retrieved.tokens.cached_tokens, Some(50));
    assert_eq!(retrieved.tokens.reasoning_tokens, Some(100));
    assert_eq!(retrieved.tokens.total_tokens(), 450);
    assert!(retrieved.cost_info.is_some());
    assert_eq!(retrieved.cost_info.as_ref().unwrap().total_cost, 0.065);
}

#[tokio::test]
async fn test_privacy_levels() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    let content = json!({"text": "This is a secret message with sensitive data"});

    // Privacy: Off
    let msg_off = Message::new(
        "conv_001".to_string(),
        MessageRole::User,
        content.clone(),
        PrivacyLevel::Off,
    );
    assert_eq!(msg_off.content, json!({"redacted": true}));
    assert!(msg_off.content_hash.is_some());

    // Privacy: Summary
    let msg_summary = Message::new(
        "conv_001".to_string(),
        MessageRole::User,
        content.clone(),
        PrivacyLevel::Summary,
    );
    assert!(msg_summary.content.get("summary").is_some());
    assert!(msg_summary.content_hash.is_some());

    // Privacy: Full
    let msg_full = Message::new(
        "conv_001".to_string(),
        MessageRole::User,
        content.clone(),
        PrivacyLevel::Full,
    );
    assert_eq!(msg_full.content, content);
    assert!(msg_full.content_hash.is_some());
}

#[tokio::test]
async fn test_routing_filter_by_backend() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    // OpenAI message
    let msg1 = Message::new(
        "conv_001".to_string(),
        MessageRole::User,
        json!({"text": "Hello"}),
        PrivacyLevel::Full,
    )
    .with_routing(RoutingInfo {
        backend: Some("openai".to_string()),
        requested_model: Some("gpt-4".to_string()),
        actual_model: Some("gpt-4-0613".to_string()),
        ..Default::default()
    });

    // Anthropic message
    let msg2 = Message::new(
        "conv_001".to_string(),
        MessageRole::Assistant,
        json!({"text": "Hi"}),
        PrivacyLevel::Full,
    )
    .with_routing(RoutingInfo {
        backend: Some("anthropic".to_string()),
        requested_model: Some("gpt-4".to_string()),
        actual_model: Some("claude-3-opus".to_string()),
        ..Default::default()
    });

    // Bedrock message
    let msg3 = Message::new(
        "conv_001".to_string(),
        MessageRole::Assistant,
        json!({"text": "Hello there"}),
        PrivacyLevel::Full,
    )
    .with_routing(RoutingInfo {
        backend: Some("bedrock".to_string()),
        requested_model: Some("gpt-4".to_string()),
        actual_model: Some("anthropic.claude-3-sonnet".to_string()),
        ..Default::default()
    });

    store.record_messages(&[msg1, msg2, msg3]).await.unwrap();

    // Filter by OpenAI
    let filters = MessageFilters {
        backend: Some("openai".to_string()),
        ..Default::default()
    };
    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].routing.backend, Some("openai".to_string()));

    // Filter by Anthropic
    let filters = MessageFilters {
        backend: Some("anthropic".to_string()),
        ..Default::default()
    };
    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].routing.backend, Some("anthropic".to_string()));
}

#[tokio::test]
async fn test_model_aliasing_queries() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    // Client requested gpt-4, got routed to claude
    let msg1 = Message::new(
        "conv_001".to_string(),
        MessageRole::Assistant,
        json!({"text": "Response 1"}),
        PrivacyLevel::Full,
    )
    .with_routing(RoutingInfo {
        requested_model: Some("gpt-4".to_string()),
        actual_model: Some("claude-3-opus".to_string()),
        backend: Some("anthropic".to_string()),
        ..Default::default()
    });

    // Client requested gpt-4, got actual gpt-4
    let msg2 = Message::new(
        "conv_001".to_string(),
        MessageRole::Assistant,
        json!({"text": "Response 2"}),
        PrivacyLevel::Full,
    )
    .with_routing(RoutingInfo {
        requested_model: Some("gpt-4".to_string()),
        actual_model: Some("gpt-4-0613".to_string()),
        backend: Some("openai".to_string()),
        ..Default::default()
    });

    store.record_messages(&[msg1, msg2]).await.unwrap();

    // Find all gpt-4 requests
    let filters = MessageFilters {
        requested_model: Some("gpt-4".to_string()),
        ..Default::default()
    };
    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 2);

    // Find actual claude usage
    let filters = MessageFilters {
        actual_model: Some("claude-3-opus".to_string()),
        ..Default::default()
    };
    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].routing.requested_model,
        Some("gpt-4".to_string())
    );
}

#[tokio::test]
async fn test_mcp_tracking() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    // Message with MCP
    let msg1 = Message::new(
        "conv_001".to_string(),
        MessageRole::User,
        json!({"text": "Search GitHub"}),
        PrivacyLevel::Full,
    )
    .with_mcp(MCPInfo {
        mcp_enabled: true,
        mcp_servers: vec!["github".to_string(), "filesystem".to_string()],
        system_prompt_applied: true,
    });

    // Message without MCP
    let msg2 = Message::new(
        "conv_001".to_string(),
        MessageRole::Assistant,
        json!({"text": "Regular response"}),
        PrivacyLevel::Full,
    )
    .with_mcp(MCPInfo {
        mcp_enabled: false,
        mcp_servers: vec![],
        system_prompt_applied: false,
    });

    store.record_messages(&[msg1, msg2]).await.unwrap();

    // Filter by MCP enabled
    let filters = MessageFilters {
        mcp_enabled: Some(true),
        ..Default::default()
    };
    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].mcp.mcp_enabled);
    assert_eq!(messages[0].mcp.mcp_servers.len(), 2);

    // Filter by MCP disabled
    let filters = MessageFilters {
        mcp_enabled: Some(false),
        ..Default::default()
    };
    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert!(!messages[0].mcp.mcp_enabled);
}

#[tokio::test]
async fn test_upstream_mode_filter() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    let msg_chat = Message::new(
        "conv_001".to_string(),
        MessageRole::User,
        json!({"text": "Hello"}),
        PrivacyLevel::Full,
    )
    .with_routing(RoutingInfo {
        upstream_mode: Some("chat".to_string()),
        ..Default::default()
    });

    let msg_responses = Message::new(
        "conv_001".to_string(),
        MessageRole::Assistant,
        json!({"text": "Hi"}),
        PrivacyLevel::Full,
    )
    .with_routing(RoutingInfo {
        upstream_mode: Some("responses".to_string()),
        ..Default::default()
    });

    store
        .record_messages(&[msg_chat, msg_responses])
        .await
        .unwrap();

    // Filter by chat mode
    let filters = MessageFilters {
        upstream_mode: Some("chat".to_string()),
        ..Default::default()
    };
    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].routing.upstream_mode, Some("chat".to_string()));

    // Filter by responses mode
    let filters = MessageFilters {
        upstream_mode: Some("responses".to_string()),
        ..Default::default()
    };
    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].routing.upstream_mode,
        Some("responses".to_string())
    );
}

#[tokio::test]
async fn test_route_id_filter() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    let msg1 = Message::new(
        "conv_001".to_string(),
        MessageRole::User,
        json!({"text": "Hello"}),
        PrivacyLevel::Full,
    )
    .with_routing(RoutingInfo {
        route_id: Some("rule_001".to_string()),
        ..Default::default()
    });

    let msg2 = Message::new(
        "conv_001".to_string(),
        MessageRole::Assistant,
        json!({"text": "Hi"}),
        PrivacyLevel::Full,
    )
    .with_routing(RoutingInfo {
        route_id: Some("rule_002".to_string()),
        ..Default::default()
    });

    store.record_messages(&[msg1, msg2]).await.unwrap();

    let filters = MessageFilters {
        route_id: Some("rule_001".to_string()),
        ..Default::default()
    };
    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].routing.route_id, Some("rule_001".to_string()));
}

#[tokio::test]
async fn test_time_range_filters() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    let now = current_timestamp();

    let msg1 = Message::new(
        "conv_001".to_string(),
        MessageRole::User,
        json!({"text": "First"}),
        PrivacyLevel::Full,
    );

    store.record_message(&msg1).await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let msg2 = Message::new(
        "conv_001".to_string(),
        MessageRole::Assistant,
        json!({"text": "Second"}),
        PrivacyLevel::Full,
    );

    store.record_message(&msg2).await.unwrap();

    // Get all messages
    let filters = MessageFilters {
        conversation_id: Some("conv_001".to_string()),
        ..Default::default()
    };
    let all_messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(all_messages.len(), 2);

    // Get messages after first one
    let filters = MessageFilters {
        conversation_id: Some("conv_001".to_string()),
        start_time: Some(now + 1),
        ..Default::default()
    };
    let later_messages = store.list_messages(&filters).await.unwrap();
    assert!(later_messages.len() <= 1);
}

#[tokio::test]
async fn test_jsonl_backend_persistence() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path().to_path_buf();

    // Write data
    {
        let store = JsonlChatHistoryStore::new(&path);
        store.init().await.unwrap();

        let conv = Conversation::new("conv_persist".to_string());
        store.record_conversation(&conv).await.unwrap();

        let msg = Message::new(
            "conv_persist".to_string(),
            MessageRole::User,
            json!({"text": "Persistent message"}),
            PrivacyLevel::Full,
        )
        .with_routing(RoutingInfo {
            backend: Some("openai".to_string()),
            ..Default::default()
        });

        store.record_message(&msg).await.unwrap();
    }

    // Read from new instance
    {
        let store = JsonlChatHistoryStore::new(&path);

        let conv = store.get_conversation("conv_persist").await.unwrap();
        assert_eq!(conv.conversation_id, "conv_persist");

        let filters = MessageFilters {
            conversation_id: Some("conv_persist".to_string()),
            ..Default::default()
        };
        let messages = store.list_messages(&filters).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].routing.backend, Some("openai".to_string()));
    }
}

#[tokio::test]
async fn test_composite_store() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path().to_path_buf();

    let primary = Box::new(MemoryChatHistoryStore::new());
    let sink = Box::new(JsonlChatHistoryStore::new(&path));

    let mut composite = CompositeStore::new(primary, false);
    composite.add_sink(sink);

    composite.init().await.unwrap();

    let conv = Conversation::new("conv_composite".to_string());
    composite.record_conversation(&conv).await.unwrap();

    // Should be in primary
    let from_primary = composite.get_conversation("conv_composite").await.unwrap();
    assert_eq!(from_primary.conversation_id, "conv_composite");

    // Should also be in sink
    let jsonl_store = JsonlChatHistoryStore::new(&path);
    let from_sink = jsonl_store
        .get_conversation("conv_composite")
        .await
        .unwrap();
    assert_eq!(from_sink.conversation_id, "conv_composite");
}

#[tokio::test]
async fn test_manager_integration() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path().to_string_lossy().to_string();

    let config = ChatHistoryConfig {
        enabled: true,
        primary_backend: "memory".to_string(),
        sink_backends: vec!["jsonl".to_string()],
        privacy_level: PrivacyLevel::Full,
        ttl_seconds: 2592000,
        strict: false,
        jsonl_path: Some(path.clone()),
        memory_max_messages: Some(1000),
        sqlite_url: None,
        postgres_url: None,
        turso_url: None,
        turso_auth_token: None,
    };

    let manager = ChatHistoryManager::new(config).await.unwrap();
    assert!(manager.is_enabled());
    assert_eq!(manager.privacy_level(), PrivacyLevel::Full);

    let conv = Conversation::new("conv_manager".to_string());
    manager.record_conversation(&conv).await.unwrap();

    let msg = Message::new(
        "conv_manager".to_string(),
        MessageRole::User,
        json!({"text": "Test message"}),
        PrivacyLevel::Full,
    )
    .with_routing(RoutingInfo {
        requested_model: Some("gpt-4".to_string()),
        actual_model: Some("gpt-4-0613".to_string()),
        backend: Some("openai".to_string()),
        ..Default::default()
    });

    manager.record_message(&msg).await.unwrap();

    let stats = manager.stats().await.unwrap();
    assert_eq!(stats.total_conversations, 1);
    assert_eq!(stats.total_messages, 1);

    let filters = MessageFilters {
        conversation_id: Some("conv_manager".to_string()),
        ..Default::default()
    };
    let messages = manager.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 1);
}

#[tokio::test]
async fn test_conversation_list_with_filters() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    let now = current_timestamp();

    let conv1 = Conversation::new("conv_001".to_string());
    let conv2 = Conversation::new("conv_002".to_string());
    let conv3 = Conversation::new("conv_003".to_string());

    store.record_conversation(&conv1).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    store.record_conversation(&conv2).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    store.record_conversation(&conv3).await.unwrap();

    // List all
    let filters = ConversationFilters::default();
    let convs = store.list_conversations(&filters).await.unwrap();
    assert_eq!(convs.len(), 3);

    // List with limit
    let filters = ConversationFilters {
        limit: Some(2),
        ..Default::default()
    };
    let convs = store.list_conversations(&filters).await.unwrap();
    assert_eq!(convs.len(), 2);

    // List with time filter
    let filters = ConversationFilters {
        start_time: Some(now),
        ..Default::default()
    };
    let convs = store.list_conversations(&filters).await.unwrap();
    assert!(convs.len() >= 1);
}

#[tokio::test]
async fn test_delete_conversation() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    let conv = Conversation::new("conv_delete".to_string());
    store.record_conversation(&conv).await.unwrap();

    let msg1 = Message::new(
        "conv_delete".to_string(),
        MessageRole::User,
        json!({"text": "Message 1"}),
        PrivacyLevel::Full,
    );
    let msg2 = Message::new(
        "conv_delete".to_string(),
        MessageRole::Assistant,
        json!({"text": "Message 2"}),
        PrivacyLevel::Full,
    );

    store.record_messages(&[msg1, msg2]).await.unwrap();

    // Verify data exists
    assert!(store.get_conversation("conv_delete").await.is_ok());

    let filters = MessageFilters {
        conversation_id: Some("conv_delete".to_string()),
        ..Default::default()
    };
    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 2);

    // Delete conversation
    store.delete_conversation("conv_delete").await.unwrap();

    // Verify deleted
    assert!(store.get_conversation("conv_delete").await.is_err());

    let messages = store.list_messages(&filters).await.unwrap();
    assert_eq!(messages.len(), 0);
}

#[tokio::test]
async fn test_clear_all_data() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    for i in 0..5 {
        let conv = Conversation::new(format!("conv_{}", i));
        store.record_conversation(&conv).await.unwrap();

        let msg = Message::new(
            format!("conv_{}", i),
            MessageRole::User,
            json!({"text": format!("Message {}", i)}),
            PrivacyLevel::Full,
        );
        store.record_message(&msg).await.unwrap();
    }

    let stats = store.stats().await.unwrap();
    assert_eq!(stats.total_conversations, 5);
    assert_eq!(stats.total_messages, 5);

    store.clear().await.unwrap();

    let stats = store.stats().await.unwrap();
    assert_eq!(stats.total_conversations, 0);
    assert_eq!(stats.total_messages, 0);
}

#[tokio::test]
async fn test_complex_routing_scenario() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    let conv = Conversation::new("conv_complex".to_string());
    store.record_conversation(&conv).await.unwrap();

    // User message - routed to OpenAI
    let msg1 = Message::new(
        "conv_complex".to_string(),
        MessageRole::User,
        json!({"text": "Translate this to French"}),
        PrivacyLevel::Full,
    )
    .with_request_id("req_001".to_string())
    .with_routing(RoutingInfo {
        requested_model: Some("gpt-4".to_string()),
        actual_model: Some("gpt-4-0613".to_string()),
        backend: Some("openai".to_string()),
        backend_url: Some("https://api.openai.com/v1".to_string()),
        upstream_mode: Some("chat".to_string()),
        route_id: Some("rule_openai_primary".to_string()),
        transformations_applied: None,
    })
    .with_tokens(TokenInfo {
        input_tokens: Some(20),
        output_tokens: None,
        cached_tokens: None,
        reasoning_tokens: None,
    });

    // Assistant response - came from OpenAI
    let msg2 = Message::new(
        "conv_complex".to_string(),
        MessageRole::Assistant,
        json!({"text": "Traduisez ceci en français"}),
        PrivacyLevel::Full,
    )
    .with_request_id("req_001".to_string())
    .with_routing(RoutingInfo {
        requested_model: Some("gpt-4".to_string()),
        actual_model: Some("gpt-4-0613".to_string()),
        backend: Some("openai".to_string()),
        backend_url: Some("https://api.openai.com/v1".to_string()),
        upstream_mode: Some("chat".to_string()),
        route_id: Some("rule_openai_primary".to_string()),
        transformations_applied: None,
    })
    .with_tokens(TokenInfo {
        input_tokens: Some(20),
        output_tokens: Some(15),
        cached_tokens: None,
        reasoning_tokens: None,
    })
    .with_cost(ChatCostInfo {
        input_cost: 0.0006,
        output_cost: 0.00045,
        cached_cost: None,
        total_cost: 0.00105,
        currency: "USD".to_string(),
    });

    // Next user message - with MCP
    let msg3 = Message::new(
        "conv_complex".to_string(),
        MessageRole::User,
        json!({"text": "Search GitHub for rust async examples"}),
        PrivacyLevel::Full,
    )
    .with_request_id("req_002".to_string())
    .with_routing(RoutingInfo {
        requested_model: Some("gpt-4".to_string()),
        actual_model: Some("claude-3-opus-20240229".to_string()),
        backend: Some("anthropic".to_string()),
        backend_url: Some("https://api.anthropic.com/v1".to_string()),
        upstream_mode: Some("chat".to_string()),
        route_id: Some("rule_anthropic_mcp".to_string()),
        transformations_applied: Some(json!({
            "rewrite_model": "claude-3-opus-20240229"
        })),
    })
    .with_mcp(MCPInfo {
        mcp_enabled: true,
        mcp_servers: vec!["github".to_string()],
        system_prompt_applied: true,
    })
    .with_tokens(TokenInfo {
        input_tokens: Some(50),
        output_tokens: None,
        cached_tokens: Some(20),
        reasoning_tokens: None,
    });

    store.record_messages(&[msg1, msg2, msg3]).await.unwrap();

    // Query: All messages in conversation
    let filters = MessageFilters {
        conversation_id: Some("conv_complex".to_string()),
        ..Default::default()
    };
    let all_msgs = store.list_messages(&filters).await.unwrap();
    assert_eq!(all_msgs.len(), 3);

    // Query: Messages by request_id
    let filters = MessageFilters {
        request_id: Some("req_001".to_string()),
        ..Default::default()
    };
    let req_msgs = store.list_messages(&filters).await.unwrap();
    assert_eq!(req_msgs.len(), 2);

    // Query: Messages with MCP
    let filters = MessageFilters {
        conversation_id: Some("conv_complex".to_string()),
        mcp_enabled: Some(true),
        ..Default::default()
    };
    let mcp_msgs = store.list_messages(&filters).await.unwrap();
    assert_eq!(mcp_msgs.len(), 1);
    assert_eq!(mcp_msgs[0].mcp.mcp_servers, vec!["github"]);

    // Query: Messages that were aliased to a different model family (not just versioned)
    let all_msgs = store
        .list_messages(&MessageFilters {
            conversation_id: Some("conv_complex".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    let aliased: Vec<_> = all_msgs
        .iter()
        .filter(|m| {
            // True aliasing: gpt-4 -> claude-3-opus (different model family)
            // Not aliasing: gpt-4 -> gpt-4-0613 (just version specification)
            m.routing.requested_model.as_ref().map(|r| r.as_str()) == Some("gpt-4")
                && m.routing
                    .actual_model
                    .as_ref()
                    .map(|a| a.contains("claude"))
                    .unwrap_or(false)
        })
        .collect();
    assert_eq!(aliased.len(), 1);
    assert_eq!(
        aliased[0].routing.requested_model,
        Some("gpt-4".to_string())
    );
    assert_eq!(
        aliased[0].routing.actual_model,
        Some("claude-3-opus-20240229".to_string())
    );
}

#[tokio::test]
async fn test_health_check() {
    let store = MemoryChatHistoryStore::new();
    assert!(store.health().await.unwrap());

    let temp_file = NamedTempFile::new().unwrap();
    let jsonl_store = JsonlChatHistoryStore::new(temp_file.path());
    jsonl_store.init().await.unwrap();
    assert!(jsonl_store.health().await.unwrap());
}

#[tokio::test]
async fn test_stats_accuracy() {
    let store = MemoryChatHistoryStore::new();
    store.init().await.unwrap();

    let stats = store.stats().await.unwrap();
    assert_eq!(stats.backend_type, "memory");
    assert_eq!(stats.total_conversations, 0);
    assert_eq!(stats.total_messages, 0);

    for i in 0..3 {
        let conv = Conversation::new(format!("conv_{}", i));
        store.record_conversation(&conv).await.unwrap();

        for j in 0..5 {
            let msg = Message::new(
                format!("conv_{}", i),
                MessageRole::User,
                json!({"text": format!("Message {}", j)}),
                PrivacyLevel::Full,
            );
            store.record_message(&msg).await.unwrap();
        }
    }

    let stats = store.stats().await.unwrap();
    assert_eq!(stats.total_conversations, 3);
    assert_eq!(stats.total_messages, 15);
}
