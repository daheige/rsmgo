pub mod builtin;

use crate::error::{Result, RsmgoError};
pub use crate::types::ToolDefinition;
use std::collections::HashMap;

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;
    fn execute(&self, args: serde_json::Value) -> Result<String>;
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

    pub fn execute(&self, name: &str, args: serde_json::Value) -> Result<String> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| RsmgoError::Tool(format!("tool '{}' not found", name)))?;
        tool.execute(args)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(builtin::ReadFileTool));
        registry.register(Box::new(builtin::WriteFileTool));
        registry.register(Box::new(builtin::ExecuteCommandTool));
        registry.register(Box::new(builtin::ListDirectoryTool));
        registry.register(Box::new(builtin::SearchTool));
        registry
    }
}
