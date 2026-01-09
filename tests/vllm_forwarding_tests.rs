use actix_web::{test, web, App};
use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

use routiium::server::config_routes;
use routiium::system_prompt_config::SystemPromptConfig;
use routiium::util::AppState;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone)]
struct UpstreamState {
    requests: Arc<AsyncMutex<Vec<Value>>>,
    response: Arc<AsyncMutex<Value>>,
}

async fn handle_chat(
    State(state): State<UpstreamState>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    state.requests.lock().await.push(payload);
    let response = state.response.lock().await.clone();
    Json(response)
}

struct MockUpstream {
    base_url: String,
    requests: Arc<AsyncMutex<Vec<Value>>>,
    join: JoinHandle<()>,
}

impl MockUpstream {
    async fn start(response: Value) -> Self {
        let requests = Arc::new(AsyncMutex::new(Vec::new()));
        let state = UpstreamState {
            requests: requests.clone(),
            response: Arc::new(AsyncMutex::new(response)),
        };

        let app = Router::new()
            .route("/v1/chat/completions", post(handle_chat))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind upstream");
        let addr = listener.local_addr().expect("local addr");
        let base_url = format!("http://{}", addr);

        let join = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("upstream server error");
        });

        Self {
            base_url,
            requests,
            join,
        }
    }

    async fn last_request(&self) -> Value {
        let guard = self.requests.lock().await;
        guard.last().cloned().unwrap_or_else(|| json!({}))
    }
}

impl Drop for MockUpstream {
    fn drop(&mut self) {
        self.join.abort();
    }
}

struct EnvRestore {
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvRestore {
    fn capture(keys: &[&'static str]) -> Self {
        let saved = keys.iter().map(|&k| (k, std::env::var(k).ok())).collect();
        Self { saved }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            if let Some(val) = value {
                std::env::set_var(key, val);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

#[actix_web::test]
async fn chat_passthrough_preserves_vllm_args_with_prompt_injection() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let _restore = EnvRestore::capture(&[
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "ROUTIIUM_BACKENDS",
        "ROUTIIUM_ROUTER_URL",
        "ROUTIIUM_ROUTER_STRICT",
    ]);

    let upstream_response = json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1,
        "model": "local-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "ok"},
            "finish_reason": "stop"
        }]
    });
    let upstream = MockUpstream::start(upstream_response).await;

    std::env::set_var("OPENAI_BASE_URL", format!("{}/v1", upstream.base_url));
    std::env::remove_var("OPENAI_API_KEY");
    std::env::remove_var("ROUTIIUM_BACKENDS");
    std::env::remove_var("ROUTIIUM_ROUTER_URL");
    std::env::remove_var("ROUTIIUM_ROUTER_STRICT");

    let mut per_api = HashMap::new();
    per_api.insert("chat".to_string(), "System prompt".to_string());
    let prompt_config = SystemPromptConfig {
        global: None,
        per_model: HashMap::new(),
        per_api,
        injection_mode: "prepend".to_string(),
        enabled: true,
    };

    let mut state = AppState::default();
    state.system_prompt_config = Arc::new(tokio::sync::RwLock::new(prompt_config));
    state.analytics = None;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let payload = json!({
        "model": "local-model",
        "messages": [{"role": "user", "content": "Hi"}],
        "max_tokens": 8192,
        "chat_template_kwargs": {"enable_thinking": false},
        "top_k": 42,
        "repetition_penalty": 1.1,
        "seed": 123
    });

    let req = test::TestRequest::post()
        .uri("/v1/chat/completions")
        .insert_header(("Authorization", "Bearer test"))
        .set_json(&payload)
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    let _ = test::read_body(resp).await;

    let forwarded = upstream.last_request().await;
    assert_eq!(forwarded["max_tokens"], 8192);
    assert_eq!(forwarded["top_k"], 42);
    assert_eq!(forwarded["repetition_penalty"], 1.1);
    assert_eq!(forwarded["seed"], 123);
    assert_eq!(forwarded["chat_template_kwargs"]["enable_thinking"], false);

    let messages = forwarded["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "System prompt");
    assert_eq!(messages[1]["role"], "user");
}

#[actix_web::test]
async fn chat_passthrough_preserves_reasoning_content() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let _restore = EnvRestore::capture(&[
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "ROUTIIUM_BACKENDS",
        "ROUTIIUM_ROUTER_URL",
        "ROUTIIUM_ROUTER_STRICT",
    ]);

    let upstream_response = json!({
        "id": "chatcmpl-reasoning",
        "object": "chat.completion",
        "created": 2,
        "model": "local-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "reasoning": "draft",
                "reasoning_content": "Metal heart glows bright"
            },
            "finish_reason": "stop"
        }]
    });
    let upstream = MockUpstream::start(upstream_response).await;

    std::env::set_var("OPENAI_BASE_URL", format!("{}/v1", upstream.base_url));
    std::env::remove_var("OPENAI_API_KEY");
    std::env::remove_var("ROUTIIUM_BACKENDS");
    std::env::remove_var("ROUTIIUM_ROUTER_URL");
    std::env::remove_var("ROUTIIUM_ROUTER_STRICT");

    let mut state = AppState::default();
    state.analytics = None;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let payload = json!({
        "model": "local-model",
        "messages": [{"role": "user", "content": "Write a haiku"}],
        "max_tokens": 128
    });

    let req = test::TestRequest::post()
        .uri("/v1/chat/completions")
        .insert_header(("Authorization", "Bearer test"))
        .set_json(&payload)
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    let body = test::read_body(resp).await;
    let parsed: Value = serde_json::from_slice(&body).expect("response json");

    assert_eq!(
        parsed["choices"][0]["message"]["reasoning_content"],
        "Metal heart glows bright"
    );
    assert!(parsed["choices"][0]["message"]["content"].is_null());
}

#[actix_web::test]
async fn responses_passthrough_forwards_vllm_args_in_chat_mode() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let _restore = EnvRestore::capture(&[
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "ROUTIIUM_BACKENDS",
        "ROUTIIUM_ROUTER_URL",
        "ROUTIIUM_ROUTER_STRICT",
    ]);

    let upstream_response = json!({
        "id": "chatcmpl-resp",
        "object": "chat.completion",
        "created": 3,
        "model": "local-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "ok"},
            "finish_reason": "stop"
        }]
    });
    let upstream = MockUpstream::start(upstream_response).await;

    std::env::set_var("OPENAI_BASE_URL", format!("{}/v1", upstream.base_url));
    std::env::remove_var("OPENAI_API_KEY");
    std::env::set_var(
        "ROUTIIUM_BACKENDS",
        format!("prefix=local-,base={}/v1,mode=chat", upstream.base_url),
    );

    let mut state = AppState::default();
    state.analytics = None;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let payload = json!({
        "model": "local-test",
        "input": [{"role": "user", "content": "Hello"}],
        "max_output_tokens": 7,
        "chat_template_kwargs": {"enable_thinking": false},
        "top_k": 7,
        "stop_token_ids": [1, 2, 3]
    });

    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .insert_header(("Authorization", "Bearer test"))
        .set_json(&payload)
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    let _ = test::read_body(resp).await;

    let forwarded = upstream.last_request().await;
    assert_eq!(forwarded["max_tokens"], 7);
    assert!(forwarded.get("max_output_tokens").is_none());
    assert_eq!(forwarded["top_k"], 7);
    assert_eq!(forwarded["stop_token_ids"], json!([1, 2, 3]));
    assert_eq!(forwarded["chat_template_kwargs"]["enable_thinking"], false);
    assert!(forwarded.get("input").is_none());
    assert!(forwarded.get("messages").is_some());
}
