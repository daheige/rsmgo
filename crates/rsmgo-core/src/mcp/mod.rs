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
use rmcp::model::{
    CallToolRequestParams, ClientInfo, ContentBlock, GetPromptRequestParams, Prompt,
    ReadResourceRequestParams, Resource, ResourceContents, Role,
};
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

/// Build the registry name for the resource-reader tool of an MCP server.
pub fn mcp_resource_tool_name(server: &str) -> String {
    format!("mcp__{}__read_resource", server)
}

/// Build the registry name for a prompt template imported from an MCP server.
pub fn mcp_prompt_tool_name(server: &str, prompt: &str) -> String {
    format!("mcp__{}__prompt__{}", server, prompt)
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

/// Join the contents of a resources/read result into text the model can use.
fn flatten_resource_contents(contents: &[ResourceContents]) -> String {
    let mut text = String::new();
    for content in contents {
        match content {
            ResourceContents::TextResourceContents { text: t, .. } => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
            ResourceContents::BlobResourceContents { blob, .. } => {
                text.push_str(&format!(
                    "[binary resource content: {} base64 bytes omitted]",
                    blob.len()
                ));
            }
            _ => text.push_str("[unsupported resource content]"),
        }
    }
    text
}

/// A read-only tool that reads resources exposed by an external MCP server.
/// One instance covers all of a server's resources: the tool description
/// lists the available URIs so the model can pick one, and the `uri`
/// argument is validated against that list before the call is forwarded.
pub struct McpResourceTool {
    name: String,
    description: String,
    parameters: serde_json::Value,
    uris: Vec<String>,
    client: Arc<McpClient>,
}

impl McpResourceTool {
    fn new(server: &str, resources: &[Resource], client: Arc<McpClient>) -> Self {
        let mut lines = vec![format!(
            "Read a data resource exposed by MCP server '{}'. Available resources:",
            server
        )];
        let mut properties = serde_json::Map::new();
        let mut uris = Vec::with_capacity(resources.len());
        for r in resources {
            let mut line = format!("- `{}` ({})", r.uri, r.name);
            if let Some(desc) = &r.description {
                line.push_str(&format!(": {}", desc));
            }
            lines.push(line);
            uris.push(r.uri.clone());
        }
        properties.insert(
            "uri".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "URI of the resource to read",
                "enum": uris,
            }),
        );
        Self {
            name: mcp_resource_tool_name(server),
            description: lines.join("\n"),
            parameters: serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": ["uri"],
            }),
            uris,
            client,
        }
    }
}

#[async_trait]
impl Tool for McpResourceTool {
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
        let uri = args.get("uri").and_then(|v| v.as_str()).ok_or_else(|| {
            RsmgoError::Mcp(format!("tool '{}': missing 'uri' argument", self.name))
        })?;
        if !self.uris.iter().any(|u| u == uri) {
            return Err(RsmgoError::Mcp(format!(
                "tool '{}': unknown resource uri '{}'; available: {}",
                self.name,
                uri,
                self.uris.join(", ")
            )));
        }
        let result = self
            .client
            .read_resource_once(ReadResourceRequestParams::new(uri))
            .await
            .map_err(|e| RsmgoError::Mcp(format!("tool '{}': {}", self.name, e)))?;
        match result {
            rmcp::model::ReadResourceResponse::Complete(read) => {
                let text = flatten_resource_contents(&read.contents);
                Ok(if text.is_empty() {
                    "(empty resource)".to_string()
                } else {
                    text
                })
            }
            rmcp::model::ReadResourceResponse::InputRequired(_) => Err(RsmgoError::Mcp(format!(
                "tool '{}': resource '{}' requires interactive input, which is not supported",
                self.name, uri
            ))),
            _ => Err(RsmgoError::Mcp(format!(
                "tool '{}': unexpected response for resource '{}'",
                self.name, uri
            ))),
        }
    }
}

/// A prompt template imported from an external MCP server. Calling it
/// renders the template through the server's prompts/get and returns the
/// resulting messages as text, so the model can adopt external prompt
/// libraries (coding styles, domain instructions, ...) as needed.
pub struct McpPromptTool {
    name: String,
    description: String,
    parameters: serde_json::Value,
    prompt_name: String,
    client: Arc<McpClient>,
}

impl McpPromptTool {
    fn new(server: &str, prompt: &Prompt, client: Arc<McpClient>) -> Self {
        let prompt_name = prompt.name.clone();
        Self {
            name: mcp_prompt_tool_name(server, &prompt_name),
            description: prompt
                .description
                .clone()
                .map(|d| d.to_string())
                .unwrap_or_else(|| {
                    format!(
                        "Prompt template '{}' imported from MCP server '{}'",
                        prompt_name, server
                    )
                }),
            parameters: prompt_parameters(prompt),
            prompt_name,
            client,
        }
    }
}

/// Build a JSON Schema object for a prompt's arguments.
fn prompt_parameters(prompt: &Prompt) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    if let Some(arguments) = &prompt.arguments {
        for arg in arguments {
            let mut prop = serde_json::json!({ "type": "string" });
            if let Some(desc) = &arg.description {
                prop["description"] = desc.to_string().into();
            }
            properties.insert(arg.name.clone(), prop);
            if arg.required.unwrap_or(false) {
                required.push(arg.name.clone());
            }
        }
    }
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

#[async_trait]
impl Tool for McpPromptTool {
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
        let args = match args {
            serde_json::Value::Object(map) => map,
            _ => Default::default(),
        };
        // Prompt arguments are plain strings; reject nested values early so
        // the model gets a clear error instead of a protocol failure.
        let mut arguments = serde_json::Map::new();
        for (key, value) in args {
            match value.as_str() {
                Some(s) => {
                    arguments.insert(key, serde_json::Value::String(s.to_string()));
                }
                None => {
                    return Err(RsmgoError::Mcp(format!(
                        "tool '{}': argument '{}' must be a string",
                        self.name, key
                    )));
                }
            }
        }
        let params = if arguments.is_empty() {
            GetPromptRequestParams::new(self.prompt_name.clone())
        } else {
            GetPromptRequestParams::new(self.prompt_name.clone()).with_arguments(arguments)
        };
        let result = self
            .client
            .get_prompt_once(params)
            .await
            .map_err(|e| RsmgoError::Mcp(format!("tool '{}': {}", self.name, e)))?;
        match result {
            rmcp::model::GetPromptResponse::Complete(rendered) => {
                let mut text = String::new();
                for message in &rendered.messages {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    let role = match message.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                    };
                    text.push_str(&format!("[{}] ", role));
                    text.push_str(&flatten_content(std::slice::from_ref(&message.content)));
                }
                Ok(if text.is_empty() {
                    "(empty prompt)".to_string()
                } else {
                    text
                })
            }
            rmcp::model::GetPromptResponse::InputRequired(_) => Err(RsmgoError::Mcp(format!(
                "tool '{}': prompt requires interactive input, which is not supported",
                self.name
            ))),
            _ => Err(RsmgoError::Mcp(format!(
                "tool '{}': unexpected prompt response",
                self.name
            ))),
        }
    }
}

/// Metadata about one connected MCP server, for logging and introspection.
#[derive(Debug, Clone)]
pub struct McpServerInfo {
    pub name: String,
    pub transport: String,
    pub tools: Vec<String>,
    pub resources: Vec<String>,
    pub prompts: Vec<String>,
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

                            // Import the server's resources (if any) as a
                            // single read-resource tool. Servers without the
                            // resources capability are skipped quietly.
                            let resources = match client.list_all_resources().await {
                                Ok(resources) => {
                                    if !resources.is_empty() {
                                        tracing::info!(
                                            server = %entry.name,
                                            resource_count = resources.len(),
                                            "imported MCP resources"
                                        );
                                        tools.push(Box::new(McpResourceTool::new(
                                            &entry.name,
                                            &resources,
                                            client.clone(),
                                        )));
                                    }
                                    resources.iter().map(|r| r.uri.clone()).collect::<Vec<_>>()
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        server = %entry.name,
                                        error = %e,
                                        "failed to list MCP resources; skipping"
                                    );
                                    Vec::new()
                                }
                            };

                            // Import each prompt template as a tool. Servers
                            // without the prompts capability are skipped.
                            let prompts = match client.list_all_prompts().await {
                                Ok(prompts) => {
                                    for p in &prompts {
                                        tools.push(Box::new(McpPromptTool::new(
                                            &entry.name,
                                            p,
                                            client.clone(),
                                        )));
                                    }
                                    prompts.iter().map(|p| p.name.clone()).collect::<Vec<_>>()
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        server = %entry.name,
                                        error = %e,
                                        "failed to list MCP prompts; skipping"
                                    );
                                    Vec::new()
                                }
                            };

                            servers.push(McpServerInfo {
                                name: entry.name.clone(),
                                transport: transport.to_string(),
                                tools: names,
                                resources,
                                prompts,
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
    fn mcp_resource_and_prompt_tool_name_format() {
        assert_eq!(mcp_resource_tool_name("docs"), "mcp__docs__read_resource");
        assert_eq!(
            mcp_prompt_tool_name("docs", "translate"),
            "mcp__docs__prompt__translate"
        );
    }

    #[test]
    fn prompt_parameters_schema_marks_required() {
        let prompt = Prompt::new(
            "greet",
            Some("Greeting prompt"),
            Some(vec![
                rmcp::model::PromptArgument::new("name")
                    .with_description("Who to greet")
                    .with_required(true),
                rmcp::model::PromptArgument::new("style"),
            ]),
        );
        let schema = prompt_parameters(&prompt);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["name"]["type"], "string");
        assert_eq!(schema["properties"]["name"]["description"], "Who to greet");
        assert_eq!(schema["required"], serde_json::json!(["name"]));
    }

    #[test]
    fn flattens_resource_contents() {
        let contents = vec![
            ResourceContents::text("hello", "memo://greeting"),
            ResourceContents::BlobResourceContents {
                uri: "file:///bin".into(),
                mime_type: None,
                blob: "AAAA".into(),
                meta: None,
            },
        ];
        let text = flatten_resource_contents(&contents);
        assert!(text.starts_with("hello"));
        assert!(text.contains("[binary resource content: 4 base64 bytes omitted]"));
    }

    #[test]
    fn flattens_text_content() {
        let content = vec![ContentBlock::text("hello"), ContentBlock::text("world")];
        assert_eq!(flatten_content(&content), "hello\nworld");
    }
}
