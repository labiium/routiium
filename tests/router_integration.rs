use actix_web::{test, web, App};
use axum::{extract::State, routing::post, Json, Router};
use http::StatusCode as RouterStatusCode;
use reqwest::Client;
use routiium::{
    analytics::AnalyticsManager,
    router_client::{
        CachedRouterClient, HttpRouterClient, HttpRouterConfig, JudgeMetadata, RouterClient,
    },
    server::config_routes,
    util::AppState,
};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[path = "common/router_stub.rs"]
mod router_stub;

use router_stub::{sample_plan, RouterResponseConfig, RouterStub};

static ENV_GUARD: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

#[derive(Clone)]
struct MockUpstream {
    base_url: String,
    calls: Arc<AtomicUsize>,
}

impl MockUpstream {
    #[allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "test upstream setup intentionally fails fast"
    )]
    async fn start_responses(response: serde_json::Value) -> Self {
        let calls = Arc::new(AtomicUsize::new(0));
        let state = (calls.clone(), response);
        let app = Router::new()
            .route("/v1/responses", post(mock_responses_handler))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind upstream");
        let addr = listener.local_addr().expect("upstream addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("upstream server error");
        });
        Self {
            base_url: format!("http://{addr}"),
            calls,
        }
    }

    #[allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "test upstream setup intentionally fails fast"
    )]
    async fn start_chat(response: serde_json::Value) -> Self {
        let calls = Arc::new(AtomicUsize::new(0));
        let state = (calls.clone(), response);
        let app = Router::new()
            .route("/v1/chat/completions", post(mock_responses_handler))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind upstream");
        let addr = listener.local_addr().expect("upstream addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("upstream server error");
        });
        Self {
            base_url: format!("http://{addr}"),
            calls,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

async fn mock_responses_handler(
    State((calls, response)): State<(Arc<AtomicUsize>, serde_json::Value)>,
    Json(_body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    calls.fetch_add(1, Ordering::SeqCst);
    Json(response)
}

fn basic_responses_body() -> serde_json::Value {
    json!({
        "model": "nano-basic",
        "input": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": "ping router"}
                ]
            }
        ],
        "stream": false
    })
}

fn cacheable_judge() -> JudgeMetadata {
    JudgeMetadata {
        profile: Some("standard".to_string()),
        gates: Some(vec!["content".to_string(), "safety_judge".to_string()]),
        id: Some("jdg_cacheable".to_string()),
        action: Some("route".to_string()),
        mode: Some("enforce".to_string()),
        verdict: Some("allow".to_string()),
        risk_level: Some("low".to_string()),
        reason: Some("safe exact-match cache fixture".to_string()),
        target: None,
        categories: None,
        requires_approval: None,
        policy_rev: Some("cache_policy_v1".to_string()),
        policy_fingerprint: Some("sha256:test-cache-policy".to_string()),
        cacheable: Some(true),
        selector_scope: None,
        selector_action: None,
        selector_rules: None,
        selector_reason: None,
    }
}

fn clear_router_env() {
    std::env::remove_var("ROUTIIUM_ROUTER_STRICT");
    std::env::remove_var("ROUTIIUM_CACHE_TTL_MS");
    std::env::remove_var("ROUTIIUM_GOVERNANCE_MODE");
    std::env::remove_var("ROUTIIUM_EXACT_RESPONSE_CACHE");
    std::env::remove_var("ROUTIIUM_EXACT_RESPONSE_CACHE_TTL_MS");
    std::env::remove_var("ROUTIIUM_EXACT_RESPONSE_CACHE_MAX_ENTRIES");
    std::env::remove_var("ROUTIIUM_REJECTION_MODE");
    std::env::remove_var("PATCHBAY_PROFILE");
    std::env::remove_var("ROUTIIUM_ADMIN_TOKEN");
    std::env::remove_var("OPENAI_BASE_URL");
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test client setup intentionally fails fast"
)]
fn build_app_state(router_url: &str, cache_ttl: u64) -> AppState {
    let router_http_client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(500))
        .connect_timeout(Duration::from_millis(100))
        .build()
        .expect("router http client");
    let http_router = HttpRouterClient::new(HttpRouterConfig {
        url: router_url.to_string(),
        timeout_ms: 500,
        mtls: false,
        client: Some(router_http_client),
    })
    .expect("http router client");
    let cached = CachedRouterClient::new(Box::new(http_router), cache_ttl);
    let router_client: Arc<dyn RouterClient> = Arc::new(cached);
    let base_state = AppState::default();
    AppState {
        router_client: Some(router_client),
        router_url: Some(router_url.to_string()),
        router_strict: std::env::var("ROUTIIUM_ROUTER_STRICT")
            .ok()
            .is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            }),
        router_cache_ttl_ms: Some(cache_ttl),
        analytics: Some(Arc::new(AnalyticsManager::new_memory(1000))),
        // Use a short timeout so unreachable upstreams fail fast in tests rather than hanging.
        http: Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(250))
            .connect_timeout(Duration::from_millis(100))
            .build()
            .expect("test http client"),
        ..base_state
    }
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn router_headers_are_forwarded() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");

    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(sample_plan(
        "rte_headers",
        "gpt-4o-mini",
    ))))
    .await;
    let state = build_app_state(&router.url(), 60_000);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .insert_header(("Authorization", "Bearer router-test"))
        .set_json(basic_responses_body())
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_server_error());

    let headers = resp.headers();
    assert_eq!(
        headers.get("x-route-id").and_then(|v| v.to_str().ok()),
        Some("rte_headers")
    );
    assert_eq!(
        headers.get("router-schema").and_then(|v| v.to_str().ok()),
        Some("1.1")
    );
    assert_eq!(
        headers.get("x-policy-rev").and_then(|v| v.to_str().ok()),
        Some("router_policy_v1")
    );
    assert_eq!(
        headers.get("x-content-used").and_then(|v| v.to_str().ok()),
        Some("none")
    );

    assert_eq!(router.calls(), 1);
    let captured = router.take_requests();
    assert_eq!(captured.len(), 1);
    let req = &captured[0];
    assert_eq!(req.alias, "nano-basic");
    assert_eq!(req.api, "responses");
    assert_eq!(req.schema_version.as_deref(), Some("1.1"));
    assert!(req.caps.contains(&"text".to_string()));
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "Bench handshake integration fixtures intentionally fail fast"
)]
#[actix_web::test]
async fn bench_accepts_certified_remote_plan_without_legacy_fallback() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("PATCHBAY_PROFILE", "bench");
    let mut plan = sample_plan("rte_bench_certified", "gpt-4o-mini");
    plan.judge = Some(JudgeMetadata {
        profile: Some("bench".to_string()),
        gates: Some(Vec::new()),
        action: Some("allow".to_string()),
        verdict: Some("allow".to_string()),
        ..Default::default()
    });
    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(plan))).await;
    let state = build_app_state(&router.url(), 0);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/v1/responses")
            .insert_header(("Authorization", "Bearer router-test"))
            .set_json(basic_responses_body())
            .to_request(),
    )
    .await;
    assert_eq!(router.calls(), 1);
    assert_ne!(resp.status().as_u16(), 503);
    std::env::remove_var("PATCHBAY_PROFILE");
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "Bench handshake integration fixtures intentionally fail fast"
)]
#[actix_web::test]
async fn bench_rejects_uncertified_remote_plan_without_legacy_fallback() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("PATCHBAY_PROFILE", "bench");
    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(sample_plan(
        "rte_bench_uncertified",
        "gpt-4o-mini",
    ))))
    .await;
    let state = build_app_state(&router.url(), 0);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/v1/responses")
            .insert_header(("Authorization", "Bearer router-test"))
            .set_json(basic_responses_body())
            .to_request(),
    )
    .await;
    assert_eq!(router.calls(), 1);
    assert_eq!(resp.status().as_u16(), 503);
    let body: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(body["error"]["code"], "BENCH_ROUTER_UNCERTIFIED");
    std::env::remove_var("PATCHBAY_PROFILE");
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn router_judge_headers_are_forwarded() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");

    let mut plan = sample_plan("rte_judged", "gpt-4o-mini");
    plan.judge = Some(JudgeMetadata {
        profile: Some("standard".to_string()),
        gates: Some(vec!["content".to_string(), "safety_judge".to_string()]),
        id: None,
        action: Some("route".to_string()),
        mode: Some("enforce".to_string()),
        verdict: Some("downgrade".to_string()),
        risk_level: Some("medium".to_string()),
        reason: Some("Use a safer route".to_string()),
        target: Some("gpt-4o-mini".to_string()),
        categories: None,
        requires_approval: None,
        policy_rev: None,
        policy_fingerprint: None,
        cacheable: None,
        selector_scope: None,
        selector_action: None,
        selector_rules: None,
        selector_reason: None,
    });
    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(plan))).await;
    let state = build_app_state(&router.url(), 60_000);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .insert_header(("Authorization", "Bearer router-test"))
        .set_json(basic_responses_body())
        .to_request();
    let resp = test::call_service(&app, req).await;

    let headers = resp.headers();
    assert_eq!(
        headers.get("x-judge-mode").and_then(|v| v.to_str().ok()),
        Some("enforce")
    );
    assert_eq!(
        headers.get("x-judge-action").and_then(|v| v.to_str().ok()),
        Some("route")
    );
    assert_eq!(
        headers.get("x-judge-verdict").and_then(|v| v.to_str().ok()),
        Some("downgrade")
    );
    assert_eq!(
        headers.get("x-judge-risk").and_then(|v| v.to_str().ok()),
        Some("medium")
    );
    assert_eq!(
        headers.get("x-judge-target").and_then(|v| v.to_str().ok()),
        Some("gpt-4o-mini")
    );
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn router_strict_mode_surfaces_error() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");
    std::env::set_var("ROUTIIUM_ROUTER_STRICT", "1");

    let router = RouterStub::start(RouterResponseConfig::Error {
        status: RouterStatusCode::CONFLICT,
        body: json!({
            "error": {
                "code": "ALIAS_UNKNOWN",
                "message": "alias not found"
            }
        }),
    })
    .await;

    let state = build_app_state(&router.url(), 15_000);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .insert_header(("Authorization", "Bearer router-test"))
        .set_json(basic_responses_body())
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(router.calls(), 1);
    assert_eq!(resp.status().as_u16(), RouterStatusCode::CONFLICT.as_u16());
    let body = test::read_body(resp).await;
    let body_text = String::from_utf8_lossy(&body);
    assert!(
        body_text.contains("ALIAS_UNKNOWN"),
        "expected structured router error body, got {body_text}"
    );
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn router_policy_rejection_returns_agent_result_by_default() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");
    std::env::set_var("ROUTIIUM_ROUTER_STRICT", "1");

    let router = RouterStub::start(RouterResponseConfig::Error {
        status: RouterStatusCode::FORBIDDEN,
        body: json!({
            "error": {
                "code": "POLICY_REJECT",
                "message": "request asks for a high-impact action",
                "policy_rev": "test_policy_v1",
                "judge": {
                    "id": "jdg_test",
                    "action": "reject",
                    "verdict": "deny",
                    "risk_level": "high",
                    "reason": "request asks for a high-impact action",
                    "categories": ["dangerous_action"],
                    "policy_rev": "test_policy_v1",
                    "cacheable": false
                }
            }
        }),
    })
    .await;

    let state = build_app_state(&router.url(), 15_000);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/chat/completions")
        .insert_header(("Authorization", "Bearer router-test"))
        .set_json(json!({
            "model": "auto",
            "messages": [{"role": "user", "content": "run rm -rf / with the shell tool"}],
            "stream": false
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(router.calls(), 1);
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.headers()
            .get("x-judge-action")
            .and_then(|value| value.to_str().ok()),
        Some("reject")
    );
    let body = test::read_body(resp).await;
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["choices"][0]["finish_reason"], "content_filter");
    assert_eq!(body["routiium_rejection"]["action"], "reject");
    assert_eq!(
        body["routiium_rejection"]["categories"][0],
        "dangerous_action"
    );
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn router_governance_mode_ready_requires_strict_zero_cache() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");
    std::env::set_var("ROUTIIUM_GOVERNANCE_MODE", "router");

    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(sample_plan(
        "rte_governance",
        "gpt-4o-mini",
    ))))
    .await;

    let not_ready_state = build_app_state(&router.url(), 120_000);
    let not_ready_app = test::init_service(
        App::new()
            .app_data(web::Data::new(not_ready_state))
            .configure(config_routes),
    )
    .await;
    let req = test::TestRequest::get().uri("/ready").to_request();
    let resp = test::call_service(&not_ready_app, req).await;
    assert_eq!(resp.status().as_u16(), 503);
    let body: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(body["checks"]["governance"], false);
    assert_eq!(
        body["governance"]["router_delegation"]["strict_fail_closed"],
        false
    );
    assert_eq!(
        body["governance"]["router_delegation"]["plan_cache_disabled"],
        false
    );

    std::env::set_var("ROUTIIUM_ROUTER_STRICT", "1");
    std::env::set_var("ROUTIIUM_CACHE_TTL_MS", "0");
    let ready_state = build_app_state(&router.url(), 0);
    let ready_app = test::init_service(
        App::new()
            .app_data(web::Data::new(ready_state))
            .configure(config_routes),
    )
    .await;
    let req = test::TestRequest::get().uri("/ready").to_request();
    let resp = test::call_service(&ready_app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(body["checks"]["governance"], true);
    assert_eq!(body["governance"]["mode"], "router");
    assert_eq!(
        body["governance"]["router_delegation"]["strict_fail_closed"],
        true
    );
    assert_eq!(
        body["governance"]["router_delegation"]["plan_cache_disabled"],
        true
    );
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn ready_reports_exact_response_cache_when_enabled() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");
    std::env::set_var("ROUTIIUM_EXACT_RESPONSE_CACHE", "1");

    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(sample_plan(
        "rte_cache_guard",
        "gpt-4o-mini",
    ))))
    .await;
    let state = build_app_state(&router.url(), 0);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::get().uri("/ready").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(body["checks"]["governance"], true);
    assert_eq!(body["governance"]["exact_response_cache"]["enabled"], true);
    assert_eq!(body["governance"]["exact_response_cache"]["ready"], true);
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn router_governance_mode_rejects_exact_response_cache() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");
    std::env::set_var("ROUTIIUM_GOVERNANCE_MODE", "router");
    std::env::set_var("ROUTIIUM_ROUTER_STRICT", "1");
    std::env::set_var("ROUTIIUM_CACHE_TTL_MS", "0");
    std::env::set_var("ROUTIIUM_EXACT_RESPONSE_CACHE", "1");

    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(sample_plan(
        "rte_governed_cache",
        "gpt-4o-mini",
    ))))
    .await;
    let state = build_app_state(&router.url(), 0);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::get().uri("/ready").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 503);
    let body: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(body["checks"]["governance"], false);
    assert_eq!(body["governance"]["router_delegation"]["ready"], true);
    assert_eq!(body["governance"]["exact_response_cache"]["enabled"], true);
    assert_eq!(body["governance"]["exact_response_cache"]["ready"], false);
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn exact_response_cache_reuses_safe_non_streaming_response_and_clears() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");
    std::env::set_var("ROUTIIUM_EXACT_RESPONSE_CACHE", "1");
    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");

    let upstream = MockUpstream::start_responses(json!({
        "id": "rsp-cache-1",
        "object": "response",
        "created": 1_720_000_000u64,
        "model": "gpt-4o-mini",
        "output_text": "cached answer",
        "output": [{
            "id": "msg-cache-1",
            "type": "assistant_message",
            "content": "cached answer"
        }],
        "usage": {
            "input_tokens": 4,
            "output_tokens": 2,
            "total_tokens": 6
        }
    }))
    .await;

    let mut plan = sample_plan("rte_exact_cache", "gpt-4o-mini");
    plan.upstream.base_url = format!("{}/v1", upstream.base_url);
    plan.judge = Some(cacheable_judge());
    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(plan))).await;
    let state = build_app_state(&router.url(), 60_000);
    let analytics = state.analytics.clone().expect("analytics enabled");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let body = json!({
        "model": "nano-basic",
        "messages": [{"role": "user", "content": "cache me"}],
        "stream": false
    });

    for idx in 0..2 {
        let req = test::TestRequest::post()
            .uri("/v1/chat/completions")
            .insert_header(("Authorization", "Bearer router-test"))
            .set_json(&body)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 200);
        if idx == 1 {
            assert_eq!(
                resp.headers()
                    .get("x-routiium-cache")
                    .and_then(|value| value.to_str().ok()),
                Some("hit")
            );
        }
        let body: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "cached answer");
    }
    assert_eq!(upstream.calls(), 1);
    let events = analytics
        .query_range(0, u64::MAX, None)
        .await
        .expect("analytics events");
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| {
        event.response.as_ref().is_some_and(|response| {
            response.usage_source == routiium::analytics::UsageSource::Measured
        })
    }));
    assert!(events
        .iter()
        .all(|event| event.token_usage.as_ref().map(|usage| usage.total_tokens) == Some(6)));

    let clear_req = test::TestRequest::post()
        .uri("/admin/cache/exact/clear")
        .insert_header(("Authorization", "Bearer admin-test"))
        .to_request();
    let clear_resp = test::call_service(&app, clear_req).await;
    assert_eq!(clear_resp.status().as_u16(), 200);

    let req = test::TestRequest::post()
        .uri("/v1/chat/completions")
        .insert_header(("Authorization", "Bearer router-test"))
        .set_json(&body)
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(upstream.calls(), 2);
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn exact_response_cache_requires_explicit_cacheable_judge_metadata() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");
    std::env::set_var("ROUTIIUM_EXACT_RESPONSE_CACHE", "1");

    let upstream = MockUpstream::start_responses(json!({
        "id": "rsp-no-cache-metadata",
        "object": "response",
        "created": 1_720_000_000u64,
        "model": "gpt-4o-mini",
        "output_text": "uncached answer",
        "output": [{
            "id": "msg-no-cache-metadata",
            "type": "assistant_message",
            "content": "uncached answer"
        }],
        "usage": {
            "input_tokens": 4,
            "output_tokens": 2,
            "total_tokens": 6
        }
    }))
    .await;

    let mut plan = sample_plan("rte_missing_cacheable", "gpt-4o-mini");
    plan.upstream.base_url = format!("{}/v1", upstream.base_url);
    plan.judge = None;
    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(plan))).await;
    let state = build_app_state(&router.url(), 60_000);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;
    let body = json!({
        "model": "nano-basic",
        "messages": [{"role": "user", "content": "do not cache without metadata"}],
        "stream": false
    });

    for _ in 0..2 {
        let req = test::TestRequest::post()
            .uri("/v1/chat/completions")
            .insert_header(("Authorization", "Bearer router-test"))
            .set_json(&body)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 200);
        assert_ne!(
            resp.headers()
                .get("x-routiium-cache")
                .and_then(|value| value.to_str().ok()),
            Some("hit")
        );
    }
    assert_eq!(
        upstream.calls(),
        2,
        "missing explicit judge.cacheable=true metadata must bypass exact response cache"
    );
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn exact_response_cache_misses_when_policy_revision_changes() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");
    std::env::set_var("ROUTIIUM_EXACT_RESPONSE_CACHE", "1");

    let upstream = MockUpstream::start_responses(json!({
        "id": "rsp-policy-cache",
        "object": "response",
        "created": 1_720_000_000u64,
        "model": "gpt-4o-mini",
        "output_text": "policy scoped answer",
        "output": [{
            "id": "msg-policy-cache",
            "type": "assistant_message",
            "content": "policy scoped answer"
        }],
        "usage": {
            "input_tokens": 4,
            "output_tokens": 2,
            "total_tokens": 6
        }
    }))
    .await;

    let mut first = sample_plan("rte_exact_cache_policy", "gpt-4o-mini");
    first.upstream.base_url = format!("{}/v1", upstream.base_url);
    first.judge = Some(cacheable_judge());
    first.policy_rev = Some("router_policy_v1".to_string());

    let mut second = first.clone();
    second.policy_rev = Some("router_policy_v2".to_string());
    if let Some(judge) = second.judge.as_mut() {
        judge.policy_rev = Some("cache_policy_v2".to_string());
        judge.policy_fingerprint = Some("sha256:test-cache-policy-v2".to_string());
    }

    let router = RouterStub::start(RouterResponseConfig::Plans(vec![first, second])).await;
    let state = build_app_state(&router.url(), 0);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;
    let body = json!({
        "model": "nano-basic",
        "messages": [{"role": "user", "content": "cache policy scoped"}],
        "stream": false
    });

    for _ in 0..2 {
        let req = test::TestRequest::post()
            .uri("/v1/chat/completions")
            .insert_header(("Authorization", "Bearer router-test"))
            .set_json(&body)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 200);
        assert_ne!(
            resp.headers()
                .get("x-routiium-cache")
                .and_then(|value| value.to_str().ok()),
            Some("hit")
        );
    }
    assert_eq!(router.calls(), 2);
    assert_eq!(
        upstream.calls(),
        2,
        "policy revision changes must not reuse exact cache entries"
    );
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn chat_non_streaming_uses_router_fallback_after_primary_transport_failure() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");

    let upstream = MockUpstream::start_responses(json!({
        "id": "rsp-fallback-1",
        "object": "response",
        "created": 1_720_000_000u64,
        "model": "gpt-4o-mini",
        "output_text": "fallback answer",
        "output": [{
            "id": "msg-fallback-1",
            "type": "assistant_message",
            "content": "fallback answer"
        }],
        "usage": {
            "input_tokens": 4,
            "output_tokens": 2,
            "total_tokens": 6
        }
    }))
    .await;

    let mut plan = sample_plan("rte_chat_fallback", "primary-model");
    plan.upstream.base_url = "http://127.0.0.1:9/v1".to_string();
    plan.fallbacks = vec![routiium::router_client::FallbackConfig {
        base_url: format!("{}/v1", upstream.base_url),
        mode: routiium::router_client::UpstreamMode::Responses,
        model_id: "fallback-model".to_string(),
        reason: Some("primary_transport".to_string()),
        auth_env: None,
        penalty: None,
    }];
    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(plan))).await;
    let state = build_app_state(&router.url(), 60_000);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/chat/completions")
        .insert_header(("Authorization", "Bearer router-test"))
        .set_json(json!({
            "model": "nano-basic",
            "messages": [{"role": "user", "content": "use fallback"}],
            "stream": false
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "fallback answer");
    assert_eq!(router.calls(), 1);
    assert_eq!(upstream.calls(), 1);
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn chat_non_streaming_converts_responses_fallback_to_chat_schema() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");

    let upstream = MockUpstream::start_responses(json!({
        "id": "rsp-cross-mode-fallback",
        "object": "response",
        "created": 1_720_000_000u64,
        "model": "gpt-4o-mini",
        "output_text": "responses fallback answer",
        "output": [{
            "id": "msg-cross-mode-fallback",
            "type": "assistant_message",
            "content": "responses fallback answer"
        }],
        "usage": {
            "input_tokens": 4,
            "output_tokens": 2,
            "total_tokens": 6
        }
    }))
    .await;

    let mut plan = sample_plan("rte_chat_to_responses_fallback", "primary-chat-model");
    plan.upstream.mode = routiium::router_client::UpstreamMode::Chat;
    plan.upstream.base_url = "http://127.0.0.1:9/v1".to_string();
    plan.fallbacks = vec![routiium::router_client::FallbackConfig {
        base_url: format!("{}/v1", upstream.base_url),
        mode: routiium::router_client::UpstreamMode::Responses,
        model_id: "fallback-responses-model".to_string(),
        reason: Some("primary_transport".to_string()),
        auth_env: None,
        penalty: None,
    }];
    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(plan))).await;
    let state = build_app_state(&router.url(), 60_000);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/chat/completions")
        .insert_header(("Authorization", "Bearer router-test"))
        .set_json(json!({
            "model": "nano-basic",
            "messages": [{"role": "user", "content": "use responses fallback"}],
            "stream": false
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "responses fallback answer"
    );
    assert!(body.get("output").is_none(), "raw Responses schema leaked");
    assert_eq!(upstream.calls(), 1);
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn exact_response_cache_does_not_store_fallback_terminal_response_under_primary_key() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");
    std::env::set_var("ROUTIIUM_EXACT_RESPONSE_CACHE", "1");

    let upstream = MockUpstream::start_responses(json!({
        "id": "rsp-fallback-no-cache",
        "object": "response",
        "created": 1_720_000_000u64,
        "model": "gpt-4o-mini",
        "output_text": "fallback should not cache",
        "output": [{
            "id": "msg-fallback-no-cache",
            "type": "assistant_message",
            "content": "fallback should not cache"
        }],
        "usage": {
            "input_tokens": 4,
            "output_tokens": 2,
            "total_tokens": 6
        }
    }))
    .await;

    let mut plan = sample_plan("rte_fallback_no_exact_cache", "primary-chat-model");
    plan.upstream.mode = routiium::router_client::UpstreamMode::Chat;
    plan.upstream.base_url = "http://127.0.0.1:9/v1".to_string();
    plan.judge = Some(cacheable_judge());
    plan.fallbacks = vec![routiium::router_client::FallbackConfig {
        base_url: format!("{}/v1", upstream.base_url),
        mode: routiium::router_client::UpstreamMode::Responses,
        model_id: "fallback-responses-model".to_string(),
        reason: Some("primary_transport".to_string()),
        auth_env: None,
        penalty: None,
    }];
    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(plan))).await;
    let state = build_app_state(&router.url(), 60_000);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let body = json!({
        "model": "nano-basic",
        "messages": [{"role": "user", "content": "fallback cache safety"}],
        "stream": false
    });

    for _ in 0..2 {
        let req = test::TestRequest::post()
            .uri("/v1/chat/completions")
            .insert_header(("Authorization", "Bearer router-test"))
            .set_json(&body)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 200);
        assert_ne!(
            resp.headers()
                .get("x-routiium-cache")
                .and_then(|value| value.to_str().ok()),
            Some("hit")
        );
        let body: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
        assert_eq!(
            body["choices"][0]["message"]["content"],
            "fallback should not cache"
        );
    }
    assert_eq!(
        upstream.calls(),
        2,
        "fallback terminal responses must not be inserted under the primary exact-cache key"
    );
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn responses_non_streaming_converts_chat_fallback_to_responses_schema() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");

    let upstream = MockUpstream::start_chat(json!({
        "id": "chatcmpl-cross-mode-fallback",
        "object": "chat.completion",
        "created": 1_720_000_000u64,
        "model": "gpt-4o-mini",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "chat fallback answer"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 4,
            "completion_tokens": 2,
            "total_tokens": 6
        }
    }))
    .await;

    let mut plan = sample_plan("rte_responses_to_chat_fallback", "primary-responses-model");
    plan.upstream.base_url = "http://127.0.0.1:9/v1".to_string();
    plan.upstream.mode = routiium::router_client::UpstreamMode::Responses;
    plan.fallbacks = vec![routiium::router_client::FallbackConfig {
        base_url: format!("{}/v1", upstream.base_url),
        mode: routiium::router_client::UpstreamMode::Chat,
        model_id: "fallback-chat-model".to_string(),
        reason: Some("primary_transport".to_string()),
        auth_env: None,
        penalty: None,
    }];
    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(plan))).await;
    let state = build_app_state(&router.url(), 60_000);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .insert_header(("Authorization", "Bearer router-test"))
        .set_json(basic_responses_body())
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(body["object"], "response");
    assert_eq!(body["output_text"], "chat fallback answer");
    assert!(
        body.get("choices").is_none(),
        "raw Chat Completions schema leaked"
    );
    assert_eq!(upstream.calls(), 1);
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn router_plan_is_cached_between_requests() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");

    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(sample_plan(
        "rte_cached",
        "gpt-4o-mini",
    ))))
    .await;
    let state = build_app_state(&router.url(), 120_000);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;
    let body = basic_responses_body();

    for _ in 0..2 {
        let req = test::TestRequest::post()
            .uri("/v1/responses")
            .insert_header(("Authorization", "Bearer router-test"))
            .set_json(&body)
            .to_request();
        let _ = test::call_service(&app, req).await;
    }

    assert_eq!(
        router.calls(),
        1,
        "cached router client should reuse plan without extra HTTP calls"
    );
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn zero_ttl_router_plan_is_not_cached_between_requests() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");

    let mut plan = sample_plan("rte_no_store", "gpt-4o-mini");
    if let Some(cache) = plan.cache.as_mut() {
        cache.ttl_ms = 0;
    }
    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(plan))).await;
    let state = build_app_state(&router.url(), 120_000);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;
    let body = basic_responses_body();

    for _ in 0..2 {
        let req = test::TestRequest::post()
            .uri("/v1/responses")
            .insert_header(("Authorization", "Bearer router-test"))
            .set_json(&body)
            .to_request();
        let _ = test::call_service(&app, req).await;
    }

    assert_eq!(
        router.calls(),
        2,
        "zero-TTL router plans must not be cached, so judged routes run every request"
    );
    clear_router_env();
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests intentionally fail fast on malformed fixtures"
)]
#[actix_web::test]
async fn zero_default_cache_ttl_disables_router_plan_cache() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");

    let router = RouterStub::start(RouterResponseConfig::Plan(Box::new(sample_plan(
        "rte_default_no_cache",
        "gpt-4o-mini",
    ))))
    .await;
    let state = build_app_state(&router.url(), 0);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;
    let body = basic_responses_body();

    for _ in 0..2 {
        let req = test::TestRequest::post()
            .uri("/v1/responses")
            .insert_header(("Authorization", "Bearer router-test"))
            .set_json(&body)
            .to_request();
        let _ = test::call_service(&app, req).await;
    }

    assert_eq!(
        router.calls(),
        2,
        "ROUTIIUM_CACHE_TTL_MS=0 must disable plan caching even if the router returns a TTL"
    );
    clear_router_env();
}
