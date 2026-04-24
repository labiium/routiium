//! Lightweight safety audit trail for judge/router/response-guard events.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

const DEFAULT_MAX_EVENTS: usize = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyAuditEvent {
    pub id: String,
    pub timestamp: u64,
    pub kind: String,
    pub endpoint: String,
    pub client_ip: Option<String>,
    pub requested_model: Option<String>,
    pub resolved_model: Option<String>,
    pub route_id: Option<String>,
    pub action: Option<String>,
    pub target_alias: Option<String>,
    pub verdict: Option<String>,
    pub risk_level: Option<String>,
    pub reason: Option<String>,
    pub categories: Vec<String>,
    pub policy_rev: Option<String>,
    pub policy_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SafetyAuditEventBuilder {
    pub kind: String,
    pub endpoint: String,
    pub client_ip: Option<String>,
    pub requested_model: Option<String>,
    pub resolved_model: Option<String>,
    pub route_id: Option<String>,
    pub action: Option<String>,
    pub target_alias: Option<String>,
    pub verdict: Option<String>,
    pub risk_level: Option<String>,
    pub reason: Option<String>,
    pub categories: Vec<String>,
    pub policy_rev: Option<String>,
    pub policy_fingerprint: Option<String>,
}

#[derive(Debug)]
pub struct SafetyAuditManager {
    events: Mutex<VecDeque<SafetyAuditEvent>>,
    max_events: usize,
    jsonl_path: Option<PathBuf>,
}

impl SafetyAuditManager {
    pub fn from_env() -> Arc<Self> {
        let max_events = std::env::var("ROUTIIUM_SAFETY_AUDIT_MAX_EVENTS")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MAX_EVENTS);
        let jsonl_path = std::env::var("ROUTIIUM_SAFETY_AUDIT_PATH")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        Arc::new(Self {
            events: Mutex::new(VecDeque::new()),
            max_events,
            jsonl_path,
        })
    }

    pub async fn record(&self, builder: SafetyAuditEventBuilder) -> SafetyAuditEvent {
        let event = SafetyAuditEvent {
            id: new_event_id(),
            timestamp: unix_timestamp(),
            kind: builder.kind,
            endpoint: builder.endpoint,
            client_ip: builder.client_ip,
            requested_model: builder.requested_model,
            resolved_model: builder.resolved_model,
            route_id: builder.route_id,
            action: builder.action,
            target_alias: builder.target_alias,
            verdict: builder.verdict,
            risk_level: builder.risk_level,
            reason: builder.reason,
            categories: builder.categories,
            policy_rev: builder.policy_rev,
            policy_fingerprint: builder.policy_fingerprint,
        };

        {
            let mut events = self.events.lock().await;
            events.push_back(event.clone());
            while events.len() > self.max_events {
                events.pop_front();
            }
        }

        if let Some(path) = self.jsonl_path.as_ref() {
            if let Err(err) = append_jsonl(path, &event).await {
                tracing::warn!("Failed to append safety audit event: {}", err);
            }
        }

        event
    }

    pub async fn recent(&self, limit: usize) -> Vec<SafetyAuditEvent> {
        let limit = limit.max(1).min(self.max_events);
        let events = self.events.lock().await;
        events.iter().rev().take(limit).cloned().collect()
    }

    pub fn jsonl_path(&self) -> Option<&PathBuf> {
        self.jsonl_path.as_ref()
    }

    pub fn max_events(&self) -> usize {
        self.max_events
    }
}

async fn append_jsonl(path: &PathBuf, event: &SafetyAuditEvent) -> std::io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut line = serde_json::to_string(event)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;
    line.push('\n');
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(line.as_bytes()).await
}

fn new_event_id() -> String {
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    format!("saf_{}", &uuid[..16])
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
