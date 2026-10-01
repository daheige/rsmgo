use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Core types used throughout the RSMGO codebase, including messages, tool calls,
/// chat requests/responses, stream events, tool definitions, model info, and agent configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    pub parts: Vec<MultiModalPart>,
}

/// A single multimodal part attached to a message (e.g. an image), used for
/// vision-capable models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiModalPart {
    pub content_type: String,
    pub data: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
            parts: vec![],
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
            parts: vec![],
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
            parts: vec![],
        }
    }

    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            tool_call_id: None,
            tool_calls: Some(tool_calls),
            parts: vec![],
        }
    }

    pub fn tool(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: None,
            parts: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// llm usage information, including token counts for prompt, completion, and total.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Chat request sent to a provider, including session id, messages, provider/model selection,
/// tool names, streaming flag, and optional workspace information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub session_id: String,
    pub messages: Vec<Message>,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub tool_names: Vec<String>,

    /// Whether the model should stream its response (true) or return it all at once (false).
    /// When streaming, the provider will yield `StreamEvent` items until the final response is complete.
    #[serde(default)]
    pub stream: bool,

    /// Optional workspace directory path the agent should operate in.
    #[serde(default)]
    pub workspace: String,
    /// Optional workspace id, used to build download links for generated files.
    #[serde(default)]
    pub workspace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub session_id: String,
    pub message: Message,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub usage: Usage,
}

/// A single event emitted while streaming an agent response. Together they
/// trace the ReAct agent loop in English-labeled stages:
///
/// - `Reasoning`: the model's thinking (Thought) before it acts
/// - `Action`: a tool the agent decided to call, with its arguments
/// - `Observation`: the result the tool returned
/// - `Delta`: a slice of the assistant's final answer text
/// - `Done`: the complete final response (Final Answer), which may include
///   structured tool calls when the model decided to act
///
/// Reasoning/Action/Observation repeat in a loop until the model produces a
/// final answer; `round` counts the loop iteration (1-based).
#[derive(Debug, Clone)]
pub enum StreamEvent {
    Reasoning {
        text: String,
    },
    Delta {
        text: String,
    },
    Action {
        round: usize,
        name: String,
        arguments: serde_json::Value,
    },
    Observation {
        round: usize,
        name: String,
        output: String,
    },
    Done {
        response: ChatResponse,
    },
}

/// A single tool definition, used to register a tool with the agent and
/// provide the model with its name, description, and parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// model information returned by a provider's `list_models` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub display_name: String,
}

/// provider configuration loaded from the agent config file, used to
/// instantiate a provider and register it with the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider: String,
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Agent configuration loaded from the config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub memory_path: Option<String>,
    #[serde(default)]
    pub enabled_tools: Vec<String>,
}

impl AgentConfig {
    pub fn provider_config(&self, provider: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.provider == provider)
    }
}
