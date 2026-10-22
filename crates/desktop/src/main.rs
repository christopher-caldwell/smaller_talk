mod benchmark;
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
    SqliteChatStore, SqliteUnitOfWork, create_empty_database, migrate_existing, open_existing,
};
use smaller_talk_infrastructure::{AgentRouter, ClaudeAdapter, CodexAdapter, LocalAttachments};
use std::sync::Arc;
use theme::*;
use tokio::runtime::Builder;

// Separate Rust's transient transcript allocations from long-lived AppKit
// allocations, and return freed pages without waiting for system pressure.
#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(target_os = "macos")]
fn reclaim_transcript_memory() -> std::time::Duration {
    let started = std::time::Instant::now();
    // SAFETY: mimalloc permits collection of the current thread's heap at any
    // time. It returns only unused pages and preserves all live allocations.
    unsafe { libmimalloc_sys::mi_collect(true) };
    started.elapsed()
}

actions!(
    smaller_talk,
    [
        Quit,
        CloseTab,
        NewChat,
        NextTab,
        PreviousTab,
        ToggleSidebar,
        SearchChats,
        OpenSettings
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
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next();
    let benchmark = if command.as_deref() == Some("benchmark") {
        let directory = arguments
            .next()
            .context("benchmark requires its run directory")?;
        if arguments.next().is_some() {
            bail!("unexpected benchmark argument");
        }
        Some(benchmark::Session::start(std::path::Path::new(&directory))?)
    } else {
        None
    };
    let path = startup::database_path()?;
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not start the async runtime")?;
    match command.as_deref() {
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
        Some("benchmark") => {}
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
    let chats =
        Arc::new(ChatService::new(store.clone(), store.clone()).with_setup_writes(store.clone()));
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
    let settings = Arc::new(SettingsService::new(store.clone(), store.clone()));
    let instructions = Arc::new(InstructionService::new(
        store.clone(),
        store.clone(),
        store.clone(),
    ));
    let app_settings = runtime
        .block_on(settings.load())
        .context("Could not load settings; run `just db-migrate`.")?;
    let router = AgentRouter::new(
        CodexAdapter::new(startup::codex_executable()),
        ClaudeAdapter::new(startup::claude_executable()),
    );
    for provider in smaller_talk_domain::PROVIDERS {
        let prefs = app_settings.provider(provider);
        router.configure(
            provider,
            startup::provider_executable(provider, &prefs),
            prefs.enabled,
        );
    }
    router.configure_skill_locations(smaller_talk_infrastructure::SkillLocations {
        home: std::env::var_os("HOME").map(std::path::PathBuf::from),
        claude_config: std::env::var_os("CLAUDE_CONFIG_DIR").map(std::path::PathBuf::from),
    });
    let skills = Arc::new(SkillService::new(router.clone()));
    let conversation = Arc::new(
        ConversationService::new(
            store.clone(),
            store.clone(),
            store.clone(),
            store.clone(),
            router.clone(),
            std::env::current_dir()?,
        )
        .with_workspace(store.clone(), store.clone())
        .with_steered_input_writes(store.clone())
        .with_settings(store.clone())
        .with_instructions(instructions.clone(), store.clone())
        .with_skills(skills.clone(), store.clone())
        .with_display(store.clone()),
    );
    runtime.block_on(store.recover_queues())?;
    let (queue_output, queue_events) = tokio::sync::mpsc::unbounded_channel();
    let queue = runtime.block_on(async {
        QueueCoordinator::start(
            store.clone(),
            store.clone(),
            Arc::new(EnqueueService::<SqliteUnitOfWork>::new(
                store.clone(),
                store.clone(),
            )),
            conversation.clone(),
            queue_output,
        )
    });
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
    let rewriter = Arc::new(RewriteService::new(
        Arc::new(smaller_talk_infrastructure::CliTextGenerator::new(
            router.clone(),
            path.parent()
                .context("database has no parent")?
                .join("rewrite"),
            startup::managed_claude_policy_paths(),
        )),
        store.clone(),
        store.clone(),
    ));
    let services = shell::Services {
        skills,
        skill_queries: store.clone(),
        display: store.clone(),
        rewriter,
        settings,
        instructions,
        providers: router,
        repositories: Arc::new(smaller_talk_infrastructure::GitRepositoryReader::new(
            "git".into(),
            "gh".into(),
        )),
        launch_folder: std::env::current_dir()?,
        chats,
        conversation,
        queue,
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
            KeyBinding::new("cmd-,", OpenSettings, None),
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
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("New Chat", NewChat),
                MenuItem::action("Search Chats", SearchChats),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Close Tab", CloseTab),
                MenuItem::separator(),
                MenuItem::action("Quit Smaller Talk", Quit),
            ],
        }]);
        let view = cx.new(|cx| {
            shell::ChatsView::new(
                services,
                initial_chats,
                projects,
                session,
                app_settings,
                queue_events,
                cx,
            )
        });
        if let Some(benchmark) = benchmark {
            view.update(cx, |view, cx| view.attach_benchmark(benchmark, cx));
        }
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
