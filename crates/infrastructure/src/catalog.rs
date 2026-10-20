use crate::{ClaudeAdapter, CodexAdapter};
use async_trait::async_trait;
use smaller_talk_application::{
    ExecutionAdapter, ExecutionError, ExecutionEvent, ExecutionRequest, ProviderControl,
    SteerCommand,
};
use smaller_talk_domain::*;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{process::Command, sync::mpsc, time::timeout};

#[derive(Clone)]
struct Connection {
    executable: PathBuf,
    enabled: bool,
    revision: u64,
    catalog: Option<(Instant, ProviderCapabilities)>,
}
pub struct AgentRouter {
    connections: Mutex<BTreeMap<Provider, Connection>>,
}
impl AgentRouter {
    pub fn new(codex: CodexAdapter, claude: ClaudeAdapter) -> Arc<Self> {
        Arc::new(Self {
            connections: Mutex::new(
                [
                    (
                        Provider::Codex,
                        Connection {
                            executable: codex.executable().into(),
                            enabled: true,
                            revision: 0,
                            catalog: None,
                        },
                    ),
                    (
                        Provider::Claude,
                        Connection {
                            executable: claude.executable().into(),
                            enabled: true,
                            revision: 0,
                            catalog: None,
                        },
                    ),
                ]
                .into(),
            ),
        })
    }
    fn connection(&self, provider: Provider) -> Result<Connection, ExecutionError> {
        let connection = self
            .connections
            .lock()
            .map_err(|_| ExecutionError::new("Provider configuration is unavailable"))?[&provider]
            .clone();
        if !connection.enabled {
            return Err(ExecutionError::new(format!(
                "{} is disabled. Enable it in Settings or select another provider; queued messages are preserved.",
                provider.label()
            )));
        }
        Ok(connection)
    }
    pub(crate) fn executable_for(&self, provider: Provider) -> Result<PathBuf, ExecutionError> {
        Ok(self.connection(provider)?.executable)
    }
    async fn catalog(
        &self,
        provider: Provider,
        refresh: bool,
    ) -> Result<ProviderCapabilities, ExecutionError> {
        let connection = self.connection(provider)?;
        if !refresh
            && let Some((at, catalog)) = &connection.catalog
            && at.elapsed() < Duration::from_secs(300)
        {
            return Ok(catalog.clone());
        }
        let catalog = match provider {
            Provider::Codex => {
                CodexAdapter::new(&connection.executable)
                    .capabilities(provider)
                    .await?
            }
            Provider::Claude => {
                ClaudeAdapter::new(&connection.executable)
                    .capabilities(provider)
                    .await?
            }
        };
        let mut connections = self
            .connections
            .lock()
            .map_err(|_| ExecutionError::new("Provider configuration is unavailable"))?;
        let current = connections.get_mut(&provider).expect("known provider");
        if current.revision != connection.revision {
            return Err(ExecutionError::new(
                "Provider configuration changed during discovery. Refresh to try again.",
            ));
        }
        current.catalog = Some((Instant::now(), catalog.clone()));
        Ok(catalog)
    }
}
#[async_trait]
impl ProviderControl for AgentRouter {
    fn configure(&self, provider: Provider, executable: PathBuf, enabled: bool) {
        if let Ok(mut connections) = self.connections.lock() {
            let current = connections.get_mut(&provider).expect("known provider");
            if current.executable != executable || current.enabled != enabled {
                current.executable = executable;
                current.enabled = enabled;
                current.revision += 1;
                current.catalog = None;
            }
        }
    }
    async fn readiness(&self, provider: Provider) -> ProviderReadiness {
        let executable = match self.connections.lock() {
            Ok(c) => c[&provider].executable.clone(),
            Err(_) => {
                return ProviderReadiness::Unavailable(
                    "Provider configuration is unavailable".into(),
                );
            }
        };
        let mut command = Command::new(executable);
        command
            .args(match provider {
                Provider::Codex => vec!["login", "status"],
                Provider::Claude => vec!["auth", "status", "--json"],
            })
            .kill_on_drop(true);
        match timeout(Duration::from_secs(8), command.output()).await {
            Ok(Ok(output)) => {
                if provider == Provider::Claude {
                    match serde_json::from_slice::<serde_json::Value>(&output.stdout).ok().and_then(|v| v["loggedIn"].as_bool()) {
                        Some(true) => ProviderReadiness::Ready,
                        Some(false) => ProviderReadiness::SignedOut,
                        None => ProviderReadiness::Unavailable("The CLI did not return a readable sign-in status. Check its installation.".into()),
                    }
                } else if output.status.success() {
                    ProviderReadiness::Ready
                } else {
                    ProviderReadiness::SignedOut
                }
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => ProviderReadiness::Missing,
            Ok(Err(_)) => {
                ProviderReadiness::Unavailable("Could not run the configured executable".into())
            }
            Err(_) => ProviderReadiness::Unavailable("Sign-in status check timed out".into()),
        }
    }
    async fn refresh(&self, provider: Provider) -> Result<ProviderCapabilities, ExecutionError> {
        {
            let mut connections = self
                .connections
                .lock()
                .map_err(|_| ExecutionError::new("Provider configuration is unavailable"))?;
            let connection = connections.get_mut(&provider).expect("known provider");
            connection.revision += 1;
            connection.catalog = None;
        }
        self.catalog(provider, true).await
    }
}
#[async_trait]
impl ExecutionAdapter for AgentRouter {
    async fn capabilities(
        &self,
        provider: Provider,
    ) -> Result<ProviderCapabilities, ExecutionError> {
        self.catalog(provider, false).await
    }
    async fn execute(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError> {
        let (_, responses) = mpsc::unbounded_channel();
        self.execute_interactive(request, events, responses).await
    }
    async fn execute_controlled(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
        responses: mpsc::UnboundedReceiver<InteractionResponse>,
        controls: mpsc::UnboundedReceiver<SteerCommand>,
    ) -> Result<(), ExecutionError> {
        let c = self.connection(request.settings.provider)?;
        match request.settings.provider {
            Provider::Codex => {
                CodexAdapter::new(c.executable)
                    .execute_controlled(request, events, responses, controls)
                    .await
            }
            Provider::Claude => {
                ClaudeAdapter::new(c.executable)
                    .execute_controlled(request, events, responses, controls)
                    .await
            }
        }
    }
    async fn execute_interactive(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
        responses: mpsc::UnboundedReceiver<InteractionResponse>,
    ) -> Result<(), ExecutionError> {
        let (_, controls) = mpsc::unbounded_channel();
        self.execute_controlled(request, events, responses, controls)
            .await
    }
}
