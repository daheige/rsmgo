use crate::error::{Result, RsmgoError};
pub use crate::types::ToolDefinition;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub mod builtin;
pub mod web;

/// Per-request context handed to tool executions. Carries the active workspace
/// (directory path and id) so file tools can operate within it.
#[derive(Debug, Clone, Default)]
pub struct ToolContext {
    pub workspace: Option<PathBuf>,
    pub workspace_id: Option<String>,
}

impl ToolContext {
    /// The directory file output tools should write into. When a workspace is
    /// selected, files are written directly into that directory (the agent's
    /// true working directory). Without a workspace, files fall back to the
    /// default `outputs` directory under `default_dir` so the control plane can
    /// serve a download link.
    pub fn output_dir(&self, default_dir: &PathBuf) -> PathBuf {
        match &self.workspace {
            Some(dir) => dir.clone(),
            None => default_dir.join("outputs"),
        }
    }

    /// Resolve a user-supplied path against the active workspace. Absolute paths
    /// are used as-is; relative paths are joined onto the workspace directory
    /// when one is set, and otherwise kept relative to the process working
    /// directory.
    pub fn resolve(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(dir) = &self.workspace {
            dir.join(p)
        } else {
            p.to_path_buf()
        }
    }
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;
    fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> Result<String>;
}

impl ToolDefinition {
    pub fn from_tool(tool: &dyn Tool) -> Self {
        Self {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            parameters: tool.parameters(),
        }
    }
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&Box<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn list(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|t| ToolDefinition::from_tool(t.as_ref()))
            .collect()
    }

    pub fn execute(
        &self,
        name: &str,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<String> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| RsmgoError::Tool(format!("tool '{}' not found", name)))?;
        tool.execute(args, ctx)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::with_workspace(".")
    }
}

impl ToolRegistry {
    /// Create a registry with the built-in tools, using `workspace_dir` as the
    /// base directory for file output tools.
    pub fn with_workspace(workspace_dir: impl Into<PathBuf>) -> Self {
        let workspace_dir = workspace_dir.into();
        let mut registry = Self::new();
        registry.register(Box::new(builtin::ReadFileTool));
        registry.register(Box::new(builtin::WriteFileTool::new(workspace_dir)));
        registry.register(Box::new(builtin::ExecuteCommandTool));
        registry.register(Box::new(builtin::ListDirectoryTool));
        registry.register(Box::new(builtin::SearchTool));
        registry.register(Box::new(web::WebSearchTool));
        registry.register(Box::new(web::FetchUrlTool));
        registry
    }
}
