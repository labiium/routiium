use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

/// Responses API message in chat-form input.
/// This mirrors the minimal fields needed to forward Chat Completions style
/// messages to the Responses API without lossy transformation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesMessage {
    /// "system" | "user" | "assistant" | "tool"
    pub role: String,
    /// Either a string or an array of content parts for multimodal inputs.
    pub content: serde_json::Value,
    /// Optional name for function/tool messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional correlation id when returning tool outputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Optional assistant tool call metadata when returning original tool decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
}

/// Tool definition for Responses API.
///
/// The Responses API uses a flat structure where name, description, and parameters
/// are at the same level as type, not nested under a "function" key.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesToolDefinition {
    Function {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        parameters: serde_json::Value,
    },
    // Extend here with built-in tools (e.g., web_search, file_search, mcp) if needed.
}

/// Minimal yet robust Responses API request model using top-level `messages`.
///
/// We keep chat-form messages under top-level `messages` for fidelity when translating
/// from Chat Completions payloads. Null/None fields are omitted during serialization.
///
/// Notes:
/// - `max_output_tokens` is the Responses equivalent of Chat's `max_tokens`.
/// - `response_format` is forwarded as an arbitrary JSON object to support
///   structured output hints (e.g., `{ "type": "json_object", "schema": {...} }`).
/// - `conversation` enables stateful interactions if the server supports it.
#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesRequest {
    pub model: String,

    /// Internal representation of chat messages; serialized as `input.messages`.
    pub messages: Vec<ResponsesMessage>,

    // Sampling / decoding
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    /// Responses naming for output token cap.
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// Accepts a single string or an array of strings.
    #[serde(default)]
    pub stop: Option<serde_json::Value>,
    #[serde(default)]
    pub presence_penalty: Option<f64>,
    #[serde(default)]
    pub frequency_penalty: Option<f64>,
    #[serde(default)]
    pub logit_bias: Option<HashMap<String, f64>>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub n: Option<u32>,

    // Tools
    #[serde(default)]
    pub tools: Option<Vec<ResponsesToolDefinition>>,
    #[serde(default)]
    pub tool_choice: Option<serde_json::Value>,

    // Output shaping
    #[serde(default)]
    pub response_format: Option<serde_json::Value>,

    // Streaming
    #[serde(default)]
    pub stream: Option<bool>,

    // Optional: stateful conversation id in Responses
    #[serde(default)]
    pub conversation: Option<String>,

    /// Optional pointer to a prior Responses ID (state chaining)
    #[serde(default)]
    pub previous_response_id: Option<String>,

    /// Provider-specific extra options (e.g., Gemini thinking_config)
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
}

impl Serialize for ResponsesRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde_json::{Map, Number, Value};

        let mut root = Map::new();

        // Required: model
        root.insert("model".to_string(), Value::String(self.model.clone()));

        // Place chat messages under `input` (Responses API format)
        let messages_val =
            serde_json::to_value(&self.messages).map_err(serde::ser::Error::custom)?;
        root.insert("input".to_string(), messages_val);

        // Helper closures
        let to_num = |f: f64, label: &str| {
            Number::from_f64(f).ok_or_else(|| serde::ser::Error::custom(format!("invalid {label}")))
        };

        // Optional fields: only include when Some(..)
        if let Some(v) = self.temperature {
            root.insert(
                "temperature".into(),
                Value::Number(to_num(v, "temperature")?),
            );
        }
        if let Some(v) = self.top_p {
            root.insert("top_p".into(), Value::Number(to_num(v, "top_p")?));
        }
        if let Some(v) = self.max_output_tokens {
            root.insert("max_output_tokens".into(), Value::Number(v.into()));
        }
        if let Some(v) = self.stop.clone() {
            root.insert("stop".into(), v);
        }
        if let Some(v) = self.presence_penalty {
            root.insert(
                "presence_penalty".into(),
                Value::Number(to_num(v, "presence_penalty")?),
            );
        }
        if let Some(v) = self.frequency_penalty {
            root.insert(
                "frequency_penalty".into(),
                Value::Number(to_num(v, "frequency_penalty")?),
            );
        }
        if let Some(map) = self.logit_bias.as_ref() {
            let mut obj = Map::new();
            for (k, v) in map {
                let num = Number::from_f64(*v)
                    .ok_or_else(|| serde::ser::Error::custom("invalid logit_bias value"))?;
                obj.insert(k.clone(), Value::Number(num));
            }
            root.insert("logit_bias".into(), Value::Object(obj));
        }
        if let Some(u) = self.user.as_ref() {
            root.insert("user".into(), Value::String(u.clone()));
        }
        if let Some(n) = self.n {
            root.insert("n".into(), Value::Number(n.into()));
        }
        if let Some(tools) = self.tools.as_ref() {
            root.insert(
                "tools".into(),
                serde_json::to_value(tools).map_err(serde::ser::Error::custom)?,
            );
        }
        if let Some(tc) = self.tool_choice.as_ref() {
            root.insert("tool_choice".into(), tc.clone());
        }
        if let Some(rf) = self.response_format.as_ref() {
            root.insert("response_format".into(), rf.clone());
        }
        if let Some(s) = self.stream {
            root.insert("stream".into(), Value::Bool(s));
        }
        if let Some(conv) = self.conversation.as_ref() {
            root.insert("conversation".into(), Value::String(conv.clone()));
        }
        if let Some(prev) = self.previous_response_id.as_ref() {
            root.insert("previous_response_id".into(), Value::String(prev.clone()));
        }
        if let Some(extra) = self.extra_body.as_ref() {
            root.insert("extra_body".into(), extra.clone());
        }

        Value::Object(root).serialize(serializer)
    }
}

// ============================================================================
// Responses API Response Models
// ============================================================================

/// Output item types in Responses API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    #[serde(rename = "assistant_message")]
    AssistantMessage { id: String, content: String },
    Reasoning {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encrypted_content: Option<String>,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
        call_id: String,
    },
    #[serde(rename = "function_call_output")]
    FunctionCallOutput {
        id: String,
        call_id: String,
        content: String,
    },
}

/// Usage statistics in Responses API response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ResponsesUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,

    /// Reasoning tokens for reasoning-capable models (o1, o3, GPT-5)
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,

    /// Cached tokens (subset of input_tokens)
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

/// Complete Responses API response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ResponsesResponse {
    pub id: String,
    pub object: String, // "response"
    pub created: u64,
    pub model: String,

    /// Primary text output (convenience field)
    #[serde(default)]
    pub output_text: Option<String>,

    /// Array of output items (reasoning, messages, tool calls, etc.)
    pub output: Vec<OutputItem>,

    /// Token usage statistics
    #[serde(default)]
    pub usage: Option<ResponsesUsage>,

    #[serde(default)]
    pub system_fingerprint: Option<String>,
}

// ============================================================================
// Responses API Streaming Response Models
// ============================================================================

/// Streaming chunk from Responses API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ResponsesChunk {
    pub id: String,
    pub object: String, // "response.chunk"
    pub created: u64,
    pub model: String,

    /// Incremental text delta
    #[serde(default)]
    pub output_text_delta: Option<String>,

    /// Incremental output item deltas
    #[serde(default)]
    pub output_deltas: Option<Vec<OutputItem>>,

    /// Final usage (only in last chunk)
    #[serde(default)]
    pub usage: Option<ResponsesUsage>,
}
