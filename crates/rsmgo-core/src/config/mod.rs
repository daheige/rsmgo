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
    pub control_plane: ControlPlaneConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
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
    let mut result = content.to_string();
    // Simple ${VAR} expansion.
    loop {
        let start = match result.find("${") {
            Some(i) => i,
            None => break,
        };
        let end = match result[start + 2..].find('}') {
            Some(i) => start + 2 + i,
            None => break,
        };
        let var_name = &result[start + 2..end];
        let value = std::env::var(var_name).unwrap_or_default();
        result.replace_range(start..=end, &value);
    }
    result
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
