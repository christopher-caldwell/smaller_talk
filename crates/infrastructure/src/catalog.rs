use std::sync::Arc;

use async_trait::async_trait;
use smaller_talk_application::{
    ExecutionAdapter, ExecutionError, ExecutionEvent, ExecutionRequest,
};
use smaller_talk_domain::*;
use tokio::sync::{OnceCell, mpsc};

use crate::{ClaudeAdapter, CodexAdapter};

pub struct AgentRouter {
    codex: CodexAdapter,
    claude: ClaudeAdapter,
    codex_models: OnceCell<ProviderCapabilities>,
    claude_models: OnceCell<ProviderCapabilities>,
}
impl AgentRouter {
    pub fn new(codex: CodexAdapter, claude: ClaudeAdapter) -> Arc<Self> {
        Arc::new(Self {
            codex,
            claude,
            codex_models: OnceCell::new(),
            claude_models: OnceCell::new(),
        })
    }
}
#[async_trait]
impl ExecutionAdapter for AgentRouter {
    async fn capabilities(
        &self,
        provider: Provider,
    ) -> Result<ProviderCapabilities, ExecutionError> {
        let cell = match provider {
            Provider::Codex => &self.codex_models,
            Provider::Claude => &self.claude_models,
        };
        cell.get_or_try_init(|| async {
            match provider {
                Provider::Codex => self.codex.capabilities(provider).await,
                Provider::Claude => self.claude.capabilities(provider).await,
            }
        })
        .await
        .cloned()
    }
    async fn execute(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError> {
        let (_, responses) = mpsc::unbounded_channel();
        self.execute_interactive(request, events, responses).await
    }
    async fn execute_interactive(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
        responses: mpsc::UnboundedReceiver<InteractionResponse>,
    ) -> Result<(), ExecutionError> {
        match request.settings.provider {
            Provider::Codex => {
                self.codex
                    .execute_interactive(request, events, responses)
                    .await
            }
            Provider::Claude => {
                self.claude
                    .execute_interactive(request, events, responses)
                    .await
            }
        }
    }
}
