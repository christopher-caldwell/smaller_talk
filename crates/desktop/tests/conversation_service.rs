use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use smaller_talk_application::{
    ChatQueries, ChatService, ChatWrites, ConversationEvent, ConversationService, ExecutionAdapter,
    ExecutionError, ExecutionEvent, ExecutionRequest, MessageQueries, MessageWrites,
    SendMessageInput,
};
use smaller_talk_database::{
    SqliteChatStore, create_empty_database, migrate_existing, open_existing,
};
use smaller_talk_domain::{ExecutionBinding, MessageRole};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Default)]
struct RecordingExecutor {
    requests: Mutex<Vec<ExecutionRequest>>,
}

#[async_trait]
impl ExecutionAdapter for RecordingExecutor {
    async fn execute(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError> {
        self.requests
            .lock()
            .expect("request list lock")
            .push(request.clone());
        if request.binding.is_none() {
            let _ = events.send(ExecutionEvent::BindingEstablished(ExecutionBinding {
                instructions_fingerprint: None,
                adapter_key: "codex".into(),
                native_session_id: "native-thread-1".into(),
            }));
        }
        let response = if request.prompt == "first prompt" {
            "A **bold** answer.\n\n```rust\nfn main() {}\n```"
        } else {
            "Follow-up completed."
        };
        for chunk in response.as_bytes().chunks(7) {
            let delta = String::from_utf8(chunk.to_vec()).expect("fixture chunk is UTF-8");
            let _ = events.send(ExecutionEvent::TextDelta(delta));
        }
        Ok(())
    }
}

fn services(
    store: &Arc<SqliteChatStore>,
    executor: Arc<dyn ExecutionAdapter>,
) -> (ChatService, ConversationService) {
    let chats: Arc<dyn ChatQueries> = store.clone();
    let writes: Arc<dyn ChatWrites> = store.clone();
    let messages: Arc<dyn MessageQueries> = store.clone();
    let message_writes: Arc<dyn MessageWrites> = store.clone();
    (
        ChatService::new(chats.clone(), writes.clone()),
        ConversationService::new(
            chats,
            writes,
            messages,
            message_writes,
            executor,
            PathBuf::from("/tmp/smaller-talk-test-workspace"),
        ),
    )
}

async fn send_and_collect(
    service: Arc<ConversationService>,
    chat_id: Uuid,
    prompt: &str,
) -> Vec<ConversationEvent> {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let prompt = prompt.to_owned();
    let send = tokio::spawn(async move {
        service
            .send_message(SendMessageInput { chat_id, prompt }, sender)
            .await
            .expect("message turn succeeds")
    });
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    send.await.expect("send task joins");
    events
}

#[tokio::test]
async fn streamed_transcript_and_codex_binding_survive_restart_and_stay_per_chat() {
    let directory = tempfile::tempdir().expect("temporary database directory");
    let path = directory.path().join("chats.sqlite3");
    create_empty_database(&path).await.expect("create database");
    migrate_existing(&path).await.expect("apply migrations");

    let executor = Arc::new(RecordingExecutor::default());
    let store = Arc::new(SqliteChatStore::new(
        open_existing(&path).await.expect("open database"),
    ));
    let (chats, conversation) = services(&store, executor.clone());
    let first = chats.create_empty_chat().await.expect("create first chat");
    let second = chats.create_empty_chat().await.expect("create second chat");

    let first_events = send_and_collect(Arc::new(conversation), first.id, "first prompt").await;
    assert!(matches!(
        first_events.first(),
        Some(ConversationEvent::UserMessage(message)) if message.body == "first prompt"
    ));
    let streamed = first_events
        .iter()
        .filter_map(|event| match event {
            ConversationEvent::AssistantDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(streamed.contains("bold"));
    assert!(matches!(
        first_events.last(),
        Some(ConversationEvent::AssistantMessage(message))
            if message.body.contains("```rust") && message.role == MessageRole::Assistant
    ));

    let first_messages = chats
        .list_chats()
        .await
        .expect("list chats")
        .into_iter()
        .find(|chat| chat.id == first.id)
        .expect("first chat remains present");
    assert_eq!(
        first_messages
            .binding
            .as_ref()
            .expect("Codex thread binding saved")
            .native_session_id,
        "native-thread-1"
    );
    assert_eq!(
        store
            .list_messages(first.id)
            .await
            .expect("load first transcript")
            .len(),
        2
    );
    assert!(
        store
            .list_messages(second.id)
            .await
            .expect("load second transcript")
            .is_empty()
    );

    store.pool().close().await;
    drop(chats);
    drop(store);

    let reopened_store = Arc::new(SqliteChatStore::new(
        open_existing(&path).await.expect("reopen database"),
    ));
    let (reopened_chats, reopened_conversation) = services(&reopened_store, executor.clone());
    let reopened_messages = reopened_conversation
        .list_messages(first.id)
        .await
        .expect("reload application-owned transcript");
    assert_eq!(reopened_messages.len(), 2);
    assert_eq!(reopened_messages[0].role, MessageRole::User);
    assert_eq!(reopened_messages[0].body, "first prompt");
    assert_eq!(reopened_messages[1].role, MessageRole::Assistant);

    let follow_up = send_and_collect(
        Arc::new(reopened_conversation),
        first.id,
        "continue after restart",
    )
    .await;
    assert!(matches!(
        follow_up.last(),
        Some(ConversationEvent::AssistantMessage(message))
            if message.body == "Follow-up completed."
    ));
    {
        let requests = executor.requests.lock().expect("request list lock");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].binding, None);
        assert_eq!(
            requests[1]
                .binding
                .as_ref()
                .expect("reopened chat uses its provider binding")
                .native_session_id,
            "native-thread-1"
        );
    }
    assert_eq!(
        reopened_chats
            .list_chats()
            .await
            .expect("other chat is still independent")
            .len(),
        2
    );
    reopened_store.pool().close().await;
}
