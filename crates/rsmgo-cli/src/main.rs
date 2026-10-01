use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::StreamExt;
use render::MarkdownRenderer;
use rsmgo_core::agent::Agent;
use rsmgo_core::config::AppConfig;
use rsmgo_core::memory::MemoryStore;
use rsmgo_core::providers::registry_from_config;
use rsmgo_core::types::{ChatRequest, Message, StreamEvent};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

mod highlight;
mod render;

/// ANSI style helpers: colors only when stdout is a terminal.
fn styled(s: &str, code: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("\x1b[{}m{}\x1b[0m", code, s)
    } else {
        s.to_string()
    }
}

fn dim(s: &str) -> String {
    styled(s, "2;3")
}

fn cyan(s: &str) -> String {
    styled(s, "36;1")
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

#[derive(Parser)]
#[command(name = "rsmgo")]
#[command(about = "Model-agnostic AI Agent CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start an interactive chat session
    Chat {
        #[arg(short, long, default_value = "")]
        provider: String,
        #[arg(short, long, default_value = "")]
        model: String,
        #[arg(short, long)]
        session: Option<String>,
    },
    /// Run a single prompt and print the response
    Run {
        #[arg(short, long, default_value = "")]
        provider: String,
        #[arg(short, long, default_value = "")]
        model: String,
        prompt: String,
    },
    /// List available providers and tools
    Config,
}

/// Build the agent from app.yaml, returning it alongside the default provider.
async fn build_agent() -> Result<(Arc<Agent>, String)> {
    let config = AppConfig::load_default()?;

    let data_dir = PathBuf::from(&config.engine.data_dir);
    std::fs::create_dir_all(&data_dir)?;
    let memory = Arc::new(MemoryStore::open(data_dir.join("memory.db"))?);

    let providers = registry_from_config(&config);
    let mut agent = Agent::new(memory, &data_dir).with_providers(providers);
    if let Some(prompt) = &config.engine.system_prompt {
        agent = agent.with_system_prompt(prompt.clone());
    }

    let default_provider = config
        .default_provider_name()
        .unwrap_or("openai")
        .to_string();
    Ok((Arc::new(agent), default_provider))
}

/// Tools enabled by default in the CLI, mirroring the web UI: everything
/// except `web_search`, which requires separate provider-side configuration.
fn default_tool_names(agent: &Agent) -> Vec<String> {
    agent
        .list_tools()
        .into_iter()
        .filter(|name| *name != "web_search")
        .map(|s| s.to_string())
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::WARN.into()))
        .init();

    let cli = Cli::parse();
    let (agent, default_provider) = build_agent().await?;

    match cli.command {
        Commands::Chat {
            provider,
            model,
            session,
        } => {
            let provider = if provider.is_empty() {
                default_provider.clone()
            } else {
                provider
            };
            let session_id = session.unwrap_or_else(|| Uuid::new_v4().to_string());
            println!("rsmgo chat session: {}", session_id);
            println!(
                "provider: {} | model: {} | type '/quit' (or '/exit') to exit",
                provider, model
            );
            let mut editor = DefaultEditor::new()?;
            loop {
                // rustyline edits by character (not byte), so multi-byte
                // UTF-8 input like Chinese is deleted and navigated
                // correctly; read line's byte-based editing garbles it.
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
                if input == "/quit" || input == "/exit" {
                    break;
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
                    workspace: String::new(),
                    workspace_id: String::new(),
                };

                match stream_turn(agent.clone(), request).await {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Error: {}", e);
                    }
                }
            }
        }
        Commands::Run {
            provider,
            model,
            prompt,
        } => {
            let provider = if provider.is_empty() {
                default_provider.clone()
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
                // Raw passthrough adds no trailing newline; keep piped
                // output line-oriented.
                println!();
            }
        }
        Commands::Config => {
            println!("Providers:");
            for p in agent.list_providers() {
                println!("  - {}", p);
            }
            println!("Tools:");
            for t in agent.list_tools() {
                println!("  - {}", t);
            }
        }
    }

    Ok(())
}
