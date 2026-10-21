use async_trait::async_trait;
use smaller_talk_application::*;
use smaller_talk_database::{
    SqliteChatStore, create_empty_database, migrate_existing, open_existing,
};
use smaller_talk_domain::*;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;
use uuid::Uuid;

async fn store(path: &std::path::Path) -> Arc<SqliteChatStore> {
    create_empty_database(path).await.unwrap();
    migrate_existing(path).await.unwrap();
    Arc::new(SqliteChatStore::new(open_existing(path).await.unwrap()))
}

#[tokio::test]
async fn workspace_migration_preserves_legacy_history_and_bindings() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("old.sqlite3");
    create_empty_database(&path).await.unwrap();
    let migrations = temp.path().join("migrations");
    std::fs::create_dir(&migrations).unwrap();
    std::fs::write(
        migrations.join("0001_create_chat_history.sql"),
        include_str!("../../database/migrations/0001_create_chat_history.sql"),
    )
    .unwrap();
    let pool = open_existing(&path).await.unwrap();
    sqlx::migrate::Migrator::new(migrations.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    let old = SqliteChatStore::new(pool.clone());
    let chats = ChatService::new(Arc::new(old.clone()), Arc::new(old.clone()));
    let chat = chats.create_empty_chat().await.unwrap();
    old.save_execution_binding(
        chat.id,
        ExecutionBinding {
            adapter_key: "codex".into(),
            native_session_id: "old-native-session".into(),
        },
    )
    .await
    .unwrap();
    old.append_message(NewMessage {
        id: Uuid::new_v4(),
        chat_id: chat.id,
        role: MessageRole::User,
        body: "Saved before the upgrade".into(),
        created_at: chrono::Utc::now(),
    })
    .await
    .unwrap();
    pool.close().await;
    migrate_existing(&path).await.unwrap();
    let upgraded = SqliteChatStore::new(open_existing(&path).await.unwrap());
    assert_eq!(
        upgraded
            .find_chat(chat.id)
            .await
            .unwrap()
            .unwrap()
            .binding
            .unwrap()
            .native_session_id,
        "old-native-session"
    );
    assert_eq!(
        upgraded.list_messages(chat.id).await.unwrap()[0].body,
        "Saved before the upgrade"
    );
    assert!(upgraded.list_projects().await.unwrap().is_empty());
    assert_eq!(
        upgraded.preferences(chat.id).await.unwrap(),
        ChatPreferences::default()
    );
}

#[tokio::test]
async fn projects_search_drafts_attachments_and_tabs_survive_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("chats.sqlite3");
    let db = store(&path).await;
    let chats = ChatService::new(db.clone(), db.clone());
    let workspace = WorkspaceService::new(db.clone(), db.clone());
    let chat = chats.create_empty_chat().await.unwrap();
    let chat = chats
        .rename_chat(chat.id, "Memory discussion".into())
        .await
        .unwrap();
    let project = workspace
        .save_project(Project {
            id: Uuid::new_v4(),
            name: "Research Café".into(),
            folder: Some(temp.path().into()),
        })
        .await
        .unwrap();
    workspace
        .save_preferences(
            chat.id,
            ChatPreferences {
                project_id: Some(project.id.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let message = db
        .append_message(NewMessage {
            id: Uuid::new_v4(),
            chat_id: chat.id,
            role: MessageRole::User,
            body: "A literal 100%_needle and École".into(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    let file = Attachment {
        id: Uuid::new_v4().to_string(),
        name: "context.txt".into(),
        path: temp.path().join("stored.txt"),
        mime_type: "text/plain".into(),
        byte_len: 7,
        preview_path: None,
    };
    db.save_message_attachments(message.id, std::slice::from_ref(&file))
        .await
        .unwrap();
    let draft = Draft {
        text: "unfinished 🦀".into(),
        attachments: vec![file.clone()],
        selection_start: 0,
        selection_end: 10,
    };
    db.save_draft(chat.id, &draft).await.unwrap();
    let session = SessionState {
        open_tabs: vec![chat.id.to_string()],
        active_tab: Some(chat.id.to_string()),
        sidebar_collapsed: true,
        scroll_positions: [(chat.id.to_string(), (3, 42.))].into_iter().collect(),
        new_chat: None,
    };
    db.save_session(&session).await.unwrap();
    for query in ["MEMORY", "research", "100%_needle", "CAFÉ", "éCOLE"] {
        assert_eq!(db.search(query).await.unwrap()[0].chat_id, chat.id);
    }
    assert_eq!(
        db.search("100%_needle").await.unwrap()[0].message_id,
        Some(message.id)
    );
    assert!(db.search("not%_a_wildcard").await.unwrap().is_empty());
    assert!(db.search("'").await.unwrap().is_empty());
    db.pool().close().await;
    let reopened = SqliteChatStore::new(open_existing(&path).await.unwrap());
    assert_eq!(reopened.draft(chat.id).await.unwrap(), draft);
    assert_eq!(reopened.session().await.unwrap(), session);
    assert_eq!(
        reopened.message_attachments(chat.id).await.unwrap(),
        vec![(message.id, file.clone())]
    );
    assert!(
        reopened
            .attachment_references()
            .await
            .unwrap()
            .contains(&file.path)
    );
    let workspace = WorkspaceService::new(Arc::new(reopened.clone()), Arc::new(reopened.clone()));
    workspace.remove_project(project.id).await.unwrap();
    assert!(
        reopened
            .preferences(chat.id)
            .await
            .unwrap()
            .project_id
            .is_none()
    );
    assert!(reopened.find_chat(chat.id).await.unwrap().is_some());
    assert_eq!(reopened.list_messages(chat.id).await.unwrap().len(), 1);
}

#[derive(Default)]
struct RecordingAgent {
    requests: Mutex<Vec<ExecutionRequest>>,
    fail_once: std::sync::atomic::AtomicBool,
}
#[async_trait]
impl ExecutionAdapter for RecordingAgent {
    async fn capabilities(
        &self,
        provider: Provider,
    ) -> Result<ProviderCapabilities, ExecutionError> {
        Ok(ProviderCapabilities {
            provider,
            models: vec![ModelCapability {
                id: format!("{}-model", provider.key()),
                label: "Fixture".into(),
                efforts: vec!["low".into()],
                default_effort: Some("low".into()),
                images: true,
                is_default: true,
            }],
        })
    }
    async fn execute(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError> {
        self.requests.lock().unwrap().push(request.clone());
        let _ = events.send(ExecutionEvent::BindingEstablished(ExecutionBinding {
            adapter_key: request.settings.provider.key().into(),
            native_session_id: Uuid::new_v4().to_string(),
        }));
        if self
            .fail_once
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(ExecutionError::new("Connection lost"));
        }
        let _ = events.send(ExecutionEvent::TextDelta(format!(
            "{} answered {}",
            request.settings.provider.label(),
            request.prompt
        )));
        Ok(())
    }
}
fn conversation(
    db: &Arc<SqliteChatStore>,
    agent: Arc<dyn ExecutionAdapter>,
    workspace: PathBuf,
) -> ConversationService {
    ConversationService::new(
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        agent,
        workspace,
    )
    .with_workspace(db.clone(), db.clone())
}
async fn send(
    service: &ConversationService,
    id: Uuid,
    text: &str,
    retry: Option<Uuid>,
) -> Result<(), SendMessageError> {
    let (output, _events) = mpsc::unbounded_channel();
    let (_, answers) = mpsc::unbounded_channel();
    service
        .send_turn(
            SendTurnInput {
                chat_id: id,
                prompt: text.into(),
                attachments: vec![],
                retry_of: retry,
            },
            output,
            answers,
        )
        .await
}

#[tokio::test]
async fn provider_and_folder_changes_transfer_visible_context_once() {
    let temp = tempfile::tempdir().unwrap();
    let db = store(&temp.path().join("chats.sqlite3")).await;
    let agent = Arc::new(RecordingAgent::default());
    let service = conversation(&db, agent.clone(), temp.path().into());
    let chats = ChatService::new(db.clone(), db.clone());
    let workspace = WorkspaceService::new(db.clone(), db.clone());
    let id = chats.create_empty_chat().await.unwrap().id;
    send(&service, id, "Remember amber", None).await.unwrap();
    send(&service, id, "Same provider", None).await.unwrap();
    let mut prefs = db.preferences(id).await.unwrap();
    prefs.execution.provider = Provider::Claude;
    workspace.save_preferences(id, prefs).await.unwrap();
    send(&service, id, "Continue in Claude", None)
        .await
        .unwrap();
    let mut prefs = db.preferences(id).await.unwrap();
    prefs.execution.provider = Provider::Codex;
    workspace.save_preferences(id, prefs).await.unwrap();
    send(&service, id, "Return to Codex", None).await.unwrap();
    let folder = temp.path().join("other");
    std::fs::create_dir(&folder).unwrap();
    let project = workspace
        .save_project(Project {
            id: Uuid::new_v4(),
            name: "Other folder".into(),
            folder: Some(folder.clone()),
        })
        .await
        .unwrap();
    let mut prefs = db.preferences(id).await.unwrap();
    prefs.project_id = Some(project.id.to_string());
    workspace.save_preferences(id, prefs).await.unwrap();
    send(&service, id, "Use the project folder", None)
        .await
        .unwrap();
    {
        let requests = agent.requests.lock().unwrap();
        assert_eq!(requests.len(), 5);
        assert!(requests[0].binding.is_none());
        assert!(requests[0].prior_context.is_none());
        assert!(requests[1].binding.is_some());
        assert!(requests[1].prior_context.is_none());
        for request in &requests[2..] {
            assert!(request.binding.is_none());
            assert!(
                request
                    .prior_context
                    .as_ref()
                    .unwrap()
                    .contains("Remember amber")
            );
        }
        assert!(
            !requests[2]
                .prior_context
                .as_ref()
                .unwrap()
                .contains("Continue in Claude")
        );
        assert!(
            requests[3]
                .prior_context
                .as_ref()
                .unwrap()
                .contains("Claude answered")
        );
        assert_eq!(requests[4].workspace, folder.canonicalize().unwrap());
        assert_eq!(requests[0].settings.model.as_deref(), Some("codex-model"));
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM turns WHERE status = 'completed'")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 5);
    assert_eq!(db.list_messages(id).await.unwrap().len(), 10);
}

#[tokio::test]
async fn retry_after_failure_reuses_the_user_message_and_starts_fresh() {
    let temp = tempfile::tempdir().unwrap();
    let db = store(&temp.path().join("chats.sqlite3")).await;
    let agent = Arc::new(RecordingAgent::default());
    agent
        .fail_once
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let service = conversation(&db, agent.clone(), temp.path().into());
    let id = ChatService::new(db.clone(), db.clone())
        .create_empty_chat()
        .await
        .unwrap()
        .id;
    assert!(
        send(&service, id, "Retry this exact prompt", None)
            .await
            .is_err()
    );
    let user = db.list_messages(id).await.unwrap()[0].clone();
    send(&service, id, &user.body, Some(user.id)).await.unwrap();
    assert_eq!(db.list_messages(id).await.unwrap().len(), 2);
    let requests = agent.requests.lock().unwrap();
    assert!(requests[1].binding.is_none());
    assert_eq!(
        requests[1]
            .full_prompt()
            .matches("Retry this exact prompt")
            .count(),
        1
    );
}

struct GatedAgent {
    entered: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
}
#[async_trait]
impl ExecutionAdapter for GatedAgent {
    async fn execute(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError> {
        self.entered.add_permits(1);
        let permit = self.release.acquire().await.unwrap();
        permit.forget();
        let _ = events.send(ExecutionEvent::TextDelta(format!(
            "Finished {}",
            request.prompt
        )));
        Ok(())
    }
}
#[tokio::test]
async fn separate_chats_execute_concurrently_and_same_chat_rejects_a_second_turn() {
    let temp = tempfile::tempdir().unwrap();
    let db = store(&temp.path().join("chats.sqlite3")).await;
    let chats = ChatService::new(db.clone(), db.clone());
    let a = chats.create_empty_chat().await.unwrap().id;
    let b = chats.create_empty_chat().await.unwrap().id;
    let agent = Arc::new(GatedAgent {
        entered: tokio::sync::Semaphore::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let service = Arc::new(ConversationService::new(
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        agent.clone(),
        temp.path().into(),
    ));
    let first = {
        let service = service.clone();
        tokio::spawn(async move { send(&service, a, "A", None).await })
    };
    let second = {
        let service = service.clone();
        tokio::spawn(async move { send(&service, b, "B", None).await })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        agent.entered.acquire_many(2),
    )
    .await
    .unwrap()
    .unwrap()
    .forget();
    assert!(
        send(&service, a, "duplicate", None)
            .await
            .unwrap_err()
            .to_string()
            .contains("active turn")
    );
    agent.release.add_permits(2);
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(db.list_messages(a).await.unwrap().len(), 2);
    assert_eq!(db.list_messages(b).await.unwrap().len(), 2);
}

#[tokio::test]
async fn search_results_are_bounded_without_treating_query_text_as_operators() {
    let temp = tempfile::tempdir().unwrap();
    let db = store(&temp.path().join("search.sqlite3")).await;
    let service = ChatService::new(db.clone(), db.clone());
    for _ in 0..60 {
        service.create_empty_chat().await.unwrap();
    }
    assert_eq!(db.search("UNTITLED").await.unwrap().len(), 50);
    assert!(db.search("*").await.unwrap().is_empty());
    assert!(db.search("%_").await.unwrap().is_empty());
}

#[tokio::test]
async fn display_previews_use_first_user_message_without_changing_saved_titles() {
    let temp = tempfile::tempdir().unwrap();
    let db = store(&temp.path().join("previews.sqlite3")).await;
    let chats = ChatService::new(db.clone(), db.clone());
    let chat = chats.create_empty_chat().await.unwrap();
    for (index, role, body) in [
        (0, MessageRole::Assistant, "Not the title"),
        (1, MessageRole::User, "First question"),
        (2, MessageRole::User, "Later question"),
    ] {
        db.append_message(NewMessage {
            id: Uuid::new_v4(),
            chat_id: chat.id,
            role,
            body: body.into(),
            created_at: chrono::Utc::now() + chrono::Duration::seconds(index),
        })
        .await
        .unwrap();
    }
    assert_eq!(
        db.chat_previews().await.unwrap(),
        vec![(chat.id, "First question".into())]
    );
    assert_eq!(
        db.find_chat(chat.id).await.unwrap().unwrap().title,
        "Untitled"
    );
    chats
        .rename_chat(chat.id, "Chosen name".into())
        .await
        .unwrap();
    assert!(db.chat_previews().await.unwrap().is_empty());
    assert_eq!(
        db.find_chat(chat.id).await.unwrap().unwrap().title,
        "Chosen name"
    );
}

#[tokio::test]
async fn delayed_draft_save_after_chat_deletion_is_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let db = store(&temp.path().join("chats.sqlite3")).await;
    let chats = ChatService::new(db.clone(), db.clone());
    let deleted = chats.create_empty_chat().await.unwrap();
    let retained = chats.create_empty_chat().await.unwrap();
    let draft = Draft {
        text: "Unsent text".into(),
        ..Draft::default()
    };
    db.save_draft(deleted.id, &draft).await.unwrap();
    db.save_draft(retained.id, &draft).await.unwrap();
    chats.delete_chat(deleted.id).await.unwrap();

    // Tab-close and editor notifications may arrive after the delete completes.
    db.save_draft(deleted.id, &draft).await.unwrap();
    assert!(db.find_chat(deleted.id).await.unwrap().is_none());
    assert_eq!(db.draft(deleted.id).await.unwrap(), Draft::default());
    assert_eq!(db.draft(retained.id).await.unwrap(), draft);
}
