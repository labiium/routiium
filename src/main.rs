mod cli;

use actix_web::{web, App, HttpServer};
use cli::{Cli, Command, ServeArgs};
use routiium::auth::ApiKeyManager;
use routiium::mcp_client::McpClientManager;
use routiium::mcp_config::McpConfig;
use routiium::server::config_routes;
use routiium::util::{
    build_http_client_from_env, cors_config_from_env, env_bind_addr, init_tracing_with_env_source,
    load_env_with_config, AppState,
};
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    if env::args_os().any(|arg| {
        matches!(
            arg.to_string_lossy().as_ref(),
            "--help" | "-h" | "--version" | "-V"
        )
    }) {
        let cli = Cli::parse_compat();
        return dispatch(cli, None).await;
    }

    let config_hint = config_arg_from_os_args(env::args_os());
    let env_source = load_env_with_config(config_hint.as_deref());
    let cli = Cli::parse_compat();
    dispatch(cli, Some(env_source)).await
}

fn config_arg_from_os_args<I>(args: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = OsString>,
{
    let mut iter = args.into_iter().skip(1);
    while let Some(arg) = iter.next() {
        let text = arg.to_string_lossy();
        if text == "--config" {
            return iter.next().map(PathBuf::from);
        }
        if let Some(value) = text.strip_prefix("--config=") {
            if !value.trim().is_empty() {
                return Some(PathBuf::from(value));
            }
        }
    }
    None
}

async fn dispatch(cli: Cli, env_source: Option<String>) -> anyhow::Result<()> {
    match cli
        .command
        .unwrap_or_else(|| Command::Serve(ServeArgs::default()))
    {
        Command::Serve(args) => {
            init_tracing_with_env_source(env_source.as_deref().unwrap_or("none"));
            serve(args).await.map_err(Into::into)
        }
        command => cli::run(command).await,
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().trim().to_string()
}

#[derive(Debug, Clone)]
struct RuntimeConfig {
    config_yaml_path: Option<String>,
    keys_backend: Option<String>,
    mcp_config_path: Option<String>,
    system_prompt_config_path: Option<String>,
    routing_config_path: Option<String>,
    router_config_path: Option<String>,
    rate_limit_config_path: Option<String>,
    router_url: Option<String>,
    router_timeout_ms: u64,
    router_mtls: bool,
    router_cache_ttl_ms: u64,
    router_strict: bool,
    router_privacy_mode: String,
}

impl RuntimeConfig {
    fn from_serve_args(args: ServeArgs) -> Self {
        Self {
            config_yaml_path: args
                .config_yaml
                .as_deref()
                .map(path_to_string)
                .filter(|value| !value.is_empty())
                .or_else(|| env_string("ROUTIIUM_CONFIG_YAML")),
            keys_backend: args.keys_backend,
            mcp_config_path: args
                .mcp_config
                .as_deref()
                .map(path_to_string)
                .filter(|value| !value.is_empty()),
            system_prompt_config_path: args
                .system_prompt_config
                .as_deref()
                .map(path_to_string)
                .filter(|value| !value.is_empty()),
            routing_config_path: args
                .routing_config
                .as_deref()
                .map(path_to_string)
                .filter(|value| !value.is_empty()),
            router_config_path: args
                .router_config
                .as_deref()
                .map(path_to_string)
                .filter(|value| !value.is_empty()),
            rate_limit_config_path: args
                .rate_limit_config
                .as_deref()
                .map(path_to_string)
                .filter(|value| !value.is_empty()),
            router_url: env_string("ROUTIIUM_ROUTER_URL"),
            router_timeout_ms: env_string("ROUTIIUM_ROUTER_TIMEOUT_MS")
                .and_then(|s| s.parse().ok())
                .unwrap_or(15),
            router_mtls: env::var_os("ROUTIIUM_ROUTER_MTLS").is_some(),
            router_cache_ttl_ms: env_string("ROUTIIUM_CACHE_TTL_MS")
                .and_then(|s| s.parse().ok())
                .unwrap_or(15_000),
            router_strict: env_truthy("ROUTIIUM_ROUTER_STRICT"),
            router_privacy_mode: env_string("ROUTIIUM_ROUTER_PRIVACY_MODE")
                .unwrap_or_else(|| "features".to_string()),
        }
    }
}

fn env_string(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_truthy(name: &str) -> bool {
    matches!(
        env::var(name)
            .ok()
            .as_deref()
            .map(|v| v.trim().to_ascii_lowercase()),
        Some(ref v) if v == "1" || v == "true" || v == "yes" || v == "on"
    )
}

fn env_falsey_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off" | "disabled" | "legacy"
    )
}

fn embedded_router_enabled() -> bool {
    env_string("ROUTIIUM_ROUTER_MODE")
        .map(|value| !env_falsey_value(&value))
        .unwrap_or(true)
}

fn apply_yaml_server_env(config: &routiium::app_config::CompiledRuntimeConfig) {
    let server = &config.raw.server;
    if env_string("BIND_ADDR").is_none() {
        if let Some(bind_addr) = server
            .bind_addr
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            env::set_var("BIND_ADDR", bind_addr);
        }
    }
    if env_string("ROUTIIUM_MANAGED_MODE").is_none() {
        if let Some(managed_mode) = server.managed_mode {
            env::set_var(
                "ROUTIIUM_MANAGED_MODE",
                if managed_mode { "true" } else { "false" },
            );
        }
    }
    if env_string("ROUTIIUM_HTTP_TIMEOUT_SECONDS").is_none() {
        if let Some(seconds) = server.http_timeout_seconds {
            env::set_var("ROUTIIUM_HTTP_TIMEOUT_SECONDS", seconds.to_string());
        }
    }
    if env_string("ROUTIIUM_ADMIN_TOKEN").is_none() {
        if let Some(env_name) = server
            .admin_token_env
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Some(token) = env_string(env_name) {
                env::set_var("ROUTIIUM_ADMIN_TOKEN", token);
            }
        }
    }
}

async fn serve(args: ServeArgs) -> std::io::Result<()> {
    let runtime_config = RuntimeConfig::from_serve_args(args);

    let yaml_config = match runtime_config.config_yaml_path.as_deref() {
        Some(path) => match routiium::app_config::RoutiiumConfig::load_yaml(path) {
            Ok(config) => {
                tracing::info!(
                    "YAML runtime config loaded from {} ({} aliases)",
                    path,
                    config.alias_count()
                );
                Some(Arc::new(std::sync::RwLock::new(config)))
            }
            Err(err) => {
                tracing::error!("Failed to load YAML runtime config {}: {}", path, err);
                return Err(std::io::Error::other(err.to_string()));
            }
        },
        None => None,
    };
    if let Some(config) = yaml_config.as_ref() {
        if let Ok(config) = config.read() {
            apply_yaml_server_env(&config);
        }
    }

    // Initialize key backend and optional MCP config from CLI args
    let backend_opt = runtime_config
        .keys_backend
        .as_deref()
        .and_then(ApiKeyManager::backend_from_arg_spec);
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
    if routiium::util::managed_mode_from_env() && api_keys.is_none() {
        return Err(std::io::Error::other(
            "managed auth is enabled but the API key manager could not be initialized",
        ));
    }

    // Optional MCP config path via --mcp-config=<path> or ROUTIIUM_MCP_CONFIG
    let mcp_config_arg = runtime_config.mcp_config_path.clone();

    // Optional system prompt config path via --system-prompt-config or ROUTIIUM_SYSTEM_PROMPT_CONFIG
    let system_prompt_config_arg = runtime_config.system_prompt_config_path.clone();

    // Optional routing config path via --routing-config or ROUTIIUM_ROUTING_CONFIG
    let routing_config_arg = runtime_config.routing_config_path.clone();

    // Check for --router-config flag (alias map for Router)
    let router_config_arg = runtime_config.router_config_path.clone();

    let yaml_mcp_config = yaml_config.as_ref().map(|config| {
        config
            .read()
            .ok()
            .and_then(|config| config.mcp_config())
            .unwrap_or_else(|| McpConfig {
                mcp_servers: std::collections::HashMap::new(),
            })
    });
    let (mcp_manager_arc, mcp_config_path) = if let Some(config) = yaml_mcp_config {
        tracing::info!(
            "Loading MCP configuration from YAML runtime config ({} servers)",
            config.mcp_servers.len()
        );
        match McpClientManager::new(config).await {
            Ok(manager) => (
                Some(Arc::new(tokio::sync::RwLock::new(manager))),
                runtime_config.config_yaml_path.clone(),
            ),
            Err(e) => {
                tracing::error!("Failed to initialize YAML MCP client manager: {}", e);
                return Err(std::io::Error::other(format!(
                    "YAML MCP configuration could not be initialized: {e}"
                )));
            }
        }
    } else if let Some(mcp_config_path) = mcp_config_arg.clone() {
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
            "Usage: routiium serve [--mcp-config mcp.json] [--keys-backend redis://...|sled:<path>|memory] [--system-prompt-config system_prompt.json] [--routing-config routing.json]"
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

    // Load router configuration. Explicit local/remote routers retain precedence;
    // otherwise Routiium installs the embedded secure router by default so users
    // get policy-router aliases and request judging without another service.
    let mut router_config_path_state: Option<String> = None;
    let mut router_url_state: Option<String> = None;
    let mut embedded_router_active = false;
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
        } else if let Some(router_url) = runtime_config.router_url.clone() {
            tracing::info!("Connecting to remote router: {}", router_url);

            let config = routiium::router_client::HttpRouterConfig {
                url: router_url.clone(),
                timeout_ms: runtime_config.router_timeout_ms,
                mtls: runtime_config.router_mtls,
                client: None,
            };

            match routiium::router_client::HttpRouterClient::new(config) {
                Ok(client) => {
                    tracing::info!("Connected to remote router");
                    router_url_state = Some(router_url);
                    Some(Arc::new(routiium::router_client::CachedRouterClient::new(
                        Box::new(client),
                        runtime_config.router_cache_ttl_ms,
                    )))
                }
                Err(e) => {
                    tracing::error!("Failed to connect to router: {}", e);
                    tracing::warn!("Continuing without router");
                    None
                }
            }
        } else if embedded_router_enabled() {
            tracing::info!(
                "No router configured; enabling Routiium embedded router + safety judge"
            );
            embedded_router_active = true;
            Some(Arc::new(
                routiium::router_client::EmbeddedDefaultRouter::from_env(),
            ))
        } else {
            tracing::info!(
                "Embedded router disabled by ROUTIIUM_ROUTER_MODE; using legacy routing"
            );
            None
        };

    let effective_router_strict = if embedded_router_active {
        true
    } else {
        runtime_config.router_strict
    };
    let effective_router_cache_ttl_ms = if embedded_router_active {
        0
    } else {
        runtime_config.router_cache_ttl_ms
    };
    let effective_router_privacy_mode =
        if embedded_router_active && env_string("ROUTIIUM_ROUTER_PRIVACY_MODE").is_none() {
            "full".to_string()
        } else {
            runtime_config.router_privacy_mode.clone()
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
    let rate_limit_config_arg = runtime_config.rate_limit_config_path.clone();

    let rate_limit_manager = {
        let enabled = env::var("ROUTIIUM_RATE_LIMIT_ENABLED")
            .map(|v| v.trim().to_lowercase())
            .map(|v| v == "true" || v == "1" || v == "yes")
            .unwrap_or(true); // enabled by default when a config or backend is provided

        // Only initialise when explicitly configured or when a backend env is set.
        let has_backend = env::var("ROUTIIUM_RATE_LIMIT_BACKEND").is_ok()
            || env::var("ROUTIIUM_REDIS_URL").is_ok();
        let has_config = rate_limit_config_arg.is_some();
        let has_yaml_policies = yaml_config
            .as_ref()
            .and_then(|config| {
                config
                    .read()
                    .ok()
                    .map(|config| !config.raw.rate_limit_policies.is_empty())
            })
            .unwrap_or(false);

        if enabled && (has_backend || has_config || has_yaml_policies) {
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
                    if let Some(config) = yaml_config.as_ref().and_then(|config| {
                        config
                            .read()
                            .ok()
                            .map(|config| config.raw.rate_limit_policies.clone())
                    }) {
                        for (id, def) in config {
                            let policy = routiium::rate_limit::RateLimitPolicy {
                                id: id.clone(),
                                buckets: def.buckets,
                            };
                            if let Err(e) = mgr.create_policy(policy).await {
                                tracing::warn!(
                                    "Failed to register YAML rate limit policy {}: {}",
                                    id,
                                    e
                                );
                            }
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
        router_strict: effective_router_strict,
        router_cache_ttl_ms: Some(effective_router_cache_ttl_ms),
        router_privacy_mode: effective_router_privacy_mode,
        rate_limit_manager,
        safety_audit: routiium::safety_audit::SafetyAuditManager::from_env(),
        runtime_config: yaml_config,
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
