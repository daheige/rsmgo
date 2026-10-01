//! `rsmgo config`: list configured providers, their models, and tools.
use crate::providers::print_provider_list;
use rsmgo_core::agent::Agent;
use rsmgo_core::config::AppConfig;

pub fn show_config(agent: &Agent, config: &AppConfig) {
    print_provider_list(config);
    println!("Tools:");
    for t in agent.list_tools() {
        println!("  - {}", t);
    }
}
