use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, bail};
use gpui::{
    App, Application, Bounds, Context, KeyBinding, Menu, MenuItem, Render, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, rgb, size,
};
use smaller_talk_application::{ChatQueries, ChatService, ChatWrites, DeleteChatError};
use smaller_talk_database::{
    SqliteChatStore, create_empty_database, migrate_existing, open_existing,
};
use smaller_talk_domain::Chat;
use tokio::{runtime::Builder, sync::oneshot};
use uuid::Uuid;

actions!(smaller_talk, [CloseWindow, Quit]);

const BACKGROUND: u32 = 0x111318;
const PANEL: u32 = 0x191c22;
const PANEL_HOVER: u32 = 0x232730;
const ACCENT: u32 = 0x84d7c1;
const TEXT: u32 = 0xf1f2f4;
const MUTED: u32 = 0x969ca8;
const DANGER: u32 = 0xff8d91;

fn database_path() -> anyhow::Result<PathBuf> {
    if let Some(data_dir) = std::env::var_os("SMALLER_TALK_DATA_DIR") {
        return Ok(PathBuf::from(data_dir).join("chats.sqlite3"));
    }

    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Application Support/SmallerTalk/chats.sqlite3"))
}

fn main() -> anyhow::Result<()> {
    let command = std::env::args().nth(1);
    let path = database_path()?;
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not start the SQLite async runtime")?;

    match command.as_deref() {
        Some("db-create") => {
            runtime
                .block_on(create_empty_database(&path))
                .with_context(|| format!("could not create {}", path.display()))?;
            println!("Empty SQLite database created at {}", path.display());
            return Ok(());
        }
        Some("db-migrate") => {
            runtime
                .block_on(migrate_existing(&path))
                .with_context(|| format!("could not migrate {}", path.display()))?;
            println!("SQLite schema is ready at {}", path.display());
            return Ok(());
        }
        Some("--help" | "-h") => {
            println!(
                "Smaller Talk\n\nCommands:\n  smaller-talk-desktop db-create   Create the local SQLite database file\n  smaller-talk-desktop db-migrate  Apply schema migrations to the existing database\n  smaller-talk-desktop              Open the chat window"
            );
            return Ok(());
        }
        Some(other) => bail!("unknown command {other:?}; use --help"),
        None => {}
    }

    let pool = runtime.block_on(open_existing(&path)).with_context(|| {
        format!(
            "could not open {}; run `cargo run -p smaller-talk-desktop -- db-create` and then `cargo run -p smaller-talk-desktop -- db-migrate` first",
            path.display()
        )
    })?;
    let store = Arc::new(SqliteChatStore::new(pool));
    let queries: Arc<dyn ChatQueries> = store.clone();
    let writes: Arc<dyn ChatWrites> = store.clone();
    let service = Arc::new(ChatService::new(queries, writes));
    let chats = runtime
        .block_on(service.list_chats())
        .context("could not load chats; run `cargo run -p smaller-talk-desktop -- db-migrate`")?;
    let tokio = runtime.handle().clone();
    let shared_state = Arc::new(Mutex::new(ChatSnapshot {
        selected_chat: chats.first().map(|chat| chat.id),
        chats,
    }));
    let app = Application::new();
    let reopen_service = service.clone();
    let reopen_tokio = tokio.clone();
    let reopen_state = shared_state.clone();
    app.on_reopen(move |cx| {
        if cx.windows().is_empty() {
            open_chat_window(
                cx,
                reopen_service.clone(),
                reopen_tokio.clone(),
                reopen_state.clone(),
            );
        } else {
            cx.activate(true);
        }
    });

    app.run(move |cx: &mut App| {
        cx.on_action(quit);
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "Smaller Talk".into(),
            items: vec![MenuItem::action("Quit Smaller Talk", Quit)],
        }]);
        open_chat_window(cx, service, tokio, shared_state);
    });

    Ok(())
}

fn quit(_: &Quit, cx: &mut App) {
    cx.quit();
}

#[derive(Clone, Default)]
struct ChatSnapshot {
    chats: Vec<Chat>,
    selected_chat: Option<Uuid>,
}

fn open_chat_window(
    cx: &mut App,
    service: Arc<ChatService>,
    tokio: tokio::runtime::Handle,
    shared_state: Arc<Mutex<ChatSnapshot>>,
) {
    let snapshot = shared_state
        .lock()
        .expect("chat snapshot mutex poisoned")
        .clone();
    let bounds = Bounds::centered(None, size(px(1000.0), px(680.0)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        },
        move |_, cx| {
            cx.new(|_| ChatsView {
                service,
                tokio,
                chats: snapshot.chats,
                selected_chat: snapshot.selected_chat,
                shared_state,
                working: false,
                error: None,
            })
        },
    )
    .expect("could not open Smaller Talk window");
    cx.activate(true);
}

struct ChatsView {
    service: Arc<ChatService>,
    tokio: tokio::runtime::Handle,
    chats: Vec<Chat>,
    selected_chat: Option<Uuid>,
    shared_state: Arc<Mutex<ChatSnapshot>>,
    working: bool,
    error: Option<String>,
}

fn chat_element_id(kind: &'static str, id: Uuid) -> gpui::ElementId {
    let (high, low) = id.as_u64_pair();
    (gpui::ElementId::from((kind, high)), low.to_string()).into()
}

impl ChatsView {
    fn create_chat(&mut self, cx: &mut Context<Self>) {
        if self.working {
            return;
        }
        self.working = true;
        self.error = None;
        let service = self.service.clone();
        let tokio = self.tokio.clone();

        cx.spawn(async move |this, cx| {
            let (sender, receiver) = oneshot::channel();
            tokio.spawn(async move {
                let _ = sender.send(service.create_empty_chat().await);
            });
            let result = receiver.await;
            let _ = this.update(cx, |view, cx| {
                view.working = false;
                match result {
                    Ok(Ok(chat)) => {
                        view.selected_chat = Some(chat.id);
                        view.chats.insert(0, chat);
                        view.save_snapshot();
                    }
                    Ok(Err(error)) => view.error = Some(error.to_string()),
                    Err(_) => view.error = Some("The database task stopped unexpectedly.".into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn delete_chat(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.working {
            return;
        }
        self.working = true;
        self.error = None;
        let service = self.service.clone();
        let tokio = self.tokio.clone();

        cx.spawn(async move |this, cx| {
            let (sender, receiver) = oneshot::channel();
            tokio.spawn(async move {
                let _ = sender.send(service.delete_chat(id).await);
            });
            let result = receiver.await;
            let _ = this.update(cx, |view, cx| {
                view.working = false;
                match result {
                    Ok(Ok(())) | Ok(Err(DeleteChatError::NotFound)) => {
                        view.chats.retain(|chat| chat.id != id);
                        if view.selected_chat == Some(id) {
                            view.selected_chat = view.chats.first().map(|chat| chat.id);
                        }
                        view.save_snapshot();
                    }
                    Ok(Err(error)) => view.error = Some(error.to_string()),
                    Err(_) => view.error = Some("The database task stopped unexpectedly.".into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select_chat(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.selected_chat = Some(id);
        self.error = None;
        self.save_snapshot();
        cx.notify();
    }

    fn save_snapshot(&self) {
        let mut snapshot = self
            .shared_state
            .lock()
            .expect("chat snapshot mutex poisoned");
        snapshot.chats.clone_from(&self.chats);
        snapshot.selected_chat = self.selected_chat;
    }
}

impl Render for ChatsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self
            .selected_chat
            .and_then(|id| self.chats.iter().find(|chat| chat.id == id));

        let mut sidebar = div()
            .w(px(280.0))
            .h_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .bg(rgb(PANEL))
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(TEXT))
                    .child("Smaller Talk"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("Your local conversations"),
            )
            .child(
                div()
                    .id("create-chat")
                    .mt_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(ACCENT))
                    .text_color(rgb(0x101715))
                    .cursor_pointer()
                    .child(if self.working {
                        "Working…"
                    } else {
                        "+  New chat"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.create_chat(cx))),
            );

        let mut chat_list = div()
            .id("chat-list")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        for chat in &self.chats {
            let id = chat.id;
            let is_selected = self.selected_chat == Some(id);
            let title = chat.title.clone();
            let date = chat
                .created_at
                .with_timezone(&chrono::Local)
                .format("%b %-d, %-I:%M:%S %p")
                .to_string();
            let row_background = if is_selected { PANEL_HOVER } else { PANEL };
            chat_list = chat_list.child(
                div()
                    .id(chat_element_id("chat-row", id))
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .bg(rgb(row_background))
                    .child(
                        div()
                            .id(chat_element_id("select-chat", id))
                            .flex_1()
                            .cursor_pointer()
                            .child(div().text_color(rgb(TEXT)).child(title))
                            .child(div().text_xs().text_color(rgb(MUTED)).child(date))
                            .on_click(cx.listener(move |this, _, _, cx| this.select_chat(id, cx))),
                    )
                    .child(
                        div()
                            .id(chat_element_id("delete-chat", id))
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .text_color(rgb(DANGER))
                            .cursor_pointer()
                            .child("Delete")
                            .on_click(cx.listener(move |this, _, _, cx| this.delete_chat(id, cx))),
                    ),
            );
        }
        sidebar = sidebar.child(chat_list);

        let mut detail = div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .p_8()
            .gap_4()
            .bg(rgb(BACKGROUND));

        if let Some(chat) = selected {
            detail = detail
                .child(
                    div()
                        .text_2xl()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT))
                        .child(chat.title.clone()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child("This conversation is ready for its first message."),
                )
                .child(
                    div()
                        .flex_1()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(MUTED))
                        .child("No messages yet"),
                );
        } else if self.chats.is_empty() {
            detail = detail.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(rgb(MUTED))
                    .child("No chats yet")
                    .child("Create a local chat to get started."),
            );
        } else {
            detail = detail.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(MUTED))
                    .child("Select a chat to view it."),
            );
        }

        if let Some(error) = &self.error {
            detail = detail.child(div().text_color(rgb(DANGER)).child(error.clone()));
        }

        div()
            .flex()
            .size_full()
            .bg(rgb(BACKGROUND))
            .child(sidebar)
            .child(detail)
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
    }
}
