use smaller_talk_application::*;
use smaller_talk_database::*;
use smaller_talk_domain::*;
use smaller_talk_infrastructure::*;
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
#[ignore = "Requires authenticated Claude CLI; set SMALLER_TALK_LIVE_REWRITE=1. Uses synthetic answers only."]
async fn synthetic_fidelity_corpus_with_real_isolated_claude() {
    assert_eq!(
        std::env::var("SMALLER_TALK_LIVE_REWRITE").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.sqlite3");
    create_empty_database(&path).await.unwrap();
    migrate_existing(&path).await.unwrap();
    let db = Arc::new(SqliteChatStore::new(open_existing(&path).await.unwrap()));
    let chats = ChatService::new(db.clone(), db.clone());
    let claude = std::env::var_os("SMALLER_TALK_CLAUDE_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "claude".into());
    let router = AgentRouter::new(CodexAdapter::new("codex"), ClaudeAdapter::new(claude));
    let generator = Arc::new(CliTextGenerator::new(
        router,
        temp.path().join("rewrite"),
        vec![],
    ));
    generator.support(Provider::Claude).await.unwrap();
    let service = RewriteService::new(generator, db.clone(), db.clone());
    let model = std::env::var("SMALLER_TALK_REWRITE_MODEL").unwrap_or_else(|_| "haiku".into());
    let settings = ExecutionSettings {
        provider: Provider::Claude,
        model: Some(model),
        ..Default::default()
    };
    let cases = [
        (
            "behavior",
            "`load_items()` reads `items.json`, removes disabled entries, and returns their IDs. A missing file produces `NotFound`; invalid JSON produces `ParseError`. The function does not write files.",
        ),
        (
            "review",
            "Two findings remain. A high severity bug lets `save_user()` overwrite an existing account when `id == 0`. A medium severity bug leaves stale entries in `user_cache`. Both need fixes. No tests were run.",
        ),
        (
            "uncertainty",
            "The likely cause is an expired token, but this is unverified. The only evidence is `401` from `/api/items`; no server logs were inspected. Refresh the token, then retry. Do not claim the root cause is confirmed.",
        ),
        (
            "completion",
            "Changed `src/cache.rs` to invalidate cached entries after updates. Ran `cargo test --locked`: 12 passed, 0 failed. The concurrent update case remains untested.",
        ),
        (
            "blocker",
            "The migration is blocked because `DATABASE_URL` is missing. No migration was run and no database data changed. Set `DATABASE_URL`, then run `cargo run -- db-migrate`.",
        ),
        (
            "evidence",
            "The measured latency was 240 ms across 5 requests. See [evidence](https://example.invalid/report).\n\n```sh\ncargo test --locked\n```\n\nThese measurements do not establish production performance.",
        ),
    ];
    for (name, facts) in cases {
        let chat = chats.create_empty_chat().await.unwrap();
        let source = format!(
            "Let me walk you through what I found in a clear and detailed way. I will begin with the main result, then explain the result again so the result is easy to follow. {facts} To summarize the explanation above, the important details are the details already stated above, and those details describe the result."
        );
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
            text: source.clone(),
        })
        .await
        .unwrap();
        let result = service
            .shorten(ShortenInput {
                chat_id: chat.id,
                message_id: message.id,
                settings: settings.clone(),
            })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(db.list_messages(chat.id).await.unwrap()[0].body, source);
        println!(
            "CASE {name}: {} -> {} bytes\n{}\n",
            source.len(),
            result.text.len(),
            result.text
        );
    }
}
