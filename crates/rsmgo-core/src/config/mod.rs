use crate::error::{Result, RsmgoError};
use crate::types::{AgentConfig, ProviderConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub app: AppInfo,
    pub engine: EngineConfig,
    pub providers: Vec<ProviderEntry>,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerEntry>,
    #[serde(default)]
    pub control_plane: ControlPlaneConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

fn default_chat_stream() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    pub grpc_addr: String,
    pub http_addr: String,
    pub data_dir: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// When true, also serve a small HTTP/JSON debug API on `http_addr`.
    /// Defaults to false (gRPC-only); opt in for local curl debugging.
    #[serde(default)]
    pub app_http_debug: bool,
    /// When true (default), the control plane and HTTP debug API stream
    /// assistant responses to the client. When false, responses are buffered
    /// and returned in a single payload, which can help with proxies or
    /// clients that do not support Server-Sent Events.
    #[serde(default = "default_chat_stream")]
    pub chat_stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub name: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    #[serde(default)]
    pub enabled: Vec<String>,
}

/// An external MCP server whose tools are imported into the agent's tool
/// registry. `transport` is either `stdio` (spawn a local command and speak
/// MCP over its standard input/output) or `http` (connect to a remote
/// Streamable HTTP MCP endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    pub name: String,
    #[serde(default)]
    pub transport: String,
    /// Command to spawn; required for `stdio` transport.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Endpoint URL; required for `http` transport.
    #[serde(default)]
    pub url: Option<String>,
    /// Extra HTTP headers sent on every request (http transport only).
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControlPlaneConfig {
    #[serde(default = "default_control_addr")]
    pub addr: String,
    #[serde(default = "default_engine_addr")]
    pub engine_addr: String,
}

fn default_control_addr() -> String {
    ":9090".to_string()
}

fn default_engine_addr() -> String {
    "127.0.0.1:50051".to_string()
}

impl AppConfig {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let content = std::fs::read_to_string(&path)
            .map_err(|e| RsmgoError::Config(format!("failed to read {:?}: {}", path, e)))?;
        let expanded = expand_env_vars(&content);
        let mut config: AppConfig = serde_yaml::from_str(&expanded)
            .map_err(|e| RsmgoError::Config(format!("failed to parse {:?}: {}", path, e)))?;
        config.engine.data_dir = expand_tilde(&config.engine.data_dir);
        // Allow container runtimes to override listening addresses without editing
        // the mounted app.yaml.
        if let Ok(addr) = std::env::var("RSMGO_GRPC_ADDR") {
            config.engine.grpc_addr = addr;
        }
        if let Ok(addr) = std::env::var("RSMGO_HTTP_ADDR") {
            config.engine.http_addr = addr;
        }
        Ok(config)
    }

    /// Load configuration from the first discoverable app.yaml.
    ///
    /// Resolution order: `$RSMGO_CONFIG`, the user config dir
    /// (`~/.config/rsmgo/app.yaml`), then `./app.yaml`.
    pub fn load_default() -> Result<Self> {
        let path = find_config_path().ok_or_else(|| {
            RsmgoError::Config(
                "no app.yaml found; set RSMGO_CONFIG or run from the project root".to_string(),
            )
        })?;
        Self::load(path)
    }

    pub fn default_provider_name(&self) -> Option<&str> {
        self.providers.first().map(|p| p.name.as_str())
    }

    pub fn find_provider(&self, name: &str) -> Option<&ProviderEntry> {
        self.providers.iter().find(|p| p.name == name)
    }

    pub fn to_agent_config(&self) -> AgentConfig {
        AgentConfig {
            providers: self
                .providers
                .iter()
                .map(|p| ProviderConfig {
                    provider: p.name.clone(),
                    api_key: p.api_key.clone(),
                    base_url: p.base_url.clone(),
                    default_model: p.default_model.clone(),
                    extra: HashMap::new(),
                })
                .collect(),
            default_provider: self.providers.first().map(|p| p.name.clone()),
            memory_path: Some(self.engine.data_dir.clone()),
            enabled_tools: self.tools.enabled.clone(),
        }
    }
}

fn expand_env_vars(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' {
            if let Some(&'{') = chars.peek() {
                chars.next(); // consume '{'
                let mut var = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    var.push(c);
                }
                if closed && !var.is_empty() {
                    result.push_str(&std::env::var(&var).unwrap_or_default());
                } else {
                    // Not a valid ${VAR} placeholder; keep the literal text.
                    result.push('$');
                    result.push('{');
                    result.push_str(&var);
                }
            } else {
                result.push('$');
            }
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_env(var: &str, value: &str) {
        std::env::set_var(var, value);
    }

    fn remove_env(var: &str) {
        std::env::remove_var(var);
    }

    #[test]
    fn expands_braced_var() {
        set_env("RSMGO_TEST_BRACED", "braced_value");
        let input = "key: \"${RSMGO_TEST_BRACED}\"";
        let output = expand_env_vars(input);
        assert_eq!(output, "key: \"braced_value\"");
        remove_env("RSMGO_TEST_BRACED");
    }

    #[test]
    fn leaves_bare_dollar_alone() {
        let input = "price: $100";
        let output = expand_env_vars(input);
        assert_eq!(output, "price: $100");
    }

    #[test]
    fn missing_var_expands_to_empty() {
        remove_env("RSMGO_TEST_MISSING");
        let input = "key: \"${RSMGO_TEST_MISSING}\"";
        let output = expand_env_vars(input);
        assert_eq!(output, "key: \"\"");
    }

    #[test]
    fn load_config_parses_mcp_servers() {
        let yaml = r#"
app:
  name: test
  version: 0.0.0
engine:
  grpc_addr: "127.0.0.1:50051"
  http_addr: "127.0.0.1:8080"
  data_dir: "./share/rsmgo"
providers: []
mcp_servers:
  - name: filesystem
    transport: stdio
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    env:
      NODE_ENV: production
  - name: remote
    transport: http
    url: "https://example.com/mcp"
    headers:
      Authorization: "Bearer token123"
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.mcp_servers.len(), 2);

        let fs = &config.mcp_servers[0];
        assert_eq!(fs.name, "filesystem");
        assert_eq!(fs.transport, "stdio");
        assert_eq!(fs.command.as_deref(), Some("npx"));
        assert_eq!(
            fs.args,
            ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
        );
        assert_eq!(
            fs.env.get("NODE_ENV").map(String::as_str),
            Some("production")
        );

        let remote = &config.mcp_servers[1];
        assert_eq!(remote.transport, "http");
        assert_eq!(remote.url.as_deref(), Some("https://example.com/mcp"));
        assert_eq!(
            remote.headers.get("Authorization").map(String::as_str),
            Some("Bearer token123")
        );
    }

    #[test]
    fn mcp_servers_defaults_to_empty() {
        let yaml = r#"
app:
  name: test
  version: 0.0.0
engine:
  grpc_addr: "127.0.0.1:50051"
  http_addr: "127.0.0.1:8080"
  data_dir: "./share/rsmgo"
providers: []
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn load_config_expands_provider_api_key() {
        set_env("RSMGO_TEST_API_KEY", "secret_key_123");
        let yaml = r#"
app:
  name: test
  version: 0.0.0
engine:
  grpc_addr: "127.0.0.1:50051"
  http_addr: "127.0.0.1:8080"
  data_dir: "./share/rsmgo"
providers:
  - name: deepseek
    api_key: "${RSMGO_TEST_API_KEY}"
    base_url: "https://api.deepseek.com"
    default_model: "deepseek-chat"
    models:
      - id: "deepseek-chat"
        display_name: "DeepSeek Chat"
tools:
  enabled: []
control_plane:
  addr: ":9090"
  engine_addr: "127.0.0.1:50051"
"#;
        let expanded = expand_env_vars(yaml);
        let config: AppConfig = serde_yaml::from_str(&expanded).unwrap();
        let entry = config.find_provider("deepseek").unwrap();
        assert_eq!(entry.api_key, "secret_key_123");
        remove_env("RSMGO_TEST_API_KEY");
    }
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

pub fn default_config_path() -> PathBuf {
    std::env::var("RSMGO_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("rsmgo")
                .join("app.yaml")
        })
}

pub fn find_config_path() -> Option<PathBuf> {
    let env_path = std::env::var("RSMGO_CONFIG").ok().map(PathBuf::from);
    if env_path.as_ref().map(|p| p.exists()).unwrap_or(false) {
        return env_path;
    }
    let default = default_config_path();
    if default.exists() {
        return Some(default);
    }
    // Look in current working directory.
    let cwd = PathBuf::from("app.yaml");
    if cwd.exists() {
        return Some(cwd);
    }
    None
}
