//! Generic HTTP client tool: lets the model call REST APIs, webhooks, and
//! other HTTP endpoints with arbitrary methods, headers, and bodies.

use crate::error::{Result, RsmgoError};
use crate::tools::{Tool, ToolContext};
use async_trait::async_trait;
use reqwest::Method;
use serde_json::json;

/// Default cap on the response body returned to the model, in characters.
const MAX_BODY_CHARS: usize = 8000;

/// HTTP methods the tool accepts. Exotic methods are excluded to keep the
/// attack surface (and the schema) small.
const ALLOWED_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

pub struct HttpRequestTool;

#[async_trait]
impl Tool for HttpRequestTool {
    fn name(&self) -> &str {
        "http_request"
    }

    fn description(&self) -> &str {
        "Make an HTTP request (GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS) to a URL and return the status code, response headers, and body (truncated to 8000 characters). Useful for calling REST APIs and webhooks."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The request URL, including query string" },
                "method": { "type": "string", "enum": ALLOWED_METHODS, "description": "HTTP method (default GET)" },
                "headers": { "type": "object", "description": "Optional request headers as a JSON object, e.g. {\"Authorization\": \"Bearer ...\"}", "additionalProperties": { "type": "string" } },
                "body": { "type": "string", "description": "Optional request body (sent as-is; set Content-Type header yourself)" },
                "timeout_secs": { "type": "integer", "description": "Timeout in seconds (default 30, max 120)" }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> Result<String> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'url' argument".to_string()))?;
        let method = args["method"].as_str().unwrap_or("GET").to_uppercase();
        if !ALLOWED_METHODS.contains(&method.as_str()) {
            return Err(RsmgoError::Tool(format!(
                "unsupported method '{}'; allowed: {}",
                method,
                ALLOWED_METHODS.join(", ")
            )));
        }
        let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30).clamp(1, 120);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| RsmgoError::Tool(format!("failed to build HTTP client: {}", e)))?;

        let mut request = client.request(
            Method::from_bytes(method.as_bytes())
                .map_err(|e| RsmgoError::Tool(format!("invalid method: {}", e)))?,
            url,
        );
        if let Some(headers) = args["headers"].as_object() {
            for (key, value) in headers {
                let value = value.as_str().ok_or_else(|| {
                    RsmgoError::Tool(format!("header '{}' must be a string", key))
                })?;
                request = request.header(key, value);
            }
        }
        if let Some(body) = args["body"].as_str() {
            request = request.body(body.to_string());
        }

        let response = request
            .send()
            .await
            .map_err(|e| RsmgoError::Tool(format!("request failed: {}", e)))?;

        let status = response.status();
        let mut headers_out = Vec::new();
        for (key, value) in response.headers() {
            headers_out.push(format!("{}: {}", key, value.to_str().unwrap_or("?")));
        }
        let body = response
            .text()
            .await
            .map_err(|e| RsmgoError::Tool(format!("failed to read response body: {}", e)))?;
        let mut body: String = body.chars().take(MAX_BODY_CHARS).collect();
        if body.chars().count() >= MAX_BODY_CHARS {
            body.push_str("\n...(truncated)");
        }

        Ok(format!(
            "HTTP {} {}\n{}\n\n{}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            headers_out.join("\n"),
            body.trim_end()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Spawn a one-shot HTTP server on a loopback port that answers every
    /// request with a fixed JSON payload (and echoes the request method/head).
    fn spawn_test_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let mut stream = stream.unwrap();
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let method = request.split(' ').next().unwrap_or("").to_string();
                let body = format!(r#"{{"method":"{}","ok":true}}"#, method);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
        });
        port
    }

    #[tokio::test]
    async fn http_request_get_returns_status_and_body() {
        let port = spawn_test_server();
        let tool = HttpRequestTool;
        let out = tool
            .execute(
                json!({"url": format!("http://127.0.0.1:{}/api", port), "method": "get"}),
                &ToolContext::default(),
            )
            .await
            .unwrap();
        assert!(out.starts_with("HTTP 200"), "unexpected output: {}", out);
        assert!(out.contains(r#""method":"GET""#), "{}", out);
        assert!(out.contains("content-type: application/json"), "{}", out);
    }

    #[tokio::test]
    async fn http_request_post_sends_body() {
        let port = spawn_test_server();
        let tool = HttpRequestTool;
        let out = tool
            .execute(
                json!({
                    "url": format!("http://127.0.0.1:{}/submit", port),
                    "method": "POST",
                    "headers": {"Content-Type": "application/json"},
                    "body": r#"{"hello":"world"}"#
                }),
                &ToolContext::default(),
            )
            .await
            .unwrap();
        assert!(out.contains(r#""method":"POST""#), "{}", out);
    }

    #[tokio::test]
    async fn http_request_rejects_bad_method_and_bad_header() {
        let tool = HttpRequestTool;
        let err = tool
            .execute(
                json!({"url": "http://127.0.0.1:1/", "method": "TRACE"}),
                &ToolContext::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unsupported method"), "{}", err);

        let err = tool
            .execute(
                json!({"url": "http://127.0.0.1:1/", "headers": {"X-Bad": 42}}),
                &ToolContext::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("must be a string"), "{}", err);
    }
}
