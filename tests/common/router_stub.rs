use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use http::StatusCode;
use routiium::router_client::{
    CacheControl, ModelCatalog, PolicyInfo, RouteHints, RouteLimits, RoutePlan, RouteRequest,
    Stickiness, UpstreamConfig, UpstreamMode,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[derive(Clone)]
pub struct RouterStub {
    base_url: String,
    calls: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<RouteRequest>>>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

#[derive(Clone)]
pub enum RouterResponseConfig {
    Plan(Box<RoutePlan>),
    Error {
        status: StatusCode,
        body: serde_json::Value,
    },
}

#[derive(Clone)]
struct StubState {
    response: RouterResponseConfig,
    calls: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<RouteRequest>>>,
}

impl RouterStub {
    pub async fn start(response: RouterResponseConfig) -> Self {
        let calls = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(StubState {
            response,
            calls: calls.clone(),
            requests: requests.clone(),
        });

        let router = Router::new()
            .route("/route/plan", post(plan_handler))
            .route("/catalog/models", get(catalog_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub router");
        let addr = listener.local_addr().expect("stub router local addr");
        let (tx, rx) = oneshot::channel::<()>();

        let server = axum::serve(listener, router.into_make_service());
        tokio::spawn(async move {
            tokio::select! {
                res = server => {
                    if let Err(err) = res {
                        eprintln!("Stub router server error: {err:?}");
                    }
                }
                _ = rx => {}
            }
        });

        RouterStub {
            base_url: format!("http://{}", addr),
            calls,
            requests,
            shutdown: Arc::new(Mutex::new(Some(tx))),
        }
    }

    pub fn url(&self) -> String {
        self.base_url.clone()
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn take_requests(&self) -> Vec<RouteRequest> {
        let mut guard = self.requests.lock().expect("lock stub requests");
        guard.drain(..).collect()
    }
}

impl Drop for RouterStub {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.shutdown.lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.send(());
            }
        }
    }
}

async fn plan_handler(
    State(state): State<Arc<StubState>>,
    Json(req): Json<RouteRequest>,
) -> Result<Json<RoutePlan>, (StatusCode, Json<serde_json::Value>)> {
    state.calls.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut guard) = state.requests.lock() {
        guard.push(req.clone());
    }

    match &state.response {
        RouterResponseConfig::Plan(plan) => Ok(Json((**plan).clone())),
        RouterResponseConfig::Error { status, body } => Err((*status, Json(body.clone()))),
    }
}

async fn catalog_handler() -> Json<ModelCatalog> {
    let catalog = ModelCatalog {
        revision: "stub_v1".to_string(),
        models: vec![],
    };
    Json(catalog)
}

pub fn sample_plan(route_id: &str, model_id: &str) -> RoutePlan {
    RoutePlan {
        schema_version: Some("1.1".to_string()),
        route_id: route_id.to_string(),
        upstream: UpstreamConfig {
            base_url: "http://127.0.0.1:9/v1".to_string(),
            mode: UpstreamMode::Responses,
            model_id: model_id.to_string(),
            auth_env: None,
            headers: Some({
                let mut map = std::collections::HashMap::new();
                map.insert("X-Test-Header".to_string(), "stub".to_string());
                map
            }),
        },
        limits: RouteLimits {
            max_input_tokens: Some(4096),
            max_output_tokens: Some(256),
            timeout_ms: Some(1000),
        },
        prompt_overlays: None,
        hints: RouteHints {
            currency: Some("USD".to_string()),
            tier: Some("T1".to_string()),
            est_cost_micro: Some(1_000),
            ..Default::default()
        },
        fallbacks: vec![],
        cache: Some(CacheControl {
            ttl_ms: 60_000,
            etag: Some("stub".to_string()),
            valid_until: None,
            freeze_key: None,
        }),
        policy_rev: Some("router_policy_v1".to_string()),
        policy: Some(PolicyInfo {
            revision: Some("router_policy_v1".to_string()),
            id: Some("stub_policy".to_string()),
            explain: Some("Stub router response".to_string()),
        }),
        stickiness: Some(Stickiness {
            plan_token: Some("plan_stub".to_string()),
            max_turns: Some(3),
            expires_at: Some("2099-01-01T00:00:00Z".to_string()),
        }),
        content_used: Some("none".to_string()),
        judge: None,
    }
}
