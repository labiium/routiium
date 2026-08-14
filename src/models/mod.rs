#![allow(clippy::unwrap_used)]
//! Data models for the Chat Completions and Responses APIs.
//!
//! This module groups two submodules:
//! - `chat`: Types representing a commonly used subset of the `OpenAI` Chat Completions request models.
//! - `responses`: Types representing a minimal yet robust subset of the `OpenAI` Responses API request models.
//!
//! The mapping logic that converts `chat::ChatCompletionRequest` to
//! `responses::ResponsesRequest` is implemented in `crate::conversion`.

pub mod chat;
pub mod responses;

// Optional convenience re-exports for downstream users.
// These allow importing commonly-used types directly from `routiium::models::*`.
pub use chat::{
    ChatCompletionRequest, ChatMessage, FunctionDef, ResponseFormat, Role, ToolDefinition,
};
pub use responses::{ResponsesMessage, ResponsesRequest, ResponsesToolDefinition};
