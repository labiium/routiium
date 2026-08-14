use crate::mcp_config::{McpConfig, McpServerConfig};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

/// MCP JSON-RPC message
#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcMessage {
    jsonrpc: String,
    id: Option<u64>,
    method: Option<String>,
    params: Option<Value>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    data: Option<Value>,
}

/// MCP tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDefinition {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// MCP client for a single server
struct McpServerClient {
    #[allow(dead_code)] // Keep for potential cleanup operations
    process: Child,
    stdin: tokio::process::ChildStdin,
    stdout_reader: Mutex<BufReader<tokio::process::ChildStdout>>,
    next_id: Mutex<u64>,
}

impl McpServerClient {
    async fn new(server_config: &McpServerConfig) -> Result<Self> {
        let mut command = Command::new(&server_config.command);

        // Add arguments
        if let Some(args) = &server_config.args {
            command.args(args);
        }

        // Set environment variables
        if let Some(env) = &server_config.env {
            for (key, value) in env {
                command.env(key, value);
            }
        }

        // Configure stdio
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::null());

        let mut process = command
            .spawn()
            .context("Failed to spawn MCP server process")?;

        let stdin = process
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to get stdin handle"))?;

        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to get stdout handle"))?;

        let stdout_reader = Mutex::new(BufReader::new(stdout));
        let next_id = Mutex::new(1);

        let mut client = Self {
            process,
            stdin,
            stdout_reader,
            next_id,
        };

        // Initialize the MCP connection
        client.initialize().await?;

        Ok(client)
    }

    async fn initialize(&mut self) -> Result<()> {
        let init_params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "clientInfo": {
                "name": "routiium",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        let response = self.send_request("initialize", init_params).await?;

        if response.error.is_some() {
            return Err(anyhow::anyhow!(
                "Failed to initialize MCP server: {:?}",
                response.error
            ));
        }

        // Send initialized notification
        self.send_notification("notifications/initialized", serde_json::json!({}))
            .await?;

        Ok(())
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Result<JsonRpcMessage> {
        let mut id_guard = self.next_id.lock().await;
        let id = *id_guard;
        *id_guard = id_guard
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("MCP request id exhausted"))?;
        drop(id_guard);

        let message = JsonRpcMessage {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: Some(method.to_string()),
            params: Some(params),
            result: None,
            error: None,
        };

        let message_json = serde_json::to_string(&message)?;
        self.stdin
            .write_all(format!("{message_json}\n").as_bytes())
            .await?;
        self.stdin.flush().await?;

        // Read response
        let mut stdout_guard = self.stdout_reader.lock().await;
        let mut line = String::new();
        stdout_guard.read_line(&mut line).await?;
        drop(stdout_guard);

        let response: JsonRpcMessage = serde_json::from_str(line.trim())?;
        Ok(response)
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let message = JsonRpcMessage {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: Some(method.to_string()),
            params: Some(params),
            result: None,
            error: None,
        };

        let message_json = serde_json::to_string(&message)?;
        self.stdin
            .write_all(format!("{message_json}\n").as_bytes())
            .await?;
        self.stdin.flush().await?;

        Ok(())
    }

    async fn list_tools(&mut self) -> Result<Vec<McpToolDefinition>> {
        let response = self
            .send_request("tools/list", serde_json::json!({}))
            .await?;

        if let Some(error) = response.error {
            return Err(anyhow::anyhow!("MCP error: {}", error.message));
        }

        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("No result in tools/list response"))?;

        let tools_array = result
            .get("tools")
            .ok_or_else(|| anyhow::anyhow!("No tools field in response"))?
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Tools field is not an array"))?;

        let tools: Vec<McpToolDefinition> = tools_array
            .iter()
            .filter_map(|t| serde_json::from_value(t.clone()).ok())
            .collect();

        Ok(tools)
    }

    async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });

        let response = self.send_request("tools/call", params).await?;

        if let Some(error) = response.error {
            return Err(anyhow::anyhow!("MCP tool call error: {}", error.message));
        }

        response
            .result
            .ok_or_else(|| anyhow::anyhow!("No result in tool call response"))
    }
}

/// MCP client manager that handles multiple MCP server connections
pub struct McpClientManager {
    clients: HashMap<String, Mutex<McpServerClient>>,
    #[allow(dead_code)] // Keep for potential server introspection
    config: McpConfig,
}

impl McpClientManager {
    /// Create a new MCP client manager from configuration
    ///
    /// # Errors
    /// Returns an error if client manager initialization fails.
    pub async fn new(config: McpConfig) -> Result<Self> {
        let mut clients = HashMap::new();

        for (server_name, server_config) in &config.mcp_servers {
            info!("Connecting to MCP server: {}", server_name);

            match McpServerClient::new(server_config).await {
                Ok(client) => {
                    clients.insert(server_name.clone(), Mutex::new(client));
                    info!("Successfully connected to MCP server: {}", server_name);
                }
                Err(e) => {
                    error!("Failed to connect to MCP server {}: {}", server_name, e);
                    // Continue with other servers even if one fails
                }
            }
        }

        info!("Connected to {} MCP servers", clients.len());

        Ok(Self { clients, config })
    }

    /// Get all available tools from all connected MCP servers
    ///
    /// # Errors
    /// Returns an error if an internal client operation fails.
    pub async fn list_all_tools(&self) -> Result<Vec<McpTool>> {
        let mut all_tools = Vec::new();

        for (server_name, client_mutex) in &self.clients {
            let mut client = client_mutex.lock().await;
            match client.list_tools().await {
                Ok(tools) => {
                    for tool in tools {
                        all_tools.push(McpTool {
                            server_name: server_name.clone(),
                            name: tool.name,
                            description: tool.description,
                            input_schema: tool.input_schema,
                        });
                    }
                }
                Err(e) => {
                    warn!("Failed to list tools from server {}: {}", server_name, e);
                }
            }
        }

        debug!(
            "Found {} total tools across all MCP servers",
            all_tools.len()
        );
        Ok(all_tools)
    }

    /// Execute a tool on the appropriate MCP server
    ///
    /// # Errors
    /// Returns an error when the server is unavailable or tool execution fails.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value> {
        let client_mutex = self
            .clients
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{server_name}' not found"))?;

        debug!(
            "Calling tool {} on server {} with args: {}",
            tool_name, server_name, arguments
        );

        let mut client = client_mutex.lock().await;
        client.call_tool(tool_name, arguments).await
    }

    /// Get the list of connected server names
    #[must_use]
    pub fn connected_servers(&self) -> Vec<String> {
        self.clients.keys().cloned().collect()
    }
}

/// Represents an MCP tool with its metadata
#[derive(Debug, Clone)]
pub struct McpTool {
    pub server_name: String,
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

impl McpTool {
    /// Convert to `OpenAI` tool definition format
    #[must_use]
    pub fn to_openai_tool(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": format!("{}_{}", self.server_name, self.name),
                "description": self.description.as_deref().unwrap_or("MCP tool"),
                "parameters": self.input_schema
            }
        })
    }

    /// Extract server name and tool name from a combined tool name (e.g., "`filesystem_read_file`")
    #[must_use]
    pub fn parse_combined_name(combined_name: &str) -> Option<(String, String)> {
        combined_name
            .split_once('_')
            .map(|(server_name, tool_name)| (server_name.to_string(), tool_name.to_string()))
    }
}
