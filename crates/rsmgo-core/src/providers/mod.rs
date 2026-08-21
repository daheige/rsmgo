use crate::config::AppConfig;
use crate::error::Result;
use crate::types::{ChatRequest, ChatResponse, ModelInfo, ToolDefinition};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub mod anthropic;
pub mod openai;

pub use anthropic::AnthropicProvider;
pub use openai::OpenAiCompatibleProvider;

/// A generic LLM provider trait.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn chat(&self, request: ChatRequest, tools: Vec<ToolDefinition>) -> Result<ChatResponse>;

    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
}

pub type ProviderRef = Arc<dyn LlmProvider>;

pub struct ProviderRegistry {
    providers: HashMap<String, ProviderRef>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    pub fn register(&mut self, provider: ProviderRef) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    pub fn get(&self, name: &str) -> Option<ProviderRef> {
        self.providers.get(name).cloned()
    }

    pub fn list(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn default_registry() -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(OpenAiCompatibleProvider::default_openai()));
    registry.register(Arc::new(OpenAiCompatibleProvider::default_deepseek()));
    registry.register(Arc::new(OpenAiCompatibleProvider::default_qwen()));
    registry.register(Arc::new(OpenAiCompatibleProvider::default_kimi()));
    registry.register(Arc::new(AnthropicProvider::default()));
    registry
}

/// Build a provider registry from app.yaml's `providers` section.
///
/// `anthropic` entries use the Anthropic API; everything else is treated as an
/// OpenAI-compatible provider (OpenAI, DeepSeek, Qwen, Kimi, ...).
pub fn registry_from_config(config: &AppConfig) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    for entry in &config.providers {
        let base_url = entry.base_url.clone().unwrap_or_default();
        let default_model = entry.default_model.clone().unwrap_or_default();
        let models: Vec<ModelInfo> = entry
            .models
            .iter()
            .map(|m| ModelInfo {
                id: m.id.clone(),
                provider: entry.name.clone(),
                display_name: if m.display_name.is_empty() {
                    m.id.clone()
                } else {
                    m.display_name.clone()
                },
            })
            .collect();

        let provider: ProviderRef = if entry.name == "anthropic" {
            Arc::new(AnthropicProvider::new(
                base_url,
                entry.api_key.clone(),
                default_model,
                models,
            ))
        } else {
            Arc::new(OpenAiCompatibleProvider::new(
                entry.name.clone(),
                base_url,
                entry.api_key.clone(),
                default_model,
                models,
            ))
        };
        registry.register(provider);
    }
    registry
}
