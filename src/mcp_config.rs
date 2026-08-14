use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// MCP server configuration loaded from mcp.json
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpConfig {
    #[serde(rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpServerConfig {
    pub command: String,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
}

impl McpConfig {
    /// Load MCP configuration from a JSON file
    ///
    /// # Errors
    /// Returns an error when the file cannot be read or parsed as MCP JSON.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref()).with_context(|| {
            format!(
                "Failed to read MCP config file: {}",
                path.as_ref().display()
            )
        })?;

        let config: Self =
            serde_json::from_str(&content).with_context(|| "Failed to parse MCP config JSON")?;

        Ok(config)
    }

    /// Get all server names
    #[must_use]
    pub fn server_names(&self) -> Vec<String> {
        self.mcp_servers.keys().cloned().collect()
    }

    /// Get a specific server config
    #[must_use]
    pub fn get_server(&self, name: &str) -> Option<&McpServerConfig> {
        self.mcp_servers.get(name)
    }
}

impl McpServerConfig {
    /// Get the full command with arguments
    #[must_use]
    pub fn get_command_args(&self) -> Vec<String> {
        let mut cmd_args = vec![self.command.clone()];
        if let Some(args) = &self.args {
            cmd_args.extend(args.iter().cloned());
        }
        cmd_args
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "test fixtures are fixed")]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_config_parsing() {
        let json = r#"
        {
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
                },
                "brave-search": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-brave-search"],
                    "env": {
                        "BRAVE_API_KEY": "your-api-key"
                    }
                }
            }
        }
        "#;

        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.mcp_servers.len(), 2);

        let fs_config = config.get_server("filesystem").unwrap();
        assert_eq!(fs_config.command, "npx");
        assert_eq!(fs_config.args.as_ref().unwrap().len(), 3);

        let brave_config = config.get_server("brave-search").unwrap();
        assert!(brave_config.env.is_some());
        assert_eq!(
            brave_config.env.as_ref().unwrap().get("BRAVE_API_KEY"),
            Some(&"your-api-key".to_string())
        );
    }
}
