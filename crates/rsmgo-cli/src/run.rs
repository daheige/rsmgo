//! `rsmgo run`: one prompt, one answer, then exit.
use crate::app::default_tool_names;
use crate::render::MarkdownRenderer;
use anyhow::Result;
use rsmgo_core::agent::Agent;
use rsmgo_core::types::{ChatRequest, Message};
use std::io::{self, IsTerminal};
use std::sync::Arc;
use uuid::Uuid;

pub async fn run_prompt(
    agent: Arc<Agent>,
    default_provider: &str,
    provider: String,
    model: String,
    prompt: String,
) -> Result<()> {
    let provider = if provider.is_empty() {
        default_provider.to_string()
    } else {
        provider
    };
    let request = ChatRequest {
        session_id: Uuid::new_v4().to_string(),
        messages: vec![Message::user(prompt)],
        provider,
        model,
        tool_names: default_tool_names(&agent),
        stream: false,
        workspace: String::new(),
        workspace_id: String::new(),
    };
    let resp = agent.chat(request).await?;
    let mut renderer = MarkdownRenderer::new();
    renderer.feed(&resp.message.content);
    renderer.flush();
    if !io::stdout().is_terminal() {
        // Raw passthrough adds no trailing newline; keep piped output
        // line-oriented.
        println!();
    }
    Ok(())
}
