use crate::{ExecutionSettings, Provider};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub const PROVIDERS: [Provider; 2] = [Provider::Codex, Provider::Claude];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderPreferences {
    pub enabled: bool,
    pub executable: Option<PathBuf>,
    pub hidden_models: BTreeSet<String>,
}
impl Default for ProviderPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            executable: None,
            hidden_models: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub providers: BTreeMap<Provider, ProviderPreferences>,
    pub default_execution: Option<ExecutionSettings>,
    pub rewrite_model: Option<ExecutionSettings>,
    pub automatic_shortening: bool,
}
impl AppSettings {
    pub fn validate(&self) -> Result<(), AppSettingsValidationError> {
        if self.automatic_shortening && self.rewrite_model.is_none() {
            Err(AppSettingsValidationError::RewriteModelRequired)
        } else {
            Ok(())
        }
    }

    pub fn provider(&self, provider: Provider) -> ProviderPreferences {
        self.providers.get(&provider).cloned().unwrap_or_default()
    }
    pub fn visible(&self, provider: Provider, model: &str) -> bool {
        let p = self.provider(provider);
        p.enabled && !p.hidden_models.contains(model)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppSettingsValidationError {
    RewriteModelRequired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderReadiness {
    Missing,
    SignedOut,
    Ready,
    Unavailable(String),
}
impl ProviderReadiness {
    pub fn label(&self) -> &str {
        match self {
            Self::Missing => "Not installed",
            Self::SignedOut => "Sign-in required",
            Self::Ready => "Signed in",
            Self::Unavailable(_) => "Status unavailable",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalAction {
    Search,
    Command,
    Files,
    Permissions,
    Tool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalField {
    pub label: String,
    pub value: String,
    pub monospace: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalPresentation {
    pub action: ApprovalAction,
    pub fields: Vec<ApprovalField>,
    pub explanation: Option<String>,
    pub raw_arguments: String,
    pub allow_label: String,
}

pub fn model_matches(query: &str, provider: Provider, label: &str, id: &str) -> bool {
    let text = format!("{} {label} {id}", provider.label()).to_lowercase();
    query
        .split_whitespace()
        .all(|word| text.contains(&word.to_lowercase()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalAnswer {
    pub message_id: uuid::Uuid,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseVariant {
    pub message_id: uuid::Uuid,
    pub source_hash: String,
    pub provider: Provider,
    pub model: String,
    pub prompt_version: i64,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_matches_all_tokens_across_provider_and_identifier() {
        assert!(model_matches(
            "CLAUDE haiku",
            Provider::Claude,
            "Haiku",
            "model-1"
        ));
        assert!(!model_matches(
            "codex haiku",
            Provider::Claude,
            "Haiku",
            "model-1"
        ));
        assert!(model_matches("  ", Provider::Codex, "Anything", "x"));
    }
    #[test]
    fn hidden_and_disabled_models_are_excluded_without_losing_preferences() {
        let mut settings = AppSettings::default();
        settings
            .providers
            .entry(Provider::Claude)
            .or_default()
            .hidden_models
            .insert("hidden".into());
        assert!(!settings.visible(Provider::Claude, "hidden"));
        assert!(settings.visible(Provider::Claude, "other"));
        settings
            .providers
            .entry(Provider::Claude)
            .or_default()
            .enabled = false;
        assert!(!settings.visible(Provider::Claude, "other"));
    }
}
