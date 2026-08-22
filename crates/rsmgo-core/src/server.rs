use crate::agent::Agent;
use crate::types::{
    ChatRequest, ChatResponse, Message, MultiModalPart, ToolCall, ToolDefinition, Usage,
};
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use futures::Stream;
use rsmgo_pb::proto;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tonic::{Request, Response, Status};

use proto::engine_server::{Engine, EngineServer};
use proto::{
    ChatRequest as ProtoChatRequest, ChatResponse as ProtoChatResponse, ChatStreamChunk,
    ExecuteToolRequest, ExecuteToolResponse, HealthRequest, HealthResponse, ListModelsRequest,
    ListModelsResponse, ListToolsRequest, ListToolsResponse, Message as ProtoMessage,
    ModelInfo as ProtoModelInfo, MultiModalPart as ProtoMultiModalPart, ToolCall as ProtoToolCall,
    ToolInfo, Usage as ProtoUsage,
};

#[derive(Clone)]
pub struct EngineService {
    agent: Arc<Agent>,
}

impl EngineService {
    pub fn new(agent: Arc<Agent>) -> Self {
        Self { agent }
    }
}

fn map_message(m: &Message) -> ProtoMessage {
    ProtoMessage {
        role: m.role.clone(),
        content: m.content.clone(),
        tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
        tool_calls: m
            .tool_calls
            .as_ref()
            .map(|tcs| tcs.iter().map(map_tool_call).collect())
            .unwrap_or_default(),
        parts: m.parts.iter().map(map_part).collect(),
    }
}

fn map_proto_message(m: &ProtoMessage) -> Message {
    Message {
        role: m.role.clone(),
        content: m.content.clone(),
        tool_call_id: if m.tool_call_id.is_empty() {
            None
        } else {
            Some(m.tool_call_id.clone())
        },
        tool_calls: if m.tool_calls.is_empty() {
            None
        } else {
            Some(
                m.tool_calls
                    .iter()
                    .map(|tc| ToolCall {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        arguments: serde_json::from_str(&tc.arguments)
                            .unwrap_or(serde_json::Value::Null),
                    })
                    .collect(),
            )
        },
        parts: m.parts.iter().map(map_proto_part).collect(),
    }
}

fn map_part(img: &MultiModalPart) -> ProtoMultiModalPart {
    ProtoMultiModalPart {
        content_type: img.content_type.clone(),
        data: img.data.clone(),
    }
}

fn map_proto_part(img: &ProtoMultiModalPart) -> MultiModalPart {
    MultiModalPart {
        content_type: img.content_type.clone(),
        data: img.data.clone(),
    }
}

fn map_tool_call(tc: &ToolCall) -> ProtoToolCall {
    ProtoToolCall {
        id: tc.id.clone(),
        name: tc.name.clone(),
        arguments: tc.arguments.to_string(),
    }
}

fn map_usage(u: &Usage) -> ProtoUsage {
    ProtoUsage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
    }
}

fn map_tool_definition(t: &ToolDefinition) -> ToolInfo {
    ToolInfo {
        name: t.name.clone(),
        description: t.description.clone(),
        parameters_schema: t.parameters.to_string(),
    }
}

fn map_chat_request(req: ProtoChatRequest) -> ChatRequest {
    ChatRequest {
        session_id: req.session_id,
        messages: req.messages.iter().map(map_proto_message).collect(),
        provider: req.provider,
        model: req.model,
        tool_names: req.tool_names,
        stream: req.stream,
    }
}

fn map_chat_response(resp: ChatResponse) -> ProtoChatResponse {
    ProtoChatResponse {
        session_id: resp.session_id,
        message: Some(map_message(&resp.message)),
        tool_calls: resp.tool_calls.iter().map(map_tool_call).collect(),
        usage: Some(map_usage(&resp.usage)),
    }
}

#[tonic::async_trait]
impl Engine for EngineService {
    type ChatStreamStream =
        Pin<Box<dyn Stream<Item = std::result::Result<ChatStreamChunk, Status>> + Send>>;

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> std::result::Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            status: "ok".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }))
    }

    async fn chat(
        &self,
        request: Request<ProtoChatRequest>,
    ) -> std::result::Result<Response<ProtoChatResponse>, Status> {
        let req = map_chat_request(request.into_inner());
        let resp = self
            .agent
            .chat(req)
            .await
            .map_err(|e| Status::internal(format!("agent error: {}", e)))?;
        Ok(Response::new(map_chat_response(resp)))
    }

    async fn chat_stream(
        &self,
        _request: Request<ProtoChatRequest>,
    ) -> std::result::Result<Response<Self::ChatStreamStream>, Status> {
        // MVP: stream placeholder - return a single final chunk.
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let _ = tx
            .send(Ok(ChatStreamChunk {
                session_id: "placeholder".to_string(),
                delta: "streaming not yet implemented in MVP".to_string(),
                done: true,
                message: None,
                tool_calls: vec![],
            }))
            .await;
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(stream) as Self::ChatStreamStream))
    }

    async fn list_models(
        &self,
        _request: Request<ListModelsRequest>,
    ) -> std::result::Result<Response<ListModelsResponse>, Status> {
        // Aggregate models from all providers.
        let mut models = Vec::new();
        for name in self.agent.list_providers() {
            // We cannot directly call provider here; expose through agent later if needed.
            // For MVP return provider names as model IDs.
            models.push(ProtoModelInfo {
                id: name.to_string(),
                provider: name.to_string(),
                display_name: name.to_string(),
            });
        }
        Ok(Response::new(ListModelsResponse { models }))
    }

    async fn execute_tool(
        &self,
        _request: Request<ExecuteToolRequest>,
    ) -> std::result::Result<Response<ExecuteToolResponse>, Status> {
        Ok(Response::new(ExecuteToolResponse {
            success: false,
            output: "".to_string(),
            error: "not implemented".to_string(),
        }))
    }

    async fn list_tools(
        &self,
        _request: Request<ListToolsRequest>,
    ) -> std::result::Result<Response<ListToolsResponse>, Status> {
        let tools = self
            .agent
            .tool_definitions()
            .iter()
            .map(map_tool_definition)
            .collect();
        Ok(Response::new(ListToolsResponse { tools }))
    }
}

// HTTP routes.

#[derive(serde::Serialize)]
struct HealthJson {
    status: String,
    version: String,
}

async fn http_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let _ = state;
    axum::Json(HealthJson {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn http_chat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> std::result::Result<impl IntoResponse, StatusCode> {
    let resp = state.agent.chat(req).await.map_err(|e| {
        tracing::error!("chat error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(axum::Json(resp))
}

async fn http_list_tools(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let defs = state.agent.tool_definitions();
    axum::Json(defs)
}

async fn http_list_providers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let providers: Vec<String> = state
        .agent
        .list_providers()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    axum::Json(providers)
}

pub struct AppState {
    pub agent: Arc<Agent>,
}

pub fn http_router(agent: Arc<Agent>) -> Router {
    let state = Arc::new(AppState { agent });
    Router::new()
        .route("/health", get(http_health))
        .route("/api/v1/chat", post(http_chat))
        .route("/api/v1/tools", get(http_list_tools))
        .route("/api/v1/providers", get(http_list_providers))
        .with_state(state)
}

pub async fn run_server(
    agent: Arc<Agent>,
    grpc_addr: SocketAddr,
    http_addr: SocketAddr,
    app_http_debug: bool,
) -> crate::error::Result<()> {
    let service = EngineService::new(agent.clone());
    let grpc = tonic::transport::Server::builder()
        .add_service(EngineServer::new(service))
        .serve(grpc_addr);

    tracing::info!("gRPC listening on {}", grpc_addr);

    // The HTTP/JSON API is a debug convenience only; the main data path is
    // gRPC. When disabled, run the gRPC server alone.
    if !app_http_debug {
        return grpc.await.map_err(|e| e.into());
    }

    let app = http_router(agent);
    let listener = tokio::net::TcpListener::bind(http_addr).await?;
    let http = axum::serve(listener, app);
    tracing::info!("HTTP debug API listening on {}", http_addr);

    tokio::select! {
        result = grpc => result.map_err(|e| e.into()),
        result = http => result.map_err(|e| e.into()),
    }
}
