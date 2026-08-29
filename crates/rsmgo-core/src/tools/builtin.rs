use crate::error::{Result, RsmgoError};
use crate::tools::{Tool, ToolContext};
use serde_json::json;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Maximum wall-clock time a single shell command may run before it is killed.
/// Prevents long-lived commands (e.g. `node server.js`) from blocking the whole
/// request until the control plane's gRPC deadline.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

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

    fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'path' argument".to_string()))?;
        fs::read_to_string(ctx.resolve(path))
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
        "Write content to a file. In a workspace, the path is relative to the workspace directory; otherwise it is relative to the outputs directory. Parent directories are created automatically."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File name or relative path to write" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }

    fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> Result<String> {
        let path = args["path"]
            .as_str()
            .or_else(|| args["file_path"].as_str())
            .ok_or_else(|| RsmgoError::Tool("missing 'path' argument".to_string()))?;
        let content = args["content"].as_str().unwrap_or("");

        // In workspace mode the file is written directly into the workspace
        // directory (the agent's true working directory). Without a workspace we
        // write under the default outputs directory so the control plane can
        // serve a download link.
        let base_dir = ctx.workspace.as_ref().unwrap_or(&self.workspace_dir);
        let write_dir = ctx.output_dir(&self.workspace_dir);

        // Resolve the model-supplied path. Models often echo the workspace's
        // full absolute path (e.g. /Users/.../mywork/server.js); re-base that
        // onto the workspace so the file lands at the right place instead of
        // nesting the absolute path inside the workspace. Relative paths are
        // joined below as-is.
        let p = Path::new(path);
        let rel: PathBuf = if p.is_absolute() {
            match &ctx.workspace {
                Some(ws) => p
                    .strip_prefix(ws)
                    .map(|r| r.to_path_buf())
                    .unwrap_or_else(|_| p.strip_prefix("/").unwrap_or(p).to_path_buf()),
                None => p.strip_prefix("/").unwrap_or(p).to_path_buf(),
            }
        } else {
            p.to_path_buf()
        };

        // Reject parent-directory references to prevent traversal.
        for component in rel.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err(RsmgoError::Tool(format!(
                    "write path '{}' contains '..' which is not allowed",
                    path
                )));
            }
        }

        let target = write_dir.join(rel);

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| RsmgoError::Tool(format!("failed to create directory: {}", e)))?;
        }
        fs::write(&target, content)
            .map_err(|e| RsmgoError::Tool(format!("failed to write file: {}", e)))?;

        let relative = target
            .strip_prefix(base_dir)
            .unwrap_or(&target)
            .to_string_lossy()
            .to_string();

        // A download link only makes sense when writing into the default
        // outputs directory. In workspace mode the user reads the file directly
        // from the workspace, so we just report the relative path.
        match &ctx.workspace {
            Some(_) => Ok(format!("File written: {}", relative)),
            None => {
                let file_name = target.file_name().and_then(|n| n.to_str()).unwrap_or(path);
                let encoded_name = percent_encode_filename(file_name);
                let download_link = format!("/api/v1/files/{}", encoded_name);
                Ok(format!(
                    "File written: {}\nDownload: [Download {}]({})",
                    relative, file_name, download_link
                ))
            }
        }
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

    fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> Result<String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'command' argument".to_string()))?;
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        if let Some(dir) = args["working_dir"].as_str() {
            cmd.current_dir(dir);
        } else if let Some(dir) = &ctx.workspace {
            cmd.current_dir(dir);
        }

        // Spawn with piped output so we can enforce a timeout. `Command::output`
        // blocks until the child exits, so a long-lived command (`node server.js`)
        // would stall the request indefinitely. We poll `try_wait` instead.
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| RsmgoError::Tool(format!("failed to execute command: {}", e)))?;

        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if started.elapsed() >= COMMAND_TIMEOUT {
                        // Kill the child and report a timeout. We deliberately do
                        // not drain the pipes here: a surviving grandchild could
                        // still hold the write end open and block the read.
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(RsmgoError::Tool(format!(
                            "command timed out after {}s and was killed. The command likely does not exit (a server, watcher, or REPL). Do not retry it — write your output and tell the user how to run it separately.",
                            COMMAND_TIMEOUT.as_secs()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    return Err(RsmgoError::Tool(format!(
                        "failed to wait for command: {}",
                        e
                    )));
                }
            }
        };

        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut out) = child.stdout.take() {
            let _ = out.read_to_string(&mut stdout);
        }
        if let Some(mut err) = child.stderr.take() {
            let _ = err.read_to_string(&mut stderr);
        }
        if status.success() {
            Ok(format!("{}{}", stdout, stderr))
        } else {
            Err(RsmgoError::Tool(format!(
                "command failed ({}): {} {}",
                status, stdout, stderr
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

    fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'path' argument".to_string()))?;
        let entries = fs::read_dir(ctx.resolve(path))
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
        // Return an explicit marker for an empty directory. An empty string is a
        // poor tool result: some providers then stop and merely describe what
        // they plan to do next instead of continuing with the next tool call.
        if lines.is_empty() {
            Ok("(empty directory)".to_string())
        } else {
            Ok(lines.join("\n"))
        }
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

    fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> Result<String> {
        let directory = args["directory"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'directory' argument".to_string()))?;
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'pattern' argument".to_string()))?;
        let output = Command::new("find")
            .arg(ctx.resolve(directory))
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
        let dir = std::env::temp_dir().join(format!("rsmgo-test-{}-{}", label, std::process::id()));
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
        let result = tool.execute(args, &ToolContext::default());
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
        let result = tool.execute(args, &ToolContext::default()).unwrap();
        assert!(result.contains("outputs/reports/summary.md"));
        assert!(result.contains("[Download summary.md](/api/v1/files/summary.md)"));
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
        let result = tool.execute(args, &ToolContext::default()).unwrap();
        assert!(result.contains("[Download my file.md](/api/v1/files/my%20file.md)"));
        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn write_file_tool_writes_into_workspace() {
        let data_dir = temp_workspace("ws-default");
        let ws_dir = temp_workspace("ws-target");
        let tool = WriteFileTool::new(&data_dir);
        let ctx = ToolContext {
            workspace: Some(ws_dir.clone()),
            workspace_id: Some("ws-1".to_string()),
        };
        let args = json!({ "path": "notes/todo.md", "content": "buy milk" });
        let result = tool.execute(args, &ctx).unwrap();
        // In workspace mode the file lands directly in the workspace (not under
        // outputs/) and no download link is emitted.
        assert!(result.contains("File written: notes/todo.md"));
        assert!(!result.contains("Download"));
        assert_eq!(
            fs::read_to_string(ws_dir.join("notes/todo.md")).unwrap(),
            "buy milk"
        );
        assert!(!data_dir.join("outputs/notes/todo.md").exists());
        let _ = fs::remove_dir_all(&data_dir);
        let _ = fs::remove_dir_all(&ws_dir);
    }

    #[test]
    fn write_file_tool_rebases_absolute_path_into_workspace() {
        let data_dir = temp_workspace("ws-abs-default");
        let ws_dir = temp_workspace("ws-abs-target");
        let tool = WriteFileTool::new(&data_dir);
        let ctx = ToolContext {
            workspace: Some(ws_dir.clone()),
            workspace_id: Some("ws-1".to_string()),
        };
        // Models frequently echo the workspace's full absolute path back. The
        // tool must strip the workspace prefix so the file lands at the right
        // place instead of nesting the absolute path inside the workspace.
        let args = json!({
            "path": ws_dir.join("demo/server.js").to_string_lossy(),
            "content": "console.log('hi')"
        });
        let result = tool.execute(args, &ctx).unwrap();
        assert!(result.contains("File written: demo/server.js"));
        assert_eq!(
            fs::read_to_string(ws_dir.join("demo/server.js")).unwrap(),
            "console.log('hi')"
        );
        // The nested absolute path must NOT have been created.
        let nested = ws_dir
            .join(ws_dir.strip_prefix("/").unwrap_or(&ws_dir))
            .join("demo/server.js");
        assert!(!nested.exists());
        let _ = fs::remove_dir_all(&data_dir);
        let _ = fs::remove_dir_all(&ws_dir);
    }
}
