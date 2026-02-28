use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

/// Chat Completions role enumeration.
///
/// Uses lowercase serialization to match the OpenAI Chat API:
/// "system" | "user" | "assistant" | "tool" | "function"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
    /// Legacy alias present in some Chat Completions payloads.
    /// When converting to Responses, this typically maps to "tool".
    Function,
}

/// Minimal Chat message model compatible with the Chat Completions API.
///
/// Notes:
/// - `content` may be a string or an array of message parts; we accept `serde_json::Value`
///   to allow both shapes (and future-proof for multimodal content).
/// - `name` and `tool_call_id` are optional fields that may appear on assistant or tool messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    /// Chat API allows a string or an array of content parts (for multimodal).
    pub content: serde_json::Value,
    /// Optional name for function/tool messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional tool call identifier (tool result correlation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Optional list of tool calls (assistant tool invocation metadata).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// JSON Schema for a function tool definition in Chat Completions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct FunctionDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// JSON Schema object describing the function parameters.
    pub parameters: serde_json::Value,
}

/// Chat Completions tool definition (subset).
///
/// Example:
/// {
///   "type": "function",
///   "function": { "name": "...", "description": "...", "parameters": { ... } }
/// }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolDefinition {
    Function { function: FunctionDef },
}

/// Response format hint for structured outputs in Chat Completions.
///
/// Example: { "type": "json_object", "schema": { ... } }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ResponseFormat {
    /// e.g., "json_object"
    #[serde(rename = "type")]
    pub kind: String,
    /// Additional fields such as "schema" may be present.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Chat Completions request (commonly used subset).
///
/// This model intentionally uses flexible types (e.g., `serde_json::Value` for `stop`)
/// to accept both strings and arrays where the API allows it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,

    // Sampling / decoding
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Newer parameter name for max tokens (used by GPT-5 and reasoning models)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    /// Accepts a single string or an array of strings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logit_bias: Option<HashMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,

    // Tools
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,

    // Formatting
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,

    // Streaming
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,

    /// Provider-specific extra options (e.g., Gemini thinking_config)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<serde_json::Value>,
}

// ============================================================================
// Chat Completions Response Models
// ============================================================================

/// Tool call in a Chat Completions response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String, // "function"
    pub function: FunctionCall,
    /// Provider-specific extra content (e.g., Gemini thought_signature)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_content: Option<serde_json::Value>,
}

impl Default for ToolCall {
    fn default() -> Self {
        Self {
            id: String::new(),
            call_type: "function".to_string(),
            function: FunctionCall::default(),
            extra_content: None,
        }
    }
}

/// Function call details
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[skip_serializing_none]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String, // JSON string
}

/// Message in a Chat Completions response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ChatResponseMessage {
    pub role: String, // "assistant"
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    pub function_call: Option<FunctionCall>, // Legacy
    /// Reasoning content extracted from <thought> tags (Gemini and other reasoning models)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

/// Choice in a Chat Completions response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ChatChoice {
    pub index: u32,
    pub message: ChatResponseMessage,
    pub finish_reason: Option<String>, // "stop", "length", "tool_calls", "content_filter"
    #[serde(default)]
    pub logprobs: Option<serde_json::Value>,
}

/// Usage statistics in Chat Completions response
///
/// Extended to support reasoning_tokens as a custom field for reasoning models.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ChatUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,

    /// Reasoning tokens (custom extension for o1, o3, GPT-5 models)
    /// Not part of the standard Chat API but critical for reasoning model support
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,

    /// Cached tokens (subset of prompt_tokens)
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

/// Complete Chat Completions API response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String, // "chat.completion"
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
    pub usage: Option<ChatUsage>,
    #[serde(default)]
    pub system_fingerprint: Option<String>,
}

// ============================================================================
// Chat Completions Streaming Response Models
// ============================================================================

/// Delta in a streaming chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ChatDelta {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallDelta>>,
    /// Reasoning content delta for streaming (Gemini and other reasoning models)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

/// Tool call delta in streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ToolCallDelta {
    pub index: u32,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub call_type: Option<String>,
    #[serde(default)]
    pub function: Option<FunctionCallDelta>,
    /// Provider-specific extra content (e.g., Gemini thought_signature)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_content: Option<serde_json::Value>,
}

/// Function call delta
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct FunctionCallDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

/// Choice in a streaming chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ChatStreamChoice {
    pub index: u32,
    pub delta: ChatDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

/// Streaming chunk response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String, // "chat.completion.chunk"
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatStreamChoice>,
    #[serde(default)]
    pub usage: Option<ChatUsage>, // Only in final chunk
}

/// OpenAI-compatible model information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    /// Model ID
    pub id: String,
    /// Object type (always "model")
    pub object: String,
    /// Creation timestamp
    pub created: u64,
    /// Model owner (e.g., "openai")
    pub owned_by: String,
}

/// OpenAI-compatible models list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsResponse {
    /// Object type (always "list")
    pub object: String,
    /// List of models
    pub data: Vec<Model>,
}
