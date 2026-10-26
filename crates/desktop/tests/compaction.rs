use async_trait::async_trait;
use smaller_talk_application::*;
use smaller_talk_database::*;
use smaller_talk_domain::*;
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
struct Agent {
    compactions: mpsc::UnboundedSender<(
        CompactionRequest,
        oneshot::Sender<Result<CompactionOutcome, String>>,
    )>,
    turns: mpsc::UnboundedSender<String>,
}
#[async_trait]
impl ExecutionAdapter for Agent {
    async fn compact(
        &self,
        request: CompactionRequest,
    ) -> Result<CompactionOutcome, ExecutionError> {
        let (tx, rx) = oneshot::channel();
        self.compactions.send((request, tx)).unwrap();
        rx.await.unwrap().map_err(ExecutionError::new)
    }
    async fn capabilities(
        &self,
        provider: Provider,
    ) -> Result<ProviderCapabilities, ExecutionError> {
        Ok(ProviderCapabilities {
            provider,
            models: vec![ModelCapability {
                id: "test".into(),
                label: "Test".into(),
                efforts: vec![],
                default_effort: None,
                images: false,
                is_default: true,
            }],
        })
    }
    async fn execute(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError> {
        self.turns.send(request.prompt).unwrap();
        let _ = events.send(ExecutionEvent::TextDelta("Follow-up".into()));
        Ok(())
    }
}
struct Setup {
    db: Arc<SqliteChatStore>,
    service: Arc<ConversationService>,
    queue: QueueCoordinator,
    chat: Uuid,
    compactions: mpsc::UnboundedReceiver<(
        CompactionRequest,
        oneshot::Sender<Result<CompactionOutcome, String>>,
    )>,
    turns: mpsc::UnboundedReceiver<String>,
    events: mpsc::UnboundedReceiver<QueueEvent>,
    _temp: tempfile::TempDir,
}
async fn setup() -> Setup {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("chats.sqlite3");
    create_empty_database(&path).await.unwrap();
    migrate_existing(&path).await.unwrap();
    let db = Arc::new(SqliteChatStore::new(open_existing(&path).await.unwrap()));
    let chat = ChatService::new(db.clone(), db.clone())
        .create_empty_chat()
        .await
        .unwrap()
        .id;
    db.save_execution_binding(
        chat,
        ExecutionBinding {
            adapter_key: "codex".into(),
            native_session_id: "existing-native".into(),
            instructions_fingerprint: None,
        },
    )
    .await
    .unwrap();
    let (compactions_tx, compactions) = mpsc::unbounded_channel();
    let (turns_tx, turns) = mpsc::unbounded_channel();
    let service = Arc::new(
        ConversationService::new(
            db.clone(),
            db.clone(),
            db.clone(),
            db.clone(),
            Arc::new(Agent {
                compactions: compactions_tx,
                turns: turns_tx,
            }),
            temp.path().into(),
        )
        .with_workspace(db.clone(), db.clone()),
    );
    let (tx, events) = mpsc::unbounded_channel();
    let queue = QueueCoordinator::start(
        db.clone(),
        db.clone(),
        Arc::new(EnqueueService::<SqliteUnitOfWork>::new(
            db.clone(),
            db.clone(),
        )),
        service.clone(),
        tx,
    );
    Setup {
        db,
        service,
        queue,
        chat,
        compactions,
        turns,
        events,
        _temp: temp,
    }
}
async fn event(s: &mut Setup, condition: impl Fn(&QueueEvent) -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if condition(&s.events.recv().await.unwrap()) {
                return;
            }
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn compaction_serializes_queue_and_archive_without_adding_a_message() {
    let mut s = setup().await;
    s.queue.compact(s.chat).await.unwrap();
    let (request, finish) = s.compactions.recv().await.unwrap();
    assert_eq!(request.binding.native_session_id, "existing-native");
    assert!(s.db.list_messages(s.chat).await.unwrap().is_empty());
    assert!(matches!(
        s.queue.set_archived(s.chat, true).await,
        Err(ArchiveChatError::Busy)
    ));
    assert!(s.queue.compact(s.chat).await.is_err());
    s.queue
        .enqueue(EnqueueInput {
            chat_id: s.chat,
            draft: Draft {
                text: "after compaction".into(),
                ..Default::default()
            },
        })
        .await
        .unwrap();
    assert!(s.turns.try_recv().is_err());
    finish.send(Ok(CompactionOutcome::Compacted)).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), s.turns.recv())
            .await
            .unwrap()
            .unwrap(),
        "after compaction"
    );
    event(&mut s, |e| matches!(e, QueueEvent::Finished(_, Ok(())))).await;
    let messages = s.db.list_messages(s.chat).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert!(messages.iter().all(|m| m.body != "/compact"));
    s.queue.set_archived(s.chat, true).await.unwrap();
}
#[tokio::test]
async fn failed_compaction_holds_queued_input_for_review() {
    let mut s = setup().await;
    s.queue.compact(s.chat).await.unwrap();
    let (_, finish) = s.compactions.recv().await.unwrap();
    s.queue
        .enqueue(EnqueueInput {
            chat_id: s.chat,
            draft: Draft {
                text: "preserve queued".into(),
                ..Default::default()
            },
        })
        .await
        .unwrap();
    finish.send(Err("Native compaction failed".into())).unwrap();
    event(&mut s, |e| {
        matches!(e, QueueEvent::CompactionFinished(_, Err(_)))
    })
    .await;
    let queue = s.db.queue(s.chat).await.unwrap();
    assert_eq!(queue.entries.len(), 1);
    assert!(queue.paused.unwrap().contains("compaction failed"));
    assert!(s.turns.try_recv().is_err());
    assert!(s.db.list_messages(s.chat).await.unwrap().is_empty());
}
#[tokio::test]
async fn compaction_rejects_stale_binding_archived_and_unsent_chats() {
    let s = setup().await;
    let prefs = ChatPreferences {
        new_session: true,
        ..Default::default()
    };
    s.db.save_preferences(s.chat, &prefs).await.unwrap();
    assert!(
        s.service
            .compact_conversation(s.chat)
            .await
            .unwrap_err()
            .to_string()
            .contains("normal message")
    );
    s.db.save_preferences(
        s.chat,
        &ChatPreferences {
            execution: ExecutionSettings {
                provider: Provider::Claude,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(s.service.compact_conversation(s.chat).await.is_err());
    s.queue.set_archived(s.chat, true).await.unwrap();
    assert!(
        s.service
            .compact_conversation(s.chat)
            .await
            .unwrap_err()
            .to_string()
            .contains("Restore")
    );
    assert!(
        s.service
            .compact_conversation(Uuid::new_v4())
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "Requires authenticated Codex and Claude CLIs; set SMALLER_TALK_LIVE=1"]
async fn native_compaction_preserves_context_and_local_history_for_both_providers() {
    assert_eq!(std::env::var("SMALLER_TALK_LIVE").as_deref(), Ok("1"));
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("live.sqlite3");
    create_empty_database(&path).await.unwrap();
    migrate_existing(&path).await.unwrap();
    let db = Arc::new(SqliteChatStore::new(open_existing(&path).await.unwrap()));
    let router = smaller_talk_infrastructure::AgentRouter::new(
        smaller_talk_infrastructure::CodexAdapter::new(
            std::env::var("SMALLER_TALK_CODEX_BIN").unwrap_or_else(|_| "codex".into()),
        ),
        smaller_talk_infrastructure::ClaudeAdapter::new(
            std::env::var("SMALLER_TALK_CLAUDE_BIN").unwrap_or_else(|_| "claude".into()),
        ),
    );
    let service = Arc::new(
        ConversationService::new(
            db.clone(),
            db.clone(),
            db.clone(),
            db.clone(),
            router.clone(),
            temp.path().into(),
        )
        .with_workspace(db.clone(), db.clone()),
    );
    for provider in [Provider::Codex, Provider::Claude] {
        let catalog = router.capabilities(provider).await.unwrap();
        let model = catalog
            .models
            .iter()
            .find(|m| {
                m.id.contains(if provider == Provider::Codex {
                    "luna"
                } else {
                    "haiku"
                })
            })
            .or_else(|| catalog.models.iter().find(|m| m.is_default))
            .unwrap();
        let chat = ChatService::new(db.clone(), db.clone())
            .create_empty_chat()
            .await
            .unwrap();
        db.save_preferences(
            chat.id,
            &ChatPreferences {
                execution: ExecutionSettings {
                    provider,
                    model: Some(model.id.clone()),
                    effort: model.efforts.iter().find(|e| *e == "low").cloned(),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let notes=(0..24).map(|n|format!("Synthetic note {n}: the sample workshop contains a green chair, a blue desk, and a silver lamp. These are fictional testing facts.\n")).collect::<String>();
        let first = format!(
            "Do not use tools or change files. Remember the exact test token st013-silver-53 for later. Here are fictional notes for testing compaction:\n{notes}\nReply with the token only."
        );
        live_turn(service.clone(), chat.id, &first).await;
        live_turn(service.clone(),chat.id,"Do not use tools. Repeat the test token and the three furniture colors in one sentence.").await;
        let before = db.list_messages(chat.id).await.unwrap();
        let binding = db.find_chat(chat.id).await.unwrap().unwrap().binding;
        let result = service.compact_conversation(chat.id).await.unwrap();
        println!("{} / {}: {:?}", provider.label(), model.id, result);
        assert_eq!(result, CompactionOutcome::Compacted);
        assert_eq!(db.list_messages(chat.id).await.unwrap(), before);
        assert_eq!(
            db.find_chat(chat.id).await.unwrap().unwrap().binding,
            binding
        );
        let answer=live_turn(service.clone(),chat.id,"Do not use tools. What is the exact test token I asked you to remember? Reply with the token only.").await;
        assert!(
            answer.contains("st013-silver-53"),
            "{}: {answer}",
            provider.label()
        );
        println!(
            "{} post-compaction continuation retained the token",
            provider.label()
        );
    }
}
async fn live_turn(service: Arc<ConversationService>, chat_id: Uuid, prompt: &str) -> String {
    let input = SendTurnInput {
        chat_id,
        prompt: prompt.into(),
        skills: vec![],
        attachments: vec![],
        retry_of: None,
    };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (_answers, answers) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move { service.send_turn(input, tx, answers).await });
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(120), async {
        while let Some(event) = rx.recv().await {
            match event {
                ConversationEvent::AssistantDelta { delta, .. } => text.push_str(&delta),
                ConversationEvent::AssistantSnapshot { text: full, .. } => text = full,
                _ => {}
            }
        }
        task.await.unwrap().unwrap();
    })
    .await
    .unwrap();
    text
}
