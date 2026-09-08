//! Git operations tool. Wraps the `git` CLI with a subcommand allowlist and
//! blocks argument-injection flags, so the model can inspect and modify the
//! repository without gaining arbitrary shell or git-config access.

use crate::error::{Result, RsmgoError};
use crate::tools::{Tool, ToolContext};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

/// Maximum time a single git invocation may run.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// Cap on command output returned to the model, in characters.
const MAX_OUTPUT_CHARS: usize = 10000;

/// Subcommands the model may run. Anything touching remotes (push/pull/fetch/
/// clone), git configuration, submodules, worktrees, or destructive cleanup
/// (`clean`) is deliberately excluded; those are better done by a human.
const ALLOWED_SUBCOMMANDS: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "branch",
    "tag",
    "remote",
    "ls-files",
    "ls-tree",
    "rev-parse",
    "shortlog",
    "blame",
    "grep",
    "describe",
    "commit",
    "add",
    "rm",
    "checkout",
    "switch",
    "restore",
    "merge",
    "rebase",
    "stash",
    "reset",
    "cherry-pick",
    "revert",
    "init",
    "mv",
    "apply",
];

/// Flags that would let arguments smuggle in extra configuration or escape
/// the working-directory sandbox. Rejected anywhere in the argument list.
const BLOCKED_FLAGS: &[&str] = &[
    "-c",
    "--git-dir",
    "--work-tree",
    "--exec-path",
    "--namespace",
    "-C",
    "--bare",
];

fn validate_args(args: &[String]) -> Result<()> {
    let Some(sub) = args.first() else {
        return Err(RsmgoError::Tool(
            "missing git subcommand; args must start with one of: status, diff, log, add, commit, ..."
                .to_string(),
        ));
    };
    if !ALLOWED_SUBCOMMANDS.contains(&sub.as_str()) {
        return Err(RsmgoError::Tool(format!(
            "git subcommand '{}' is not allowed; allowed: {}",
            sub,
            ALLOWED_SUBCOMMANDS.join(", ")
        )));
    }
    for (i, arg) in args.iter().enumerate() {
        if i == 0 {
            continue;
        }
        for blocked in BLOCKED_FLAGS {
            if arg == blocked || arg.starts_with(&format!("{}=", blocked)) {
                return Err(RsmgoError::Tool(format!(
                    "argument '{}' is not allowed ({} would let the call override configuration or escape the working directory)",
                    arg, blocked
                )));
            }
        }
    }
    Ok(())
}

pub struct GitTool;

#[async_trait]
impl Tool for GitTool {
    fn name(&self) -> &str {
        "git"
    }

    fn description(&self) -> &str {
        "Run a git command against a repository: status, diff, log, show, blame, branch, add, commit, checkout, switch, restore, merge, rebase, stash, reset, cherry-pick, revert, tag, init, and more. Remote operations (push/pull/fetch/clone) and config changes are not allowed. Pass the full argument list as an array, e.g. [\"log\", \"--oneline\", \"-5\"]."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Full git argument list; the first element must be a subcommand, e.g. [\"status\", \"--short\"]"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Directory of the repository (default: the workspace root)"
                }
            },
            "required": ["args"]
        })
    }

    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> Result<String> {
        let argv: Vec<String> = args["args"]
            .as_array()
            .ok_or_else(|| RsmgoError::Tool("missing 'args' array".to_string()))?
            .iter()
            .map(|v| {
                v.as_str().map(|s| s.to_string()).ok_or_else(|| {
                    RsmgoError::Tool("'args' must be an array of strings".to_string())
                })
            })
            .collect::<Result<Vec<String>>>()?;
        validate_args(&argv)?;

        let dir: PathBuf = match args["working_dir"].as_str() {
            Some(d) if !d.trim().is_empty() => ctx.resolve(d),
            _ => ctx.workspace.clone().unwrap_or_else(|| PathBuf::from(".")),
        };
        if !dir.is_dir() {
            return Err(RsmgoError::Tool(format!(
                "working_dir '{}' is not a directory",
                dir.display()
            )));
        }

        let mut command = Command::new("git");
        command
            .args(&argv)
            .current_dir(&dir)
            // Never let git block on a credential prompt.
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "/bin/echo");
        let output = tokio::time::timeout(COMMAND_TIMEOUT, command.output())
            .await
            .map_err(|_| RsmgoError::Tool("git command timed out (60s)".to_string()))?
            .map_err(|e| RsmgoError::Tool(format!("failed to run git: {}", e)))?;

        let status = output.status;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut text = String::new();
        if !stdout.trim().is_empty() {
            text.push_str(&truncate(&stdout, MAX_OUTPUT_CHARS));
        }
        if !stderr.trim().is_empty() {
            if !text.is_empty() {
                text.push_str("\n\n[stderr]\n");
            }
            text.push_str(&truncate(&stderr, MAX_OUTPUT_CHARS));
        }
        if text.trim().is_empty() {
            text = "(no output)".to_string();
        }

        if !status.success() {
            return Err(RsmgoError::Tool(format!(
                "git {} failed (exit {}): {}",
                argv[0],
                status.code().unwrap_or(-1),
                text.trim()
            )));
        }
        Ok(text)
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push_str("\n...(truncated)");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command as StdCommand;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Create a throwaway git repository with one committed file.
    fn temp_repo(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rsmgo-git-test-{}-{}", name, nanos));
        fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            StdCommand::new("git")
                .args(args)
                .current_dir(&dir)
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .expect("git must be installed for these tests")
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "rsmgo test"]);
        fs::write(dir.join("hello.txt"), "hello world\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "initial"]);
        dir
    }

    #[test]
    fn validate_args_enforces_allowlist_and_blocks_injection() {
        assert!(validate_args(&["status".to_string()]).is_ok());
        assert!(validate_args(&["log".to_string(), "--oneline".to_string()]).is_ok());

        let err = validate_args(&["push".to_string()]).unwrap_err();
        assert!(err.to_string().contains("not allowed"), "{}", err);

        let err = validate_args(&["-c".to_string(), "status".to_string()]).unwrap_err();
        assert!(err.to_string().contains("not allowed"), "{}", err);

        let err = validate_args(&["log".to_string(), "--git-dir=/tmp/x".to_string()]).unwrap_err();
        assert!(err.to_string().contains("--git-dir"), "{}", err);

        let err =
            validate_args(&["log".to_string(), "-C".to_string(), "/tmp".to_string()]).unwrap_err();
        assert!(err.to_string().contains("-C"), "{}", err);

        assert!(validate_args(&[]).is_err());
    }

    #[tokio::test]
    async fn git_status_and_log_in_temp_repo() {
        let dir = temp_repo("read");
        let tool = GitTool;
        let ctx = ToolContext::default();

        let out = tool
            .execute(
                json!({"args": ["status", "--short"], "working_dir": dir.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            out.contains("nothing to commit") || out.trim() == "(no output)",
            "{}",
            out
        );

        let out = tool
            .execute(
                json!({"args": ["log", "--oneline"], "working_dir": dir.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.contains("initial"), "{}", out);

        // A failing command surfaces stderr and an error.
        let err = tool
            .execute(
                json!({"args": ["log", "--bad-flag-xyz"], "working_dir": dir.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("failed"), "{}", err);
    }

    #[tokio::test]
    async fn git_commit_requires_explicit_call_but_works() {
        let dir = temp_repo("write");
        let tool = GitTool;
        let ctx = ToolContext::default();

        fs::write(dir.join("second.txt"), "second file\n").unwrap();
        tool.execute(
            json!({"args": ["add", "second.txt"], "working_dir": dir.to_str().unwrap()}),
            &ctx,
        )
        .await
        .unwrap();
        let out = tool
            .execute(
                json!({"args": ["commit", "-m", "add second file"], "working_dir": dir.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            out.contains("second file") || out.contains("1 file changed"),
            "{}",
            out
        );

        let out = tool
            .execute(
                json!({"args": ["log", "--oneline"], "working_dir": dir.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.contains("add second file"), "{}", out);
    }
}
