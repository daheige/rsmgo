use anyhow::Result;
use clap::{Parser, Subcommand};
use rsmgo_core::agent::Agent;
use rsmgo_core::config::AppConfig;
use rsmgo_core::memory::MemoryStore;
use rsmgo_core::providers::registry_from_config;
use rsmgo_core::types::{ChatRequest, Message};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

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
    let mut agent = Agent::new(memory).with_providers(providers);
    if let Some(prompt) = &config.engine.system_prompt {
        agent = agent.with_system_prompt(prompt.clone());
    }

    let default_provider = config
        .default_provider_name()
        .unwrap_or("openai")
        .to_string();
    Ok((Arc::new(agent), default_provider))
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
                "provider: {} | model: {} | type '/quit' to exit",
                provider, model
            );
            loop {
                print!("> ");
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let input = input.trim();
                if input == "/quit" {
                    break;
                }
                if input.is_empty() {
                    continue;
                }

                let request = ChatRequest {
                    session_id: session_id.clone(),
                    messages: vec![Message::user(input)],
                    provider: provider.clone(),
                    model: model.clone(),
                    tool_names: vec![],
                    stream: false,
                };

                match agent.chat(request).await {
                    Ok(resp) => {
                        println!("{}", resp.message.content);
                    }
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
                tool_names: vec![],
                stream: false,
            };
            let resp = agent.chat(request).await?;
            println!("{}", resp.message.content);
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
