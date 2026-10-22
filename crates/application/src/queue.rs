//! Durable delivery is independent of UI lifetime. One actor serializes claims and completion.
mod enqueue;
use crate::*;
use async_trait::async_trait;
pub use enqueue::*;
use std::{collections::HashMap, error::Error, sync::Arc};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("{0}")]
    Policy(String),
    #[error("Queue operation failed: {0}")]
    Source(#[source] Box<dyn Error + Send + Sync + 'static>),
    #[error("Message queue is unavailable")]
    Unavailable,
}
#[derive(Debug, thiserror::Error)]
#[error("Could not list queued messages: {0}")]
pub struct ListQueuedIdsError(#[from] QueueError);
impl QueueError {
    fn from_source(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Source(Box::new(error))
    }
}
impl From<String> for QueueError {
    fn from(value: String) -> Self {
        Self::Policy(value)
    }
}
impl From<&str> for QueueError {
    fn from(value: &str) -> Self {
        Self::Policy(value.to_owned())
    }
}

#[async_trait]
pub trait QueueQueries: Send + Sync {
    async fn queue(&self, chat: Uuid) -> Result<MessageQueue, PersistenceError>;
}

#[async_trait]
pub trait QueueWrites: Send + Sync {
    /// Atomically move an undelivered queue draft into an empty composer draft.
    async fn edit_queued(&self, chat: Uuid, entry: Uuid)
    -> Result<Option<Draft>, PersistenceError>;
    async fn transition(
        &self,
        id: Uuid,
        from: &str,
        to: &str,
        error: Option<&str>,
    ) -> Result<bool, PersistenceError>;
    async fn remove_queued(
        &self,
        chat: Uuid,
        id: Uuid,
        delivered: bool,
    ) -> Result<(), PersistenceError>;
    async fn pause_queue(&self, chat: Uuid, reason: Option<&str>) -> Result<(), PersistenceError>;
    async fn recover_queues(&self) -> Result<(), PersistenceError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnqueueInput {
    pub chat_id: Uuid,
    pub draft: Draft,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SteerQueueInput {
    pub chat_id: Uuid,
    pub entry_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditQueueInput {
    pub chat_id: Uuid,
    pub entry_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoveQueueInput {
    pub chat_id: Uuid,
    pub entry_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryQueueInput {
    pub chat_id: Uuid,
    pub entry_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RespondQueueInput {
    pub chat_id: Uuid,
    pub response: InteractionResponse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SteerInput {
    pub skills: Vec<SkillMention>,
    pub id: Uuid,
    pub prompt: String,
    pub attachments: Vec<Attachment>,
}
impl SteerInput {
    pub fn text_with_files(&self) -> String {
        let mut text = self.prompt.clone();
        for file in &self.attachments {
            text.push_str(&format!(
                "\n\n[Attached file {:?}, saved at {}]",
                file.name,
                file.path.display()
            ));
        }
        text
    }
}
pub struct SteerCommand {
    pub input: SteerInput,
    pub result: oneshot::Sender<SteerOutcome>,
}
#[derive(Debug)]
pub enum SteerOutcome {
    Accepted,
    Rejected(String),
    Uncertain(String),
    NotRunning,
}

pub enum QueueEvent {
    Snapshot(Uuid, MessageQueue),
    Started(Uuid),
    Conversation(ConversationEvent),
    Finished(Uuid, Result<(), QueueError>),
    Error(Uuid, QueueError),
}

#[derive(Clone)]
pub struct QueueCoordinator {
    commands: mpsc::UnboundedSender<Command>,
}
enum Command {
    Edit(Uuid, Uuid, oneshot::Sender<Result<Draft, QueueError>>),
    Enqueue(Uuid, Draft, oneshot::Sender<Result<(), EnqueueError>>),
    Refresh(Uuid),
    Ids(Uuid, oneshot::Sender<Result<Vec<Uuid>, ListQueuedIdsError>>),
    Resume(Uuid),
    Remove(Uuid, Uuid),
    Retry(Uuid, Uuid),
    Steer(Uuid, Uuid),
    Respond(Uuid, InteractionResponse),
    Event(Uuid, ConversationEvent),
    Finished(Uuid, Result<(), QueueError>),
    SteeringResult(Uuid, Uuid, SteerOutcome),
}
struct Running {
    entry: Uuid,
    entry_delivered: bool,
    controls: mpsc::UnboundedSender<SteerCommand>,
    responses: mpsc::UnboundedSender<InteractionResponse>,
    waiting: bool,
    pending_request: Option<String>,
    has_steered: bool,
    steering_available: bool,
}
struct Coordinator<U: UnitOfWork> {
    enqueuer: Arc<EnqueueService<U>>,
    queries: Arc<dyn QueueQueries>,
    writes: Arc<dyn QueueWrites>,
    service: Arc<ConversationService>,
    sender: mpsc::UnboundedSender<Command>,
    output: mpsc::UnboundedSender<QueueEvent>,
    running: HashMap<Uuid, Running>,
}
impl QueueCoordinator {
    /// Call once after startup recovery, while inside the application's runtime.
    pub fn start<U: UnitOfWork + 'static>(
        queries: Arc<dyn QueueQueries>,
        writes: Arc<dyn QueueWrites>,
        enqueuer: Arc<EnqueueService<U>>,
        service: Arc<ConversationService>,
        output: mpsc::UnboundedSender<QueueEvent>,
    ) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        tokio::spawn(
            Coordinator {
                enqueuer,
                queries,
                writes,
                service,
                sender: sender.clone(),
                output,
                running: HashMap::new(),
            }
            .run(receiver),
        );
        Self { commands: sender }
    }
    pub async fn enqueue(&self, input: EnqueueInput) -> Result<(), EnqueueError> {
        let EnqueueInput {
            chat_id: chat,
            draft,
        } = input;
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(Command::Enqueue(chat, draft, tx))
            .map_err(|_| EnqueueError::Unavailable)?;
        rx.await.map_err(|_| EnqueueError::Unavailable)?
    }
    pub async fn queued_ids(&self, chat: Uuid) -> Result<Vec<Uuid>, ListQueuedIdsError> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(Command::Ids(chat, tx))
            .map_err(|_| QueueError::Unavailable)?;
        rx.await.map_err(|_| QueueError::Unavailable)?
    }
    pub fn refresh(&self, chat: Uuid) {
        let _ = self.commands.send(Command::Refresh(chat));
    }
    pub fn resume(&self, chat: Uuid) {
        let _ = self.commands.send(Command::Resume(chat));
    }
    pub async fn edit(&self, input: EditQueueInput) -> Result<Draft, QueueError> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(Command::Edit(input.chat_id, input.entry_id, tx))
            .map_err(|_| QueueError::Unavailable)?;
        rx.await.map_err(|_| QueueError::Unavailable)?
    }
    pub fn remove(&self, input: RemoveQueueInput) {
        let _ = self
            .commands
            .send(Command::Remove(input.chat_id, input.entry_id));
    }
    pub fn retry(&self, input: RetryQueueInput) {
        let _ = self
            .commands
            .send(Command::Retry(input.chat_id, input.entry_id));
    }
    pub fn steer(&self, input: SteerQueueInput) {
        let _ = self
            .commands
            .send(Command::Steer(input.chat_id, input.entry_id));
    }
    pub fn respond(&self, input: RespondQueueInput) {
        let _ = self
            .commands
            .send(Command::Respond(input.chat_id, input.response));
    }
}
impl<U: UnitOfWork + 'static> Coordinator<U> {
    async fn publish(&self, chat: Uuid) -> Result<(), QueueError> {
        let mut queue = self
            .queries
            .queue(chat)
            .await
            .map_err(QueueError::from_source)?;
        queue.steer_available = self
            .running
            .get(&chat)
            .is_some_and(|r| r.steering_available && !r.waiting);
        if let Some(run) = self.running.get(&chat)
            && run.entry_delivered
        {
            queue.entries.retain(|e| e.id != run.entry);
        }
        for entry in &mut queue.entries {
            entry.draft.text = entry.draft.text.chars().take(160).collect();
        }
        let _ = self.output.send(QueueEvent::Snapshot(chat, queue));
        Ok(())
    }
    async fn pump(&mut self, chat: Uuid) -> Result<(), QueueError> {
        if self.running.contains_key(&chat) {
            return Ok(());
        }
        let queue = self
            .queries
            .queue(chat)
            .await
            .map_err(QueueError::from_source)?;
        if queue.paused.is_some() {
            return Ok(());
        }
        let Some(entry) = queue.entries.first() else {
            return Ok(());
        };
        if entry.state != "pending" {
            return Ok(());
        }
        if !self
            .writes
            .transition(entry.id, "pending", "dispatching", None)
            .await
            .map_err(QueueError::from_source)?
        {
            return Ok(());
        }
        let (controls, control_rx) = mpsc::unbounded_channel();
        let (responses, response_rx) = mpsc::unbounded_channel();
        self.running.insert(
            chat,
            Running {
                entry: entry.id,
                entry_delivered: false,
                controls,
                responses,
                waiting: false,
                pending_request: None,
                has_steered: false,
                steering_available: false,
            },
        );
        let _ = self.output.send(QueueEvent::Started(chat));
        let service = self.service.clone();
        let sender = self.sender.clone();
        let entry = entry.clone();
        tokio::spawn(async move {
            let (events, mut rx) = mpsc::unbounded_channel();
            let relay = sender.clone();
            let forwarding = tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let _ = relay.send(Command::Event(chat, event));
                }
            });
            let result = service
                .send_queued_turn(
                    SendTurnInput {
                        chat_id: chat,
                        skills: entry.draft.skills,
                        prompt: entry.draft.text,
                        attachments: entry.draft.attachments,
                        retry_of: None,
                    },
                    entry.id,
                    events,
                    response_rx,
                    control_rx,
                )
                .await
                .map_err(QueueError::from_source);
            let _ = forwarding.await;
            let _ = sender.send(Command::Finished(chat, result));
        });
        self.publish(chat).await?;
        Ok(())
    }
    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        while let Some(command) = commands.recv().await {
            let chat = match &command {
                Command::Edit(c, ..)
                | Command::Enqueue(c, ..)
                | Command::Ids(c, _)
                | Command::Refresh(c)
                | Command::Resume(c)
                | Command::Remove(c, _)
                | Command::Retry(c, _)
                | Command::Steer(c, _)
                | Command::Respond(c, _)
                | Command::Event(c, _)
                | Command::Finished(c, _)
                | Command::SteeringResult(c, ..) => *c,
            };
            if let Err(error) = self.handle(command).await {
                let reason = error.to_string();
                let _ = self.writes.pause_queue(chat, Some(&reason)).await;
                let _ = self.publish(chat).await;
                let _ = self.output.send(QueueEvent::Error(chat, error));
            }
        }
    }
    async fn handle(&mut self, command: Command) -> Result<(), QueueError> {
        match command {
            Command::Enqueue(chat, draft, reply) => {
                let result = self
                    .enqueuer
                    .enqueue(&QueuedMessage {
                        id: Uuid::new_v4(),
                        chat_id: chat,
                        draft,
                        state: "pending".into(),
                        error: None,
                    })
                    .await;
                let saved = result.is_ok();
                let _ = reply.send(result);
                if saved {
                    self.publish(chat).await?;
                    self.pump(chat).await?;
                }
            }
            Command::Ids(chat, reply) => {
                let result = self
                    .queries
                    .queue(chat)
                    .await
                    .map(|q| q.entries.into_iter().map(|e| e.id).collect())
                    .map_err(QueueError::from_source)
                    .map_err(ListQueuedIdsError::from);
                let _ = reply.send(result);
            }
            Command::Refresh(chat) => self.publish(chat).await?,
            Command::Resume(chat) => {
                let queue = self
                    .queries
                    .queue(chat)
                    .await
                    .map_err(QueueError::from_source)?;
                if queue.entries.iter().any(|e| e.state == "uncertain") {
                    return Err(
                        "Review uncertain entries and choose Retry or Remove before resuming."
                            .into(),
                    );
                }
                for entry in queue.entries.iter().filter(|e| e.state == "failed") {
                    self.writes
                        .transition(entry.id, "failed", "pending", None)
                        .await
                        .map_err(QueueError::from_source)?;
                }
                self.writes
                    .pause_queue(chat, None)
                    .await
                    .map_err(QueueError::from_source)?;
                self.publish(chat).await?;
                self.pump(chat).await?;
            }
            Command::Edit(chat, id, reply) => {
                let result = self.writes.edit_queued(chat,id).await.map_err(QueueError::from_source).and_then(|d| d.ok_or_else(|| QueueError::Policy("The entry is sending, or the composer already has a draft. Save or clear that draft first.".into())));
                let _ = reply.send(result);
                self.publish(chat).await?;
            }
            Command::Remove(chat, id) => {
                self.writes
                    .remove_queued(chat, id, false)
                    .await
                    .map_err(QueueError::from_source)?;
                self.publish(chat).await?;
            }
            Command::Retry(chat, id) => {
                if !self
                    .queries
                    .queue(chat)
                    .await
                    .map_err(QueueError::from_source)?
                    .entries
                    .iter()
                    .any(|e| e.id == id && e.state == "uncertain")
                {
                    return Ok(());
                }
                self.writes
                    .transition(id, "uncertain", "pending", None)
                    .await
                    .map_err(QueueError::from_source)?;
                self.publish(chat).await?;
            }
            Command::Respond(chat, response) => {
                if let Some(run) = self.running.get_mut(&chat) {
                    if run.pending_request.as_deref() != Some(response.id.as_str()) {
                        return Err("This is not the pending provider request.".into());
                    }
                    run.responses
                        .send(response)
                        .map_err(|_| "The provider is no longer waiting".to_string())?;
                    run.waiting = false;
                    run.pending_request = None;
                }
                self.publish(chat).await?;
            }
            Command::Event(chat, event) => {
                if let ConversationEvent::SteeringAvailable { available, .. } = &event {
                    if let Some(run) = self.running.get_mut(&chat) {
                        run.steering_available = *available;
                    }
                    self.publish(chat).await?;
                }
                if let ConversationEvent::Interaction { interaction, .. } = &event
                    && let Some(run) = self.running.get_mut(&chat)
                {
                    run.waiting = true;
                    run.pending_request = Some(interaction.id.clone());
                }
                if matches!(event, ConversationEvent::Interaction { .. }) {
                    self.publish(chat).await?;
                }
                let mut queue_changed = false;
                if let ConversationEvent::UserMessage(message) = &event {
                    if let Some(run) = self.running.get_mut(&chat)
                        && run.entry == message.id
                    {
                        run.entry_delivered = true;
                        queue_changed = true;
                    }
                    let queue = self
                        .queries
                        .queue(chat)
                        .await
                        .map_err(QueueError::from_source)?;
                    if queue.entries.iter().any(|e| {
                        e.id == message.id && matches!(e.state.as_str(), "steering" | "uncertain")
                    }) {
                        self.writes
                            .remove_queued(chat, message.id, true)
                            .await
                            .map_err(QueueError::from_source)?;
                        if let Some(run) = self.running.get_mut(&chat) {
                            run.has_steered = true;
                        }
                        queue_changed = true;
                    }
                }
                let _ = self.output.send(QueueEvent::Conversation(event));
                if queue_changed {
                    self.publish(chat).await?;
                }
            }
            Command::Finished(chat, result) => {
                if let Some(run) = self.running.remove(&chat) {
                    if result.is_ok() || run.has_steered {
                        self.writes
                            .remove_queued(chat, run.entry, true)
                            .await
                            .map_err(QueueError::from_source)?;
                    } else {
                        let result_error = result.as_ref().err().map(ToString::to_string);
                        self.writes
                            .transition(run.entry, "dispatching", "failed", result_error.as_deref())
                            .await
                            .map_err(QueueError::from_source)?;
                        let reason = result_error.as_deref().unwrap_or(
                            "The previous response failed. Review it, then Resume to retry.",
                        );
                        self.writes
                            .pause_queue(chat, Some(reason))
                            .await
                            .map_err(QueueError::from_source)?;
                    }
                    if result.is_err() && run.has_steered {
                        self.writes.pause_queue(chat, Some("The previous response failed. Review the conversation before resuming.")).await.map_err(QueueError::from_source)?;
                    }
                    let queue = self
                        .queries
                        .queue(chat)
                        .await
                        .map_err(QueueError::from_source)?;
                    for entry in queue.entries.iter().filter(|e| e.state == "steering") {
                        self.writes.transition(entry.id, "steering", "uncertain", Some("Provider delivery could not be confirmed. Check the conversation before Retry or Remove.")).await.map_err(QueueError::from_source)?;
                        self.writes
                            .pause_queue(chat, Some("A steered message has uncertain delivery."))
                            .await
                            .map_err(QueueError::from_source)?;
                    }
                }
                let _ = self.output.send(QueueEvent::Finished(chat, result));
                self.publish(chat).await?;
                self.pump(chat).await?;
            }
            Command::Steer(chat, id) => {
                self.service
                    .check_provider_enabled(chat)
                    .await
                    .map_err(QueueError::from_source)?;
                let Some(run) = self.running.get(&chat) else {
                    return Ok(());
                };
                if run.waiting {
                    return Err("Answer the pending request before steering.".into());
                }
                if !run.steering_available {
                    return Err("This provider could not verify steering support. The message remains queued.".into());
                }
                let queue = self
                    .queries
                    .queue(chat)
                    .await
                    .map_err(QueueError::from_source)?;
                if queue.paused.is_some() {
                    return Err("Resume the queue before steering.".into());
                }
                let Some(entry) = queue
                    .entries
                    .iter()
                    .find(|e| e.id == id && e.state == "pending")
                else {
                    return Ok(());
                };
                if let Err(error) = self
                    .service
                    .validate_queued_skills(chat, &entry.draft)
                    .await
                {
                    self.writes
                        .pause_queue(chat, Some(&error.to_string()))
                        .await
                        .map_err(QueueError::from_source)?;
                    self.publish(chat).await?;
                    return Ok(());
                }
                if !self
                    .writes
                    .transition(id, "pending", "steering", None)
                    .await
                    .map_err(QueueError::from_source)?
                {
                    return Ok(());
                }
                let (tx, rx) = oneshot::channel();
                let input = SteerInput {
                    id,
                    skills: entry.draft.skills.clone(),
                    prompt: entry.draft.text.clone(),
                    attachments: entry.draft.attachments.clone(),
                };
                if run
                    .controls
                    .send(SteerCommand { input, result: tx })
                    .is_err()
                {
                    self.writes
                        .transition(id, "steering", "pending", None)
                        .await
                        .map_err(QueueError::from_source)?;
                    return Ok(());
                }
                let sender = self.sender.clone();
                tokio::spawn(async move {
                    let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx).await;
                    let outcome = match result { Ok(Ok(result)) => result, _ => SteerOutcome::Uncertain("Provider did not confirm delivery. Check the conversation before Retry or Remove.".into()) };
                    let _ = sender.send(Command::SteeringResult(chat, id, outcome));
                });
                self.publish(chat).await?;
            }
            Command::SteeringResult(chat, id, outcome) => {
                match outcome {
                    SteerOutcome::Accepted => {} // Removal follows persistence of the UserMessage event.
                    SteerOutcome::NotRunning => {
                        let queue = self
                            .queries
                            .queue(chat)
                            .await
                            .map_err(QueueError::from_source)?;
                        if queue
                            .entries
                            .iter()
                            .any(|e| e.id == id && e.state == "uncertain")
                        {
                            self.writes
                                .transition(id, "uncertain", "pending", None)
                                .await
                                .map_err(QueueError::from_source)?;
                            if queue.paused.as_deref()
                                == Some("A steered message has uncertain delivery.")
                            {
                                self.writes
                                    .pause_queue(chat, None)
                                    .await
                                    .map_err(QueueError::from_source)?;
                            }
                        }
                        self.writes
                            .transition(id, "steering", "pending", None)
                            .await
                            .map_err(QueueError::from_source)?;
                    }
                    SteerOutcome::Rejected(error) => {
                        self.writes
                            .pause_queue(chat, Some(&error))
                            .await
                            .map_err(QueueError::from_source)?;
                        self.writes
                            .transition(id, "steering", "pending", Some(&error))
                            .await
                            .map_err(QueueError::from_source)?;
                        let _ = self.output.send(QueueEvent::Error(chat, error.into()));
                    }
                    SteerOutcome::Uncertain(error) => {
                        if self
                            .writes
                            .transition(id, "steering", "uncertain", Some(&error))
                            .await
                            .map_err(QueueError::from_source)?
                        {
                            self.writes
                                .pause_queue(chat, Some(&error))
                                .await
                                .map_err(QueueError::from_source)?;
                        }
                    }
                }
                self.publish(chat).await?;
                self.pump(chat).await?;
            }
        }
        Ok(())
    }
}
