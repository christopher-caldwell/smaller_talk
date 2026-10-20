mod composer;
mod interactions;
mod markdown;
mod rich_text;
mod shell;
mod startup;
mod theme;
mod transcript;

use anyhow::{Context as _, bail};
use gpui::{
    App, AppContext, Application, Bounds, Global, KeyBinding, Menu, MenuItem, WindowBounds,
    WindowOptions, actions, px, size,
};
use smaller_talk_application::*;
use smaller_talk_database::{
    SqliteChatStore, create_empty_database, migrate_existing, open_existing,
};
use smaller_talk_infrastructure::{AgentRouter, ClaudeAdapter, CodexAdapter, LocalAttachments};
use std::sync::Arc;
use theme::*;
use tokio::runtime::Builder;

actions!(
    smaller_talk,
    [
        Quit,
        CloseTab,
        NewChat,
        NextTab,
        PreviousTab,
        ToggleSidebar,
        SearchChats
    ]
);
struct Root(gpui::Entity<shell::ChatsView>);
impl Global for Root {}

fn open_window(cx: &mut App) {
    let view = cx.global::<Root>().0.clone();
    view.update(cx, |view, _| view.focus_composer = true);
    let bounds = Bounds::centered(None, size(px(1120.), px(780.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(480.), px(480.))),
            ..Default::default()
        },
        move |window, _| {
            window.set_window_title("Smaller Talk");
            view
        },
    )
    .expect("could not open Smaller Talk");
    cx.activate(true);
}

fn main() -> anyhow::Result<()> {
    let path = startup::database_path()?;
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not start the async runtime")?;
    match std::env::args().nth(1).as_deref() {
        Some("db-create") => {
            runtime.block_on(create_empty_database(&path))?;
            println!("Created {}", path.display());
            return Ok(());
        }
        Some("db-migrate") => {
            runtime.block_on(migrate_existing(&path))?;
            println!("Migrated {}", path.display());
            return Ok(());
        }
        Some("--help" | "-h") => {
            println!(
                "Smaller Talk\n\n  db-create   Create the local database\n  db-migrate  Apply schema migrations\n  (no args)   Open the app\n\nSMALLER_TALK_DATA_DIR, SMALLER_TALK_CODEX_BIN, SMALLER_TALK_CLAUDE_BIN override local paths."
            );
            return Ok(());
        }
        Some(other) => bail!("unknown command {other:?}; use --help"),
        None => {}
    }
    let store = Arc::new(SqliteChatStore::new(
        runtime.block_on(open_existing(&path)).with_context(|| {
            format!(
                "Could not open {}. Run `just db-setup` first.",
                path.display()
            )
        })?,
    ));
    let chats = Arc::new(ChatService::new(store.clone(), store.clone()));
    let workspace = Arc::new(WorkspaceService::new(store.clone(), store.clone()));
    let initial_chats = runtime
        .block_on(chats.list_chats())
        .context("Could not load chats; run `just db-migrate`.")?;
    runtime
        .block_on(workspace.recover_interrupted_turns())
        .context("Could not recover interrupted turns; run `just db-migrate`.")?;
    let session = runtime
        .block_on(workspace.session())
        .context("Could not load the workspace; run `just db-migrate`.")?;
    let projects = runtime.block_on(workspace.list_projects())?;
    let router = AgentRouter::new(
        CodexAdapter::new(startup::codex_executable()),
        ClaudeAdapter::new(startup::claude_executable()),
    );
    let conversation = Arc::new(
        ConversationService::new(
            store.clone(),
            store.clone(),
            store.clone(),
            store.clone(),
            router,
            std::env::current_dir()?,
        )
        .with_workspace(store.clone(), store.clone()),
    );
    let attachment_storage: Arc<dyn AttachmentStorage> = Arc::new(LocalAttachments::new(
        path.parent()
            .context("database has no parent")?
            .canonicalize()?
            .join("attachments"),
    ));
    runtime.block_on(
        attachment_storage
            .collect_unreferenced(runtime.block_on(workspace.attachment_references())?),
    )?;
    let services = shell::Services {
        chats,
        conversation,
        workspace,
        attachments: attachment_storage,
        runtime: runtime.handle().clone(),
    };
    let app = Application::new().with_assets(theme::Icons);
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_window(cx);
        } else {
            cx.activate(true);
        }
    });
    app.run(move |cx: &mut App| {
        composer::bind_keys(cx);
        rich_text::bind_keys(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseTab, None),
            KeyBinding::new("cmd-t", NewChat, None),
            KeyBinding::new("cmd-n", NewChat, None),
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
            KeyBinding::new("cmd-shift-bracketright", NextTab, None),
            KeyBinding::new("cmd-shift-bracketleft", PreviousTab, None),
            KeyBinding::new("cmd-b", ToggleSidebar, None),
            KeyBinding::new("cmd-f", SearchChats, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "Smaller Talk".into(),
            items: vec![
                MenuItem::action("New Chat", NewChat),
                MenuItem::action("Search Chats", SearchChats),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Close Tab", CloseTab),
                MenuItem::separator(),
                MenuItem::action("Quit Smaller Talk", Quit),
            ],
        }]);
        let view =
            cx.new(|cx| shell::ChatsView::new(services, initial_chats, projects, session, cx));
        cx.set_global(Root(view));
        let root = cx.global::<Root>().0.clone();
        cx.on_app_quit(move |cx| {
            let receiver = root.update(cx, |view, cx| view.persist_all(cx));
            async move {
                let _ = receiver.await;
            }
        })
        .detach();
        open_window(cx);
    });
    Ok(())
}
