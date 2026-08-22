use crate::error::{Result, RsmgoError};
use crate::memory::MemoryStore;
use crate::providers::{default_registry, ProviderRegistry};
use crate::tools::{ToolDefinition, ToolRegistry};
use crate::types::{ChatRequest, ChatResponse, Message, ToolCall};
use std::sync::Arc;

const DEFAULT_SYSTEM_PROMPT: &str = r#"
You are rsmgo, a model-agnostic AI agent assistant. You can use tools when helpful.
When you need to perform actions on the user's machine, emit tool calls with precise arguments.
Always prefer safe, read-only operations unless the user explicitly asks for changes.
"#;

pub struct Agent {
    providers: ProviderRegistry,
    tools: ToolRegistry,
    memory: Arc<MemoryStore>,
    system_prompt: String,
}

impl Agent {
    pub fn new(memory: Arc<MemoryStore>) -> Self {
        Self {
            providers: default_registry(),
            tools: ToolRegistry::default(),
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

        let mut response = provider.chat(request.clone(), tool_defs.clone()).await?;

        // Some OpenAI-compatible providers (e.g. DeepSeek, Kimi) return tool
        // calls as DSML/XML inside message.content instead of the structured
        // tool_calls field. When tools were requested, try to parse them so the
        // user sees the final answer instead of raw markup.
        if !tool_defs.is_empty() && response.tool_calls.is_empty() {
            let dsml_calls = parse_dsml_tool_calls(&response.message.content);
            if !dsml_calls.is_empty() {
                response.message.content = strip_dsml_tool_calls(&response.message.content);
                response.tool_calls = dsml_calls;
            }
        }

        // Execute tool calls if any.
        if !response.tool_calls.is_empty() {
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
                    Ok(out) => out,
                    Err(e) => format!("Error: {}", e),
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

            let final_response = provider.chat(follow_up_request, Vec::new()).await?;
            self.memory
                .add_message(&request.session_id, &final_response.message)?;
            return Ok(final_response);
        }

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
