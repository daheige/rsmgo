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

/// Resolve the model to use: an explicit `--model` wins, otherwise fall back
/// to the provider's configured default model.
fn resolve_model(config: &AppConfig, provider: &str, model: &str) -> String {
    if !model.is_empty() {
        return model.to_string();
    }
    config
        .find_provider(provider)
        .and_then(|p| p.default_model.clone())
        .unwrap_or_default()
}

/// Names of all configured providers, for slash-command help text.
fn provider_names(config: &AppConfig) -> Vec<&str> {
    config.providers.iter().map(|p| p.name.as_str()).collect()
}

/// Models configured for a provider, for `/model` validation.
fn model_names<'a>(config: &'a AppConfig, provider: &str) -> Vec<&'a str> {
    config
        .find_provider(provider)
        .map(|p| p.models.iter().map(|m| m.id.as_str()).collect())
        .unwrap_or_default()
}

/// Print every configured provider with its default model and model list.
fn print_provider_list(config: &AppConfig) {
    println!("Providers:");
    for p in &config.providers {
        let default = p.default_model.as_deref().unwrap_or("");
        println!("  - {} (default: {})", p.name, default);
        let models = p
            .models
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if !models.is_empty() {
            println!("      models: {}", models);
        }
    }
}

/// Compact listing of configured providers and their models, for the chat
/// banner so users know what `--provider` / `--model` accept.
fn provider_summary(config: &AppConfig) -> String {
    config
        .providers
        .iter()
        .map(|p| {
            let models = p
                .models
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            if models.is_empty() {
                p.name.clone()
            } else {
                format!("{} ({})", p.name, models)
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
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
async fn build_agent() -> Result<(Arc<Agent>, String, AppConfig)> {
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
    Ok((Arc::new(agent), default_provider, config))
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
    let (agent, default_provider, config) = build_agent().await?;

    match cli.command {
        Commands::Chat {
            provider,
            model,
            session,
        } => {
            let mut provider = if provider.is_empty() {
                default_provider.clone()
            } else {
                provider
            };
            let mut model = resolve_model(&config, &provider, &model);
            let session_id = session.unwrap_or_else(|| Uuid::new_v4().to_string());
            println!("rsmgo chat session: {}", session_id);
            println!(
                "provider: {} | model: {} | /provider <name>, /model <name>, /quit to exit",
                provider, model
            );
            println!("providers: {}", provider_summary(&config));
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
                if input == "/providers" {
                    print_provider_list(&config);
                    continue;
                }
                if input == "/help" {
                    println!("commands:");
                    println!("  /provider <name>  switch provider (e.g. /provider kimi)");
                    println!("  /model <name>     switch model (e.g. /model deepseek-reasoner)");
                    println!("  /providers        list providers and their models");
                    println!("  /quit, /exit      exit chat");
                    continue;
                }
                if let Some(name) = input.strip_prefix("/provider") {
                    let name = name.trim();
                    if name.is_empty() {
                        let available = provider_names(&config).join(", ");
                        println!("current provider: {} (available: {})", provider, available);
                    } else if let Some(entry) = config.find_provider(name) {
                        provider = name.to_string();
                        model = entry.default_model.clone().unwrap_or_default();
                        println!("switched to provider: {} | model: {}", provider, model);
                    } else {
                        let available = provider_names(&config).join(", ");
                        println!("unknown provider: {} (available: {})", name, available);
                    }
                    continue;
                }
                if let Some(name) = input.strip_prefix("/model") {
                    let name = name.trim();
                    let known = model_names(&config, &provider);
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
                // Any other `/...` input is a mistyped or unknown command; do
                // not send it to the model.
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
            print_provider_list(&config);
            println!("Tools:");
            for t in agent.list_tools() {
                println!("  - {}", t);
            }
        }
    }

    Ok(())
}
