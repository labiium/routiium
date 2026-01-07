-- Chat History SQL Schema
-- Compatible with SQLite, PostgreSQL, and Turso (libsql)

-- Conversations table
CREATE TABLE IF NOT EXISTS conversations (
    conversation_id TEXT PRIMARY KEY,
    created_at BIGINT NOT NULL,
    last_seen_at BIGINT NOT NULL,
    title TEXT,
    metadata TEXT -- JSON serialized
);

CREATE INDEX IF NOT EXISTS idx_conversations_created_at ON conversations(created_at);
CREATE INDEX IF NOT EXISTS idx_conversations_last_seen_at ON conversations(last_seen_at);

-- Messages table
CREATE TABLE IF NOT EXISTS messages (
    message_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    request_id TEXT,
    role TEXT NOT NULL,
    content TEXT NOT NULL, -- JSON serialized
    tool_calls TEXT, -- JSON serialized, nullable
    created_at BIGINT NOT NULL,

    -- Routing Information
    requested_model TEXT,
    actual_model TEXT,
    backend TEXT,
    backend_url TEXT,
    upstream_mode TEXT,
    route_id TEXT,
    transformations_applied TEXT, -- JSON serialized, nullable

    -- MCP Information
    mcp_enabled INTEGER NOT NULL DEFAULT 0, -- Boolean (0/1)
    mcp_servers TEXT, -- JSON serialized array
    system_prompt_applied INTEGER NOT NULL DEFAULT 0, -- Boolean (0/1)

    -- Token Information
    input_tokens BIGINT,
    output_tokens BIGINT,
    cached_tokens BIGINT,
    reasoning_tokens BIGINT,

    -- Cost Information
    cost_info TEXT, -- JSON serialized, nullable

    -- Privacy and Audit
    content_hash TEXT,
    privacy_level TEXT NOT NULL,

    FOREIGN KEY (conversation_id) REFERENCES conversations(conversation_id) ON DELETE CASCADE
);

-- Indexes for efficient querying
CREATE INDEX IF NOT EXISTS idx_messages_conversation_id ON messages(conversation_id);
CREATE INDEX IF NOT EXISTS idx_messages_request_id ON messages(request_id);
CREATE INDEX IF NOT EXISTS idx_messages_created_at ON messages(created_at);
CREATE INDEX IF NOT EXISTS idx_messages_backend ON messages(backend);
CREATE INDEX IF NOT EXISTS idx_messages_requested_model ON messages(requested_model);
CREATE INDEX IF NOT EXISTS idx_messages_actual_model ON messages(actual_model);
CREATE INDEX IF NOT EXISTS idx_messages_route_id ON messages(route_id);
CREATE INDEX IF NOT EXISTS idx_messages_mcp_enabled ON messages(mcp_enabled);
CREATE INDEX IF NOT EXISTS idx_messages_upstream_mode ON messages(upstream_mode);

-- Composite indexes for common queries
CREATE INDEX IF NOT EXISTS idx_messages_conv_created ON messages(conversation_id, created_at);
CREATE INDEX IF NOT EXISTS idx_messages_backend_created ON messages(backend, created_at);
