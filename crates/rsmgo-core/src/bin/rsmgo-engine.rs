use rsmgo_core::agent::Agent;
use rsmgo_core::config::AppConfig;
use rsmgo_core::memory::MemoryStore;
use rsmgo_core::providers::registry_from_config;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    let config = AppConfig::load_default()?;

    let data_dir = PathBuf::from(&config.engine.data_dir);
    std::fs::create_dir_all(&data_dir)?;
    let memory = Arc::new(MemoryStore::open(data_dir.join("memory.db"))?);

    let providers = registry_from_config(&config);
    let mut agent = Agent::new(memory).with_providers(providers);
    if let Some(prompt) = &config.engine.system_prompt {
        agent = agent.with_system_prompt(prompt.clone());
    }
    let agent = Arc::new(agent);

    let grpc_addr: SocketAddr = config.engine.grpc_addr.parse()?;
    let http_addr: SocketAddr = config.engine.http_addr.parse()?;

    rsmgo_core::server::run_server(agent, grpc_addr, http_addr, config.engine.app_http_debug)
        .await?;
    Ok(())
}
