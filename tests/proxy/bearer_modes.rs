use routiium::models::chat::{ChatCompletionRequest, ChatMessage, Role};
use serde_json::Value;

mod util;
use super::mod as testmod; // Not needed; kept to satisfy potential module layout.

#[path = "mod.rs"]
mod common;
use common::{
    sample_chat_request, spawn_managed, spawn_passthrough, TestServer,
};

/// Helper: generate an internal access key by calling /keys/generate (managed mode).
async fn generate_internal_key(srv: &TestServer) -> String {
    #[derive(serde::Deserialize)]
    struct GenResp {
        token: String,
    }
    let body = serde_json::json!({"label":"test","ttl_seconds":600});
    let resp = srv
        .post_json("/keys/generate", &body, None)
        .await
        .expect("request ok");
    assert!(
        resp.status().is_success(),
        "expected success generating key, got {}",
        resp.status()
    );
    let jr: GenResp = resp.json().await.expect("json parse");
    jr.token
}

/* =============================================
   Chat Completions (/chat/completions) tests
   ============================================= */

#[tokio::test]
async fn passthrough_chat_missing_bearer_returns_401() {
    let srv = spawn_passthrough().await;

    let body = sample_chat_request();
    let resp = srv.post_json("/chat/completions", &body, None).await.unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "Expected 401 for missing bearer in passthrough /chat/completions"
    );
}

#[tokio::test]
async fn managed_chat_missing_bearer_returns_401() {
    let srv = spawn_managed().await;
    let body = sample_chat_request();
    let resp = srv.post_json("/chat/completions", &body, None).await.unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "Expected 401 for missing bearer in managed /chat/completions"
    );
}

#[tokio::test]
async fn managed_chat_valid_token_not_unauthorized() {
    let srv = spawn_managed().await;
    let token = generate_internal_key(&srv).await;
    let body = sample_chat_request();
    let resp = srv
        .post_json("/chat/completions", &body, Some(&token))
        .await
        .unwrap();
    // Upstream will fail (dummy base URL), so we allow 502/500/etc, but must NOT be 401
    assert_ne!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "Should not 401 with valid internal token"
    );
}

#[tokio::test]
async fn managed_chat_invalid_token_401() {
    let srv = spawn_managed().await;
    let body = sample_chat_request();
    let resp = srv
        .post_json("/chat/completions", &body, Some("sk_invalid.invalid"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

/* =============================================
   Proxy (/proxy) tests
   ============================================= */

/// In passthrough mode, /proxy does NOT enforce bearer; missing Authorization should still
/// progress to an upstream attempt (resulting in a non-401 error due to dummy upstream).
#[tokio::test]
async fn passthrough_proxy_allows_missing_bearer() {
    let srv = spawn_passthrough().await;

    let chat_req = ChatCompletionRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: Value::String("Hello".into()),
            name: None,
            tool_call_id: None,
                tool_calls: None,
        }],
        temperature: None,
        top_p: None,
        max_tokens: None,
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

    let resp = srv
        .post_json("/proxy", &chat_req, None)
        .await
        .expect("request");
    assert_ne!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "Passthrough /proxy should not 401 when bearer missing"
    );
}

#[tokio::test]
async fn managed_proxy_invalid_token_401() {
    let srv = spawn_managed().await;

    let chat_req = ChatCompletionRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: Value::String("Ping".into()),
            name: None,
            tool_call_id: None,
                tool_calls: None,
        }],
        temperature: None,
        top_p: None,
        max_tokens: None,
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

    let resp = srv
        .post_json("/proxy", &chat_req, Some("sk_nope.bad"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn managed_proxy_valid_token_not_unauthorized() {
    let srv = spawn_managed().await;
    let token = generate_internal_key(&srv).await;

    let chat_req = ChatCompletionRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: Value::String("Hello upstream".into()),
            name: None,
            tool_call_id: None,
                tool_calls: None,
        }],
        temperature: Some(0.2),
        top_p: None,
        max_tokens: Some(16),
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        logit_bias: None,
        user: Some("tester".into()),
        n: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        stream: Some(false),
        extra_body: None,    };

    let resp = srv
        .post_json("/proxy", &chat_req, Some(&token))
        .await
        .expect("request");
    assert_ne!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "Valid issued token should pass auth gate"
    );
}
