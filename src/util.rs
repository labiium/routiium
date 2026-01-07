use actix_web::HttpResponse;
use http::StatusCode;
use tracing_subscriber::{fmt, EnvFilter};

/// Initialize dotenv and structured tracing based on RUST_LOG.
/// Enhanced:
/// - Supports explicit env file paths via ENV_FILE, ENVFILE, DOTENV_PATH
/// - Falls back to .envfile, then default .env
/// - If all fail, tries a tolerant manual parser for ./.env (no overwrite of existing vars)
/// - Logs the source used
pub fn init_tracing() {
    // Try explicit environment file variables first
    let mut env_source: String = "none".into();
    for key in ["ENV_FILE", "ENVFILE", "DOTENV_PATH"] {
        if let Ok(p) = std::env::var(key) {
            let p = p.trim();
            if !p.is_empty()
                && std::path::Path::new(p).is_file()
                && dotenvy::from_filename(p).is_ok()
            {
                env_source = format!("{p} ({key})");
                break;
            }
        }
    }

    // Next, support conventional ".envfile"
    if env_source == "none"
        && std::path::Path::new(".envfile").is_file()
        && dotenvy::from_filename(".envfile").is_ok()
    {
        env_source = ".envfile".into();
    }

    // Default to standard ".env" discovery in current working directory
    if env_source == "none" && dotenvy::dotenv().is_ok() {
        env_source = ".env".into();
    }

    // If still not found, search upward from the executable directory for a .env file.
    if env_source == "none" {
        if let Ok(exe) = std::env::current_exe() {
            let mut dir_opt = exe.parent();
            while let Some(dir) = dir_opt {
                let candidate = dir.join(".env");
                if candidate.is_file() && dotenvy::from_filename(&candidate).is_ok() {
                    env_source = candidate.display().to_string();
                    break;
                }
                dir_opt = dir.parent();
            }
        }
    }

    // Tolerant manual parser: if still none, try reading "./.env" and set keys not already present.
    if env_source == "none" {
        if let Ok(cwd) = std::env::current_dir() {
            let candidate = cwd.join(".env");
            if candidate.is_file() {
                if let Ok(text) = std::fs::read_to_string(&candidate) {
                    let mut loaded = 0usize;
                    for raw in text.lines() {
                        let line = raw.trim();
                        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
                            continue;
                        }
                        let mut parts = line.splitn(2, '=');
                        if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                            let key = k.trim();
                            if key.is_empty() || std::env::var_os(key).is_some() {
                                continue; // don't overwrite existing env
                            }
                            let mut val = v.trim().to_string();
                            // Strip surrounding single or double quotes if present
                            if (val.starts_with('"') && val.ends_with('"'))
                                || (val.starts_with('\'') && val.ends_with('\''))
                            {
                                val = val[1..val.len().saturating_sub(1)].to_string();
                            }
                            std::env::set_var(key, val);
                            loaded += 1;
                        }
                    }
                    if loaded > 0 {
                        env_source = format!("{} (manual)", candidate.display());
                    }
                }
            }
        }
    }

    // Initialize tracing (respects RUST_LOG potentially provided by the env file)
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info,tower_http=info".into());
    let subscriber = fmt().with_env_filter(EnvFilter::new(filter)).finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    // Log where the environment was loaded from for observability
    tracing::info!("Environment loaded from: {}", env_source);
}

/// Get the bind address for the HTTP server from env or default to 0.0.0.0:8088.
pub fn env_bind_addr() -> String {
    std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8088".into())
}

/// Upstream selection for the proxy:
/// - Responses: talk to /v1/responses with Responses-shaped payload
/// - Chat: talk to /v1/chat/completions with Chat-shaped payload (vLLM, Ollama, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamMode {
    Responses,
    Chat,
    Bedrock,
}

/// Read ROUTIIUM_UPSTREAM_MODE from env ("responses" | "chat" | "bedrock"), default "responses".
pub fn upstream_mode_from_env() -> UpstreamMode {
    match std::env::var("ROUTIIUM_UPSTREAM_MODE")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "chat" => UpstreamMode::Chat,
        "bedrock" => UpstreamMode::Bedrock,
        _ => UpstreamMode::Responses,
    }
}

/// If mode == Chat and the given URL ends with "/responses", rewrite to "/chat/completions".
pub fn rewrite_responses_url_for_mode(url: &str, mode: UpstreamMode) -> String {
    if matches!(mode, UpstreamMode::Chat) && url.ends_with("/responses") {
        let base = &url[..url.len() - "/responses".len()];
        format!("{base}/chat/completions")
    } else {
        url.to_string()
    }
}

/// Shared application state used by the HTTP server and handlers.
pub struct AppState {
    pub http: reqwest::Client,
    pub mcp_manager:
        Option<std::sync::Arc<tokio::sync::RwLock<crate::mcp_client::McpClientManager>>>,
    /// Optional API key manager for inbound auth (generation/expiration/revocation handled in crate::auth)
    pub api_keys: Option<std::sync::Arc<crate::auth::ApiKeyManager>>,
    /// System prompt configuration with runtime reload support
    pub system_prompt_config:
        std::sync::Arc<tokio::sync::RwLock<crate::system_prompt_config::SystemPromptConfig>>,
    /// Analytics manager for tracking request metrics
    pub analytics: Option<std::sync::Arc<crate::analytics::AnalyticsManager>>,
    /// Pricing configuration for cost calculation
    pub pricing: std::sync::Arc<crate::pricing::PricingConfig>,
    /// Path to MCP config file for reload operations
    pub mcp_config_path: Option<String>,
    /// Path to system prompt config file for reload operations
    pub system_prompt_config_path: Option<String>,
    /// Routing configuration with runtime reload support
    pub routing_config: std::sync::Arc<tokio::sync::RwLock<crate::routing_config::RoutingConfig>>,
    /// Path to routing config file for reload operations
    pub routing_config_path: Option<String>,
    /// Router client for model routing decisions
    pub router_client: Option<std::sync::Arc<dyn crate::router_client::RouterClient>>,
}

/// Build an HTTP client honoring proxy and timeout environment variables.
///
/// Environment:
/// - ROUTIIUM_NO_PROXY = 1|true|yes|on  -> disable all proxies
/// - ROUTIIUM_PROXY_URL = <url>         -> proxy for all schemes
/// - HTTP_PROXY / http_proxy                 -> HTTP proxy
/// - HTTPS_PROXY / https_proxy               -> HTTPS proxy
/// - ROUTIIUM_HTTP_TIMEOUT_SECONDS      -> overall request timeout (u64)
pub fn build_http_client_from_env() -> reqwest::Client {
    let mut builder = reqwest::Client::builder();

    // Optional timeout
    if let Ok(secs) = std::env::var("ROUTIIUM_HTTP_TIMEOUT_SECONDS") {
        if let Ok(n) = secs.trim().parse::<u64>() {
            builder = builder.timeout(std::time::Duration::from_secs(n));
        }
    }

    // Proxy configuration
    let no_proxy = std::env::var("ROUTIIUM_NO_PROXY")
        .map(|v| v.trim().to_ascii_lowercase())
        .map(|v| v == "1" || v == "true" || v == "yes" || v == "on")
        .unwrap_or(false);

    if no_proxy {
        builder = builder.no_proxy();
    } else {
        // All-scheme proxy
        if let Ok(url) = std::env::var("ROUTIIUM_PROXY_URL") {
            let u = url.trim();
            if !u.is_empty() {
                if let Ok(p) = reqwest::Proxy::all(u) {
                    builder = builder.proxy(p);
                }
            }
        }
        // Scheme-specific proxies
        if let Ok(http_p) = std::env::var("HTTP_PROXY").or_else(|_| std::env::var("http_proxy")) {
            let u = http_p.trim();
            if !u.is_empty() {
                if let Ok(p) = reqwest::Proxy::http(u) {
                    builder = builder.proxy(p);
                }
            }
        }
        if let Ok(https_p) = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("https_proxy"))
        {
            let u = https_p.trim();
            if !u.is_empty() {
                if let Ok(p) = reqwest::Proxy::https(u) {
                    builder = builder.proxy(p);
                }
            }
        }
    }

    // User-Agent for observability
    builder = builder.user_agent(format!("routiium/{}", env!("CARGO_PKG_VERSION")));

    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            http: build_http_client_from_env(),
            mcp_manager: None,
            api_keys: (|| {
                if let Ok(url) = std::env::var("ROUTIIUM_REDIS_URL") {
                    let u = url.trim().to_string();
                    if !u.is_empty() {
                        if let Ok(m) = crate::auth::ApiKeyManager::new_with_redis_url(&u) {
                            return Some(std::sync::Arc::new(m));
                        }
                    }
                }
                crate::auth::ApiKeyManager::new_default()
                    .ok()
                    .map(std::sync::Arc::new)
            })(),
            system_prompt_config: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::system_prompt_config::SystemPromptConfig::empty(),
            )),
            analytics: crate::analytics::AnalyticsManager::from_env()
                .ok()
                .map(std::sync::Arc::new),
            pricing: std::sync::Arc::new(crate::pricing::PricingConfig::default()),
            mcp_config_path: None,
            system_prompt_config_path: None,
            routing_config: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::routing_config::RoutingConfig::empty(),
            )),
            routing_config_path: None,
            router_client: None,
        }
    }
}

impl AppState {
    /// Create AppState with MCP manager
    pub fn with_mcp_manager(mcp_manager: crate::mcp_client::McpClientManager) -> Self {
        Self {
            http: build_http_client_from_env(),
            mcp_manager: Some(std::sync::Arc::new(tokio::sync::RwLock::new(mcp_manager))),
            api_keys: (|| {
                if let Ok(url) = std::env::var("ROUTIIUM_REDIS_URL") {
                    let u = url.trim().to_string();
                    if !u.is_empty() {
                        if let Ok(m) = crate::auth::ApiKeyManager::new_with_redis_url(&u) {
                            return Some(std::sync::Arc::new(m));
                        }
                    }
                }
                crate::auth::ApiKeyManager::new_default()
                    .ok()
                    .map(std::sync::Arc::new)
            })(),
            system_prompt_config: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::system_prompt_config::SystemPromptConfig::empty(),
            )),
            analytics: crate::analytics::AnalyticsManager::from_env()
                .ok()
                .map(std::sync::Arc::new),
            pricing: std::sync::Arc::new(crate::pricing::PricingConfig::default()),
            mcp_config_path: None,
            system_prompt_config_path: None,
            routing_config: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::routing_config::RoutingConfig::empty(),
            )),
            routing_config_path: None,
            router_client: None,
        }
    }

    /// Create AppState with MCP manager wrapped in Arc
    /// Create AppState with MCP manager Arc
    pub fn with_mcp_manager_arc(
        mcp_manager: std::sync::Arc<tokio::sync::RwLock<crate::mcp_client::McpClientManager>>,
    ) -> Self {
        Self {
            http: build_http_client_from_env(),
            mcp_manager: Some(mcp_manager),
            api_keys: (|| {
                if let Ok(url) = std::env::var("ROUTIIUM_REDIS_URL") {
                    let u = url.trim().to_string();
                    if !u.is_empty() {
                        if let Ok(m) = crate::auth::ApiKeyManager::new_with_redis_url(&u) {
                            return Some(std::sync::Arc::new(m));
                        }
                    }
                }
                crate::auth::ApiKeyManager::new_default()
                    .ok()
                    .map(std::sync::Arc::new)
            })(),
            system_prompt_config: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::system_prompt_config::SystemPromptConfig::empty(),
            )),
            analytics: crate::analytics::AnalyticsManager::from_env()
                .ok()
                .map(std::sync::Arc::new),
            pricing: std::sync::Arc::new(crate::pricing::PricingConfig::default()),
            mcp_config_path: None,
            system_prompt_config_path: None,
            routing_config: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::routing_config::RoutingConfig::empty(),
            )),
            routing_config_path: None,
            router_client: None,
        }
    }
    /// Read the OpenAI API key from environment if present. Optional for /proxy.
    pub fn api_key(&self) -> String {
        std::env::var("OPENAI_API_KEY").unwrap_or_default()
    }

    /// Verify incoming Authorization: Bearer header against the API key manager (if configured).
    /// Returns None if no manager was configured.
    pub fn verify_bearer_header(
        &self,
        headers: &http::HeaderMap,
    ) -> Option<crate::auth::Verification> {
        let auth = headers
            .get(http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        self.api_keys
            .as_ref()
            .map(|m| crate::auth::verify_bearer(m.as_ref(), auth))
    }
}

/// Build a JSON error response with the given HTTP status and message.
pub fn error_response(status: StatusCode, msg: &str) -> HttpResponse {
    let body = serde_json::json!({ "error": { "message": msg } });
    HttpResponse::build(actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap()).json(body)
}

/// Resolve the OpenAI base URL from environment or use the default public endpoint.
pub fn openai_base_url() -> String {
    match std::env::var("OPENAI_BASE_URL") {
        Ok(val) if !val.trim().is_empty() => val,
        _ => {
            static LOGGED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
            LOGGED.get_or_init(|| {
                tracing::warn!("OPENAI_BASE_URL not set; defaulting to https://api.openai.com/v1");
            });
            "https://api.openai.com/v1".into()
        }
    }
}

/// Forward a request upstream with streaming enabled and return an SSE response.
///
/// Behavior:
/// - Default: POST to `{base_url}/responses` with Responses-shaped payload.
/// - If ROUTIIUM_UPSTREAM_MODE=chat: rewrite to `/chat/completions` and translate payload to Chat.
///   This enables vLLM/Ollama upstreams while keeping a Responses surface.
///
/// Note: For very long streams, consider a true streaming passthrough.
pub async fn sse_proxy_stream(
    client: &reqwest::Client,
    url: &str,
    payload: &serde_json::Value,
) -> Result<HttpResponse, anyhow::Error> {
    use bytes::Bytes;
    use futures_util::TryStreamExt;
    use http::header;

    let api_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|s| !s.is_empty());

    // Compute upstream URL and payload according to mode
    let mode = upstream_mode_from_env();
    let real_url = rewrite_responses_url_for_mode(url, mode);
    let mut body = payload.clone();
    if matches!(mode, UpstreamMode::Chat) {
        // Translate Responses-shaped input to Chat request
        let chat_req = crate::conversion::responses_json_to_chat_request(&body);
        if let Ok(v) = serde_json::to_value(chat_req) {
            body = v;
        }
    }

    let mut rb = client
        .post(&real_url)
        .header(header::ACCEPT, "text/event-stream")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONNECTION, "close")
        .json(&body);
    if let Some(k) = api_key {
        if !k.is_empty() {
            rb = rb.bearer_auth(k);
        }
    }
    let resp = rb.send().await?;

    let status = resp.status();
    if !status.is_success() {
        let bytes = resp.bytes().await.unwrap_or_default();
        return Ok(HttpResponse::build(
            actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap(),
        )
        .body(bytes));
    }

    // Stream SSE passthrough without buffering the entire response.
    let upstream_ct = resp.headers().get(header::CONTENT_TYPE).cloned();
    let stream = resp
        .bytes_stream()
        .map_err(|e| std::io::Error::other(e.to_string()))
        .map_ok(Bytes::from);

    let mut response = HttpResponse::Ok();
    if let Some(ct) = upstream_ct {
        if let Ok(ct_str) = ct.to_str() {
            response.insert_header(("content-type", ct_str));
        } else {
            response.insert_header(("content-type", "text/event-stream"));
        }
    } else {
        response.insert_header(("content-type", "text/event-stream"));
    }

    Ok(response
        .insert_header(("cache-control", "no-cache"))
        .insert_header(("connection", "keep-alive"))
        .streaming(stream))
}

/// Multi-backend routing based on model prefixes declared in ROUTIIUM_BACKENDS.
/// Format (semicolon-separated rules; commas within a rule):
///   ROUTIIUM_BACKENDS="prefix=gpt-;base=https://api.openai.com/v1;key_env=OPENAI_API_KEY;mode=responses; prefix=claude-;base=https://api.anthropic.com/v1;key_env=ANTHROPIC_API_KEY;mode=responses; prefix=llama;base=http://localhost:11434/v1;mode=chat"
///
/// Notes:
/// - prefix: string matched with starts_with against the request's model
/// - base: provider base URL (include /v1 if required by provider)
/// - key_env (optional): env var name that holds the upstream API key for this provider
/// - mode (optional): "responses" or "chat" (defaults to ROUTIIUM_UPSTREAM_MODE or "responses")
#[derive(Debug, Clone)]
struct BackendRule {
    prefix: String,
    base_url: String,
    key_env: Option<String>,
    mode: Option<UpstreamMode>,
}

#[derive(Debug, Clone)]
struct ResolvedBackend {
    base_url: String,
    key_env: Option<String>,
    mode: UpstreamMode,
}

fn parse_mode(s: &str) -> Option<UpstreamMode> {
    let v = s.trim().to_ascii_lowercase();
    match v.as_str() {
        "chat" => Some(UpstreamMode::Chat),
        "responses" => Some(UpstreamMode::Responses),
        "bedrock" => Some(UpstreamMode::Bedrock),
        _ => None,
    }
}

fn backends_from_env() -> Vec<BackendRule> {
    let mut out = Vec::new();
    let raw = match std::env::var("ROUTIIUM_BACKENDS") {
        Ok(s) => s,
        Err(_) => return out,
    };
    for rule_raw in raw.split(';') {
        let r = rule_raw.trim();
        if r.is_empty() {
            continue;
        }
        // Support both "k=v,k=v" and "k=v; k=v" styles by normalizing separators to commas first.
        let parts = r
            .split([',', ';'])
            .map(|p| p.trim())
            .filter(|p| !p.is_empty());
        let mut prefix: Option<String> = None;
        let mut base: Option<String> = None;
        let mut key_env: Option<String> = None;
        let mut mode: Option<UpstreamMode> = None;

        for kv in parts {
            let mut it = kv.splitn(2, '=');
            let k = it.next().unwrap_or("").trim().to_ascii_lowercase();
            let v = it.next().unwrap_or("").trim().to_string();
            if v.is_empty() {
                continue;
            }
            match k.as_str() {
                "prefix" => prefix = Some(v),
                "base" | "base_url" => base = Some(v),
                "key_env" | "api_key_env" => key_env = Some(v),
                "mode" => mode = parse_mode(&v),
                _ => {}
            }
        }

        if let (Some(pfx), Some(bu)) = (prefix, base) {
            out.push(BackendRule {
                prefix: pfx,
                base_url: bu,
                key_env,
                mode,
            });
        }
    }
    out
}

fn resolve_backend_for_model(model: Option<&str>) -> Option<ResolvedBackend> {
    let model = model?;
    let rules = backends_from_env();
    for rule in rules {
        if model.starts_with(rule.prefix.as_str()) {
            let mode = rule.mode.unwrap_or_else(upstream_mode_from_env);
            return Some(ResolvedBackend {
                base_url: rule.base_url,
                key_env: rule.key_env,
                mode,
            });
        }
    }
    None
}

/// Streaming helper that auto-resolves backend based on model from payload.
/// Honors explicit bearer argument; if absent, uses provider-specific key_env or OPENAI_API_KEY.
/// Endpoint path is selected by provider rule.mode (or ROUTIIUM_UPSTREAM_MODE).
pub async fn sse_proxy_stream_with_bearer_routed(
    client: &reqwest::Client,
    payload: &serde_json::Value,
    bearer: Option<&str>,
) -> Result<HttpResponse, anyhow::Error> {
    use bytes::Bytes;
    use futures_util::TryStreamExt;
    use http::header;

    // Extract model to resolve backend
    let model = payload.get("model").and_then(|v| v.as_str()).or({
        // Try Responses-style input.messages[...].model is not standard; model is required.
        None
    });

    let resolved = resolve_backend_for_model(model);

    // Choose base URL and mode
    let (base_url, mode, key_env) = if let Some(r) = resolved {
        (r.base_url, r.mode, r.key_env)
    } else {
        (
            openai_base_url(),
            upstream_mode_from_env(),
            Some("OPENAI_API_KEY".to_string()),
        )
    };

    // Build endpoint according to mode
    let endpoint = match mode {
        UpstreamMode::Responses => "/responses",
        UpstreamMode::Chat => "/chat/completions",
        UpstreamMode::Bedrock => "/bedrock/invoke", // Not actually used for Bedrock (uses AWS SDK)
    };
    let real_url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        endpoint.trim_start_matches('/')
    );

    // Translate payload if mode = Chat (vLLM/Ollama path)
    let mut body = payload.clone();
    if matches!(mode, UpstreamMode::Chat) {
        let chat_req = crate::conversion::responses_json_to_chat_request(&body);
        if let Ok(v) = serde_json::to_value(chat_req) {
            body = v;
        }
    }

    // Determine effective bearer: explicit > provider key_env > OPENAI_API_KEY
    let effective_bearer = bearer
        .and_then(|b| {
            let t = b.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        })
        .or_else(|| {
            key_env
                .as_deref()
                .and_then(|k| std::env::var(k).ok())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|s| !s.is_empty())
        });

    // Build upstream request
    let mut rb = client
        .post(&real_url)
        .header(header::ACCEPT, "text/event-stream")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONNECTION, "close")
        .json(&body);
    if let Some(k) = &effective_bearer {
        rb = rb.bearer_auth(k);
    }
    let resp = rb.send().await?;

    let status = resp.status();
    if !status.is_success() {
        let bytes = resp.bytes().await.unwrap_or_default();
        return Ok(HttpResponse::build(
            actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap(),
        )
        .body(bytes));
    }

    // Stream SSE passthrough without buffering the entire response.
    let upstream_ct = resp.headers().get(header::CONTENT_TYPE).cloned();
    let stream = resp
        .bytes_stream()
        .map_err(|e| std::io::Error::other(e.to_string()))
        .map_ok(Bytes::from);

    let mut response = HttpResponse::Ok();
    if let Some(ct) = upstream_ct {
        if let Ok(ct_str) = ct.to_str() {
            response.insert_header(("content-type", ct_str));
        } else {
            response.insert_header(("content-type", "text/event-stream"));
        }
    } else {
        response.insert_header(("content-type", "text/event-stream"));
    }

    Ok(response
        .insert_header(("cache-control", "no-cache"))
        .insert_header(("connection", "keep-alive"))
        .streaming(stream))
}

/// Best-effort derivation of a single-string "input" from a Responses-shaped payload.
fn derive_input_string(payload: &serde_json::Value) -> String {
    // Prefer last user message text; else concatenate text-like parts; else empty string.
    let mut derived: Option<String> = None;

    if let Some(msgs) = payload.get("messages").and_then(|m| m.as_array()) {
        // Pick last user; else last message
        let mut candidate = None;
        for m in msgs.iter().rev() {
            if let Some(role) = m.get("role").and_then(|r| r.as_str()) {
                if role == "user" {
                    candidate = Some(m);
                    break;
                }
            }
            if candidate.is_none() {
                candidate = Some(m);
            }
        }
        if let Some(m) = candidate {
            if let Some(content) = m.get("content") {
                match content {
                    serde_json::Value::String(s) => derived = Some(s.clone()),
                    serde_json::Value::Array(parts) => {
                        let mut pieces = Vec::new();
                        for p in parts {
                            if let Some(ty) = p.get("type").and_then(|t| t.as_str()) {
                                if ty == "text" || ty == "input_text" {
                                    if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                                        pieces.push(t.to_string());
                                    }
                                }
                            }
                        }
                        if !pieces.is_empty() {
                            derived = Some(pieces.join("\n"));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    derived.unwrap_or_default()
}

/// Non-streaming POST helper with a single retry when upstream requires top-level 'input'.
///
/// Behavior:
/// - Sends JSON payload as-is with Bearer auth.
/// - If 400 and the response body hints that 'input' is required, derive an 'input' string
///   from messages and retry once with that field added.
/// - Returns the upstream response (first or retried).
pub async fn post_responses_with_input_retry(
    client: &reqwest::Client,
    url: &str,
    payload: &serde_json::Value,
    bearer: Option<String>,
) -> Result<HttpResponse, anyhow::Error> {
    let effective_bearer = bearer
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
        .or_else(|| {
            std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|s| !s.is_empty())
        });
    let mut req = client
        .post(url)
        .header(http::header::CONTENT_TYPE, "application/json");
    if let Some(key) = effective_bearer.clone() {
        req = req.bearer_auth(key);
    }
    let first = req.try_clone().unwrap().json(payload).send().await?;
    let status = first.status();
    if status != http::StatusCode::BAD_REQUEST {
        let bytes = first.bytes().await.unwrap_or_default();
        return Ok(HttpResponse::build(
            actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap(),
        )
        .body(bytes));
    }
    let body_bytes = first.bytes().await.unwrap_or_default();
    let body_text = String::from_utf8_lossy(&body_bytes);
    // Heuristic: look for "input" and "missing" in the error text.
    let needs_input = body_text.contains("'input'")
        || body_text.contains("\"input\"")
        || body_text.contains("Field required") && body_text.contains("input");
    if !needs_input {
        return Ok(HttpResponse::build(
            actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap(),
        )
        .body(body_bytes));
    }

    // Retry with derived input injected (do not overwrite if already present).
    let mut patched = payload.clone();
    let s = derive_input_string(&patched);
    if let Some(obj) = patched.as_object_mut() {
        if !obj.contains_key("input") {
            obj.insert("input".into(), serde_json::Value::String(s));
        }
    }

    let mut second_req = client
        .post(url)
        .header(http::header::CONTENT_TYPE, "application/json");
    if let Some(key) = effective_bearer {
        second_req = second_req.bearer_auth(key);
    }
    let second = second_req.json(&patched).send().await?;

    let status2 = second.status();
    let bytes2 = second.bytes().await.unwrap_or_default();
    Ok(
        HttpResponse::build(actix_web::http::StatusCode::from_u16(status2.as_u16()).unwrap())
            .body(bytes2),
    )
}

/// Simple HTTP GET helper supporting optional Bearer auth and proxy (via provided client).
pub async fn http_get_with_bearer(
    client: &reqwest::Client,
    url: &str,
    bearer: Option<&str>,
) -> Result<HttpResponse, anyhow::Error> {
    let mut rb = client.get(url);
    if let Some(tok) = bearer {
        if !tok.is_empty() {
            rb = rb.bearer_auth(tok);
        }
    }
    let resp = rb.send().await?;
    let status = resp.status();
    let bytes = resp.bytes().await.unwrap_or_default();
    Ok(
        HttpResponse::build(actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap())
            .body(bytes),
    )
}

pub async fn sse_proxy_stream_with_bearer(
    client: &reqwest::Client,
    url: &str,
    payload: &serde_json::Value,
    bearer: Option<&str>,
) -> Result<HttpResponse, anyhow::Error> {
    use bytes::Bytes;
    use futures_util::TryStreamExt;
    use http::header;

    // Compute upstream URL and payload according to mode
    let mode = upstream_mode_from_env();
    let real_url = rewrite_responses_url_for_mode(url, mode);
    let mut body = payload.clone();
    if matches!(mode, UpstreamMode::Chat) {
        // Translate Responses-shaped input to Chat request
        let chat_req = crate::conversion::responses_json_to_chat_request(&body);
        if let Ok(v) = serde_json::to_value(chat_req) {
            body = v;
        }
    }

    // Determine effective bearer (explicit Authorization header overrides env OPENAI_API_KEY fallback)
    let effective_bearer = bearer
        .and_then(|b| {
            let t = b.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        })
        .or_else(|| {
            std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|s| !s.is_empty())
        });

    let has_bearer = effective_bearer.is_some();
    tracing::debug!(
        has_bearer = has_bearer,
        "sse_proxy_stream_with_bearer: preparing upstream request"
    );

    // Build upstream request (closure so we can retry easily)
    let build_req = || {
        let mut b = client
            .post(&real_url)
            .header(header::ACCEPT, "text/event-stream")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONNECTION, "close")
            .json(&body);
        if let Some(k) = &effective_bearer {
            b = b.bearer_auth(k);
        }
        b
    };

    let mut resp_opt: Option<reqwest::Response> = None;
    let mut last_err: Option<anyhow::Error> = None;
    for delay_ms in [0u64, 100, 200, 400] {
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        match build_req().send().await {
            Ok(r) => {
                resp_opt = Some(r);
                break;
            }
            Err(e) => {
                tracing::warn!(error=%e, attempt_delay_ms=%delay_ms, "sse upstream send attempt failed");
                last_err = Some(anyhow::Error::new(e));
                continue;
            }
        }
    }
    let resp = match resp_opt {
        Some(r) => r,
        None => {
            return Err(
                last_err.unwrap_or_else(|| anyhow::anyhow!("upstream streaming request failed"))
            );
        }
    };
    let status = resp.status();
    if !status.is_success() {
        let bytes = resp.bytes().await.unwrap_or_default();
        return Ok(HttpResponse::build(
            actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap(),
        )
        .body(bytes));
    }

    // Stream SSE passthrough without buffering the entire response.
    let upstream_ct = resp.headers().get(header::CONTENT_TYPE).cloned();
    let stream = resp
        .bytes_stream()
        .map_err(|e| std::io::Error::other(e.to_string()))
        .map_ok(Bytes::from);

    let mut response = HttpResponse::Ok();
    if let Some(ct) = upstream_ct {
        if let Ok(ct_str) = ct.to_str() {
            response.insert_header(("content-type", ct_str));
        } else {
            response.insert_header(("content-type", "text/event-stream"));
        }
    } else {
        response.insert_header(("content-type", "text/event-stream"));
    }

    Ok(response
        .insert_header(("cache-control", "no-cache"))
        .insert_header(("connection", "keep-alive"))
        .streaming(stream))
}

/// Build a CORS configuration from environment variables for Actix-web.
///
/// Environment variables:
/// - CORS_ALLOWED_ORIGINS: "*" or comma-separated origins (e.g., "https://a.com, https://b.com")
/// - CORS_ALLOWED_METHODS: "*" or comma-separated methods (e.g., "GET,POST,OPTIONS")
/// - CORS_ALLOWED_HEADERS: "*" or comma-separated request header names
/// - CORS_ALLOW_CREDENTIALS: enable with 1,true,yes,on
/// - CORS_MAX_AGE: max age in seconds (usize)
///
/// Defaults are permissive to match prior behavior when not configured.
pub fn cors_config_from_env() -> actix_cors::Cors {
    let mut cors = actix_cors::Cors::default();

    // Allowed origins
    if let Ok(origins) = std::env::var("CORS_ALLOWED_ORIGINS") {
        let s = origins.trim();
        if s == "*" {
            cors = cors.allow_any_origin();
        } else {
            for part in s.split(',') {
                let p = part.trim();
                if !p.is_empty() {
                    cors = cors.allowed_origin(p);
                }
            }
        }
    } else {
        cors = cors.allow_any_origin();
    }

    // Allowed methods
    if let Ok(methods) = std::env::var("CORS_ALLOWED_METHODS") {
        let s = methods.trim();
        if s == "*" {
            cors = cors.allow_any_method();
        } else {
            let mut methods = Vec::new();
            for part in s.split(',') {
                let p = part.trim();
                if !p.is_empty() {
                    methods.push(p);
                }
            }
            if !methods.is_empty() {
                cors = cors.allowed_methods(methods);
            }
        }
    } else {
        cors = cors.allow_any_method();
    }

    // Allowed headers
    if let Ok(headers) = std::env::var("CORS_ALLOWED_HEADERS") {
        let s = headers.trim();
        if s == "*" {
            cors = cors.allow_any_header();
        } else {
            let mut header_list = Vec::new();
            for part in s.split(',') {
                let p = part.trim();
                if !p.is_empty() {
                    header_list.push(p);
                }
            }
            if !header_list.is_empty() {
                for h in header_list {
                    cors = cors.allowed_header(h);
                }
            }
        }
    } else {
        cors = cors.allow_any_header();
    }

    // Credentials
    if let Ok(val) = std::env::var("CORS_ALLOW_CREDENTIALS") {
        let v = val.trim().to_ascii_lowercase();
        if v == "1" || v == "true" || v == "yes" || v == "on" {
            cors = cors.supports_credentials();
        }
    }

    // Max age
    if let Ok(secs) = std::env::var("CORS_MAX_AGE") {
        if let Ok(n) = secs.trim().parse::<usize>() {
            cors = cors.max_age(n);
        }
    }

    cors
}
