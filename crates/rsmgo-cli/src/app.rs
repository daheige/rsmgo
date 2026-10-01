//! Agent construction and tool selection shared by all subcommands.
use anyhow::Result;
use rsmgo_core::agent::Agent;
use rsmgo_core::config::AppConfig;
use rsmgo_core::memory::MemoryStore;
use rsmgo_core::providers::registry_from_config;
use std::path::PathBuf;
use std::sync::Arc;

/// Build the agent from app.yaml, returning it alongside the default provider.
pub async fn build_agent() -> Result<(Arc<Agent>, String, AppConfig)> {
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
pub fn default_tool_names(agent: &Agent) -> Vec<String> {
    agent
        .list_tools()
        .into_iter()
        .filter(|name| *name != "web_search")
        .map(|s| s.to_string())
        .collect()
}
