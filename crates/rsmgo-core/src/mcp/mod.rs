//! MCP client support: connect to external MCP servers (over stdio child
//! processes or Streamable HTTP) and import their tools into the agent's tool
//! registry, so the model can use them like any built-in tool.
//!
//! Imported tools are registered under `mcp__{server}__{tool}` to avoid
//! collisions with built-in tools and with tools from other MCP servers.

use crate::config::McpServerEntry;
use crate::error::{Result, RsmgoError};
use crate::tools::{Tool, ToolContext};
use async_trait::async_trait;
use rmcp::model::{CallToolRequestParams, ClientInfo, ContentBlock};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::ServiceExt;
use std::sync::Arc;
use tokio::process::Command;

/// A live, initialized MCP client connection. Both supported transports
/// (stdio child process and Streamable HTTP) yield this same type, which
/// keeps the handle object-safe without extra type erasure.
type McpClient = RunningService<RoleClient, ClientInfo>;

/// Build the registry name for a tool imported from an MCP server.
pub fn mcp_tool_name(server: &str, tool: &str) -> String {
    format!("mcp__{}__{}", server, tool)
}

/// Connect to one configured MCP server and run the initialization handshake.
async fn connect(entry: &McpServerEntry) -> Result<McpClient> {
    match entry.transport.as_str() {
        "stdio" => {
            let command = entry.command.as_deref().ok_or_else(|| {
                RsmgoError::Config(format!(
                    "mcp server '{}': command is required for stdio transport",
                    entry.name
                ))
            })?;
            let transport = TokioChildProcess::new(Command::new(command).configure(|cmd| {
                cmd.args(&entry.args).envs(&entry.env);
            }))
            .map_err(|e| RsmgoError::Mcp(format!("mcp server '{}': spawn: {}", entry.name, e)))?;
            ClientInfo::default()
                .serve(transport)
                .await
                .map_err(|e| RsmgoError::Mcp(format!("mcp server '{}': init: {}", entry.name, e)))
        }
        "http" => {
            let url = entry.url.as_deref().ok_or_else(|| {
                RsmgoError::Config(format!(
                    "mcp server '{}': url is required for http transport",
                    entry.name
                ))
            })?;
            let mut config = StreamableHttpClientTransportConfig::with_uri(url);
            for (key, value) in &entry.headers {
                let name = key.parse::<http::HeaderName>().map_err(|e| {
                    RsmgoError::Config(format!(
                        "mcp server '{}': invalid header name '{}': {}",
                        entry.name, key, e
                    ))
                })?;
                let value = value.parse::<http::HeaderValue>().map_err(|e| {
                    RsmgoError::Config(format!(
                        "mcp server '{}': invalid header value for '{}': {}",
                        entry.name, key, e
                    ))
                })?;
                config.custom_headers.insert(name, value);
            }
            let transport = StreamableHttpClientTransport::<reqwest::Client>::from_config(config);
            ClientInfo::default()
                .serve(transport)
                .await
                .map_err(|e| RsmgoError::Mcp(format!("mcp server '{}': init: {}", entry.name, e)))
        }
        other => Err(RsmgoError::Config(format!(
            "mcp server '{}': unknown transport '{}' (expected 'stdio' or 'http')",
            entry.name, other
        ))),
    }
}

/// A tool proxied from an external MCP server. Holds a shared handle to the
/// server's client connection so one connection serves all of its tools.
pub struct McpTool {
    name: String,
    description: String,
    parameters: serde_json::Value,
    tool_name: String,
    client: Arc<McpClient>,
}

impl McpTool {
    fn new(server: &str, tool: &rmcp::model::Tool, client: Arc<McpClient>) -> Self {
        let tool_name = tool.name.to_string();
        Self {
            name: mcp_tool_name(server, &tool_name),
            description: tool
                .description
                .clone()
                .map(|d| d.to_string())
                .unwrap_or_else(|| {
                    format!("Tool '{}' imported from MCP server '{}'", tool_name, server)
                }),
            parameters: serde_json::Value::Object((*tool.input_schema).clone()),
            tool_name,
            client,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> serde_json::Value {
        self.parameters.clone()
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> Result<String> {
        let arguments = match args {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        };
        let params = match arguments {
            Some(args) => CallToolRequestParams::new(self.tool_name.clone()).with_arguments(args),
            None => CallToolRequestParams::new(self.tool_name.clone()),
        };
        let result = self
            .client
            .call_tool(params)
            .await
            .map_err(|e| RsmgoError::Mcp(format!("tool '{}': {}", self.name, e)))?;

        if result.is_error == Some(true) {
            return Err(RsmgoError::Mcp(format!(
                "tool '{}' returned an error: {}",
                self.name,
                flatten_content(&result.content)
            )));
        }
        let text = flatten_content(&result.content);
        Ok(if text.is_empty() {
            "(empty result)".to_string()
        } else {
            text
        })
    }
}

/// Join the text of all content blocks in a tool result, noting non-text
/// blocks so the model knows they existed.
fn flatten_content(content: &[ContentBlock]) -> String {
    let mut text = String::new();
    for block in content {
        match block {
            ContentBlock::Text(t) => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t.text);
            }
            ContentBlock::Image(_) => text.push_str("[image content omitted]"),
            ContentBlock::Audio(_) => text.push_str("[audio content omitted]"),
            ContentBlock::Resource(r) => text.push_str(&r.get_text()),
            ContentBlock::ResourceLink(link) => {
                text.push_str(&format!("[resource: {}]", link.uri));
            }
            _ => text.push_str("[content omitted]"),
        }
    }
    text
}

/// Metadata about one connected MCP server, for logging and introspection.
#[derive(Debug, Clone)]
pub struct McpServerInfo {
    pub name: String,
    pub transport: String,
    pub tools: Vec<String>,
}

/// Holds all live MCP client connections. Must be kept alive for the lifetime
/// of the process: dropping it closes the underlying connections.
pub struct McpManager {
    servers: Vec<McpServerInfo>,
}

impl McpManager {
    /// Connect to every configured MCP server. Servers that fail to connect
    /// (bad config, process spawn failure, handshake timeout, ...) are logged
    /// and skipped so a broken external server never blocks engine startup.
    ///
    /// Returns the manager plus the imported tools, ready to be registered in
    /// the agent's tool registry.
    pub async fn connect_all(entries: &[McpServerEntry]) -> (Self, Vec<Box<dyn Tool>>) {
        let mut servers = Vec::new();
        let mut tools: Vec<Box<dyn Tool>> = Vec::new();

        for entry in entries {
            let transport = if entry.transport.is_empty() {
                "stdio"
            } else {
                entry.transport.as_str()
            };
            let effective = McpServerEntry {
                transport: transport.to_string(),
                ..entry.clone()
            };

            match connect(&effective).await {
                Ok(client) => {
                    let client = Arc::new(client);
                    match client.list_all_tools().await {
                        Ok(remote_tools) => {
                            let names: Vec<String> = remote_tools
                                .iter()
                                .map(|t| mcp_tool_name(&entry.name, &t.name))
                                .collect();
                            tracing::info!(
                                server = %entry.name,
                                transport = %transport,
                                tool_count = remote_tools.len(),
                                "connected to MCP server"
                            );
                            for t in remote_tools {
                                tools.push(Box::new(McpTool::new(&entry.name, &t, client.clone())));
                            }
                            servers.push(McpServerInfo {
                                name: entry.name.clone(),
                                transport: transport.to_string(),
                                tools: names,
                            });
                        }
                        Err(e) => {
                            tracing::warn!(
                                server = %entry.name,
                                error = %e,
                                "connected to MCP server but failed to list tools; skipping"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(server = %entry.name, error = %e, "failed to connect to MCP server; skipping");
                }
            }
        }

        (Self { servers }, tools)
    }

    /// Info about the servers that connected successfully.
    pub fn servers(&self) -> &[McpServerInfo] {
        &self.servers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_tool_name_format() {
        assert_eq!(
            mcp_tool_name("filesystem", "read_file"),
            "mcp__filesystem__read_file"
        );
        assert_eq!(mcp_tool_name("my-server", "tool"), "mcp__my-server__tool");
    }

    #[test]
    fn flattens_text_content() {
        let content = vec![ContentBlock::text("hello"), ContentBlock::text("world")];
        assert_eq!(flatten_content(&content), "hello\nworld");
    }
}
