use async_trait::async_trait;
use smaller_talk_application::*;
use smaller_talk_database::*;
use smaller_talk_domain::*;
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

struct Run {
    request: ExecutionRequest,
    events: mpsc::UnboundedSender<ExecutionEvent>,
    controls: mpsc::UnboundedReceiver<SteerCommand>,
    responses: mpsc::UnboundedReceiver<InteractionResponse>,
    finish: oneshot::Sender<Result<(), String>>,
}
struct Agent(mpsc::UnboundedSender<Run>);
#[async_trait]
impl ExecutionAdapter for Agent {
    async fn capabilities(
        &self,
        provider: Provider,
    ) -> Result<ProviderCapabilities, ExecutionError> {
        Ok(ProviderCapabilities {
            provider,
            models: vec![ModelCapability {
                id: "test-model".into(),
                label: "Test".into(),
                efforts: vec![],
                default_effort: None,
                images: true,
                is_default: true,
            }],
        })
    }
    async fn execute(
        &self,
        _: ExecutionRequest,
        _: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError> {
        unreachable!()
    }
    async fn execute_controlled(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
        responses: mpsc::UnboundedReceiver<InteractionResponse>,
        controls: mpsc::UnboundedReceiver<SteerCommand>,
    ) -> Result<(), ExecutionError> {
        let (finish, receiver) = oneshot::channel();
        self.0
            .send(Run {
                request,
                events,
                controls,
                responses,
                finish,
            })
            .unwrap_or_else(|_| panic!("test stopped"));
        receiver.await.unwrap().map_err(ExecutionError::new)
    }
}
struct Setup {
    db: Arc<SqliteChatStore>,
    queue: QueueCoordinator,
    runs: mpsc::UnboundedReceiver<Run>,
    events: mpsc::UnboundedReceiver<QueueEvent>,
    chats: ChatService,
    _temp: tempfile::TempDir,
}
async fn setup() -> Setup {
    setup_with_skills(None).await
}
async fn setup_with_skills(catalog: Option<Arc<dyn SkillCatalog>>) -> Setup {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("chats.sqlite3");
    create_empty_database(&path).await.unwrap();
    migrate_existing(&path).await.unwrap();
    let db = Arc::new(SqliteChatStore::new(open_existing(&path).await.unwrap()));
    let (tx, runs) = mpsc::unbounded_channel();
    let mut service = ConversationService::new(
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        Arc::new(Agent(tx)),
        temp.path().into(),
    )
    .with_workspace(db.clone(), db.clone())
    .with_steered_input_writes(db.clone())
    .with_settings(db.clone())
    .with_instructions(
        Arc::new(InstructionService::new(db.clone(), db.clone(), db.clone())),
        db.clone(),
    )
    .with_display(db.clone());
    if let Some(catalog) = catalog {
        service = service.with_skills(Arc::new(SkillService::new(catalog)), db.clone());
    }
    let service = Arc::new(service);
    let (output, events) = mpsc::unbounded_channel();
    let queue = QueueCoordinator::start(
        db.clone(),
        db.clone(),
        Arc::new(EnqueueService::<SqliteUnitOfWork>::new(
            db.clone(),
            db.clone(),
        )),
        service,
        output,
    );
    Setup {
        chats: ChatService::new(db.clone(), db.clone()),
        db,
        queue,
        runs,
        events,
        _temp: temp,
    }
}

#[tokio::test]
async fn accepted_steering_persistence_rolls_back_when_turn_link_fails() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("initial"),
        })
        .await
        .unwrap();
    let mut run = next_run(&mut s).await;
    run.events
        .send(ExecutionEvent::TextDelta("Before".into()))
        .unwrap();
    run.events
        .send(ExecutionEvent::SteeringAvailable(true))
        .unwrap();
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("correction"),
        })
        .await
        .unwrap();
    event(
        &mut s,
        |e| matches!(e, QueueEvent::Snapshot(_, q) if q.steer_available),
    )
    .await;
    let id = s.db.queue(chat).await.unwrap().entries.last().unwrap().id;
    sqlx::query("CREATE TRIGGER fail_turn_input BEFORE INSERT ON turn_inputs BEGIN SELECT RAISE(FAIL, 'injected link failure'); END")
        .execute(s.db.pool()).await.unwrap();
    s.queue.steer(SteerQueueInput {
        chat_id: chat,
        entry_id: id,
    });
    let command = run.controls.recv().await.unwrap();
    run.events
        .send(ExecutionEvent::InputAccepted(command.input))
        .unwrap();
    command.result.send(SteerOutcome::Accepted).unwrap();
    event(&mut s, |e| matches!(e, QueueEvent::Finished(_, Err(_)))).await;
    let history = s.db.list_messages(chat).await.unwrap();
    assert_eq!(history.iter().filter(|m| m.body == "initial").count(), 1);
    assert!(!history.iter().any(|m| m.body == "correction"));
}

#[tokio::test]
async fn retry_after_restart_finds_original_input_before_accepted_steering() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    let initial = QueuedMessage {
        id: Uuid::new_v4(),
        chat_id: chat,
        draft: draft("initial"),
        state: "pending".into(),
        error: None,
    };
    EnqueueService::<SqliteUnitOfWork>::new(s.db.clone(), s.db.clone())
        .enqueue(&initial)
        .await
        .unwrap();
    s.db.transition(initial.id, "pending", "dispatching", None)
        .await
        .unwrap();
    let steering_id = Uuid::new_v4();
    for (id, text) in [(initial.id, "initial"), (steering_id, "accepted steering")] {
        s.db.append_message(NewMessage {
            id,
            chat_id: chat,
            role: MessageRole::User,
            body: text.into(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let turn_id = Uuid::new_v4();
    s.db.save_turn(&TurnRecord {
        instructions: Default::default(),
        id: turn_id,
        chat_id: chat,
        user_message_id: initial.id,
        settings: ExecutionSettings {
            provider: Provider::Codex,
            model: Some("test-model".into()),
            effort: None,
            plan_mode: false,
        },
        workspace: s._temp.path().into(),
        status: "running".into(),
        error: None,
    })
    .await
    .unwrap();
    s.db.save_turn_input(turn_id, steering_id).await.unwrap();
    s.db.recover_interrupted_turns().await.unwrap();
    s.db.recover_queues().await.unwrap();
    s.queue.retry(RetryQueueInput {
        chat_id: chat,
        entry_id: initial.id,
    });
    s.queue.resume(chat);
    loop {
        let queued_event = tokio::time::timeout(Duration::from_secs(5), s.events.recv())
            .await
            .unwrap()
            .unwrap();
        match queued_event {
            QueueEvent::Error(_, error) => panic!("queue recovery failed: {error}"),
            QueueEvent::Started(id) if id == chat => break,
            _ => {}
        }
    }
    let run = next_run(&mut s).await;
    assert_eq!(run.request.prompt, "initial");
    let recovered_prompt = run.request.full_prompt();
    assert!(recovered_prompt.contains("Confirmed user steering accepted"));
    assert!(recovered_prompt.contains(&steering_id.to_string()));
    assert!(recovered_prompt.contains("accepted steering"));
    run.events
        .send(ExecutionEvent::TextDelta("recovered answer".into()))
        .unwrap();
    run.finish.send(Ok(())).unwrap();
    drop(run.events);
    event(&mut s, |e| matches!(e, QueueEvent::Finished(_, Ok(())))).await;
    let history = s.db.list_messages(chat).await.unwrap();
    assert_eq!(history.iter().filter(|m| m.id == initial.id).count(), 1);
    assert!(history.iter().any(|m| m.body == "accepted steering"));
}
fn draft(text: &str) -> Draft {
    Draft {
        skills: Vec::new(),
        text: text.into(),
        ..Default::default()
    }
}
async fn next_run(s: &mut Setup) -> Run {
    tokio::time::timeout(Duration::from_secs(5), s.runs.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn event(s: &mut Setup, predicate: impl Fn(&QueueEvent) -> bool) {
    loop {
        let e = tokio::time::timeout(Duration::from_secs(5), s.events.recv())
            .await
            .unwrap()
            .unwrap();
        if predicate(&e) {
            break;
        }
    }
}
fn finish(run: Run, result: Result<(), String>) {
    run.events
        .send(ExecutionEvent::TextDelta("Answer".into()))
        .unwrap();
    drop(run.events);
    run.finish.send(result).unwrap();
}

#[tokio::test]
async fn queued_turns_use_new_instructions_while_steering_keeps_the_active_snapshot() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    let mut settings = AppSettings {
        instructions: CustomInstructions {
            enabled: true,
            text: "Initial rules".into(),
        },
        ..Default::default()
    };
    s.db.save_app_settings(&settings).await.unwrap();
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("initial"),
        })
        .await
        .unwrap();
    let mut run = next_run(&mut s).await;
    let original_snapshot = run.request.instructions.clone();
    assert!(original_snapshot.text.contains("Initial rules"));
    run.events
        .send(ExecutionEvent::SteeringAvailable(true))
        .unwrap();
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("follow up"),
        })
        .await
        .unwrap();
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("steered correction"),
        })
        .await
        .unwrap();
    event(&mut s, |e| matches!(e, QueueEvent::Snapshot(_, q) if q.steer_available && q.entries.iter().any(|e| e.draft.text == "steered correction"))).await;
    settings.instructions.text = "New rules".into();
    s.db.save_app_settings(&settings).await.unwrap();
    let steering =
        s.db.queue(chat)
            .await
            .unwrap()
            .entries
            .into_iter()
            .find(|e| e.draft.text == "steered correction")
            .unwrap()
            .id;
    s.queue.steer(SteerQueueInput {
        chat_id: chat,
        entry_id: steering,
    });
    let command = run.controls.recv().await.unwrap();
    run.events
        .send(ExecutionEvent::InputAccepted(command.input))
        .unwrap();
    command.result.send(SteerOutcome::Accepted).unwrap();
    event(
        &mut s,
        |e| matches!(e, QueueEvent::Snapshot(_, q) if !q.entries.iter().any(|e| e.id == steering)),
    )
    .await;
    assert_eq!(
        s.db.message_instructions(chat, steering)
            .await
            .unwrap()
            .unwrap(),
        original_snapshot
    );
    finish(run, Ok(()));
    let follow = next_run(&mut s).await;
    assert_eq!(follow.request.prompt, "follow up");
    assert!(follow.request.instructions.text.contains("New rules"));
    finish(follow, Ok(()));
}

#[tokio::test]
async fn fifo_and_other_chats_continue_without_a_view() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    let other = s.chats.create_empty_chat().await.unwrap().id;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("first"),
        })
        .await
        .unwrap();
    let first = next_run(&mut s).await;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("second"),
        })
        .await
        .unwrap();
    assert_eq!(s.db.list_messages(chat).await.unwrap().len(), 1);
    s.queue
        .enqueue(EnqueueInput {
            chat_id: other,
            draft: draft("independent"),
        })
        .await
        .unwrap();
    let independent = next_run(&mut s).await;
    assert_eq!(independent.request.chat_id, other);
    assert!(s.runs.try_recv().is_err());
    finish(first, Ok(()));
    let second = next_run(&mut s).await;
    assert_eq!(second.request.prompt, "second");
    finish(second, Ok(()));
    finish(independent, Ok(()));
    event(
        &mut s,
        |e| matches!(e, QueueEvent::Snapshot(c, q) if *c == chat && q.entries.is_empty()),
    )
    .await;
    let history = s.db.list_messages(chat).await.unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
}

#[tokio::test]
async fn steering_preserves_segments_and_waits_for_acknowledgement() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("initial"),
        })
        .await
        .unwrap();
    let mut run = next_run(&mut s).await;
    run.events
        .send(ExecutionEvent::TextDelta("Before".into()))
        .unwrap();
    run.events
        .send(ExecutionEvent::SteeringAvailable(true))
        .unwrap();
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("correction"),
        })
        .await
        .unwrap();
    event(&mut s, |e| matches!(e, QueueEvent::Snapshot(_, q) if q.steer_available && q.entries.iter().any(|e| e.draft.text == "correction"))).await;
    let id = s.db.queue(chat).await.unwrap().entries.last().unwrap().id;
    s.queue.steer(SteerQueueInput {
        chat_id: chat,
        entry_id: id,
    });
    let command = tokio::time::timeout(Duration::from_secs(5), run.controls.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        s.db.queue(chat)
            .await
            .unwrap()
            .entries
            .last()
            .unwrap()
            .state,
        "steering"
    );
    run.events
        .send(ExecutionEvent::InputAccepted(command.input))
        .unwrap();
    command.result.send(SteerOutcome::Accepted).unwrap();
    run.events
        .send(ExecutionEvent::TextDelta("After".into()))
        .unwrap();
    drop(run.events);
    run.finish.send(Ok(())).unwrap();
    event(
        &mut s,
        |e| matches!(e, QueueEvent::Finished(c, Ok(())) if *c == chat),
    )
    .await;
    let history = s.db.list_messages(chat).await.unwrap();
    assert_eq!(
        history.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        vec!["initial", "Before", "correction", "After"]
    );
    assert_eq!(history[2].id, id);
    assert!(s.db.queue(chat).await.unwrap().entries.is_empty());
    let association: i64 =
        sqlx::query_scalar("SELECT count(*) FROM turn_inputs WHERE message_id = ?")
            .bind(id.to_string())
            .fetch_one(s.db.pool())
            .await
            .unwrap();
    assert_eq!(association, 1);
}

#[tokio::test]
async fn durable_transfer_preserves_newer_drafts_and_attachment_ownership() {
    let s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    let mut queued = draft("snapshot");
    let file = s._temp.path().join("owned.png");
    std::fs::write(&file, b"image").unwrap();
    queued.attachments.push(Attachment {
        id: "image".into(),
        name: "image.png".into(),
        path: file.clone(),
        mime_type: "image/png".into(),
        byte_len: 5,
        preview_path: None,
    });
    s.db.save_draft(chat, &queued).await.unwrap();
    let entry = QueuedMessage {
        id: Uuid::new_v4(),
        chat_id: chat,
        draft: queued.clone(),
        state: "pending".into(),
        error: None,
    };
    EnqueueService::<SqliteUnitOfWork>::new(s.db.clone(), s.db.clone())
        .enqueue(&entry)
        .await
        .unwrap();
    assert_eq!(s.db.draft(chat).await.unwrap(), Draft::default());
    assert!(s.db.attachment_references().await.unwrap().contains(&file));
    s.db.save_draft(chat, &draft("newer draft")).await.unwrap();
    let second = QueuedMessage {
        id: Uuid::new_v4(),
        ..entry
    };
    EnqueueService::<SqliteUnitOfWork>::new(s.db.clone(), s.db.clone())
        .enqueue(&second)
        .await
        .unwrap();
    assert_eq!(s.db.draft(chat).await.unwrap().text, "newer draft");
    s.db.transition(second.id, "pending", "steering", None)
        .await
        .unwrap();
    s.db.recover_queues().await.unwrap();
    let restored = s.db.queue(chat).await.unwrap();
    assert!(restored.paused.is_some());
    assert_eq!(restored.entries[1].state, "uncertain");
    s.chats.delete_chat(chat).await.unwrap();
    assert!(s.db.queue(chat).await.unwrap().entries.is_empty());
    assert!(!s.db.attachment_references().await.unwrap().contains(&file));
    assert!(file.exists());
}

#[tokio::test]
async fn errors_pause_until_resume_and_retry_reuses_message_id() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("fail once"),
        })
        .await
        .unwrap();
    let run = next_run(&mut s).await;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("wait"),
        })
        .await
        .unwrap();
    finish(run, Err("Disconnected".into()));
    event(
        &mut s,
        |e| matches!(e, QueueEvent::Snapshot(_, q) if q.paused.is_some()),
    )
    .await;
    assert!(s.runs.try_recv().is_err());
    let first_id = s.db.list_messages(chat).await.unwrap()[0].id;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("also waits"),
        })
        .await
        .unwrap();
    assert!(s.runs.try_recv().is_err());
    s.queue.resume(chat);
    let retry = next_run(&mut s).await;
    assert_eq!(retry.request.prompt, "fail once");
    finish(retry, Ok(()));
    let next = next_run(&mut s).await;
    assert_eq!(next.request.prompt, "wait");
    let history = s.db.list_messages(chat).await.unwrap();
    assert_eq!(history.iter().filter(|m| m.id == first_id).count(), 1);
    finish(next, Ok(()));
    let last = next_run(&mut s).await;
    finish(last, Ok(()));
}

#[tokio::test]
async fn pending_approval_blocks_steering_and_dispatch() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("plan"),
        })
        .await
        .unwrap();
    let mut run = next_run(&mut s).await;
    run.events
        .send(ExecutionEvent::SteeringAvailable(true))
        .unwrap();
    run.events
        .send(ExecutionEvent::Interaction(Interaction {
            id: "leave-plan".into(),
            title: "Implementation approval".into(),
            kind: InteractionKind::LeavePlan {
                detail: "Implement?".into(),
            },
        }))
        .unwrap();
    event(&mut s, |e| {
        matches!(
            e,
            QueueEvent::Conversation(ConversationEvent::Interaction { .. })
        )
    })
    .await;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("follow up"),
        })
        .await
        .unwrap();
    let id = s.db.queue(chat).await.unwrap().entries.last().unwrap().id;
    s.queue.steer(SteerQueueInput {
        chat_id: chat,
        entry_id: id,
    });
    event(&mut s, |e| matches!(e, QueueEvent::Error(_, _))).await;
    assert!(run.controls.try_recv().is_err());
    assert!(s.runs.try_recv().is_err());
    s.queue.respond(RespondQueueInput {
        chat_id: chat,
        response: InteractionResponse {
            id: "leave-plan".into(),
            approved: false,
            ..Default::default()
        },
    });
    assert!(!run.responses.recv().await.unwrap().approved);
    finish(run, Ok(()));
}

#[tokio::test]
async fn uncertain_delivery_requires_review_and_never_auto_replays() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    let uncertain = QueuedMessage {
        id: Uuid::new_v4(),
        chat_id: chat,
        draft: draft("possibly delivered"),
        state: "pending".into(),
        error: None,
    };
    EnqueueService::<SqliteUnitOfWork>::new(s.db.clone(), s.db.clone())
        .enqueue(&uncertain)
        .await
        .unwrap();
    s.db.transition(uncertain.id, "pending", "dispatching", None)
        .await
        .unwrap();
    s.db.recover_queues().await.unwrap();
    s.queue.resume(chat);
    event(&mut s, |e| matches!(e, QueueEvent::Error(_, _))).await;
    assert!(s.runs.try_recv().is_err());
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("later"),
        })
        .await
        .unwrap();
    assert!(s.runs.try_recv().is_err());
    s.queue.remove(RemoveQueueInput {
        chat_id: chat,
        entry_id: uncertain.id,
    });
    event(
        &mut s,
        |e| matches!(e, QueueEvent::Snapshot(_, q) if q.entries.len() == 1),
    )
    .await;
    s.queue.resume(chat);
    let run = next_run(&mut s).await;
    assert_eq!(run.request.prompt, "later");
    finish(run, Ok(()));
}

#[tokio::test]
async fn settings_are_resolved_at_dispatch_and_failed_writes_keep_the_draft() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    s.db.pause_queue(chat, Some("test hold")).await.unwrap();
    let queued = QueuedMessage {
        id: Uuid::new_v4(),
        chat_id: chat,
        draft: draft("held"),
        state: "pending".into(),
        error: None,
    };
    EnqueueService::<SqliteUnitOfWork>::new(s.db.clone(), s.db.clone())
        .enqueue(&queued)
        .await
        .unwrap();
    s.db.save_draft(chat, &queued.draft).await.unwrap();
    assert!(
        EnqueueService::<SqliteUnitOfWork>::new(s.db.clone(), s.db.clone())
            .enqueue(&queued)
            .await
            .is_err()
    );
    assert_eq!(s.db.draft(chat).await.unwrap().text, "held");
    let preferences = ChatPreferences {
        execution: ExecutionSettings {
            provider: Provider::Claude,
            model: Some("test-model".into()),
            plan_mode: true,
            ..Default::default()
        },
        ..Default::default()
    };
    WorkspaceService::new(s.db.clone(), s.db.clone())
        .save_preferences(chat, preferences)
        .await
        .unwrap();
    s.queue.resume(chat);
    let run = next_run(&mut s).await;
    assert_eq!(run.request.settings.provider, Provider::Claude);
    assert!(run.request.settings.plan_mode);
    finish(run, Ok(()));
}

#[tokio::test]
async fn unavailable_attachment_pauses_before_provider_execution() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    let mut input = draft("");
    input.attachments.push(Attachment {
        id: "missing".into(),
        name: "missing.txt".into(),
        path: s._temp.path().join("missing.txt"),
        mime_type: "text/plain".into(),
        byte_len: 1,
        preview_path: None,
    });
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: input,
        })
        .await
        .unwrap();
    event(
        &mut s,
        |e| matches!(e, QueueEvent::Snapshot(_, q) if q.paused.is_some()),
    )
    .await;
    assert!(s.runs.try_recv().is_err());
    let queue = s.db.queue(chat).await.unwrap();
    assert_eq!(queue.entries[0].state, "failed");
    assert_eq!(queue.entries[0].draft.attachments.len(), 1);
    assert!(s.db.list_messages(chat).await.unwrap().is_empty());
}

#[tokio::test]
async fn turn_completion_before_steer_delivery_returns_entry_to_normal_queue() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("initial"),
        })
        .await
        .unwrap();
    let mut run = next_run(&mut s).await;
    run.events
        .send(ExecutionEvent::SteeringAvailable(true))
        .unwrap();
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("next"),
        })
        .await
        .unwrap();
    event(&mut s, |e| matches!(e, QueueEvent::Snapshot(_, q) if q.steer_available && q.entries.iter().any(|e| e.draft.text == "next"))).await;
    let id = s.db.queue(chat).await.unwrap().entries.last().unwrap().id;
    s.queue.steer(SteerQueueInput {
        chat_id: chat,
        entry_id: id,
    });
    let command = run.controls.recv().await.unwrap();
    finish(run, Ok(()));
    event(&mut s, |e| matches!(e, QueueEvent::Snapshot(_, q) if q.entries.iter().any(|e| e.state == "uncertain"))).await;
    command.result.send(SteerOutcome::NotRunning).unwrap();
    let next = next_run(&mut s).await;
    assert_eq!(next.request.prompt, "next");
    finish(next, Ok(()));
}

#[tokio::test]
async fn unacknowledged_steer_pauses_remaining_entries_even_after_success() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("initial"),
        })
        .await
        .unwrap();
    let mut run = next_run(&mut s).await;
    run.events
        .send(ExecutionEvent::SteeringAvailable(true))
        .unwrap();
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("uncertain input"),
        })
        .await
        .unwrap();
    event(&mut s, |e| matches!(e, QueueEvent::Snapshot(_, q) if q.steer_available && q.entries.iter().any(|e| e.draft.text == "uncertain input"))).await;
    let id = s.db.queue(chat).await.unwrap().entries.last().unwrap().id;
    s.queue.steer(SteerQueueInput {
        chat_id: chat,
        entry_id: id,
    });
    let command = run.controls.recv().await.unwrap();
    command
        .result
        .send(SteerOutcome::Uncertain(
            "Connection lost after write".into(),
        ))
        .unwrap();
    event(
        &mut s,
        |e| matches!(e, QueueEvent::Snapshot(_, q) if q.paused.is_some()),
    )
    .await;
    finish(run, Ok(()));
    event(&mut s, |e| matches!(e, QueueEvent::Finished(_, Ok(())))).await;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("later"),
        })
        .await
        .unwrap();
    assert!(s.runs.try_recv().is_err());
    assert_eq!(
        s.db.queue(chat).await.unwrap().entries[0].state,
        "uncertain"
    );
}

#[tokio::test]
async fn disabling_a_provider_finishes_active_work_and_pauses_queued_delivery() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("active"),
        })
        .await
        .unwrap();
    let first = next_run(&mut s).await;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("preserved"),
        })
        .await
        .unwrap();
    let mut settings = AppSettings::default();
    settings
        .providers
        .entry(Provider::Codex)
        .or_default()
        .enabled = false;
    s.db.save_app_settings(&settings).await.unwrap();
    finish(first, Ok(()));
    event(
        &mut s,
        |e| matches!(e,QueueEvent::Snapshot(c,q) if *c==chat && q.paused.is_some()),
    )
    .await;
    assert!(s.runs.try_recv().is_err());
    let queue = s.db.queue(chat).await.unwrap();
    assert!(queue.entries.iter().any(|e| e.draft.text == "preserved"));
    assert_eq!(s.db.list_messages(chat).await.unwrap().len(), 2);
    settings
        .providers
        .entry(Provider::Codex)
        .or_default()
        .enabled = true;
    s.db.save_app_settings(&settings).await.unwrap();
    s.queue.resume(chat);
    let next = next_run(&mut s).await;
    assert_eq!(next.request.prompt, "preserved");
    finish(next, Ok(()));
}
#[tokio::test]
async fn final_answer_metadata_requires_success_and_a_verifiable_suffix() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("first"),
        })
        .await
        .unwrap();
    let run = next_run(&mut s).await;
    run.events
        .send(ExecutionEvent::TextDelta(
            "Interim status. Final answer.".into(),
        ))
        .unwrap();
    run.events
        .send(ExecutionEvent::FinalAnswerText("Final answer.".into()))
        .unwrap();
    drop(run.events);
    run.finish.send(Ok(())).unwrap();
    event(
        &mut s,
        |e| matches!(e,QueueEvent::Finished(c,Ok(())) if *c==chat),
    )
    .await;
    assert_eq!(
        s.db.final_answers(chat).await.unwrap()[0].text,
        "Final answer."
    );
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft("second"),
        })
        .await
        .unwrap();
    let run = next_run(&mut s).await;
    run.events
        .send(ExecutionEvent::TextDelta("Partial".into()))
        .unwrap();
    run.events
        .send(ExecutionEvent::FinalAnswerText("Partial".into()))
        .unwrap();
    drop(run.events);
    run.finish.send(Err("failed".into())).unwrap();
    event(
        &mut s,
        |e| matches!(e,QueueEvent::Finished(c,Err(_)) if *c==chat),
    )
    .await;
    assert_eq!(s.db.final_answers(chat).await.unwrap().len(), 1);
}

#[tokio::test]
async fn plans_and_unmatched_final_boundaries_never_become_rewrite_sources() {
    let mut s = setup().await;
    for (plan, text, final_text) in [
        (true, "A plan.", "A plan."),
        (false, "Saved text.", "Not the saved answer."),
    ] {
        let chat = s.chats.create_empty_chat().await.unwrap().id;
        WorkspaceService::new(s.db.clone(), s.db.clone())
            .save_preferences(
                chat,
                ChatPreferences {
                    execution: ExecutionSettings {
                        plan_mode: plan,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        s.queue
            .enqueue(EnqueueInput {
                chat_id: chat,
                draft: draft("question"),
            })
            .await
            .unwrap();
        let run = next_run(&mut s).await;
        run.events
            .send(ExecutionEvent::TextDelta(text.into()))
            .unwrap();
        run.events
            .send(ExecutionEvent::FinalAnswerText(final_text.into()))
            .unwrap();
        drop(run.events);
        run.finish.send(Ok(())).unwrap();
        event(
            &mut s,
            |e| matches!(e,QueueEvent::Finished(c,Ok(())) if *c==chat),
        )
        .await;
        assert!(s.db.final_answers(chat).await.unwrap().is_empty());
        assert_eq!(
            s.db.list_messages(chat).await.unwrap().last().unwrap().body,
            text
        );
    }
}

#[tokio::test]
async fn full_queue_is_a_policy_error_and_preserves_the_saved_draft() {
    let mut s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    let service = EnqueueService::<SqliteUnitOfWork>::new(s.db.clone(), s.db.clone());
    for index in 0..MAX_QUEUED_MESSAGES {
        service
            .enqueue(&QueuedMessage {
                id: Uuid::new_v4(),
                chat_id: chat,
                draft: draft(&format!("queued {index}")),
                state: "pending".into(),
                error: None,
            })
            .await
            .unwrap();
    }
    let unsent = draft("keep this draft");
    s.db.save_draft(chat, &unsent).await.unwrap();
    let result = service
        .enqueue(&QueuedMessage {
            id: Uuid::new_v4(),
            chat_id: chat,
            draft: unsent.clone(),
            state: "pending".into(),
            error: None,
        })
        .await;
    assert!(matches!(
        result,
        Err(EnqueueError::Full {
            limit: MAX_QUEUED_MESSAGES
        })
    ));
    assert_eq!(
        s.db.queue(chat).await.unwrap().entries.len(),
        MAX_QUEUED_MESSAGES as usize
    );
    assert_eq!(s.db.draft(chat).await.unwrap(), unsent);
    let result = s
        .queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: unsent.clone(),
        })
        .await;
    assert!(matches!(result, Err(EnqueueError::Full { .. })));
    assert_eq!(s.db.draft(chat).await.unwrap(), unsent);
    assert!(s.runs.try_recv().is_err());
}

#[tokio::test]
async fn concurrent_enqueue_services_cannot_overfill_the_last_slot() {
    let s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    let service = Arc::new(EnqueueService::<SqliteUnitOfWork>::new(
        s.db.clone(),
        s.db.clone(),
    ));
    for index in 0..MAX_QUEUED_MESSAGES - 1 {
        service
            .enqueue(&QueuedMessage {
                id: Uuid::new_v4(),
                chat_id: chat,
                draft: draft(&format!("queued {index}")),
                state: "pending".into(),
                error: None,
            })
            .await
            .unwrap();
    }
    // A separate pool/service bypasses the coordinator's in-memory serialization.
    let competing_db = Arc::new(SqliteChatStore::new(
        open_existing(&s._temp.path().join("chats.sqlite3"))
            .await
            .unwrap(),
    ));
    let competitor = Arc::new(EnqueueService::<SqliteUnitOfWork>::new(
        competing_db.clone(),
        competing_db,
    ));
    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let mut tasks = Vec::new();
    for index in 0..8 {
        let enqueue = if index % 2 == 0 {
            service.clone()
        } else {
            competitor.clone()
        };
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            enqueue
                .enqueue(&QueuedMessage {
                    id: Uuid::new_v4(),
                    chat_id: chat,
                    draft: draft(&format!("competitor {index}")),
                    state: "pending".into(),
                    error: None,
                })
                .await
        }));
    }
    let mut accepted = 0;
    let mut full = 0;
    for task in tasks {
        match tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
        {
            Ok(()) => accepted += 1,
            Err(EnqueueError::Full { .. }) => full += 1,
            Err(error) => panic!("unexpected enqueue failure: {error}"),
        }
    }
    assert_eq!((accepted, full), (1, 7));
    assert_eq!(
        s.db.queue(chat).await.unwrap().entries.len(),
        MAX_QUEUED_MESSAGES as usize
    );
}

#[tokio::test]
async fn failed_draft_transfer_rolls_back_the_insert_on_a_single_connection_pool() {
    let s = setup().await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    let unsent = draft("keep after rollback");
    s.db.save_draft(chat, &unsent).await.unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(s._temp.path().join("chats.sqlite3"))
                .foreign_keys(true),
        )
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER reject_draft_transfer BEFORE DELETE ON drafts BEGIN SELECT RAISE(ABORT, 'injected draft-transfer failure'); END")
        .execute(&pool).await.unwrap();
    let db = Arc::new(SqliteChatStore::new(pool));
    let service = EnqueueService::<SqliteUnitOfWork>::new(db.clone(), db.clone());
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        service.enqueue(&QueuedMessage {
            id: Uuid::new_v4(),
            chat_id: chat,
            draft: unsent.clone(),
            state: "pending".into(),
            error: None,
        }),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(EnqueueError::Storage(_))));
    assert!(db.queue(chat).await.unwrap().entries.is_empty());
    assert_eq!(db.draft(chat).await.unwrap(), unsent);
    // Rollback releases the only connection, so a following enqueue can commit.
    sqlx::query("DROP TRIGGER reject_draft_transfer")
        .execute(db.pool())
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        service.enqueue(&QueuedMessage {
            id: Uuid::new_v4(),
            chat_id: chat,
            draft: unsent,
            state: "pending".into(),
            error: None,
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(db.queue(chat).await.unwrap().entries.len(), 1);
    assert_eq!(db.draft(chat).await.unwrap(), Draft::default());
}

struct TestSkills(std::sync::atomic::AtomicBool);
#[async_trait]
impl SkillCatalog for TestSkills {
    fn revision(&self, _: Provider) -> u64 {
        0
    }
    async fn discover(
        &self,
        provider: Provider,
        workspace: &std::path::Path,
    ) -> Result<Vec<SkillDescriptor>, ExecutionError> {
        Ok(["review", "correct"]
            .into_iter()
            .map(|name| SkillDescriptor {
                reference: SkillReference {
                    provider,
                    name: name.into(),
                    path: workspace.join(name).join("SKILL.md"),
                    workspace: workspace.into(),
                },
                description: name.into(),
                source: "Project".into(),
                command: name.into(),
                unavailable: (!self.0.load(std::sync::atomic::Ordering::SeqCst))
                    .then(|| "Skill is disabled".into()),
            })
            .collect())
    }
    async fn inspect(&self, _: &SkillReference) -> Result<String, ExecutionError> {
        unreachable!()
    }
}
fn selected_draft(root: &std::path::Path, name: &str) -> Draft {
    let workspace = root.canonicalize().unwrap();
    Draft {
        text: format!("${name} instructions"),
        skills: vec![SkillMention {
            range: 0..name.len() + 1,
            reference: SkillReference {
                provider: Provider::Codex,
                name: name.into(),
                path: workspace.join(name).join("SKILL.md"),
                workspace,
            },
        }],
        ..Default::default()
    }
}
#[tokio::test]
async fn skill_identity_survives_accepted_steering_and_failed_turn_retry() {
    let catalog = Arc::new(TestSkills(std::sync::atomic::AtomicBool::new(true)));
    let mut s = setup_with_skills(Some(catalog)).await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    let original = selected_draft(s._temp.path(), "review");
    let correction = selected_draft(s._temp.path(), "correct");
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: original.clone(),
        })
        .await
        .unwrap();
    let mut run = next_run(&mut s).await;
    assert_eq!(run.request.skills, original.skills);
    run.events
        .send(ExecutionEvent::SteeringAvailable(true))
        .unwrap();
    run.events
        .send(ExecutionEvent::TextDelta("Before".into()))
        .unwrap();
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: correction.clone(),
        })
        .await
        .unwrap();
    event(
        &mut s,
        |e| matches!(e,QueueEvent::Snapshot(_,q) if q.steer_available && q.entries.len()==1),
    )
    .await;
    let entry =
        s.db.queue(chat)
            .await
            .unwrap()
            .entries
            .iter()
            .find(|e| e.draft.text == correction.text)
            .unwrap()
            .id;
    s.queue.steer(SteerQueueInput {
        chat_id: chat,
        entry_id: entry,
    });
    let command = tokio::time::timeout(Duration::from_secs(5), run.controls.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(command.input.skills, correction.skills);
    run.events
        .send(ExecutionEvent::InputAccepted(command.input))
        .unwrap();
    command.result.send(SteerOutcome::Accepted).unwrap();
    event(
        &mut s,
        |e| matches!(e,QueueEvent::Conversation(ConversationEvent::UserMessage(m)) if m.id==entry),
    )
    .await;
    run.finish.send(Err("interrupted".into())).unwrap();
    drop(run.events);
    event(&mut s, |e| matches!(e, QueueEvent::Finished(_, Err(_)))).await;
    let history = s.db.list_messages(chat).await.unwrap();
    let original_id = history.iter().find(|m| m.body == original.text).unwrap().id;
    assert_eq!(
        s.db.message_skills(chat, original_id).await.unwrap(),
        original.skills
    );
    assert_eq!(
        s.db.message_skills(chat, entry).await.unwrap(),
        correction.skills
    );
    // Re-enqueue the durable original ID to exercise the normal restart retry path.
    let enqueuer = EnqueueService::<SqliteUnitOfWork>::new(s.db.clone(), s.db.clone());
    enqueuer
        .enqueue(&QueuedMessage {
            id: original_id,
            chat_id: chat,
            draft: original.clone(),
            state: "pending".into(),
            error: None,
        })
        .await
        .unwrap();
    s.queue.resume(chat);
    let retried = next_run(&mut s).await;
    assert_eq!(retried.request.skills, original.skills);
    assert_eq!(retried.request.retry_inputs.len(), 1);
    assert_eq!(retried.request.retry_inputs[0].skills, correction.skills);
    assert_eq!(retried.request.retry_inputs[0].id, entry);
    retried
        .events
        .send(ExecutionEvent::TextDelta("Recovered".into()))
        .unwrap();
    retried.finish.send(Ok(())).unwrap();
    drop(retried.events);
    event(&mut s, |e| matches!(e, QueueEvent::Finished(_, Ok(())))).await;
}
#[tokio::test]
async fn unavailable_selection_pauses_before_delivery_and_can_be_edited_without_losing_identity() {
    let catalog = Arc::new(TestSkills(std::sync::atomic::AtomicBool::new(false)));
    let mut s = setup_with_skills(Some(catalog.clone())).await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    let draft = selected_draft(s._temp.path(), "review");
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: draft.clone(),
        })
        .await
        .unwrap();
    event(&mut s, |e| matches!(e, QueueEvent::Finished(_, Err(_)))).await;
    let queue = s.db.queue(chat).await.unwrap();
    assert!(
        queue
            .paused
            .as_deref()
            .unwrap()
            .contains("Skill is disabled")
    );
    assert!(s.db.list_messages(chat).await.unwrap().is_empty());
    assert!(s.runs.try_recv().is_err());
    let edited = s
        .queue
        .edit(EditQueueInput {
            chat_id: chat,
            entry_id: queue.entries[0].id,
        })
        .await
        .unwrap();
    assert_eq!(edited, draft);
    assert_eq!(s.db.draft(chat).await.unwrap(), draft);
    assert!(s.db.queue(chat).await.unwrap().entries.is_empty());
    catalog.0.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[tokio::test]
async fn skill_disabled_after_enqueue_pauses_steering_without_sending_it() {
    let catalog = Arc::new(TestSkills(std::sync::atomic::AtomicBool::new(true)));
    let mut s = setup_with_skills(Some(catalog.clone())).await;
    let chat = s.chats.create_empty_chat().await.unwrap().id;
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: selected_draft(s._temp.path(), "review"),
        })
        .await
        .unwrap();
    let run = next_run(&mut s).await;
    run.events
        .send(ExecutionEvent::SteeringAvailable(true))
        .unwrap();
    run.events
        .send(ExecutionEvent::TextDelta("Original answer".into()))
        .unwrap();
    let correction = selected_draft(s._temp.path(), "correct");
    s.queue
        .enqueue(EnqueueInput {
            chat_id: chat,
            draft: correction.clone(),
        })
        .await
        .unwrap();
    event(
        &mut s,
        |e| matches!(e,QueueEvent::Snapshot(_,q) if q.steer_available && q.entries.len()==1),
    )
    .await;
    let entry =
        s.db.queue(chat)
            .await
            .unwrap()
            .entries
            .iter()
            .find(|e| e.draft.text == correction.text)
            .unwrap()
            .id;
    catalog.0.store(false, std::sync::atomic::Ordering::SeqCst);
    s.queue.steer(SteerQueueInput {
        chat_id: chat,
        entry_id: entry,
    });
    event(&mut s,|e| matches!(e,QueueEvent::Snapshot(_,q) if q.paused.as_deref().is_some_and(|p| p.contains("disabled")))).await;
    let queued = s.db.queue(chat).await.unwrap();
    let queued = queued.entries.iter().find(|e| e.id == entry).unwrap();
    assert_eq!(queued.draft, correction);
    assert_eq!(queued.state, "pending");
    assert_eq!(s.db.list_messages(chat).await.unwrap().len(), 1);
    run.finish.send(Ok(())).unwrap();
    drop(run.events);
    event(&mut s, |e| matches!(e, QueueEvent::Finished(_, Ok(())))).await;
}
