use actix_web::{test, web, App};
use axum::{routing::post, Json, Router};
use once_cell::sync::Lazy;
use routiium::{server::config_routes, util::AppState};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

static ENV_GUARD: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

async fn start_guard_triggering_upstream() -> SocketAddr {
    async fn responses() -> Json<Value> {
        Json(json!({
            "id": "resp_guard_test",
            "object": "response",
            "created_at": 1,
            "model": "guard-test",
            "output_text": "api_key = sk-testkeymaterialthatshouldnotleak1234567890",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "api_key = sk-testkeymaterialthatshouldnotleak1234567890"
                }]
            }]
        }))
    }

    let app = Router::new().route("/v1/responses", post(responses));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

fn clear_env() {
    for key in [
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "ROUTIIUM_MANAGED_MODE",
        "ROUTIIUM_RESPONSE_GUARD",
        "ROUTIIUM_ROUTER_MODE",
        "ROUTIIUM_ADMIN_TOKEN",
        "ROUTIIUM_SAFETY_AUDIT_PATH",
    ] {
        std::env::remove_var(key);
    }
}

#[actix_web::test]
async fn response_guard_block_is_available_in_safety_events() {
    let _guard = ENV_GUARD.lock().await;
    clear_env();
    let upstream_addr = start_guard_triggering_upstream().await;
    let temp_dir = tempfile::tempdir().unwrap();
    let audit_path = temp_dir.path().join("safety.jsonl");

    std::env::set_var("OPENAI_BASE_URL", format!("http://{upstream_addr}/v1"));
    std::env::set_var("ROUTIIUM_MANAGED_MODE", "0");
    std::env::set_var("ROUTIIUM_RESPONSE_GUARD", "protect");
    std::env::set_var("ROUTIIUM_ROUTER_MODE", "off");
    std::env::set_var(
        "ROUTIIUM_ADMIN_TOKEN",
        "test-admin-token-with-enough-entropy",
    );
    std::env::set_var("ROUTIIUM_SAFETY_AUDIT_PATH", &audit_path);

    let app_state = AppState {
        http: reqwest::Client::builder().no_proxy().build().unwrap(),
        ..Default::default()
    };
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(app_state))
            .configure(config_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .insert_header(("Authorization", "Bearer client-test"))
        .set_json(json!({
            "model": "guard-test",
            "input": [{"role": "user", "content": [{"type": "text", "text": "hello"}]}],
            "stream": false
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
    assert_eq!(
        resp.headers()
            .get("x-response-guard-verdict")
            .and_then(|value| value.to_str().ok()),
        Some("deny")
    );
    let body = test::read_body(resp).await;
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"]["code"], "response_guard_blocked");

    let unauth = test::TestRequest::get()
        .uri("/admin/safety/events")
        .to_request();
    let unauth_resp = test::call_service(&app, unauth).await;
    assert_eq!(unauth_resp.status(), 401);

    let events_req = test::TestRequest::get()
        .uri("/admin/safety/events?limit=5")
        .insert_header((
            "Authorization",
            "Bearer test-admin-token-with-enough-entropy",
        ))
        .to_request();
    let events_resp = test::call_service(&app, events_req).await;
    assert_eq!(events_resp.status(), 200);
    let events_body = test::read_body(events_resp).await;
    let events: Value = serde_json::from_slice(&events_body).unwrap();
    assert_eq!(events["count"], 1);
    assert_eq!(events["events"][0]["kind"], "response_guard_block");
    assert_eq!(events["events"][0]["risk_level"], "critical");

    let audit_contents = std::fs::read_to_string(audit_path).unwrap();
    assert!(audit_contents.contains("response_guard_block"));
    clear_env();
}
