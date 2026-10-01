mod app;
mod chat;
mod cli;
mod config_cmd;
mod highlight;
mod providers;
mod render;
mod run;
mod style;
mod workspace;

use anyhow::Result;
use cli::{Cli, Commands};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::WARN.into()))
        .init();

    // Parse first so `-h` exits before any agent/config work happens.
    let command = Cli::parse_or_default();
    let (agent, default_provider, config) = app::build_agent().await?;

    match command {
        Commands::Chat {
            provider,
            model,
            session,
        } => {
            chat::run_chat(
                agent,
                &default_provider,
                &config,
                provider,
                model,
                session,
            )
            .await
        }
        Commands::Run {
            provider,
            model,
            prompt,
        } => {
            run::run_prompt(agent, &default_provider, provider, model, prompt).await
        }
        Commands::Config => {
            config_cmd::show_config(&agent, &config);
            Ok(())
        }
    }
}
