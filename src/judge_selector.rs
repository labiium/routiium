use crate::router_client::{RouteRequest, ToolSignal};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeSelectorScope {
    BaselineAlways,
    GateAll,
}

impl JudgeSelectorScope {
    pub fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "gate_all" | "gate-all" | "all" => Self::GateAll,
            _ => Self::BaselineAlways,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BaselineAlways => "baseline_always",
            Self::GateAll => "gate_all",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeSelectorAction {
    Judge,
    Skip,
    Deny,
}

impl JudgeSelectorAction {
    pub fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "skip" | "off" | "false" => Self::Skip,
            "deny" | "reject" | "block" => Self::Deny,
            _ => Self::Judge,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Judge => "judge",
            Self::Skip => "skip",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeSelectorOnError {
    Judge,
    Skip,
    Deny,
}

impl JudgeSelectorOnError {
    pub fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "skip" | "off" | "false" => Self::Skip,
            "deny" | "reject" | "block" => Self::Deny,
            _ => Self::Judge,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeSelectorConfig {
    #[serde(default)]
    pub scope: Option<JudgeSelectorScope>,
    #[serde(default)]
    pub default: Option<JudgeSelectorAction>,
    #[serde(default)]
    pub on_error: Option<JudgeSelectorOnError>,
    #[serde(default)]
    pub tool_groups: HashMap<String, ToolGroupConfig>,
    #[serde(default)]
    pub embedding_classifiers: Vec<EmbeddingClassifierConfig>,
    #[serde(default)]
    pub rules: Vec<JudgeSelectorRule>,
}

impl JudgeSelectorConfig {
    pub fn from_policy_and_env(policy: Option<Self>) -> Option<Self> {
        let mut config = policy;

        let has_env = [
            "ROUTIIUM_JUDGE_SELECTOR_SCOPE",
            "ROUTIIUM_JUDGE_SELECTOR_DEFAULT",
            "ROUTIIUM_JUDGE_SELECTOR_ON_ERROR",
            "ROUTIIUM_JUDGE_SELECTOR_TOOL_ONLY",
            "ROUTIIUM_JUDGE_SELECTOR_TOOL_TYPES",
            "ROUTIIUM_JUDGE_SELECTOR_REGEX",
        ]
        .iter()
        .any(|key| std::env::var(key).is_ok());

        if !has_env {
            return config;
        }

        let cfg = config.get_or_insert_with(|| Self {
            scope: None,
            default: None,
            on_error: None,
            tool_groups: HashMap::new(),
            embedding_classifiers: Vec::new(),
            rules: Vec::new(),
        });

        if let Ok(value) = std::env::var("ROUTIIUM_JUDGE_SELECTOR_SCOPE") {
            cfg.scope = Some(JudgeSelectorScope::from_env_value(&value));
        }
        if let Ok(value) = std::env::var("ROUTIIUM_JUDGE_SELECTOR_DEFAULT") {
            cfg.default = Some(JudgeSelectorAction::from_env_value(&value));
        }
        if let Ok(value) = std::env::var("ROUTIIUM_JUDGE_SELECTOR_ON_ERROR") {
            cfg.on_error = Some(JudgeSelectorOnError::from_env_value(&value));
        }
        if truthy_env("ROUTIIUM_JUDGE_SELECTOR_TOOL_ONLY") {
            cfg.rules.push(JudgeSelectorRule {
                id: Some("env_tool_calls_only".to_string()),
                when: JudgeSelectorWhen {
                    has_tools: Some(true),
                    ..Default::default()
                },
                action: JudgeSelectorAction::Judge,
            });
        }
        if let Ok(value) = std::env::var("ROUTIIUM_JUDGE_SELECTOR_TOOL_TYPES") {
            let tool_types = split_list(&value);
            if !tool_types.is_empty() {
                cfg.rules.push(JudgeSelectorRule {
                    id: Some("env_tool_types".to_string()),
                    when: JudgeSelectorWhen {
                        tool_types_any: tool_types,
                        ..Default::default()
                    },
                    action: JudgeSelectorAction::Judge,
                });
            }
        }
        if let Ok(value) = std::env::var("ROUTIIUM_JUDGE_SELECTOR_REGEX") {
            let regexes = split_list(&value);
            if !regexes.is_empty() {
                cfg.rules.push(JudgeSelectorRule {
                    id: Some("env_regex".to_string()),
                    when: JudgeSelectorWhen {
                        content_regex_any: regexes,
                        ..Default::default()
                    },
                    action: JudgeSelectorAction::Judge,
                });
            }
        }

        config
    }

    pub fn scope(&self) -> JudgeSelectorScope {
        self.scope.unwrap_or(JudgeSelectorScope::BaselineAlways)
    }

    fn default_action(&self) -> JudgeSelectorAction {
        self.default.unwrap_or(JudgeSelectorAction::Judge)
    }

    fn on_error(&self) -> JudgeSelectorOnError {
        self.on_error.unwrap_or(JudgeSelectorOnError::Judge)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolGroupConfig {
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub name_regex: Vec<String>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingClassifierConfig {
    pub id: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub base_url_env: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    pub model: String,
    pub threshold: f32,
    #[serde(default)]
    pub positive_examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeSelectorRule {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub when: JudgeSelectorWhen,
    pub action: JudgeSelectorAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JudgeSelectorWhen {
    #[serde(default)]
    pub has_tools: Option<bool>,
    #[serde(default)]
    pub tool_names_any: Vec<String>,
    #[serde(default)]
    pub tool_types_any: Vec<String>,
    #[serde(default)]
    pub tool_sources_any: Vec<String>,
    #[serde(default)]
    pub tools_outside_groups: Vec<String>,
    #[serde(default)]
    pub content_regex_any: Vec<String>,
    #[serde(default)]
    pub embedding_classifier: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JudgeSelectorDecision {
    pub scope: JudgeSelectorScope,
    pub action: JudgeSelectorAction,
    pub matched_rules: Vec<String>,
    pub reason: String,
}

impl JudgeSelectorDecision {
    pub fn should_judge(&self) -> bool {
        matches!(self.action, JudgeSelectorAction::Judge)
    }
}

pub async fn evaluate_selector(
    config: &JudgeSelectorConfig,
    client: Option<&reqwest::Client>,
    req: &RouteRequest,
    request_text: &str,
) -> JudgeSelectorDecision {
    match try_evaluate_selector(config, client, req, request_text).await {
        Ok(decision) => decision,
        Err(err) => match config.on_error() {
            JudgeSelectorOnError::Judge => JudgeSelectorDecision {
                scope: config.scope(),
                action: JudgeSelectorAction::Judge,
                matched_rules: vec!["selector_error".to_string()],
                reason: format!("selector error; judging request: {}", sanitize_reason(&err)),
            },
            JudgeSelectorOnError::Skip => JudgeSelectorDecision {
                scope: config.scope(),
                action: JudgeSelectorAction::Skip,
                matched_rules: vec!["selector_error".to_string()],
                reason: format!(
                    "selector error; skipping extra judge: {}",
                    sanitize_reason(&err)
                ),
            },
            JudgeSelectorOnError::Deny => JudgeSelectorDecision {
                scope: config.scope(),
                action: JudgeSelectorAction::Deny,
                matched_rules: vec!["selector_error".to_string()],
                reason: format!("selector error; denying request: {}", sanitize_reason(&err)),
            },
        },
    }
}

async fn try_evaluate_selector(
    config: &JudgeSelectorConfig,
    client: Option<&reqwest::Client>,
    req: &RouteRequest,
    request_text: &str,
) -> Result<JudgeSelectorDecision, String> {
    for (index, rule) in config.rules.iter().enumerate() {
        if rule_matches(config, client, req, request_text, &rule.when).await? {
            let id = rule
                .id
                .clone()
                .unwrap_or_else(|| format!("rule_{}", index + 1));
            return Ok(JudgeSelectorDecision {
                scope: config.scope(),
                action: rule.action,
                matched_rules: vec![id.clone()],
                reason: format!("matched judge selector rule {id}"),
            });
        }
    }

    Ok(JudgeSelectorDecision {
        scope: config.scope(),
        action: config.default_action(),
        matched_rules: Vec::new(),
        reason: "no judge selector rule matched".to_string(),
    })
}

async fn rule_matches(
    config: &JudgeSelectorConfig,
    client: Option<&reqwest::Client>,
    req: &RouteRequest,
    request_text: &str,
    when: &JudgeSelectorWhen,
) -> Result<bool, String> {
    if let Some(has_tools) = when.has_tools {
        if req.tools.is_empty() == has_tools {
            return Ok(false);
        }
    }

    if !when.tool_names_any.is_empty()
        && !req.tools.iter().any(|tool| {
            when.tool_names_any
                .iter()
                .any(|name| tool.name.eq_ignore_ascii_case(name))
        })
    {
        return Ok(false);
    }

    if !when.tool_types_any.is_empty()
        && !req.tools.iter().any(|tool| {
            tool.tool_type.as_deref().is_some_and(|tool_type| {
                when.tool_types_any
                    .iter()
                    .any(|expected| tool_type.eq_ignore_ascii_case(expected))
            })
        })
    {
        return Ok(false);
    }

    if !when.tool_sources_any.is_empty()
        && !req.tools.iter().any(|tool| {
            tool.source.as_deref().is_some_and(|source| {
                when.tool_sources_any
                    .iter()
                    .any(|expected| source.eq_ignore_ascii_case(expected))
            })
        })
    {
        return Ok(false);
    }

    if !when.tools_outside_groups.is_empty() {
        let mut has_tool_outside_groups = false;
        for tool in &req.tools {
            if !tool_in_any_group(config, tool, &when.tools_outside_groups)? {
                has_tool_outside_groups = true;
                break;
            }
        }
        if !has_tool_outside_groups {
            return Ok(false);
        }
    }

    if !when.content_regex_any.is_empty() {
        let matched = when
            .content_regex_any
            .iter()
            .map(|pattern| {
                Regex::new(pattern)
                    .map_err(|err| format!("invalid selector regex {pattern:?}: {err}"))
                    .map(|regex| regex.is_match(request_text))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|matched| matched);
        if !matched {
            return Ok(false);
        }
    }

    if let Some(classifier_id) = when.embedding_classifier.as_deref() {
        let Some(classifier) = config
            .embedding_classifiers
            .iter()
            .find(|classifier| classifier.id == classifier_id)
        else {
            return Err(format!("unknown embedding classifier {classifier_id}"));
        };
        if !embedding_classifier_matches(classifier, client, req, request_text).await? {
            return Ok(false);
        }
    }

    Ok(true)
}

fn tool_in_any_group(
    config: &JudgeSelectorConfig,
    tool: &ToolSignal,
    groups: &[String],
) -> Result<bool, String> {
    for group_name in groups {
        let Some(group) = config.tool_groups.get(group_name) else {
            return Err(format!("unknown tool group {group_name}"));
        };
        if tool_matches_group(tool, group)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn tool_matches_group(tool: &ToolSignal, group: &ToolGroupConfig) -> Result<bool, String> {
    if group
        .names
        .iter()
        .any(|name| tool.name.eq_ignore_ascii_case(name))
    {
        return Ok(true);
    }
    if group.types.iter().any(|expected| {
        tool.tool_type
            .as_deref()
            .is_some_and(|tool_type| tool_type.eq_ignore_ascii_case(expected))
    }) {
        return Ok(true);
    }
    if group.sources.iter().any(|expected| {
        tool.source
            .as_deref()
            .is_some_and(|source| source.eq_ignore_ascii_case(expected))
    }) {
        return Ok(true);
    }
    for pattern in &group.name_regex {
        let regex = Regex::new(pattern)
            .map_err(|err| format!("invalid tool group regex {pattern:?}: {err}"))?;
        if regex.is_match(&tool.name) {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn embedding_classifier_matches(
    classifier: &EmbeddingClassifierConfig,
    client: Option<&reqwest::Client>,
    req: &RouteRequest,
    request_text: &str,
) -> Result<bool, String> {
    let Some(client) = client else {
        return Err("embedding classifier needs an HTTP client".to_string());
    };
    if classifier.positive_examples.is_empty() {
        return Err(format!(
            "embedding classifier {} has no positive_examples",
            classifier.id
        ));
    }

    let request_input = selector_embedding_input(req, request_text);
    let request_embedding = embed_text(classifier, client, &request_input).await?;
    let examples = cached_example_embeddings(classifier, client).await?;
    let best = examples
        .iter()
        .map(|example| cosine_similarity(&request_embedding, example))
        .fold(0.0_f32, f32::max);
    Ok(best >= classifier.threshold)
}

async fn cached_example_embeddings(
    classifier: &EmbeddingClassifierConfig,
    client: &reqwest::Client,
) -> Result<Vec<Vec<f32>>, String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<Vec<f32>>>>> = OnceLock::new();
    let key = format!(
        "{}|{}|{}|{:?}",
        classifier.id, classifier.model, classifier.threshold, classifier.positive_examples
    );
    if let Some(cached) = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return Ok(cached);
    }

    let mut embeddings = Vec::new();
    for example in &classifier.positive_examples {
        embeddings.push(embed_text(classifier, client, example).await?);
    }

    if let Ok(mut cache) = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        cache.insert(key, embeddings.clone());
    }
    Ok(embeddings)
}

async fn embed_text(
    classifier: &EmbeddingClassifierConfig,
    client: &reqwest::Client,
    input: &str,
) -> Result<Vec<f32>, String> {
    let base_url = classifier
        .base_url
        .clone()
        .or_else(|| {
            classifier
                .base_url_env
                .as_deref()
                .and_then(|key| std::env::var(key).ok())
        })
        .or_else(|| std::env::var("ROUTIIUM_EMBEDDINGS_BASE_URL").ok())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_key_env = classifier
        .api_key_env
        .as_deref()
        .unwrap_or("OPENAI_API_KEY");
    let api_key = std::env::var(api_key_env).map_err(|_| format!("missing {api_key_env}"))?;
    let url = format!("{}/embeddings", base_url.trim_end_matches('/'));
    let response = client
        .post(url)
        .timeout(Duration::from_millis(1_000))
        .bearer_auth(api_key)
        .json(&serde_json::json!({
            "model": classifier.model,
            "input": input,
        }))
        .send()
        .await
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "embedding provider returned {status}: {}",
            sanitize_reason(&body)
        ));
    }

    let value: Value = response.json().await.map_err(|err| err.to_string())?;
    let embedding = value
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("embedding"))
        .and_then(Value::as_array)
        .ok_or_else(|| "embedding response did not include data[0].embedding".to_string())?;
    embedding
        .iter()
        .map(|value| {
            value
                .as_f64()
                .map(|number| number as f32)
                .ok_or_else(|| "embedding vector contained a non-number".to_string())
        })
        .collect()
}

fn selector_embedding_input(req: &RouteRequest, request_text: &str) -> String {
    let tools = req
        .tools
        .iter()
        .map(|tool| {
            format!(
                "{}:{}:{}",
                tool.source.as_deref().unwrap_or("unknown"),
                tool.tool_type.as_deref().unwrap_or("unknown"),
                tool.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "alias: {}\napi: {}\ntools:\n{}\nrequest:\n{}",
        req.alias, req.api, tools, request_text
    )
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut a_norm = 0.0;
    let mut b_norm = 0.0;
    for (left, right) in a.iter().zip(b.iter()) {
        dot += left * right;
        a_norm += left * left;
        b_norm += right * right;
    }
    if a_norm == 0.0 || b_norm == 0.0 {
        return 0.0;
    }
    dot / (a_norm.sqrt() * b_norm.sqrt())
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split([';', ','])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn truthy_env(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn sanitize_reason(reason: &str) -> String {
    reason.chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router_client::{
        Budget, ContentAttestation, ConversationSignals, Estimates, GeoHint, OrgContext,
        PrivacyMode, PrivacyTier, Targets, TraceContext,
    };

    fn req(tools: Vec<ToolSignal>, text: &str) -> RouteRequest {
        RouteRequest {
            schema_version: None,
            request_id: None,
            trace: None::<TraceContext>,
            alias: "auto".to_string(),
            api: "responses".to_string(),
            privacy_mode: PrivacyMode::Full,
            content_attestation: Some(ContentAttestation {
                included: Some("full".to_string()),
            }),
            caps: Vec::new(),
            stream: false,
            params: None,
            plan_token: None,
            targets: Targets::default(),
            budget: None::<Budget>,
            estimates: Estimates::default(),
            conversation: ConversationSignals {
                summary: Some(text.to_string()),
                ..Default::default()
            },
            org: OrgContext::default(),
            geo: None::<GeoHint>,
            tools,
            overrides: None,
            role: None,
            task: None,
            privacy: None::<PrivacyTier>,
            hints: None,
        }
    }

    #[tokio::test]
    async fn matches_tool_only_rule() {
        let config = JudgeSelectorConfig {
            scope: None,
            default: Some(JudgeSelectorAction::Skip),
            on_error: None,
            tool_groups: HashMap::new(),
            embedding_classifiers: Vec::new(),
            rules: vec![JudgeSelectorRule {
                id: Some("tools".to_string()),
                when: JudgeSelectorWhen {
                    has_tools: Some(true),
                    ..Default::default()
                },
                action: JudgeSelectorAction::Judge,
            }],
        };
        let decision = evaluate_selector(
            &config,
            None,
            &req(
                vec![ToolSignal {
                    name: "lookup".to_string(),
                    json_schema_hash: None,
                    tool_type: Some("function".to_string()),
                    source: Some("client".to_string()),
                    groups: Vec::new(),
                }],
                "hello",
            ),
            "hello",
        )
        .await;
        assert_eq!(decision.action, JudgeSelectorAction::Judge);
        assert_eq!(decision.matched_rules, vec!["tools"]);
    }

    #[tokio::test]
    async fn detects_tools_outside_group() {
        let mut groups = HashMap::new();
        groups.insert(
            "readonly".to_string(),
            ToolGroupConfig {
                names: vec!["read_file".to_string()],
                ..Default::default()
            },
        );
        let config = JudgeSelectorConfig {
            scope: None,
            default: Some(JudgeSelectorAction::Skip),
            on_error: None,
            tool_groups: groups,
            embedding_classifiers: Vec::new(),
            rules: vec![JudgeSelectorRule {
                id: Some("outside".to_string()),
                when: JudgeSelectorWhen {
                    tools_outside_groups: vec!["readonly".to_string()],
                    ..Default::default()
                },
                action: JudgeSelectorAction::Judge,
            }],
        };
        let decision = evaluate_selector(
            &config,
            None,
            &req(
                vec![ToolSignal {
                    name: "delete_file".to_string(),
                    json_schema_hash: None,
                    tool_type: Some("function".to_string()),
                    source: Some("client".to_string()),
                    groups: Vec::new(),
                }],
                "hello",
            ),
            "hello",
        )
        .await;
        assert_eq!(decision.action, JudgeSelectorAction::Judge);
    }

    #[tokio::test]
    async fn regex_error_follows_on_error() {
        let config = JudgeSelectorConfig {
            scope: None,
            default: Some(JudgeSelectorAction::Skip),
            on_error: Some(JudgeSelectorOnError::Deny),
            tool_groups: HashMap::new(),
            embedding_classifiers: Vec::new(),
            rules: vec![JudgeSelectorRule {
                id: Some("bad_regex".to_string()),
                when: JudgeSelectorWhen {
                    content_regex_any: vec!["[".to_string()],
                    ..Default::default()
                },
                action: JudgeSelectorAction::Judge,
            }],
        };
        let decision = evaluate_selector(&config, None, &req(Vec::new(), "hello"), "hello").await;
        assert_eq!(decision.action, JudgeSelectorAction::Deny);
        assert_eq!(decision.matched_rules, vec!["selector_error"]);
    }
}
