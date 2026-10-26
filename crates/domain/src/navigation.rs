use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Project;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSort {
    #[default]
    Alphabetical,
    LastChatUpdated,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavigationPreferences {
    pub project_sort: ProjectSort,
    pub hidden_projects: BTreeSet<Uuid>,
    pub collapsed_projects: BTreeSet<Uuid>,
}

impl NavigationPreferences {
    pub fn sorted_projects(
        &self,
        projects: &[Project],
        activity: &BTreeMap<Uuid, DateTime<Utc>>,
    ) -> Vec<Project> {
        let mut projects = projects.to_vec();
        projects.sort_by(|a, b| {
            let recent = if self.project_sort == ProjectSort::LastChatUpdated {
                activity.get(&b.id).cmp(&activity.get(&a.id))
            } else {
                std::cmp::Ordering::Equal
            };
            recent
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.id.cmp(&b.id))
        });
        projects
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchChatsInput {
    pub text: String,
    pub archived: bool,
    pub hidden_projects: BTreeSet<Uuid>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComposerCommand {
    Compact,
    Invalid(String),
    Message,
}

/// Only a leading command-shaped token is interpreted. Paths, quoted/code text,
/// escaped slashes and commands mentioned in prose remain ordinary messages.
pub fn composer_command(text: &str) -> ComposerCommand {
    let text = text.trim();
    let token = text.split_whitespace().next().unwrap_or_default();
    let Some(name) = token.strip_prefix('/') else {
        return ComposerCommand::Message;
    };
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return ComposerCommand::Message;
    }
    if token != "/compact" {
        return ComposerCommand::Invalid(format!(
            "Unsupported command {token}. Use /compact; use $name for skills."
        ));
    }
    if text != token {
        return ComposerCommand::Invalid(
            "Use /compact alone, without arguments or other content.".into(),
        );
    }
    ComposerCommand::Compact
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_do_not_capture_paths_code_or_prose() {
        for text in [
            "/Users/me/code",
            "`/compact`",
            "\\/compact",
            "Try /compact",
            "```\n/compact\n```",
            "https://example.com",
        ] {
            assert_eq!(composer_command(text), ComposerCommand::Message, "{text}");
        }
        assert_eq!(composer_command(" /compact \n"), ComposerCommand::Compact);
        assert!(matches!(
            composer_command("/compact keep this"),
            ComposerCommand::Invalid(_)
        ));
        assert!(matches!(
            composer_command("/clear"),
            ComposerCommand::Invalid(_)
        ));
    }
    #[test]
    fn recent_projects_have_stable_ties_and_empty_projects_last() {
        let projects: Vec<_> = [(3, "Zebra"), (2, "café"), (1, "Alpha")]
            .into_iter()
            .map(|(id, name)| Project {
                id: Uuid::from_u128(id),
                name: name.into(),
                folder: None,
            })
            .collect();
        let preferences = NavigationPreferences {
            project_sort: ProjectSort::LastChatUpdated,
            ..Default::default()
        };
        let now = DateTime::from_timestamp_millis(1000).unwrap();
        let activity = [(Uuid::from_u128(3), now), (Uuid::from_u128(1), now)].into();
        assert_eq!(
            preferences
                .sorted_projects(&projects, &activity)
                .iter()
                .map(|p| p.id.as_u128())
                .collect::<Vec<_>>(),
            vec![1, 3, 2]
        );
    }
}
