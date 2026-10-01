//! Interactive chat REPL: banner, workspace trust prompt, slash commands
//! (/provider, /model, /workspace, /providers, /help, /quit), and streaming
//! one turn at a time through [`stream_turn`].
use crate::app::default_tool_names;
use crate::providers::{
    model_names, print_provider_list, provider_names, provider_summary, resolve_model,
};
use crate::render::MarkdownRenderer;
use crate::style::{cyan, dim};
use crate::workspace::{expand_tilde, prompt_workspace_trust};
use anyhow::Result;
use futures::StreamExt;
use rsmgo_core::agent::Agent;
use rsmgo_core::config::AppConfig;
use rsmgo_core::types::{ChatRequest, Message, StreamEvent};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::io::{self, Write};
use std::sync::Arc;
use uuid::Uuid;

/// Run the interactive chat session until the user quits.
pub async fn run_chat(
    agent: Arc<Agent>,
    default_provider: &str,
    config: &AppConfig,
    provider: String,
    model: String,
    session: Option<String>,
) -> Result<()> {
    let mut provider = if provider.is_empty() {
        default_provider.to_string()
    } else {
        provider
    };
    let mut model = resolve_model(config, &provider, &model);
    let session_id = session.unwrap_or_else(|| Uuid::new_v4().to_string());
    println!("rsmgo chat session: {}", session_id);
    println!(
        "provider: {} | model: {} | /provider <name>, /model <name>, /workspace <dir>, /quit",
        provider, model
    );
    println!("providers: {}", provider_summary(config));
    let mut editor = DefaultEditor::new()?;

    // Confirm trust in the working directory before handing the file tools
    // access to it. Declining (or Ctrl-C/Ctrl-D) exits the CLI.
    let mut workspace = std::env::current_dir()?;
    if !prompt_workspace_trust(&mut editor, &workspace)? {
        println!("Exiting without accessing the workspace.");
        return Ok(());
    }
    println!("Workspace: {}", workspace.display());
    loop {
        // rustyline edits by character (not byte), so multi-byte UTF-8 input
        // like Chinese is deleted and navigated correctly; a byte-based line
        // editor would garble it.
        let input = match editor.readline("> ") {
            Ok(line) => line,
            // Ctrl-C / Ctrl-D: leave the chat without an error trace.
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("input error: {}", e);
                break;
            }
        };
        let input = input.trim();
        if input == "/quit" || input == "/exit" || input == "quit" || input == "exit" {
            break;
        }
        if input == "/providers" {
            print_provider_list(config);
            continue;
        }
        if input == "/help" {
            println!("commands:");
            println!("  /provider <name>  switch provider (e.g. /provider kimi)");
            println!("  /model <name>     switch model (e.g. /model deepseek-reasoner)");
            println!("  /providers        list providers and their models");
            println!("  /workspace <dir>  set the workspace directory");
            println!("  /quit, /exit      exit chat (typing exit or quit also works)");
            continue;
        }
        if let Some(name) = input.strip_prefix("/provider") {
            let name = name.trim();
            if name.is_empty() {
                let available = provider_names(config).join(", ");
                println!("current provider: {} (available: {})", provider, available);
            } else if let Some(entry) = config.find_provider(name) {
                provider = name.to_string();
                model = entry.default_model.clone().unwrap_or_default();
                println!("switched to provider: {} | model: {}", provider, model);
            } else {
                let available = provider_names(config).join(", ");
                println!("unknown provider: {} (available: {})", name, available);
            }
            continue;
        }
        if let Some(name) = input.strip_prefix("/model") {
            let name = name.trim();
            let known = model_names(config, &provider);
            if name.is_empty() {
                let available = if known.is_empty() {
                    "any".to_string()
                } else {
                    known.join(", ")
                };
                println!("current model: {} (available: {})", model, available);
            } else if known.is_empty() || known.contains(&name) {
                model = name.to_string();
                println!("switched to model: {}", model);
            } else {
                println!(
                    "unknown model: {} for provider {} (available: {})",
                    name,
                    provider,
                    known.join(", ")
                );
            }
            continue;
        }
        if let Some(dir) = input.strip_prefix("/workspace") {
            let dir = dir.trim();
            if dir.is_empty() {
                println!("current workspace: {}", workspace.display());
                continue;
            }
            let raw = expand_tilde(dir);
            let resolved = if raw.is_absolute() {
                raw
            } else {
                std::env::current_dir().unwrap_or_default().join(raw)
            };
            match std::fs::canonicalize(&resolved) {
                Ok(path) if path.is_dir() => {
                    workspace = path;
                    println!("switched to workspace: {}", workspace.display());
                }
                _ => println!(
                    "workspace not found or not a directory: {}",
                    resolved.display()
                ),
            }
            continue;
        }
        // Any other `/...` input is a mistyped or unknown command; do not send
        // it to the model.
        if input.starts_with('/') {
            println!(
                "unknown command: {} (try /provider <name>, /model <name>, /providers, /quit)",
                input
            );
            continue;
        }
        if input.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(input);

        let request = ChatRequest {
            session_id: session_id.clone(),
            messages: vec![Message::user(input)],
            provider: provider.clone(),
            model: model.clone(),
            tool_names: default_tool_names(&agent),
            stream: true,
            workspace: workspace.to_string_lossy().into_owned(),
            workspace_id: String::new(),
        };

        match stream_turn(agent.clone(), request).await {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Error: {}", e);
            }
        }
    }
    Ok(())
}

/// Stream one chat turn to stdout, tracing the ReAct agent loop in
/// English-labeled stages: Thought (reasoning, dimmed) → Action (tool call)
/// → Observation (tool result) → ... → Final Answer. Ctrl-C interrupts the
/// turn ("[Request interrupted by user]") and returns to the prompt, like
/// Claude Code / Codex.
async fn stream_turn(agent: Arc<Agent>, request: ChatRequest) -> Result<()> {
    let mut stream = agent.chat_stream(request).await?;
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
    let mut renderer = MarkdownRenderer::new();
    let mut thinking = false;
    let mut answered = false;

    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                renderer.flush();
                println!("\n[Request interrupted by user]");
                return Ok(());
            }
            event = stream.next() => match event {
                Some(Ok(StreamEvent::Reasoning { text })) => {
                    if !thinking {
                        println!("{}", dim("Thought:"));
                        thinking = true;
                    }
                    print!("{}", dim(&text));
                    io::stdout().flush()?;
                }
                Some(Ok(StreamEvent::Action { round, name, arguments })) => {
                    // The answer and thought sections are over; start the
                    // action trace on a fresh line.
                    if answered || thinking {
                        renderer.flush();
                        println!();
                        answered = false;
                        thinking = false;
                    }
                    let args = serde_json::to_string(&arguments).unwrap_or_default();
                    println!("{} {}", cyan(&format!("Action (round {}):", round)), name);
                    println!("  {}", dim(&args));
                    io::stdout().flush()?;
                }
                Some(Ok(StreamEvent::Observation { round, name, output })) => {
                    println!(
                        "{} {}",
                        cyan(&format!("Observation (round {}):", round)),
                        name
                    );
                    for line in output.lines().take(10) {
                        println!("  {}", dim(line));
                    }
                    io::stdout().flush()?;
                }
                Some(Ok(StreamEvent::Delta { text })) => {
                    if !answered {
                        // Separate the trace stages from the answer; the
                        // answer itself needs no header.
                        println!();
                        thinking = false;
                        answered = true;
                    }
                    renderer.feed(&text);
                }
                Some(Ok(StreamEvent::Done { .. })) => {
                    renderer.flush();
                    println!();
                    return Ok(());
                }
                Some(Err(e)) => {
                    renderer.flush();
                    println!();
                    return Err(e.into());
                }
                None => {
                    renderer.flush();
                    println!();
                    return Ok(());
                }
            },
        }
    }
}
