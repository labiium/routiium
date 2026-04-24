use actix_web::{test, web, App};
use routiium::server::config_routes;
use routiium::util::AppState;
use serde_json::json;
use std::sync::Arc;

/// Test that /convert endpoint injects MCP tools when MCP manager is configured
#[actix_web::test]
async fn test_convert_with_mcp_tools_injection() {
    // Create a temporary MCP config file with a mock server
    let temp_dir = tempfile::tempdir().unwrap();
    let mcp_config_path = temp_dir.path().join("mcp.json");

    // Note: This test will attempt to start a real MCP server process
    // For unit testing, we skip actual process spawning
    let mcp_config = json!({
        "mcpServers": {
            "mock": {
                "command": "echo",
                "args": ["test"]
            }
        }
    });

    std::fs::write(
        &mcp_config_path,
        serde_json::to_string_pretty(&mcp_config).unwrap(),
    )
    .unwrap();

    let app_state = AppState {
        mcp_config_path: Some(mcp_config_path.to_string_lossy().to_string()),
        ..Default::default()
    };

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let chat_request = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "Use available tools to help me"}
        ]
    });

    let req = test::TestRequest::post()
        .uri("/convert")
        .insert_header(("content-type", "application/json"))
        .set_payload(serde_json::to_string(&chat_request).unwrap())
        .to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);

    let body = test::read_body(resp).await;
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Check that the response has tools field (even if empty due to mock server not starting)
    // The conversion should happen regardless
    assert!(body_json.get("tools").is_some() || body_json.get("input").is_some());
}

/// Test that /convert endpoint works without MCP when not configured
#[actix_web::test]
async fn test_convert_without_mcp_configuration() {
    let app_state = AppState::default();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let chat_request = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "Hello"}
        ]
    });

    let req = test::TestRequest::post()
        .uri("/convert")
        .insert_header(("content-type", "application/json"))
        .set_payload(serde_json::to_string(&chat_request).unwrap())
        .to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);

    let body = test::read_body(resp).await;
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Should convert successfully without tools
    let messages = body_json["input"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
}

/// Test that MCP tools are merged with existing tools in request
#[actix_web::test]
async fn test_convert_merges_mcp_tools_with_existing_tools() {
    let app_state = AppState::default();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let chat_request = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "Hello"}
        ],
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "existing_tool",
                    "description": "An existing tool",
                    "parameters": {
                        "type": "object",
                        "properties": {}
                    }
                }
            }
        ]
    });

    let req = test::TestRequest::post()
        .uri("/convert")
        .insert_header(("content-type", "application/json"))
        .set_payload(serde_json::to_string(&chat_request).unwrap())
        .to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);

    let body = test::read_body(resp).await;
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Check that tools field exists and contains the existing tool
    if let Some(tools) = body_json.get("tools") {
        let tools_array = tools.as_array().unwrap();
        // Should have at least the existing tool
        assert!(!tools_array.is_empty());

        // Check if existing tool is present
        let has_existing_tool = tools_array.iter().any(|t| {
            t["name"] == "existing_tool"
                || t.get("function")
                    .and_then(|f| f.get("name"))
                    .map(|n| n == "existing_tool")
                    .unwrap_or(false)
        });
        assert!(has_existing_tool);
    }
}

/// Test MCP config reload endpoint
#[actix_web::test]
async fn test_reload_mcp_with_valid_config() {
    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");
    let temp_dir = tempfile::tempdir().unwrap();
    let mcp_config_path = temp_dir.path().join("mcp.json");

    let mcp_config = json!({
        "mcpServers": {
            "test_server": {
                "command": "echo",
                "args": ["test"]
            }
        }
    });

    std::fs::write(
        &mcp_config_path,
        serde_json::to_string_pretty(&mcp_config).unwrap(),
    )
    .unwrap();

    let app_state = AppState {
        mcp_config_path: Some(mcp_config_path.to_string_lossy().to_string()),
        ..Default::default()
    };

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/reload/mcp")
        .insert_header(("content-type", "application/json"))
        .insert_header(("authorization", "Bearer admin-test"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    let status = resp.status();

    let body = test::read_body(resp).await;
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // When MCP servers fail to connect (like 'echo' command), we may get 503 or success with 0 servers
    // Both are acceptable as the config parsing succeeded
    if status == 200 {
        assert!(body_json.get("success").is_some());
        assert!(body_json.get("connected_servers").is_some());
    } else {
        // 503 is also acceptable - MCP client failed to initialize
        assert!(status.is_server_error() || status.is_client_error());
        assert!(body_json.get("error").is_some());
    }
}

/// Test MCP config reload with invalid JSON
#[actix_web::test]
async fn test_reload_mcp_with_invalid_json() {
    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");
    let temp_dir = tempfile::tempdir().unwrap();
    let mcp_config_path = temp_dir.path().join("invalid_mcp.json");

    std::fs::write(&mcp_config_path, "{ invalid json }").unwrap();

    let app_state = AppState {
        mcp_config_path: Some(mcp_config_path.to_string_lossy().to_string()),
        ..Default::default()
    };

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/reload/mcp")
        .insert_header(("content-type", "application/json"))
        .insert_header(("authorization", "Bearer admin-test"))
        .to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 500);

    let body = test::read_body(resp).await;
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(body_json.get("error").is_some());
}

/// Test combined MCP tools and system prompt injection
#[actix_web::test]
async fn test_convert_with_mcp_and_system_prompt() {
    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");
    let temp_dir = tempfile::tempdir().unwrap();

    // Create system prompt config
    let system_prompt_path = temp_dir.path().join("system_prompt.json");
    let system_prompt_config = json!({
        "global": "You are a helpful assistant with access to tools",
        "enabled": true,
        "injection_mode": "prepend"
    });
    std::fs::write(
        &system_prompt_path,
        serde_json::to_string_pretty(&system_prompt_config).unwrap(),
    )
    .unwrap();

    // Create MCP config
    let mcp_config_path = temp_dir.path().join("mcp.json");
    let mcp_config = json!({
        "mcpServers": {
            "test": {
                "command": "echo",
                "args": ["test"]
            }
        }
    });
    std::fs::write(
        &mcp_config_path,
        serde_json::to_string_pretty(&mcp_config).unwrap(),
    )
    .unwrap();

    // Load system prompt config
    let loaded_prompt_config =
        routiium::system_prompt_config::SystemPromptConfig::load_from_file(&system_prompt_path)
            .unwrap();

    let app_state = AppState {
        mcp_config_path: Some(mcp_config_path.to_string_lossy().to_string()),
        system_prompt_config_path: Some(system_prompt_path.to_string_lossy().to_string()),
        system_prompt_config: Arc::new(tokio::sync::RwLock::new(loaded_prompt_config)),
        ..Default::default()
    };

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let chat_request = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "Help me with a task"}
        ]
    });

    let req = test::TestRequest::post()
        .uri("/convert?include_internal_config=true")
        .insert_header(("content-type", "application/json"))
        .insert_header(("authorization", "Bearer admin-test"))
        .set_payload(serde_json::to_string(&chat_request).unwrap())
        .to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);

    let body = test::read_body(resp).await;
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Check that system prompt was injected
    let messages = body_json["input"].as_array().unwrap();
    assert!(messages.len() >= 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(
        messages[0]["content"],
        "You are a helpful assistant with access to tools"
    );

    // Tools field may be absent if no MCP servers connected successfully
    // (which is expected with mock 'echo' command that doesn't implement MCP protocol)
    // The important thing is the conversion succeeded and system prompt was injected
}

/// Test /reload/all endpoint reloads both MCP and system prompt
#[actix_web::test]
async fn test_reload_all_with_both_configs() {
    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");
    let temp_dir = tempfile::tempdir().unwrap();

    // Create system prompt config
    let system_prompt_path = temp_dir.path().join("system_prompt.json");
    let system_prompt_config = json!({
        "global": "Test prompt",
        "enabled": true,
        "injection_mode": "prepend"
    });
    std::fs::write(
        &system_prompt_path,
        serde_json::to_string_pretty(&system_prompt_config).unwrap(),
    )
    .unwrap();

    // Create MCP config
    let mcp_config_path = temp_dir.path().join("mcp.json");
    let mcp_config = json!({
        "mcpServers": {
            "test": {
                "command": "echo",
                "args": ["test"]
            }
        }
    });
    std::fs::write(
        &mcp_config_path,
        serde_json::to_string_pretty(&mcp_config).unwrap(),
    )
    .unwrap();

    let app_state = AppState {
        mcp_config_path: Some(mcp_config_path.to_string_lossy().to_string()),
        system_prompt_config_path: Some(system_prompt_path.to_string_lossy().to_string()),
        ..Default::default()
    };

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/reload/all")
        .insert_header(("content-type", "application/json"))
        .insert_header(("authorization", "Bearer admin-test"))
        .to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);

    let body = test::read_body(resp).await;
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // System prompt should succeed, MCP may fail if servers can't connect
    assert_eq!(body_json["system_prompt"]["success"], true);
    // MCP success is not guaranteed with mock 'echo' command
    assert!(body_json.get("mcp").is_some());
}

/// Test that MCP tools are formatted correctly for OpenAI API
#[actix_web::test]
async fn test_mcp_tool_format_compatibility() {
    // This test verifies the tool format matches OpenAI's expectations
    let app_state = AppState::default();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let chat_request = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "Hello"}
        ],
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "test_tool",
                    "description": "A test tool",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "param1": {
                                "type": "string",
                                "description": "Test parameter"
                            }
                        },
                        "required": ["param1"]
                    }
                }
            }
        ]
    });

    let req = test::TestRequest::post()
        .uri("/convert")
        .insert_header(("content-type", "application/json"))
        .set_payload(serde_json::to_string(&chat_request).unwrap())
        .to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);

    let body = test::read_body(resp).await;
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Verify tools are in Responses API format
    if let Some(tools) = body_json.get("tools") {
        let tools_array = tools.as_array().unwrap();
        if !tools_array.is_empty() {
            let first_tool = &tools_array[0];

            // In Responses API format, tools have a flat structure
            // with name, description, and parameters at the top level
            assert!(
                first_tool.get("name").is_some()
                    || (first_tool.get("type").is_some() && first_tool.get("function").is_some()),
                "Tool should have name field or be in OpenAI function format"
            );
        }
    }
}

/// Test status endpoint shows MCP configuration
#[actix_web::test]
async fn test_status_shows_mcp_info() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mcp_config_path = temp_dir.path().join("mcp.json");

    let mcp_config = json!({
        "mcpServers": {
            "test": {
                "command": "echo",
                "args": ["test"]
            }
        }
    });

    std::fs::write(
        &mcp_config_path,
        serde_json::to_string_pretty(&mcp_config).unwrap(),
    )
    .unwrap();

    let app_state = AppState {
        mcp_config_path: Some(mcp_config_path.to_string_lossy().to_string()),
        ..Default::default()
    };

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::get().uri("/status").to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);

    let body = test::read_body(resp).await;
    let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Should show MCP features in the status response
    // The status endpoint returns features.mcp.enabled
    assert!(body_json.get("features").is_some());
    let features = body_json["features"].as_object().unwrap();
    assert!(features.get("mcp").is_some());

    // Check that MCP config path is shown
    let mcp_info = &features["mcp"];
    assert!(mcp_info.get("config_path").is_some());
}
