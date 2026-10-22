use smaller_talk_application::*;
use smaller_talk_database::*;
use smaller_talk_domain::*;
use std::sync::Arc;
use uuid::Uuid;

fn draft(workspace: &std::path::Path, source: &str) -> Draft {
    Draft {
        text: "$review check".into(),
        skills: vec![SkillMention {
            range: 0..7,
            reference: SkillReference {
                provider: Provider::Codex,
                name: "review".into(),
                path: workspace.join(source).join("SKILL.md"),
                workspace: workspace.into(),
            },
        }],
        ..Default::default()
    }
}
async fn database() -> (tempfile::TempDir, Arc<SqliteChatStore>, Uuid) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("skills.sqlite3");
    create_empty_database(&path).await.unwrap();
    migrate_existing(&path).await.unwrap();
    let db = Arc::new(SqliteChatStore::new(open_existing(&path).await.unwrap()));
    let chat = ChatService::new(db.clone(), db.clone())
        .create_empty_chat()
        .await
        .unwrap()
        .id;
    (temp, db, chat)
}
#[tokio::test]
async fn enqueue_ack_compares_source_and_drafts_and_queue_survive_reopen() {
    let (temp, db, chat) = database().await;
    let sent = draft(temp.path(), "one");
    let edited = draft(temp.path(), "two");
    db.save_draft(chat, &edited).await.unwrap();
    let queue = EnqueueService::<SqliteUnitOfWork>::new(db.clone(), db.clone());
    let id = Uuid::new_v4();
    queue
        .enqueue(&QueuedMessage {
            id,
            chat_id: chat,
            draft: sent.clone(),
            state: "pending".into(),
            error: None,
        })
        .await
        .unwrap();
    let reopened = SqliteChatStore::new(
        open_existing(&temp.path().join("skills.sqlite3"))
            .await
            .unwrap(),
    );
    assert_eq!(reopened.draft(chat).await.unwrap(), edited);
    assert_eq!(reopened.queue(chat).await.unwrap().entries[0].draft, sent);
    db.save_draft(chat, &sent).await.unwrap();
    queue
        .enqueue(&QueuedMessage {
            id: Uuid::new_v4(),
            chat_id: chat,
            draft: sent,
            state: "pending".into(),
            error: None,
        })
        .await
        .unwrap();
    assert_eq!(db.draft(chat).await.unwrap(), Draft::default());
}
#[tokio::test]
async fn initial_and_steered_metadata_are_atomic_durable_and_chat_scoped() {
    let (temp, db, chat) = database().await;
    let draft = draft(temp.path(), "selected");
    let original = Uuid::new_v4();
    let turn_id = Uuid::new_v4();
    let turn = TurnRecord {
        instructions: Default::default(),
        id: turn_id,
        chat_id: chat,
        user_message_id: original,
        settings: Default::default(),
        workspace: temp.path().into(),
        status: "running".into(),
        error: None,
    };
    db.record_turn_start(RecordTurnStart {
        user: Message {
            id: original,
            chat_id: chat,
            role: MessageRole::User,
            body: draft.text.clone(),
            created_at: chrono::Utc::now(),
        },
        insert_user: true,
        attachments: vec![],
        skills: draft.skills.clone(),
        turn,
    })
    .await
    .unwrap();
    let user = |id| NewMessage {
        id,
        chat_id: chat,
        role: MessageRole::User,
        body: draft.text.clone(),
        created_at: chrono::Utc::now(),
    };
    let bad = Uuid::new_v4();
    assert!(
        db.persist_steered_input(None, user(bad), &[], &draft.skills, Uuid::new_v4())
            .await
            .is_err()
    );
    assert!(
        db.list_messages(chat)
            .await
            .unwrap()
            .iter()
            .all(|m| m.id != bad)
    );
    assert!(db.message_skills(chat, bad).await.unwrap().is_empty());
    let accepted = Uuid::new_v4();
    db.persist_steered_input(None, user(accepted), &[], &draft.skills, turn_id)
        .await
        .unwrap();
    db.recover_interrupted_turns().await.unwrap();
    let reopened = SqliteChatStore::new(
        open_existing(&temp.path().join("skills.sqlite3"))
            .await
            .unwrap(),
    );
    assert_eq!(
        reopened.message_skills(chat, original).await.unwrap(),
        draft.skills
    );
    assert_eq!(
        reopened.message_skills(chat, accepted).await.unwrap(),
        draft.skills
    );
    assert_eq!(
        reopened.accepted_turn_inputs(chat, original).await.unwrap(),
        [accepted]
    );
    assert!(
        reopened
            .message_skills(Uuid::new_v4(), original)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn editing_queue_preserves_an_existing_draft_and_rolls_back_failed_transfer() {
    let (temp, db, chat) = database().await;
    let queued = draft(temp.path(), "queued");
    let current = draft(temp.path(), "current");
    let enqueuer = EnqueueService::<SqliteUnitOfWork>::new(db.clone(), db.clone());
    let id = Uuid::new_v4();
    enqueuer
        .enqueue(&QueuedMessage {
            id,
            chat_id: chat,
            draft: queued.clone(),
            state: "pending".into(),
            error: None,
        })
        .await
        .unwrap();
    db.save_draft(chat, &current).await.unwrap();
    assert!(db.edit_queued(chat, id).await.unwrap().is_none());
    assert_eq!(db.draft(chat).await.unwrap(), current);
    assert_eq!(db.queue(chat).await.unwrap().entries[0].draft, queued);
    db.save_draft(chat, &Draft::default()).await.unwrap();
    sqlx::query("CREATE TRIGGER refuse_edited_draft BEFORE UPDATE ON drafts BEGIN SELECT RAISE(ABORT, 'test disk failure'); END;").execute(db.pool()).await.unwrap();
    assert!(db.edit_queued(chat, id).await.is_err());
    assert_eq!(db.queue(chat).await.unwrap().entries[0].draft, queued);
}
