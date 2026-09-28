//! Headless browser automation tool. Drives a locally installed Chrome via
//! headless_chrome to fetch page text, take screenshots, and evaluate
//! JavaScript. Everything runs inside `spawn_blocking` because the library is
//! synchronous.

use crate::error::{Result, RsmgoError};
use crate::tools::{Tool, ToolContext};
use async_trait::async_trait;
use headless_chrome::browser::LaunchOptions;
use headless_chrome::protocol::cdp::Page;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Maximum time the browser may stay idle before the call is aborted.
const MAX_TIMEOUT_MS: u64 = 60_000;

/// Cap on text/eval output returned to the model, in characters.
const MAX_OUTPUT_CHARS: usize = 12000;

/// Well-known Chrome / Chromium locations, checked in order after $CHROME_PATH.
const CHROME_CANDIDATES: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/snap/bin/chromium",
];

/// Prefix of the per-invocation Chrome profile directory. A custom prefix (not
/// headless_chrome's default) lets us find and reap Chrome processes that were
/// orphaned when the rsmgo process died before closing the browser.
const PROFILE_PREFIX: &str = "rsmgo-chrome-profile";

static PROFILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique profile directory for one browser invocation, e.g.
/// /tmp/rsmgo-chrome-profile-<pid>-<n>.
fn profile_dir() -> PathBuf {
    let n = PROFILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "{}-{}-{}",
        PROFILE_PREFIX,
        std::process::id(),
        n
    ))
}

/// Best-effort kill of a single PID and its descendants (SIGKILL on Unix).
fn kill_pid_tree(pid: u32) {
    #[cfg(unix)]
    {
        let pid_str = pid.to_string();
        // `pkill -KILL -P` targets children recursively on macOS/Linux.
        let _ = std::process::Command::new("pkill")
            .args(["-KILL", "-P", &pid_str])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Reap headless Chrome instances orphaned by a previous rsmgo process that
/// was killed before it could close the browser. Chrome processes launched by
/// headless_chrome are children of the rsmgo process; when rsmgo dies they are
/// reparented to PID 1, which is how we distinguish orphans from browsers
/// actively managed by a running rsmgo. Also removes their stale profile dirs.
fn reap_orphaned_chrome() {
    let _ = reap_orphaned_chrome_inner();
}

#[cfg(unix)]
fn reap_orphaned_chrome_inner() -> std::result::Result<(), Box<dyn std::error::Error>> {
    use std::process::{Command, Stdio};

    let own_pid = std::process::id().to_string();
    let out = Command::new("pgrep")
        .args(["-f", PROFILE_PREFIX])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut killed = Vec::new();
    for line in stdout.lines() {
        let pid = line.trim();
        if pid.is_empty() || pid == own_pid {
            continue;
        }
        // Only reap processes reparented to init/launchd (PPID 1); anything
        // else is owned by a live rsmgo process.
        let ps_out = Command::new("ps")
            .args(["-o", "ppid=", "-p", pid])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()?;
        let ppid = String::from_utf8_lossy(&ps_out.stdout).trim().to_string();
        if ppid == "1" {
            let _ = Command::new("pkill")
                .args(["-KILL", "-P", pid])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            unsafe {
                libc::kill(pid.parse::<i32>().unwrap_or(-1), libc::SIGKILL);
            }
            killed.push(pid.to_string());
        }
    }
    // Remove profile dirs that no longer have a live Chrome process.
    let entries = match std::fs::read_dir(std::env::temp_dir()) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(PROFILE_PREFIX) {
            continue;
        }
        let dir_pid = name
            .trim_start_matches(PROFILE_PREFIX)
            .trim_start_matches('-')
            .split('-')
            .next()
            .unwrap_or("");
        if dir_pid == own_pid || killed.iter().any(|k| k == dir_pid) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn reap_orphaned_chrome_inner() -> std::result::Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

fn percent_encode_filename(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
            out.push(c);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Locate a Chrome binary: explicit parameter first, then $CHROME_PATH, then
/// well-known install locations.
fn find_chrome(browser_path: Option<&str>) -> Result<PathBuf> {
    if let Some(p) = browser_path {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Ok(path);
        }
        return Err(RsmgoError::Tool(format!(
            "browser_path '{}' does not exist or is not a file",
            p
        )));
    }
    if let Ok(env_path) = std::env::var("CHROME_PATH") {
        let path = PathBuf::from(&env_path);
        if path.is_file() {
            return Ok(path);
        }
    }
    for candidate in CHROME_CANDIDATES {
        let path = Path::new(candidate);
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
    }
    Err(RsmgoError::Tool(
        "no Chrome/Chromium binary found; install Chrome, set the CHROME_PATH environment variable, or pass browser_path"
            .to_string(),
    ))
}

struct RunParams {
    action: String,
    url: String,
    selector: Option<String>,
    expression: Option<String>,
    wait_ms: u64,
    width: u32,
    height: u32,
    timeout_ms: u64,
    output: Option<String>,
    chrome: PathBuf,
}

/// The synchronous body of the tool, executed on the blocking thread pool.
fn run_blocking(
    params: RunParams,
    workspace_dir: &Path,
    workspace: Option<&PathBuf>,
) -> Result<String> {
    // A throwaway profile keeps cookies and cache out of the user's browser.
    // The custom prefix identifies our Chrome processes so orphaned ones (left
    // behind when the rsmgo process dies mid-call) can be reaped on next start.
    let profile = profile_dir();
    if let Err(e) = std::fs::create_dir_all(&profile) {
        return Err(RsmgoError::Tool(format!(
            "failed to create browser profile directory: {}",
            e
        )));
    }

    let result = run_with_browser(&params, workspace_dir, workspace, &profile);

    // Always remove the throwaway profile, however the call ended.
    let _ = std::fs::remove_dir_all(&profile);
    result
}

/// Launch Chrome with the given profile and run the requested action. The
/// browser is closed deterministically before returning on every path.
fn run_with_browser(
    params: &RunParams,
    workspace_dir: &Path,
    workspace: Option<&PathBuf>,
    profile: &Path,
) -> Result<String> {
    let mut launch = LaunchOptions::default();
    launch.path = Some(params.chrome.clone());
    launch.window_size = Some((params.width, params.height));
    launch.idle_browser_timeout = Duration::from_millis(params.timeout_ms);
    launch.user_data_dir = Some(profile.to_path_buf());

    let browser = headless_chrome::Browser::new(launch).map_err(|e| {
        RsmgoError::Tool(format!(
            "failed to launch Chrome at '{}': {}",
            params.chrome.display(),
            e
        ))
    })?;
    let browser_pid = browser.get_process_id();

    let result = run_action(&browser, params, workspace_dir, workspace);

    // Drop closes the WebSocket and asks Chrome to exit; kill the process
    // tree as well in case the graceful close did not go through (e.g. the
    // transport already timed out).
    drop(browser);
    if let Some(pid) = browser_pid {
        kill_pid_tree(pid);
    }
    result
}

/// Drive one action; the caller owns browser lifetime and cleanup.
fn run_action(
    browser: &headless_chrome::Browser,
    params: &RunParams,
    workspace_dir: &Path,
    workspace: Option<&PathBuf>,
) -> Result<String> {
    let tab = browser
        .new_tab()
        .map_err(|e| RsmgoError::Tool(format!("failed to open a browser tab: {}", e)))?;

    tab.navigate_to(&params.url)
        .map_err(|e| RsmgoError::Tool(format!("failed to navigate to '{}': {}", params.url, e)))?;
    let _ = tab.wait_until_navigated();
    if params.wait_ms > 0 {
        std::thread::sleep(Duration::from_millis(params.wait_ms));
    }

    match params.action.as_str() {
        "text" => {
            let text = match &params.selector {
                Some(selector) => {
                    let element = tab.wait_for_element(selector).map_err(|e| {
                        RsmgoError::Tool(format!(
                            "selector '{}' did not match any element: {}",
                            selector, e
                        ))
                    })?;
                    element.get_inner_text().map_err(|e| {
                        RsmgoError::Tool(format!("failed to read element text: {}", e))
                    })?
                }
                None => {
                    let result = tab
                        .evaluate("document.body ? document.body.innerText : ''", false)
                        .map_err(|e| {
                            RsmgoError::Tool(format!("failed to extract page text: {}", e))
                        })?;
                    result
                        .value
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        .unwrap_or_default()
                }
            };
            let mut out: String = text.chars().take(MAX_OUTPUT_CHARS).collect();
            if text.chars().count() > MAX_OUTPUT_CHARS {
                out.push_str("\n...(truncated)");
            }
            if out.trim().is_empty() {
                out = "(page has no visible text)".to_string();
            }
            Ok(out)
        }
        "eval" => {
            let expression = params
                .expression
                .clone()
                .ok_or_else(|| RsmgoError::Tool("missing 'expression' argument".to_string()))?;
            let result = tab
                .evaluate(&expression, true)
                .map_err(|e| RsmgoError::Tool(format!("failed to evaluate expression: {}", e)))?;
            let out = match (result.value, result.description) {
                (Some(value), _) => {
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
                }
                (None, Some(desc)) => desc,
                (None, None) => "(undefined)".to_string(),
            };
            let mut out: String = out.chars().take(MAX_OUTPUT_CHARS).collect();
            if out.chars().count() > MAX_OUTPUT_CHARS {
                out.push_str("\n...(truncated)");
            }
            Ok(out)
        }
        "screenshot" => {
            let png = tab
                .capture_screenshot(Page::CaptureScreenshotFormatOption::Png, None, None, true)
                .map_err(|e| RsmgoError::Tool(format!("failed to capture screenshot: {}", e)))?;

            let file_name = match params.output.as_deref() {
                Some(name) if !name.trim().is_empty() => {
                    let name = name.trim();
                    if name.contains('/') || name.contains("..") {
                        return Err(RsmgoError::Tool(
                            "output must be a plain file name, not a path".to_string(),
                        ));
                    }
                    if !name.ends_with(".png") {
                        format!("{}.png", name)
                    } else {
                        name.to_string()
                    }
                }
                _ => {
                    let nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    format!("screenshot-{}.png", nanos)
                }
            };

            // Mirror write_file: inside a workspace the file lands in the
            // workspace directory; without one it goes to the outputs
            // directory so the control plane can serve a download link.
            let write_dir = match workspace {
                Some(dir) => dir.clone(),
                None => workspace_dir.join("outputs"),
            };
            std::fs::create_dir_all(&write_dir).map_err(|e| {
                RsmgoError::Tool(format!("failed to create output directory: {}", e))
            })?;
            let target = write_dir.join(&file_name);
            std::fs::write(&target, &png)
                .map_err(|e| RsmgoError::Tool(format!("failed to write screenshot: {}", e)))?;

            let base = workspace
                .cloned()
                .unwrap_or_else(|| workspace_dir.to_path_buf());
            let relative = target
                .strip_prefix(&base)
                .unwrap_or(&target)
                .to_string_lossy()
                .to_string();
            match workspace {
                Some(_) => Ok(format!(
                    "Screenshot saved: {} ({} bytes)",
                    relative,
                    png.len()
                )),
                None => {
                    let encoded = percent_encode_filename(&file_name);
                    Ok(format!(
                        "Screenshot saved: {}\nDownload: [Download {}](/api/v1/files/{})",
                        relative, file_name, encoded
                    ))
                }
            }
        }
        other => Err(RsmgoError::Tool(format!(
            "unknown action '{}'; expected one of: text, screenshot, eval",
            other
        ))),
    }
}

pub struct BrowserTool {
    workspace_dir: PathBuf,
}

impl BrowserTool {
    pub fn new(workspace_dir: impl Into<PathBuf>) -> Self {
        // Clean up Chrome instances orphaned by a previous rsmgo process that
        // was killed before it could close its browser.
        reap_orphaned_chrome();
        Self {
            workspace_dir: workspace_dir.into(),
        }
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Drive a headless Chrome browser. Actions: 'text' extracts the visible text of the page (or of one CSS selector), 'screenshot' saves a PNG (returned as a download link), 'eval' runs a JavaScript expression and returns its value. Requires a local Chrome/Chromium (auto-detected, or set CHROME_PATH / browser_path)."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["text", "screenshot", "eval"], "description": "What to do on the page" },
                "url": { "type": "string", "description": "Page URL to open" },
                "selector": { "type": "string", "description": "CSS selector (text action); without it the whole page text is returned" },
                "expression": { "type": "string", "description": "JavaScript expression (eval action)" },
                "wait_ms": { "type": "integer", "description": "Extra time to wait after page load, for dynamic content (default 1000, max 10000)" },
                "width": { "type": "integer", "description": "Viewport width in pixels (default 1280)" },
                "height": { "type": "integer", "description": "Viewport height in pixels (default 800)" },
                "timeout_ms": { "type": "integer", "description": "Abort if the browser stays idle this long (default 30000, max 60000)" },
                "output": { "type": "string", "description": "Screenshot file name (default auto-generated .png)" },
                "browser_path": { "type": "string", "description": "Path to a Chrome/Chromium binary, overriding auto-detection" }
            },
            "required": ["action", "url"]
        })
    }

    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> Result<String> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'action' argument".to_string()))?
            .to_string();
        if !matches!(action.as_str(), "text" | "screenshot" | "eval") {
            return Err(RsmgoError::Tool(format!(
                "unknown action '{}'; expected one of: text, screenshot, eval",
                action
            )));
        }
        let url = args["url"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'url' argument".to_string()))?
            .to_string();
        let wait_ms = args["wait_ms"].as_u64().unwrap_or(1000).min(10_000);
        let width = args["width"].as_u64().unwrap_or(1280).clamp(320, 3840) as u32;
        let height = args["height"].as_u64().unwrap_or(800).clamp(320, 2160) as u32;
        let timeout_ms = args["timeout_ms"]
            .as_u64()
            .unwrap_or(30_000)
            .clamp(1000, MAX_TIMEOUT_MS);

        let chrome = find_chrome(args["browser_path"].as_str())?;
        let params = RunParams {
            action,
            url,
            selector: args["selector"].as_str().map(|s| s.to_string()),
            expression: args["expression"].as_str().map(|s| s.to_string()),
            wait_ms,
            width,
            height,
            timeout_ms,
            output: args["output"].as_str().map(|s| s.to_string()),
            chrome,
        };

        let workspace_dir = self.workspace_dir.clone();
        let workspace = ctx.workspace.clone();
        tokio::task::spawn_blocking(move || {
            run_blocking(params, &workspace_dir, workspace.as_ref())
        })
        .await
        .map_err(|e| RsmgoError::Tool(format!("browser task failed to complete: {}", e)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Launch Chrome and hit a data: URL. Returns an error string when no
    /// Chrome is available so the test can skip instead of failing.
    fn chrome_or_skip() -> std::result::Result<PathBuf, String> {
        match find_chrome(None) {
            Ok(p) => Ok(p),
            Err(e) => Err(format!("skipping: {}", e)),
        }
    }

    fn data_url(html: &str) -> String {
        let escaped: String = html
            .chars()
            .map(|c| match c {
                '#' => "%23".to_string(),
                '%' => "%25".to_string(),
                c => c.to_string(),
            })
            .collect();
        format!("data:text/html,{}", escaped)
    }

    #[tokio::test]
    async fn browser_text_screenshot_and_eval() {
        let chrome = match chrome_or_skip() {
            Ok(c) => c,
            Err(skip) => {
                eprintln!("{}", skip);
                return;
            }
        };
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rsmgo-browser-test-{}", nanos));
        fs::create_dir_all(&dir).unwrap();
        let tool = BrowserTool::new(dir.join("work"));
        let ctx = ToolContext::default();
        let url =
            data_url("<html><body><h1 id='title'>Hello rsmgo</h1><p>body text</p></body></html>");

        // Whole-page and selector-scoped text.
        let out = tool
            .execute(
                json!({"action": "text", "url": url, "wait_ms": 0, "browser_path": chrome.to_str().unwrap()}),
                &ctx,
            )
            .await
            .expect("text action");
        assert!(out.contains("Hello rsmgo"), "{}", out);

        let out = tool
            .execute(
                json!({"action": "text", "url": url, "selector": "#title", "wait_ms": 0, "browser_path": chrome.to_str().unwrap()}),
                &ctx,
            )
            .await
            .expect("selector text");
        assert!(out.contains("Hello rsmgo"), "{}", out);
        assert!(!out.contains("body text"), "{}", out);

        // JavaScript evaluation (awaits promises).
        let out = tool
            .execute(
                json!({"action": "eval", "url": url, "expression": "Promise.resolve(6 * 7)", "wait_ms": 0, "browser_path": chrome.to_str().unwrap()}),
                &ctx,
            )
            .await
            .expect("eval action");
        assert!(out.contains("42"), "{}", out);

        // Screenshot lands in outputs/ with a download link (no workspace).
        let out = tool
            .execute(
                json!({"action": "screenshot", "url": url, "output": "page", "wait_ms": 0, "browser_path": chrome.to_str().unwrap()}),
                &ctx,
            )
            .await
            .expect("screenshot action");
        assert!(
            out.contains("Screenshot saved: outputs/page.png"),
            "{}",
            out
        );
        assert!(out.contains("/api/v1/files/page.png"), "{}", out);
        let png = fs::read(dir.join("work/outputs/page.png")).unwrap();
        assert_eq!(&png[..4], &[0x89, 0x50, 0x4E, 0x47], "not a PNG");
    }

    #[test]
    fn find_chrome_prefers_explicit_path_and_rejects_bad_ones() {
        let err = find_chrome(Some("/no/such/chrome")).unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{}", err);

        if let Ok(env_path) = std::env::var("CHROME_PATH") {
            if Path::new(&env_path).is_file() {
                assert_eq!(find_chrome(None).unwrap(), PathBuf::from(env_path));
                return;
            }
        }
        // On this dev machine Chrome is installed at the macOS default path.
        let found = find_chrome(None).unwrap();
        assert!(found.ends_with("Google Chrome") || found.ends_with("Chromium"));
    }
}
