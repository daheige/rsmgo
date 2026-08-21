use crate::error::{Result, RsmgoError};
use crate::tools::Tool;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::process::Command;

pub struct ReadFileTool;

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Absolute or relative path to the file" }
            },
            "required": ["path"]
        })
    }

    fn execute(&self, args: serde_json::Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'path' argument".to_string()))?;
        fs::read_to_string(path)
            .map_err(|e| RsmgoError::Tool(format!("failed to read file: {}", e)))
    }
}

pub struct WriteFileTool;

impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file, creating parent directories if needed."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }

    fn execute(&self, args: serde_json::Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'path' argument".to_string()))?;
        let content = args["content"].as_str().unwrap_or("");
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent)
                .map_err(|e| RsmgoError::Tool(format!("failed to create directory: {}", e)))?;
        }
        fs::write(path, content)
            .map_err(|e| RsmgoError::Tool(format!("failed to write file: {}", e)))?;
        Ok(format!("File written: {}", path))
    }
}

pub struct ExecuteCommandTool;

impl Tool for ExecuteCommandTool {
    fn name(&self) -> &str {
        "execute_command"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return stdout/stderr."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "working_dir": { "type": "string", "description": "Optional working directory" }
            },
            "required": ["command"]
        })
    }

    fn execute(&self, args: serde_json::Value) -> Result<String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'command' argument".to_string()))?;
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        if let Some(dir) = args["working_dir"].as_str() {
            cmd.current_dir(dir);
        }
        let output = cmd
            .output()
            .map_err(|e| RsmgoError::Tool(format!("failed to execute command: {}", e)))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.success() {
            Ok(format!("{}{}", stdout, stderr))
        } else {
            Err(RsmgoError::Tool(format!(
                "command failed ({}): {} {}",
                output.status, stdout, stderr
            )))
        }
    }
}

pub struct ListDirectoryTool;

impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List files and directories at a given path."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        })
    }

    fn execute(&self, args: serde_json::Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'path' argument".to_string()))?;
        let entries = fs::read_dir(path)
            .map_err(|e| RsmgoError::Tool(format!("failed to read directory: {}", e)))?;
        let mut lines = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| RsmgoError::Tool(format!("entry error: {}", e)))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let typ = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                "dir"
            } else {
                "file"
            };
            lines.push(format!("{} {}", typ, name));
        }
        Ok(lines.join("\n"))
    }
}

pub struct SearchTool;

impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Search for files by name pattern under a directory using find."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "directory": { "type": "string", "description": "Directory to search under" },
                "pattern": { "type": "string", "description": "Filename pattern, e.g. '*.rs'" }
            },
            "required": ["directory", "pattern"]
        })
    }

    fn execute(&self, args: serde_json::Value) -> Result<String> {
        let directory = args["directory"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'directory' argument".to_string()))?;
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'pattern' argument".to_string()))?;
        let output = Command::new("find")
            .arg(directory)
            .arg("-name")
            .arg(pattern)
            .arg("-type")
            .arg("f")
            .output()
            .map_err(|e| RsmgoError::Tool(format!("find failed: {}", e)))?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
