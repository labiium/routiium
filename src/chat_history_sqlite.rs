//! SQLite chat history storage backend
//!
//! Persistent, single-file storage with SQL query capabilities.
//! Ideal for single-instance deployments and edge computing.

use crate::chat_history::{
    ChatHistoryError, ChatHistoryStore, Conversation, ConversationFilters, CostInfo, MCPInfo,
    Message, MessageFilters, MessageRole, PrivacyLevel, Result, RoutingInfo, StorageStats,
    TokenInfo,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::{ConnectOptions, Row};
use std::str::FromStr;

/// SQLite storage backend
#[derive(Clone)]
pub struct SqliteChatHistoryStore {
    pool: SqlitePool,
}

impl SqliteChatHistoryStore {
    /// Create a new SQLite store with the given database URL
    /// URL format: "sqlite:path/to/database.db" or "sqlite::memory:" for in-memory
    pub async fn new(database_url: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(database_url)
            .map_err(|e| ChatHistoryError::Storage(format!("Invalid database URL: {}", e)))?
            .create_if_missing(true)
            .disable_statement_logging();

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("SQLite connection error: {}", e)))?;

        Ok(Self { pool })
    }

    async fn init_schema(&self) -> Result<()> {
        let schema = include_str!("chat_history_schema.sql");

        // Execute schema in batches (SQLite doesn't support multiple statements in one execute)
        // Split by semicolon and clean up each statement
        let mut statements = Vec::new();
        for statement in schema.split(';') {
            let trimmed = statement
                .lines()
                .filter(|line| {
                    let l = line.trim();
                    !l.is_empty() && !l.starts_with("--")
                })
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string();

            if !trimmed.is_empty() {
                statements.push(trimmed);
            }
        }

        for statement in statements {
            tracing::debug!(
                "Executing SQL: {}",
                statement.chars().take(80).collect::<String>()
            );

            sqlx::query(&statement)
                .execute(&self.pool)
                .await
                .map_err(|e| {
                    ChatHistoryError::Storage(format!(
                        "Schema creation error for statement '{}...': {}",
                        statement.chars().take(80).collect::<String>(),
                        e
                    ))
                })?;
        }

        Ok(())
    }

    fn serialize_json<T: serde::Serialize>(value: &T) -> Result<String> {
        serde_json::to_string(value).map_err(ChatHistoryError::Serialization)
    }

    fn deserialize_json<T: serde::de::DeserializeOwned>(s: &str) -> Result<T> {
        serde_json::from_str(s).map_err(ChatHistoryError::Serialization)
    }
}

#[async_trait::async_trait]
impl ChatHistoryStore for SqliteChatHistoryStore {
    async fn init(&self) -> Result<()> {
        self.init_schema().await
    }

    async fn record_conversation(&self, conversation: &Conversation) -> Result<()> {
        let metadata_json = Self::serialize_json(&conversation.metadata)?;

        sqlx::query(
            r#"
            INSERT INTO conversations (conversation_id, created_at, last_seen_at, title, metadata)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(conversation_id) DO UPDATE SET
                last_seen_at = excluded.last_seen_at,
                title = excluded.title,
                metadata = excluded.metadata
            "#,
        )
        .bind(&conversation.conversation_id)
        .bind(conversation.created_at as i64)
        .bind(conversation.last_seen_at as i64)
        .bind(&conversation.title)
        .bind(&metadata_json)
        .execute(&self.pool)
        .await
        .map_err(|e| ChatHistoryError::Storage(format!("Insert conversation error: {}", e)))?;

        Ok(())
    }

    async fn record_message(&self, message: &Message) -> Result<()> {
        let role_json = Self::serialize_json(&message.role)?;
        let content_json = Self::serialize_json(&message.content)?;
        let tool_calls_json = message
            .tool_calls
            .as_ref()
            .map(|t| Self::serialize_json(t))
            .transpose()?;
        let transformations_json = message
            .routing
            .transformations_applied
            .as_ref()
            .map(|t| Self::serialize_json(t))
            .transpose()?;
        let mcp_servers_json = Self::serialize_json(&message.mcp.mcp_servers)?;
        let cost_info_json = message
            .cost_info
            .as_ref()
            .map(|c| Self::serialize_json(c))
            .transpose()?;
        let privacy_level_json = Self::serialize_json(&message.privacy_level)?;

        sqlx::query(
            r#"
            INSERT INTO messages (
                message_id, conversation_id, request_id, role, content, tool_calls,
                created_at, requested_model, actual_model, backend, backend_url,
                upstream_mode, route_id, transformations_applied, mcp_enabled,
                mcp_servers, system_prompt_applied, input_tokens, output_tokens,
                cached_tokens, reasoning_tokens, cost_info, content_hash, privacy_level
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
            )
            "#,
        )
        .bind(&message.message_id)
        .bind(&message.conversation_id)
        .bind(&message.request_id)
        .bind(&role_json)
        .bind(&content_json)
        .bind(&tool_calls_json)
        .bind(message.created_at as i64)
        .bind(&message.routing.requested_model)
        .bind(&message.routing.actual_model)
        .bind(&message.routing.backend)
        .bind(&message.routing.backend_url)
        .bind(&message.routing.upstream_mode)
        .bind(&message.routing.route_id)
        .bind(&transformations_json)
        .bind(if message.mcp.mcp_enabled { 1i64 } else { 0i64 })
        .bind(&mcp_servers_json)
        .bind(if message.mcp.system_prompt_applied {
            1i64
        } else {
            0i64
        })
        .bind(message.tokens.input_tokens.map(|t| t as i64))
        .bind(message.tokens.output_tokens.map(|t| t as i64))
        .bind(message.tokens.cached_tokens.map(|t| t as i64))
        .bind(message.tokens.reasoning_tokens.map(|t| t as i64))
        .bind(&cost_info_json)
        .bind(&message.content_hash)
        .bind(&privacy_level_json)
        .execute(&self.pool)
        .await
        .map_err(|e| ChatHistoryError::Storage(format!("Insert message error: {}", e)))?;

        Ok(())
    }

    async fn record_messages(&self, messages: &[Message]) -> Result<()> {
        // Use a transaction for batch inserts
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Transaction error: {}", e)))?;

        for message in messages {
            let role_json = Self::serialize_json(&message.role)?;
            let content_json = Self::serialize_json(&message.content)?;
            let tool_calls_json = message
                .tool_calls
                .as_ref()
                .map(|t| Self::serialize_json(t))
                .transpose()?;
            let transformations_json = message
                .routing
                .transformations_applied
                .as_ref()
                .map(|t| Self::serialize_json(t))
                .transpose()?;
            let mcp_servers_json = Self::serialize_json(&message.mcp.mcp_servers)?;
            let cost_info_json = message
                .cost_info
                .as_ref()
                .map(|c| Self::serialize_json(c))
                .transpose()?;
            let privacy_level_json = Self::serialize_json(&message.privacy_level)?;

            sqlx::query(
                r#"
                INSERT INTO messages (
                    message_id, conversation_id, request_id, role, content, tool_calls,
                    created_at, requested_model, actual_model, backend, backend_url,
                    upstream_mode, route_id, transformations_applied, mcp_enabled,
                    mcp_servers, system_prompt_applied, input_tokens, output_tokens,
                    cached_tokens, reasoning_tokens, cost_info, content_hash, privacy_level
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                    ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
                )
                "#,
            )
            .bind(&message.message_id)
            .bind(&message.conversation_id)
            .bind(&message.request_id)
            .bind(&role_json)
            .bind(&content_json)
            .bind(&tool_calls_json)
            .bind(message.created_at as i64)
            .bind(&message.routing.requested_model)
            .bind(&message.routing.actual_model)
            .bind(&message.routing.backend)
            .bind(&message.routing.backend_url)
            .bind(&message.routing.upstream_mode)
            .bind(&message.routing.route_id)
            .bind(&transformations_json)
            .bind(if message.mcp.mcp_enabled { 1i64 } else { 0i64 })
            .bind(&mcp_servers_json)
            .bind(if message.mcp.system_prompt_applied {
                1i64
            } else {
                0i64
            })
            .bind(message.tokens.input_tokens.map(|t| t as i64))
            .bind(message.tokens.output_tokens.map(|t| t as i64))
            .bind(message.tokens.cached_tokens.map(|t| t as i64))
            .bind(message.tokens.reasoning_tokens.map(|t| t as i64))
            .bind(&cost_info_json)
            .bind(&message.content_hash)
            .bind(&privacy_level_json)
            .execute(&mut *tx)
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Insert message error: {}", e)))?;
        }

        tx.commit()
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Transaction commit error: {}", e)))?;

        Ok(())
    }

    async fn get_conversation(&self, conversation_id: &str) -> Result<Conversation> {
        let row = sqlx::query(
            "SELECT conversation_id, created_at, last_seen_at, title, metadata FROM conversations WHERE conversation_id = ?1"
        )
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ChatHistoryError::Storage(format!("Query error: {}", e)))?
        .ok_or_else(|| ChatHistoryError::NotFound(format!("Conversation {} not found", conversation_id)))?;

        let metadata_str: String = row.get("metadata");
        let metadata = Self::deserialize_json(&metadata_str)?;

        Ok(Conversation {
            conversation_id: row.get("conversation_id"),
            created_at: row.get::<i64, _>("created_at") as u64,
            last_seen_at: row.get::<i64, _>("last_seen_at") as u64,
            title: row.get("title"),
            metadata,
        })
    }

    async fn list_conversations(&self, filters: &ConversationFilters) -> Result<Vec<Conversation>> {
        let mut query = String::from("SELECT conversation_id, created_at, last_seen_at, title, metadata FROM conversations WHERE 1=1");

        if filters.start_time.is_some() {
            query.push_str(" AND created_at >= ?");
        }
        if filters.end_time.is_some() {
            query.push_str(" AND created_at <= ?");
        }

        query.push_str(" ORDER BY last_seen_at DESC");

        if let Some(limit) = filters.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        let mut sql_query = sqlx::query(&query);

        if let Some(start) = filters.start_time {
            sql_query = sql_query.bind(start as i64);
        }
        if let Some(end) = filters.end_time {
            sql_query = sql_query.bind(end as i64);
        }

        let rows = sql_query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Query error: {}", e)))?;

        let mut conversations = Vec::new();
        for row in rows {
            let metadata_str: String = row.get("metadata");
            let metadata = Self::deserialize_json(&metadata_str)?;

            conversations.push(Conversation {
                conversation_id: row.get("conversation_id"),
                created_at: row.get::<i64, _>("created_at") as u64,
                last_seen_at: row.get::<i64, _>("last_seen_at") as u64,
                title: row.get("title"),
                metadata,
            });
        }

        Ok(conversations)
    }

    async fn list_messages(&self, filters: &MessageFilters) -> Result<Vec<Message>> {
        let mut query = String::from("SELECT * FROM messages WHERE 1=1");
        let mut bind_count = 0;

        if filters.conversation_id.is_some() {
            bind_count += 1;
            query.push_str(&format!(" AND conversation_id = ?{}", bind_count));
        }
        if filters.request_id.is_some() {
            bind_count += 1;
            query.push_str(&format!(" AND request_id = ?{}", bind_count));
        }
        if filters.start_time.is_some() {
            bind_count += 1;
            query.push_str(&format!(" AND created_at >= ?{}", bind_count));
        }
        if filters.end_time.is_some() {
            bind_count += 1;
            query.push_str(&format!(" AND created_at <= ?{}", bind_count));
        }
        if filters.backend.is_some() {
            bind_count += 1;
            query.push_str(&format!(" AND backend = ?{}", bind_count));
        }
        if filters.requested_model.is_some() {
            bind_count += 1;
            query.push_str(&format!(" AND requested_model = ?{}", bind_count));
        }
        if filters.actual_model.is_some() {
            bind_count += 1;
            query.push_str(&format!(" AND actual_model = ?{}", bind_count));
        }
        if filters.route_id.is_some() {
            bind_count += 1;
            query.push_str(&format!(" AND route_id = ?{}", bind_count));
        }
        if filters.mcp_enabled.is_some() {
            bind_count += 1;
            query.push_str(&format!(" AND mcp_enabled = ?{}", bind_count));
        }
        if filters.upstream_mode.is_some() {
            bind_count += 1;
            query.push_str(&format!(" AND upstream_mode = ?{}", bind_count));
        }

        query.push_str(" ORDER BY created_at ASC");

        if let Some(limit) = filters.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        let mut sql_query = sqlx::query(&query);

        if let Some(ref conv_id) = filters.conversation_id {
            sql_query = sql_query.bind(conv_id);
        }
        if let Some(ref req_id) = filters.request_id {
            sql_query = sql_query.bind(req_id);
        }
        if let Some(start) = filters.start_time {
            sql_query = sql_query.bind(start as i64);
        }
        if let Some(end) = filters.end_time {
            sql_query = sql_query.bind(end as i64);
        }
        if let Some(ref backend) = filters.backend {
            sql_query = sql_query.bind(backend);
        }
        if let Some(ref model) = filters.requested_model {
            sql_query = sql_query.bind(model);
        }
        if let Some(ref model) = filters.actual_model {
            sql_query = sql_query.bind(model);
        }
        if let Some(ref route_id) = filters.route_id {
            sql_query = sql_query.bind(route_id);
        }
        if let Some(mcp) = filters.mcp_enabled {
            sql_query = sql_query.bind(if mcp { 1i64 } else { 0i64 });
        }
        if let Some(ref mode) = filters.upstream_mode {
            sql_query = sql_query.bind(mode);
        }

        let rows = sql_query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Query error: {}", e)))?;

        let mut messages = Vec::new();
        for row in rows {
            let role_str: String = row.get("role");
            let role: MessageRole = Self::deserialize_json(&role_str)?;

            let content_str: String = row.get("content");
            let content: serde_json::Value = Self::deserialize_json(&content_str)?;

            let tool_calls: Option<serde_json::Value> = row
                .try_get::<Option<String>, _>("tool_calls")
                .ok()
                .flatten()
                .map(|s| Self::deserialize_json(&s))
                .transpose()?;

            let transformations_applied: Option<serde_json::Value> = row
                .try_get::<Option<String>, _>("transformations_applied")
                .ok()
                .flatten()
                .map(|s| Self::deserialize_json(&s))
                .transpose()?;

            let mcp_servers_str: String = row.get("mcp_servers");
            let mcp_servers: Vec<String> = Self::deserialize_json(&mcp_servers_str)?;

            let cost_info: Option<CostInfo> = row
                .try_get::<Option<String>, _>("cost_info")
                .ok()
                .flatten()
                .map(|s| Self::deserialize_json(&s))
                .transpose()?;

            let privacy_level_str: String = row.get("privacy_level");
            let privacy_level: PrivacyLevel = Self::deserialize_json(&privacy_level_str)?;

            messages.push(Message {
                message_id: row.get("message_id"),
                conversation_id: row.get("conversation_id"),
                request_id: row.get("request_id"),
                role,
                content,
                tool_calls,
                created_at: row.get::<i64, _>("created_at") as u64,
                routing: RoutingInfo {
                    requested_model: row.get("requested_model"),
                    actual_model: row.get("actual_model"),
                    backend: row.get("backend"),
                    backend_url: row.get("backend_url"),
                    upstream_mode: row.get("upstream_mode"),
                    route_id: row.get("route_id"),
                    transformations_applied,
                },
                mcp: MCPInfo {
                    mcp_enabled: row.get::<i64, _>("mcp_enabled") != 0,
                    mcp_servers,
                    system_prompt_applied: row.get::<i64, _>("system_prompt_applied") != 0,
                },
                tokens: TokenInfo {
                    input_tokens: row.get::<Option<i64>, _>("input_tokens").map(|t| t as u64),
                    output_tokens: row.get::<Option<i64>, _>("output_tokens").map(|t| t as u64),
                    cached_tokens: row.get::<Option<i64>, _>("cached_tokens").map(|t| t as u64),
                    reasoning_tokens: row
                        .get::<Option<i64>, _>("reasoning_tokens")
                        .map(|t| t as u64),
                },
                cost_info,
                content_hash: row.get("content_hash"),
                privacy_level,
            });
        }

        Ok(messages)
    }

    async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM conversations WHERE conversation_id = ?1")
            .bind(conversation_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Delete error: {}", e)))?;
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        sqlx::query("DELETE FROM messages")
            .execute(&self.pool)
            .await
            .map_err(|e| ChatHistoryError::Storage(e.to_string()))?;
        sqlx::query("DELETE FROM conversations")
            .execute(&self.pool)
            .await
            .map_err(|e| ChatHistoryError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn stats(&self) -> Result<StorageStats> {
        let conv_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| ChatHistoryError::Storage(e.to_string()))?;

        let msg_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| ChatHistoryError::Storage(e.to_string()))?;

        Ok(StorageStats {
            total_conversations: conv_count as usize,
            total_messages: msg_count as usize,
            backend_type: "sqlite".to_string(),
            storage_path: None,
        })
    }

    async fn health(&self) -> Result<bool> {
        // Try a simple query to check if the database is accessible
        sqlx::query("SELECT 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Health check error: {}", e)))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_history::{MessageRole, PrivacyLevel};

    #[tokio::test]
    async fn test_sqlite_store_conversation() {
        let store = SqliteChatHistoryStore::new("sqlite::memory:")
            .await
            .unwrap();
        store.init().await.unwrap();

        let conv = Conversation::new("conv_123".to_string());
        store.record_conversation(&conv).await.unwrap();

        let retrieved = store.get_conversation("conv_123").await.unwrap();
        assert_eq!(retrieved.conversation_id, "conv_123");
    }

    #[tokio::test]
    async fn test_sqlite_store_message() {
        let store = SqliteChatHistoryStore::new("sqlite::memory:")
            .await
            .unwrap();
        store.init().await.unwrap();

        // Create conversation first
        let conv = Conversation::new("conv_123".to_string());
        store.record_conversation(&conv).await.unwrap();

        let msg = Message::new(
            "conv_123".to_string(),
            MessageRole::User,
            serde_json::json!({"text": "Hello"}),
            PrivacyLevel::Full,
        );

        store.record_message(&msg).await.unwrap();

        let filters = MessageFilters {
            conversation_id: Some("conv_123".to_string()),
            ..Default::default()
        };

        let messages = store.list_messages(&filters).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].conversation_id, "conv_123");
    }

    #[tokio::test]
    async fn test_sqlite_store_batch_messages() {
        let store = SqliteChatHistoryStore::new("sqlite::memory:")
            .await
            .unwrap();
        store.init().await.unwrap();

        // Create conversation first
        let conv = Conversation::new("conv_123".to_string());
        store.record_conversation(&conv).await.unwrap();

        let messages = vec![
            Message::new(
                "conv_123".to_string(),
                MessageRole::User,
                serde_json::json!({"text": "Hello"}),
                PrivacyLevel::Full,
            ),
            Message::new(
                "conv_123".to_string(),
                MessageRole::Assistant,
                serde_json::json!({"text": "Hi"}),
                PrivacyLevel::Full,
            ),
        ];

        store.record_messages(&messages).await.unwrap();

        let filters = MessageFilters {
            conversation_id: Some("conv_123".to_string()),
            ..Default::default()
        };

        let stored = store.list_messages(&filters).await.unwrap();
        assert_eq!(stored.len(), 2);
    }

    #[tokio::test]
    async fn test_sqlite_store_filters() {
        let store = SqliteChatHistoryStore::new("sqlite::memory:")
            .await
            .unwrap();
        store.init().await.unwrap();

        // Create conversation first
        let conv = Conversation::new("conv_1".to_string());
        store.record_conversation(&conv).await.unwrap();

        let msg1 = Message::new(
            "conv_1".to_string(),
            MessageRole::User,
            serde_json::json!({"text": "Hello"}),
            PrivacyLevel::Full,
        )
        .with_routing(RoutingInfo {
            backend: Some("openai".to_string()),
            requested_model: Some("gpt-4".to_string()),
            actual_model: Some("gpt-4-0613".to_string()),
            ..Default::default()
        });

        let msg2 = Message::new(
            "conv_1".to_string(),
            MessageRole::Assistant,
            serde_json::json!({"text": "Hi"}),
            PrivacyLevel::Full,
        )
        .with_routing(RoutingInfo {
            backend: Some("anthropic".to_string()),
            requested_model: Some("gpt-4".to_string()),
            actual_model: Some("claude-3-opus".to_string()),
            ..Default::default()
        });

        store.record_message(&msg1).await.unwrap();
        store.record_message(&msg2).await.unwrap();

        // Filter by backend
        let filters = MessageFilters {
            backend: Some("openai".to_string()),
            ..Default::default()
        };
        let messages = store.list_messages(&filters).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].routing.backend, Some("openai".to_string()));
    }

    #[tokio::test]
    async fn test_sqlite_store_delete() {
        let store = SqliteChatHistoryStore::new("sqlite::memory:")
            .await
            .unwrap();
        store.init().await.unwrap();

        let conv = Conversation::new("conv_123".to_string());
        store.record_conversation(&conv).await.unwrap();

        let msg = Message::new(
            "conv_123".to_string(),
            MessageRole::User,
            serde_json::json!({"text": "Hello"}),
            PrivacyLevel::Full,
        );
        store.record_message(&msg).await.unwrap();

        store.delete_conversation("conv_123").await.unwrap();

        assert!(store.get_conversation("conv_123").await.is_err());
    }

    #[tokio::test]
    async fn test_sqlite_store_clear() {
        let store = SqliteChatHistoryStore::new("sqlite::memory:")
            .await
            .unwrap();
        store.init().await.unwrap();

        let conv = Conversation::new("conv_123".to_string());
        store.record_conversation(&conv).await.unwrap();

        store.clear().await.unwrap();

        let stats = store.stats().await.unwrap();
        assert_eq!(stats.total_conversations, 0);
        assert_eq!(stats.total_messages, 0);
    }

    #[tokio::test]
    async fn test_sqlite_store_health() {
        let store = SqliteChatHistoryStore::new("sqlite::memory:")
            .await
            .unwrap();
        store.init().await.unwrap();

        let healthy = store.health().await.unwrap();
        assert!(healthy);
    }
}
