use crate::error::{Result, RsmgoError};
use crate::tools::Tool;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Percent-encode a file name so it is safe to use inside a Markdown link href.
fn percent_encode_filename(name: &str) -> String {
    name.bytes()
        .map(|b| match b {
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

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

pub struct WriteFileTool {
    workspace_dir: PathBuf,
}

impl WriteFileTool {
    pub fn new(workspace_dir: impl Into<PathBuf>) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
        }
    }
}

impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file under the workspace outputs directory, creating parent directories if needed."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File name or relative path inside outputs/" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }

    fn execute(&self, args: serde_json::Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .or_else(|| args["file_path"].as_str())
            .ok_or_else(|| RsmgoError::Tool("missing 'path' argument".to_string()))?;
        let content = args["content"].as_str().unwrap_or("");

        // Restrict writes to workspace/outputs so the control plane can serve
        // them through a predictable download endpoint.
        let outputs_dir = self.workspace_dir.join("outputs");

        // Strip a leading slash from absolute paths so models can still pass
        // them, but reject parent-directory references to prevent traversal.
        let rel = Path::new(path);
        let rel = rel.strip_prefix("/").unwrap_or(rel);
        for component in rel.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err(RsmgoError::Tool(format!(
                    "write path '{}' contains '..' which is not allowed",
                    path
                )));
            }
        }

        let target = outputs_dir.join(rel);

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| RsmgoError::Tool(format!("failed to create directory: {}", e)))?;
        }
        fs::write(&target, content)
            .map_err(|e| RsmgoError::Tool(format!("failed to write file: {}", e)))?;

        let relative = target
            .strip_prefix(&self.workspace_dir)
            .unwrap_or(&target)
            .to_string_lossy()
            .to_string();
        let file_name = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        let encoded_name = percent_encode_filename(file_name);
        Ok(format!(
            "File written: {}\nDownload: [下载 {}](/api/v1/files/{})",
            relative, file_name, encoded_name
        ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_workspace(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rsmgo-test-{}-{}",
            label,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_file_tool_rejects_traversal() {
        let workspace = temp_workspace("traversal");
        let tool = WriteFileTool::new(&workspace);
        let args = json!({
            "path": "../secret.txt",
            "content": "should not be written"
        });
        let result = tool.execute(args);
        assert!(result.is_err(), "path with .. should be rejected");
        assert!(!workspace.parent().unwrap().join("secret.txt").exists());
        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn write_file_tool_writes_to_outputs() {
        let workspace = temp_workspace("outputs");
        let tool = WriteFileTool::new(&workspace);
        let args = json!({
            "path": "reports/summary.md",
            "content": "hello"
        });
        let result = tool.execute(args).unwrap();
        assert!(result.contains("outputs/reports/summary.md"));
        assert!(result.contains("[下载 summary.md](/api/v1/files/summary.md)"));
        assert_eq!(
            fs::read_to_string(workspace.join("outputs/reports/summary.md")).unwrap(),
            "hello"
        );
        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn write_file_tool_url_encodes_filename() {
        let workspace = temp_workspace("encode");
        let tool = WriteFileTool::new(&workspace);
        let args = json!({
            "path": "my file.md",
            "content": "hello"
        });
        let result = tool.execute(args).unwrap();
        assert!(result.contains("[下载 my file.md](/api/v1/files/my%20file.md)"));
        let _ = fs::remove_dir_all(&workspace);
    }
}
