use crate::error::{Result, RsmgoError};
use crate::providers::LlmProvider;
use crate::types::{
    ChatRequest, ChatResponse, Message, ModelInfo, ToolCall, ToolDefinition, Usage,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;

#[derive(Clone)]
pub struct AnthropicProvider {
    base_url: String,
    api_key: String,
    default_model: String,
    models: Vec<ModelInfo>,
}

impl AnthropicProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        default_model: impl Into<String>,
        models: Vec<ModelInfo>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            default_model: default_model.into(),
            models,
        }
    }

    pub fn default() -> Self {
        Self::new(
            "https://api.anthropic.com/v1".to_string(),
            std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            "claude-sonnet-4-5-20251001",
            vec![
                ModelInfo {
                    id: "claude-opus-5-20251101".to_string(),
                    provider: "anthropic".to_string(),
                    display_name: "Claude Opus 5".to_string(),
                },
                ModelInfo {
                    id: "claude-sonnet-4-5-20251001".to_string(),
                    provider: "anthropic".to_string(),
                    display_name: "Claude Sonnet 4.5".to_string(),
                },
                ModelInfo {
                    id: "claude-haiku-4-5-20251001".to_string(),
                    provider: "anthropic".to_string(),
                    display_name: "Claude Haiku 4.5".to_string(),
                },
            ],
        )
    }
}

#[derive(Debug, Serialize)]
struct AnthropicChatRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct AnthropicChatResponse {
    content: Vec<AnthropicContentBlock>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn chat(&self, request: ChatRequest, tools: Vec<ToolDefinition>) -> Result<ChatResponse> {
        if self.api_key.is_empty() {
            return Err(RsmgoError::Provider(
                "Anthropic API key not configured".to_string(),
            ));
        }

        let model = if request.model.is_empty() {
            self.default_model.clone()
        } else {
            request.model
        };

        let messages: Vec<AnthropicMessage> = request
            .messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| AnthropicMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let tools_payload: Vec<AnthropicTool> = tools
            .into_iter()
            .map(|t| AnthropicTool {
                name: t.name,
                description: t.description,
                input_schema: t.parameters,
            })
            .collect();

        let payload = AnthropicChatRequest {
            model,
            max_tokens: 4096,
            messages,
            tools: tools_payload,
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECONDS))
            .build()?;

        let response = client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| RsmgoError::Provider(format!("HTTP error: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(RsmgoError::Provider(format!(
                "Anthropic returned {}: {}",
                status, text
            )));
        }

        let data: AnthropicChatResponse = response
            .json()
            .await
            .map_err(|e| RsmgoError::Provider(format!("JSON parse error: {}", e)))?;

        let mut content_parts: Vec<String> = Vec::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        for block in data.content {
            match block {
                AnthropicContentBlock::Text { text } => content_parts.push(text),
                AnthropicContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: input,
                    });
                }
            }
        }

        Ok(ChatResponse {
            session_id: request.session_id,
            message: Message {
                role: "assistant".to_string(),
                content: content_parts.join("\n"),
                tool_call_id: None,
                tool_calls: None,
            },
            tool_calls,
            usage: Usage {
                prompt_tokens: data.usage.input_tokens,
                completion_tokens: data.usage.output_tokens,
                total_tokens: data.usage.input_tokens + data.usage.output_tokens,
            },
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self.models.clone())
    }
}
