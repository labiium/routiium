//! Chat History Manager
//!
//! Coordinates chat history storage, configuration, and integration with analytics.

use crate::chat_history::{
    ChatHistoryError, ChatHistoryStore, Conversation, ConversationFilters, Message, MessageFilters,
    PrivacyLevel, Result, StorageStats,
};
use crate::chat_history_jsonl::JsonlChatHistoryStore;
use crate::chat_history_memory::MemoryChatHistoryStore;

#[cfg(feature = "sqlite")]
use crate::chat_history_sqlite::SqliteChatHistoryStore;

#[cfg(feature = "postgres")]
use crate::chat_history_postgres::PostgresChatHistoryStore;

#[cfg(feature = "turso")]
use crate::chat_history_turso::TursoChatHistoryStore;

use std::sync::Arc;

/// Configuration for chat history
#[derive(Debug, Clone)]
pub struct ChatHistoryConfig {
    /// Whether chat history is enabled
    pub enabled: bool,
    /// Primary storage backend
    pub primary_backend: String,
    /// Sink backends (best-effort)
    pub sink_backends: Vec<String>,
    /// Privacy level
    pub privacy_level: PrivacyLevel,
    /// TTL in seconds (0 = no expiration)
    pub ttl_seconds: u64,
    /// Strict mode (fail if primary fails)
    pub strict: bool,
    /// JSONL file path
    pub jsonl_path: Option<String>,
    /// Max messages for memory backend
    pub memory_max_messages: Option<usize>,
    /// SQLite database URL
    pub sqlite_url: Option<String>,
    /// PostgreSQL database URL
    pub postgres_url: Option<String>,
    /// Turso database URL
    pub turso_url: Option<String>,
    /// Turso auth token
    pub turso_auth_token: Option<String>,
}

impl Default for ChatHistoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            primary_backend: "memory".to_string(),
            sink_backends: Vec::new(),
            privacy_level: PrivacyLevel::Full,
            ttl_seconds: 2592000, // 30 days
            strict: false,
            jsonl_path: Some("./data/chat_history.jsonl".to_string()),
            memory_max_messages: Some(10000),
            sqlite_url: None,
            postgres_url: None,
            turso_url: None,
            turso_auth_token: None,
        }
    }
}

impl ChatHistoryConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        let enabled = std::env::var("ROUTIIUM_CHAT_HISTORY_ENABLED")
            .unwrap_or_else(|_| "false".to_string())
            .parse()
            .unwrap_or(false);

        let primary_backend =
            std::env::var("ROUTIIUM_CHAT_HISTORY_PRIMARY").unwrap_or_else(|_| "memory".to_string());

        let sink_backends: Vec<String> = std::env::var("ROUTIIUM_CHAT_HISTORY_SINKS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let privacy_level = match std::env::var("ROUTIIUM_CHAT_HISTORY_PRIVACY")
            .unwrap_or_else(|_| "full".to_string())
            .to_lowercase()
            .as_str()
        {
            "off" => PrivacyLevel::Off,
            "summary" => PrivacyLevel::Summary,
            _ => PrivacyLevel::Full,
        };

        let ttl_seconds = std::env::var("ROUTIIUM_CHAT_HISTORY_TTL_SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2592000);

        let strict = std::env::var("ROUTIIUM_CHAT_HISTORY_STRICT")
            .unwrap_or_else(|_| "false".to_string())
            .parse()
            .unwrap_or(false);

        let jsonl_path = std::env::var("ROUTIIUM_CHAT_HISTORY_JSONL_PATH")
            .ok()
            .or_else(|| Some("./data/chat_history.jsonl".to_string()));

        let memory_max_messages = std::env::var("ROUTIIUM_CHAT_HISTORY_MEMORY_MAX_MESSAGES")
            .ok()
            .and_then(|s| s.parse().ok());

        let sqlite_url = std::env::var("ROUTIIUM_CHAT_HISTORY_SQLITE_URL").ok();

        let postgres_url = std::env::var("ROUTIIUM_CHAT_HISTORY_POSTGRES_URL").ok();

        let turso_url = std::env::var("ROUTIIUM_CHAT_HISTORY_TURSO_URL").ok();

        let turso_auth_token = std::env::var("ROUTIIUM_CHAT_HISTORY_TURSO_AUTH_TOKEN").ok();

        Self {
            enabled,
            primary_backend,
            sink_backends,
            privacy_level,
            ttl_seconds,
            strict,
            jsonl_path,
            memory_max_messages,
            sqlite_url,
            postgres_url,
            turso_url,
            turso_auth_token,
        }
    }
}

/// Chat history manager
pub struct ChatHistoryManager {
    store: Arc<dyn ChatHistoryStore>,
    config: ChatHistoryConfig,
}

impl ChatHistoryManager {
    /// Create a new chat history manager with the given configuration
    pub async fn new(config: ChatHistoryConfig) -> Result<Self> {
        let store = Self::create_store(&config).await?;
        store.init().await?;

        Ok(Self { store, config })
    }

    /// Create storage backend based on configuration
    async fn create_store(config: &ChatHistoryConfig) -> Result<Arc<dyn ChatHistoryStore>> {
        let primary = Self::create_backend(&config.primary_backend, config).await?;

        if config.sink_backends.is_empty() {
            // Single backend
            Ok(Arc::from(primary))
        } else {
            // Multi-backend with composite store
            let mut composite = crate::chat_history::CompositeStore::new(primary, config.strict);

            for sink_name in &config.sink_backends {
                let sink = Self::create_backend(sink_name, config).await?;
                composite.add_sink(sink);
            }

            Ok(Arc::new(composite))
        }
    }

    async fn create_backend(
        backend: &str,
        config: &ChatHistoryConfig,
    ) -> Result<Box<dyn ChatHistoryStore>> {
        match backend {
            "memory" => {
                let store = if let Some(max) = config.memory_max_messages {
                    MemoryChatHistoryStore::with_max_messages(max)
                } else {
                    MemoryChatHistoryStore::new()
                };
                Ok(Box::new(store))
            }
            "jsonl" => {
                let path = config.jsonl_path.as_ref().ok_or_else(|| {
                    ChatHistoryError::InvalidInput("JSONL path not configured".to_string())
                })?;
                Ok(Box::new(JsonlChatHistoryStore::new(path)))
            }
            #[cfg(feature = "sqlite")]
            "sqlite" => {
                let url = config.sqlite_url.as_ref().ok_or_else(|| {
                    ChatHistoryError::InvalidInput("SQLite URL not configured".to_string())
                })?;
                let store = SqliteChatHistoryStore::new(url).await?;
                Ok(Box::new(store))
            }
            #[cfg(feature = "postgres")]
            "postgres" | "postgresql" => {
                let url = config.postgres_url.as_ref().ok_or_else(|| {
                    ChatHistoryError::InvalidInput("PostgreSQL URL not configured".to_string())
                })?;
                let store = PostgresChatHistoryStore::new(url).await?;
                Ok(Box::new(store))
            }
            #[cfg(feature = "turso")]
            "turso" | "libsql" => {
                let url = config.turso_url.as_ref().ok_or_else(|| {
                    ChatHistoryError::InvalidInput("Turso URL not configured".to_string())
                })?;
                let store =
                    TursoChatHistoryStore::new(url, config.turso_auth_token.clone()).await?;
                Ok(Box::new(store))
            }
            _ => Err(ChatHistoryError::InvalidInput(format!(
                "Unknown backend: {}",
                backend
            ))),
        }
    }

    /// Check if chat history is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get the privacy level
    pub fn privacy_level(&self) -> PrivacyLevel {
        self.config.privacy_level
    }

    /// Record a conversation
    pub async fn record_conversation(&self, conversation: &Conversation) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        self.store.record_conversation(conversation).await
    }

    /// Record a message
    pub async fn record_message(&self, message: &Message) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        self.store.record_message(message).await
    }

    /// Record multiple messages
    pub async fn record_messages(&self, messages: &[Message]) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        self.store.record_messages(messages).await
    }

    /// Get a conversation by ID
    pub async fn get_conversation(&self, conversation_id: &str) -> Result<Conversation> {
        self.store.get_conversation(conversation_id).await
    }

    /// List conversations with filters
    pub async fn list_conversations(
        &self,
        filters: &ConversationFilters,
    ) -> Result<Vec<Conversation>> {
        self.store.list_conversations(filters).await
    }

    /// List messages with filters
    pub async fn list_messages(&self, filters: &MessageFilters) -> Result<Vec<Message>> {
        self.store.list_messages(filters).await
    }

    /// Delete a conversation
    pub async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        self.store.delete_conversation(conversation_id).await
    }

    /// Clear all data
    pub async fn clear(&self) -> Result<()> {
        self.store.clear().await
    }

    /// Get storage statistics
    pub async fn stats(&self) -> Result<StorageStats> {
        self.store.stats().await
    }

    /// Health check
    pub async fn health(&self) -> Result<bool> {
        self.store.health().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = ChatHistoryConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.primary_backend, "memory");
        assert_eq!(config.privacy_level, PrivacyLevel::Full);
    }

    #[tokio::test]
    async fn test_manager_memory_backend() {
        let config = ChatHistoryConfig {
            enabled: true,
            primary_backend: "memory".to_string(),
            ..Default::default()
        };

        let manager = ChatHistoryManager::new(config).await.unwrap();
        assert!(manager.is_enabled());

        let conv = Conversation::new("conv_123".to_string());
        manager.record_conversation(&conv).await.unwrap();

        let retrieved = manager.get_conversation("conv_123").await.unwrap();
        assert_eq!(retrieved.conversation_id, "conv_123");
    }

    #[tokio::test]
    async fn test_manager_disabled() {
        let config = ChatHistoryConfig {
            enabled: false,
            ..Default::default()
        };

        let manager = ChatHistoryManager::new(config).await.unwrap();
        assert!(!manager.is_enabled());

        // Should not error even when disabled
        let conv = Conversation::new("conv_123".to_string());
        manager.record_conversation(&conv).await.unwrap();
    }

    #[tokio::test]
    async fn test_manager_jsonl_backend() {
        use tempfile::NamedTempFile;

        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let config = ChatHistoryConfig {
            enabled: true,
            primary_backend: "jsonl".to_string(),
            jsonl_path: Some(path),
            ..Default::default()
        };

        let manager = ChatHistoryManager::new(config).await.unwrap();

        let conv = Conversation::new("conv_123".to_string());
        manager.record_conversation(&conv).await.unwrap();

        let retrieved = manager.get_conversation("conv_123").await.unwrap();
        assert_eq!(retrieved.conversation_id, "conv_123");
    }

    #[tokio::test]
    async fn test_manager_composite_backend() {
        use tempfile::NamedTempFile;

        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let config = ChatHistoryConfig {
            enabled: true,
            primary_backend: "memory".to_string(),
            sink_backends: vec!["jsonl".to_string()],
            jsonl_path: Some(path.clone()),
            strict: false,
            ..Default::default()
        };

        let manager = ChatHistoryManager::new(config).await.unwrap();

        let conv = Conversation::new("conv_123".to_string());
        manager.record_conversation(&conv).await.unwrap();

        // Should be in primary (memory)
        let retrieved = manager.get_conversation("conv_123").await.unwrap();
        assert_eq!(retrieved.conversation_id, "conv_123");

        // Should also be in sink (jsonl)
        let jsonl_store = JsonlChatHistoryStore::new(&path);
        let from_sink = jsonl_store.get_conversation("conv_123").await.unwrap();
        assert_eq!(from_sink.conversation_id, "conv_123");
    }
}
