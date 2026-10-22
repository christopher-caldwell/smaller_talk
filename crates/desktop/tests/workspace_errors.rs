use smaller_talk_application::*;
use smaller_talk_database::*;
use smaller_talk_domain::*;
use std::{error::Error, path::Path, sync::Arc};
use uuid::Uuid;

async fn store(path: &Path) -> Arc<SqliteChatStore> {
    create_empty_database(path).await.unwrap();
    migrate_existing(path).await.unwrap();
    Arc::new(SqliteChatStore::new(open_existing(path).await.unwrap()))
}

fn sqlite_source<'a>(error: &'a (dyn Error + 'static)) -> &'a sqlx::Error {
    error
        .source()
        .expect("use-case errors retain the capability failure")
        .downcast_ref::<PersistenceError>()
        .expect("persistence failure stays application-owned")
        .source()
        .expect("persistence errors retain the adapter diagnostic")
        .downcast_ref::<sqlx::Error>()
        .expect("the original SQLite adapter error is preserved")
}

#[tokio::test]
async fn workspace_validation_and_missing_targets_have_operation_specific_errors() {
    let temp = tempfile::tempdir().unwrap();
    let db = store(&temp.path().join("db.sqlite3")).await;
    let service = WorkspaceService::new(db.clone(), db.clone());
    let mut project = Project {
        id: Uuid::new_v4(),
        name: " \t ".into(),
        folder: None,
    };
    assert!(matches!(
        service.save_project(project.clone()).await,
        Err(SaveProjectError::EmptyName)
    ));
    project.name = "Example".into();
    let missing_folder = temp.path().join("does-not-exist");
    project.folder = Some(missing_folder.clone());
    assert!(matches!(
        service.save_project(project.clone()).await,
        Err(SaveProjectError::MissingFolder(path)) if path == missing_folder.display().to_string()
    ));
    assert!(matches!(
        service.remove_project(project.id).await,
        Err(RemoveProjectError::NotFound)
    ));
    assert!(matches!(
        service
            .save_preferences(Uuid::new_v4(), ChatPreferences::default())
            .await,
        Err(SaveChatPreferencesError::NotFound)
    ));
    let chat = ChatService::new(db.clone(), db.clone())
        .create_empty_chat()
        .await
        .unwrap();
    assert!(matches!(
        service
            .save_preferences(
                chat.id,
                ChatPreferences {
                    project_id: Some(project.id.to_string()),
                    ..Default::default()
                }
            )
            .await,
        Err(SaveChatPreferencesError::NotFound)
    ));
    assert!(db.list_projects().await.unwrap().is_empty());
    assert_eq!(
        db.preferences(chat.id).await.unwrap(),
        ChatPreferences::default()
    );
}

#[tokio::test]
async fn independent_query_and_write_errors_keep_the_original_adapter_source() {
    let temp = tempfile::tempdir().unwrap();
    let db = store(&temp.path().join("db.sqlite3")).await;
    let service = WorkspaceService::new(db.clone(), db.clone());
    db.pool().close().await;

    let read: ListProjectsError = service.list_projects().await.unwrap_err();
    let draft: LoadDraftError = service.draft(Uuid::new_v4()).await.unwrap_err();
    let write: SaveSessionError = service
        .save_session(&SessionState::default())
        .await
        .unwrap_err();
    for error in [&read as &dyn Error, &draft, &write] {
        assert!(matches!(sqlite_source(error), sqlx::Error::PoolClosed));
        assert!(error.to_string().starts_with("Workspace storage failed:"));
    }
}

#[tokio::test]
async fn project_write_failure_preserves_sqlite_diagnostics_and_does_not_save_project() {
    let temp = tempfile::tempdir().unwrap();
    let db = store(&temp.path().join("db.sqlite3")).await;
    sqlx::query("CREATE TRIGGER fail_project BEFORE INSERT ON projects BEGIN SELECT RAISE(FAIL, 'project write rejected'); END")
        .execute(db.pool())
        .await
        .unwrap();
    let service = WorkspaceService::new(db.clone(), db.clone());
    let error: SaveProjectError = service
        .save_project(Project {
            id: Uuid::new_v4(),
            name: "Example".into(),
            folder: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        sqlite_source(&error),
        sqlx::Error::Database(diagnostic) if diagnostic.message() == "project write rejected"
    ));
    assert!(db.list_projects().await.unwrap().is_empty());
}
