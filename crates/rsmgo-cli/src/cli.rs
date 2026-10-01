//! Command-line interface definition. Bare `rsmgo` (no subcommand) defaults
//! to `chat`, matching how most agent CLIs behave.
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rsmgo")]
#[command(about = "Model-agnostic AI Agent CLI")]
#[command(after_help = "Note: running `rsmgo` with no command starts an interactive\nchat session, the same as `rsmgo chat`.")]
pub struct Cli {
    /// No subcommand defaults to `chat`.
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
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

impl Cli {
    /// Parse argv; a missing subcommand drops into `chat` with defaults.
    pub fn parse_or_default() -> Commands {
        Self::parse().command.unwrap_or(Commands::Chat {
            provider: String::new(),
            model: String::new(),
            session: None,
        })
    }
}
