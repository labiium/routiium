//! In-memory chat history storage backend
//!
//! Fast, simple storage for development and testing.
//! Data is lost when the process exits.

use crate::chat_history::{
    ChatHistoryError, ChatHistoryStore, Conversation, ConversationFilters, Message, MessageFilters,
    Result, StorageStats,
};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// In-memory storage backend
#[derive(Clone)]
pub struct MemoryChatHistoryStore {
    conversations: Arc<RwLock<HashMap<String, Conversation>>>,
    messages: Arc<RwLock<Vec<Message>>>,
    max_messages: Option<usize>,
}

impl MemoryChatHistoryStore {
    pub fn new() -> Self {
        Self {
            conversations: Arc::new(RwLock::new(HashMap::new())),
            messages: Arc::new(RwLock::new(Vec::new())),
            max_messages: None,
        }
    }

    pub fn with_max_messages(max_messages: usize) -> Self {
        Self {
            conversations: Arc::new(RwLock::new(HashMap::new())),
            messages: Arc::new(RwLock::new(Vec::new())),
            max_messages: Some(max_messages),
        }
    }

    fn prune_messages_if_needed(&self) {
        if let Some(max) = self.max_messages {
            let mut messages = self.messages.write().unwrap();
            if messages.len() > max {
                let to_remove = messages.len() - max;
                messages.drain(0..to_remove);
            }
        }
    }
}

impl Default for MemoryChatHistoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ChatHistoryStore for MemoryChatHistoryStore {
    async fn init(&self) -> Result<()> {
        // Nothing to initialize for memory storage
        Ok(())
    }

    async fn record_conversation(&self, conversation: &Conversation) -> Result<()> {
        let mut conversations = self
            .conversations
            .write()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        conversations.insert(conversation.conversation_id.clone(), conversation.clone());
        Ok(())
    }

    async fn record_message(&self, message: &Message) -> Result<()> {
        let mut messages = self
            .messages
            .write()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        messages.push(message.clone());
        drop(messages);

        self.prune_messages_if_needed();
        Ok(())
    }

    async fn record_messages(&self, messages: &[Message]) -> Result<()> {
        let mut msgs = self
            .messages
            .write()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        msgs.extend_from_slice(messages);
        drop(msgs);

        self.prune_messages_if_needed();
        Ok(())
    }

    async fn get_conversation(&self, conversation_id: &str) -> Result<Conversation> {
        let conversations = self
            .conversations
            .read()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        conversations.get(conversation_id).cloned().ok_or_else(|| {
            ChatHistoryError::NotFound(format!("Conversation {} not found", conversation_id))
        })
    }

    async fn list_conversations(&self, filters: &ConversationFilters) -> Result<Vec<Conversation>> {
        let conversations = self
            .conversations
            .read()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        let mut results: Vec<Conversation> = conversations
            .values()
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
            .cloned()
            .collect();

        // Sort by last_seen_at descending
        results.sort_by(|a, b| b.last_seen_at.cmp(&a.last_seen_at));

        if let Some(limit) = filters.limit {
            results.truncate(limit);
        }

        Ok(results)
    }

    async fn list_messages(&self, filters: &MessageFilters) -> Result<Vec<Message>> {
        let messages = self
            .messages
            .read()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        let mut results: Vec<Message> = messages
            .iter()
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
            .cloned()
            .collect();

        // Sort by created_at ascending (chronological order)
        results.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        if let Some(limit) = filters.limit {
            results.truncate(limit);
        }

        Ok(results)
    }

    async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        let mut conversations = self
            .conversations
            .write()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        conversations.remove(conversation_id);

        let mut messages = self
            .messages
            .write()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        messages.retain(|msg| msg.conversation_id != conversation_id);

        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        let mut conversations = self
            .conversations
            .write()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        let mut messages = self
            .messages
            .write()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        conversations.clear();
        messages.clear();

        Ok(())
    }

    async fn stats(&self) -> Result<StorageStats> {
        let conversations = self
            .conversations
            .read()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        let messages = self
            .messages
            .read()
            .map_err(|e| ChatHistoryError::Storage(format!("Lock error: {}", e)))?;

        Ok(StorageStats {
            total_conversations: conversations.len(),
            total_messages: messages.len(),
            backend_type: "memory".to_string(),
            storage_path: None,
        })
    }

    async fn health(&self) -> Result<bool> {
        // Always healthy for in-memory storage
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_history::{MessageRole, PrivacyLevel, RoutingInfo};

    #[tokio::test]
    async fn test_memory_store_conversation() {
        let store = MemoryChatHistoryStore::new();
        store.init().await.unwrap();

        let conv = Conversation::new("conv_123".to_string());
        store.record_conversation(&conv).await.unwrap();

        let retrieved = store.get_conversation("conv_123").await.unwrap();
        assert_eq!(retrieved.conversation_id, "conv_123");
    }

    #[tokio::test]
    async fn test_memory_store_message() {
        let store = MemoryChatHistoryStore::new();
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
    async fn test_memory_store_filters() {
        let store = MemoryChatHistoryStore::new();
        store.init().await.unwrap();

        // Add messages with different backends
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

        // Filter by actual model
        let filters = MessageFilters {
            actual_model: Some("claude-3-opus".to_string()),
            ..Default::default()
        };
        let messages = store.list_messages(&filters).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].routing.actual_model,
            Some("claude-3-opus".to_string())
        );
    }

    #[tokio::test]
    async fn test_memory_store_delete() {
        let store = MemoryChatHistoryStore::new();
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

        let filters = MessageFilters {
            conversation_id: Some("conv_123".to_string()),
            ..Default::default()
        };
        let messages = store.list_messages(&filters).await.unwrap();
        assert_eq!(messages.len(), 0);
    }

    #[tokio::test]
    async fn test_memory_store_clear() {
        let store = MemoryChatHistoryStore::new();
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
    async fn test_memory_store_max_messages() {
        let store = MemoryChatHistoryStore::with_max_messages(3);
        store.init().await.unwrap();

        for i in 0..5 {
            let msg = Message::new(
                "conv_123".to_string(),
                MessageRole::User,
                serde_json::json!({"text": format!("Message {}", i)}),
                PrivacyLevel::Full,
            );
            store.record_message(&msg).await.unwrap();
        }

        let filters = MessageFilters::default();
        let messages = store.list_messages(&filters).await.unwrap();
        assert_eq!(messages.len(), 3);
    }

    #[tokio::test]
    async fn test_memory_store_stats() {
        let store = MemoryChatHistoryStore::new();
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

        let stats = store.stats().await.unwrap();
        assert_eq!(stats.backend_type, "memory");
        assert_eq!(stats.total_conversations, 1);
        assert_eq!(stats.total_messages, 1);
        assert!(stats.storage_path.is_none());
    }

    #[tokio::test]
    async fn test_memory_store_time_filters() {
        let store = MemoryChatHistoryStore::new();
        store.init().await.unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let msg1 = Message::new(
            "conv_123".to_string(),
            MessageRole::User,
            serde_json::json!({"text": "Hello"}),
            PrivacyLevel::Full,
        );
        store.record_message(&msg1).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let msg2 = Message::new(
            "conv_123".to_string(),
            MessageRole::Assistant,
            serde_json::json!({"text": "Hi"}),
            PrivacyLevel::Full,
        );
        store.record_message(&msg2).await.unwrap();

        // Filter by start time
        let filters = MessageFilters {
            start_time: Some(now + 1),
            ..Default::default()
        };
        let messages = store.list_messages(&filters).await.unwrap();
        assert!(messages.len() <= 1); // Only second message might be after start time
    }
}
