mod composer;
mod markdown;

use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, bail};
use gpui::{
    App, Application, Bounds, Context, Entity, KeyBinding, Menu, MenuItem, Render, ScrollHandle,
    StyledText, Window, WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size,
};
use smaller_talk_application::{
    ChatQueries, ChatService, ChatWrites, ConversationEvent, ConversationService, CreateChatError,
    DeleteChatError, ExecutionAdapter, MessageQueries, MessageWrites, SendMessageInput,
};
use smaller_talk_database::{
    SqliteChatStore, create_empty_database, migrate_existing, open_existing,
};
use smaller_talk_domain::{Chat, Message, MessageRole};
use smaller_talk_infrastructure::CodexAdapter;
use tokio::{
    runtime::Builder,
    sync::{oneshot, watch},
};
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

fn codex_executable() -> PathBuf {
    resolve_codex_executable_from(
        std::env::var_os("SMALLER_TALK_CODEX_BIN"),
        std::env::var_os("PATH"),
        std::env::var_os("HOME"),
    )
}

fn resolve_codex_executable_from(
    explicit: Option<OsString>,
    path: Option<OsString>,
    home: Option<OsString>,
) -> PathBuf {
    if let Some(path) = explicit {
        return PathBuf::from(path);
    }

    if let Some(path) = path {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join("codex");
            if is_executable_file(&candidate) {
                return candidate;
            }
        }
    }

    if let Some(home) = home {
        let home = PathBuf::from(home);
        for candidate in [
            home.join(".local/bin/codex"),
            PathBuf::from("/opt/homebrew/bin/codex"),
            PathBuf::from("/usr/local/bin/codex"),
        ] {
            if is_executable_file(&candidate) {
                return candidate;
            }
        }
        let nvm_bins = home.join(".nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(nvm_bins) {
            let mut candidates = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("bin/codex"))
                .filter(|path| is_executable_file(path))
                .collect::<Vec<_>>();
            candidates.sort();
            if let Some(candidate) = candidates.pop() {
                return candidate;
            }
        }
    }
    PathBuf::from("codex")
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
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
    let conversation_chats = queries.clone();
    let conversation_writes = writes.clone();
    let service = Arc::new(ChatService::new(queries, writes));
    let chats = runtime
        .block_on(service.list_chats())
        .context("could not load chats; run `cargo run -p smaller-talk-desktop -- db-migrate`")?;
    let messages: Arc<dyn MessageQueries> = store.clone();
    let message_writes: Arc<dyn MessageWrites> = store.clone();
    let execution_adapter: Arc<dyn ExecutionAdapter> =
        Arc::new(CodexAdapter::new(codex_executable()));
    let conversation_service = Arc::new(ConversationService::new(
        conversation_chats,
        conversation_writes,
        messages,
        message_writes,
        execution_adapter,
        std::env::current_dir().context("could not read the launch working directory")?,
    ));
    let initial_selected_chat = chats.first().map(|chat| chat.id);
    let initial_messages = match initial_selected_chat {
        Some(chat_id) => runtime
            .block_on(conversation_service.list_messages(chat_id))
            .context("could not load the selected chat transcript")?,
        None => Vec::new(),
    };
    let tokio = runtime.handle().clone();
    let (snapshot, _) = watch::channel(ChatSnapshot {
        selected_chat: initial_selected_chat,
        chats,
        messages: initial_messages.clone(),
        active_transcripts: HashMap::new(),
    });
    let shared_state = SharedChatState {
        snapshot,
        write_gate: Arc::new(tokio::sync::Mutex::new(())),
    };
    let app = Application::new();
    let reopen_service = service.clone();
    let reopen_conversation_service = conversation_service.clone();
    let reopen_tokio = tokio.clone();
    let reopen_state = shared_state.clone();
    app.on_reopen(move |cx| {
        if cx.windows().is_empty() {
            open_chat_window(
                cx,
                reopen_service.clone(),
                reopen_conversation_service.clone(),
                reopen_tokio.clone(),
                reopen_state.clone(),
            );
        } else {
            cx.activate(true);
        }
    });

    app.run(move |cx: &mut App| {
        composer::bind_keys(cx);
        cx.on_action(quit);
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "Smaller Talk".into(),
            items: vec![MenuItem::action("Quit Smaller Talk", Quit)],
        }]);
        open_chat_window(cx, service, conversation_service, tokio, shared_state);
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
    messages: Vec<Message>,
    active_transcripts: HashMap<Uuid, Vec<Message>>,
}

#[derive(Clone)]
struct SharedChatState {
    snapshot: watch::Sender<ChatSnapshot>,
    write_gate: Arc<tokio::sync::Mutex<()>>,
}

impl SharedChatState {
    fn publish_created_chat(&self, chat: Chat) {
        self.snapshot.send_modify(|snapshot| {
            snapshot.chats.push(chat.clone());
            snapshot.chats.sort_by(|left, right| {
                right
                    .created_at
                    .cmp(&left.created_at)
                    .then_with(|| right.id.cmp(&left.id))
            });
            snapshot.selected_chat = Some(chat.id);
            snapshot.messages.clear();
        });
    }

    fn publish_deleted_chat(&self, id: Uuid) {
        self.snapshot.send_modify(|snapshot| {
            snapshot.chats.retain(|chat| chat.id != id);
            snapshot.active_transcripts.remove(&id);
            if snapshot.selected_chat == Some(id) {
                snapshot.selected_chat = snapshot.chats.first().map(|chat| chat.id);
                snapshot.messages = snapshot
                    .selected_chat
                    .and_then(|chat_id| snapshot.active_transcripts.get(&chat_id).cloned())
                    .unwrap_or_default();
            }
        });
    }

    fn select_chat(&self, id: Uuid) -> ChatSnapshot {
        self.snapshot.send_modify(|snapshot| {
            if snapshot.chats.iter().any(|chat| chat.id == id) {
                if snapshot.selected_chat != Some(id) {
                    snapshot.selected_chat = Some(id);
                    snapshot.messages = snapshot
                        .active_transcripts
                        .get(&id)
                        .cloned()
                        .unwrap_or_default();
                }
            }
        });
        self.snapshot.borrow().clone()
    }

    fn publish_messages(&self, id: Uuid, messages: Vec<Message>) {
        self.snapshot.send_if_modified(|snapshot| {
            if snapshot.selected_chat != Some(id)
                || !snapshot.chats.iter().any(|chat| chat.id == id)
            {
                return false;
            }
            let mut merged = messages;
            if let Some(live_messages) = snapshot.active_transcripts.get(&id) {
                for live in live_messages {
                    if let Some(existing) = merged.iter_mut().find(|message| message.id == live.id)
                    {
                        *existing = live.clone();
                    } else {
                        merged.push(live.clone());
                    }
                }
            }
            merged.sort_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.id.cmp(&right.id))
            });
            snapshot.messages = merged;
            true
        });
    }

    fn publish_conversation_event(&self, event: ConversationEvent) {
        let chat_id = match &event {
            ConversationEvent::UserMessage(message)
            | ConversationEvent::AssistantMessage(message) => message.chat_id,
            ConversationEvent::AssistantDelta { chat_id, .. } => *chat_id,
        };
        self.snapshot.send_modify(|snapshot| match event {
            event => {
                if !snapshot.chats.iter().any(|chat| chat.id == chat_id) {
                    return;
                }
                let selected = snapshot.selected_chat == Some(chat_id);
                match event {
                    ConversationEvent::UserMessage(message) => {
                        upsert_message(
                            snapshot.active_transcripts.entry(chat_id).or_default(),
                            message.clone(),
                        );
                        if selected {
                            upsert_message(&mut snapshot.messages, message);
                        }
                    }
                    ConversationEvent::AssistantDelta {
                        chat_id,
                        message_id,
                        delta,
                    } => {
                        append_assistant_delta(
                            snapshot.active_transcripts.entry(chat_id).or_default(),
                            chat_id,
                            message_id,
                            delta.clone(),
                        );
                        if selected {
                            append_assistant_delta(
                                &mut snapshot.messages,
                                chat_id,
                                message_id,
                                delta,
                            );
                        }
                    }
                    ConversationEvent::AssistantMessage(message) => {
                        if selected {
                            upsert_message(&mut snapshot.messages, message);
                        }
                        snapshot.active_transcripts.remove(&chat_id);
                    }
                }
            }
        });
    }
}

fn upsert_message(messages: &mut Vec<Message>, message: Message) {
    if let Some(existing) = messages.iter_mut().find(|stored| stored.id == message.id) {
        *existing = message;
    } else {
        messages.push(message);
    }
}

fn append_assistant_delta(messages: &mut Vec<Message>, chat_id: Uuid, id: Uuid, delta: String) {
    if let Some(message) = messages.iter_mut().find(|message| message.id == id) {
        message.body.push_str(&delta);
    } else {
        messages.push(Message {
            id,
            chat_id,
            role: MessageRole::Assistant,
            body: delta,
            created_at: chrono::Utc::now(),
        });
    }
}

async fn create_and_publish(
    service: Arc<ChatService>,
    shared_state: SharedChatState,
) -> Result<Chat, CreateChatError> {
    let _write_guard = shared_state.write_gate.lock().await;
    let result = service.create_empty_chat().await;
    if let Ok(chat) = &result {
        shared_state.publish_created_chat(chat.clone());
    }
    result
}

async fn delete_and_publish(
    service: Arc<ChatService>,
    shared_state: SharedChatState,
    id: Uuid,
) -> Result<(), DeleteChatError> {
    let _write_guard = shared_state.write_gate.lock().await;
    let result = service.delete_chat(id).await;
    if matches!(&result, Ok(()) | Err(DeleteChatError::NotFound)) {
        shared_state.publish_deleted_chat(id);
    }
    result
}

fn open_chat_window(
    cx: &mut App,
    service: Arc<ChatService>,
    conversation_service: Arc<ConversationService>,
    tokio: tokio::runtime::Handle,
    shared_state: SharedChatState,
) {
    let state_updates = shared_state.snapshot.subscribe();
    let snapshot = state_updates.borrow().clone();
    let bounds = Bounds::centered(None, size(px(1000.0), px(680.0)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        },
        move |_, cx| {
            cx.new(|view_cx| {
                let composer = view_cx.new(composer::Composer::new);
                view_cx
                    .subscribe(
                        &composer,
                        |view: &mut ChatsView, _, _: &composer::Submitted, cx| {
                            view.send_prompt(view.composer.read(cx).content(), cx);
                        },
                    )
                    .detach();
                ChatsView {
                    service,
                    conversation_service,
                    tokio,
                    chats: snapshot.chats,
                    selected_chat: snapshot.selected_chat,
                    messages: snapshot.messages,
                    shared_state,
                    state_updates,
                    state_listener_started: false,
                    working: false,
                    error: None,
                    composer,
                    transcript_scroll: ScrollHandle::new(),
                    initial_scroll_done: false,
                }
            })
        },
    )
    .expect("could not open Smaller Talk window");
    cx.activate(true);
}

struct ChatsView {
    service: Arc<ChatService>,
    conversation_service: Arc<ConversationService>,
    tokio: tokio::runtime::Handle,
    chats: Vec<Chat>,
    selected_chat: Option<Uuid>,
    messages: Vec<Message>,
    shared_state: SharedChatState,
    state_updates: watch::Receiver<ChatSnapshot>,
    state_listener_started: bool,
    working: bool,
    error: Option<String>,
    composer: Entity<composer::Composer>,
    transcript_scroll: ScrollHandle,
    initial_scroll_done: bool,
}

fn chat_element_id(kind: &'static str, id: Uuid) -> gpui::ElementId {
    let (high, low) = id.as_u64_pair();
    (gpui::ElementId::from((kind, high)), low.to_string()).into()
}

fn render_markdown(body: &str, conversation_width: gpui::Pixels) -> impl IntoElement {
    let mut content = div().flex().flex_col().gap_3().w_full().min_w(px(0.0));
    for block in markdown::parse_markdown(body) {
        match block.kind {
            markdown::MarkdownBlockKind::Rule => {
                content = content.child(div().h(px(1.0)).w_full().bg(rgb(0x3b4049)));
            }
            markdown::MarkdownBlockKind::Code(language) => {
                let mut code = div()
                    .w(conversation_width)
                    .max_w(conversation_width)
                    .min_w(px(0.0))
                    .p_3()
                    .rounded_md()
                    .bg(rgb(0x191c22));
                if !language.is_empty() {
                    code = code.child(
                        div()
                            .mb_2()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(language),
                    );
                }
                let mut code_lines = div().flex().flex_col().items_start();
                for line in block.text.trim_end_matches('\n').split('\n') {
                    let line = line.replace('\t', "    ").replace(' ', " ");
                    code_lines = code_lines.child(
                        div()
                            .font_family("Menlo")
                            .text_sm()
                            .text_color(rgb(0xd5d9e0))
                            .child(if line.is_empty() {
                                " ".to_owned()
                            } else {
                                line
                            }),
                    );
                }
                let code = code.child(code_lines);
                content = content.child(code);
            }
            kind => {
                let text = StyledText::new(block.text).with_highlights(
                    block
                        .highlights
                        .into_iter()
                        .map(|(range, style)| (range, gpui::HighlightStyle::from(style))),
                );
                let element = match kind {
                    markdown::MarkdownBlockKind::Heading(1) => div()
                        .text_xl()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(text),
                    markdown::MarkdownBlockKind::Heading(2) => div()
                        .text_lg()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(text),
                    markdown::MarkdownBlockKind::Heading(_) => div()
                        .text_base()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(text),
                    markdown::MarkdownBlockKind::Quote => div()
                        .pl_3()
                        .border_l_2()
                        .border_color(rgb(ACCENT))
                        .child(text),
                    _ => div().text_base().child(text),
                }
                .text_color(rgb(TEXT));
                content = content.child(element);
            }
        }
    }
    content
}

impl ChatsView {
    fn apply_snapshot(&mut self, snapshot: ChatSnapshot) {
        let selection_changed = self.selected_chat != snapshot.selected_chat;
        let transcript_changed = self.messages != snapshot.messages;
        let follow = selection_changed || (transcript_changed && self.at_transcript_bottom());
        self.chats = snapshot.chats;
        self.selected_chat = snapshot.selected_chat;
        self.messages = snapshot.messages;
        if follow {
            self.transcript_scroll.scroll_to_bottom();
        }
    }

    fn at_transcript_bottom(&self) -> bool {
        // GPUI offsets are negative when scrolled down. Inspect the old layout,
        // before applying the new stream text, so growing content can follow it.
        follows_transcript(
            self.transcript_scroll.offset().y,
            self.transcript_scroll.max_offset().height,
        )
    }

    fn create_chat(&mut self, cx: &mut Context<Self>) {
        if self.working {
            return;
        }
        self.working = true;
        self.error = None;
        let service = self.service.clone();
        let tokio = self.tokio.clone();
        let shared_state = self.shared_state.clone();
        let (sender, receiver) = oneshot::channel();
        tokio.spawn(async move {
            let _ = sender.send(create_and_publish(service, shared_state).await);
        });

        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            let _ = this.update(cx, |view, cx| {
                view.working = false;
                match result {
                    Ok(Ok(_)) => {}
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
        let shared_state = self.shared_state.clone();
        let (sender, receiver) = oneshot::channel();
        tokio.spawn(async move {
            let _ = sender.send(delete_and_publish(service, shared_state, id).await);
        });

        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            let _ = this.update(cx, |view, cx| {
                view.working = false;
                match result {
                    Ok(Ok(())) | Ok(Err(DeleteChatError::NotFound)) => {
                        let selected_chat = view.shared_state.snapshot.borrow().selected_chat;
                        let snapshot = view.shared_state.snapshot.borrow().clone();
                        view.apply_snapshot(snapshot);
                        if let Some(selected_chat) = selected_chat {
                            view.reload_messages(selected_chat, cx);
                        }
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
        let snapshot = self.shared_state.select_chat(id);
        self.apply_snapshot(snapshot);
        self.error = None;
        self.reload_messages(id, cx);
        cx.notify();
    }

    fn reload_messages(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let service = self.conversation_service.clone();
        let tokio = self.tokio.clone();
        let (sender, receiver) = oneshot::channel();
        tokio.spawn(async move {
            let _ = sender.send(service.list_messages(id).await);
        });
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            let _ = this.update(cx, |view, cx| {
                if view.selected_chat != Some(id) {
                    return;
                }
                match result {
                    Ok(Ok(messages)) => {
                        view.shared_state.publish_messages(id, messages.clone());
                        let snapshot = view.shared_state.snapshot.borrow().clone();
                        view.apply_snapshot(snapshot);
                    }
                    Ok(Err(error)) => view.error = Some(error.to_string()),
                    Err(_) => {
                        view.error = Some("The transcript task stopped unexpectedly.".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn send_prompt(&mut self, prompt: String, cx: &mut Context<Self>) {
        let Some(chat_id) = self.selected_chat else {
            return;
        };
        let prompt = prompt.trim().to_owned();
        if prompt.is_empty() || self.working {
            return;
        }

        self.composer.update(cx, |composer, cx| composer.reset(cx));
        self.transcript_scroll.scroll_to_bottom();
        self.working = true;
        self.error = None;
        let service = self.conversation_service.clone();
        let tokio = self.tokio.clone();
        let (event_sender, mut event_receiver) = tokio::sync::mpsc::unbounded_channel();
        let (result_sender, result_receiver) = oneshot::channel();
        tokio.spawn(async move {
            let result = service
                .send_message(SendMessageInput { chat_id, prompt }, event_sender)
                .await
                .map_err(|error| error.to_string());
            let _ = result_sender.send(result);
        });

        cx.spawn(async move |this, cx| {
            while let Some(event) = event_receiver.recv().await {
                if this
                    .update(cx, |view, cx| {
                        view.shared_state.publish_conversation_event(event);
                        let snapshot = view.shared_state.snapshot.borrow().clone();
                        view.apply_snapshot(snapshot);
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            let result = result_receiver.await;
            let _ = this.update(cx, |view, cx| {
                view.working = false;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        view.error = Some(error);
                        view.reload_messages(chat_id, cx);
                    }
                    Err(_) => {
                        view.error = Some("The agent task stopped unexpectedly.".into());
                        view.reload_messages(chat_id, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn start_state_listener(&mut self, cx: &mut Context<Self>) {
        if self.state_listener_started {
            return;
        }
        self.state_listener_started = true;
        let mut state_updates = self.state_updates.clone();
        cx.spawn(async move |this, cx| {
            while state_updates.changed().await.is_ok() {
                let snapshot = state_updates.borrow_and_update().clone();
                if this
                    .update(cx, |view, cx| {
                        view.apply_snapshot(snapshot);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
}

fn follows_transcript(offset: gpui::Pixels, maximum: gpui::Pixels) -> bool {
    maximum + offset <= px(2.0)
}

impl Render for ChatsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.start_state_listener(cx);
        if !self.initial_scroll_done {
            self.transcript_scroll.scroll_to_bottom();
            self.initial_scroll_done = true;
        }
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
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .p_8()
            .gap_4()
            .bg(rgb(BACKGROUND));

        if let Some(chat) = selected {
            let mut transcript = div()
                .id("transcript")
                .flex_1()
                .w_full()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_4()
                .py_2()
                .track_scroll(&self.transcript_scroll);
            if self.messages.is_empty() {
                transcript = transcript.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(MUTED))
                        .child("Ask Codex something to start the conversation."),
                );
            } else {
                for message in &self.messages {
                    match message.role {
                        MessageRole::User => {
                            transcript = transcript.child(
                                div().w_full().min_w(px(0.0)).flex().justify_end().child(
                                    div()
                                        .w_full()
                                        .max_w(px(650.0))
                                        .min_w(px(0.0))
                                        .px_4()
                                        .py_3()
                                        .rounded_lg()
                                        .bg(rgb(PANEL_HOVER))
                                        .text_color(rgb(TEXT))
                                        .child(message.body.clone()),
                                ),
                            );
                        }
                        MessageRole::Assistant => {
                            transcript = transcript.child(
                                div()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(ACCENT))
                                            .child("CODEX"),
                                    )
                                    .child(render_markdown(
                                        &message.body,
                                        (window.bounds().size.width - px(344.0)).max(px(120.0)),
                                    )),
                            );
                        }
                    }
                }
            }

            let send_button = div()
                .id("send-message")
                .px_4()
                .py_2()
                .rounded_md()
                .bg(rgb(if self.working { PANEL_HOVER } else { ACCENT }))
                .text_color(rgb(0x101715))
                .cursor_pointer()
                .child(if self.working { "Sending…" } else { "Send" })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.send_prompt(this.composer.read(cx).content(), cx)
                }));
            let action_row = div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .child(div().flex_1().min_w(px(0.0)))
                .child(send_button);
            let composer = div()
                .id("composer")
                .min_w(px(0.0))
                .w_full()
                .flex()
                .flex_col()
                .gap_2()
                .px_3()
                .py_3()
                .rounded_lg()
                .border_1()
                .border_color(rgb(0x3b4049))
                .bg(rgb(PANEL))
                .child(self.composer.clone())
                .child(action_row);

            detail = detail
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(TEXT))
                                .child(chat.title.clone()),
                        )
                        .child(div().text_xs().text_color(rgb(MUTED)).child("Codex")),
                )
                .child(transcript)
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .flex()
                        .items_end()
                        .child(composer),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("Enter to send · Shift+Enter for a new line"),
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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

    use async_trait::async_trait;
    use chrono::{Duration as ChronoDuration, Utc};
    use gpui::px;
    use smaller_talk_application::{
        ChatQueries, ChatService, ChatWrites, DeleteChatOutcome, NewChat, PersistenceError,
    };
    use smaller_talk_database::{
        SqliteChatStore, create_empty_database, migrate_existing, open_existing,
    };
    use tokio::{sync::Notify, time::timeout};

    use super::{
        ChatSnapshot, SharedChatState, create_and_publish, delete_and_publish, follows_transcript,
        resolve_codex_executable_from,
    };
    use smaller_talk_domain::{Chat, Message, MessageRole};

    struct DelayedChatWrites {
        store: Arc<SqliteChatStore>,
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[test]
    fn entry_resolves_an_explicit_codex_path_before_searching_path() {
        let resolved = resolve_codex_executable_from(
            Some("/tmp/codex-fixture".into()),
            Some("/usr/bin".into()),
            None,
        );
        assert_eq!(resolved, PathBuf::from("/tmp/codex-fixture"));
    }

    impl DelayedChatWrites {
        async fn pause_before_write(&self) {
            self.started.notify_one();
            self.release.notified().await;
        }
    }

    #[async_trait]
    impl ChatWrites for DelayedChatWrites {
        async fn create_chat(
            &self,
            chat: NewChat,
        ) -> Result<smaller_talk_domain::Chat, PersistenceError> {
            self.pause_before_write().await;
            self.store.create_chat(chat).await
        }

        async fn delete_chat(&self, id: uuid::Uuid) -> Result<DeleteChatOutcome, PersistenceError> {
            self.pause_before_write().await;
            self.store.delete_chat(id).await
        }

        async fn save_execution_binding(
            &self,
            id: uuid::Uuid,
            binding: smaller_talk_domain::ExecutionBinding,
        ) -> Result<bool, PersistenceError> {
            self.store.save_execution_binding(id, binding).await
        }
    }

    #[tokio::test]
    async fn writes_committed_after_window_replacement_reach_the_new_view() {
        let directory = tempfile::tempdir().expect("temporary database directory");
        let path = directory.path().join("chats.sqlite3");
        create_empty_database(&path).await.expect("create database");
        migrate_existing(&path).await.expect("migrate database");
        let store = Arc::new(SqliteChatStore::new(
            open_existing(&path).await.expect("open migrated database"),
        ));

        let queries: Arc<dyn ChatQueries> = store.clone();
        let writes: Arc<dyn ChatWrites> = store.clone();
        let initial_service = ChatService::new(queries, writes);
        let existing = initial_service
            .create_empty_chat()
            .await
            .expect("create existing chat");

        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let delayed_writes: Arc<dyn ChatWrites> = Arc::new(DelayedChatWrites {
            store: store.clone(),
            started: started.clone(),
            release: release.clone(),
        });
        let queries: Arc<dyn ChatQueries> = store.clone();
        let service = Arc::new(ChatService::new(queries, delayed_writes));
        let (snapshot, _) = tokio::sync::watch::channel(ChatSnapshot {
            chats: vec![existing.clone()],
            selected_chat: Some(existing.id),
            messages: Vec::new(),
            active_transcripts: HashMap::new(),
        });
        let shared_state = SharedChatState {
            snapshot,
            write_gate: Arc::new(tokio::sync::Mutex::new(())),
        };

        let old_view = shared_state.snapshot.subscribe();
        let create_task = tokio::spawn(create_and_publish(service.clone(), shared_state.clone()));
        timeout(Duration::from_secs(2), started.notified())
            .await
            .expect("create reaches delayed storage");
        drop(old_view);

        let mut replacement_view = shared_state.snapshot.subscribe();
        release.notify_one();
        let created = timeout(Duration::from_secs(2), create_task)
            .await
            .expect("create task completes")
            .expect("create task joins")
            .expect("create succeeds");
        timeout(Duration::from_secs(2), replacement_view.changed())
            .await
            .expect("replacement view receives the create")
            .expect("shared state remains open");
        let after_create = replacement_view.borrow_and_update().clone();
        assert!(after_create.chats.iter().any(|chat| chat.id == created.id));
        assert_eq!(after_create.selected_chat, Some(created.id));

        let delete_task = tokio::spawn(delete_and_publish(
            service,
            shared_state.clone(),
            created.id,
        ));
        timeout(Duration::from_secs(2), started.notified())
            .await
            .expect("delete reaches delayed storage");
        drop(replacement_view);

        let mut reopened_view = shared_state.snapshot.subscribe();
        release.notify_one();
        timeout(Duration::from_secs(2), delete_task)
            .await
            .expect("delete task completes")
            .expect("delete task joins")
            .expect("delete succeeds");
        timeout(Duration::from_secs(2), reopened_view.changed())
            .await
            .expect("reopened view receives the delete")
            .expect("shared state remains open");
        let after_delete = reopened_view.borrow_and_update().clone();
        assert!(!after_delete.chats.iter().any(|chat| chat.id == created.id));
        assert_eq!(after_delete.selected_chat, Some(existing.id));
        assert!(
            after_delete
                .chats
                .iter()
                .any(|chat| Some(chat.id) == after_delete.selected_chat)
        );

        let persisted_chats = initial_service
            .list_chats()
            .await
            .expect("list persisted chats");
        assert!(persisted_chats.iter().any(|chat| chat.id == existing.id));
        assert!(!persisted_chats.iter().any(|chat| chat.id == created.id));
        store.pool().close().await;
    }

    #[test]
    fn streaming_follows_only_the_bottom_of_the_transcript() {
        assert!(follows_transcript(px(0.), px(0.)));
        assert!(follows_transcript(px(-100.), px(100.)));
        assert!(!follows_transcript(px(-50.), px(100.)));
    }

    #[test]
    fn active_stream_survives_chat_switch_and_transcript_reload() {
        let first = Chat {
            id: uuid::Uuid::new_v4(),
            title: "First".into(),
            created_at: Utc::now(),
            binding: None,
        };
        let second = Chat {
            id: uuid::Uuid::new_v4(),
            title: "Second".into(),
            created_at: Utc::now() + ChronoDuration::milliseconds(1),
            binding: None,
        };
        let (snapshot, _) = tokio::sync::watch::channel(ChatSnapshot {
            chats: vec![second.clone(), first.clone()],
            selected_chat: Some(first.id),
            messages: Vec::new(),
            active_transcripts: HashMap::new(),
        });
        let state = SharedChatState {
            snapshot,
            write_gate: Arc::new(tokio::sync::Mutex::new(())),
        };
        let user = Message {
            id: uuid::Uuid::new_v4(),
            chat_id: first.id,
            role: MessageRole::User,
            body: "uppercase TEST prompt".into(),
            created_at: Utc::now(),
        };
        let assistant_id = uuid::Uuid::new_v4();

        state.publish_conversation_event(super::ConversationEvent::UserMessage(user.clone()));
        state.publish_conversation_event(super::ConversationEvent::AssistantDelta {
            chat_id: first.id,
            message_id: assistant_id,
            delta: "PREFIX".into(),
        });
        assert_eq!(state.snapshot.borrow().messages[1].body, "PREFIX");

        assert!(state.select_chat(second.id).messages.is_empty());
        state.publish_conversation_event(super::ConversationEvent::AssistantDelta {
            chat_id: first.id,
            message_id: assistant_id,
            delta: "MIDDLE".into(),
        });
        assert!(state.snapshot.borrow().messages.is_empty());

        let returned = state.select_chat(first.id);
        assert_eq!(returned.messages[0].body, "uppercase TEST prompt");
        assert_eq!(returned.messages[1].body, "PREFIXMIDDLE");

        state.publish_messages(first.id, vec![user.clone()]);
        assert_eq!(state.snapshot.borrow().messages[1].body, "PREFIXMIDDLE");
        state.publish_conversation_event(super::ConversationEvent::AssistantDelta {
            chat_id: first.id,
            message_id: assistant_id,
            delta: "TAIL".into(),
        });
        assert_eq!(state.snapshot.borrow().messages[1].body, "PREFIXMIDDLETAIL");

        let completed = Message {
            id: assistant_id,
            chat_id: first.id,
            role: MessageRole::Assistant,
            body: "PREFIXMIDDLETAIL".into(),
            created_at: user.created_at + ChronoDuration::milliseconds(1),
        };
        state.publish_conversation_event(super::ConversationEvent::AssistantMessage(completed));
        assert_eq!(state.snapshot.borrow().messages[1].body, "PREFIXMIDDLETAIL");
        assert_eq!(state.snapshot.borrow().messages[0].id, user.id);
    }
}
