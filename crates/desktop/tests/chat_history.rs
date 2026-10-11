use std::time::{SystemTime, UNIX_EPOCH};
use std::{error::Error, sync::Arc};

use chrono::Utc;
use smaller_talk_application::{
    ChatQueries, ChatService, ChatWrites, DeleteChatError, NewChat, PersistenceError,
};
use smaller_talk_database::{
    SqliteChatStore, create_empty_database, migrate_existing, open_existing,
};
use smaller_talk_domain::ExecutionBinding;

#[tokio::test]
async fn sqlite_chat_history_survives_reopen_and_delete_cascades() {
    let directory = tempfile::tempdir().expect("temporary database directory");
    let path = directory.path().join("chats.sqlite3");
    create_empty_database(&path).await.expect("create database");
    migrate_existing(&path)
        .await
        .expect("apply schema migrations");
    let pool = open_existing(&path).await.expect("open migrated database");
    let store = Arc::new(SqliteChatStore::new(pool));
    let queries: Arc<dyn ChatQueries> = store.clone();
    let writes: Arc<dyn ChatWrites> = store.clone();
    let service = ChatService::new(queries, writes);

    let first = service
        .create_empty_chat()
        .await
        .expect("create first chat");
    let second = service
        .create_empty_chat()
        .await
        .expect("create second chat");
    assert_eq!(service.list_chats().await.expect("list chats").len(), 2);

    let proposed_created_at = Utc::now();
    let bound_chat = NewChat {
        id: uuid::Uuid::new_v4(),
        title: "Bound fixture".to_owned(),
        created_at: proposed_created_at,
        binding: Some(ExecutionBinding {
            adapter_key: "test-adapter".to_owned(),
            native_session_id: "test-session".to_owned(),
        }),
    };
    let bound_chat = store
        .create_chat(bound_chat)
        .await
        .expect("store opaque provider binding fixture");
    assert_eq!(
        bound_chat.created_at,
        chrono::DateTime::from_timestamp_millis(proposed_created_at.timestamp_millis())
            .expect("proposed time can be stored by SQLite")
    );

    let message_id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO messages (id, chat_id, role, body, created_at) VALUES (?, ?, 'user', 'stored text', ?)")
        .bind(&message_id)
        .bind(first.id.to_string())
        .bind(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock is after Unix epoch")
                .as_millis() as i64,
        )
        .execute(store.pool())
        .await
        .expect("insert a stored message fixture");

    store.pool().close().await;
    drop(service);
    drop(store);

    let reopened_pool = open_existing(&path).await.expect("reopen database");
    let reopened_store = Arc::new(SqliteChatStore::new(reopened_pool));
    let queries: Arc<dyn ChatQueries> = reopened_store.clone();
    let writes: Arc<dyn ChatWrites> = reopened_store.clone();
    let reopened_service = ChatService::new(queries, writes);
    let reopened_chats = reopened_service
        .list_chats()
        .await
        .expect("list after reopen");
    assert_eq!(reopened_chats.len(), 3);
    assert!(reopened_chats.iter().any(|chat| chat.id == first.id));
    assert!(reopened_chats.iter().any(|chat| chat.id == second.id));
    let persisted_binding = reopened_chats
        .iter()
        .find(|chat| chat.id == bound_chat.id)
        .and_then(|chat| chat.binding.as_ref())
        .expect("load opaque provider binding after reopen");
    assert_eq!(persisted_binding.adapter_key, "test-adapter");
    assert_eq!(persisted_binding.native_session_id, "test-session");
    let persisted_message: String = sqlx::query_scalar("SELECT body FROM messages WHERE id = ?")
        .bind(&message_id)
        .fetch_one(reopened_store.pool())
        .await
        .expect("load message after reopen");
    assert_eq!(persisted_message, "stored text");

    reopened_service
        .delete_chat(first.id)
        .await
        .expect("delete first chat");
    assert!(matches!(
        reopened_service.delete_chat(first.id).await,
        Err(DeleteChatError::NotFound)
    ));
    let message_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE chat_id = ?")
        .bind(first.id.to_string())
        .fetch_one(reopened_store.pool())
        .await
        .expect("count messages after delete");
    assert_eq!(message_count, 0);

    reopened_store.pool().close().await;
    drop(reopened_service);
    drop(reopened_store);

    let final_pool = open_existing(&path).await.expect("reopen after delete");
    let final_store = Arc::new(SqliteChatStore::new(final_pool));
    let queries: Arc<dyn ChatQueries> = final_store.clone();
    let writes: Arc<dyn ChatWrites> = final_store.clone();
    let final_service = ChatService::new(queries, writes);
    let remaining = final_service.list_chats().await.expect("list after delete");
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().any(|chat| chat.id == second.id));
    assert!(remaining.iter().any(|chat| chat.id == bound_chat.id));

    final_store.pool().close().await;
    let storage_error = final_service
        .list_chats()
        .await
        .expect_err("closed database reports a storage error");
    let persistence_source =
        Error::source(&storage_error).expect("service preserves adapter error");
    assert!(
        persistence_source
            .downcast_ref::<PersistenceError>()
            .is_some()
    );
    assert!(
        Error::source(persistence_source)
            .expect("persistence error preserves SQLx source")
            .is::<sqlx::Error>()
    );
}

#[tokio::test]
async fn renaming_preserves_chat_identity_binding_messages_and_survives_reopen() {
    use smaller_talk_application::{MessageQueries, MessageWrites, NewMessage, RenameChatError};
    use smaller_talk_domain::MessageRole;

    let directory = tempfile::tempdir().expect("temporary database directory");
    let path = directory.path().join("chats.sqlite3");
    create_empty_database(&path).await.expect("create database");
    migrate_existing(&path).await.expect("migrate database");
    let store = Arc::new(SqliteChatStore::new(
        open_existing(&path).await.expect("open database"),
    ));
    let queries: Arc<dyn ChatQueries> = store.clone();
    let writes: Arc<dyn ChatWrites> = store.clone();
    let service = ChatService::new(queries, writes.clone());
    let first = service.create_empty_chat().await.expect("create chat");
    let second = service
        .create_empty_chat()
        .await
        .expect("create second chat");
    assert_eq!(first.title, "Untitled");
    assert_eq!(second.title, "Untitled");

    let binding = ExecutionBinding {
        adapter_key: "codex".into(),
        native_session_id: "thread-rename-fixture".into(),
    };
    writes
        .save_execution_binding(first.id, binding.clone())
        .await
        .expect("save provider binding");
    store
        .append_message(NewMessage {
            id: uuid::Uuid::new_v4(),
            chat_id: first.id,
            role: MessageRole::User,
            body: "keep this prompt".into(),
            created_at: Utc::now(),
        })
        .await
        .expect("save existing message");

    let renamed = service
        .rename_chat(first.id, "  Rust refactoring  ".into())
        .await
        .expect("rename chat");
    assert_eq!(renamed.id, first.id);
    assert_eq!(renamed.title, "Rust refactoring");
    assert_eq!(renamed.binding.as_ref(), Some(&binding));
    assert!(matches!(
        service.rename_chat(first.id, "   ".into()).await,
        Err(RenameChatError::EmptyTitle)
    ));

    store.pool().close().await;
    drop(service);
    drop(store);

    let reopened = Arc::new(SqliteChatStore::new(
        open_existing(&path).await.expect("reopen database"),
    ));
    let persisted = reopened
        .list_chats()
        .await
        .expect("load chats after restart");
    let persisted_first = persisted
        .iter()
        .find(|chat| chat.id == first.id)
        .expect("renamed chat is still the same row");
    assert_eq!(persisted_first.title, "Rust refactoring");
    assert_eq!(persisted_first.binding.as_ref(), Some(&binding));
    assert_eq!(
        persisted
            .iter()
            .find(|chat| chat.id == second.id)
            .expect("second chat remains independent")
            .title,
        "Untitled"
    );
    let messages = reopened
        .list_messages(first.id)
        .await
        .expect("existing transcript remains attached");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "keep this prompt");
    reopened.pool().close().await;
}
