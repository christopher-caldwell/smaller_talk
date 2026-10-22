use crate::AgentRouter;
use async_trait::async_trait;
use serde_json::Value;
use smaller_talk_application::{ExecutionError, SkillCatalog};
use smaller_talk_domain::*;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};
use yaml_rust::{Yaml, YamlLoader};

#[derive(Clone, Debug, Default)]
pub struct SkillLocations {
    pub home: Option<PathBuf>,
    pub claude_config: Option<PathBuf>,
}

#[async_trait]
impl SkillCatalog for AgentRouter {
    fn revision(&self, provider: Provider) -> u64 {
        self.skill_revision(provider)
    }
    async fn discover(
        &self,
        provider: Provider,
        workspace: &Path,
    ) -> Result<Vec<SkillDescriptor>, ExecutionError> {
        let executable = self.executable_for(provider)?;
        match provider {
            Provider::Codex => {
                crate::CodexAdapter::new(executable)
                    .discover_skills(workspace)
                    .await
            }
            Provider::Claude => {
                let locations = self.skill_locations();
                let workspace = workspace.to_owned();
                tokio::task::spawn_blocking(move || claude_skills(&locations, &workspace))
                    .await
                    .map_err(|e| ExecutionError::with_source("Skill discovery stopped", e))?
            }
        }
    }
    async fn inspect(&self, reference: &SkillReference) -> Result<String, ExecutionError> {
        let metadata = tokio::fs::metadata(&reference.path)
            .await
            .map_err(|e| ExecutionError::with_source("Skill instructions are unavailable", e))?;
        if metadata.len() > 256 * 1024 {
            return Err(ExecutionError::new(
                "Skill instructions exceed the 256 KiB preview limit",
            ));
        }
        tokio::fs::read_to_string(&reference.path)
            .await
            .map_err(|e| ExecutionError::with_source("Could not read skill instructions", e))
    }
}

fn read_json(path: &Path) -> Result<Value, ExecutionError> {
    match std::fs::read_to_string(path) {
        Ok(text) => json5::from_str(&text).map_err(|e| {
            ExecutionError::with_source(format!("Could not read {}", path.display()), e)
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(e) => Err(ExecutionError::with_source(
            format!("Could not read {}", path.display()),
            e,
        )),
    }
}
fn documents(root: &Path, commands: bool) -> Result<Vec<PathBuf>, ExecutionError> {
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        let canonical = match dir.canonicalize() {
            Ok(p) => p,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(ExecutionError::with_source(
                    "Skill folder is unavailable",
                    e,
                ));
            }
        };
        if !visited.insert(canonical) {
            continue;
        }
        if !commands && dir.join("SKILL.md").is_file() {
            result.push(dir.join("SKILL.md"));
            continue;
        }
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| ExecutionError::with_source("Could not list skill folder", e))?
        {
            let path = entry
                .map_err(|e| ExecutionError::with_source("Could not read skill entry", e))?
                .path();
            if path.is_dir() {
                pending.push(path);
            } else if path.file_name().is_some_and(|n| n == "SKILL.md")
                || (commands && path.extension().is_some_and(|e| e == "md"))
            {
                result.push(path);
            }
        }
    }
    result.sort();
    Ok(result)
}
fn flag(value: &Yaml) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| value.as_i64().map(|v| v != 0))
        .or_else(|| {
            value
                .as_str()
                .and_then(|s| match s.to_lowercase().as_str() {
                    "true" | "yes" | "on" | "y" => Some(true),
                    "false" | "no" | "off" | "n" => Some(false),
                    _ => None,
                })
        })
}
fn scan(
    root: &Path,
    source: &str,
    prefix: &str,
    workspace: &Path,
    commands: bool,
) -> Result<Vec<SkillDescriptor>, ExecutionError> {
    let mut result = Vec::new();
    for path in documents(root, commands)? {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ExecutionError::with_source("Could not read skill metadata", e))?;
        let text = text.trim_start_matches('\u{feff}');
        let frontmatter = text
            .strip_prefix("---\n")
            .or_else(|| text.strip_prefix("---\r\n"));
        let metadata = match frontmatter {
            Some(rest) => {
                let end = rest.find("\n---").ok_or_else(|| {
                    ExecutionError::new(format!("Unclosed skill frontmatter: {}", path.display()))
                })?;
                YamlLoader::load_from_str(&rest[..end])
                    .map_err(|e| {
                        ExecutionError::with_source(
                            format!("Invalid skill metadata: {}", path.display()),
                            e,
                        )
                    })?
                    .into_iter()
                    .next()
                    .unwrap_or(Yaml::Null)
            }
            None => Yaml::Null,
        };
        let fallback = if commands {
            path.strip_prefix(root)
                .unwrap_or(&path)
                .with_extension("")
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, ":")
        } else {
            path.parent()
                .and_then(Path::file_name)
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        let name = metadata["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .unwrap_or(&fallback);
        let command = format!("{prefix}{name}");
        let unavailable = (flag(&metadata["user-invocable"]) == Some(false))
            .then(|| "This skill is not user-invocable".to_owned());
        result.push(SkillDescriptor {
            reference: SkillReference {
                provider: Provider::Claude,
                name: command.clone(),
                path: path
                    .canonicalize()
                    .map_err(|e| ExecutionError::with_source("Skill source is unavailable", e))?,
                workspace: workspace.to_owned(),
            },
            command,
            description: metadata["description"].as_str().unwrap_or_default().into(),
            source: source.into(),
            unavailable,
        });
    }
    Ok(result)
}

pub(crate) fn claude_skills(
    locations: &SkillLocations,
    workspace: &Path,
) -> Result<Vec<SkillDescriptor>, ExecutionError> {
    // Scan low to high precedence. Native command names select only the winner.
    let config = locations
        .claude_config
        .clone()
        .or_else(|| locations.home.as_ref().map(|h| h.join(".claude")));
    let config = config.map(|p| {
        if p.is_absolute() {
            p
        } else {
            workspace.join(p)
        }
    });
    let mut roots: Vec<(PathBuf, String, String)> = Vec::new();
    let mut settings = Vec::new();
    if let Some(config) = &config {
        settings.push(read_json(&config.join("settings.json"))?);
    }
    let ancestors: Vec<_> = workspace.ancestors().collect();
    for ancestor in ancestors.iter().rev() {
        if config
            .as_ref()
            .is_some_and(|c| *c == ancestor.join(".claude"))
        {
            continue;
        }
        roots.push((ancestor.join(".claude"), "Project".into(), String::new()));
        settings.push(read_json(&ancestor.join(".claude/settings.json"))?);
        settings.push(read_json(&ancestor.join(".claude/settings.local.json"))?);
    }
    if let Some(config) = &config {
        roots.push((config.clone(), "Personal".into(), String::new()));
    }
    let managed = Path::new("/Library/Application Support/ClaudeCode");
    roots.push((managed.to_owned(), "Managed".into(), String::new()));
    settings.push(read_json(&managed.join("managed-settings.json"))?);
    let mut overrides = BTreeMap::new();
    let mut enabled_plugins = BTreeMap::new();
    for setting in settings {
        if let Some(map) = setting["skillOverrides"].as_object()
            && map.values().all(|v| {
                matches!(
                    v.as_str(),
                    Some("on" | "off" | "name-only" | "user-invocable-only")
                )
            })
        {
            for (k, v) in map {
                overrides.insert(k.clone(), v == "off");
            }
        }
        if let Some(map) = setting["enabledPlugins"].as_object() {
            for (k, v) in map {
                enabled_plugins.insert(k.clone(), v.as_bool() == Some(true));
            }
        }
    }
    let mut entries = Vec::new();
    if let Some(config) = &config {
        let registry = read_json(&config.join("plugins/installed_plugins.json"))?;
        if let Some(plugins) = registry["plugins"].as_object() {
            for (id, installs) in plugins {
                if enabled_plugins.get(id) != Some(&true) {
                    continue;
                }
                for install in installs.as_array().into_iter().flatten() {
                    if let Some(project) = install["projectPath"].as_str()
                        && !workspace.starts_with(project)
                    {
                        continue;
                    }
                    if let Some(path) = install["installPath"].as_str() {
                        let plugin = Path::new(path);
                        let manifest = read_json(&plugin.join(".claude-plugin/plugin.json"))?;
                        let prefix = format!(
                            "{}:",
                            manifest["name"]
                                .as_str()
                                .unwrap_or_else(|| id.split('@').next().unwrap_or(id))
                        );
                        for (kind, commands) in [("commands", true), ("skills", false)] {
                            let mut folders = vec![plugin.join(kind)];
                            match &manifest[kind] {
                                Value::String(folder) => folders.push(plugin.join(folder)),
                                Value::Array(array) => folders.extend(
                                    array
                                        .iter()
                                        .filter_map(Value::as_str)
                                        .map(|folder| plugin.join(folder)),
                                ),
                                _ => {}
                            }
                            folders.sort();
                            folders.dedup();
                            for folder in folders {
                                entries
                                    .extend(scan(&folder, "Plugin", &prefix, workspace, commands)?);
                            }
                        }
                    }
                }
            }
        }
    }
    for (root, source, prefix) in roots {
        entries.extend(scan(
            &root.join("commands"),
            &source,
            &prefix,
            workspace,
            true,
        )?);
        entries.extend(scan(
            &root.join("skills"),
            &source,
            &prefix,
            workspace,
            false,
        )?);
    }
    let mut seen_sources = HashSet::new();
    entries.reverse();
    entries.retain(|d| seen_sources.insert((d.command.clone(), d.reference.path.clone())));
    entries.reverse();
    let mut winners = BTreeMap::new();
    for (i, entry) in entries.iter().enumerate() {
        winners.insert(entry.command.clone(), i);
    }
    for (i, entry) in entries.iter_mut().enumerate() {
        if winners[&entry.command] != i {
            entry.unavailable =
                Some("Another source takes precedence for this Claude command".into());
        }
        if entry.source != "Plugin" && overrides.get(&entry.command) == Some(&true) {
            entry.unavailable = Some("This skill is disabled in Claude settings".into());
        }
    }
    entries.sort_by(|a, b| {
        a.reference
            .name
            .cmp(&b.reference.name)
            .then(a.reference.path.cmp(&b.reference.path))
    });
    Ok(entries)
}

pub(crate) fn validate_selected(
    entries: &[SkillDescriptor],
    provider: Provider,
    workspace: &Path,
    text: &str,
    mentions: &[SkillMention],
) -> Result<(), ExecutionError> {
    let mut selected = HashSet::new();
    for mention in mentions {
        let r = &mention.reference;
        if r.provider != provider
            || r.workspace != workspace
            || !mention.matches(text)
            || !mention.is_prose(text)
            || !entries
                .iter()
                .any(|d| d.reference == *r && d.unavailable.is_none())
        {
            return Err(ExecutionError::new(format!(
                "${} is unavailable at its selected source. Replace or remove it, or refresh Skills.",
                r.name
            )));
        }
        selected.insert(&r.path);
    }
    if provider == Provider::Claude && selected.len() > 1 {
        return Err(ExecutionError::new(
            "Claude supports one explicit skill per message",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn personal_root_is_not_duplicated_as_an_ancestor_project() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let workspace = home.join("repo");
        std::fs::create_dir_all(&workspace).unwrap();
        let skill = home.join(".claude/skills/personal");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: personal\n---\nPersonal instructions",
        )
        .unwrap();
        let entries = claude_skills(
            &SkillLocations {
                home: Some(home.into()),
                claude_config: None,
            },
            &workspace,
        )
        .unwrap();
        let matches: Vec<_> = entries
            .iter()
            .filter(|d| d.reference.name == "personal")
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].source, "Personal");
        assert!(matches[0].unavailable.is_none());
    }
    #[test]
    fn claude_precedence_visibility_plugins_and_inspection_do_not_execute_scripts() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let config = root.join("config");
        let workspace = root.join("project");
        for (directory, body) in [
            (
                workspace.join(".claude/skills/review"),
                "---\nname: review\ndescription: Project\n---\n!`touch should-not-exist`",
            ),
            (
                config.join("skills/review"),
                "---\nname: review\ndescription: Personal\n---\nPersonal body",
            ),
            (
                config.join("skills/manual"),
                "---\nname: manual\ndisable-model-invocation: true\n---\nManual body",
            ),
            (
                config.join("skills/hidden"),
                "---\nname: hidden\nuser-invocable: false\n---\nHidden body",
            ),
            (config.join("skills/off"), "---\nname: off\n---\nOff body"),
            (
                root.join("plugin/skills/check"),
                "---\nname: check\n---\nPlugin body",
            ),
        ] {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), body).unwrap();
        }
        std::fs::write(
            config.join("settings.json"),
            "{ // JSONC\n skillOverrides: {off:'off'}, enabledPlugins:{'sample@market':true}, }",
        )
        .unwrap();
        std::fs::create_dir_all(config.join("plugins")).unwrap();
        std::fs::write(config.join("plugins/installed_plugins.json"),serde_json::json!({"plugins":{"sample@market":[{"installPath":root.join("plugin"),"scope":"user"}]}}).to_string()).unwrap();
        let entries = claude_skills(
            &SkillLocations {
                home: None,
                claude_config: Some(config),
            },
            &workspace,
        )
        .unwrap();
        let project = entries
            .iter()
            .find(|e| e.source == "Project" && e.reference.name == "review")
            .unwrap();
        assert!(project.unavailable.is_some());
        assert!(entries.iter().any(|e| e.source == "Personal"
            && e.reference.name == "review"
            && e.unavailable.is_none()));
        assert!(
            entries
                .iter()
                .any(|e| e.reference.name == "manual" && e.unavailable.is_none())
        );
        assert!(
            entries
                .iter()
                .any(|e| e.reference.name == "hidden" && e.unavailable.is_some())
        );
        assert!(
            entries
                .iter()
                .any(|e| e.reference.name == "off" && e.unavailable.is_some())
        );
        assert!(
            entries
                .iter()
                .any(|e| e.reference.name == "sample:check" && e.unavailable.is_none())
        );
        assert!(!workspace.join("should-not-exist").exists());
    }
}
