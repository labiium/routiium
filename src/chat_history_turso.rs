//! Turso (libsql) chat history storage backend
//!
//! Cloud-native SQLite storage powered by Turso/libsql.
//! Ideal for edge deployments with global replication.

use crate::chat_history::{
    ChatHistoryError, ChatHistoryStore, Conversation, ConversationFilters, CostInfo, MCPInfo,
    Message, MessageFilters, MessageRole, PrivacyLevel, Result, RoutingInfo, StorageStats,
    TokenInfo,
};
use libsql::{Builder, Connection, Database};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Turso/libsql storage backend
pub struct TursoChatHistoryStore {
    #[allow(dead_code)]
    db: Database,
    // Shared connection for thread-safe access
    conn: Arc<Mutex<Connection>>,
}

impl TursoChatHistoryStore {
    /// Create a new Turso store with the given database URL and auth token
    /// URL format: "libsql://[your-database].turso.io"
    /// For local: "file:path/to/database.db"
    pub async fn new(database_url: &str, auth_token: Option<String>) -> Result<Self> {
        // If it's a local file, use local builder
        let db = if database_url.starts_with("file:") {
            Builder::new_local(database_url.trim_start_matches("file:"))
                .build()
                .await
                .map_err(|e| {
                    ChatHistoryError::Storage(format!("Turso local connection error: {}", e))
                })?
        } else {
            // Remote Turso database
            Builder::new_remote(database_url.to_string(), auth_token.unwrap_or_default())
                .build()
                .await
                .map_err(|e| ChatHistoryError::Storage(format!("Turso connection error: {}", e)))?
        };

        // Create and store a shared connection
        let conn = db
            .connect()
            .map_err(|e| ChatHistoryError::Storage(format!("Connection error: {}", e)))?;

        Ok(Self {
            db,
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    async fn init_schema(&self) -> Result<()> {
        let schema = include_str!("chat_history_schema.sql");

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

        let conn = self.conn.lock().await;
        for statement in statements {
            conn.execute(&statement, ()).await.map_err(|e| {
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
impl ChatHistoryStore for TursoChatHistoryStore {
    async fn init(&self) -> Result<()> {
        self.init_schema().await
    }

    async fn record_conversation(&self, conversation: &Conversation) -> Result<()> {
        let metadata_json = Self::serialize_json(&conversation.metadata)?;
        let conn = self.conn.lock().await;

        conn.execute(
            r#"
            INSERT INTO conversations (conversation_id, created_at, last_seen_at, title, metadata)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(conversation_id) DO UPDATE SET
                last_seen_at = excluded.last_seen_at,
                title = excluded.title,
                metadata = excluded.metadata
            "#,
            libsql::params![
                conversation.conversation_id.clone(),
                conversation.created_at as i64,
                conversation.last_seen_at as i64,
                conversation.title.clone(),
                metadata_json,
            ],
        )
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
            .map(Self::serialize_json)
            .transpose()?;
        let transformations_json = message
            .routing
            .transformations_applied
            .as_ref()
            .map(Self::serialize_json)
            .transpose()?;
        let mcp_servers_json = Self::serialize_json(&message.mcp.mcp_servers)?;
        let cost_info_json = message
            .cost_info
            .as_ref()
            .map(Self::serialize_json)
            .transpose()?;
        let privacy_level_json = Self::serialize_json(&message.privacy_level)?;

        let conn = self.conn.lock().await;

        conn.execute(
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
            libsql::params![
                message.message_id.clone(),
                message.conversation_id.clone(),
                message.request_id.clone(),
                role_json,
                content_json,
                tool_calls_json,
                message.created_at as i64,
                message.routing.requested_model.clone(),
                message.routing.actual_model.clone(),
                message.routing.backend.clone(),
                message.routing.backend_url.clone(),
                message.routing.upstream_mode.clone(),
                message.routing.route_id.clone(),
                transformations_json,
                if message.mcp.mcp_enabled { 1i64 } else { 0i64 },
                mcp_servers_json,
                if message.mcp.system_prompt_applied {
                    1i64
                } else {
                    0i64
                },
                message.tokens.input_tokens.map(|t| t as i64),
                message.tokens.output_tokens.map(|t| t as i64),
                message.tokens.cached_tokens.map(|t| t as i64),
                message.tokens.reasoning_tokens.map(|t| t as i64),
                cost_info_json,
                message.content_hash.clone(),
                privacy_level_json,
            ],
        )
        .await
        .map_err(|e| ChatHistoryError::Storage(format!("Insert message error: {}", e)))?;

        Ok(())
    }

    async fn record_messages(&self, messages: &[Message]) -> Result<()> {
        let conn = self.conn.lock().await;

        // Use transaction for batch inserts
        conn.execute("BEGIN TRANSACTION", ())
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Transaction begin error: {}", e)))?;

        for message in messages {
            let role_json = Self::serialize_json(&message.role)?;
            let content_json = Self::serialize_json(&message.content)?;
            let tool_calls_json = message
                .tool_calls
                .as_ref()
                .map(Self::serialize_json)
                .transpose()?;
            let transformations_json = message
                .routing
                .transformations_applied
                .as_ref()
                .map(Self::serialize_json)
                .transpose()?;
            let mcp_servers_json = Self::serialize_json(&message.mcp.mcp_servers)?;
            let cost_info_json = message
                .cost_info
                .as_ref()
                .map(Self::serialize_json)
                .transpose()?;
            let privacy_level_json = Self::serialize_json(&message.privacy_level)?;

            let result = conn
                .execute(
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
                    libsql::params![
                        message.message_id.clone(),
                        message.conversation_id.clone(),
                        message.request_id.clone(),
                        role_json,
                        content_json,
                        tool_calls_json,
                        message.created_at as i64,
                        message.routing.requested_model.clone(),
                        message.routing.actual_model.clone(),
                        message.routing.backend.clone(),
                        message.routing.backend_url.clone(),
                        message.routing.upstream_mode.clone(),
                        message.routing.route_id.clone(),
                        transformations_json,
                        if message.mcp.mcp_enabled { 1i64 } else { 0i64 },
                        mcp_servers_json,
                        if message.mcp.system_prompt_applied {
                            1i64
                        } else {
                            0i64
                        },
                        message.tokens.input_tokens.map(|t| t as i64),
                        message.tokens.output_tokens.map(|t| t as i64),
                        message.tokens.cached_tokens.map(|t| t as i64),
                        message.tokens.reasoning_tokens.map(|t| t as i64),
                        cost_info_json,
                        message.content_hash.clone(),
                        privacy_level_json,
                    ],
                )
                .await;

            if result.is_err() {
                conn.execute("ROLLBACK", ()).await.ok();
                return Err(ChatHistoryError::Storage(format!(
                    "Insert message error: {:?}",
                    result.err()
                )));
            }
        }

        conn.execute("COMMIT", ())
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Transaction commit error: {}", e)))?;

        Ok(())
    }

    async fn get_conversation(&self, conversation_id: &str) -> Result<Conversation> {
        let conn = self.conn.lock().await;

        let mut rows = conn.query(
            "SELECT conversation_id, created_at, last_seen_at, title, metadata FROM conversations WHERE conversation_id = ?1",
            libsql::params![conversation_id],
        )
        .await
        .map_err(|e| ChatHistoryError::Storage(format!("Query error: {}", e)))?;

        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Row fetch error: {}", e)))?
        {
            let metadata_str: String = row
                .get(4)
                .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?;
            let metadata = Self::deserialize_json(&metadata_str)?;

            Ok(Conversation {
                conversation_id: row
                    .get(0)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                created_at: row
                    .get::<i64>(1)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?
                    as u64,
                last_seen_at: row
                    .get::<i64>(2)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?
                    as u64,
                title: row
                    .get(3)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                metadata,
            })
        } else {
            Err(ChatHistoryError::NotFound(format!(
                "Conversation {} not found",
                conversation_id
            )))
        }
    }

    async fn list_conversations(&self, filters: &ConversationFilters) -> Result<Vec<Conversation>> {
        let mut query = String::from("SELECT conversation_id, created_at, last_seen_at, title, metadata FROM conversations WHERE 1=1");
        let mut params = Vec::new();

        if let Some(start) = filters.start_time {
            query.push_str(" AND created_at >= ?");
            params.push(libsql::Value::Integer(start as i64));
        }
        if let Some(end) = filters.end_time {
            query.push_str(" AND created_at <= ?");
            params.push(libsql::Value::Integer(end as i64));
        }

        query.push_str(" ORDER BY last_seen_at DESC");

        if let Some(limit) = filters.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        let conn = self.conn.lock().await;
        let mut rows = conn
            .query(&query, libsql::params_from_iter(params))
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Query error: {}", e)))?;

        let mut conversations = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Row fetch error: {}", e)))?
        {
            let metadata_str: String = row
                .get(4)
                .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?;
            let metadata = Self::deserialize_json(&metadata_str)?;

            conversations.push(Conversation {
                conversation_id: row
                    .get(0)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                created_at: row
                    .get::<i64>(1)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?
                    as u64,
                last_seen_at: row
                    .get::<i64>(2)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?
                    as u64,
                title: row
                    .get(3)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                metadata,
            });
        }

        Ok(conversations)
    }

    async fn list_messages(&self, filters: &MessageFilters) -> Result<Vec<Message>> {
        let mut query = String::from("SELECT * FROM messages WHERE 1=1");
        let mut params = Vec::new();

        if let Some(ref conv_id) = filters.conversation_id {
            query.push_str(" AND conversation_id = ?");
            params.push(libsql::Value::Text(conv_id.clone()));
        }
        if let Some(ref req_id) = filters.request_id {
            query.push_str(" AND request_id = ?");
            params.push(libsql::Value::Text(req_id.clone()));
        }
        if let Some(start) = filters.start_time {
            query.push_str(" AND created_at >= ?");
            params.push(libsql::Value::Integer(start as i64));
        }
        if let Some(end) = filters.end_time {
            query.push_str(" AND created_at <= ?");
            params.push(libsql::Value::Integer(end as i64));
        }
        if let Some(ref backend) = filters.backend {
            query.push_str(" AND backend = ?");
            params.push(libsql::Value::Text(backend.clone()));
        }
        if let Some(ref model) = filters.requested_model {
            query.push_str(" AND requested_model = ?");
            params.push(libsql::Value::Text(model.clone()));
        }
        if let Some(ref model) = filters.actual_model {
            query.push_str(" AND actual_model = ?");
            params.push(libsql::Value::Text(model.clone()));
        }
        if let Some(ref route_id) = filters.route_id {
            query.push_str(" AND route_id = ?");
            params.push(libsql::Value::Text(route_id.clone()));
        }
        if let Some(mcp) = filters.mcp_enabled {
            query.push_str(" AND mcp_enabled = ?");
            params.push(libsql::Value::Integer(if mcp { 1 } else { 0 }));
        }
        if let Some(ref mode) = filters.upstream_mode {
            query.push_str(" AND upstream_mode = ?");
            params.push(libsql::Value::Text(mode.clone()));
        }

        query.push_str(" ORDER BY created_at ASC");

        if let Some(limit) = filters.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        let conn = self.conn.lock().await;
        let mut rows = conn
            .query(&query, libsql::params_from_iter(params))
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Query error: {}", e)))?;

        let mut messages = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Row fetch error: {}", e)))?
        {
            let role_str: String = row
                .get(3)
                .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?;
            let role: MessageRole = Self::deserialize_json(&role_str)?;

            let content_str: String = row
                .get(4)
                .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?;
            let content: serde_json::Value = Self::deserialize_json(&content_str)?;

            let tool_calls: Option<serde_json::Value> = row
                .get::<Option<String>>(5)
                .ok()
                .flatten()
                .map(|s| Self::deserialize_json(&s))
                .transpose()?;

            let transformations_applied: Option<serde_json::Value> = row
                .get::<Option<String>>(13)
                .ok()
                .flatten()
                .map(|s| Self::deserialize_json(&s))
                .transpose()?;

            let mcp_servers_str: String = row
                .get(15)
                .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?;
            let mcp_servers: Vec<String> = Self::deserialize_json(&mcp_servers_str)?;

            let cost_info: Option<CostInfo> = row
                .get::<Option<String>>(21)
                .ok()
                .flatten()
                .map(|s| Self::deserialize_json(&s))
                .transpose()?;

            let privacy_level_str: String = row
                .get(23)
                .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?;
            let privacy_level: PrivacyLevel = Self::deserialize_json(&privacy_level_str)?;

            messages.push(Message {
                message_id: row
                    .get(0)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                conversation_id: row
                    .get(1)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                request_id: row
                    .get(2)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                role,
                content,
                tool_calls,
                created_at: row
                    .get::<i64>(6)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?
                    as u64,
                routing: RoutingInfo {
                    requested_model: row
                        .get(7)
                        .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                    actual_model: row
                        .get(8)
                        .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                    backend: row
                        .get(9)
                        .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                    backend_url: row
                        .get(10)
                        .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                    upstream_mode: row
                        .get(11)
                        .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                    route_id: row
                        .get(12)
                        .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                    transformations_applied,
                },
                mcp: MCPInfo {
                    mcp_enabled: row
                        .get::<i64>(14)
                        .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?
                        != 0,
                    mcp_servers,
                    system_prompt_applied: row
                        .get::<i64>(16)
                        .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?
                        != 0,
                },
                tokens: TokenInfo {
                    input_tokens: row.get::<Option<i64>>(17).ok().flatten().map(|t| t as u64),
                    output_tokens: row.get::<Option<i64>>(18).ok().flatten().map(|t| t as u64),
                    cached_tokens: row.get::<Option<i64>>(19).ok().flatten().map(|t| t as u64),
                    reasoning_tokens: row.get::<Option<i64>>(20).ok().flatten().map(|t| t as u64),
                },
                cost_info,
                content_hash: row
                    .get(22)
                    .map_err(|e| ChatHistoryError::Storage(format!("Column error: {}", e)))?,
                privacy_level,
            });
        }

        Ok(messages)
    }

    async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        let conn = self.conn.lock().await;

        conn.execute(
            "DELETE FROM conversations WHERE conversation_id = ?1",
            libsql::params![conversation_id],
        )
        .await
        .map_err(|e| ChatHistoryError::Storage(format!("Delete error: {}", e)))?;

        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        let conn = self.conn.lock().await;

        conn.execute("DELETE FROM messages", ())
            .await
            .map_err(|e| ChatHistoryError::Storage(e.to_string()))?;
        conn.execute("DELETE FROM conversations", ())
            .await
            .map_err(|e| ChatHistoryError::Storage(e.to_string()))?;

        Ok(())
    }

    async fn stats(&self) -> Result<StorageStats> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query("SELECT COUNT(*) FROM conversations", ())
            .await
            .map_err(|e| ChatHistoryError::Storage(e.to_string()))?;
        let conv_count = if let Some(row) = rows
            .next()
            .await
            .map_err(|e| ChatHistoryError::Storage(e.to_string()))?
        {
            row.get::<i64>(0).unwrap_or(0)
        } else {
            0
        };

        let mut rows = conn
            .query("SELECT COUNT(*) FROM messages", ())
            .await
            .map_err(|e| ChatHistoryError::Storage(e.to_string()))?;
        let msg_count = if let Some(row) = rows
            .next()
            .await
            .map_err(|e| ChatHistoryError::Storage(e.to_string()))?
        {
            row.get::<i64>(0).unwrap_or(0)
        } else {
            0
        };

        Ok(StorageStats {
            total_conversations: conv_count as usize,
            total_messages: msg_count as usize,
            backend_type: "turso".to_string(),
            storage_path: None,
        })
    }

    async fn health(&self) -> Result<bool> {
        let conn = self.conn.lock().await;

        // Try a simple query to check if the database is accessible
        conn.query("SELECT 1", ())
            .await
            .map_err(|e| ChatHistoryError::Storage(format!("Health check error: {}", e)))?;

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_history::{MessageRole, PrivacyLevel};
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_turso_store_conversation() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = format!("file:{}", temp_file.path().to_string_lossy());
        let store = TursoChatHistoryStore::new(&path, None).await.unwrap();
        store.init().await.unwrap();

        let conv = Conversation::new("conv_123".to_string());
        store.record_conversation(&conv).await.unwrap();

        let retrieved = store.get_conversation("conv_123").await.unwrap();
        assert_eq!(retrieved.conversation_id, "conv_123");
    }

    #[tokio::test]
    async fn test_turso_store_message() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = format!("file:{}", temp_file.path().to_string_lossy());
        let store = TursoChatHistoryStore::new(&path, None).await.unwrap();
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
    async fn test_turso_store_batch_messages() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = format!("file:{}", temp_file.path().to_string_lossy());
        let store = TursoChatHistoryStore::new(&path, None).await.unwrap();
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
    async fn test_turso_store_filters() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = format!("file:{}", temp_file.path().to_string_lossy());
        let store = TursoChatHistoryStore::new(&path, None).await.unwrap();
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
    async fn test_turso_store_delete() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = format!("file:{}", temp_file.path().to_string_lossy());
        let store = TursoChatHistoryStore::new(&path, None).await.unwrap();
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
    async fn test_turso_store_clear() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = format!("file:{}", temp_file.path().to_string_lossy());
        let store = TursoChatHistoryStore::new(&path, None).await.unwrap();
        store.init().await.unwrap();

        let conv = Conversation::new("conv_123".to_string());
        store.record_conversation(&conv).await.unwrap();

        store.clear().await.unwrap();

        let stats = store.stats().await.unwrap();
        assert_eq!(stats.total_conversations, 0);
        assert_eq!(stats.total_messages, 0);
    }

    #[tokio::test]
    async fn test_turso_store_health() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = format!("file:{}", temp_file.path().to_string_lossy());
        let store = TursoChatHistoryStore::new(&path, None).await.unwrap();
        store.init().await.unwrap();

        let healthy = store.health().await.unwrap();
        assert!(healthy);
    }
}
