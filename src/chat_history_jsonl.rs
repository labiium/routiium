//! JSONL chat history storage backend
//!
//! Append-only JSON Lines format for simple persistence and export.
//! Good as a sink backend for audit trails.

use crate::chat_history::{
    ChatHistoryError, ChatHistoryStore, Conversation, ConversationFilters, Message, MessageFilters,
    Result, StorageStats,
};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// JSONL storage backend
#[derive(Clone)]
pub struct JsonlChatHistoryStore {
    path: PathBuf,
    file: Arc<Mutex<Option<File>>>,
}

impl JsonlChatHistoryStore {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            file: Arc::new(Mutex::new(None)),
        }
    }

    fn ensure_file(&self) -> Result<()> {
        let mut file_guard = self
            .file
            .lock()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        if file_guard.is_none() {
            // Create parent directories
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;

            *file_guard = Some(file);
        }

        Ok(())
    }

    fn write_line(&self, line: &str) -> Result<()> {
        self.ensure_file()?;

        let mut file_guard = self
            .file
            .lock()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        if let Some(ref mut file) = *file_guard {
            writeln!(file, "{}", line)?;
            file.flush()?;
        }

        Ok(())
    }

    fn read_all_lines(&self) -> Result<Vec<String>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);

        let lines: std::result::Result<Vec<String>, _> = reader.lines().collect();
        lines.map_err(|e| e.into())
    }

    fn parse_conversations(&self) -> Result<Vec<Conversation>> {
        let lines = self.read_all_lines()?;
        let mut conversations = Vec::new();

        for line in lines {
            if line.trim().is_empty() {
                continue;
            }

            // Try to parse as conversation
            if let Ok(record) = serde_json::from_str::<serde_json::Value>(&line) {
                if record.get("type").and_then(|t| t.as_str()) == Some("conversation") {
                    if let Ok(conv) = serde_json::from_value::<Conversation>(
                        record.get("data").unwrap_or(&serde_json::json!({})).clone(),
                    ) {
                        conversations.push(conv);
                    }
                }
            }
        }

        Ok(conversations)
    }

    fn parse_messages(&self) -> Result<Vec<Message>> {
        let lines = self.read_all_lines()?;
        let mut messages = Vec::new();

        for line in lines {
            if line.trim().is_empty() {
                continue;
            }

            // Try to parse as message
            if let Ok(record) = serde_json::from_str::<serde_json::Value>(&line) {
                if record.get("type").and_then(|t| t.as_str()) == Some("message") {
                    if let Ok(msg) = serde_json::from_value::<Message>(
                        record.get("data").unwrap_or(&serde_json::json!({})).clone(),
                    ) {
                        messages.push(msg);
                    }
                }
            }
        }

        Ok(messages)
    }
}

#[async_trait::async_trait]
impl ChatHistoryStore for JsonlChatHistoryStore {
    async fn init(&self) -> Result<()> {
        self.ensure_file()?;
        Ok(())
    }

    async fn record_conversation(&self, conversation: &Conversation) -> Result<()> {
        let record = serde_json::json!({
            "type": "conversation",
            "data": conversation,
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        });

        let line = serde_json::to_string(&record)?;
        self.write_line(&line)?;
        Ok(())
    }

    async fn record_message(&self, message: &Message) -> Result<()> {
        let record = serde_json::json!({
            "type": "message",
            "data": message,
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        });

        let line = serde_json::to_string(&record)?;
        self.write_line(&line)?;
        Ok(())
    }

    async fn record_messages(&self, messages: &[Message]) -> Result<()> {
        for message in messages {
            self.record_message(message).await?;
        }
        Ok(())
    }

    async fn get_conversation(&self, conversation_id: &str) -> Result<Conversation> {
        let conversations = self.parse_conversations()?;

        conversations
            .into_iter()
            .find(|c| c.conversation_id == conversation_id)
            .ok_or_else(|| {
                ChatHistoryError::NotFound(format!("Conversation {} not found", conversation_id))
            })
    }

    async fn list_conversations(&self, filters: &ConversationFilters) -> Result<Vec<Conversation>> {
        let conversations = self.parse_conversations()?;

        let mut results: Vec<Conversation> = conversations
            .into_iter()
            .filter(|conv| {
                if let Some(start) = filters.start_time {
                    if conv.created_at < start {
                        return false;
                    }
                }
                if let Some(end) = filters.end_time {
                    if conv.created_at > end {
                        return false;
                    }
                }
                true
            })
            .collect();

        // Sort by last_seen_at descending
        results.sort_by(|a, b| b.last_seen_at.cmp(&a.last_seen_at));

        if let Some(limit) = filters.limit {
            results.truncate(limit);
        }

        Ok(results)
    }

    async fn list_messages(&self, filters: &MessageFilters) -> Result<Vec<Message>> {
        let messages = self.parse_messages()?;

        let mut results: Vec<Message> = messages
            .into_iter()
            .filter(|msg| {
                if let Some(ref conv_id) = filters.conversation_id {
                    if &msg.conversation_id != conv_id {
                        return false;
                    }
                }
                if let Some(ref req_id) = filters.request_id {
                    if msg.request_id.as_ref() != Some(req_id) {
                        return false;
                    }
                }
                if let Some(start) = filters.start_time {
                    if msg.created_at < start {
                        return false;
                    }
                }
                if let Some(end) = filters.end_time {
                    if msg.created_at > end {
                        return false;
                    }
                }
                if let Some(ref backend) = filters.backend {
                    if msg.routing.backend.as_ref() != Some(backend) {
                        return false;
                    }
                }
                if let Some(ref model) = filters.requested_model {
                    if msg.routing.requested_model.as_ref() != Some(model) {
                        return false;
                    }
                }
                if let Some(ref model) = filters.actual_model {
                    if msg.routing.actual_model.as_ref() != Some(model) {
                        return false;
                    }
                }
                if let Some(ref route_id) = filters.route_id {
                    if msg.routing.route_id.as_ref() != Some(route_id) {
                        return false;
                    }
                }
                if let Some(mcp) = filters.mcp_enabled {
                    if msg.mcp.mcp_enabled != mcp {
                        return false;
                    }
                }
                if let Some(ref mode) = filters.upstream_mode {
                    if msg.routing.upstream_mode.as_ref() != Some(mode) {
                        return false;
                    }
                }
                true
            })
            .collect();

        // Sort by created_at ascending
        results.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        if let Some(limit) = filters.limit {
            results.truncate(limit);
        }

        Ok(results)
    }

    async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        // JSONL is append-only, so we can't delete efficiently
        // This would require rewriting the entire file
        // For now, just log a warning
        tracing::warn!(
            "JSONL backend does not support deletion: {}",
            conversation_id
        );
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        // Clear by truncating the file
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }

        // Reset the file handle
        let mut file_guard = self
            .file
            .lock()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;
        *file_guard = None;

        Ok(())
    }

    async fn stats(&self) -> Result<StorageStats> {
        let conversations = self.parse_conversations()?;
        let messages = self.parse_messages()?;

        Ok(StorageStats {
            total_conversations: conversations.len(),
            total_messages: messages.len(),
            backend_type: "jsonl".to_string(),
            storage_path: Some(self.path.to_string_lossy().to_string()),
        })
    }

    async fn health(&self) -> Result<bool> {
        // Check if we can write to the file
        self.ensure_file()?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_history::{MessageRole, PrivacyLevel, RoutingInfo};
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_jsonl_store_conversation() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = JsonlChatHistoryStore::new(temp_file.path());
        store.init().await.unwrap();

        let conv = Conversation::new("conv_123".to_string());
        store.record_conversation(&conv).await.unwrap();

        let retrieved = store.get_conversation("conv_123").await.unwrap();
        assert_eq!(retrieved.conversation_id, "conv_123");
    }

    #[tokio::test]
    async fn test_jsonl_store_message() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = JsonlChatHistoryStore::new(temp_file.path());
        store.init().await.unwrap();

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
    async fn test_jsonl_store_filters() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = JsonlChatHistoryStore::new(temp_file.path());
        store.init().await.unwrap();

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
    async fn test_jsonl_store_clear() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();
        let store = JsonlChatHistoryStore::new(&path);
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

        store.clear().await.unwrap();

        let stats = store.stats().await.unwrap();
        assert_eq!(stats.total_conversations, 0);
        assert_eq!(stats.total_messages, 0);
    }

    #[tokio::test]
    async fn test_jsonl_store_persistence() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();

        // Write data
        {
            let store = JsonlChatHistoryStore::new(&path);
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
        }

        // Read data from new instance
        {
            let store = JsonlChatHistoryStore::new(&path);
            let stats = store.stats().await.unwrap();
            assert_eq!(stats.total_conversations, 1);
            assert_eq!(stats.total_messages, 1);

            let conv = store.get_conversation("conv_123").await.unwrap();
            assert_eq!(conv.conversation_id, "conv_123");
        }
    }

    #[tokio::test]
    async fn test_jsonl_store_batch_messages() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = JsonlChatHistoryStore::new(temp_file.path());
        store.init().await.unwrap();

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
}
