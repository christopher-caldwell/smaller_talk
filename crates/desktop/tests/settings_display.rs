use async_trait::async_trait;
use smaller_talk_application::*;
use smaller_talk_database::*;
use smaller_talk_domain::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use uuid::Uuid;
const FILLER: &str = "Here is a long introduction which adds nothing useful. ";
async fn setup() -> (tempfile::TempDir, Arc<SqliteChatStore>, Uuid, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite3");
    create_empty_database(&path).await.unwrap();
    migrate_existing(&path).await.unwrap();
    let db = Arc::new(SqliteChatStore::new(open_existing(&path).await.unwrap()));
    let chat = ChatService::new(db.clone(), db.clone())
        .create_empty_chat()
        .await
        .unwrap();
    let source = format!("{FILLER}The result is uncertain. Run `cargo test` before proceeding.");
    let message = db
        .append_message(NewMessage {
            id: Uuid::new_v4(),
            chat_id: chat.id,
            role: MessageRole::Assistant,
            body: source.clone(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    db.save_final_answer(&FinalAnswer {
        message_id: message.id,
        text: source,
    })
    .await
    .unwrap();
    (dir, db, chat.id, message.id)
}

#[tokio::test]
async fn chat_creation_rolls_back_when_default_preferences_cannot_be_saved() {
    let (_dir, db, existing_chat, _) = setup().await;
    ChatService::new(db.clone(), db.clone())
        .delete_chat(existing_chat)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_default_preferences BEFORE INSERT ON chat_settings BEGIN SELECT RAISE(FAIL, 'injected preference failure'); END")
        .execute(db.pool())
        .await
        .unwrap();
    let chats = ChatService::new(db.clone(), db.clone()).with_setup_store(db.clone());
    assert!(
        chats
            .create_chat_with_defaults(Some(ExecutionSettings::default()))
            .await
            .is_err()
    );
    assert!(chats.list_chats().await.unwrap().is_empty());
}
struct Generator {
    calls: AtomicUsize,
    corrupt: bool,
}
#[async_trait]
impl TextGenerator for Generator {
    async fn support(&self, _: Provider) -> Result<(), ExecutionError> {
        Ok(())
    }
    async fn generate(&self, input: TextGenerationInput) -> Result<String, ExecutionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(input.instructions.contains("verification status"));
        assert!(!input.text.contains("cargo test"));
        tokio::task::yield_now().await;
        Ok(if self.corrupt {
            "Success, all 999 tests passed.".into()
        } else {
            input.text.replacen(FILLER, "", 1)
        })
    }
}
fn settings() -> ExecutionSettings {
    ExecutionSettings {
        provider: Provider::Claude,
        model: Some("fixture".into()),
        ..Default::default()
    }
}
#[tokio::test]
async fn global_settings_survive_restart_without_changing_chat_selections() {
    let (dir, db, chat, _) = setup().await;
    let old = ChatPreferences {
        execution: ExecutionSettings {
            provider: Provider::Claude,
            model: Some("old".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    WorkspaceService::new(db.clone(), db.clone())
        .save_preferences(chat, old.clone())
        .await
        .unwrap();
    let service = SettingsService::new(db.clone(), db.clone());
    let mut preferences = AppSettings {
        default_execution: Some(settings()),
        rewrite_model: Some(settings()),
        ..Default::default()
    };
    preferences
        .providers
        .entry(Provider::Codex)
        .or_default()
        .enabled = false;
    preferences
        .providers
        .entry(Provider::Claude)
        .or_default()
        .hidden_models
        .insert("hidden".into());
    service.save(&preferences).await.unwrap();
    let reopened =
        SqliteChatStore::new(open_existing(&dir.path().join("db.sqlite3")).await.unwrap());
    assert_eq!(reopened.app_settings().await.unwrap(), preferences);
    let current = reopened.preferences(chat).await.unwrap();
    assert_eq!(current.execution, old.execution);
    let invalid = AppSettings {
        automatic_shortening: true,
        ..Default::default()
    };
    assert!(service.save(&invalid).await.is_err());
}
#[tokio::test]
async fn duplicate_rewrites_use_one_job_and_never_replace_original_history() {
    let (_dir, db, chat, message) = setup().await;
    let generator = Arc::new(Generator {
        calls: AtomicUsize::new(0),
        corrupt: false,
    });
    let service = RewriteService::new(generator.clone(), db.clone(), db.clone());
    let (a, b) = tokio::join!(
        service.shorten(ShortenInput {
            chat_id: chat,
            message_id: message,
            settings: settings()
        }),
        service.shorten(ShortenInput {
            chat_id: chat,
            message_id: message,
            settings: settings()
        })
    );
    let a = a.unwrap();
    assert_eq!(a, b.unwrap());
    assert_eq!(generator.calls.load(Ordering::SeqCst), 1);
    assert!(!a.text.contains(FILLER));
    assert!(a.text.contains("uncertain"));
    assert!(a.text.contains("`cargo test`"));
    assert!(
        db.list_messages(chat).await.unwrap()[0]
            .body
            .contains(FILLER)
    );
    assert_eq!(
        db.final_answers(chat).await.unwrap()[0].text,
        db.list_messages(chat).await.unwrap()[0].body
    );
    let hits = WorkspaceService::new(db.clone(), db.clone())
        .search("long introduction")
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    ChatService::new(db.clone(), db.clone())
        .delete_chat(chat)
        .await
        .unwrap();
    assert!(db.variants(chat).await.unwrap().is_empty());
}
#[tokio::test]
async fn malformed_rewrites_are_rejected_without_saving_a_variant() {
    let (_dir, db, chat, message) = setup().await;
    let service = RewriteService::new(
        Arc::new(Generator {
            calls: AtomicUsize::new(0),
            corrupt: true,
        }),
        db.clone(),
        db.clone(),
    );
    assert!(
        service
            .shorten(ShortenInput {
                chat_id: chat,
                message_id: message,
                settings: settings()
            })
            .await
            .is_err()
    );
    assert!(db.variants(chat).await.unwrap().is_empty());
    assert!(
        db.list_messages(chat).await.unwrap()[0]
            .body
            .contains(FILLER)
    );
}

struct WaitingGenerator {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait]
impl TextGenerator for WaitingGenerator {
    async fn support(&self, _: Provider) -> Result<(), ExecutionError> {
        Ok(())
    }
    async fn generate(&self, input: TextGenerationInput) -> Result<String, ExecutionError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(input.text.replacen(FILLER, "", 1))
    }
}
#[tokio::test]
async fn stale_sources_are_rejected_and_timed_out_jobs_release_the_worker() {
    let (_dir, db, chat, message) = setup().await;
    let generator = Arc::new(WaitingGenerator {
        started: Default::default(),
        release: Default::default(),
    });
    let service = Arc::new(RewriteService::new(
        generator.clone(),
        db.clone(),
        db.clone(),
    ));
    let worker = service.clone();
    let task = tokio::spawn(async move {
        worker
            .shorten(ShortenInput {
                chat_id: chat,
                message_id: message,
                settings: settings(),
            })
            .await
    });
    generator.started.notified().await;
    db.save_final_answer(&FinalAnswer {
        message_id: message,
        text: format!("{FILLER}Different source `run()`."),
    })
    .await
    .unwrap();
    generator.release.notify_one();
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    assert!(db.variants(chat).await.unwrap().is_empty());
    let worker = service.clone();
    let task = tokio::spawn(async move {
        worker
            .shorten(ShortenInput {
                chat_id: chat,
                message_id: message,
                settings: settings(),
            })
            .await
    });
    generator.started.notified().await;
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    tokio::time::resume();
    let worker = service.clone();
    let task = tokio::spawn(async move {
        worker
            .shorten(ShortenInput {
                chat_id: chat,
                message_id: message,
                settings: settings(),
            })
            .await
    });
    generator.started.notified().await;
    generator.release.notify_one();
    assert!(task.await.unwrap().is_ok());
}
