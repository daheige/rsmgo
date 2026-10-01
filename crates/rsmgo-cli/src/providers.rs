//! Provider / model lookup and listing helpers shared by the chat banner,
//! slash commands, and the `config` subcommand.
use rsmgo_core::config::AppConfig;

/// Resolve the model to use: an explicit `--model` wins, otherwise fall back
/// to the provider's configured default model.
pub fn resolve_model(config: &AppConfig, provider: &str, model: &str) -> String {
    if !model.is_empty() {
        return model.to_string();
    }
    config
        .find_provider(provider)
        .and_then(|p| p.default_model.clone())
        .unwrap_or_default()
}

/// Names of all configured providers, for slash-command help text.
pub fn provider_names(config: &AppConfig) -> Vec<&str> {
    config.providers.iter().map(|p| p.name.as_str()).collect()
}

/// Models configured for a provider, for `/model` validation.
pub fn model_names<'a>(config: &'a AppConfig, provider: &str) -> Vec<&'a str> {
    config
        .find_provider(provider)
        .map(|p| p.models.iter().map(|m| m.id.as_str()).collect())
        .unwrap_or_default()
}

/// Print every configured provider with its default model and model list.
pub fn print_provider_list(config: &AppConfig) {
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
pub fn provider_summary(config: &AppConfig) -> String {
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
