use actix_web::{web, App, HttpServer};
use routiium::auth::ApiKeyManager;
use routiium::mcp_client::McpClientManager;
use routiium::mcp_config::McpConfig;
use routiium::server::config_routes;
use routiium::util::{
    build_http_client_from_env, cors_config_from_env, env_bind_addr, init_tracing, AppState,
};
use std::env;
use std::sync::Arc;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    init_tracing();

    // Parse command line arguments
    let args: Vec<String> = env::args().collect();

    // Initialize key backend and optional MCP config from CLI args
    let backend_opt = ApiKeyManager::backend_from_args(&args);
    let api_keys = match backend_opt {
        Some(backend) => match ApiKeyManager::from_backend(backend) {
            Ok(mgr) => {
                tracing::info!("API key backend initialized from CLI");
                Some(Arc::new(mgr))
            }
            Err(e) => {
                tracing::error!("Failed to initialize API key backend from CLI: {}", e);
                None
            }
        },
        None => {
            // Env-based fallback (Redis URL -> Redis, else sled if available, else memory)
            match ApiKeyManager::new_default() {
                Ok(mgr) => Some(Arc::new(mgr)),
                Err(e) => {
                    tracing::warn!("Falling back to no API key manager: {}", e);
                    None
                }
            }
        }
    };

    // Optional MCP config path via --mcp-config=<path> or ROUTIIUM_MCP_CONFIG
    let mcp_config_arg = args
        .iter()
        .find(|a| a.starts_with("--mcp-config="))
        .and_then(|a| a.strip_prefix("--mcp-config=").map(|s| s.to_string()))
        .or_else(|| {
            env::var("ROUTIIUM_MCP_CONFIG")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });

    // Optional system prompt config path via --system-prompt-config or ROUTIIUM_SYSTEM_PROMPT_CONFIG
    let system_prompt_config_arg = args
        .iter()
        .find(|a| a.starts_with("--system-prompt-config="))
        .and_then(|a| a.strip_prefix("--system-prompt-config="))
        .map(|s| s.to_string())
        .or_else(|| {
            env::var("ROUTIIUM_SYSTEM_PROMPT_CONFIG")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });

    // Optional routing config path via --routing-config or ROUTIIUM_ROUTING_CONFIG
    let routing_config_arg = args
        .iter()
        .find(|a| a.starts_with("--routing-config="))
        .and_then(|a| a.strip_prefix("--routing-config="))
        .map(|s| s.to_string())
        .or_else(|| {
            env::var("ROUTIIUM_ROUTING_CONFIG")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });

    // Check for --router-config flag (alias map for Router)
    let router_config_arg = args
        .iter()
        .find(|a| a.starts_with("--router-config="))
        .and_then(|a| a.strip_prefix("--router-config="))
        .map(|s| s.to_string());

    let (mcp_manager_arc, mcp_config_path) = if let Some(mcp_config_path) = mcp_config_arg.clone() {
        tracing::info!("Loading MCP configuration from: {}", mcp_config_path);
        match McpConfig::load_from_file(&mcp_config_path) {
            Ok(config) => {
                tracing::info!("Found {} MCP servers in config", config.mcp_servers.len());
                match McpClientManager::new(config).await {
                    Ok(manager) => {
                        tracing::info!("Successfully initialized MCP client manager");
                        (
                            Some(Arc::new(tokio::sync::RwLock::new(manager))),
                            Some(mcp_config_path),
                        )
                    }
                    Err(e) => {
                        tracing::error!("Failed to initialize MCP client manager: {}", e);
                        tracing::warn!("Continuing without MCP support");
                        (None, None)
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to load MCP config: {}", e);
                tracing::warn!("Continuing without MCP support");
                (None, None)
            }
        }
    } else {
        tracing::info!("No MCP config provided, running without MCP support");
        tracing::info!(
                "Usage: {} [--mcp-config=mcp.json] [--keys-backend=redis://...|sled:<path>|memory] [--system-prompt-config=system_prompt.json] [--routing-config=routing.json]",
                args[0]
            );
        (None, None)
    };

    // Load system prompt configuration if provided
    let (system_prompt_config, system_prompt_config_path) =
        if let Some(prompt_config_path) = system_prompt_config_arg.clone() {
            tracing::info!(
                "Loading system prompt configuration from: {}",
                prompt_config_path
            );
            match routiium::system_prompt_config::SystemPromptConfig::load_from_file(
                &prompt_config_path,
            ) {
                Ok(config) => {
                    tracing::info!(
                        "System prompt configuration loaded (enabled: {})",
                        config.enabled
                    );
                    (
                        Arc::new(tokio::sync::RwLock::new(config)),
                        Some(prompt_config_path),
                    )
                }
                Err(e) => {
                    tracing::error!("Failed to load system prompt config: {}", e);
                    tracing::warn!("Continuing with default (empty) system prompt config");
                    (
                        Arc::new(tokio::sync::RwLock::new(
                            routiium::system_prompt_config::SystemPromptConfig::empty(),
                        )),
                        None,
                    )
                }
            }
        } else {
            tracing::info!("No system prompt config provided");
            (
                Arc::new(tokio::sync::RwLock::new(
                    routiium::system_prompt_config::SystemPromptConfig::empty(),
                )),
                None,
            )
        };

    // Load routing configuration if provided
    let (routing_config, routing_config_path) =
        if let Some(routing_path) = routing_config_arg.clone() {
            tracing::info!("Loading routing configuration from: {}", routing_path);
            match routiium::routing_config::RoutingConfig::load_from_file(&routing_path) {
                Ok(config) => {
                    tracing::info!(
                        "Routing configuration loaded ({} rules, {} aliases)",
                        config.rules.len(),
                        config.aliases.len()
                    );
                    (
                        Arc::new(tokio::sync::RwLock::new(config)),
                        Some(routing_path),
                    )
                }
                Err(e) => {
                    tracing::error!("Failed to load routing config: {}", e);
                    tracing::warn!("Continuing with empty routing config");
                    (
                        Arc::new(tokio::sync::RwLock::new(
                            routiium::routing_config::RoutingConfig::empty(),
                        )),
                        None,
                    )
                }
            }
        } else {
            tracing::info!("No routing config provided, using legacy ROUTIIUM_BACKENDS");
            (
                Arc::new(tokio::sync::RwLock::new(
                    routiium::routing_config::RoutingConfig::empty(),
                )),
                None,
            )
        };

    // Load router configuration if provided
    let mut router_config_path_state: Option<String> = None;
    let mut router_url_state: Option<String> = None;
    let router_client: Option<Arc<dyn routiium::router_client::RouterClient>> =
        if let Some(router_path) = router_config_arg.clone() {
            tracing::info!("Loading router configuration from: {}", router_path);
            match routiium::router_client::LocalPolicyRouter::from_file(&router_path) {
                Ok(router) => {
                    tracing::info!("Router configuration loaded (local policy)");
                    router_config_path_state = Some(router_path);
                    Some(Arc::new(router))
                }
                Err(e) => {
                    tracing::error!("Failed to load router config: {}", e);
                    tracing::warn!("Continuing without router");
                    None
                }
            }
        } else if let Ok(router_url) = env::var("ROUTIIUM_ROUTER_URL") {
            tracing::info!("Connecting to remote router: {}", router_url);
            let timeout_ms = env::var("ROUTIIUM_ROUTER_TIMEOUT_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(15);

            let config = routiium::router_client::HttpRouterConfig {
                url: router_url.clone(),
                timeout_ms,
                mtls: env::var("ROUTIIUM_ROUTER_MTLS").is_ok(),
                client: None,
            };

            match routiium::router_client::HttpRouterClient::new(config) {
                Ok(client) => {
                    tracing::info!("Connected to remote router");
                    router_url_state = Some(router_url);
                    // Wrap with cache
                    let cache_ttl = env::var("ROUTIIUM_CACHE_TTL_MS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(15000);
                    Some(Arc::new(routiium::router_client::CachedRouterClient::new(
                        Box::new(client),
                        cache_ttl,
                    )))
                }
                Err(e) => {
                    tracing::error!("Failed to connect to router: {}", e);
                    tracing::warn!("Continuing without router");
                    None
                }
            }
        } else {
            tracing::info!("No router configured");
            None
        };

    // Initialize analytics manager
    let analytics = match routiium::analytics::AnalyticsManager::from_env() {
        Ok(mgr) => {
            tracing::info!("Analytics initialized successfully");
            Some(Arc::new(mgr))
        }
        Err(e) => {
            tracing::warn!("Analytics initialization failed: {}", e);
            tracing::info!("Continuing without analytics");
            None
        }
    };

    // Initialize chat history manager
    let chat_history = {
        let config = routiium::chat_history_manager::ChatHistoryConfig::from_env();
        if config.enabled {
            match routiium::chat_history_manager::ChatHistoryManager::new(config).await {
                Ok(mgr) => {
                    tracing::info!("Chat history initialized successfully");
                    Some(Arc::new(mgr))
                }
                Err(e) => {
                    tracing::warn!("Chat history initialization failed: {}", e);
                    tracing::info!("Continuing without chat history");
                    None
                }
            }
        } else {
            tracing::info!("Chat history is disabled");
            None
        }
    };

    // Initialize rate limit manager
    // Check for --rate-limit-config CLI flag first, then ROUTIIUM_RATE_LIMIT_CONFIG env var.
    let rate_limit_config_arg = args
        .iter()
        .find(|a| a.starts_with("--rate-limit-config="))
        .and_then(|a| {
            a.strip_prefix("--rate-limit-config=")
                .map(|s| s.to_string())
        })
        .or_else(|| env::var("ROUTIIUM_RATE_LIMIT_CONFIG").ok());

    let rate_limit_manager = {
        let enabled = env::var("ROUTIIUM_RATE_LIMIT_ENABLED")
            .map(|v| v.trim().to_lowercase())
            .map(|v| v == "true" || v == "1" || v == "yes")
            .unwrap_or(true); // enabled by default when a config or backend is provided

        // Only initialise when explicitly configured or when a backend env is set.
        let has_backend = env::var("ROUTIIUM_RATE_LIMIT_BACKEND").is_ok()
            || env::var("ROUTIIUM_REDIS_URL").is_ok();
        let has_config = rate_limit_config_arg.is_some();

        if enabled && (has_backend || has_config) {
            match routiium::rate_limit::RateLimitManager::from_env() {
                Ok(mgr) => {
                    let config_path = rate_limit_config_arg.clone();
                    let mgr = mgr.with_config_path(config_path.clone());
                    let mgr = std::sync::Arc::new(mgr);
                    if let Some(path) = config_path {
                        tracing::info!("Loading rate limit config from: {}", path);
                        if let Err(e) = mgr.load_file_config(&path).await {
                            tracing::warn!("Failed to load rate limit config: {}", e);
                        }
                    }
                    // Apply default env-based policy if present and no file config set it.
                    if let Some(default_policy) = routiium::rate_limit::default_policy_from_env() {
                        if let Err(e) = mgr.create_policy(default_policy.clone()).await {
                            tracing::warn!(
                                "Failed to register default env rate limit policy: {}",
                                e
                            );
                        } else {
                            let _ = mgr.set_default_policy(&default_policy.id).await;
                        }
                    }
                    tracing::info!("Rate limiting initialized");
                    Some(mgr)
                }
                Err(e) => {
                    tracing::warn!("Rate limit manager initialization failed: {}", e);
                    tracing::info!("Continuing without rate limiting");
                    None
                }
            }
        } else {
            tracing::info!("Rate limiting disabled (set ROUTIIUM_RATE_LIMIT_ENABLED=true or provide a backend/config to enable)");
            None
        }
    };

    // Load pricing configuration
    let mut pricing_config_path_state = None;
    let pricing = if let Ok(pricing_path) = env::var("ROUTIIUM_PRICING_CONFIG") {
        let path = pricing_path.trim();
        if !path.is_empty() {
            tracing::info!("Loading pricing configuration from: {}", path);
            match routiium::pricing::PricingConfig::load_from_file(path) {
                Ok(config) => {
                    tracing::info!("Pricing configuration loaded");
                    pricing_config_path_state = Some(path.to_string());
                    Arc::new(config)
                }
                Err(e) => {
                    tracing::warn!("Failed to load pricing config: {}, using defaults", e);
                    Arc::new(routiium::pricing::PricingConfig::default())
                }
            }
        } else {
            tracing::info!("Using default OpenAI pricing");
            Arc::new(routiium::pricing::PricingConfig::default())
        }
    } else {
        tracing::info!("Using default OpenAI pricing");
        Arc::new(routiium::pricing::PricingConfig::default())
    };

    let app_state = AppState {
        http: build_http_client_from_env(),
        mcp_manager: mcp_manager_arc,
        api_keys,
        system_prompt_config,
        analytics,
        chat_history,
        pricing,
        pricing_config_path: pricing_config_path_state,
        mcp_config_path,
        system_prompt_config_path,
        routing_config,
        routing_config_path,
        router_client,
        router_config_path: router_config_path_state,
        router_url: router_url_state,
        rate_limit_manager,
    };

    // Startup mode announcement (managed vs passthrough)
    let managed_override = std::env::var("ROUTIIUM_MANAGED_MODE")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let managed_mode = routiium::util::managed_mode_from_env();
    if managed_mode {
        if let Some(raw) = managed_override {
            tracing::info!(
                "Auth mode: managed (override via ROUTIIUM_MANAGED_MODE={})",
                raw
            );
        } else {
            tracing::info!("Auth mode: managed (OPENAI_API_KEY present)");
        }
    } else if let Some(raw) = managed_override {
        tracing::info!(
            "Auth mode: passthrough (override via ROUTIIUM_MANAGED_MODE={})",
            raw
        );
    } else {
        tracing::info!("Auth mode: passthrough (no OPENAI_API_KEY)");
    }

    let addr = env_bind_addr();
    tracing::info!("Routiium listening on http://{}", addr);

    let app_state_data = web::Data::new(app_state);

    HttpServer::new(move || {
        let cors = cors_config_from_env();

        App::new()
            .wrap(cors)
            .app_data(app_state_data.clone())
            .configure(config_routes)
    })
    .bind(&addr)?
    .run()
    .await
}
