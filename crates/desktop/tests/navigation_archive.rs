use smaller_talk_application::*;
use smaller_talk_database::*;
use smaller_talk_domain::*;
use std::sync::Arc;
use uuid::Uuid;

async fn database(path: &std::path::Path) -> Arc<SqliteChatStore> {
    create_empty_database(path).await.unwrap();
    migrate_existing(path).await.unwrap();
    Arc::new(SqliteChatStore::new(open_existing(path).await.unwrap()))
}
async fn message(db: &SqliteChatStore, id: Uuid, body: &str, time: i64) -> Message {
    db.append_message(NewMessage {
        id: Uuid::new_v4(),
        chat_id: id,
        role: MessageRole::User,
        body: body.into(),
        created_at: chrono::DateTime::from_timestamp_millis(time).unwrap(),
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn archive_preserves_history_binding_draft_attachments_and_reopens() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("chats.sqlite3");
    let db = database(&path).await;
    let chats = ChatService::new(db.clone(), db.clone());
    let chat = chats.create_empty_chat().await.unwrap();
    let binding = ExecutionBinding {
        adapter_key: "codex".into(),
        native_session_id: "native-existing".into(),
        instructions_fingerprint: None,
    };
    db.save_execution_binding(chat.id, binding.clone())
        .await
        .unwrap();
    let saved = message(&db, chat.id, "archive searchable", 1000).await;
    let draft = Draft {
        text: "retain this draft".into(),
        ..Default::default()
    };
    db.save_draft(chat.id, &draft).await.unwrap();
    let attachment = Attachment {
        id: "file".into(),
        name: "saved.txt".into(),
        path: temp.path().join("saved.txt"),
        mime_type: "text/plain".into(),
        byte_len: 4,
        preview_path: None,
    };
    db.save_message_attachments(saved.id, std::slice::from_ref(&attachment))
        .await
        .unwrap();
    let archived = chats.set_archived(chat.id, true).await.unwrap();
    assert!(archived.archived_at.is_some());
    assert_eq!(archived.binding, Some(binding.clone()));
    assert!(db.search("searchable").await.unwrap().is_empty());
    assert_eq!(
        db.search_scoped(&SearchChatsInput {
            text: "searchable".into(),
            archived: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .len(),
        1
    );
    assert_eq!(db.list_messages(chat.id).await.unwrap(), vec![saved]);
    assert_eq!(db.draft(chat.id).await.unwrap(), draft);
    assert_eq!(
        db.message_attachments(chat.id).await.unwrap()[0].1,
        attachment
    );
    assert!(
        db.attachment_references()
            .await
            .unwrap()
            .contains(&attachment.path)
    );
    drop(chats);
    db.pool().close().await;
    let reopened = Arc::new(SqliteChatStore::new(open_existing(&path).await.unwrap()));
    assert!(
        reopened
            .find_chat(chat.id)
            .await
            .unwrap()
            .unwrap()
            .archived_at
            .is_some()
    );
    assert_eq!(reopened.draft(chat.id).await.unwrap(), draft);
    let restored = ChatService::new(reopened.clone(), reopened.clone())
        .set_archived(chat.id, false)
        .await
        .unwrap();
    assert!(restored.archived_at.is_none());
    assert_eq!(restored.binding, Some(binding));
    assert_eq!(reopened.search("searchable").await.unwrap().len(), 1);
}

#[tokio::test]
async fn archive_rechecks_queue_and_running_turn_and_failed_write_does_not_archive() {
    let temp = tempfile::tempdir().unwrap();
    let db = database(&temp.path().join("chats.sqlite3")).await;
    let chats = ChatService::new(db.clone(), db.clone());
    let chat = chats.create_empty_chat().await.unwrap();
    let entry = QueuedMessage {
        id: Uuid::new_v4(),
        chat_id: chat.id,
        draft: Draft {
            text: "queued".into(),
            ..Default::default()
        },
        state: "pending".into(),
        error: None,
    };
    let enqueuer = EnqueueService::<SqliteUnitOfWork>::new(db.clone(), db.clone());
    enqueuer.enqueue(&entry).await.unwrap();
    assert!(matches!(
        chats.set_archived(chat.id, true).await,
        Err(ArchiveChatError::Busy)
    ));
    db.remove_queued(chat.id, entry.id, false).await.unwrap();
    let user = message(&db, chat.id, "turn", 1000).await;
    let mut turn = TurnRecord {
        id: Uuid::new_v4(),
        chat_id: chat.id,
        user_message_id: user.id,
        settings: Default::default(),
        workspace: temp.path().into(),
        status: "running".into(),
        error: None,
        instructions: Default::default(),
    };
    db.save_turn(&turn).await.unwrap();
    assert!(matches!(
        chats.set_archived(chat.id, true).await,
        Err(ArchiveChatError::Busy)
    ));
    turn.status = "completed".into();
    db.save_turn(&turn).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_archive BEFORE UPDATE OF archived_at ON chats BEGIN SELECT RAISE(FAIL, 'injected archive failure'); END").execute(db.pool()).await.unwrap();
    assert!(chats.set_archived(chat.id, true).await.is_err());
    assert!(
        db.find_chat(chat.id)
            .await
            .unwrap()
            .unwrap()
            .archived_at
            .is_none()
    );
    sqlx::query("DROP TRIGGER reject_archive")
        .execute(db.pool())
        .await
        .unwrap();
    chats.set_archived(chat.id, true).await.unwrap();
    assert!(enqueuer.enqueue(&entry).await.is_err());
    assert!(db.queue(chat.id).await.unwrap().entries.is_empty());
}

#[tokio::test]
async fn project_filters_apply_before_limit_and_activity_excludes_archive() {
    let temp = tempfile::tempdir().unwrap();
    let db = database(&temp.path().join("chats.sqlite3")).await;
    let chats = ChatService::new(db.clone(), db.clone());
    let hidden = Project {
        id: Uuid::new_v4(),
        name: "Hidden café".into(),
        folder: None,
    };
    db.save_project(&hidden).await.unwrap();
    let visible = chats.create_empty_chat().await.unwrap();
    message(&db, visible.id, "needle école", 1000).await;
    for i in 0..55 {
        let c = chats.create_empty_chat().await.unwrap();
        db.save_preferences(
            c.id,
            &ChatPreferences {
                project_id: Some(hidden.id.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        message(&db, c.id, "needle école", 2000 + i).await;
    }
    assert_eq!(
        db.project_activity().await.unwrap()[&hidden.id].timestamp_millis(),
        2054
    );
    let hits = db
        .search_scoped(&SearchChatsInput {
            text: "ÉCOLE".into(),
            hidden_projects: [hidden.id].into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chat_id, visible.id);
    for c in db
        .list_chats()
        .await
        .unwrap()
        .into_iter()
        .filter(|c| c.id != visible.id)
    {
        chats.set_archived(c.id, true).await.unwrap();
    }
    assert!(db.project_activity().await.unwrap().is_empty());
    // An old conversation archived most recently must lead even a bounded search.
    chats.set_archived(visible.id, true).await.unwrap();
    let newest = chrono::Utc::now().timestamp_millis() + 1000;
    sqlx::query("UPDATE chats SET archived_at = ? WHERE id = ?")
        .bind(newest)
        .bind(visible.id.to_string())
        .execute(db.pool())
        .await
        .unwrap();
    let archive_hits = db
        .search_scoped(&SearchChatsInput {
            text: "needle".into(),
            archived: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(archive_hits[0].chat_id, visible.id);
    // Archive search ignores focus filters so all archived records remain recoverable.
    assert_eq!(
        db.search_scoped(&SearchChatsInput {
            text: "needle".into(),
            archived: true,
            hidden_projects: [hidden.id].into()
        })
        .await
        .unwrap()
        .len(),
        50
    );
}

#[tokio::test]
async fn navigation_preferences_round_trip_and_legacy_json_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("chats.sqlite3");
    let db = database(&path).await;
    let project = Uuid::new_v4();
    let session = SessionState {
        navigation: NavigationPreferences {
            project_sort: ProjectSort::LastChatUpdated,
            hidden_projects: [project].into(),
            collapsed_projects: [project].into(),
        },
        ..Default::default()
    };
    db.save_session(&session).await.unwrap();
    db.pool().close().await;
    let db = SqliteChatStore::new(open_existing(&path).await.unwrap());
    assert_eq!(db.session().await.unwrap(), session);
    let legacy: SessionState = serde_json::from_str(
        r#"{"open_tabs":[],"active_tab":null,"sidebar_collapsed":false,"scroll_positions":{}}"#,
    )
    .unwrap();
    assert_eq!(legacy.navigation, NavigationPreferences::default());
}
