use crate::error::{Result, RsmgoError};
use crate::providers::LlmProvider;
use crate::types::{
    ChatRequest, ChatResponse, Message, ModelInfo, ToolCall, ToolDefinition, Usage,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;

#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    name: String,
    base_url: String,
    api_key: String,
    default_model: String,
    models: Vec<ModelInfo>,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        default_model: impl Into<String>,
        models: Vec<ModelInfo>,
    ) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            default_model: default_model.into(),
            models,
        }
    }

    pub fn default_openai() -> Self {
        Self::new(
            "openai",
            "https://api.openai.com/v1".to_string(),
            std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            "gpt-4o-mini",
            vec![
                ModelInfo {
                    id: "gpt-4o".to_string(),
                    provider: "openai".to_string(),
                    display_name: "GPT-4o".to_string(),
                },
                ModelInfo {
                    id: "gpt-4o-mini".to_string(),
                    provider: "openai".to_string(),
                    display_name: "GPT-4o Mini".to_string(),
                },
            ],
        )
    }

    pub fn default_deepseek() -> Self {
        Self::new(
            "deepseek",
            "https://api.deepseek.com".to_string(),
            std::env::var("DEEPSEEK_API_KEY").unwrap_or_default(),
            "deepseek-chat",
            vec![
                ModelInfo {
                    id: "deepseek-chat".to_string(),
                    provider: "deepseek".to_string(),
                    display_name: "DeepSeek V3".to_string(),
                },
                ModelInfo {
                    id: "deepseek-reasoner".to_string(),
                    provider: "deepseek".to_string(),
                    display_name: "DeepSeek R1".to_string(),
                },
            ],
        )
    }

    pub fn default_qwen() -> Self {
        Self::new(
            "qwen",
            "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            std::env::var("DASHSCOPE_API_KEY").unwrap_or_default(),
            "qwen-max",
            vec![
                ModelInfo {
                    id: "qwen-max".to_string(),
                    provider: "qwen".to_string(),
                    display_name: "Qwen Max".to_string(),
                },
                ModelInfo {
                    id: "qwen-plus".to_string(),
                    provider: "qwen".to_string(),
                    display_name: "Qwen Plus".to_string(),
                },
            ],
        )
    }

    pub fn default_kimi() -> Self {
        Self::new(
            "kimi",
            "https://api.moonshot.cn/v1".to_string(),
            std::env::var("MOONSHOT_API_KEY").unwrap_or_default(),
            "kimi-latest",
            vec![
                ModelInfo {
                    id: "kimi-latest".to_string(),
                    provider: "kimi".to_string(),
                    display_name: "Kimi Latest".to_string(),
                },
                ModelInfo {
                    id: "kimi-k2-0711-preview".to_string(),
                    provider: "kimi".to_string(),
                    display_name: "Kimi K2".to_string(),
                },
            ],
        )
    }
}

#[derive(Debug, Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAiTool>,
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    typ: String,
    function: OpenAiFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: Option<OpenAiResponseMessage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    role: Option<String>,
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    typ: String,
    function: OpenAiToolCallFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize, Default)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, request: ChatRequest, tools: Vec<ToolDefinition>) -> Result<ChatResponse> {
        if self.api_key.is_empty() {
            return Err(RsmgoError::Provider(format!(
                "{} API key not configured",
                self.name
            )));
        }

        let model = if request.model.is_empty() {
            self.default_model.clone()
        } else {
            request.model
        };

        let messages: Vec<OpenAiMessage> = request
            .messages
            .iter()
            .map(|m| OpenAiMessage {
                role: m.role.clone(),
                content: m.content.clone(),
                tool_call_id: m.tool_call_id.clone(),
                tool_calls: m.tool_calls.as_ref().map(|tcs| {
                    tcs.iter()
                        .map(|tc| OpenAiToolCall {
                            id: tc.id.clone(),
                            typ: "function".to_string(),
                            function: OpenAiToolCallFunction {
                                name: tc.name.clone(),
                                arguments: tc.arguments.to_string(),
                            },
                        })
                        .collect()
                }),
            })
            .collect();

        let tools_payload: Vec<OpenAiTool> = tools
            .into_iter()
            .map(|t| OpenAiTool {
                typ: "function".to_string(),
                function: OpenAiFunction {
                    name: t.name,
                    description: t.description,
                    parameters: t.parameters,
                },
            })
            .collect();

        let payload = OpenAiChatRequest {
            model,
            messages,
            tools: tools_payload,
            stream: false,
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECONDS))
            .build()?;

        let response = client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| RsmgoError::Provider(format!("HTTP error: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(RsmgoError::Provider(format!(
                "{} returned {}: {}",
                self.name, status, text
            )));
        }

        let data: OpenAiChatResponse = response
            .json()
            .await
            .map_err(|e| RsmgoError::Provider(format!("JSON parse error: {}", e)))?;

        let choice = data
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| RsmgoError::Provider("empty response from provider".to_string()))?;

        let msg = choice.message.unwrap_or(OpenAiResponseMessage {
            role: Some("assistant".to_string()),
            content: Some("".to_string()),
            tool_calls: None,
        });

        let tool_calls: Vec<ToolCall> = msg
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| {
                let args = serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                ToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    arguments: args,
                }
            })
            .collect();

        let usage = data
            .usage
            .map(|u| Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            })
            .unwrap_or_default();

        Ok(ChatResponse {
            session_id: request.session_id,
            message: Message {
                role: msg.role.unwrap_or_else(|| "assistant".to_string()),
                content: msg.content.unwrap_or_default(),
                tool_call_id: None,
                tool_calls: None,
            },
            tool_calls,
            usage,
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self.models.clone())
    }
}
