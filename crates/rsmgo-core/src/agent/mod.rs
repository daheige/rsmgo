use crate::error::{Result, RsmgoError};
use crate::memory::MemoryStore;
use crate::providers::{default_registry, ProviderRegistry};
use crate::tools::{ToolDefinition, ToolRegistry};
use crate::types::{ChatRequest, ChatResponse, Message};
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

        // Select tools to expose.
        let tool_defs: Vec<ToolDefinition> = if request.tool_names.is_empty() {
            self.tool_definitions()
        } else {
            self.tool_definitions()
                .into_iter()
                .filter(|t| request.tool_names.contains(&t.name))
                .collect()
        };

        let response = provider.chat(request.clone(), tool_defs).await?;

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
