use smaller_talk_application::*;
use smaller_talk_database::*;
use smaller_talk_domain::*;
use std::{path::Path, sync::Arc};
use uuid::Uuid;

async fn store(path: &Path) -> Arc<SqliteChatStore> {
    create_empty_database(path).await.unwrap();
    migrate_existing(path).await.unwrap();
    Arc::new(SqliteChatStore::new(open_existing(path).await.unwrap()))
}

fn pending() -> NewChatDraft {
    NewChatDraft {
        id: Uuid::new_v4().to_string(),
        preferences: ChatPreferences {
            execution: ExecutionSettings {
                provider: Provider::Claude,
                model: Some("draft-model".into()),
                effort: Some("low".into()),
                plan_mode: true,
                permissions: Default::default(),
            },
            ..Default::default()
        },
        draft: Draft {
            skills: Vec::new(),
            text: "My unsent draft 🦀".into(),
            selection_start: 3,
            selection_end: 8,
            ..Default::default()
        },
        submitted: None,
    }
}

#[tokio::test]
async fn new_chat_draft_reopens_without_creating_history_and_keeps_attachment_references() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.sqlite3");
    let db = store(&path).await;
    let mut draft = pending();
    let attachment = temp.path().join("image.png");
    let thumbnail = temp.path().join("thumbnail.png");
    draft.draft.attachments.push(Attachment {
        id: "draft-image".into(),
        name: "image.png".into(),
        path: attachment.clone(),
        preview_path: Some(thumbnail.clone()),
        mime_type: "image/png".into(),
        byte_len: 4,
    });
    let session = SessionState {
        open_tabs: vec![draft.id.clone()],
        active_tab: Some(draft.id.clone()),
        new_chat: Some(draft),
        ..Default::default()
    };
    db.save_session(&session).await.unwrap();
    db.pool().close().await;
    let reopened = SqliteChatStore::new(open_existing(&path).await.unwrap());
    assert!(reopened.list_chats().await.unwrap().is_empty());
    assert!(reopened.search("unsent").await.unwrap().is_empty());
    assert_eq!(reopened.session().await.unwrap(), session);
    let references = reopened.attachment_references().await.unwrap();
    assert!(references.contains(&attachment));
    assert!(references.contains(&thumbnail));
}

#[tokio::test]
async fn first_send_atomically_creates_one_named_chat_with_settings_and_queued_content() {
    let temp = tempfile::tempdir().unwrap();
    let db = store(&temp.path().join("db.sqlite3")).await;
    let draft = pending();
    let id = Uuid::parse_str(&draft.id).unwrap();
    let input = StartChatInput {
        chat_id: id,
        preferences: draft.preferences.clone(),
        draft: draft.draft.clone(),
    };
    let service = ChatService::new(db.clone(), db.clone()).with_setup_writes(db.clone());
    let chat = service.start_chat(input.clone()).await.unwrap();
    assert_eq!(chat.title, draft.draft.text);
    assert_eq!(db.list_chats().await.unwrap(), vec![chat]);
    assert_eq!(db.preferences(id).await.unwrap(), draft.preferences);
    let queue = db.queue(id).await.unwrap();
    assert_eq!(queue.entries.len(), 1);
    assert_eq!(queue.entries[0].draft, draft.draft);
    // A duplicate UI acknowledgement cannot create another conversation or first input.
    assert!(service.start_chat(input).await.is_err());
    assert_eq!(db.list_chats().await.unwrap().len(), 1);
    assert_eq!(db.queue(id).await.unwrap().entries.len(), 1);
}

#[tokio::test]
async fn project_draft_and_first_send_keep_the_project_and_working_folder_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.sqlite3");
    let db = store(&path).await;
    let project_folder = temp.path().join("family-photos");
    std::fs::create_dir(&project_folder).unwrap();
    let project = Project {
        id: Uuid::new_v4(),
        name: "Family Photos".into(),
        folder: Some(project_folder.clone()),
    };
    db.save_project(&project).await.unwrap();
    let mut draft = pending();
    draft.preferences.project_id = Some(project.id.to_string());
    let id = Uuid::parse_str(&draft.id).unwrap();
    let session = SessionState {
        open_tabs: vec![draft.id.clone()],
        active_tab: Some(draft.id.clone()),
        new_chat: Some(draft.clone()),
        ..Default::default()
    };
    db.save_session(&session).await.unwrap();
    db.pool().close().await;

    let reopened = Arc::new(SqliteChatStore::new(open_existing(&path).await.unwrap()));
    assert!(reopened.list_chats().await.unwrap().is_empty());
    assert_eq!(reopened.session().await.unwrap(), session);
    let service =
        ChatService::new(reopened.clone(), reopened.clone()).with_setup_writes(reopened.clone());
    service
        .start_chat(StartChatInput {
            chat_id: id,
            preferences: draft.preferences.clone(),
            draft: draft.draft.clone(),
        })
        .await
        .unwrap();
    reopened.pool().close().await;

    let saved = SqliteChatStore::new(open_existing(&path).await.unwrap());
    assert_eq!(saved.list_chats().await.unwrap().len(), 1);
    let preferences = saved.preferences(id).await.unwrap();
    assert_eq!(preferences, draft.preferences);
    assert_eq!(saved.queue(id).await.unwrap().entries[0].draft, draft.draft);
    assert_eq!(
        effective_workspace(
            temp.path(),
            &preferences,
            &saved.list_projects().await.unwrap()
        )
        .unwrap(),
        project_folder.canonicalize().unwrap(),
    );
}

#[tokio::test]
async fn whitespace_first_send_creates_nothing_but_attachment_only_input_is_valid() {
    let temp = tempfile::tempdir().unwrap();
    let db = store(&temp.path().join("db.sqlite3")).await;
    let service = ChatService::new(db.clone(), db.clone()).with_setup_writes(db.clone());
    let mut input = StartChatInput {
        chat_id: Uuid::new_v4(),
        preferences: ChatPreferences::default(),
        draft: Draft {
            skills: Vec::new(),
            text: " \n\t ".into(),
            ..Default::default()
        },
    };
    assert!(matches!(
        service.start_chat(input.clone()).await,
        Err(StartChatError::EmptyMessage)
    ));
    assert!(db.list_chats().await.unwrap().is_empty());
    input.draft.attachments.push(Attachment {
        id: "file".into(),
        name: "notes.txt".into(),
        path: temp.path().join("notes.txt"),
        mime_type: "text/plain".into(),
        byte_len: 4,
        preview_path: None,
    });
    let chat = service.start_chat(input).await.unwrap();
    assert_eq!(chat.title, "Attachment: notes.txt");
    assert_eq!(db.queue(chat.id).await.unwrap().entries.len(), 1);
}

#[tokio::test]
async fn failed_settings_or_queue_write_rolls_back_creation_and_keeps_the_unsent_draft() {
    for table in ["chat_settings", "message_queue"] {
        let temp = tempfile::tempdir().unwrap();
        let db = store(&temp.path().join("db.sqlite3")).await;
        let draft = pending();
        let session = SessionState {
            new_chat: Some(draft.clone()),
            ..Default::default()
        };
        db.save_session(&session).await.unwrap();
        let sql = format!(
            "CREATE TRIGGER fail_first_send BEFORE INSERT ON {table} BEGIN SELECT RAISE(FAIL, 'injected write failure'); END"
        );
        sqlx::query(&sql).execute(db.pool()).await.unwrap();
        let service = ChatService::new(db.clone(), db.clone()).with_setup_writes(db.clone());
        let input = StartChatInput {
            chat_id: Uuid::parse_str(&draft.id).unwrap(),
            preferences: draft.preferences,
            draft: draft.draft,
        };
        assert!(service.start_chat(input).await.is_err());
        assert!(db.list_chats().await.unwrap().is_empty());
        assert_eq!(db.session().await.unwrap(), session);
    }
}
