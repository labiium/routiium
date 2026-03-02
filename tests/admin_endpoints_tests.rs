use actix_web::{http::header, test, web, App};
use routiium::analytics::{
    AnalyticsEvent, AnalyticsManager, AuthMetadata, CostInfo, PerformanceMetrics, RequestMetadata,
    ResponseMetadata, RoutingMetadata, TokenUsage,
};
use routiium::auth::{ApiKeyManager, KeyBackend};
use routiium::chat_history::{Conversation, Message, MessageRole, PrivacyLevel};
use routiium::chat_history_manager::{ChatHistoryConfig, ChatHistoryManager};
use routiium::pricing::PricingConfig;
use routiium::routing_config::RoutingConfig;
use routiium::server::config_routes;
use routiium::system_prompt_config::SystemPromptConfig;
use routiium::util::AppState;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    saved: Vec<(String, Option<String>)>,
}

impl EnvGuard {
    fn new(keys: &[&str]) -> Self {
        let saved = keys
            .iter()
            .map(|k| (k.to_string(), std::env::var(k).ok()))
            .collect();
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn app_state_with_keys(
    api_keys: Arc<ApiKeyManager>,
    analytics: Option<Arc<AnalyticsManager>>,
    chat_history: Option<Arc<ChatHistoryManager>>,
) -> AppState {
    AppState {
        http: reqwest::Client::new(),
        mcp_manager: None,
        api_keys: Some(api_keys),
        system_prompt_config: Arc::new(RwLock::new(SystemPromptConfig::empty())),
        analytics,
        chat_history,
        pricing: Arc::new(PricingConfig::default()),
        mcp_config_path: None,
        system_prompt_config_path: None,
        routing_config: Arc::new(RwLock::new(RoutingConfig::empty())),
        routing_config_path: None,
        router_client: None,
        router_config_path: None,
        router_url: None,
    }
}

fn sample_analytics_event(id: &str, ts: u64, endpoint: &str, success: bool) -> AnalyticsEvent {
    AnalyticsEvent {
        id: id.to_string(),
        timestamp: ts,
        request: RequestMetadata {
            endpoint: endpoint.to_string(),
            method: "POST".to_string(),
            model: Some("gpt-4o-mini".to_string()),
            stream: false,
            size_bytes: 128,
            message_count: Some(1),
            input_tokens: Some(12),
            user_agent: Some("test-agent".to_string()),
            client_ip: Some("127.0.0.1".to_string()),
        },
        response: Some(ResponseMetadata {
            status_code: if success { 200 } else { 500 },
            size_bytes: 64,
            output_tokens: Some(8),
            success,
            error_message: if success {
                None
            } else {
                Some("boom".to_string())
            },
        }),
        performance: PerformanceMetrics {
            duration_ms: 55,
            ttfb_ms: Some(20),
            upstream_duration_ms: Some(45),
            tokens_per_second: Some(145.2),
        },
        auth: AuthMetadata {
            authenticated: true,
            api_key_id: Some("key-1".to_string()),
            api_key_label: Some("test".to_string()),
            auth_method: Some("bearer".to_string()),
        },
        routing: RoutingMetadata {
            backend: "default".to_string(),
            upstream_mode: "responses".to_string(),
            mcp_enabled: false,
            mcp_servers: vec![],
            system_prompt_applied: false,
        },
        token_usage: Some(TokenUsage {
            prompt_tokens: 12,
            completion_tokens: 8,
            total_tokens: 20,
            cached_tokens: Some(2),
            reasoning_tokens: Some(3),
        }),
        cost: Some(CostInfo {
            input_cost: 0.001,
            output_cost: 0.002,
            cached_cost: Some(0.0002),
            total_cost: 0.0032,
            currency: "USD".to_string(),
            pricing_model: Some("default".to_string()),
        }),
    }
}

#[actix_web::test]
async fn keys_endpoints_enforce_admin_auth_and_support_lifecycle() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let _saved = EnvGuard::new(&[
        "ROUTIIUM_ADMIN_TOKEN",
        "ROUTIIUM_KEYS_REQUIRE_EXPIRATION",
        "ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS",
    ]);

    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");
    std::env::remove_var("ROUTIIUM_KEYS_REQUIRE_EXPIRATION");
    std::env::remove_var("ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS");

    let keys = Arc::new(
        ApiKeyManager::from_backend(KeyBackend::Memory).expect("memory key backend available"),
    );
    let app_state = app_state_with_keys(keys, None, None);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let unauthorized = test::TestRequest::get().uri("/keys").to_request();
    let unauthorized_resp = test::call_service(&app, unauthorized).await;
    assert_eq!(unauthorized_resp.status(), 401);

    let generate_batch_req = test::TestRequest::post()
        .uri("/keys/generate_batch")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .set_json(json!({
            "labels": ["one", "two"],
            "label_prefix": "team-",
            "ttl_seconds": 120
        }))
        .to_request();
    let generate_batch_resp = test::call_service(&app, generate_batch_req).await;
    assert_eq!(generate_batch_resp.status(), 200);
    let batch_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(generate_batch_resp).await).expect("batch json");
    let keys_arr = batch_body.as_array().expect("batch array");
    assert_eq!(keys_arr.len(), 2);
    assert_eq!(keys_arr[0]["label"], "team-one");
    assert_eq!(keys_arr[1]["label"], "team-two");

    let revoked_id = keys_arr[0]["id"].as_str().expect("key id").to_string();
    let active_id = keys_arr[1]["id"].as_str().expect("key id").to_string();

    let revoke_req = test::TestRequest::post()
        .uri("/keys/revoke")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .set_json(json!({ "id": revoked_id }))
        .to_request();
    let revoke_resp = test::call_service(&app, revoke_req).await;
    assert_eq!(revoke_resp.status(), 200);
    let revoke_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(revoke_resp).await).expect("revoke json");
    assert_eq!(revoke_body["revoked"], true);

    let revoke_again_req = test::TestRequest::post()
        .uri("/keys/revoke")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .set_json(json!({ "id": revoked_id }))
        .to_request();
    let revoke_again_resp = test::call_service(&app, revoke_again_req).await;
    assert_eq!(revoke_again_resp.status(), 200);
    let revoke_again_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(revoke_again_resp).await).expect("revoke json");
    assert_eq!(revoke_again_body["revoked"], false);

    let filtered_list_req = test::TestRequest::get()
        .uri("/keys?label_prefix=team-&include_revoked=false")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let filtered_list_resp = test::call_service(&app, filtered_list_req).await;
    assert_eq!(filtered_list_resp.status(), 200);
    let filtered_list_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(filtered_list_resp).await).expect("list json");
    let filtered_list = filtered_list_body.as_array().expect("list array");
    assert_eq!(filtered_list.len(), 1);
    assert_eq!(filtered_list[0]["id"], active_id);

    let set_exp_req = test::TestRequest::post()
        .uri("/keys/set_expiration")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .set_json(json!({ "id": active_id, "ttl_seconds": 90 }))
        .to_request();
    let set_exp_resp = test::call_service(&app, set_exp_req).await;
    assert_eq!(set_exp_resp.status(), 200);
    let set_exp_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(set_exp_resp).await).expect("set-exp json");
    assert_eq!(set_exp_body["updated"], true);
    assert!(set_exp_body["expires_at"].is_number());
}

#[actix_web::test]
async fn keys_respect_required_expiration_policy() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let _saved = EnvGuard::new(&[
        "ROUTIIUM_ADMIN_TOKEN",
        "ROUTIIUM_KEYS_REQUIRE_EXPIRATION",
        "ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS",
    ]);

    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");
    std::env::set_var("ROUTIIUM_KEYS_REQUIRE_EXPIRATION", "true");
    std::env::remove_var("ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS");

    let keys = Arc::new(
        ApiKeyManager::from_backend(KeyBackend::Memory).expect("memory key backend available"),
    );
    let app_state = app_state_with_keys(keys, None, None);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/keys/generate")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .set_json(json!({ "label": "no-exp" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn analytics_admin_endpoints_work_end_to_end() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let _saved = EnvGuard::new(&["ROUTIIUM_ADMIN_TOKEN"]);
    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");

    let analytics = Arc::new(AnalyticsManager::new_memory(1000));
    analytics
        .record(sample_analytics_event(
            "evt-1",
            1_720_000_000,
            "/v1/chat/completions",
            true,
        ))
        .await
        .expect("record event");
    analytics
        .record(sample_analytics_event(
            "evt-2",
            1_720_000_050,
            "/v1/responses",
            false,
        ))
        .await
        .expect("record event");

    let keys = Arc::new(
        ApiKeyManager::from_backend(KeyBackend::Memory).expect("memory key backend available"),
    );
    let app_state = app_state_with_keys(keys, Some(analytics), None);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let stats_req = test::TestRequest::get()
        .uri("/analytics/stats")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let stats_resp = test::call_service(&app, stats_req).await;
    assert_eq!(stats_resp.status(), 200);
    let stats_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(stats_resp).await).expect("stats json");
    assert_eq!(stats_body["total_events"], 2);
    assert_eq!(stats_body["backend_type"], "memory");

    let events_req = test::TestRequest::get()
        .uri("/analytics/events?start=1719999999&end=1720000100&limit=1")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let events_resp = test::call_service(&app, events_req).await;
    assert_eq!(events_resp.status(), 200);
    let events_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(events_resp).await).expect("events json");
    assert_eq!(events_body["count"], 1);
    assert_eq!(
        events_body["events"]
            .as_array()
            .expect("events array")
            .len(),
        1
    );

    let agg_req = test::TestRequest::get()
        .uri("/analytics/aggregate?start=1719999999&end=1720001000")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let agg_resp = test::call_service(&app, agg_req).await;
    assert_eq!(agg_resp.status(), 200);
    let agg_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(agg_resp).await).expect("agg json");
    assert_eq!(agg_body["total_requests"], 2);
    assert_eq!(agg_body["successful_requests"], 1);
    assert_eq!(agg_body["failed_requests"], 1);

    let export_req = test::TestRequest::get()
        .uri("/analytics/export?start=1719999999&end=1720001000&format=csv")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let export_resp = test::call_service(&app, export_req).await;
    assert_eq!(export_resp.status(), 200);
    let export_headers = export_resp.headers().clone();
    let export_body = test::read_body(export_resp).await;
    let export_text = String::from_utf8(export_body.to_vec()).expect("csv body");
    assert!(export_headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .contains("text/csv"));
    assert!(export_text.starts_with("id,timestamp,endpoint,method,model"));

    let clear_req = test::TestRequest::post()
        .uri("/analytics/clear")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let clear_resp = test::call_service(&app, clear_req).await;
    assert_eq!(clear_resp.status(), 200);

    let stats_after_req = test::TestRequest::get()
        .uri("/analytics/stats")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let stats_after_resp = test::call_service(&app, stats_after_req).await;
    let stats_after_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(stats_after_resp).await).expect("stats json");
    assert_eq!(stats_after_body["total_events"], 0);
}

#[actix_web::test]
async fn analytics_and_chat_history_return_503_when_disabled() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let _saved = EnvGuard::new(&["ROUTIIUM_ADMIN_TOKEN"]);
    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");

    let keys = Arc::new(
        ApiKeyManager::from_backend(KeyBackend::Memory).expect("memory key backend available"),
    );
    let app_state = app_state_with_keys(keys, None, None);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let analytics_req = test::TestRequest::get()
        .uri("/analytics/stats")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let analytics_resp = test::call_service(&app, analytics_req).await;
    assert_eq!(analytics_resp.status(), 503);

    let history_req = test::TestRequest::get()
        .uri("/chat_history/stats")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let history_resp = test::call_service(&app, history_req).await;
    assert_eq!(history_resp.status(), 503);
}

#[actix_web::test]
async fn chat_history_admin_endpoints_work_end_to_end() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let _saved = EnvGuard::new(&["ROUTIIUM_ADMIN_TOKEN"]);
    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");

    let chat_history_config = ChatHistoryConfig {
        enabled: true,
        primary_backend: "memory".to_string(),
        sink_backends: vec![],
        privacy_level: PrivacyLevel::Full,
        ttl_seconds: 3600,
        strict: false,
        jsonl_path: None,
        memory_max_messages: Some(1000),
        sqlite_url: None,
        postgres_url: None,
        turso_url: None,
        turso_auth_token: None,
    };

    let manager = Arc::new(
        ChatHistoryManager::new(chat_history_config)
            .await
            .expect("chat history manager"),
    );
    let conversation = Conversation::new("conv-admin".to_string());
    manager
        .record_conversation(&conversation)
        .await
        .expect("record conversation");
    let message = Message::new(
        "conv-admin".to_string(),
        MessageRole::User,
        json!({"text": "hello"}),
        PrivacyLevel::Full,
    );
    manager
        .record_message(&message)
        .await
        .expect("record message");

    let keys = Arc::new(
        ApiKeyManager::from_backend(KeyBackend::Memory).expect("memory key backend available"),
    );
    let app_state = app_state_with_keys(keys, None, Some(manager));
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let stats_req = test::TestRequest::get()
        .uri("/chat_history/stats")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let stats_resp = test::call_service(&app, stats_req).await;
    assert_eq!(stats_resp.status(), 200);
    let stats_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(stats_resp).await).expect("stats json");
    assert_eq!(stats_body["total_conversations"], 1);
    assert_eq!(stats_body["total_messages"], 1);

    let convs_req = test::TestRequest::get()
        .uri("/chat_history/conversations?limit=5")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let convs_resp = test::call_service(&app, convs_req).await;
    assert_eq!(convs_resp.status(), 200);
    let convs_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(convs_resp).await).expect("convs json");
    assert_eq!(convs_body["count"], 1);

    let conv_req = test::TestRequest::get()
        .uri("/chat_history/conversations/conv-admin")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let conv_resp = test::call_service(&app, conv_req).await;
    assert_eq!(conv_resp.status(), 200);

    let bad_messages_req = test::TestRequest::get()
        .uri("/chat_history/messages")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let bad_messages_resp = test::call_service(&app, bad_messages_req).await;
    assert_eq!(bad_messages_resp.status(), 400);

    let messages_req = test::TestRequest::get()
        .uri("/chat_history/messages?conversation_id=conv-admin&limit=10")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let messages_resp = test::call_service(&app, messages_req).await;
    assert_eq!(messages_resp.status(), 200);
    let messages_body: serde_json::Value =
        serde_json::from_slice(&test::read_body(messages_resp).await).expect("msgs json");
    assert_eq!(messages_body["count"], 1);

    let delete_req = test::TestRequest::delete()
        .uri("/chat_history/conversations/conv-admin")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let delete_resp = test::call_service(&app, delete_req).await;
    assert_eq!(delete_resp.status(), 200);

    let clear_req = test::TestRequest::post()
        .uri("/chat_history/clear")
        .insert_header((header::AUTHORIZATION, "Bearer admin-test"))
        .to_request();
    let clear_resp = test::call_service(&app, clear_req).await;
    assert_eq!(clear_resp.status(), 200);
}
