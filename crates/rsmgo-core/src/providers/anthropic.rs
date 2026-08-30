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
// Streaming responses can legitimately run longer than a single buffered call.
const STREAM_TIMEOUT_SECONDS: u64 = 300;

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
            "".to_string(), // configured via app.yaml using ${ANTHROPIC_API_KEY}
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

    /// Build the `/v1/messages` request body, shared by the buffered and
    /// streaming paths. Only the `stream` flag differs between them.
    fn build_payload(
        &self,
        request: &ChatRequest,
        tools: Vec<ToolDefinition>,
        stream: bool,
    ) -> AnthropicChatRequest {
        let model = if request.model.is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };

        let messages: Vec<AnthropicMessage> = request
            .messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| AnthropicMessage {
                role: m.role.clone(),
                content: anthropic_content(m),
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

        AnthropicChatRequest {
            model,
            max_tokens: 4096,
            messages,
            tools: tools_payload,
            stream,
        }
    }
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Debug, Serialize)]
struct AnthropicChatRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
    #[serde(skip_serializing_if = "is_false")]
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

/// Build the Anthropic content field for a message. Plain text messages stay a
/// string; messages carrying images become a content array of `text` and
/// `image` (base64 source) blocks.
fn anthropic_content(m: &Message) -> serde_json::Value {
    if m.parts.is_empty() {
        return serde_json::Value::String(m.content.clone());
    }
    let mut parts: Vec<serde_json::Value> =
        vec![json!({ "type": "text", "text": m.content.clone() })];
    for img in &m.parts {
        parts.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": img.content_type.clone(),
                "data": img.data.clone(),
            }
        }));
    }
    serde_json::Value::Array(parts)
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

#[derive(Debug, Deserialize)]
struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    content_block: Option<AnthropicStreamContentBlock>,
    #[serde(default)]
    delta: Option<AnthropicStreamDelta>,
    #[serde(default)]
    usage: Option<AnthropicStreamUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicStreamContentBlock {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        name: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicStreamDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicStreamUsage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
}

/// Mutable state accumulated while streaming one Anthropic message.
struct AnthropicStreamState {
    text: String,
    tcs: BTreeMap<usize, AnthropicToolUseAccum>,
    input_tokens: u32,
    output_tokens: u32,
}

/// Accumulates the fragmented pieces of a single streamed `tool_use` block,
/// keyed by the block `index` reported in `content_block_start`.
#[derive(Debug, Default)]
struct AnthropicToolUseAccum {
    id: String,
    name: String,
    input_json: String,
}

/// Parse one SSE event body (the text between `\n\n` boundaries) and fold it
/// into the stream state. Returns the concatenated text content carried by the
/// event so the caller can forward it as a `Delta`.
fn apply_anthropic_event(event: &str, state: &mut AnthropicStreamState) -> String {
    let mut appended = String::new();
    for line in event.lines() {
        let line = line.trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        let Ok(ev) = serde_json::from_str::<AnthropicStreamEvent>(data) else {
            continue;
        };
        match ev.event_type.as_str() {
            "content_block_start" => {
                if let (Some(index), Some(block)) = (ev.index, ev.content_block) {
                    if let AnthropicStreamContentBlock::ToolUse { id, name } = block {
                        let acc = state.tcs.entry(index).or_default();
                        if let Some(id) = id {
                            acc.id = id;
                        }
                        if let Some(name) = name {
                            acc.name = name;
                        }
                    }
                }
            }
            "content_block_delta" => {
                if let (Some(index), Some(delta)) = (ev.index, ev.delta) {
                    match delta {
                        AnthropicStreamDelta::TextDelta { text } => appended.push_str(&text),
                        AnthropicStreamDelta::InputJsonDelta { partial_json } => {
                            state
                                .tcs
                                .entry(index)
                                .or_default()
                                .input_json
                                .push_str(&partial_json);
                        }
                    }
                }
            }
            "message_start" => {
                if let Some(u) = ev.usage {
                    if let Some(t) = u.input_tokens {
                        state.input_tokens = t;
                    }
                }
            }
            "message_delta" => {
                if let Some(u) = ev.usage {
                    if let Some(t) = u.output_tokens {
                        state.output_tokens = t;
                    }
                }
            }
            _ => {}
        }
    }
    appended
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

        let payload = self.build_payload(&request, tools, false);

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
                parts: vec![],
            },
            tool_calls,
            usage: Usage {
                prompt_tokens: data.usage.input_tokens,
                completion_tokens: data.usage.output_tokens,
                total_tokens: data.usage.input_tokens + data.usage.output_tokens,
            },
        })
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
        tools: Vec<ToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        if self.api_key.is_empty() {
            return Err(RsmgoError::Provider(
                "Anthropic API key not configured".to_string(),
            ));
        }

        let session_id = request.session_id.clone();
        let payload = self.build_payload(&request, tools, true);
        let base_url = self.base_url.clone();
        let api_key = self.api_key.clone();

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(STREAM_TIMEOUT_SECONDS))
            .build()?;

        let response = client
            .post(format!("{}/messages", base_url))
            .header("x-api-key", api_key)
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

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamEvent>>(64);

        tokio::spawn(async move {
            let mut body = response.bytes_stream();
            let mut buffer = String::new();
            let mut state = AnthropicStreamState {
                text: String::new(),
                tcs: BTreeMap::new(),
                input_tokens: 0,
                output_tokens: 0,
            };

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
                    let delta = apply_anthropic_event(&event, &mut state);
                    if !delta.is_empty() {
                        state.text.push_str(&delta);
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
                let delta = apply_anthropic_event(&buffer, &mut state);
                if !delta.is_empty() {
                    state.text.push_str(&delta);
                    let _ = tx.send(Ok(StreamEvent::Delta { text: delta })).await;
                }
            }

            let tool_calls: Vec<ToolCall> = state
                .tcs
                .into_values()
                .map(|acc| ToolCall {
                    id: acc.id,
                    name: acc.name,
                    arguments: serde_json::from_str(&acc.input_json).unwrap_or(json!({})),
                })
                .collect();

            let response = ChatResponse {
                session_id,
                message: Message {
                    role: "assistant".to_string(),
                    content: state.text,
                    tool_call_id: None,
                    tool_calls: None,
                    parts: vec![],
                },
                tool_calls,
                usage: Usage {
                    prompt_tokens: state.input_tokens,
                    completion_tokens: state.output_tokens,
                    total_tokens: state.input_tokens + state.output_tokens,
                },
            };
            let _ = tx.send(Ok(StreamEvent::Done { response })).await;
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self.models.clone())
    }
}
