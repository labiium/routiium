//! AWS Bedrock integration module
//!
//! Provides conversion between OpenAI Chat Completions / Responses API formats
//! and AWS Bedrock's InvokeModel API, with full support for:
//! - Tool/function calling
//! - Multimodal input (vision)
//! - Streaming responses
//! - AWS SigV4 authentication

use crate::models::chat;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Bedrock model provider types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BedrockProvider {
    Anthropic,
    AmazonTitan,
    AI21,
    Cohere,
    Meta,
    Mistral,
}

impl BedrockProvider {
    /// Detect provider from model ID
    pub fn from_model_id(model_id: &str) -> Result<Self> {
        if model_id.starts_with("anthropic.") {
            Ok(BedrockProvider::Anthropic)
        } else if model_id.starts_with("amazon.titan") {
            Ok(BedrockProvider::AmazonTitan)
        } else if model_id.starts_with("ai21.") {
            Ok(BedrockProvider::AI21)
        } else if model_id.starts_with("cohere.") {
            Ok(BedrockProvider::Cohere)
        } else if model_id.starts_with("meta.") {
            Ok(BedrockProvider::Meta)
        } else if model_id.starts_with("mistral.") {
            Ok(BedrockProvider::Mistral)
        } else {
            Err(anyhow!("Unknown Bedrock model provider: {}", model_id))
        }
    }
}

/// Bedrock message content part (for multimodal)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BedrockContentPart {
    Text { text: String },
    Image { source: BedrockImageSource },
}

/// Bedrock image source
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BedrockImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

/// Bedrock message format (Anthropic-style)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockMessage {
    pub role: String,
    pub content: Vec<BedrockContentPart>,
}

/// Bedrock tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: Value,
}

/// Bedrock tool use (in request/response)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockToolUse {
    #[serde(rename = "type")]
    pub tool_type: String, // "tool_use"
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// Bedrock tool result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockToolResult {
    #[serde(rename = "type")]
    pub tool_type: String, // "tool_result"
    pub tool_use_id: String,
    pub content: String,
}

/// Convert Chat Completions request to Bedrock format
pub fn chat_to_bedrock_request(chat_req: &chat::ChatCompletionRequest) -> Result<(String, Value)> {
    let provider = BedrockProvider::from_model_id(&chat_req.model)?;

    match provider {
        BedrockProvider::Anthropic => chat_to_anthropic_bedrock(chat_req),
        BedrockProvider::AmazonTitan => chat_to_titan_bedrock(chat_req),
        BedrockProvider::Meta => chat_to_meta_bedrock(chat_req),
        BedrockProvider::Mistral => chat_to_mistral_bedrock(chat_req),
        _ => Err(anyhow!(
            "Bedrock provider {:?} not yet implemented",
            provider
        )),
    }
}

/// Convert Chat Completions to Anthropic Claude format on Bedrock
fn chat_to_anthropic_bedrock(chat_req: &chat::ChatCompletionRequest) -> Result<(String, Value)> {
    let mut messages: Vec<BedrockMessage> = Vec::new();
    let mut system_prompt: Option<String> = None;

    // Process messages
    for msg in &chat_req.messages {
        match msg.role {
            chat::Role::System => {
                // Anthropic expects system as a separate parameter
                if let Some(text) = msg.content.as_str() {
                    system_prompt = Some(text.to_string());
                }
            }
            chat::Role::User | chat::Role::Assistant => {
                let content_parts = convert_message_content(&msg.content)?;
                messages.push(BedrockMessage {
                    role: role_to_bedrock_role(&msg.role),
                    content: content_parts,
                });
            }
            chat::Role::Tool => {
                // Tool results go as user messages with tool_result content
                if let Some(tool_call_id) = &msg.tool_call_id {
                    let content_text = msg.content.as_str().unwrap_or("").to_string();

                    messages.push(BedrockMessage {
                        role: "user".to_string(),
                        content: vec![BedrockContentPart::Text {
                            text: serde_json::to_string(&BedrockToolResult {
                                tool_type: "tool_result".to_string(),
                                tool_use_id: tool_call_id.clone(),
                                content: content_text,
                            })?,
                        }],
                    });
                }
            }
            chat::Role::Function => {
                // Legacy function role, treat as tool
                if let Some(name) = &msg.name {
                    let content_text = msg.content.as_str().unwrap_or("").to_string();

                    messages.push(BedrockMessage {
                        role: "user".to_string(),
                        content: vec![BedrockContentPart::Text {
                            text: format!("Function {} result: {}", name, content_text),
                        }],
                    });
                }
            }
        }
    }

    // Handle assistant messages with tool calls
    let mut processed_messages: Vec<Value> = Vec::new();
    for msg in &messages {
        let mut msg_content: Vec<Value> = Vec::new();

        for part in &msg.content {
            match part {
                BedrockContentPart::Text { text } => {
                    msg_content.push(json!({
                        "type": "text",
                        "text": text
                    }));
                }
                BedrockContentPart::Image { source } => {
                    msg_content.push(match source {
                        BedrockImageSource::Base64 { media_type, data } => json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": media_type,
                                "data": data
                            }
                        }),
                        BedrockImageSource::Url { url } => json!({
                            "type": "image",
                            "source": {
                                "type": "url",
                                "url": url
                            }
                        }),
                    });
                }
            }
        }

        processed_messages.push(json!({
            "role": msg.role,
            "content": msg_content
        }));
    }

    // Build request body
    let mut body = json!({
        "anthropic_version": "bedrock-2023-05-31",
        "max_tokens": chat_req.max_tokens.or(chat_req.max_completion_tokens).unwrap_or(4096),
        "messages": processed_messages,
    });

    if let Some(sys) = system_prompt {
        body["system"] = json!(sys);
    }

    if let Some(temp) = chat_req.temperature {
        body["temperature"] = json!(temp);
    }

    if let Some(top_p) = chat_req.top_p {
        body["top_p"] = json!(top_p);
    }

    if let Some(stop) = &chat_req.stop {
        body["stop_sequences"] = stop.clone();
    }

    // Convert tools to Bedrock format
    if let Some(tools) = &chat_req.tools {
        let bedrock_tools: Vec<Value> = tools
            .iter()
            .filter_map(|tool| match tool {
                chat::ToolDefinition::Function { function } => Some(json!({
                    "name": function.name,
                    "description": function.description.as_ref().unwrap_or(&function.name),
                    "input_schema": function.parameters
                })),
            })
            .collect();

        if !bedrock_tools.is_empty() {
            body["tools"] = json!(bedrock_tools);
        }

        // Handle tool_choice
        if let Some(tool_choice) = &chat_req.tool_choice {
            if let Some(choice_str) = tool_choice.as_str() {
                match choice_str {
                    "auto" => {
                        body["tool_choice"] = json!({"type": "auto"});
                    }
                    "any" | "required" => {
                        body["tool_choice"] = json!({"type": "any"});
                    }
                    "none" => {
                        // Don't include tool_choice
                    }
                    _ => {}
                }
            } else if let Some(obj) = tool_choice.as_object() {
                if let Some(func) = obj.get("function").and_then(|f| f.as_object()) {
                    if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                        body["tool_choice"] = json!({
                            "type": "tool",
                            "name": name
                        });
                    }
                }
            }
        }
    }

    Ok(("application/json".to_string(), body))
}

/// Convert Chat Completions to Amazon Titan format
fn chat_to_titan_bedrock(chat_req: &chat::ChatCompletionRequest) -> Result<(String, Value)> {
    // Titan uses a simpler prompt-based format
    let mut prompt = String::new();

    for msg in &chat_req.messages {
        let role_prefix = match msg.role {
            chat::Role::System => "System: ",
            chat::Role::User => "User: ",
            chat::Role::Assistant => "Assistant: ",
            _ => "",
        };

        if let Some(text) = msg.content.as_str() {
            prompt.push_str(role_prefix);
            prompt.push_str(text);
            prompt.push('\n');
        }
    }

    prompt.push_str("Assistant: ");

    let body = json!({
        "inputText": prompt,
        "textGenerationConfig": {
            "maxTokenCount": chat_req.max_tokens.unwrap_or(512),
            "temperature": chat_req.temperature.unwrap_or(0.7),
            "topP": chat_req.top_p.unwrap_or(0.9),
        }
    });

    Ok(("application/json".to_string(), body))
}

/// Convert Chat Completions to Meta Llama format
fn chat_to_meta_bedrock(chat_req: &chat::ChatCompletionRequest) -> Result<(String, Value)> {
    let mut prompt = String::new();

    for msg in &chat_req.messages {
        match msg.role {
            chat::Role::System => {
                if let Some(text) = msg.content.as_str() {
                    prompt.push_str(&format!("<s>[INST] <<SYS>>\n{}\n<</SYS>>\n\n", text));
                }
            }
            chat::Role::User => {
                if let Some(text) = msg.content.as_str() {
                    prompt.push_str(&format!("{} [/INST]", text));
                }
            }
            chat::Role::Assistant => {
                if let Some(text) = msg.content.as_str() {
                    prompt.push_str(&format!(" {} </s><s>[INST] ", text));
                }
            }
            _ => {}
        }
    }

    let body = json!({
        "prompt": prompt,
        "max_gen_len": chat_req.max_tokens.unwrap_or(512),
        "temperature": chat_req.temperature.unwrap_or(0.5),
        "top_p": chat_req.top_p.unwrap_or(0.9),
    });

    Ok(("application/json".to_string(), body))
}

/// Convert Chat Completions to Mistral format
fn chat_to_mistral_bedrock(chat_req: &chat::ChatCompletionRequest) -> Result<(String, Value)> {
    // Mistral uses a messages format similar to OpenAI/Anthropic
    let mut messages: Vec<Value> = Vec::new();

    for msg in &chat_req.messages {
        let role = match msg.role {
            chat::Role::System => "system",
            chat::Role::User => "user",
            chat::Role::Assistant => "assistant",
            chat::Role::Tool => {
                // Handle tool results
                if let Some(tool_call_id) = &msg.tool_call_id {
                    let content_text = msg.content.as_str().unwrap_or("").to_string();
                    messages.push(json!({
                        "role": "tool",
                        "name": tool_call_id,
                        "content": content_text
                    }));
                }
                continue;
            }
            _ => continue,
        };

        // Handle multimodal content (text + images)
        if let Some(content_array) = msg.content.as_array() {
            let mut content_parts: Vec<Value> = Vec::new();

            for part in content_array {
                if let Some(obj) = part.as_object() {
                    if let Some(type_str) = obj.get("type").and_then(|t| t.as_str()) {
                        match type_str {
                            "text" => {
                                if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                                    content_parts.push(json!({
                                        "type": "text",
                                        "text": text
                                    }));
                                }
                            }
                            "image_url" => {
                                if let Some(image_url) = obj.get("image_url") {
                                    if let Some(url) = image_url.get("url").and_then(|u| u.as_str())
                                    {
                                        // Parse base64 image data
                                        if url.starts_with("data:") {
                                            if let Some(comma_pos) = url.find(',') {
                                                let header = &url[..comma_pos];
                                                let data = &url[comma_pos + 1..];

                                                // Extract media type
                                                let media_type = if header.contains("image/jpeg") {
                                                    "image/jpeg"
                                                } else if header.contains("image/png") {
                                                    "image/png"
                                                } else if header.contains("image/gif") {
                                                    "image/gif"
                                                } else if header.contains("image/webp") {
                                                    "image/webp"
                                                } else {
                                                    "image/jpeg" // default
                                                };

                                                content_parts.push(json!({
                                                    "type": "image",
                                                    "source": {
                                                        "type": "base64",
                                                        "media_type": media_type,
                                                        "data": data
                                                    }
                                                }));
                                            }
                                        } else {
                                            // URL image
                                            content_parts.push(json!({
                                                "type": "image",
                                                "source": {
                                                    "type": "url",
                                                    "url": url
                                                }
                                            }));
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            if !content_parts.is_empty() {
                messages.push(json!({
                    "role": role,
                    "content": content_parts
                }));
            }
        } else if let Some(text) = msg.content.as_str() {
            // Simple text content
            messages.push(json!({
                "role": role,
                "content": text
            }));
        }

        // Handle tool calls from assistant
        if role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                let mut tool_uses: Vec<Value> = Vec::new();
                for tool_call in tool_calls {
                    tool_uses.push(json!({
                        "type": "tool_use",
                        "id": tool_call.id,
                        "name": tool_call.function.name,
                        "input": serde_json::from_str::<Value>(&tool_call.function.arguments).unwrap_or(json!({}))
                    }));
                }

                // Add tool uses to message content
                if !tool_uses.is_empty() {
                    if let Some(last_msg) = messages.last_mut() {
                        if let Some(content) = last_msg.get_mut("content") {
                            if let Some(content_array) = content.as_array_mut() {
                                content_array.extend(tool_uses);
                            } else if content.is_string() {
                                // Convert string to array with text and tool_uses
                                let text = content.as_str().unwrap_or("").to_string();
                                let mut new_content = vec![json!({"type": "text", "text": text})];
                                new_content.extend(tool_uses);
                                *content = json!(new_content);
                            }
                        }
                    }
                }
            }
        }
    }

    let mut body = json!({
        "messages": messages,
        "max_tokens": chat_req.max_tokens.unwrap_or(512),
    });

    if let Some(temp) = chat_req.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(top_p) = chat_req.top_p {
        body["top_p"] = json!(top_p);
    }

    // Add tool definitions if present
    if let Some(tools) = &chat_req.tools {
        let mut mistral_tools: Vec<Value> = Vec::new();
        for tool in tools {
            let chat::ToolDefinition::Function { function } = tool;
            mistral_tools.push(json!({
                "type": "function",
                "function": {
                    "name": function.name,
                    "description": function.description.as_ref().unwrap_or(&String::new()),
                    "parameters": function.parameters
                }
            }));
        }
        if !mistral_tools.is_empty() {
            body["tools"] = json!(mistral_tools);
        }
    }

    Ok(("application/json".to_string(), body))
}

/// Convert message content (handles multimodal)
fn convert_message_content(content: &Value) -> Result<Vec<BedrockContentPart>> {
    match content {
        Value::String(text) => Ok(vec![BedrockContentPart::Text { text: text.clone() }]),
        Value::Array(parts) => {
            let mut result = Vec::new();
            for part in parts {
                if let Some(obj) = part.as_object() {
                    if let Some(type_str) = obj.get("type").and_then(|t| t.as_str()) {
                        match type_str {
                            "text" => {
                                if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                                    result.push(BedrockContentPart::Text {
                                        text: text.to_string(),
                                    });
                                }
                            }
                            "image_url" => {
                                if let Some(image_url) = obj.get("image_url") {
                                    result.push(convert_image_url(image_url)?);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Ok(result)
        }
        _ => Ok(vec![BedrockContentPart::Text {
            text: content.to_string(),
        }]),
    }
}

/// Convert image_url to Bedrock image format
fn convert_image_url(image_url: &Value) -> Result<BedrockContentPart> {
    let url = if let Some(url_str) = image_url.get("url").and_then(|u| u.as_str()) {
        url_str
    } else if let Some(url_str) = image_url.as_str() {
        url_str
    } else {
        return Err(anyhow!("Invalid image_url format"));
    };

    // Check if it's a base64 data URL
    if url.starts_with("data:") {
        if let Some(comma_pos) = url.find(',') {
            let header = &url[5..comma_pos]; // Skip "data:"
            let data = &url[comma_pos + 1..];

            // Parse media type from header (e.g., "image/png;base64")
            let media_type = if let Some(semicolon) = header.find(';') {
                header[..semicolon].to_string()
            } else {
                header.to_string()
            };

            return Ok(BedrockContentPart::Image {
                source: BedrockImageSource::Base64 {
                    media_type,
                    data: data.to_string(),
                },
            });
        }
    }

    // Otherwise treat as URL (note: Bedrock may not support external URLs for all models)
    Ok(BedrockContentPart::Image {
        source: BedrockImageSource::Url {
            url: url.to_string(),
        },
    })
}

/// Convert role to Bedrock role
fn role_to_bedrock_role(role: &chat::Role) -> String {
    match role {
        chat::Role::User => "user".to_string(),
        chat::Role::Assistant => "assistant".to_string(),
        chat::Role::System => "user".to_string(), // Systems prompts go separately
        chat::Role::Tool => "user".to_string(),
        chat::Role::Function => "user".to_string(),
    }
}

/// Convert Bedrock response to Chat Completions format
pub fn bedrock_to_chat_response(
    bedrock_response: Value,
    model: &str,
    request_id: Option<String>,
) -> Result<chat::ChatCompletionResponse> {
    let provider = BedrockProvider::from_model_id(model)?;

    match provider {
        BedrockProvider::Anthropic => {
            bedrock_anthropic_to_chat(bedrock_response, model, request_id)
        }
        BedrockProvider::AmazonTitan => bedrock_titan_to_chat(bedrock_response, model, request_id),
        BedrockProvider::Meta => bedrock_meta_to_chat(bedrock_response, model, request_id),
        BedrockProvider::Mistral => bedrock_mistral_to_chat(bedrock_response, model, request_id),
        _ => Err(anyhow!(
            "Bedrock provider {:?} not yet implemented",
            provider
        )),
    }
}

/// Convert Anthropic Bedrock response to Chat Completions
fn bedrock_anthropic_to_chat(
    response: Value,
    model: &str,
    request_id: Option<String>,
) -> Result<chat::ChatCompletionResponse> {
    let mut content_text = String::new();
    let mut tool_calls: Vec<chat::ToolCall> = Vec::new();
    let mut finish_reason = "stop";

    // Parse content array
    if let Some(content_array) = response.get("content").and_then(|c| c.as_array()) {
        for item in content_array {
            if let Some(item_type) = item.get("type").and_then(|t| t.as_str()) {
                match item_type {
                    "text" => {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                            if !content_text.is_empty() {
                                content_text.push('\n');
                            }
                            content_text.push_str(text);
                        }
                    }
                    "tool_use" => {
                        if let (Some(id), Some(name), Some(input)) = (
                            item.get("id").and_then(|i| i.as_str()),
                            item.get("name").and_then(|n| n.as_str()),
                            item.get("input"),
                        ) {
                            tool_calls.push(chat::ToolCall {
                                id: id.to_string(),
                                call_type: "function".to_string(),
                                function: chat::FunctionCall {
                                    name: name.to_string(),
                                    arguments: serde_json::to_string(input)?,
                                },
                            });
                            finish_reason = "tool_calls";
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Parse stop reason
    if let Some(stop_reason) = response.get("stop_reason").and_then(|s| s.as_str()) {
        finish_reason = match stop_reason {
            "end_turn" => "stop",
            "max_tokens" => "length",
            "tool_use" => "tool_calls",
            _ => stop_reason,
        };
    }

    // Parse usage
    let usage = if let Some(usage_obj) = response.get("usage") {
        let prompt_tokens = usage_obj
            .get("input_tokens")
            .and_then(|t| t.as_u64())
            .unwrap_or(0);
        let completion_tokens = usage_obj
            .get("output_tokens")
            .and_then(|t| t.as_u64())
            .unwrap_or(0);

        Some(chat::ChatUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            reasoning_tokens: None,
            cached_tokens: None,
        })
    } else {
        None
    };

    let message = chat::ChatResponseMessage {
        role: "assistant".to_string(),
        content: if content_text.is_empty() && !tool_calls.is_empty() {
            None
        } else {
            Some(content_text)
        },
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        function_call: None,
    };

    let id = request_id.unwrap_or_else(|| format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()));

    Ok(chat::ChatCompletionResponse {
        id,
        object: "chat.completion".to_string(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        model: model.to_string(),
        choices: vec![chat::ChatChoice {
            index: 0,
            message,
            finish_reason: Some(finish_reason.to_string()),
            logprobs: None,
        }],
        usage,
        system_fingerprint: None,
    })
}

/// Convert Titan Bedrock response to Chat Completions
fn bedrock_titan_to_chat(
    response: Value,
    model: &str,
    request_id: Option<String>,
) -> Result<chat::ChatCompletionResponse> {
    let content = response
        .get("results")
        .and_then(|r| r.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("outputText"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    let finish_reason = response
        .get("results")
        .and_then(|r| r.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("completionReason"))
        .and_then(|r| r.as_str())
        .map(|r| match r {
            "FINISH" => "stop",
            "LENGTH" => "length",
            _ => "stop",
        })
        .unwrap_or("stop");

    let usage =
        if let Some(input_tokens) = response.get("inputTextTokenCount").and_then(|t| t.as_u64()) {
            let output_tokens = response
                .get("results")
                .and_then(|r| r.as_array())
                .and_then(|arr| arr.first())
                .and_then(|item| item.get("tokenCount"))
                .and_then(|t| t.as_u64())
                .unwrap_or(0);

            Some(chat::ChatUsage {
                prompt_tokens: input_tokens,
                completion_tokens: output_tokens,
                total_tokens: input_tokens + output_tokens,
                reasoning_tokens: None,
                cached_tokens: None,
            })
        } else {
            None
        };

    let id = request_id.unwrap_or_else(|| format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()));

    Ok(chat::ChatCompletionResponse {
        id,
        object: "chat.completion".to_string(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        model: model.to_string(),
        choices: vec![chat::ChatChoice {
            index: 0,
            message: chat::ChatResponseMessage {
                role: "assistant".to_string(),
                content: Some(content),
                tool_calls: None,
                function_call: None,
            },
            finish_reason: Some(finish_reason.to_string()),
            logprobs: None,
        }],
        usage,
        system_fingerprint: None,
    })
}

/// Convert Meta Bedrock response to Chat Completions
fn bedrock_meta_to_chat(
    response: Value,
    model: &str,
    request_id: Option<String>,
) -> Result<chat::ChatCompletionResponse> {
    let content = response
        .get("generation")
        .and_then(|g| g.as_str())
        .unwrap_or("")
        .to_string();

    let finish_reason = response
        .get("stop_reason")
        .and_then(|r| r.as_str())
        .map(|r| match r {
            "stop" => "stop",
            "length" => "length",
            _ => "stop",
        })
        .unwrap_or("stop");

    let usage =
        if let Some(prompt_tokens) = response.get("prompt_token_count").and_then(|t| t.as_u64()) {
            let completion_tokens = response
                .get("generation_token_count")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);

            Some(chat::ChatUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
                reasoning_tokens: None,
                cached_tokens: None,
            })
        } else {
            None
        };

    let id = request_id.unwrap_or_else(|| format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()));

    Ok(chat::ChatCompletionResponse {
        id,
        object: "chat.completion".to_string(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        model: model.to_string(),
        choices: vec![chat::ChatChoice {
            index: 0,
            message: chat::ChatResponseMessage {
                role: "assistant".to_string(),
                content: Some(content),
                tool_calls: None,
                function_call: None,
            },
            finish_reason: Some(finish_reason.to_string()),
            logprobs: None,
        }],
        usage,
        system_fingerprint: None,
    })
}

/// Convert Mistral Bedrock response to Chat Completions
fn bedrock_mistral_to_chat(
    response: Value,
    model: &str,
    request_id: Option<String>,
) -> Result<chat::ChatCompletionResponse> {
    // Mistral response can be in different formats
    let mut content_text = String::new();
    let mut tool_calls: Vec<chat::ToolCall> = Vec::new();
    let mut finish_reason = "stop";

    // Check for new Ministral format (similar to Anthropic)
    if let Some(content_array) = response.get("content").and_then(|c| c.as_array()) {
        for item in content_array {
            if let Some(obj) = item.as_object() {
                if let Some(type_str) = obj.get("type").and_then(|t| t.as_str()) {
                    match type_str {
                        "text" => {
                            if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                                if !content_text.is_empty() {
                                    content_text.push('\n');
                                }
                                content_text.push_str(text);
                            }
                        }
                        "tool_use" => {
                            if let (Some(id), Some(name), Some(input)) = (
                                obj.get("id").and_then(|i| i.as_str()),
                                obj.get("name").and_then(|n| n.as_str()),
                                obj.get("input"),
                            ) {
                                tool_calls.push(chat::ToolCall {
                                    id: id.to_string(),
                                    call_type: "function".to_string(),
                                    function: chat::FunctionCall {
                                        name: name.to_string(),
                                        arguments: serde_json::to_string(input)?,
                                    },
                                });
                                finish_reason = "tool_calls";
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Check for stop reason
        if let Some(stop_reason) = response.get("stop_reason").and_then(|s| s.as_str()) {
            finish_reason = match stop_reason {
                "end_turn" => "stop",
                "max_tokens" => "length",
                "tool_use" => "tool_calls",
                _ => stop_reason,
            };
        }
    } else if let Some(outputs) = response.get("outputs").and_then(|o| o.as_array()) {
        // Legacy Mistral format: {"outputs": [{"text": "..."}]}
        if let Some(first_output) = outputs.first() {
            content_text = first_output
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();

            if let Some(stop_reason) = first_output.get("stop_reason").and_then(|r| r.as_str()) {
                finish_reason = match stop_reason {
                    "stop" => "stop",
                    "length" | "max_tokens" => "length",
                    _ => "stop",
                };
            }
        }
    }

    // Extract token usage
    let usage = if let Some(usage_obj) = response.get("usage") {
        let prompt_tokens = usage_obj
            .get("prompt_tokens")
            .or_else(|| usage_obj.get("input_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0);
        let completion_tokens = usage_obj
            .get("completion_tokens")
            .or_else(|| usage_obj.get("output_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0);

        Some(chat::ChatUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            reasoning_tokens: None,
            cached_tokens: None,
        })
    } else {
        None
    };

    let id = request_id.unwrap_or_else(|| format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()));

    Ok(chat::ChatCompletionResponse {
        id,
        object: "chat.completion".to_string(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        model: model.to_string(),
        choices: vec![chat::ChatChoice {
            index: 0,
            message: chat::ChatResponseMessage {
                role: "assistant".to_string(),
                content: if content_text.is_empty() && !tool_calls.is_empty() {
                    None
                } else {
                    Some(content_text)
                },
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                function_call: None,
            },
            finish_reason: Some(finish_reason.to_string()),
            logprobs: None,
        }],
        usage,
        system_fingerprint: None,
    })
}

/// AWS Configuration
#[derive(Debug, Clone)]
pub struct AwsConfig {
    pub region: String,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
}

impl AwsConfig {
    /// Load AWS config from environment variables
    pub fn from_env() -> Self {
        Self {
            region: std::env::var("AWS_REGION")
                .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                .unwrap_or_else(|_| "us-east-1".to_string()),
            access_key_id: std::env::var("AWS_ACCESS_KEY_ID").ok(),
            secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").ok(),
            session_token: std::env::var("AWS_SESSION_TOKEN").ok(),
        }
    }

    /// Get region from optional override or config
    pub fn get_region(&self, override_region: Option<&str>) -> String {
        override_region
            .map(|r| r.to_string())
            .unwrap_or_else(|| self.region.clone())
    }
}

// Note: AWS SigV4 signing is handled automatically by the AWS SDK
// when using invoke_bedrock_model and invoke_bedrock_model_streaming functions

/// Invoke Bedrock model using AWS SDK
///
/// This is the recommended way to call Bedrock as it handles all AWS authentication
/// and signing automatically.
pub async fn invoke_bedrock_model(model_id: &str, body: Value, region: &str) -> Result<Value> {
    use aws_config::BehaviorVersion;
    use aws_sdk_bedrockruntime::config::Region;
    use aws_sdk_bedrockruntime::primitives::Blob;

    // Load AWS SDK config
    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(region.to_string()))
        .load()
        .await;

    let client = aws_sdk_bedrockruntime::Client::new(&sdk_config);

    // Convert JSON body to bytes
    let body_bytes = serde_json::to_vec(&body)?;

    // Invoke the model
    let response = client
        .invoke_model()
        .model_id(model_id)
        .body(Blob::new(body_bytes))
        .send()
        .await
        .map_err(|e| anyhow!("Bedrock invocation failed: {}", e))?;

    // Parse response body
    let response_body = response.body().as_ref();
    let response_json: Value = serde_json::from_slice(response_body)?;

    Ok(response_json)
}

/// Streaming event from Bedrock
#[derive(Debug)]
pub struct BedrockStreamEvent {
    pub chunk: Option<Value>,
    pub done: bool,
}

/// Invoke Bedrock model with streaming using AWS SDK
///
/// Returns a stream of events that can be converted to SSE format
pub async fn invoke_bedrock_model_streaming(
    model_id: &str,
    body: Value,
    region: &str,
) -> Result<impl futures_util::Stream<Item = Result<BedrockStreamEvent>>> {
    use aws_config::BehaviorVersion;
    use aws_sdk_bedrockruntime::config::Region;
    use aws_sdk_bedrockruntime::primitives::Blob;

    // Load AWS SDK config
    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(region.to_string()))
        .load()
        .await;

    let client = aws_sdk_bedrockruntime::Client::new(&sdk_config);

    // Convert JSON body to bytes
    let body_bytes = serde_json::to_vec(&body)?;

    // Invoke the model with streaming
    let response = client
        .invoke_model_with_response_stream()
        .model_id(model_id)
        .body(Blob::new(body_bytes))
        .send()
        .await
        .map_err(|e| anyhow!("Bedrock streaming invocation failed: {}", e))?;

    // Get the event stream
    let mut stream = response.body;

    // Create a stream that yields our events
    Ok(async_stream::stream! {
        loop {
            match stream.recv().await {
                Ok(Some(event)) => {
                    use aws_sdk_bedrockruntime::types::ResponseStream;

                    match event {
                        ResponseStream::Chunk(chunk) => {
                            // Get the bytes from the chunk
                            if let Some(bytes) = chunk.bytes() {
                                // Parse the chunk bytes as JSON
                                if let Ok(chunk_json) = serde_json::from_slice::<Value>(bytes.as_ref()) {
                                    yield Ok(BedrockStreamEvent {
                                        chunk: Some(chunk_json),
                                        done: false,
                                    });
                                }
                            }
                        }
                        _ => {
                            // Other event types, skip
                        }
                    }
                }
                Ok(None) => {
                    // Stream ended
                    break;
                }
                Err(e) => {
                    yield Err(anyhow!("Stream error: {}", e));
                    break;
                }
            }
        }

        // Send final done event
        yield Ok(BedrockStreamEvent {
            chunk: None,
            done: true,
        });
    })
}

/// Convert Bedrock streaming chunk to Chat Completions SSE format
pub fn bedrock_chunk_to_sse(
    chunk: &Value,
    model: &str,
    provider: &BedrockProvider,
) -> Result<String> {
    let delta = match provider {
        BedrockProvider::Anthropic => {
            // Anthropic streaming format
            if let Some(delta_obj) = chunk.get("delta") {
                if let Some(text) = delta_obj.get("text").and_then(|t| t.as_str()) {
                    json!({
                        "content": text
                    })
                } else if delta_obj.get("type").and_then(|t| t.as_str())
                    == Some("content_block_stop")
                {
                    return Ok(format!(
                        "data: {}\n\n",
                        json!({
                            "id": format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
                            "object": "chat.completion.chunk",
                            "created": std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                            "model": model,
                            "choices": [{
                                "index": 0,
                                "delta": {},
                                "finish_reason": "stop"
                            }]
                        })
                    ));
                } else {
                    json!({})
                }
            } else {
                json!({})
            }
        }
        BedrockProvider::Mistral => {
            // Mistral streaming format (similar to Anthropic for newer models)
            if let Some(delta_obj) = chunk.get("delta") {
                if let Some(text) = delta_obj.get("text").and_then(|t| t.as_str()) {
                    json!({
                        "content": text
                    })
                } else if delta_obj.get("type").and_then(|t| t.as_str())
                    == Some("content_block_stop")
                {
                    return Ok(format!(
                        "data: {}\n\n",
                        json!({
                            "id": format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
                            "object": "chat.completion.chunk",
                            "created": std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                            "model": model,
                            "choices": [{
                                "index": 0,
                                "delta": {},
                                "finish_reason": "stop"
                            }]
                        })
                    ));
                } else {
                    json!({})
                }
            } else if let Some(text) = chunk.get("outputText").and_then(|t| t.as_str()) {
                // Fallback for legacy format
                json!({
                    "content": text
                })
            } else {
                json!({})
            }
        }
        BedrockProvider::AmazonTitan | BedrockProvider::Meta => {
            // These providers use simpler streaming formats
            if let Some(text) = chunk.get("outputText").and_then(|t| t.as_str()) {
                json!({
                    "content": text
                })
            } else {
                json!({})
            }
        }
        _ => json!({}),
    };

    let sse_event = json!({
        "id": format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
        "object": "chat.completion.chunk",
        "created": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": null
        }]
    });

    Ok(format!("data: {}\n\n", sse_event))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::chat::{
        ChatCompletionRequest, ChatMessage, FunctionDef, Role, ToolDefinition,
    };

    #[test]
    fn test_provider_detection() {
        assert_eq!(
            BedrockProvider::from_model_id("anthropic.claude-3-sonnet-20240229-v1:0").unwrap(),
            BedrockProvider::Anthropic
        );
        assert_eq!(
            BedrockProvider::from_model_id("amazon.titan-text-express-v1").unwrap(),
            BedrockProvider::AmazonTitan
        );
        assert_eq!(
            BedrockProvider::from_model_id("meta.llama3-70b-instruct-v1:0").unwrap(),
            BedrockProvider::Meta
        );
        assert_eq!(
            BedrockProvider::from_model_id("mistral.mistral-7b-instruct-v0:2").unwrap(),
            BedrockProvider::Mistral
        );
    }

    #[test]
    fn test_simple_chat_to_anthropic() {
        let req = ChatCompletionRequest {
            model: "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
            messages: vec![
                ChatMessage {
                    role: Role::System,
                    content: json!("You are a helpful assistant"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                ChatMessage {
                    role: Role::User,
                    content: json!("Hello!"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
            ],
            temperature: Some(0.7),
            top_p: None,
            max_tokens: Some(100),
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
        };

        let result = chat_to_bedrock_request(&req);
        assert!(result.is_ok());

        let (content_type, body) = result.unwrap();
        assert_eq!(content_type, "application/json");
        assert!(body.get("system").is_some());
        assert_eq!(body["system"], "You are a helpful assistant");
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["temperature"], 0.7);
    }

    #[test]
    fn test_chat_with_tools() {
        let req = ChatCompletionRequest {
            model: "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("What's the weather?"),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: Some(1024),
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: Some(vec![ToolDefinition::Function {
                function: FunctionDef {
                    name: "get_weather".to_string(),
                    description: Some("Get weather for location".to_string()),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "location": {"type": "string"}
                        }
                    }),
                },
            }]),
            tool_choice: Some(json!("auto")),
            response_format: None,
            stream: None,
        };

        let result = chat_to_bedrock_request(&req);
        assert!(result.is_ok());

        let (_, body) = result.unwrap();
        assert!(body.get("tools").is_some());
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "get_weather");
    }

    #[test]
    fn test_multimodal_conversion() {
        let content = json!([
            {"type": "text", "text": "What's in this image?"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KG"}}
        ]);

        let parts = convert_message_content(&content).unwrap();
        assert_eq!(parts.len(), 2);

        match &parts[0] {
            BedrockContentPart::Text { text } => {
                assert_eq!(text, "What's in this image?");
            }
            _ => panic!("Expected text part"),
        }

        match &parts[1] {
            BedrockContentPart::Image { source } => match source {
                BedrockImageSource::Base64 { media_type, data } => {
                    assert_eq!(media_type, "image/png");
                    assert_eq!(data, "iVBORw0KG");
                }
                _ => panic!("Expected base64 source"),
            },
            _ => panic!("Expected image part"),
        }
    }

    #[test]
    fn test_mistral_conversion() {
        let req = ChatCompletionRequest {
            model: "mistral.mistral-7b-instruct-v0:2".to_string(),
            messages: vec![
                ChatMessage {
                    role: Role::System,
                    content: json!("You are a helpful assistant"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                ChatMessage {
                    role: Role::User,
                    content: json!("Hello!"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
            ],
            temperature: Some(0.7),
            top_p: None,
            max_tokens: Some(100),
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
        };

        let result = chat_to_bedrock_request(&req);
        assert!(result.is_ok());

        let (content_type, body) = result.unwrap();
        assert_eq!(content_type, "application/json");
        assert!(body.get("messages").is_some());
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["temperature"], 0.7);
    }

    #[test]
    fn test_mistral_ministral_with_tools() {
        let req = ChatCompletionRequest {
            model: "mistral.ministral-3b-instruct".to_string(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: json!("What's the weather in Paris?"),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: Some(1024),
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: Some(vec![ToolDefinition::Function {
                function: FunctionDef {
                    name: "get_weather".to_string(),
                    description: Some("Get weather for a location".to_string()),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "location": {
                                "type": "string",
                                "description": "City name"
                            },
                            "unit": {
                                "type": "string",
                                "enum": ["celsius", "fahrenheit"]
                            }
                        },
                        "required": ["location"]
                    }),
                },
            }]),
            tool_choice: Some(json!("auto")),
            response_format: None,
            stream: None,
        };

        let result = chat_to_bedrock_request(&req);
        assert!(result.is_ok());

        let (_, body) = result.unwrap();
        assert!(body.get("tools").is_some());
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "get_weather");
        assert!(tools[0]["function"]["parameters"].is_object());
    }

    #[test]
    fn test_mistral_ministral_multimodal() {
        let content = json!([
            {"type": "text", "text": "What's in this image?"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KGgoAAAANSU"}}
        ]);

        let req = ChatCompletionRequest {
            model: "mistral.ministral-3b-instruct".to_string(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: content,
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens: Some(512),
            max_completion_tokens: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            n: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            stream: None,
        };

        let result = chat_to_bedrock_request(&req);
        assert!(result.is_ok());

        let (_, body) = result.unwrap();
        assert!(body.get("messages").is_some());
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);

        let content = messages[0].get("content").unwrap();
        assert!(content.is_array());
        let content_array = content.as_array().unwrap();
        assert_eq!(content_array.len(), 2);

        // Check text part
        assert_eq!(content_array[0]["type"], "text");
        assert_eq!(content_array[0]["text"], "What's in this image?");

        // Check image part
        assert_eq!(content_array[1]["type"], "image");
        assert_eq!(content_array[1]["source"]["type"], "base64");
        assert_eq!(content_array[1]["source"]["media_type"], "image/png");
        assert_eq!(content_array[1]["source"]["data"], "iVBORw0KGgoAAAANSU");
    }

    #[test]
    fn test_mistral_ministral_response_with_tools() {
        // Simulate Mistral response with tool use
        let response = json!({
            "content": [
                {
                    "type": "text",
                    "text": "I'll check the weather for you."
                },
                {
                    "type": "tool_use",
                    "id": "tool_123",
                    "name": "get_weather",
                    "input": {
                        "location": "Paris",
                        "unit": "celsius"
                    }
                }
            ],
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 25,
                "output_tokens": 15
            }
        });

        let result = bedrock_mistral_to_chat(response, "mistral.ministral-3b-instruct", None);
        assert!(result.is_ok());

        let chat_response = result.unwrap();
        assert_eq!(chat_response.choices.len(), 1);

        let choice = &chat_response.choices[0];
        assert_eq!(choice.finish_reason, Some("tool_calls".to_string()));

        // Check tool calls
        assert!(choice.message.tool_calls.is_some());
        let tool_calls = choice.message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "tool_123");
        assert_eq!(tool_calls[0].function.name, "get_weather");

        // Check usage
        assert!(chat_response.usage.is_some());
        let usage = chat_response.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 25);
        assert_eq!(usage.completion_tokens, 15);
        assert_eq!(usage.total_tokens, 40);
    }
}
