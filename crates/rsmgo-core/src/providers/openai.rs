use crate::error::{Result, RsmgoError};
use crate::providers::LlmProvider;
use crate::types::{
    ChatRequest, ChatResponse, Message, ModelInfo, StreamEvent, ToolCall, ToolDefinition, Usage,
};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::pin::Pin;

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
// Streaming responses can legitimately run longer than a single buffered call
// (token-by-token delivery), so give them a more generous total timeout.
const STREAM_TIMEOUT_SECONDS: u64 = 300;

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
            "moonshot-v1-8k",
            vec![
                ModelInfo {
                    id: "moonshot-v1-8k".to_string(),
                    provider: "kimi".to_string(),
                    display_name: "Moonshot V1 8K".to_string(),
                },
                ModelInfo {
                    id: "moonshot-v1-32k".to_string(),
                    provider: "kimi".to_string(),
                    display_name: "Moonshot V1 32K".to_string(),
                },
                ModelInfo {
                    id: "moonshot-v1-128k".to_string(),
                    provider: "kimi".to_string(),
                    display_name: "Moonshot V1 128K".to_string(),
                },
                ModelInfo {
                    id: "moonshot-v1-8k-vision-preview".to_string(),
                    provider: "kimi".to_string(),
                    display_name: "Moonshot V1 8K Vision".to_string(),
                },
            ],
        )
    }

    pub fn default_gemini() -> Self {
        Self::new(
            "gemini",
            "https://generativelanguage.googleapis.com/v1beta/openai".to_string(),
            std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            "gemini-2.5-flash",
            vec![
                ModelInfo {
                    id: "gemini-2.5-flash".to_string(),
                    provider: "gemini".to_string(),
                    display_name: "Gemini 2.5 Flash".to_string(),
                },
                ModelInfo {
                    id: "gemini-2.5-pro".to_string(),
                    provider: "gemini".to_string(),
                    display_name: "Gemini 2.5 Pro".to_string(),
                },
            ],
        )
    }

    /// Build the `/chat/completions` request body, shared by the buffered and
    /// streaming paths. Only the `stream` flag differs between them.
    fn build_payload(
        &self,
        request: &ChatRequest,
        tools: Vec<ToolDefinition>,
        stream: bool,
    ) -> OpenAiChatRequest {
        let model = if request.model.is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };

        let include_images = is_vision_model(&model);

        let messages: Vec<OpenAiMessage> = request
            .messages
            .iter()
            .map(|m| OpenAiMessage {
                role: m.role.clone(),
                content: openai_content(m, include_images),
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

        OpenAiChatRequest {
            model,
            messages,
            tools: tools_payload,
            stream,
        }
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
    content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

/// Build the OpenAI content field for a message. Plain text messages stay a
/// string; messages carrying images become a multimodal content array of
/// `text` and `image_url` (data URL) parts.
fn openai_content(m: &Message, include_images: bool) -> serde_json::Value {
    if !include_images || m.parts.is_empty() {
        return serde_json::Value::String(m.content.clone());
    }
    let mut parts: Vec<serde_json::Value> =
        vec![json!({ "type": "text", "text": m.content.clone() })];
    for img in &m.parts {
        parts.push(json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{};base64,{}", img.content_type, img.data) }
        }));
    }
    serde_json::Value::Array(parts)
}

/// Best-effort detection of vision-capable models for OpenAI-compatible
/// providers. When uncertain, returns false so image content is dropped rather
/// than causing a hard API error on text-only models (e.g. deepseek-chat).
fn is_vision_model(model: &str) -> bool {
    let m = model.to_lowercase();
    [
        "vl",
        "vision",
        "gpt-4o",
        "gpt-4.1",
        "gpt-4-turbo",
        "gpt-5",
        "gemini",
        "claude",
        "glm-4v",
        "kimi-k2",
    ]
    .iter()
    .any(|marker| m.contains(marker))
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

#[derive(Debug, Deserialize)]
struct OpenAiStreamChunk {
    choices: Vec<OpenAiStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiStreamDelta,
}

#[derive(Debug, Deserialize, Default)]
struct OpenAiStreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCall {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<OpenAiStreamToolCallFunction>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCallFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// Accumulates the fragmented pieces of a single streaming tool call (indexed by
/// `delta.tool_calls[].index`) into a complete `ToolCall`.
#[derive(Debug, Default)]
struct OpenAiToolCallAccum {
    id: String,
    name: String,
    arguments: String,
}

/// Parse one SSE event body (the text between `\n\n` boundaries) and fold its
/// `data:` lines into the tool-call accumulators. Returns the concatenated text
/// content carried by the event so the caller can forward it as a `Delta`.
fn apply_sse_event(event: &str, tcs: &mut BTreeMap<usize, OpenAiToolCallAccum>) -> String {
    let mut appended = String::new();
    for line in event.lines() {
        let line = line.trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(chunk) = serde_json::from_str::<OpenAiStreamChunk>(data) else {
            continue;
        };
        for choice in chunk.choices {
            let delta = choice.delta;
            if let Some(content) = delta.content {
                appended.push_str(&content);
            }
            if let Some(list) = delta.tool_calls {
                for tc in list {
                    let acc = tcs.entry(tc.index).or_default();
                    if let Some(id) = tc.id {
                        acc.id = id;
                    }
                    if let Some(f) = tc.function {
                        if let Some(n) = f.name {
                            acc.name.push_str(&n);
                        }
                        if let Some(a) = f.arguments {
                            acc.arguments.push_str(&a);
                        }
                    }
                }
            }
        }
    }
    appended
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

        let payload = self.build_payload(&request, tools, false);

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
                parts: vec![],
            },
            tool_calls,
            usage,
        })
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
        tools: Vec<ToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        if self.api_key.is_empty() {
            return Err(RsmgoError::Provider(format!(
                "{} API key not configured",
                self.name
            )));
        }

        let session_id = request.session_id.clone();
        let payload = self.build_payload(&request, tools, true);
        let base_url = self.base_url.clone();
        let api_key = self.api_key.clone();
        let name = self.name.clone();

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(STREAM_TIMEOUT_SECONDS))
            .build()?;

        let response = client
            .post(format!("{}/chat/completions", base_url))
            .header("Authorization", format!("Bearer {}", api_key))
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
                name, status, text
            )));
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamEvent>>(64);

        tokio::spawn(async move {
            let mut body = response.bytes_stream();
            let mut buffer = String::new();
            let mut text = String::new();
            let mut tcs: BTreeMap<usize, OpenAiToolCallAccum> = BTreeMap::new();

            while let Some(chunk) = body.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx
                            .send(Err(RsmgoError::Provider(format!("stream error: {}", e))))
                            .await;
                        return;
                    }
                };
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                while let Some(pos) = buffer.find("\n\n") {
                    let event = buffer[..pos].to_string();
                    buffer.drain(..pos + 2);
                    let delta = apply_sse_event(&event, &mut tcs);
                    if !delta.is_empty() {
                        text.push_str(&delta);
                        if tx
                            .send(Ok(StreamEvent::Delta { text: delta }))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }

            // Flush any trailing partial event (e.g. a final line without a
            // trailing blank line).
            if !buffer.trim().is_empty() {
                let delta = apply_sse_event(&buffer, &mut tcs);
                if !delta.is_empty() {
                    text.push_str(&delta);
                    let _ = tx.send(Ok(StreamEvent::Delta { text: delta })).await;
                }
            }

            let tool_calls: Vec<ToolCall> = tcs
                .into_values()
                .map(|acc| {
                    let args = serde_json::from_str(&acc.arguments).unwrap_or(json!({}));
                    ToolCall {
                        id: acc.id,
                        name: acc.name,
                        arguments: args,
                    }
                })
                .collect();

            let response = ChatResponse {
                session_id,
                message: Message {
                    role: "assistant".to_string(),
                    content: text,
                    tool_call_id: None,
                    tool_calls: None,
                    parts: vec![],
                },
                tool_calls,
                usage: Usage::default(),
            };
            let _ = tx.send(Ok(StreamEvent::Done { response })).await;
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self.models.clone())
    }
}
