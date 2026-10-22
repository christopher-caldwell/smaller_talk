use async_trait::async_trait;
use smaller_talk_application::*;
use smaller_talk_database::*;
use smaller_talk_domain::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;
use uuid::Uuid;

fn custom(text: &str) -> CustomInstructions {
    CustomInstructions {
        enabled: true,
        text: text.into(),
    }
}

async fn database() -> (tempfile::TempDir, Arc<SqliteChatStore>) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("instructions.sqlite3");
    create_empty_database(&path).await.unwrap();
    migrate_existing(&path).await.unwrap();
    let db = Arc::new(SqliteChatStore::new(open_existing(&path).await.unwrap()));
    (temp, db)
}

fn instruction_service(db: &Arc<SqliteChatStore>) -> Arc<InstructionService> {
    Arc::new(InstructionService::new(db.clone(), db.clone(), db.clone()))
}

#[tokio::test]
async fn settings_are_scoped_durable_and_cannot_resurrect_a_removed_project() {
    let (temp, db) = database().await;
    let service = instruction_service(&db);
    let project = Project {
        id: Uuid::new_v4(),
        name: "Writing".into(),
        folder: None,
    };
    db.save_project(&project).await.unwrap();
    let mut settings = AppSettings {
        instructions: custom("Use plain language."),
        ..Default::default()
    };
    SettingsService::new(db.clone(), db.clone())
        .save(&settings)
        .await
        .unwrap();
    service
        .save_project(
            project.id,
            &custom("Prefer British spelling.\nKeep the examples."),
        )
        .await
        .unwrap();
    let snapshot = service.resolve(Some(project.id)).await.unwrap();
    assert!(snapshot.text.contains("Use plain language."));
    assert!(snapshot.text.contains("British spelling"));
    assert!(service.resolve(None).await.unwrap().project.is_empty());
    let reopened = Arc::new(SqliteChatStore::new(
        open_existing(&temp.path().join("instructions.sqlite3"))
            .await
            .unwrap(),
    ));
    assert_eq!(
        instruction_service(&reopened)
            .resolve(Some(project.id))
            .await
            .unwrap(),
        snapshot
    );
    settings.instructions.enabled = false;
    db.save_app_settings(&settings).await.unwrap();
    assert!(
        service
            .resolve(Some(project.id))
            .await
            .unwrap()
            .global
            .is_empty()
    );
    assert!(service.resolve(None).await.unwrap().fingerprint.is_none());
    let too_long = custom(&"é".repeat(MAX_INSTRUCTION_BYTES));
    assert!(matches!(
        service.save_project(project.id, &too_long).await,
        Err(SaveProjectInstructionsError::InvalidInstructions)
    ));
    db.remove_project(project.id).await.unwrap();
    assert!(matches!(
        service
            .save_project(project.id, &custom("Do not recreate"))
            .await,
        Err(SaveProjectInstructionsError::ProjectNotFound)
    ));
    assert!(db.list_projects().await.unwrap().is_empty());
}

#[derive(Default)]
struct Agent {
    requests: Mutex<Vec<ExecutionRequest>>,
    fail: AtomicBool,
}
#[async_trait]
impl ExecutionAdapter for Agent {
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
        let binding = request.binding.clone().unwrap_or(ExecutionBinding {
            adapter_key: request.settings.provider.key().into(),
            native_session_id: Uuid::new_v4().to_string(),
            instructions_fingerprint: None,
        });
        self.requests.lock().unwrap().push(request);
        let _ = events.send(ExecutionEvent::BindingEstablished(binding));
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(ExecutionError::new("synthetic failure"));
        }
        let _ = events.send(ExecutionEvent::TextDelta("Answer".into()));
        Ok(())
    }
}

async fn send(
    service: &ConversationService,
    chat: Uuid,
    retry: Option<Uuid>,
) -> Result<(), SendMessageError> {
    let (tx, _rx) = mpsc::unbounded_channel();
    let (_responses, receiver) = mpsc::unbounded_channel();
    service
        .send_turn(
            SendTurnInput {
                skills: Vec::new(),
                chat_id: chat,
                prompt: "Original user message".into(),
                attachments: vec![],
                retry_of: retry,
            },
            tx,
            receiver,
        )
        .await
}

#[tokio::test]
async fn edits_clearing_and_retries_use_the_correct_snapshot_and_native_session() {
    let (temp, db) = database().await;
    let instructions = instruction_service(&db);
    let agent = Arc::new(Agent::default());
    let service = ConversationService::new(
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        agent.clone(),
        temp.path().into(),
    )
    .with_workspace(db.clone(), db.clone())
    .with_settings(db.clone())
    .with_instructions(instructions.clone(), db.clone());
    let chat = ChatService::new(db.clone(), db.clone())
        .create_empty_chat()
        .await
        .unwrap()
        .id;
    let mut settings = AppSettings {
        instructions: custom("First preference"),
        ..Default::default()
    };
    db.save_app_settings(&settings).await.unwrap();
    send(&service, chat, None).await.unwrap();
    send(&service, chat, None).await.unwrap();
    settings.instructions = custom("Changed preference");
    db.save_app_settings(&settings).await.unwrap();
    agent.fail.store(true, Ordering::SeqCst);
    assert!(send(&service, chat, None).await.is_err());
    let original = db.list_messages(chat).await.unwrap().last().unwrap().id;
    let original_snapshot = instructions.for_retry(chat, original).await.unwrap();
    settings.instructions = custom("Later preference");
    db.save_app_settings(&settings).await.unwrap();
    send(&service, chat, Some(original)).await.unwrap();
    assert_eq!(
        instructions.for_retry(chat, original).await.unwrap(),
        original_snapshot
    );
    send(&service, chat, None).await.unwrap();
    settings.instructions.enabled = false;
    db.save_app_settings(&settings).await.unwrap();
    send(&service, chat, None).await.unwrap();
    send(&service, chat, None).await.unwrap();
    {
        let requests = agent.requests.lock().unwrap();
        assert!(requests[0].binding.is_none());
        assert!(requests[1].binding.is_some());
        assert!(requests[2].binding.is_none());
        assert!(requests[3].binding.is_none());
        assert_eq!(requests[3].instructions, original_snapshot);
        assert!(requests[4].binding.is_none());
        assert!(requests[4].instructions.text.contains("Later preference"));
        assert!(requests[5].binding.is_none());
        assert!(requests[5].instructions.text.is_empty());
        assert!(requests[6].binding.is_some());
        assert!(requests[2].prior_context.is_some());
    }
    assert!(
        db.list_messages(chat)
            .await
            .unwrap()
            .iter()
            .all(|m| !m.body.contains("preference"))
    );
}

#[tokio::test]
async fn failed_snapshot_recording_rolls_back_the_user_message_before_execution() {
    let (temp, db) = database().await;
    let agent = Arc::new(Agent::default());
    let chat = ChatService::new(db.clone(), db.clone())
        .create_empty_chat()
        .await
        .unwrap()
        .id;
    sqlx::query("CREATE TRIGGER fail_instruction_turn BEFORE INSERT ON turns BEGIN SELECT RAISE(ABORT, 'snapshot failure'); END;").execute(db.pool()).await.unwrap();
    let service = ConversationService::new(
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        agent.clone(),
        temp.path().into(),
    )
    .with_instructions(instruction_service(&db), db.clone());
    assert!(send(&service, chat, None).await.is_err());
    assert!(db.list_messages(chat).await.unwrap().is_empty());
    assert!(agent.requests.lock().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "Requires authenticated Codex and Claude CLIs; set SMALLER_TALK_LIVE=1"]
async fn authenticated_instructions_resume_edit_clear_and_plan_both_providers() {
    use smaller_talk_infrastructure::{AgentRouter, ClaudeAdapter, CodexAdapter};
    use std::time::Duration;
    assert_eq!(std::env::var("SMALLER_TALK_LIVE").as_deref(), Ok("1"));
    for provider in [Provider::Codex, Provider::Claude] {
        let (temp, db) = database().await;
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let router = AgentRouter::new(CodexAdapter::new("codex"), ClaudeAdapter::new("claude"));
        let caps = router.capabilities(provider).await.unwrap();
        let model = caps
            .models
            .iter()
            .find(|m| m.id.contains("luna") || m.id.contains("haiku"))
            .unwrap_or(&caps.models[0]);
        let instructions = instruction_service(&db);
        let project = Project {
            id: Uuid::new_v4(),
            name: "Synthetic instruction test".into(),
            folder: Some(workspace.clone()),
        };
        db.save_project(&project).await.unwrap();
        let chat = ChatService::new(db.clone(), db.clone())
            .create_empty_chat()
            .await
            .unwrap()
            .id;
        let mut prefs = ChatPreferences {
            project_id: Some(project.id.to_string()),
            execution: ExecutionSettings {
                provider,
                model: Some(model.id.clone()),
                effort: model.efforts.iter().find(|e| *e == "low").cloned(),
                plan_mode: true,
            },
            ..Default::default()
        };
        WorkspaceService::new(db.clone(), db.clone())
            .save_preferences(chat, prefs.clone())
            .await
            .unwrap();
        let mut settings = AppSettings {
            instructions: custom("Include G_ACTIVE in every final answer."),
            ..Default::default()
        };
        db.save_app_settings(&settings).await.unwrap();
        instructions
            .save_project(
                project.id,
                &custom("Include P_ALPHA in every final answer."),
            )
            .await
            .unwrap();
        let service = Arc::new(
            ConversationService::new(
                db.clone(),
                db.clone(),
                db.clone(),
                db.clone(),
                router,
                workspace.clone(),
            )
            .with_workspace(db.clone(), db.clone())
            .with_settings(db.clone())
            .with_instructions(instructions.clone(), db.clone()),
        );
        let mut bindings = Vec::new();
        for index in 0..4 {
            if index == 2 {
                instructions
                    .save_project(project.id, &custom("Include P_BETA in every final answer."))
                    .await
                    .unwrap();
                prefs = db.preferences(chat).await.unwrap();
                prefs.execution.plan_mode = false;
                WorkspaceService::new(db.clone(), db.clone())
                    .save_preferences(chat, prefs.clone())
                    .await
                    .unwrap();
            }
            if index == 3 {
                settings.instructions.enabled = false;
                db.save_app_settings(&settings).await.unwrap();
                instructions
                    .save_project(project.id, &CustomInstructions::default())
                    .await
                    .unwrap();
            }
            let (output, mut events) = mpsc::unbounded_channel();
            let (responses, receiver) = mpsc::unbounded_channel();
            let service = service.clone();
            let task = tokio::spawn(async move {
                service.send_turn(SendTurnInput {
            skills: Vec::new(), chat_id: chat, prompt: if index == 3 { "Do not use tools or change files. Reply with exactly: done" } else { "Do not use tools or change files. Give a very short greeting using the currently active saved preferences." }.into(), attachments: vec![], retry_of: None }, output, receiver).await
            });
            let answer = tokio::time::timeout(Duration::from_secs(120), async {
                let mut answer = String::new();
                while let Some(event) = events.recv().await {
                    match event {
                        ConversationEvent::AssistantDelta { delta, .. } => answer.push_str(&delta),
                        ConversationEvent::AssistantSnapshot { text, .. } => answer = text,
                        ConversationEvent::Interaction { interaction, .. } => {
                            let _ = responses.send(InteractionResponse {
                                id: interaction.id,
                                approved: false,
                                ..Default::default()
                            });
                        }
                        _ => {}
                    }
                }
                task.await.unwrap().unwrap();
                answer
            })
            .await
            .unwrap();
            println!("{} turn {}: {}", provider.label(), index, answer.trim());
            if index < 3 {
                assert!(answer.contains("G_ACTIVE"));
                assert!(answer.contains(if index < 2 { "P_ALPHA" } else { "P_BETA" }));
            } else {
                assert_eq!(answer.trim(), "done");
            }
            bindings.push(
                db.find_chat(chat)
                    .await
                    .unwrap()
                    .unwrap()
                    .binding
                    .unwrap()
                    .native_session_id,
            );
        }
        assert_eq!(bindings[0], bindings[1]);
        assert_ne!(bindings[1], bindings[2]);
        assert_ne!(bindings[2], bindings[3]);
        assert_eq!(std::fs::read_dir(workspace).unwrap().count(), 0);
    }
}
