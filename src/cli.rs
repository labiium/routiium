use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use reqwest::header::{HeaderMap, AUTHORIZATION};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "routiium")]
#[command(
    version,
    about = "OpenAI-compatible LLM gateway, router, key manager, and judge edge"
)]
#[command(
    long_about = "Routiium serves an OpenAI-compatible API in front of providers, routers, API keys, rate limits, MCP tools, analytics, and optional LLM-as-judge policy."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

impl Cli {
    pub fn parse_compat() -> Self {
        Self::parse_from_compat(env::args_os())
    }

    pub fn parse_from_compat<I, T>(args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString>,
    {
        Self::parse_from(normalize_legacy_args(args))
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the Routiium HTTP server.
    Serve(ServeArgs),
    /// Generate starter .env/config files for a common deployment profile.
    Init(InitArgs),
    /// Validate local configuration and optional live server/router health.
    Doctor(DoctorArgs),
    /// Fetch /status from a running Routiium server.
    Status(StatusArgs),
    /// Manage Routiium API keys through the admin HTTP API.
    #[command(subcommand, alias = "keys")]
    Key(KeyCommand),
    /// Probe routing behavior through a running Routiium server.
    #[command(subcommand)]
    Router(RouterCommand),
    /// Write LLM-as-judge environment profiles.
    #[command(subcommand)]
    Judge(JudgeCommand),
    /// Print high-level documentation entry points.
    Docs(DocsArgs),
}

#[derive(Debug, Clone, Args, Default)]
pub struct ServeArgs {
    /// API key backend: redis://..., sled:<path>, or memory.
    #[arg(long, value_name = "BACKEND", value_parser = parse_key_backend_spec)]
    pub keys_backend: Option<String>,

    /// MCP server configuration file.
    #[arg(long, value_name = "PATH", env = "ROUTIIUM_MCP_CONFIG")]
    pub mcp_config: Option<PathBuf>,

    /// System prompt configuration file.
    #[arg(long, value_name = "PATH", env = "ROUTIIUM_SYSTEM_PROMPT_CONFIG")]
    pub system_prompt_config: Option<PathBuf>,

    /// Legacy routing configuration file.
    #[arg(long, value_name = "PATH", env = "ROUTIIUM_ROUTING_CONFIG")]
    pub routing_config: Option<PathBuf>,

    /// Local policy router configuration file. Takes precedence over ROUTIIUM_ROUTER_URL.
    #[arg(long, value_name = "PATH", env = "ROUTIIUM_ROUTER_CONFIG")]
    pub router_config: Option<PathBuf>,

    /// Rate limit configuration file.
    #[arg(long, value_name = "PATH", env = "ROUTIIUM_RATE_LIMIT_CONFIG")]
    pub rate_limit_config: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct InitArgs {
    /// Deployment profile to scaffold.
    #[arg(long, value_enum, default_value_t = InitProfile::Openai)]
    pub profile: InitProfile,

    /// Environment file to write.
    #[arg(long, default_value = ".env")]
    pub out: PathBuf,

    /// Directory for generated JSON config files when the profile needs them.
    #[arg(long, default_value = ".")]
    pub config_dir: PathBuf,

    /// Overwrite existing generated files.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum InitProfile {
    Openai,
    Vllm,
    Router,
    Judge,
    Bedrock,
}

impl std::fmt::Display for InitProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Openai => "openai",
            Self::Vllm => "vllm",
            Self::Router => "router",
            Self::Judge => "judge",
            Self::Bedrock => "bedrock",
        })
    }
}

#[derive(Debug, Clone, Args)]
pub struct DoctorArgs {
    /// Routiium base URL to check.
    #[arg(long, default_value = "http://127.0.0.1:8088")]
    pub url: String,

    /// Optional .env file to inspect without loading it into the process environment.
    #[arg(long, default_value = ".env")]
    pub env_file: PathBuf,

    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,

    /// Also check the configured remote router URL if present.
    #[arg(long)]
    pub check_router: bool,

    /// Fail if the Routiium server is not reachable or /status is not successful.
    #[arg(long)]
    pub require_server: bool,
}

#[derive(Debug, Clone, Args)]
pub struct StatusArgs {
    /// Routiium base URL.
    #[arg(long, default_value = "http://127.0.0.1:8088")]
    pub url: String,

    /// Admin token to send as a bearer token when needed by a deployment.
    #[arg(long, env = "ROUTIIUM_ADMIN_TOKEN")]
    pub token: Option<String>,

    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum KeyCommand {
    /// Create an API key.
    Create(KeyCreateArgs),
    /// List API keys.
    List(KeyListArgs),
    /// Revoke an API key by id.
    Revoke(KeyRevokeArgs),
}

#[derive(Debug, Clone, Args)]
pub struct AdminHttpArgs {
    /// Routiium base URL.
    #[arg(long, default_value = "http://127.0.0.1:8088")]
    pub url: String,

    /// Admin bearer token. Defaults to ROUTIIUM_ADMIN_TOKEN.
    #[arg(long, env = "ROUTIIUM_ADMIN_TOKEN")]
    pub admin_token: Option<String>,

    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct KeyCreateArgs {
    #[command(flatten)]
    pub http: AdminHttpArgs,

    /// Optional key label.
    #[arg(long)]
    pub label: Option<String>,

    /// Key lifetime in seconds.
    #[arg(long)]
    pub ttl_seconds: Option<u64>,

    /// Expiration as a Unix timestamp in seconds.
    #[arg(long)]
    pub expires_at: Option<u64>,

    /// Scope to attach to the key. May be repeated.
    #[arg(long = "scope")]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Args)]
pub struct KeyListArgs {
    #[command(flatten)]
    pub http: AdminHttpArgs,

    /// Filter by exact label.
    #[arg(long)]
    pub label: Option<String>,

    /// Filter by label prefix.
    #[arg(long)]
    pub label_prefix: Option<String>,

    /// Hide revoked keys.
    #[arg(long)]
    pub active_only: bool,
}

#[derive(Debug, Clone, Args)]
pub struct KeyRevokeArgs {
    #[command(flatten)]
    pub http: AdminHttpArgs,

    /// Key id to revoke.
    pub id: String,
}

#[derive(Debug, Clone, Subcommand)]
pub enum RouterCommand {
    /// Send a small chat completion request and show routing-related response details.
    Probe(RouterProbeArgs),
}

#[derive(Debug, Clone, Args)]
pub struct RouterProbeArgs {
    /// Routiium base URL.
    #[arg(long, default_value = "http://127.0.0.1:8088")]
    pub url: String,

    /// Model or alias to request.
    #[arg(long)]
    pub model: String,

    /// Prompt to send.
    #[arg(long, default_value = "Reply with exactly: ok")]
    pub prompt: String,

    /// Bearer token for the Routiium request. Defaults to ROUTIIUM_API_KEY.
    #[arg(long, env = "ROUTIIUM_API_KEY")]
    pub api_key: Option<String>,

    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum JudgeCommand {
    /// Write judge-related env defaults for a local profile.
    Profile(JudgeProfileArgs),
}

#[derive(Debug, Clone, Args)]
pub struct JudgeProfileArgs {
    /// Judge operating mode.
    #[arg(value_enum)]
    pub mode: JudgeMode,

    /// Environment file to update.
    #[arg(long, default_value = ".env")]
    pub out: PathBuf,

    /// Overwrite an existing file if it does not look like a key=value env file.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum JudgeMode {
    Off,
    Shadow,
    Enforce,
}

#[derive(Debug, Clone, Args)]
pub struct DocsArgs {
    /// Emit documentation links as JSON.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(command: Command) -> Result<()> {
    match command {
        Command::Serve(_) => unreachable!("serve is handled by main"),
        Command::Init(args) => run_init(args),
        Command::Doctor(args) => run_doctor(args).await,
        Command::Status(args) => run_status(args).await,
        Command::Key(command) => run_key(command).await,
        Command::Router(command) => run_router(command).await,
        Command::Judge(command) => run_judge(command),
        Command::Docs(args) => run_docs(args),
    }
}

fn normalize_legacy_args<I, T>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    if args.len() <= 1 {
        args.push(OsString::from("serve"));
        return args;
    }

    let first = args[1].to_string_lossy();
    let known_subcommands = [
        "serve", "init", "doctor", "status", "key", "keys", "router", "judge", "docs", "help",
    ];
    let root_flags = ["--help", "-h", "--version", "-V"];

    if first.starts_with('-') && !root_flags.contains(&first.as_ref()) {
        args.insert(1, OsString::from("serve"));
    } else if !first.starts_with('-') && !known_subcommands.contains(&first.as_ref()) {
        // Let clap produce the unknown-command error.
    }

    args
}

fn parse_key_backend_spec(value: &str) -> std::result::Result<String, String> {
    if routiium::auth::ApiKeyManager::backend_from_arg_spec(value).is_some() {
        Ok(value.to_string())
    } else {
        Err("expected redis://..., sled:<path>, or memory".to_string())
    }
}

fn run_init(args: InitArgs) -> Result<()> {
    let profile = init_profile_env(args.profile, &args.config_dir);
    write_new_file(&args.out, &profile, args.force)?;

    let mut created = vec![args.out.display().to_string()];
    if matches!(args.profile, InitProfile::Bedrock) {
        let routing_path = args.config_dir.join("routing.bedrock.json");
        write_new_file(&routing_path, bedrock_routing_template(), args.force)?;
        created.push(routing_path.display().to_string());
    }

    println!("Created Routiium {} starter files:", args.profile);
    for path in created {
        println!("  - {path}");
    }
    println!("Next: routiium doctor --env-file {}", args.out.display());
    println!("Then: routiium serve");
    Ok(())
}

async fn run_doctor(args: DoctorArgs) -> Result<()> {
    let file_env = read_env_file(&args.env_file).unwrap_or_default();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?;

    let mut checks = Vec::new();
    checks.push(check(
        "env_file",
        if args.env_file.exists() {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
        if args.env_file.exists() {
            format!("found {}", args.env_file.display())
        } else {
            format!(
                "{} not found; using process environment only",
                args.env_file.display()
            )
        },
    ));

    for (name, path) in [
        (
            "ROUTIIUM_MCP_CONFIG",
            env_value(&file_env, "ROUTIIUM_MCP_CONFIG"),
        ),
        (
            "ROUTIIUM_SYSTEM_PROMPT_CONFIG",
            env_value(&file_env, "ROUTIIUM_SYSTEM_PROMPT_CONFIG"),
        ),
        (
            "ROUTIIUM_ROUTING_CONFIG",
            env_value(&file_env, "ROUTIIUM_ROUTING_CONFIG"),
        ),
        (
            "ROUTIIUM_RATE_LIMIT_CONFIG",
            env_value(&file_env, "ROUTIIUM_RATE_LIMIT_CONFIG"),
        ),
        (
            "ROUTIIUM_ROUTER_CONFIG",
            env_value(&file_env, "ROUTIIUM_ROUTER_CONFIG"),
        ),
    ] {
        if let Some(path) = path.filter(|p| !p.trim().is_empty()) {
            checks.push(check(
                name,
                if Path::new(&path).exists() {
                    CheckStatus::Ok
                } else {
                    CheckStatus::Error
                },
                format!(
                    "{} -> {}",
                    path,
                    if Path::new(&path).exists() {
                        "readable"
                    } else {
                        "missing"
                    }
                ),
            ));
        }
    }

    let has_openai = env_value(&file_env, "OPENAI_API_KEY")
        .map(|v| is_real_env_value(&v))
        .unwrap_or(false);
    let has_base = env_value(&file_env, "OPENAI_BASE_URL")
        .map(|v| is_real_env_value(&v))
        .unwrap_or(false);
    checks.push(check(
        "provider",
        if has_openai || has_base {
            CheckStatus::Ok
        } else {
            CheckStatus::Error
        },
        if has_openai {
            "OPENAI_API_KEY is configured".to_string()
        } else if has_base {
            "OPENAI_BASE_URL is configured".to_string()
        } else {
            "set OPENAI_API_KEY or OPENAI_BASE_URL before proxying real requests".to_string()
        },
    ));

    let router_url = env_value(&file_env, "ROUTIIUM_ROUTER_URL");
    let cache_ttl =
        env_value(&file_env, "ROUTIIUM_CACHE_TTL_MS").unwrap_or_else(|| "15000".to_string());
    let judge_mode = env_value(&file_env, "ROUTER_JUDGE_MODE").unwrap_or_else(|| "off".to_string());
    let judge_every_request_ready = judge_mode == "off" || cache_ttl == "0";
    checks.push(check(
        "judge_cache",
        if judge_every_request_ready {
            CheckStatus::Ok
        } else {
            CheckStatus::Error
        },
        if judge_mode == "off" {
            "judge mode is off".to_string()
        } else if cache_ttl == "0" {
            "judge mode configured with ROUTIIUM_CACHE_TTL_MS=0".to_string()
        } else {
            "set ROUTIIUM_CACHE_TTL_MS=0 when every request must be judged".to_string()
        },
    ));

    match client.get(join_url(&args.url, "/status")).send().await {
        Ok(resp) => checks.push(check(
            "server_status",
            if resp.status().is_success() {
                CheckStatus::Ok
            } else if args.require_server {
                CheckStatus::Error
            } else {
                CheckStatus::Warn
            },
            format!("GET /status -> {}", resp.status()),
        )),
        Err(err) => checks.push(check(
            "server_status",
            if args.require_server {
                CheckStatus::Error
            } else {
                CheckStatus::Warn
            },
            format!("server not reachable yet; skipped live /status check ({err})"),
        )),
    }

    if args.check_router {
        if let Some(router_url) = router_url {
            match client
                .get(join_url(&router_url, "/catalog/models"))
                .send()
                .await
            {
                Ok(resp) => checks.push(check(
                    "router_catalog",
                    if resp.status().is_success() {
                        CheckStatus::Ok
                    } else {
                        CheckStatus::Error
                    },
                    format!("GET router /catalog/models -> {}", resp.status()),
                )),
                Err(err) => checks.push(check(
                    "router_catalog",
                    CheckStatus::Error,
                    format!("router check failed: {err}"),
                )),
            }
        } else {
            checks.push(check(
                "router_catalog",
                CheckStatus::Error,
                "ROUTIIUM_ROUTER_URL is not configured".to_string(),
            ));
        }
    }

    emit_checks(&checks, args.json)
}

async fn run_status(args: StatusArgs) -> Result<()> {
    let client = reqwest::Client::new();
    let mut request = client.get(join_url(&args.url, "/status"));
    if let Some(token) = args
        .token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
    {
        request = request.bearer_auth(token);
    }
    let value = request
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("Routiium status at {}", args.url);
        println!(
            "version: {}",
            value
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        );
        if let Some(router) = value.get("router") {
            println!("router: {}", serde_json::to_string(router)?);
        }
        if let Some(features) = value.get("features") {
            println!("features: {}", serde_json::to_string_pretty(features)?);
        }
    }
    Ok(())
}

async fn run_key(command: KeyCommand) -> Result<()> {
    match command {
        KeyCommand::Create(args) => {
            let scopes = if args.scopes.is_empty() {
                None
            } else {
                Some(args.scopes)
            };
            let body = json!({
                "label": args.label,
                "ttl_seconds": args.ttl_seconds,
                "expires_at": args.expires_at,
                "scopes": scopes,
            });
            let value = admin_request(
                &args.http,
                reqwest::Method::POST,
                "/keys/generate",
                Some(body),
            )
            .await?;
            print_json_or_summary(&value, args.http.json, "created key")
        }
        KeyCommand::List(args) => {
            let params = key_list_query_params(&args);
            let value =
                admin_request_with_query(&args.http, reqwest::Method::GET, "/keys", None, &params)
                    .await?;
            print_json_or_summary(&value, args.http.json, "keys")
        }
        KeyCommand::Revoke(args) => {
            let body = json!({ "id": args.id });
            let value = admin_request(
                &args.http,
                reqwest::Method::POST,
                "/keys/revoke",
                Some(body),
            )
            .await?;
            print_json_or_summary(&value, args.http.json, "revocation")
        }
    }
}

async fn run_router(command: RouterCommand) -> Result<()> {
    match command {
        RouterCommand::Probe(args) => {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()?;
            let body = json!({
                "model": args.model,
                "messages": [{"role": "user", "content": args.prompt}],
                "max_tokens": 1,
                "stream": false
            });
            let mut request = client
                .post(join_url(&args.url, "/v1/chat/completions"))
                .json(&body);
            if let Some(key) = args.api_key.as_deref().filter(|key| !key.trim().is_empty()) {
                request = request.bearer_auth(key);
            }
            let response = request.send().await?;
            let status = response.status();
            let headers = routing_headers(response.headers());
            let text = response.text().await.unwrap_or_default();
            let parsed =
                serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({ "body": text }));
            let output = json!({
                "status": status.as_u16(),
                "success": status.is_success(),
                "routing_headers": headers,
                "body": parsed,
            });
            if args.json {
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("router probe -> HTTP {}", status);
                if let Some(map) = output.get("routing_headers").and_then(Value::as_object) {
                    if map.is_empty() {
                        println!("routing headers: none returned");
                    } else {
                        println!("routing headers:");
                        for (k, v) in map {
                            println!("  {k}: {}", v.as_str().unwrap_or(""));
                        }
                    }
                }
                println!(
                    "body: {}",
                    serde_json::to_string_pretty(output.get("body").unwrap())?
                );
            }
            Ok(())
        }
    }
}

fn run_judge(command: JudgeCommand) -> Result<()> {
    match command {
        JudgeCommand::Profile(args) => {
            let updates = judge_profile_env(args.mode);
            update_env_file(&args.out, &updates, args.force)?;
            println!(
                "Updated {} for judge mode {:?}",
                args.out.display(),
                args.mode
            );
            if !matches!(args.mode, JudgeMode::Off) {
                println!("Reminder: every-request judging requires ROUTIIUM_CACHE_TTL_MS=0 and router-side cache.ttl_ms=0.");
            }
            Ok(())
        }
    }
}

fn run_docs(args: DocsArgs) -> Result<()> {
    let docs = json!({
        "getting_started": "docs/GETTING_STARTED.md",
        "cli": "docs/CLI.md",
        "configuration": "docs/CONFIGURATION.md",
        "router": "docs/ROUTER_USAGE.md",
        "api": "docs/API_REFERENCE.md",
        "production": "docs/PRODUCTION_HARDENING.md",
    });
    if args.json {
        println!("{}", serde_json::to_string_pretty(&docs)?);
    } else {
        println!("Routiium docs:");
        for (name, path) in docs.as_object().unwrap() {
            println!("  {name}: {}", path.as_str().unwrap());
        }
    }
    Ok(())
}

async fn admin_request(
    args: &AdminHttpArgs,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    admin_request_with_query(args, method, path, body, &[]).await
}

async fn admin_request_with_query(
    args: &AdminHttpArgs,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
    query: &[(String, String)],
) -> Result<Value> {
    let client = reqwest::Client::new();
    let mut request = client.request(method, join_url(&args.url, path));
    if !query.is_empty() {
        request = request.query(query);
    }
    if let Some(token) = args
        .admin_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
    {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let value = serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({ "body": text }));
    if !status.is_success() {
        return Err(anyhow!(
            "admin request failed with HTTP {status}: {}",
            serde_json::to_string(&value)?
        ));
    }
    Ok(value)
}

fn print_json_or_summary(value: &Value, json_output: bool, label: &str) -> Result<()> {
    if json_output {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{label}:");
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}

fn routing_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let key = name.as_str().to_ascii_lowercase();
            if key.starts_with("x-routiium")
                || key.starts_with("x-router")
                || key.starts_with("x-route")
            {
                Some((key, value.to_str().unwrap_or("<non-utf8>").to_string()))
            } else {
                None
            }
        })
        .collect()
}

fn join_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

fn key_list_query_params(args: &KeyListArgs) -> Vec<(String, String)> {
    let mut params = Vec::new();
    if let Some(label) = &args.label {
        params.push(("label".to_string(), label.clone()));
    }
    if let Some(prefix) = &args.label_prefix {
        params.push(("label_prefix".to_string(), prefix.clone()));
    }
    if args.active_only {
        params.push(("include_revoked".to_string(), "false".to_string()));
    }
    params
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckStatus {
    Ok,
    Warn,
    Error,
}

impl CheckStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug)]
struct Check {
    name: &'static str,
    status: CheckStatus,
    message: String,
}

fn check(name: &'static str, status: CheckStatus, message: String) -> Check {
    Check {
        name,
        status,
        message,
    }
}

fn emit_checks(checks: &[Check], json_output: bool) -> Result<()> {
    let error_count = checks
        .iter()
        .filter(|check| check.status == CheckStatus::Error)
        .count();
    let warn_count = checks
        .iter()
        .filter(|check| check.status == CheckStatus::Warn)
        .count();

    if json_output {
        let value = json!({
            "ok": error_count == 0,
            "errors": error_count,
            "warnings": warn_count,
            "checks": checks.iter().map(|check| json!({
                "name": check.name,
                "ok": check.status != CheckStatus::Error,
                "status": check.status.as_str(),
                "message": check.message,
            })).collect::<Vec<_>>()
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("Routiium doctor");
        for check in checks {
            let marker = match check.status {
                CheckStatus::Ok => "ok",
                CheckStatus::Warn => "--",
                CheckStatus::Error => "!!",
            };
            println!("{} {} - {}", marker, check.name, check.message);
        }
    }

    if error_count == 0 {
        Ok(())
    } else {
        Err(anyhow!(
            "doctor found {error_count} error(s) and {warn_count} warning(s)"
        ))
    }
}

fn init_profile_env(profile: InitProfile, config_dir: &Path) -> String {
    let bind = "BIND_ADDR=127.0.0.1:8088\nROUTIIUM_ADMIN_TOKEN=change-me-admin-token\n";
    match profile {
        InitProfile::Openai => format!(
            "# Routiium OpenAI-compatible proxy profile\n{bind}OPENAI_API_KEY=sk-your-openai-key\n# Optional: OPENAI_BASE_URL=https://api.openai.com/v1\n"
        ),
        InitProfile::Vllm => format!(
            "# Routiium local OpenAI-compatible upstream profile\n{bind}OPENAI_BASE_URL=http://127.0.0.1:8000/v1\nROUTIIUM_UPSTREAM_MODE=chat\nROUTIIUM_MANAGED_MODE=0\n"
        ),
        InitProfile::Router => format!(
            "# Routiium remote router profile\n{bind}OPENAI_API_KEY=sk-your-provider-key\nROUTIIUM_ROUTER_URL=http://127.0.0.1:9090\nROUTIIUM_ROUTER_STRICT=1\nROUTIIUM_ROUTER_PRIVACY_MODE=features\nROUTIIUM_CACHE_TTL_MS=15000\n"
        ),
        InitProfile::Judge => format!(
            "# Routiium remote router + LLM-as-judge profile\n{bind}OPENAI_API_KEY=sk-your-provider-key\nROUTIIUM_ROUTER_URL=http://127.0.0.1:9090\nROUTIIUM_ROUTER_STRICT=1\nROUTIIUM_ROUTER_PRIVACY_MODE=full\nROUTIIUM_CACHE_TTL_MS=0\nROUTER_JUDGE_MODE=shadow\nROUTER_JUDGE_CONTEXT=full\nROUTER_JUDGE_FAILURE=allow\nROUTER_JUDGE_MODEL=gpt-4o-mini\nROUTER_JUDGE_API_KEY_ENV=OPENAI_API_KEY\n"
        ),
        InitProfile::Bedrock => {
            let routing_path = config_dir.join("routing.bedrock.json");
            format!(
                "# Routiium AWS Bedrock profile\n{bind}AWS_REGION=us-east-1\nROUTIIUM_UPSTREAM_MODE=bedrock\nROUTIIUM_ROUTING_CONFIG={}\nROUTIIUM_MANAGED_MODE=1\n",
                routing_path.display()
            )
        }
    }
}

fn bedrock_routing_template() -> &'static str {
    r#"{
  "rules": [
    {
      "name": "bedrock-claude",
      "match": { "strategy": "prefix", "value": "bedrock/" },
      "backend": {
        "base_url": "bedrock://anthropic.claude-3-5-sonnet-20240620-v1:0",
        "mode": "bedrock"
      }
    }
  ],
  "aliases": []
}
"#
}

fn write_new_file(path: &Path, contents: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        return Err(anyhow!(
            "{} already exists; pass --force to overwrite",
            path.display()
        ));
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

fn read_env_file(path: &Path) -> Result<BTreeMap<String, String>> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(parse_env_contents(&contents))
}

fn parse_env_contents(contents: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        out.insert(
            key.trim().to_string(),
            trim_env_value(value.trim()).to_string(),
        );
    }
    out
}

fn trim_env_value(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value)
}

fn is_real_env_value(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.contains("your-")
        && !value.contains("change-me")
        && !value.eq_ignore_ascii_case("placeholder")
}

fn env_value(file_env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    file_env.get(key).cloned().or_else(|| env::var(key).ok())
}

fn judge_profile_env(mode: JudgeMode) -> BTreeMap<String, String> {
    match mode {
        JudgeMode::Off => BTreeMap::from([
            ("ROUTER_JUDGE_MODE".to_string(), "off".to_string()),
            ("ROUTIIUM_CACHE_TTL_MS".to_string(), "15000".to_string()),
        ]),
        JudgeMode::Shadow => BTreeMap::from([
            ("ROUTIIUM_ROUTER_STRICT".to_string(), "1".to_string()),
            (
                "ROUTIIUM_ROUTER_PRIVACY_MODE".to_string(),
                "full".to_string(),
            ),
            ("ROUTIIUM_CACHE_TTL_MS".to_string(), "0".to_string()),
            ("ROUTER_JUDGE_MODE".to_string(), "shadow".to_string()),
            ("ROUTER_JUDGE_CONTEXT".to_string(), "full".to_string()),
            ("ROUTER_JUDGE_FAILURE".to_string(), "allow".to_string()),
        ]),
        JudgeMode::Enforce => BTreeMap::from([
            ("ROUTIIUM_ROUTER_STRICT".to_string(), "1".to_string()),
            (
                "ROUTIIUM_ROUTER_PRIVACY_MODE".to_string(),
                "full".to_string(),
            ),
            ("ROUTIIUM_CACHE_TTL_MS".to_string(), "0".to_string()),
            ("ROUTER_JUDGE_MODE".to_string(), "enforce".to_string()),
            ("ROUTER_JUDGE_CONTEXT".to_string(), "full".to_string()),
            ("ROUTER_JUDGE_FAILURE".to_string(), "deny".to_string()),
        ]),
    }
}

fn update_env_file(path: &Path, updates: &BTreeMap<String, String>, force: bool) -> Result<()> {
    let existing = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };

    if !existing.is_empty() && !force && !existing.lines().any(|line| line.contains('=')) {
        return Err(anyhow!(
            "{} does not look like an env file; pass --force to overwrite",
            path.display()
        ));
    }

    let update_keys: BTreeSet<_> = updates.keys().cloned().collect();
    let mut seen = BTreeSet::new();
    let mut lines = Vec::new();
    for line in existing.lines() {
        if let Some((key, _)) = line.split_once('=') {
            let key = key.trim();
            if let Some(value) = updates.get(key) {
                lines.push(format!("{key}={value}"));
                seen.insert(key.to_string());
                continue;
            }
        }
        lines.push(line.to_string());
    }

    for key in update_keys.difference(&seen) {
        if let Some(value) = updates.get(key) {
            lines.push(format!("{key}={value}"));
        }
    }

    let mut contents = lines.join("\n");
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn legacy_root_flags_default_to_serve() {
        let cli = Cli::parse_from_compat([
            "routiium",
            "--keys-backend=memory",
            "--mcp-config",
            "mcp.json",
        ]);
        match cli.command.unwrap() {
            Command::Serve(args) => {
                assert_eq!(args.keys_backend.as_deref(), Some("memory"));
                assert_eq!(args.mcp_config.as_deref(), Some(Path::new("mcp.json")));
            }
            _ => panic!("expected serve"),
        }
    }

    #[test]
    fn parses_init_profile() {
        let cli = Cli::parse_from_compat([
            "routiium",
            "init",
            "--profile",
            "judge",
            "--out",
            "judge.env",
        ]);
        match cli.command.unwrap() {
            Command::Init(args) => {
                assert_eq!(args.profile, InitProfile::Judge);
                assert_eq!(args.out, PathBuf::from("judge.env"));
            }
            _ => panic!("expected init"),
        }
    }

    #[test]
    fn parses_key_create() {
        let cli = Cli::parse_from_compat([
            "routiium",
            "key",
            "create",
            "--label",
            "demo",
            "--scope",
            "chat",
            "--ttl-seconds",
            "60",
        ]);
        match cli.command.unwrap() {
            Command::Key(KeyCommand::Create(args)) => {
                assert_eq!(args.label.as_deref(), Some("demo"));
                assert_eq!(args.scopes, vec!["chat"]);
                assert_eq!(args.ttl_seconds, Some(60));
            }
            _ => panic!("expected key create"),
        }
    }

    #[test]
    fn parses_status_router_judge_and_docs() {
        let status = Cli::parse_from_compat([
            "routiium",
            "status",
            "--url",
            "http://localhost:9999",
            "--json",
        ]);
        match status.command.unwrap() {
            Command::Status(args) => {
                assert_eq!(args.url, "http://localhost:9999");
                assert!(args.json);
            }
            _ => panic!("expected status"),
        }

        let router =
            Cli::parse_from_compat(["routiium", "router", "probe", "--model", "safe-alias"]);
        match router.command.unwrap() {
            Command::Router(RouterCommand::Probe(args)) => assert_eq!(args.model, "safe-alias"),
            _ => panic!("expected router probe"),
        }

        let judge = Cli::parse_from_compat([
            "routiium",
            "judge",
            "profile",
            "enforce",
            "--out",
            "judge.env",
        ]);
        match judge.command.unwrap() {
            Command::Judge(JudgeCommand::Profile(args)) => {
                assert_eq!(args.mode, JudgeMode::Enforce);
                assert_eq!(args.out, PathBuf::from("judge.env"));
            }
            _ => panic!("expected judge profile"),
        }

        let docs = Cli::parse_from_compat(["routiium", "docs", "--json"]);
        match docs.command.unwrap() {
            Command::Docs(args) => assert!(args.json),
            _ => panic!("expected docs"),
        }
    }

    #[test]
    fn key_list_uses_standard_query_encoding_for_special_characters() {
        let args = KeyListArgs {
            http: AdminHttpArgs {
                url: "http://example.test".to_string(),
                admin_token: None,
                json: false,
            },
            label: Some("a+b c%&".to_string()),
            label_prefix: None,
            active_only: true,
        };
        let params = key_list_query_params(&args);
        let request = reqwest::Client::new()
            .get("http://example.test/keys")
            .query(&params)
            .build()
            .unwrap();

        assert_eq!(
            request.url().query(),
            Some("label=a%2Bb+c%25%26&include_revoked=false")
        );
    }

    #[tokio::test]
    async fn doctor_warns_when_server_is_unreachable_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = dir.path().join(".env");
        fs::write(
            &env_file,
            "OPENAI_BASE_URL=http://127.0.0.1:9/v1
",
        )
        .unwrap();

        let result = run_doctor(DoctorArgs {
            url: "http://127.0.0.1:9".to_string(),
            env_file,
            json: true,
            check_router: false,
            require_server: false,
        })
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn doctor_fails_when_required_server_is_unreachable() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = dir.path().join(".env");
        fs::write(
            &env_file,
            "OPENAI_BASE_URL=http://127.0.0.1:9/v1
",
        )
        .unwrap();

        let result = run_doctor(DoctorArgs {
            url: "http://127.0.0.1:9".to_string(),
            env_file,
            json: true,
            check_router: false,
            require_server: true,
        })
        .await;

        assert!(result.is_err());
    }

    #[test]
    fn updates_env_file_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        fs::write(&path, "A=1\nROUTER_JUDGE_MODE=off\n").unwrap();
        update_env_file(&path, &judge_profile_env(JudgeMode::Shadow), false).unwrap();
        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.contains("ROUTER_JUDGE_MODE=shadow"));
        assert!(contents.contains("ROUTIIUM_CACHE_TTL_MS=0"));
        assert!(contents.contains("A=1"));
    }

    #[test]
    fn init_refuses_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        fs::write(&path, "A=1\n").unwrap();
        let result = write_new_file(&path, "B=2\n", false);
        assert!(result.is_err());
    }
}
