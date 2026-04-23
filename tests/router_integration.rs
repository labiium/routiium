use actix_web::{test, web, App};
use http::StatusCode as RouterStatusCode;
use once_cell::sync::Lazy;
use reqwest::Client;
use routiium::{
    router_client::{
        CachedRouterClient, HttpRouterClient, HttpRouterConfig, JudgeMetadata, RouterClient,
    },
    server::config_routes,
    util::AppState,
};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[path = "common/router_stub.rs"]
mod router_stub;

use router_stub::{sample_plan, RouterResponseConfig, RouterStub};

static ENV_GUARD: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

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

fn clear_router_env() {
    std::env::remove_var("ROUTIIUM_ROUTER_STRICT");
    std::env::remove_var("ROUTIIUM_CACHE_TTL_MS");
    std::env::remove_var("OPENAI_BASE_URL");
}

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
    let base_state = AppState::default();
    AppState {
        router_client: Some(Arc::new(cached) as Arc<dyn RouterClient>),
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

#[actix_web::test]
async fn router_judge_headers_are_forwarded() {
    let _guard = ENV_GUARD.lock().await;
    clear_router_env();
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");

    let mut plan = sample_plan("rte_judged", "gpt-4o-mini");
    plan.judge = Some(JudgeMetadata {
        mode: Some("enforce".to_string()),
        verdict: Some("downgrade".to_string()),
        risk_level: Some("medium".to_string()),
        reason: Some("Use a safer route".to_string()),
        target: Some("gpt-4o-mini".to_string()),
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
