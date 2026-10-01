use crate::error::{Result, RsmgoError};
use crate::memory::MemoryStore;
use crate::providers::{default_registry, ProviderRef, ProviderRegistry};
use crate::tools::{Tool, ToolContext, ToolDefinition, ToolRegistry};
use crate::types::{ChatRequest, ChatResponse, Message, StreamEvent, ToolCall};
use futures::{Stream, StreamExt};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

pub const DEFAULT_SYSTEM_PROMPT: &str = r#"
You are rsmgo, a model-agnostic AI agent assistant. You have access to tools.
Always take action with tools when the user's request requires it: write files
with write_file, run commands with execute_command, and inspect the workspace
with list_directory or read_file. Do not merely describe what you would do next —
emit the tool call immediately, and keep working with tools until the task is
complete before writing your final summary.
Emit tool calls with precise arguments.
Always prefer safe, read-only operations unless the user explicitly asks for changes.
Do not run long-lived processes (servers, watchers, REPLs) with execute_command —
they never exit and will time out. Instead write the files and tell the user how
to start them separately.
"#;

/// Safety cap on agent tool-calling rounds, and the message used to force a
/// plain-text wrap-up when that cap is reached mid-task (so the user receives
/// a real answer instead of the model's mid-task narration).
const MAX_TOOL_ROUNDS: usize = 16;
const SUMMARY_INSTRUCTION: &str = "You have reached the maximum number of tool-calling rounds. Stop using tools and summarize what you have accomplished and any remaining steps the user should take.";

/// agent that manages LLM providers, tools, and memory, and orchestrates conversations.
/// The agent handles chat requests,
/// executes tools, and manages the ReAct loop with tool calls and observations.
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

    /// Register an additional tool at runtime (e.g. tools imported from
    /// external MCP servers).
    pub fn register_tool(&mut self, tool: Box<dyn Tool>) {
        self.tools.register(tool);
    }

    /// Execute a single tool by name with the given context, returning its
    /// textual output. Used by the ExecuteTool RPC and the HTTP debug API.
    pub async fn execute_tool(
        &self,
        name: &str,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<String> {
        self.tools.execute(name, args, ctx).await
    }

    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.definitions()
    }

    /// Shared request preamble for `chat` and `chat_stream`: resolve the
    /// provider, ensure the session exists, persist incoming user/tool messages,
    /// build the tool context and system-prompt-augmented message history, and
    /// select the tool definitions to expose.
    fn prepare(
        &self,
        request: &mut ChatRequest,
    ) -> Result<(ProviderRef, ToolContext, Vec<ToolDefinition>)> {
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

        // Build the per-request tool context, carrying the active workspace so
        // file tools (notably write_file) can operate within it.
        let tool_ctx = ToolContext {
            workspace: if request.workspace.is_empty() {
                None
            } else {
                Some(PathBuf::from(&request.workspace))
            },
            workspace_id: if request.workspace_id.is_empty() {
                None
            } else {
                Some(request.workspace_id.clone())
            },
        };

        // Build context from memory (which now includes new messages) and system
        // prompt. Inform the model about the active workspace when one is set so
        // it uses relative paths within it.
        let system_prompt = match &tool_ctx.workspace {
            Some(dir) => format!(
                "{}\n\nThe user's workspace directory is: {}. Prefer relative paths within it when reading or writing files.",
                self.system_prompt,
                dir.display()
            ),
            None => self.system_prompt.clone(),
        };
        let mut contextual_messages = vec![Message::system(&system_prompt)];
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

        Ok((provider, tool_ctx, tool_defs))
    }

    pub async fn chat(&self, mut request: ChatRequest) -> Result<ChatResponse> {
        let (provider, tool_ctx, tool_defs) = self.prepare(&mut request)?;

        let mut response = provider.chat(request.clone(), tool_defs.clone()).await?;

        tracing::info!(
            content_len = response.message.content.len(),
            raw_tool_calls = response.tool_calls.len(),
            "provider response"
        );

        // Some OpenAI-compatible providers (e.g. DeepSeek, Kimi) return tool
        // calls as DSML/XML inside message.content instead of the structured
        // tool_calls field. Recover them so the user sees the final answer
        // instead of raw markup.
        recover_embedded_tool_calls(&mut response, !tool_defs.is_empty());

        // Execute tool calls, feeding results back to the provider, until it
        // stops emitting tool calls, or we hit the round limit. Passing the tool
        // definitions on every round lets DSML-tool-call providers (DeepSeek,
        // Kimi) keep calling tools in follow-up turns instead of leaking raw
        // markup or halting after the first round.
        let mut messages = request.messages.clone();
        for _ in 0..MAX_TOOL_ROUNDS {
            if response.tool_calls.is_empty() {
                break;
            }
            tracing::info!(count = response.tool_calls.len(), "executing tool calls");
            let assistant_message = Message::assistant_with_tool_calls(
                response.message.content.clone(),
                response.tool_calls.clone(),
            );
            self.memory
                .add_message(&request.session_id, &assistant_message)?;
            messages.push(assistant_message);

            let mut tool_results: Vec<Message> = Vec::new();
            for tc in &response.tool_calls {
                let result = self
                    .tools
                    .execute(&tc.name, tc.arguments.clone(), &tool_ctx)
                    .await;
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
            messages.extend(tool_results);

            // Call provider again with tool results.
            let follow_up_request = ChatRequest {
                session_id: request.session_id.clone(),
                messages: messages.clone(),
                provider: request.provider.clone(),
                model: request.model.clone(),
                tool_names: request.tool_names.clone(),
                stream: false,
                workspace: request.workspace.clone(),
                workspace_id: request.workspace_id.clone(),
            };

            tracing::info!("calling provider with tool results");
            response = provider.chat(follow_up_request, tool_defs.clone()).await?;

            // Re-parse embedded tool calls on the follow-up response too, so a
            // second round of tool calls is executed rather than leaked as raw
            // markup.
            recover_embedded_tool_calls(&mut response, !tool_defs.is_empty());
        }

        // If the round cap was hit while the model still wanted to call tools,
        // force a plain-text summary so the caller receives a real answer
        // rather than mid-task narration.
        if !response.tool_calls.is_empty() {
            messages.push(Message::user(SUMMARY_INSTRUCTION));
            let summary_request = ChatRequest {
                session_id: request.session_id.clone(),
                messages: messages.clone(),
                provider: request.provider.clone(),
                model: request.model.clone(),
                tool_names: Vec::new(),
                stream: false,
                workspace: request.workspace.clone(),
                workspace_id: request.workspace_id.clone(),
            };
            response = provider.chat(summary_request, Vec::new()).await?;
            recover_embedded_tool_calls(&mut response, false);
        }

        tracing::info!(
            content_len = response.message.content.len(),
            "final provider response"
        );
        self.memory
            .add_message(&request.session_id, &response.message)?;
        Ok(response)
    }

    pub async fn chat_stream(
        self: Arc<Self>,
        mut request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        let (provider, tool_ctx, tool_defs) = self.prepare(&mut request)?;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamEvent>>(64);
        let this = self.clone();

        tokio::spawn(async move {
            let first = match provider
                .chat_stream(request.clone(), tool_defs.clone())
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            };
            let mut response = match collect_round(first, &tx).await {
                Some(Ok(r)) => r,
                Some(Err(e)) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
                None => return,
            };

            // DSML/inline fallback for providers that return tool calls inside
            // message.content (DeepSeek, Kimi).
            recover_embedded_tool_calls(&mut response, !tool_defs.is_empty());

            let mut messages = request.messages.clone();
            let mut round_no = 0usize;
            loop {
                if response.tool_calls.is_empty() {
                    break;
                }
                if round_no >= MAX_TOOL_ROUNDS {
                    // The model is still trying to call tools; a forced summary
                    // is issued after the loop instead.
                    break;
                }
                round_no += 1;

                // Text the model streamed before this round's tool call is
                // narration, not the answer; surface it as the Thought stage so
                // it never masquerades as the final answer.
                let narration = response.message.content.trim().to_string();
                if !narration.is_empty()
                    && tx
                        .send(Ok(StreamEvent::Reasoning { text: narration }))
                        .await
                        .is_err()
                {
                    return;
                }

                let assistant_message = Message::assistant_with_tool_calls(
                    response.message.content.clone(),
                    response.tool_calls.clone(),
                );
                if let Err(e) = this
                    .memory
                    .add_message(&request.session_id, &assistant_message)
                {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
                messages.push(assistant_message);

                let mut tool_results: Vec<Message> = Vec::new();
                for tc in &response.tool_calls {
                    // Trace the ReAct loop: announce the action before
                    // executing it, then report the observation.
                    if tx
                        .send(Ok(StreamEvent::Action {
                            round: round_no,
                            name: tc.name.clone(),
                            arguments: tc.arguments.clone(),
                        }))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let result = this
                        .tools
                        .execute(&tc.name, tc.arguments.clone(), &tool_ctx)
                        .await;
                    let content = match result {
                        Ok(out) => out,
                        Err(e) => format!("Error: {}", e),
                    };
                    if tx
                        .send(Ok(StreamEvent::Observation {
                            round: round_no,
                            name: tc.name.clone(),
                            output: truncate_for_trace(&content),
                        }))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    tool_results.push(Message::tool(content, &tc.id));
                }
                for msg in &tool_results {
                    if let Err(e) = this.memory.add_message(&request.session_id, msg) {
                        let _ = tx.send(Err(e)).await;
                        return;
                    }
                }
                messages.extend(tool_results);

                let follow_up_request = ChatRequest {
                    session_id: request.session_id.clone(),
                    messages: messages.clone(),
                    provider: request.provider.clone(),
                    model: request.model.clone(),
                    tool_names: request.tool_names.clone(),
                    stream: true,
                    workspace: request.workspace.clone(),
                    workspace_id: request.workspace_id.clone(),
                };

                let next = match provider
                    .chat_stream(follow_up_request, tool_defs.clone())
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        return;
                    }
                };
                response = match collect_round(next, &tx).await {
                    Some(Ok(r)) => r,
                    Some(Err(e)) => {
                        let _ = tx.send(Err(e)).await;
                        return;
                    }
                    None => return,
                };

                // Re-parse embedded tool calls on the follow-up response too.
                recover_embedded_tool_calls(&mut response, !tool_defs.is_empty());
            }

            // The round cap was reached while the model still wanted to call
            // tools. Ask it to wrap up in plain text so the user receives a
            // real answer instead of mid-task narration.
            if !response.tool_calls.is_empty() {
                messages.push(Message::user(SUMMARY_INSTRUCTION));
                let summary_request = ChatRequest {
                    session_id: request.session_id.clone(),
                    messages: messages.clone(),
                    provider: request.provider.clone(),
                    model: request.model.clone(),
                    tool_names: Vec::new(),
                    stream: true,
                    workspace: request.workspace.clone(),
                    workspace_id: request.workspace_id.clone(),
                };
                if let Ok(s) = provider.chat_stream(summary_request, Vec::new()).await {
                    response = match collect_round(s, &tx).await {
                        Some(Ok(r)) => r,
                        Some(Err(e)) => {
                            let _ = tx.send(Err(e)).await;
                            return;
                        }
                        None => return,
                    };
                    // Strip any stray DSML the model emits despite no tools.
                    recover_embedded_tool_calls(&mut response, false);
                }
            }

            // The final round carries the answer (or the forced summary).
            if !response.message.content.trim().is_empty()
                && tx
                    .send(Ok(StreamEvent::Delta {
                        text: response.message.content.clone(),
                    }))
                    .await
                    .is_err()
            {
                return;
            }
            if let Err(e) = this
                .memory
                .add_message(&request.session_id, &response.message)
            {
                let _ = tx.send(Err(e)).await;
                return;
            }
            let _ = tx.send(Ok(StreamEvent::Done { response })).await;
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

/// Cap on tool output embedded in an `Observation` trace event, in
/// characters. The full result is still persisted and sent to the model;
/// only the displayed trace is truncated.
const TRACE_OUTPUT_CHARS: usize = 600;

fn truncate_for_trace(output: &str) -> String {
    let mut out: String = output.chars().take(TRACE_OUTPUT_CHARS).collect();
    if output.chars().count() > TRACE_OUTPUT_CHARS {
        out.push_str("\n...(truncated)");
    }
    out
}

/// Consume one provider stream, forwarding only `Reasoning` (thinking) events
/// downstream and returning the final `Done` response. Content `Delta`s are
/// deliberately NOT forwarded: a provider like DeepSeek streams narration
/// before a tool call, which must not be shown as the answer. The full text
/// lives in the `Done` response's `message.content`; the caller classifies it
/// (narration when a tool call follows, otherwise the final answer) after
/// `recover_embedded_tool_calls` has stripped any embedded markup.
///
/// Returns `None` when the downstream receiver is gone (client disconnected —
/// the caller should stop), or `Some(Err(e))` when the stream errored before a
/// `Done` arrived.
async fn collect_round(
    mut stream: Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>,
    tx: &tokio::sync::mpsc::Sender<Result<StreamEvent>>,
) -> Option<Result<ChatResponse>> {
    while let Some(event) = stream.next().await {
        match event {
            // Agent-level stages are produced by the agent itself, never by a
            // provider stream; ignore defensively.
            Ok(StreamEvent::Action { .. }) | Ok(StreamEvent::Observation { .. }) => {}
            Ok(StreamEvent::Reasoning { text }) => {
                if tx.send(Ok(StreamEvent::Reasoning { text })).await.is_err() {
                    return None;
                }
            }
            Ok(StreamEvent::Delta { .. }) => {}
            Ok(StreamEvent::Done { response }) => return Some(Ok(response)),
            Err(e) => return Some(Err(e)),
        }
    }
    Some(Err(RsmgoError::Provider(
        "stream ended without Done".to_string(),
    )))
}

/// Parse tool calls embedded in message content as DSML/XML blocks.
///
/// DeepSeek and some other OpenAI-compatible providers return tool calls as
/// DSML markup inside `message.content` rather than the structured `tool_calls`
/// field. Each tag is wrapped in the fullwidth vertical bar U+FF5C, doubled on
/// both sides of "DSML". Models are inconsistent about the exact spelling —
/// both `<｜｜DSML｜｜tool_calls>` and `<｜｜DSML｜｜ calls>` (with a space)
/// occur in the wild — so the scanner tolerates optional ASCII whitespace
/// between the marker and the tag name, and after the closing slash.
const DSML_MARKER: &str = "\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c}";

/// A single DSML tag found in model output, e.g. `<｜｜DSML｜｜invoke
/// name="execute_command">` or `</｜｜DSML｜｜ calls>`.
struct DsmlTag<'a> {
    name: &'a str,
    is_close: bool,
    /// Text right after the tag name through the closing `>`; for open tags
    /// this holds the attributes (e.g. ` name="execute_command">`).
    rest: &'a str,
    start: usize,
    end: usize,
}

/// Find the next DSML tag at or after `from`. Returns `None` when no complete
/// tag remains — including when the string ends inside a partial tag, so
/// streaming callers can keep that tail buffered until more bytes arrive.
fn find_dsml_tag(s: &str, from: usize) -> Option<DsmlTag<'_>> {
    let mut search = from;
    while let Some(rel) = s.get(search..)?.find('<') {
        let start = search + rel;
        let after_lt = start + 1;
        let bytes = s.get(after_lt..)?.as_bytes();
        let (is_close, tag_from) = match bytes.first() {
            Some(b'/') => (true, after_lt + 1),
            Some(_) => (false, after_lt),
            None => return None,
        };
        if !s.get(tag_from..)?.starts_with(DSML_MARKER) {
            search = start + 1;
            continue;
        }
        let mut p = tag_from + DSML_MARKER.len();
        // Skip optional whitespace between the marker and the tag name.
        while let Some(c) = s.get(p..)?.chars().next() {
            if c.is_ascii_whitespace() {
                p += c.len_utf8();
            } else {
                break;
            }
        }
        // Read the tag name: ASCII identifier characters only, so multibyte
        // text can never be swallowed into a name.
        let name_start = p;
        while let Some(c) = s.get(p..)?.chars().next() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                p += c.len_utf8();
            } else {
                break;
            }
        }
        if p == name_start {
            // The buffer ended right after the marker (or its whitespace):
            // the tag may continue in the next chunk.
            if p >= s.len() {
                return None;
            }
            search = start + 1;
            continue;
        }
        let Some(gt_rel) = s.get(p..)?.find('>') else {
            // Unterminated tag: the string ends mid-tag. Report "no complete
            // tag" so the caller keeps the partial tail buffered.
            return None;
        };
        let end = p + gt_rel + 1;
        return Some(DsmlTag {
            name: &s[name_start..p],
            is_close,
            rest: &s[p..end],
            start,
            end,
        });
    }
    None
}

/// Extract a `name="value"` attribute from the tail of an open DSML tag.
fn dsml_tag_attr<'a>(rest: &'a str, attr: &str) -> Option<&'a str> {
    let needle = format!("{}=\"", attr);
    let idx = rest.find(&needle)?;
    let value_start = idx + needle.len();
    let value_end = rest[value_start..].find('"')?;
    Some(&rest[value_start..value_start + value_end])
}

/// Whether the enclosing block uses the `calls` tag name (vs `tool_calls`).
fn is_calls_tag(name: &str) -> bool {
    name == "calls" || name == "tool_calls"
}

fn parse_dsml_tool_calls(content: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();

    // Locate the enclosing `<｜｜DSML｜｜tool_calls>` open tag.
    let mut open = None;
    let mut from = 0;
    while let Some(tag) = find_dsml_tag(content, from) {
        if !tag.is_close && is_calls_tag(tag.name) {
            open = Some(tag);
            break;
        }

        from = tag.end;
    }
    let Some(open) = open else {
        return calls;
    };

    from = open.end;
    while let Some(tag) = find_dsml_tag(content, from) {
        if tag.is_close && is_calls_tag(tag.name) {
            break;
        }
        from = tag.end;
        if tag.is_close || tag.name != "invoke" {
            continue;
        }
        let Some(name) = dsml_tag_attr(tag.rest, "name") else {
            continue;
        };

        // The invoke body runs until the matching close tag.
        let mut body_end = None;
        let mut inner_from = tag.end;
        while let Some(t2) = find_dsml_tag(content, inner_from) {
            if t2.is_close && t2.name == "invoke" {
                body_end = Some(t2.start);
                from = t2.end;
                break;
            }
            inner_from = t2.end;
        }
        let Some(body_end) = body_end else {
            break;
        };
        let body = &content[tag.end..body_end];

        let mut args = serde_json::Map::new();
        let mut pfrom = 0;
        while let Some(ptag) = find_dsml_tag(body, pfrom) {
            pfrom = ptag.end;
            if ptag.is_close || ptag.name != "parameter" {
                continue;
            }
            let Some(pname) = dsml_tag_attr(ptag.rest, "name") else {
                continue;
            };
            // The parameter value runs until the matching close tag.
            let mut val_end = None;
            let mut vfrom = ptag.end;
            while let Some(t3) = find_dsml_tag(body, vfrom) {
                if t3.is_close && t3.name == "parameter" {
                    val_end = Some(t3.start);
                    pfrom = t3.end;
                    break;
                }
                vfrom = t3.end;
            }
            let Some(val_end) = val_end else {
                break;
            };
            args.insert(
                pname.to_string(),
                serde_json::Value::String(body[ptag.end..val_end].to_string()),
            );
        }

        calls.push(ToolCall {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            arguments: serde_json::Value::Object(args),
        });
    }
    calls
}

/// Remove DSML tool-call blocks from content so they are not rendered to the
/// user. Multiple blocks are supported; surviving segments are trimmed and
/// joined with a blank line.
fn strip_dsml_tool_calls(content: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    let mut from = 0;
    loop {
        // Find the next open calls tag.
        let mut open = None;
        let mut search = from;
        while let Some(tag) = find_dsml_tag(content, search) {
            if !tag.is_close && is_calls_tag(tag.name) {
                open = Some(tag);
                break;
            }
            search = tag.end;
        }
        let Some(open) = open else {
            segments.push(&content[from..]);
            break;
        };
        segments.push(&content[from..open.start]);
        // Find its close; an unterminated block drops the rest of the text.
        let mut close_end = None;
        let mut search = open.end;
        while let Some(tag) = find_dsml_tag(content, search) {
            if tag.is_close && is_calls_tag(tag.name) {
                close_end = Some(tag.end);
                break;
            }
            search = tag.end;
        }
        match close_end {
            Some(end) => from = end,
            None => break,
        }
    }
    let joined = segments
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    joined
}

/// Recover tool calls that providers embed in `message.content` — DSML/XML
/// blocks (DeepSeek, Kimi) or inline `name:{json}` calls — stripping the
/// markup so raw tags never reach the user.
///
/// When no tools were exposed for the request, the parsed calls are dropped
/// rather than executed (the caller asked for a plain chat), but the content
/// is still cleaned up.
fn recover_embedded_tool_calls(response: &mut ChatResponse, tools_exposed: bool) {
    if !response.tool_calls.is_empty() {
        return;
    }
    let dsml_calls = parse_dsml_tool_calls(&response.message.content);
    let inline_calls = if dsml_calls.is_empty() {
        parse_inline_tool_calls(&response.message.content)
    } else {
        Vec::new()
    };
    if dsml_calls.is_empty() && inline_calls.is_empty() {
        return;
    }
    let mut content = strip_dsml_tool_calls(&response.message.content);
    content = strip_inline_tool_calls(&content);
    response.message.content = content;
    if tools_exposed {
        tracing::info!(
            count = dsml_calls.len() + inline_calls.len(),
            "parsed embedded tool calls"
        );
        response.tool_calls = if !dsml_calls.is_empty() {
            dsml_calls
        } else {
            inline_calls
        };
    } else {
        tracing::info!("dropped embedded tool calls: no tools exposed for this request");
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
        i = next_char_boundary(content, i);
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

/// Advance a byte offset to the start of the next UTF-8 character. The inline
/// tool-call scanners walk `content` one character at a time; a bare `i += 1`
/// would land mid-character on multi-byte (e.g. CJK) text and panic on the next
/// slice.
fn next_char_boundary(content: &str, i: usize) -> usize {
    i + content[i..].chars().next().map_or(1, |c| c.len_utf8())
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
        i = next_char_boundary(content, i);
    }
    result.push_str(&content[last_end..]);
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::LLMProvider;
    use crate::types::{ModelInfo, Usage};
    use async_trait::async_trait;
    use futures::stream;

    /// Minimal provider that returns a canned stream, optionally with one
    /// round of tool calling before the final answer.
    struct MockProvider {
        with_tool_call: bool,
    }

    #[async_trait]
    impl LLMProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }

        async fn chat(
            &self,
            _request: ChatRequest,
            _tools: Vec<ToolDefinition>,
        ) -> Result<ChatResponse> {
            unimplemented!()
        }

        async fn chat_stream(
            &self,
            request: ChatRequest,
            _tools: Vec<ToolDefinition>,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
            let session_id = request.session_id.clone();
            let has_tool_round = self.with_tool_call
                // Tool round only on the first model call (no tool messages yet).
                && !request.messages.iter().any(|m| m.role == "tool");
            let events: Vec<Result<StreamEvent>> = if has_tool_round {
                vec![
                    Ok(StreamEvent::Reasoning {
                        text: "let me check".to_string(),
                    }),
                    Ok(StreamEvent::Done {
                        response: ChatResponse {
                            session_id: session_id.clone(),
                            message: Message::assistant(""),
                            tool_calls: vec![ToolCall {
                                id: "call_1".to_string(),
                                name: "stub".to_string(),
                                arguments: serde_json::json!({"x": 1}),
                            }],
                            usage: Usage::default(),
                        },
                    }),
                ]
            } else {
                vec![
                    Ok(StreamEvent::Delta {
                        text: "done".to_string(),
                    }),
                    Ok(StreamEvent::Done {
                        response: ChatResponse {
                            session_id,
                            message: Message::assistant("done"),
                            tool_calls: vec![],
                            usage: Usage::default(),
                        },
                    }),
                ]
            };
            Ok(Box::pin(stream::iter(events)))
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>> {
            Ok(vec![])
        }
    }

    /// Minimal tool used to observe Action/Observation trace events.
    struct StubTool;

    #[async_trait]
    impl Tool for StubTool {
        fn name(&self) -> &str {
            "stub"
        }
        fn description(&self) -> &str {
            "stub tool"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> Result<String> {
            Ok("stub result".to_string())
        }
    }

    /// Provider whose first round streams narration (in `content`) followed by a
    /// tool call, and whose second round returns the final answer. Mirrors how
    /// DeepSeek-chat emits "thinking out loud" before a DSML tool call.
    struct NarratingProvider;

    #[async_trait]
    impl LLMProvider for NarratingProvider {
        fn name(&self) -> &str {
            "narrating"
        }

        async fn chat(
            &self,
            _request: ChatRequest,
            _tools: Vec<ToolDefinition>,
        ) -> Result<ChatResponse> {
            unimplemented!()
        }

        async fn chat_stream(
            &self,
            request: ChatRequest,
            _tools: Vec<ToolDefinition>,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
            let session_id = request.session_id.clone();
            let has_tool_round = !request.messages.iter().any(|m| m.role == "tool");
            let events: Vec<Result<StreamEvent>> = if has_tool_round {
                vec![
                    Ok(StreamEvent::Delta {
                        text: "The package.json has no type field.\n".to_string(),
                    }),
                    Ok(StreamEvent::Done {
                        response: ChatResponse {
                            session_id: session_id.clone(),
                            message: Message::assistant("The package.json has no type field."),
                            tool_calls: vec![ToolCall {
                                id: "call_1".to_string(),
                                name: "stub".to_string(),
                                arguments: serde_json::json!({}),
                            }],
                            usage: Usage::default(),
                        },
                    }),
                ]
            } else {
                vec![
                    Ok(StreamEvent::Delta {
                        text: "fixed".to_string(),
                    }),
                    Ok(StreamEvent::Done {
                        response: ChatResponse {
                            session_id,
                            message: Message::assistant("fixed"),
                            tool_calls: vec![],
                            usage: Usage::default(),
                        },
                    }),
                ]
            };
            Ok(Box::pin(stream::iter(events)))
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>> {
            Ok(vec![])
        }
    }

    fn test_agent(with_tool_call: bool) -> Arc<Agent> {
        let memory = Arc::new(MemoryStore::open_in_memory().unwrap());
        let mut providers = crate::providers::ProviderRegistry::new();
        providers.register(Arc::new(MockProvider { with_tool_call }));
        let mut tools = ToolRegistry::with_workspace(std::env::temp_dir());
        tools.register(Box::new(StubTool));
        Arc::new(
            Agent::new(memory, std::env::temp_dir())
                .with_providers(providers)
                .with_tools(tools),
        )
    }

    #[tokio::test]
    async fn chat_stream_traces_react_loop_stages() {
        let agent = test_agent(true);
        let request = ChatRequest {
            session_id: "s".to_string(),
            messages: vec![Message::user("hi")],
            provider: "mock".to_string(),
            model: String::new(),
            tool_names: vec![],
            stream: true,
            workspace: String::new(),
            workspace_id: String::new(),
        };
        let mut stream = agent.chat_stream(request).await.unwrap();
        let mut stages: Vec<StreamEvent> = Vec::new();
        while let Some(ev) = stream.next().await {
            stages.push(ev.unwrap());
        }

        // Expected trace: Thought → Action → Observation → Final Answer.
        let kinds: Vec<&str> = stages
            .iter()
            .map(|e| match e {
                StreamEvent::Reasoning { .. } => "thought",
                StreamEvent::Action { .. } => "action",
                StreamEvent::Observation { .. } => "observation",
                StreamEvent::Delta { .. } => "answer",
                StreamEvent::Done { .. } => "done",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["thought", "action", "observation", "answer", "done"],
            "{:?}",
            kinds
        );

        match &stages[1] {
            StreamEvent::Action {
                round,
                name,
                arguments,
            } => {
                assert_eq!(*round, 1);
                assert_eq!(name, "stub");
                assert_eq!(arguments["x"], 1);
            }
            other => panic!("expected Action, got {:?}", other),
        }
        match &stages[2] {
            StreamEvent::Observation {
                round,
                name,
                output,
            } => {
                assert_eq!(*round, 1);
                assert_eq!(name, "stub");
                assert_eq!(output, "stub result");
            }
            other => panic!("expected Observation, got {:?}", other),
        }
    }

    /// Narration the model streams before a tool call must surface as a Thought
    /// (`Reasoning`) event, never as answer text — otherwise the CLI shows
    /// mid-task reasoning ("The package.json has no type field...") as the final
    /// answer.
    #[tokio::test]
    async fn narration_before_tool_call_is_thought_not_answer() {
        let memory = Arc::new(MemoryStore::open_in_memory().unwrap());
        let mut providers = crate::providers::ProviderRegistry::new();
        providers.register(Arc::new(NarratingProvider));
        let mut tools = ToolRegistry::with_workspace(std::env::temp_dir());
        tools.register(Box::new(StubTool));
        let agent = Arc::new(
            Agent::new(memory, std::env::temp_dir())
                .with_providers(providers)
                .with_tools(tools),
        );
        let request = ChatRequest {
            session_id: "s".to_string(),
            messages: vec![Message::user("fix it")],
            provider: "narrating".to_string(),
            model: String::new(),
            tool_names: vec![],
            stream: true,
            workspace: String::new(),
            workspace_id: String::new(),
        };
        let mut stream = agent.chat_stream(request).await.unwrap();
        let mut reasoning = String::new();
        let mut answer = String::new();
        while let Some(ev) = stream.next().await {
            match ev.unwrap() {
                StreamEvent::Reasoning { text } => reasoning.push_str(&text),
                StreamEvent::Delta { text } => answer.push_str(&text),
                _ => {}
            }
        }
        assert!(
            reasoning.contains("package.json has no type field"),
            "narration should be thought, got reasoning: {:?}",
            reasoning
        );
        assert_eq!(answer, "fixed", "answer should be only the final text");
    }

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

    #[test]
    fn parse_inline_tool_calls_skips_multibyte_content() {
        // '（' and the surrounding CJK characters are multi-byte; scanning them
        // must not panic with a non-char-boundary slice.
        let content = "好的（我来创建一个文件）write_file:0{\"path\":\"my.md\",\"content\":\"hi\"}";
        let calls = parse_inline_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "write_file");
    }

    #[test]
    fn strip_inline_tool_calls_skips_multibyte_content() {
        let content =
            "好的（我来创建一个文件）write_file:0{\"path\":\"my.md\",\"content\":\"hi\"} 完成。";
        let stripped = strip_inline_tool_calls(content);
        assert_eq!(stripped, "好的（我来创建一个文件） 完成。");
    }

    #[test]
    fn parse_dsml_tool_calls_handles_fullwidth_bars() {
        // DeepSeek wraps DSML tool calls in the fullwidth vertical bar U+FF5C,
        // doubled on each side of "DSML". The parser must recognise this exact
        // byte format rather than the older ASCII "| |" form.
        let content = concat!(
            "The workspace is empty.\n\n",
            "\u{3c}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c}tool_calls\u{3e}\n",
            "\u{3c}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c}invoke name=\u{22}write_file\u{22}\u{3e}\n",
            "\u{3c}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c}parameter name=\u{22}path\u{22} string=\u{22}true\u{22}\u{3e}mywork/package.json\u{3c}\u{2f}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c}parameter\u{3e}\n",
            "\u{3c}\u{2f}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c}invoke\u{3e}\n",
            "\u{3c}\u{2f}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c}tool_calls\u{3e}"
        );
        let calls = parse_dsml_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "write_file");
        assert_eq!(calls[0].arguments["path"], "mywork/package.json");

        let stripped = strip_dsml_tool_calls(content);
        assert_eq!(stripped, "The workspace is empty.");
    }

    /// DeepSeek sometimes spells DSML tags with a space after the marker
    /// (`<｜｜DSML｜｜ calls>` instead of `<｜｜DSML｜｜tool_calls>`). The parser
    /// must handle both, or the tool call is never executed and raw markup
    /// leaks into the answer.
    #[test]
    fn parse_dsml_tool_calls_tolerates_space_after_marker() {
        let content = concat!(
            "我来查一下天气。\n\n",
            "\u{3c}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c} calls\u{3e}\n",
            "\u{3c}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c} invoke name=\u{22}execute_command\u{22}\u{3e}\n",
            "\u{3c}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c} parameter name=\u{22}command\u{22} string=\u{22}true\u{22}\u{3e}",
            "curl -s \u{27}wttr.in/Shenzhen?lang=zh&format=3\u{27}",
            "\u{3c}\u{2f}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c} parameter\u{3e}\n",
            "\u{3c}\u{2f}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c} invoke\u{3e}\n",
            "\u{3c}\u{2f}\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c} calls\u{3e}"
        );
        let calls = parse_dsml_tool_calls(content);
        assert_eq!(calls.len(), 1, "calls: {:?}", calls);
        assert_eq!(calls[0].name, "execute_command");
        assert_eq!(
            calls[0].arguments["command"],
            "curl -s 'wttr.in/Shenzhen?lang=zh&format=3'"
        );

        let stripped = strip_dsml_tool_calls(content);
        assert_eq!(stripped, "我来查一下天气。");
    }
}
