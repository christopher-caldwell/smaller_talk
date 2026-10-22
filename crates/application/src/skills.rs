use crate::{ExecutionError, PersistenceError};
use async_trait::async_trait;
use smaller_talk_domain::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[async_trait]
pub trait SkillCatalog: Send + Sync {
    fn revision(&self, provider: Provider) -> u64;
    async fn discover(
        &self,
        provider: Provider,
        workspace: &Path,
    ) -> Result<Vec<SkillDescriptor>, ExecutionError>;
    async fn inspect(&self, reference: &SkillReference) -> Result<String, ExecutionError>;
}
#[async_trait]
pub trait SkillQueries: Send + Sync {
    async fn message_skills(
        &self,
        chat: Uuid,
        message: Uuid,
    ) -> Result<Vec<SkillMention>, PersistenceError>;
}
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("Could not discover skills: {0}")]
    Discovery(#[source] ExecutionError),
    #[error("{0}. Replace or remove the skill, or refresh Skills.")]
    Unresolved(String),
}

type CatalogKey = (Provider, u64, PathBuf);
type CachedCatalog = (Instant, Vec<SkillDescriptor>);
#[derive(Default)]
struct SkillCache {
    entries: HashMap<CatalogKey, CachedCatalog>,
    generations: HashMap<Provider, u64>,
}
pub struct SkillService {
    catalog: Arc<dyn SkillCatalog>,
    cache: Mutex<SkillCache>,
}
impl SkillService {
    pub fn new(catalog: Arc<dyn SkillCatalog>) -> Self {
        Self {
            catalog,
            cache: Mutex::new(SkillCache::default()),
        }
    }
    pub fn invalidate(&self, provider: Provider) {
        if let Ok(mut cache) = self.cache.lock() {
            *cache.generations.entry(provider).or_default() += 1;
            cache.entries.retain(|(p, _, _), _| *p != provider);
        }
    }
    pub fn revision(&self, provider: Provider) -> u64 {
        self.catalog.revision(provider)
    }
    pub async fn list(
        &self,
        provider: Provider,
        workspace: &Path,
        refresh: bool,
    ) -> Result<Vec<SkillDescriptor>, SkillError> {
        let key = (
            provider,
            self.catalog.revision(provider),
            workspace.to_owned(),
        );
        let generation = {
            let cache = self
                .cache
                .lock()
                .map_err(|_| SkillError::Unresolved("Skill cache unavailable".into()))?;
            if !refresh
                && let Some((at, entries)) = cache.entries.get(&key)
                && at.elapsed() < Duration::from_secs(60)
            {
                return Ok(entries.clone());
            }
            *cache.generations.get(&provider).unwrap_or(&0)
        };
        let entries = self
            .catalog
            .discover(provider, workspace)
            .await
            .map_err(SkillError::Discovery)?;
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| SkillError::Unresolved("Skill cache unavailable".into()))?;
        if self.catalog.revision(provider) != key.1
            || generation != *cache.generations.get(&provider).unwrap_or(&0)
        {
            return Err(SkillError::Unresolved(
                "Provider configuration or skills changed during discovery".into(),
            ));
        }
        cache
            .entries
            .retain(|(p, r, _), _| *p != provider || *r == key.1);
        cache.entries.insert(key, (Instant::now(), entries.clone()));
        Ok(entries)
    }
    pub async fn inspect(&self, reference: &SkillReference) -> Result<String, SkillError> {
        let entries = self
            .list(reference.provider, &reference.workspace, true)
            .await?;
        if !entries.iter().any(|d| d.reference == *reference) {
            return Err(SkillError::Unresolved(
                "Skill source is no longer available".into(),
            ));
        }
        self.catalog
            .inspect(reference)
            .await
            .map_err(SkillError::Discovery)
    }
    pub async fn validate(
        &self,
        provider: Provider,
        workspace: &Path,
        text: &str,
        mentions: &[SkillMention],
        fresh: bool,
    ) -> Result<Vec<SkillDescriptor>, SkillError> {
        if mentions.is_empty() {
            return Ok(Vec::new());
        }
        let entries = self.list(provider, workspace, fresh).await?;
        let mut selected = Vec::new();
        for mention in mentions {
            let reference = &mention.reference;
            if reference.provider != provider
                || reference.workspace != workspace
                || !mention.matches(text)
                || !mention.is_prose(text)
            {
                return Err(SkillError::Unresolved(format!(
                    "${} no longer matches this message, provider, or folder",
                    reference.name
                )));
            }
            let entry = entries
                .iter()
                .find(|d| d.reference == *reference)
                .ok_or_else(|| {
                    SkillError::Unresolved(format!(
                        "${} is unavailable at {}",
                        reference.name,
                        reference.path.display()
                    ))
                })?;
            if let Some(reason) = &entry.unavailable {
                return Err(SkillError::Unresolved(reason.clone()));
            }
            if !selected
                .iter()
                .any(|d: &SkillDescriptor| d.reference == *reference)
            {
                selected.push(entry.clone());
            }
        }
        if provider == Provider::Claude && selected.len() > 1 {
            return Err(SkillError::Unresolved(
                "Claude supports one explicit skill per message".into(),
            ));
        }
        Ok(selected)
    }
}

pub fn effective_workspace(
    launch: &Path,
    preferences: &ChatPreferences,
    projects: &[Project],
) -> Result<PathBuf, ExecutionError> {
    let folder = if let Some(id) = &preferences.project_id {
        projects
            .iter()
            .find(|p| p.id.to_string() == *id)
            .ok_or_else(|| ExecutionError::new("The assigned project no longer exists"))?
            .folder
            .as_deref()
            .unwrap_or(launch)
    } else {
        launch
    };
    let folder = folder
        .canonicalize()
        .map_err(|e| ExecutionError::with_source("The working folder is unavailable", e))?;
    if !folder.is_dir() {
        return Err(ExecutionError::new("The working folder is not a directory"));
    }
    Ok(folder)
}

/// Resolve only known prose names; an existing selected identity is never rebound.
pub enum SkillNameResolution {
    Unknown,
    Selected(SkillReference),
    Ambiguous,
    Unavailable(String),
}
pub fn resolve_skill_name(entries: &[SkillDescriptor], name: &str) -> SkillNameResolution {
    let known: Vec<_> = entries
        .iter()
        .filter(|d| d.reference.name == name)
        .collect();
    let mut available: Vec<_> = known.iter().filter(|d| d.unavailable.is_none()).collect();
    available.dedup_by(|a, b| a.reference == b.reference);
    match available.as_slice() {
        [entry] => SkillNameResolution::Selected(entry.reference.clone()),
        [] if known.is_empty() => SkillNameResolution::Unknown,
        [] => SkillNameResolution::Unavailable(
            known[0]
                .unavailable
                .clone()
                .unwrap_or_else(|| "This skill is unavailable".into()),
        ),
        _ => SkillNameResolution::Ambiguous,
    }
}

pub fn resolve_skill_links(
    text: &str,
    entries: &[SkillDescriptor],
) -> Vec<(std::ops::Range<usize>, SkillReference)> {
    let mut links = Vec::new();
    let mut offset = 0;
    while let Some(start) = text[offset..].find("[$").map(|i| i + offset) {
        let Some(label_end) = text[start..].find("](").map(|i| i + start) else {
            break;
        };
        let Some(end) = text[label_end + 2..].find(')').map(|i| i + label_end + 2) else {
            break;
        };
        offset = end + 1;
        // Recognize only links in prose, using the same literal/code exclusion.
        let probe = format!("{}$probe ", &text[..start]);
        if !skill_tokens(&probe).iter().any(|(r, _)| r.start == start) {
            continue;
        }
        let path = text[label_end + 2..end]
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>');
        let decoded = if path.starts_with("file:") {
            url::Url::parse(path)
                .ok()
                .and_then(|u| u.to_file_path().ok())
        } else {
            Some(std::path::PathBuf::from(path))
        };
        if let Some(path) = decoded {
            let matches: Vec<_> = entries
                .iter()
                .filter(|d| {
                    let path = if path.is_absolute() {
                        path.clone()
                    } else {
                        d.reference.workspace.join(&path)
                    };
                    let mut normalized = std::path::PathBuf::new();
                    for component in path.components() {
                        match component {
                            std::path::Component::ParentDir => {
                                normalized.pop();
                            }
                            std::path::Component::CurDir => {}
                            component => normalized.push(component.as_os_str()),
                        }
                    }
                    d.reference.path == normalized && d.unavailable.is_none()
                })
                .collect();
            if matches.len() == 1 {
                links.push((start..end + 1, matches[0].reference.clone()));
            }
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    struct Catalog {
        revision: AtomicU64,
        calls: AtomicUsize,
    }
    #[async_trait]
    impl SkillCatalog for Catalog {
        fn revision(&self, _: Provider) -> u64 {
            self.revision.load(Ordering::SeqCst)
        }
        async fn discover(
            &self,
            provider: Provider,
            workspace: &Path,
        ) -> Result<Vec<SkillDescriptor>, ExecutionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(["one", "two"]
                .into_iter()
                .map(|source| SkillDescriptor {
                    reference: SkillReference {
                        provider,
                        workspace: workspace.into(),
                        name: "review".into(),
                        path: PathBuf::from(format!("/{source}/SKILL.md")),
                    },
                    description: "Review changes".into(),
                    source: source.into(),
                    command: "review".into(),
                    unavailable: None,
                })
                .collect())
        }
        async fn inspect(&self, _: &SkillReference) -> Result<String, ExecutionError> {
            Ok("instructions".into())
        }
    }
    #[tokio::test]
    async fn changed_notifications_reject_in_flight_discovery() {
        struct PausedCatalog {
            first: std::sync::atomic::AtomicBool,
            started: tokio::sync::Notify,
            resume: tokio::sync::Notify,
        }
        #[async_trait]
        impl SkillCatalog for PausedCatalog {
            fn revision(&self, _: Provider) -> u64 {
                0
            }
            async fn discover(
                &self,
                _: Provider,
                _: &Path,
            ) -> Result<Vec<SkillDescriptor>, ExecutionError> {
                if self.first.swap(false, Ordering::SeqCst) {
                    self.started.notify_one();
                    self.resume.notified().await;
                }
                Ok(Vec::new())
            }
            async fn inspect(&self, _: &SkillReference) -> Result<String, ExecutionError> {
                unreachable!()
            }
        }
        let catalog = Arc::new(PausedCatalog {
            first: std::sync::atomic::AtomicBool::new(true),
            started: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
        });
        let service = Arc::new(SkillService::new(catalog.clone()));
        let pending = service.clone();
        let task = tokio::spawn(async move {
            pending
                .list(Provider::Codex, Path::new("/work"), false)
                .await
        });
        catalog.started.notified().await;
        service.invalidate(Provider::Codex);
        catalog.resume.notify_one();
        assert!(task.await.unwrap().is_err());
        assert!(
            service
                .list(Provider::Codex, Path::new("/work"), false)
                .await
                .unwrap()
                .is_empty()
        );
    }
    #[tokio::test]
    async fn known_links_preserve_exact_paths_and_literals_are_not_resolved() {
        let catalog = Catalog {
            revision: AtomicU64::new(0),
            calls: AtomicUsize::new(0),
        };
        let entries = catalog
            .discover(Provider::Codex, Path::new("/work"))
            .await
            .unwrap();
        assert!(matches!(
            resolve_skill_name(&entries, "review"),
            SkillNameResolution::Ambiguous
        ));
        assert!(matches!(
            resolve_skill_name(&entries, "unknown"),
            SkillNameResolution::Unknown
        ));
        let links = resolve_skill_links("[$display name](/two/SKILL.md)", &entries);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].1, entries[1].reference);
        assert_eq!(
            resolve_skill_links("[$review](../one/SKILL.md)", &entries)[0].1,
            entries[0].reference
        );
        assert!(resolve_skill_links("[$review](/unknown/SKILL.md)", &entries).is_empty());
        assert!(resolve_skill_links("`code [$review](/two/SKILL.md)`", &entries).is_empty());
        assert!(resolve_skill_links("```md\n[$review](/two/SKILL.md)\n```", &entries).is_empty());
    }
    #[tokio::test]
    async fn cache_scopes_refreshes_and_preserves_exact_source() {
        let catalog = Arc::new(Catalog {
            revision: AtomicU64::new(0),
            calls: AtomicUsize::new(0),
        });
        let service = SkillService::new(catalog.clone());
        let workspace = Path::new("/work");
        let entries = service
            .list(Provider::Codex, workspace, false)
            .await
            .unwrap();
        service
            .list(Provider::Codex, workspace, false)
            .await
            .unwrap();
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
        let mention = SkillMention {
            range: 0..7,
            reference: entries[1].reference.clone(),
        };
        let resolved = service
            .validate(
                Provider::Codex,
                workspace,
                "$review",
                std::slice::from_ref(&mention),
                false,
            )
            .await
            .unwrap();
        assert_eq!(resolved[0].source, "two");
        assert!(
            service
                .validate(
                    Provider::Claude,
                    workspace,
                    "$review",
                    std::slice::from_ref(&mention),
                    false
                )
                .await
                .is_err()
        );
        assert!(
            service
                .validate(
                    Provider::Codex,
                    Path::new("/other"),
                    "$review",
                    std::slice::from_ref(&mention),
                    false
                )
                .await
                .is_err()
        );
        assert!(
            service
                .validate(
                    Provider::Codex,
                    workspace,
                    "changed",
                    std::slice::from_ref(&mention),
                    false
                )
                .await
                .is_err()
        );
        service
            .list(Provider::Codex, workspace, true)
            .await
            .unwrap();
        let before = catalog.calls.load(Ordering::SeqCst);
        catalog.revision.store(1, Ordering::SeqCst);
        service
            .list(Provider::Codex, workspace, false)
            .await
            .unwrap();
        assert_eq!(catalog.calls.load(Ordering::SeqCst), before + 1);
        service.invalidate(Provider::Codex);
        service
            .list(Provider::Codex, workspace, false)
            .await
            .unwrap();
        assert_eq!(catalog.calls.load(Ordering::SeqCst), before + 2);
    }
}
