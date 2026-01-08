//! Chat History Module
//!
//! Provides conversation and message tracking with multiple storage backends.
//! Captures full routing context, model transformations, MCP usage, and cost information.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum ChatHistoryError {
    #[error("Storage error: {0}")]
    Storage(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ChatHistoryError>;

/// Privacy level for message content storage
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PrivacyLevel {
    /// Store only metadata, no content
    Off,
    /// Store summary/fingerprint only
    Summary,
    /// Store full content
    #[default]
    Full,
}

/// Conversation metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub conversation_id: String,
    pub created_at: u64,
    pub last_seen_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Conversation {
    pub fn new(conversation_id: String) -> Self {
        let now = current_timestamp();
        Self {
            conversation_id,
            created_at: now,
            last_seen_at: now,
            title: None,
            metadata: HashMap::new(),
        }
    }

    pub fn touch(&mut self) {
        self.last_seen_at = current_timestamp();
    }
}

/// Message role
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// Model and routing information captured from request processing
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutingInfo {
    /// Model requested by client
    pub requested_model: Option<String>,
    /// Actual model sent to upstream after transformations
    pub actual_model: Option<String>,
    /// Backend provider (openai, anthropic, bedrock, etc.)
    pub backend: Option<String>,
    /// Actual base URL used
    pub backend_url: Option<String>,
    /// API mode (chat or responses)
    pub upstream_mode: Option<String>,
    /// Routing rule ID that matched
    pub route_id: Option<String>,
    /// JSON of transformations applied
    pub transformations_applied: Option<serde_json::Value>,
}

/// MCP and system prompt information
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MCPInfo {
    /// Whether MCP was used
    pub mcp_enabled: bool,
    /// MCP servers invoked
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    /// Whether system prompt was injected
    pub system_prompt_applied: bool,
}

/// Token usage details
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenInfo {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

impl TokenInfo {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.unwrap_or(0) + self.output_tokens.unwrap_or(0)
    }
}

/// Cost information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostInfo {
    pub input_cost: f64,
    pub output_cost: f64,
    pub cached_cost: Option<f64>,
    pub total_cost: f64,
    pub currency: String,
}

/// A single message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub message_id: String,
    pub conversation_id: String,
    pub request_id: Option<String>,
    pub role: MessageRole,
    pub content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
    pub created_at: u64,

    // Routing information
    #[serde(flatten)]
    pub routing: RoutingInfo,

    // MCP and system prompts
    #[serde(flatten)]
    pub mcp: MCPInfo,

    // Token usage and cost
    #[serde(flatten)]
    pub tokens: TokenInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_info: Option<CostInfo>,

    // Metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub privacy_level: PrivacyLevel,
}

impl Message {
    pub fn new(
        conversation_id: String,
        role: MessageRole,
        content: serde_json::Value,
        privacy_level: PrivacyLevel,
    ) -> Self {
        let message_id = Uuid::new_v4().to_string();
        let created_at = current_timestamp();

        // Apply privacy level to content
        let (content, content_hash) = match privacy_level {
            PrivacyLevel::Off => {
                let hash = calculate_content_hash(&content);
                (serde_json::json!({"redacted": true}), Some(hash))
            }
            PrivacyLevel::Summary => {
                let hash = calculate_content_hash(&content);
                let summary = generate_content_summary(&content);
                (serde_json::json!({"summary": summary}), Some(hash))
            }
            PrivacyLevel::Full => {
                let hash = calculate_content_hash(&content);
                (content, Some(hash))
            }
        };

        Self {
            message_id,
            conversation_id,
            request_id: None,
            role,
            content,
            tool_calls: None,
            created_at,
            routing: RoutingInfo::default(),
            mcp: MCPInfo::default(),
            tokens: TokenInfo::default(),
            cost_info: None,
            content_hash,
            privacy_level,
        }
    }

    pub fn with_request_id(mut self, request_id: String) -> Self {
        self.request_id = Some(request_id);
        self
    }

    pub fn with_routing(mut self, routing: RoutingInfo) -> Self {
        self.routing = routing;
        self
    }

    pub fn with_mcp(mut self, mcp: MCPInfo) -> Self {
        self.mcp = mcp;
        self
    }

    pub fn with_tokens(mut self, tokens: TokenInfo) -> Self {
        self.tokens = tokens;
        self
    }

    pub fn with_cost(mut self, cost: CostInfo) -> Self {
        self.cost_info = Some(cost);
        self
    }

    pub fn with_tool_calls(mut self, tool_calls: serde_json::Value) -> Self {
        self.tool_calls = Some(tool_calls);
        self
    }
}

/// Query filters for conversations
#[derive(Debug, Clone, Default)]
pub struct ConversationFilters {
    pub start_time: Option<u64>,
    pub end_time: Option<u64>,
    pub limit: Option<usize>,
}

/// Query filters for messages
#[derive(Debug, Clone, Default)]
pub struct MessageFilters {
    pub conversation_id: Option<String>,
    pub request_id: Option<String>,
    pub start_time: Option<u64>,
    pub end_time: Option<u64>,
    pub backend: Option<String>,
    pub requested_model: Option<String>,
    pub actual_model: Option<String>,
    pub route_id: Option<String>,
    pub mcp_enabled: Option<bool>,
    pub upstream_mode: Option<String>,
    pub limit: Option<usize>,
}

/// Storage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    pub total_conversations: usize,
    pub total_messages: usize,
    pub backend_type: String,
    pub storage_path: Option<String>,
}

/// Chat history storage trait
#[async_trait::async_trait]
pub trait ChatHistoryStore: Send + Sync {
    /// Initialize the storage backend
    async fn init(&self) -> Result<()>;

    /// Record a conversation
    async fn record_conversation(&self, conversation: &Conversation) -> Result<()>;

    /// Record a message
    async fn record_message(&self, message: &Message) -> Result<()>;

    /// Record multiple messages in a batch
    async fn record_messages(&self, messages: &[Message]) -> Result<()> {
        for message in messages {
            self.record_message(message).await?;
        }
        Ok(())
    }

    /// Get a conversation by ID
    async fn get_conversation(&self, conversation_id: &str) -> Result<Conversation>;

    /// List conversations with filters
    async fn list_conversations(&self, filters: &ConversationFilters) -> Result<Vec<Conversation>>;

    /// List messages with filters
    async fn list_messages(&self, filters: &MessageFilters) -> Result<Vec<Message>>;

    /// Delete a conversation and all its messages
    async fn delete_conversation(&self, conversation_id: &str) -> Result<()>;

    /// Clear all data (admin only)
    async fn clear(&self) -> Result<()>;

    /// Get storage statistics
    async fn stats(&self) -> Result<StorageStats>;

    /// Health check
    async fn health(&self) -> Result<bool> {
        Ok(true)
    }
}

/// Multi-backend composite store
pub struct CompositeStore {
    primary: Box<dyn ChatHistoryStore>,
    sinks: Vec<Box<dyn ChatHistoryStore>>,
    strict: bool,
}

impl CompositeStore {
    pub fn new(primary: Box<dyn ChatHistoryStore>, strict: bool) -> Self {
        Self {
            primary,
            sinks: Vec::new(),
            strict,
        }
    }

    pub fn add_sink(&mut self, sink: Box<dyn ChatHistoryStore>) {
        self.sinks.push(sink);
    }
}

#[async_trait::async_trait]
impl ChatHistoryStore for CompositeStore {
    async fn init(&self) -> Result<()> {
        self.primary.init().await?;
        for sink in &self.sinks {
            if let Err(e) = sink.init().await {
                if self.strict {
                    return Err(e);
                }
                tracing::warn!("Sink init failed: {}", e);
            }
        }
        Ok(())
    }

    async fn record_conversation(&self, conversation: &Conversation) -> Result<()> {
        self.primary.record_conversation(conversation).await?;
        for sink in &self.sinks {
            if let Err(e) = sink.record_conversation(conversation).await {
                if self.strict {
                    return Err(e);
                }
                tracing::warn!("Sink record_conversation failed: {}", e);
            }
        }
        Ok(())
    }

    async fn record_message(&self, message: &Message) -> Result<()> {
        self.primary.record_message(message).await?;
        for sink in &self.sinks {
            if let Err(e) = sink.record_message(message).await {
                if self.strict {
                    return Err(e);
                }
                tracing::warn!("Sink record_message failed: {}", e);
            }
        }
        Ok(())
    }

    async fn record_messages(&self, messages: &[Message]) -> Result<()> {
        self.primary.record_messages(messages).await?;
        for sink in &self.sinks {
            if let Err(e) = sink.record_messages(messages).await {
                if self.strict {
                    return Err(e);
                }
                tracing::warn!("Sink record_messages failed: {}", e);
            }
        }
        Ok(())
    }

    async fn get_conversation(&self, conversation_id: &str) -> Result<Conversation> {
        self.primary.get_conversation(conversation_id).await
    }

    async fn list_conversations(&self, filters: &ConversationFilters) -> Result<Vec<Conversation>> {
        self.primary.list_conversations(filters).await
    }

    async fn list_messages(&self, filters: &MessageFilters) -> Result<Vec<Message>> {
        self.primary.list_messages(filters).await
    }

    async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        self.primary.delete_conversation(conversation_id).await?;
        for sink in &self.sinks {
            if let Err(e) = sink.delete_conversation(conversation_id).await {
                if self.strict {
                    return Err(e);
                }
                tracing::warn!("Sink delete_conversation failed: {}", e);
            }
        }
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        self.primary.clear().await?;
        for sink in &self.sinks {
            if let Err(e) = sink.clear().await {
                if self.strict {
                    return Err(e);
                }
                tracing::warn!("Sink clear failed: {}", e);
            }
        }
        Ok(())
    }

    async fn stats(&self) -> Result<StorageStats> {
        self.primary.stats().await
    }

    async fn health(&self) -> Result<bool> {
        let primary_ok = self.primary.health().await?;
        if !primary_ok {
            return Ok(false);
        }
        // Sinks don't affect health in non-strict mode
        if self.strict {
            for sink in &self.sinks {
                if !sink.health().await? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

// Helper functions

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn calculate_content_hash(content: &serde_json::Value) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let content_str = content.to_string();
    let mut hasher = DefaultHasher::new();
    content_str.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn generate_content_summary(content: &serde_json::Value) -> String {
    let content_str = content.to_string();
    if content_str.len() > 100 {
        format!("{}...", &content_str[..100])
    } else {
        content_str
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_creation() {
        let conv = Conversation::new("conv_123".to_string());
        assert_eq!(conv.conversation_id, "conv_123");
        assert!(conv.created_at > 0);
        assert_eq!(conv.created_at, conv.last_seen_at);
    }

    #[test]
    fn test_conversation_touch() {
        let mut conv = Conversation::new("conv_123".to_string());
        let original_time = conv.last_seen_at;
        std::thread::sleep(std::time::Duration::from_millis(10));
        conv.touch();
        assert!(conv.last_seen_at >= original_time);
    }

    #[test]
    fn test_message_creation() {
        let content = serde_json::json!({"text": "Hello, world!"});
        let msg = Message::new(
            "conv_123".to_string(),
            MessageRole::User,
            content,
            PrivacyLevel::Full,
        );
        assert_eq!(msg.conversation_id, "conv_123");
        assert_eq!(msg.role, MessageRole::User);
        assert!(!msg.message_id.is_empty());
        assert!(msg.created_at > 0);
    }

    #[test]
    fn test_message_privacy_off() {
        let content = serde_json::json!({"text": "Secret message"});
        let msg = Message::new(
            "conv_123".to_string(),
            MessageRole::User,
            content,
            PrivacyLevel::Off,
        );
        assert_eq!(msg.content, serde_json::json!({"redacted": true}));
        assert!(msg.content_hash.is_some());
    }

    #[test]
    fn test_message_privacy_summary() {
        let content = serde_json::json!({"text": "Test message"});
        let msg = Message::new(
            "conv_123".to_string(),
            MessageRole::User,
            content,
            PrivacyLevel::Summary,
        );
        assert!(msg.content.get("summary").is_some());
        assert!(msg.content_hash.is_some());
    }

    #[test]
    fn test_message_builder() {
        let content = serde_json::json!({"text": "Hello"});
        let msg = Message::new(
            "conv_123".to_string(),
            MessageRole::User,
            content,
            PrivacyLevel::Full,
        )
        .with_request_id("req_456".to_string())
        .with_routing(RoutingInfo {
            requested_model: Some("gpt-4".to_string()),
            actual_model: Some("gpt-4-0613".to_string()),
            backend: Some("openai".to_string()),
            ..Default::default()
        })
        .with_tokens(TokenInfo {
            input_tokens: Some(10),
            output_tokens: Some(20),
            ..Default::default()
        });

        assert_eq!(msg.request_id, Some("req_456".to_string()));
        assert_eq!(msg.routing.requested_model, Some("gpt-4".to_string()));
        assert_eq!(msg.tokens.input_tokens, Some(10));
        assert_eq!(msg.tokens.total_tokens(), 30);
    }

    #[test]
    fn test_privacy_level_serialization() {
        let json = serde_json::to_string(&PrivacyLevel::Full).unwrap();
        assert_eq!(json, "\"full\"");

        let level: PrivacyLevel = serde_json::from_str("\"summary\"").unwrap();
        assert_eq!(level, PrivacyLevel::Summary);
    }

    #[test]
    fn test_message_role_serialization() {
        let json = serde_json::to_string(&MessageRole::Assistant).unwrap();
        assert_eq!(json, "\"assistant\"");

        let role: MessageRole = serde_json::from_str("\"user\"").unwrap();
        assert_eq!(role, MessageRole::User);
    }
}
