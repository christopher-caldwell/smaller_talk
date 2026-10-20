//! Explicit opt-in authenticated smoke test. Uses only synthetic text and a generated test image.
use smaller_talk_application::*;
use smaller_talk_database::*;
use smaller_talk_domain::*;
use smaller_talk_infrastructure::*;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

async fn turn(
    service: Arc<ConversationService>,
    chat_id: Uuid,
    prompt: &str,
    attachments: Vec<Attachment>,
) -> String {
    let (output, mut events) = mpsc::unbounded_channel();
    let (responses, receiver) = mpsc::unbounded_channel();
    let input = SendTurnInput {
        chat_id,
        prompt: prompt.into(),
        attachments,
        retry_of: None,
    };
    let task = tokio::spawn(async move { service.send_turn(input, output, receiver).await });
    let mut answer = String::new();
    while let Some(event) = events.recv().await {
        match event {
            ConversationEvent::AssistantDelta { delta, .. } => answer.push_str(&delta),
            ConversationEvent::AssistantSnapshot { text, .. } => answer = text,
            ConversationEvent::Interaction { interaction, .. } => {
                // These prompts need no tools or clarification. Decline any unexpected action.
                responses
                    .send(InteractionResponse {
                        id: interaction.id,
                        approved: false,
                        ..Default::default()
                    })
                    .unwrap();
            }
            _ => {}
        }
    }
    task.await.unwrap().unwrap();
    answer
}

#[tokio::test]
#[ignore = "Requires authenticated Codex and Claude CLIs; set SMALLER_TALK_LIVE=1 to explicitly opt in"]
async fn authenticated_codex_claude_codex_context_images_and_plan_mode() {
    assert_eq!(std::env::var("SMALLER_TALK_LIVE").as_deref(), Ok("1"));
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("workspace");
    std::fs::create_dir(&folder).unwrap();
    let path = temp.path().join("chats.sqlite3");
    create_empty_database(&path).await.unwrap();
    migrate_existing(&path).await.unwrap();
    let db = Arc::new(SqliteChatStore::new(open_existing(&path).await.unwrap()));
    let router = AgentRouter::new(
        CodexAdapter::new(
            std::env::var("SMALLER_TALK_CODEX_BIN").unwrap_or_else(|_| "codex".into()),
        ),
        ClaudeAdapter::new(
            std::env::var("SMALLER_TALK_CLAUDE_BIN").unwrap_or_else(|_| "claude".into()),
        ),
    );
    let service = Arc::new(
        ConversationService::new(
            db.clone(),
            db.clone(),
            db.clone(),
            db.clone(),
            router.clone(),
            folder.clone(),
        )
        .with_workspace(db.clone(), db.clone()),
    );
    let workspace = WorkspaceService::new(db.clone(), db.clone());
    let chat = ChatService::new(db.clone(), db.clone())
        .create_empty_chat()
        .await
        .unwrap();
    let source = temp.path().join("blue-square.png");
    std::fs::write(&source, include_bytes!("fixtures/blue-square.png")).unwrap();
    let attachments = LocalAttachments::new(temp.path().join("attachments"))
        .import(vec![source])
        .await
        .unwrap();
    for (index, provider) in [Provider::Codex, Provider::Claude, Provider::Codex]
        .into_iter()
        .enumerate()
    {
        let catalog = router.capabilities(provider).await.unwrap();
        let model = catalog
            .models
            .iter()
            .find(|m| {
                if provider == Provider::Codex {
                    m.id.contains("luna")
                } else {
                    m.id.contains("haiku")
                }
            })
            .or_else(|| catalog.models.iter().find(|m| m.is_default))
            .unwrap();
        let effort = model.efforts.iter().find(|e| *e == "low").cloned();
        let mut prefs = db.preferences(chat.id).await.unwrap();
        prefs.execution = ExecutionSettings {
            provider,
            model: Some(model.id.clone()),
            effort,
            plan_mode: index < 2,
        };
        workspace.save_preferences(chat.id, prefs).await.unwrap();
        let prompt = match index {
            0 => {
                "Do not use tools or change any files. Remember the token violet-sparrow-27. In one short sentence, say the color of the attached image and repeat that token."
            }
            1 => {
                "Do not use tools or change files. From our saved conversation and its attached image, repeat the remembered token and the image color. One short sentence."
            }
            _ => {
                "Do not use tools or change files. Repeat the remembered token and image color only."
            }
        };
        let answer = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            turn(
                service.clone(),
                chat.id,
                prompt,
                if index == 0 {
                    attachments.clone()
                } else {
                    vec![]
                },
            ),
        )
        .await
        .unwrap();
        println!(
            "{} / {} / plan={}: {}",
            provider.label(),
            model.id,
            index < 2,
            answer.trim()
        );
        assert!(answer.contains("violet-sparrow-27"));
        assert!(answer.to_lowercase().contains("blue"));
    }
    assert_eq!(db.list_messages(chat.id).await.unwrap().len(), 6);
    assert_eq!(std::fs::read_dir(folder).unwrap().count(), 0);
}
