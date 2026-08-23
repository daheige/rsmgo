use crate::error::{Result, RsmgoError};
use crate::memory::MemoryStore;
use crate::providers::{default_registry, ProviderRegistry};
use crate::tools::{ToolDefinition, ToolRegistry};
use crate::types::{ChatRequest, ChatResponse, Message, ToolCall};
use std::path::PathBuf;
use std::sync::Arc;

pub const DEFAULT_SYSTEM_PROMPT: &str = r#"
You are rsmgo, a model-agnostic AI agent assistant. You have access to tools.
When a user asks you to write content to a file, you MUST use the write_file tool
instead of just describing what you would write. Emit tool calls with precise arguments.
Always prefer safe, read-only operations unless the user explicitly asks for changes.
When you save a file with the write_file tool, the tool result contains a Markdown download link. Always preserve that link in your final response so the user can download the file.
"#;

pub struct Agent {
    providers: ProviderRegistry,
    tools: ToolRegistry,
    memory: Arc<MemoryStore>,
    system_prompt: String,
}

impl Agent {
    pub fn new(memory: Arc<MemoryStore>, workspace_dir: impl Into<PathBuf>) -> Self {
        let workspace_dir = workspace_dir.into();
        Self {
            providers: default_registry(),
            tools: ToolRegistry::with_workspace(&workspace_dir),
            memory,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        }
    }

    pub fn with_providers(mut self, providers: ProviderRegistry) -> Self {
        self.providers = providers;
        self
    }

    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    pub fn list_providers(&self) -> Vec<&str> {
        self.providers.list()
    }

    pub fn list_tools(&self) -> Vec<&str> {
        self.tools.list()
    }

    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.definitions()
    }

    pub async fn chat(&self, mut request: ChatRequest) -> Result<ChatResponse> {
        let provider = self.providers.get(&request.provider).ok_or_else(|| {
            RsmgoError::Provider(format!("unknown provider: {}", request.provider))
        })?;

        // Ensure session exists and persist user messages.
        if self.memory.get_session(&request.session_id)?.is_none() {
            let title = request
                .messages
                .iter()
                .find(|m| m.role == "user")
                .map(|m| m.content.chars().take(40).collect::<String>())
                .unwrap_or_else(|| "New chat".to_string());
            self.memory.create_session(
                &request.session_id,
                &title,
                &request.provider,
                &request.model,
            )?;
        }

        // Persist incoming user messages.
        for msg in &request.messages {
            if msg.role == "user" || msg.role == "tool" {
                self.memory.add_message(&request.session_id, msg)?;
            }
        }

        // Build context from memory (which now includes new messages) and system prompt.
        let mut contextual_messages = vec![Message::system(&self.system_prompt)];
        let history = self.memory.get_messages(&request.session_id)?;
        contextual_messages.extend(history);
        request.messages = contextual_messages;

        // Select tools to expose. Only expose tools when the caller explicitly
        // asks for them; otherwise normal chat questions are answered directly
        // without triggering tool calls.
        let tool_defs: Vec<ToolDefinition> = if request.tool_names.is_empty() {
            Vec::new()
        } else {
            self.tool_definitions()
                .into_iter()
                .filter(|t| request.tool_names.contains(&t.name))
                .collect()
        };

        tracing::info!(
            session_id = %request.session_id,
            provider = %request.provider,
            model = %request.model,
            tool_count = tool_defs.len(),
            "chat request"
        );

        let mut response = provider.chat(request.clone(), tool_defs.clone()).await?;

        tracing::info!(
            content_len = response.message.content.len(),
            raw_tool_calls = response.tool_calls.len(),
            "provider response"
        );

        // Some OpenAI-compatible providers (e.g. DeepSeek, Kimi) return tool
        // calls as DSML/XML inside message.content instead of the structured
        // tool_calls field. When tools were requested, try to parse them so the
        // user sees the final answer instead of raw markup.
        if !tool_defs.is_empty() && response.tool_calls.is_empty() {
            let dsml_calls = parse_dsml_tool_calls(&response.message.content);
            if !dsml_calls.is_empty() {
                tracing::info!(count = dsml_calls.len(), "parsed DSML tool calls");
                response.message.content = strip_dsml_tool_calls(&response.message.content);
                response.tool_calls = dsml_calls;
            } else {
                let inline_calls = parse_inline_tool_calls(&response.message.content);
                if !inline_calls.is_empty() {
                    tracing::info!(count = inline_calls.len(), "parsed inline tool calls");
                    response.message.content =
                        strip_inline_tool_calls(&response.message.content);
                    response.tool_calls = inline_calls;
                }
            }
        }

        // Execute tool calls if any.
        if !response.tool_calls.is_empty() {
            tracing::info!(count = response.tool_calls.len(), "executing tool calls");
            let assistant_message = Message::assistant_with_tool_calls(
                response.message.content.clone(),
                response.tool_calls.clone(),
            );
            self.memory
                .add_message(&request.session_id, &assistant_message)?;

            let mut tool_results: Vec<Message> = Vec::new();
            for tc in &response.tool_calls {
                let result = self.tools.execute(&tc.name, tc.arguments.clone());
                let content = match result {
                    Ok(out) => {
                        tracing::info!(tool = %tc.name, "tool executed successfully");
                        out
                    }
                    Err(e) => {
                        tracing::warn!(tool = %tc.name, error = %e, "tool execution failed");
                        format!("Error: {}", e)
                    }
                };
                tool_results.push(Message::tool(content, &tc.id));
            }

            for msg in &tool_results {
                self.memory.add_message(&request.session_id, msg)?;
            }

            // Call provider again with tool results.
            let follow_up_request = ChatRequest {
                session_id: request.session_id.clone(),
                messages: [
                    request.messages.clone(),
                    vec![assistant_message],
                    tool_results,
                ]
                .concat(),
                provider: request.provider.clone(),
                model: request.model.clone(),
                tool_names: request.tool_names.clone(),
                stream: false,
            };

            tracing::info!("calling provider with tool results");
            let final_response = provider.chat(follow_up_request, Vec::new()).await?;
            tracing::info!(
                content_len = final_response.message.content.len(),
                "final provider response"
            );
            self.memory
                .add_message(&request.session_id, &final_response.message)?;
            return Ok(final_response);
        }

        tracing::info!("no tool calls, returning direct response");
        self.memory
            .add_message(&request.session_id, &response.message)?;
        Ok(response)
    }
}

/// Parse tool calls embedded in message content as DSML/XML blocks.
///
/// Example markup produced by some OpenAI-compatible providers:
/// ```text
/// <| | DSML | | tool_calls>
/// <| | DSML | | invoke name="list_directory">
/// <| | DSML | | parameter name="path" string="true">.claude</| | DSML | | parameter>
/// </| | DSML | | invoke>
/// </| | DSML | | tool_calls>
/// ```
fn parse_dsml_tool_calls(content: &str) -> Vec<ToolCall> {
    let Some(start) = content.find("<| | DSML | | tool_calls>") else {
        return Vec::new();
    };
    let Some(end) = content[start..].find("</| | DSML | | tool_calls>") else {
        return Vec::new();
    };
    let block = &content[start..start + end + "</| | DSML | | tool_calls>".len()];

    let mut calls = Vec::new();
    let invoke_open = "<| | DSML | | invoke name=\"";
    let invoke_close = "</| | DSML | | invoke>";
    let mut search_from = 0;
    while let Some(inv_start) = block[search_from..].find(invoke_open) {
        let inv_start_abs = search_from + inv_start;
        let after_name = &block[inv_start_abs + invoke_open.len()..];
        let Some(name_end) = after_name.find("\">") else {
            break;
        };
        let name = &after_name[..name_end];
        let body_start = inv_start_abs + invoke_open.len() + name_end + 2;
        let Some(body_end_rel) = block[body_start..].find(invoke_close) else {
            break;
        };
        let body = &block[body_start..body_start + body_end_rel];

        let mut args = serde_json::Map::new();
        let param_open = "<| | DSML | | parameter name=\"";
        let param_close = "</| | DSML | | parameter>";
        let mut param_from = 0;
        while let Some(p_start) = body[param_from..].find(param_open) {
            let p_start_abs = param_from + p_start;
            let after_pname = &body[p_start_abs + param_open.len()..];
            let Some(pname_end) = after_pname.find("\"") else {
                break;
            };
            let pname = &after_pname[..pname_end];
            let pvalue_start_offset = find_tag_end(after_pname, pname_end);
            let pvalue_start = p_start_abs + param_open.len() + pvalue_start_offset;
            let Some(pvalue_end_rel) = body[pvalue_start..].find(param_close) else {
                break;
            };
            let pvalue = &body[pvalue_start..pvalue_start + pvalue_end_rel];
            args.insert(pname.to_string(), serde_json::Value::String(pvalue.to_string()));
            param_from = pvalue_start + pvalue_end_rel + param_close.len();
        }

        calls.push(ToolCall {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            arguments: serde_json::Value::Object(args),
        });
        search_from = body_start + body_end_rel + invoke_close.len();
    }
    calls
}

/// Find the position right after the closing `>` of a parameter opening tag.
fn find_tag_end(after_pname: &str, name_end: usize) -> usize {
    let mut i = name_end;
    let bytes = after_pname.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'>' {
            return i + 1;
        }
        i += 1;
    }
    name_end
}

/// Remove DSML tool-call blocks from content so they are not rendered to the user.
fn strip_dsml_tool_calls(content: &str) -> String {
    let Some(start) = content.find("<| | DSML | | tool_calls>") else {
        return content.to_string();
    };
    let Some(end) = content[start..].find("</| | DSML | | tool_calls>") else {
        return content.to_string();
    };
    let end_abs = start + end + "</| | DSML | | tool_calls>".len();
    let before = content[..start].trim_end();
    let after = content[end_abs..].trim_start();
    if before.is_empty() {
        after.to_string()
    } else if after.is_empty() {
        before.to_string()
    } else {
        format!("{}\n\n{}", before, after)
    }
}

/// Parse inline tool calls that some providers emit directly inside
/// message.content, e.g. `write_file:0{"file_path": "my.md", ...}`.
fn parse_inline_tool_calls(content: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let mut i = 0;
    while i < content.len() {
        if let Some((name_len, index_len, brace_pos)) = find_inline_call_prefix(&content[i..]) {
            let name_start = i;
            let name_end = name_start + name_len;
            let brace_abs = i + brace_pos;
            if let Some(json_end) = find_matching_brace(content, brace_abs) {
                let raw_json = &content[brace_abs..=json_end];
                // Some models emit literal newlines inside JSON string values
                // (e.g. the content of a write_file call). Escape them so
                // serde_json can parse the block.
                let escaped_json = raw_json
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\t', "\\t");
                if let Ok(arguments) = serde_json::from_str(&escaped_json) {
                    let name = content[name_start..name_end].to_string();
                    let index = content[name_end + 1..name_end + 1 + index_len].to_string();
                    calls.push(ToolCall {
                        id: format!("inline-{}", index),
                        name,
                        arguments,
                    });
                    i = json_end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    calls
}

/// Look for a prefix of the form `tool_name:digits{` and return the length of
/// the tool name, the length of the index, and the position of the opening brace
/// relative to the start of the string.
fn find_inline_call_prefix(s: &str) -> Option<(usize, usize, usize)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    if i >= bytes.len() || !is_name_start(bytes[i]) {
        return None;
    }
    let name_start = i;
    i += 1;
    while i < bytes.len() && is_name_char(bytes[i]) {
        i += 1;
    }
    let name_len = i - name_start;
    if i >= bytes.len() || bytes[i] != b':' {
        return None;
    }
    i += 1;
    let index_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let index_len = i - index_start;
    if index_len == 0 || i >= bytes.len() || bytes[i] != b'{' {
        return None;
    }
    Some((name_len, index_len, i))
}

fn is_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Find the closing brace matching the opening brace at `open_pos`, respecting
/// string literals and nested braces.
fn find_matching_brace(content: &str, open_pos: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    if open_pos >= bytes.len() || bytes[open_pos] != b'{' {
        return None;
    }
    let mut depth = 1;
    let mut in_string = false;
    let mut escape = false;
    for i in (open_pos + 1)..bytes.len() {
        let b = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Remove inline tool-call blocks from content so they are not rendered to the user.
fn strip_inline_tool_calls(content: &str) -> String {
    let mut result = String::new();
    let mut i = 0;
    let mut last_end = 0;
    while i < content.len() {
        if let Some((_, _, brace_pos)) = find_inline_call_prefix(&content[i..]) {
            let brace_abs = i + brace_pos;
            if let Some(json_end) = find_matching_brace(content, brace_abs) {
                result.push_str(&content[last_end..i]);
                last_end = json_end + 1;
                i = last_end;
                continue;
            }
        }
        i += 1;
    }
    result.push_str(&content[last_end..]);
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_inline_write_file_with_newlines() {
        let content = "I will create my.md. write_file:4{\"file_path\":\"my.md\",\"content\":\"# Node.js 简介\n\n## 什么是 Node.js?\n\nNode.js is...\"}";
        let calls = parse_inline_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "write_file");
        assert_eq!(calls[0].arguments["file_path"], "my.md");
        assert_eq!(
            calls[0].arguments["content"].as_str().unwrap(),
            "# Node.js 简介\n\n## 什么是 Node.js?\n\nNode.js is..."
        );
    }

    #[test]
    fn strip_inline_tool_call_removes_block() {
        let content = "I will create my.md. write_file:4{\"file_path\":\"my.md\",\"content\":\"hello\"} Done.";
        let stripped = strip_inline_tool_calls(content);
        assert_eq!(stripped, "I will create my.md.  Done.");
    }
}
