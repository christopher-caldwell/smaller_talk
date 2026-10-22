mod benchmark;
mod composer_panel;
mod drafts;
mod instructions;
mod menus;
mod navigation;
mod polish;
mod repository;
mod rewriting;
mod settings;
mod skills;
use polish::*;

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use gpui::{
    Context, Entity, ExternalPaths, FocusHandle, Focusable, IntoElement, ListAlignment, ListOffset,
    ListState, Render, ScrollHandle, Window, div, prelude::*, px, rgb,
};
use smaller_talk_application::*;
use smaller_talk_domain::*;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::{
    composer::{self, Composer},
    markdown::MarkdownCache,
    rich_text::RichText,
    theme::*,
};

#[derive(Clone)]
pub struct Services {
    pub skills: Arc<SkillService>,
    pub skill_queries: Arc<dyn SkillQueries>,
    pub instructions: Arc<InstructionService>,
    pub display: Arc<dyn DisplayQueries>,
    pub rewriter: Arc<RewriteService>,
    pub settings: Arc<SettingsService>,
    pub providers: Arc<dyn ProviderControl>,
    pub repositories: Arc<dyn RepositoryReader>,
    pub launch_folder: PathBuf,
    pub chats: Arc<ChatService>,
    pub conversation: Arc<ConversationService>,
    pub queue: QueueCoordinator,
    pub workspace: Arc<WorkspaceService>,
    pub attachments: Arc<dyn AttachmentStorage>,
    pub runtime: tokio::runtime::Handle,
}

pub(crate) struct ChatViewState {
    pub queue_edit_pending: bool,
    pub composer: Entity<Composer>,
    pub attachments: Vec<Attachment>,
    pub files: HashMap<Uuid, Vec<Attachment>>,
    pub messages: Vec<Message>,
    pub preferences: ChatPreferences,
    pub list: ListState,
    pub follow: bool,
    pub loaded: bool,
    pub activity: Option<Activity>,
    pub error: Option<String>,
    pub error_details: bool,
    pub copied: Option<Uuid>,
    pub copied_code: Option<(Uuid, usize)>,
    pub pending: Option<PendingInteraction>,
    pub submitted: Option<Draft>,
    pub retry_of: Option<Uuid>,
    pub submitted_user: Option<Uuid>,
    pub settings_pending: bool,
    pub settings_error: bool,
    pub markdown: MarkdownCache,
    pub rich: HashMap<(Uuid, String), Entity<RichText>>,
    pub horizontal: HashMap<(Uuid, usize), ScrollHandle>,
    pub interaction_scroll: ScrollHandle,
    pub importing: bool,
    pub enqueue_pending: bool,
}

pub(crate) struct PendingInteraction {
    pub request: Interaction,
    pub editors: Vec<Entity<Composer>>,
    pub answers: BTreeMap<String, Vec<String>>,
    pub details: bool,
}

fn clear_approval_text(state: &mut ChatViewState, request_id: &str) {
    let command_key = format!("approval-command:{request_id}");
    let arguments_key = format!("approval-arguments:{request_id}");
    state
        .rich
        .retain(|(_, key), _| key != &command_key && key != &arguments_key);
}

struct LoadedChat {
    skills: HashMap<Uuid, Vec<SkillMention>>,
    queued_ids: Vec<Uuid>,
    messages: Vec<Message>,
    preferences: ChatPreferences,
    draft: Draft,
    files: Vec<(Uuid, Attachment)>,
    last_turn: Option<TurnSummary>,
}

#[derive(Clone, Copy)]
pub(crate) enum Popup {
    Attachments,
    Skills,
    SkillDetails,
    Models,
    Effort,
    Settings(settings::SettingsPicker),
    Chat(Uuid),
    Assign(Uuid),
    Rename(Uuid),
    Delete(Uuid),
    Project(Option<Uuid>),
    ProjectMenu(Uuid),
}

enum Persist {
    EditQueue(Uuid, Uuid),
    ProjectInstructions(Uuid, CustomInstructions),
    Enqueue(Uuid, Draft),
    Respond(Uuid, InteractionResponse),
    Draft(Uuid, Draft),
    Session(SessionState),
    Preferences(Uuid, ChatPreferences),
    StartChat(StartChatInput),
    GlobalSettings(AppSettings),
    DeleteChat(Uuid),
    Rename(Uuid, String),
    Project(Project),
    RemoveProject(Uuid),
    Barrier(oneshot::Sender<()>),
}

enum UiEvent {
    QueueEdited(Uuid, Result<Draft, String>),
    Skills(
        u64,
        (Uuid, Provider, PathBuf, u64),
        Result<Vec<SkillDescriptor>, String>,
    ),
    SkillPreview(u64, String),
    InstructionsLoaded(u64, Result<instructions::LoadedInstructions, String>),
    ProjectInstructionsSaved(Uuid, CustomInstructions, Result<(), String>),
    Queue(QueueEvent),
    Enqueued(Uuid, Draft, Result<(), String>),
    Loaded(Uuid, Result<LoadedChat, String>),
    Metadata(Vec<(Uuid, ChatPreferences)>),
    Previews(Vec<(Uuid, String)>),
    Catalog(
        Provider,
        u64,
        ProviderReadiness,
        Option<Result<ProviderCapabilities, String>>,
    ),
    GlobalSaved(AppSettings, Result<(), String>),
    Repository(u64, Result<Option<RepositoryStatus>, String>),
    RepositoryDiff(u64, PathBuf, Result<FileDiff, String>),
    PullRequest(u64, Option<String>),
    RewriteSupport(Provider, u64, Result<(), String>),
    DisplayLoaded(
        Uuid,
        Result<(Vec<FinalAnswer>, Vec<ResponseVariant>), String>,
    ),
    Rewritten(Uuid, Uuid, Result<ResponseVariant, String>),
    Search(u64, Result<Vec<SearchHit>, String>),
    Started(Uuid, Draft, Result<Chat, String>),
    Deleted(Uuid),
    Renamed(Chat),
    Projects(Vec<Project>),
    Imported(Uuid, Result<Vec<Attachment>, String>),
    Finished(Uuid, Result<(), String>),
    StorageError(String),
    SettingsResult(Uuid, ChatPreferences, Result<(), String>),
    ExecutionPreferences(Uuid, ChatPreferences),
}

pub struct ChatsView {
    benchmark: Option<benchmark::State>,
    skill_ui: skills::SkillUi,
    instructions: instructions::InstructionUi,
    pub services: Services,
    pub(crate) chats: Vec<Chat>,
    pub(crate) projects: Vec<Project>,
    pub(crate) states: HashMap<Uuid, ChatViewState>,
    pub(crate) queues: HashMap<Uuid, MessageQueue>,
    pub(crate) metadata: HashMap<Uuid, ChatPreferences>,
    pub(crate) session: SessionState,
    pub(crate) active: Option<Uuid>,
    pub(crate) popup: Option<Popup>,
    pub(crate) catalogs: HashMap<Provider, Result<ProviderCapabilities, String>>,
    rewriting: rewriting::RewriteUi,
    repository: repository::RepositoryUi,
    app_settings: AppSettings,
    settings_open: bool,
    settings_section: settings::SettingsSection,
    global_settings_save: settings::GlobalSettingsSave,
    settings_focus: Option<FocusHandle>,
    model_search: Entity<Composer>,
    model_query: String,
    model_cancelled: bool,
    picker_requested: Option<PickerAction>,
    provider_status: HashMap<Provider, ProviderReadiness>,
    catalog_generations: HashMap<Provider, u64>,
    path_editors: HashMap<Provider, Entity<Composer>>,
    expanded_providers: std::collections::HashSet<Provider>,
    search: Entity<Composer>,
    search_generation: u64,
    search_query: String,
    search_results: Vec<SearchHit>,
    search_loading: bool,
    pending_reveal: HashMap<Uuid, Uuid>,
    name_editor: Entity<Composer>,
    folder_editor: Entity<Composer>,
    pub(crate) banner: Option<String>,
    banner_details: bool,
    focus: FocusHandle,
    pub(crate) focus_composer: bool,
    tabs_scroll: ScrollHandle,
    previews: HashMap<Uuid, String>,
    picker_bounds: HashMap<&'static str, gpui::Bounds<gpui::Pixels>>,
    popup_index: usize,
    context_anchor: gpui::Point<gpui::Pixels>,
    popup_scroll: ScrollHandle,
    popup_focus: FocusHandle,
    popup_return_focus: Option<FocusHandle>,
    sender: mpsc::UnboundedSender<UiEvent>,
    persist: mpsc::UnboundedSender<Persist>,
    pub(crate) last_width: gpui::Pixels,
    detail_width: gpui::Pixels,
    pub(crate) content_inset: gpui::Pixels,
    last_window_width: gpui::Pixels,
}

impl ChatsView {
    pub fn new(
        services: Services,
        chats: Vec<Chat>,
        projects: Vec<Project>,
        mut session: SessionState,
        app_settings: AppSettings,
        mut queue_events: mpsc::UnboundedReceiver<QueueEvent>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let (persist, mut writes) = mpsc::unbounded_channel();
        let storage = services.clone();
        let notices = sender.clone();
        let queue_notices = sender.clone();
        services.runtime.spawn(async move {
            while let Some(event) = queue_events.recv().await {
                if queue_notices.send(UiEvent::Queue(event)).is_err() {
                    break;
                }
            }
        });
        services.runtime.spawn(async move {
            while let Some(command) = writes.recv().await {
                let result: Result<(), String> = async {
                    match command {
                        Persist::EditQueue(chat, entry) => {
                            let result = storage
                                .queue
                                .edit(EditQueueInput {
                                    chat_id: chat,
                                    entry_id: entry,
                                })
                                .await
                                .map_err(|e| e.to_string());
                            let _ = notices.send(UiEvent::QueueEdited(chat, result));
                        }

                        Persist::ProjectInstructions(id, value) => {
                            let result = storage
                                .instructions
                                .save_project(id, &value)
                                .await
                                .map_err(|e| e.to_string());
                            let _ =
                                notices.send(UiEvent::ProjectInstructionsSaved(id, value, result));
                        }
                        Persist::Respond(id, response) => {
                            storage.queue.respond(RespondQueueInput {
                                chat_id: id,
                                response,
                            })
                        }
                        Persist::Enqueue(id, draft) => {
                            let result = storage
                                .queue
                                .enqueue(EnqueueInput {
                                    chat_id: id,
                                    draft: draft.clone(),
                                })
                                .await
                                .map_err(|error| error.to_string());
                            let _ = notices.send(UiEvent::Enqueued(id, draft, result));
                        }
                        Persist::Draft(id, draft) => storage
                            .workspace
                            .save_draft(id, &draft)
                            .await
                            .map_err(|e| e.to_string())?,
                        Persist::Session(session) => storage
                            .workspace
                            .save_session(&session)
                            .await
                            .map_err(|e| e.to_string())?,
                        Persist::Preferences(id, prefs) => {
                            let result = storage
                                .workspace
                                .save_preferences(id, prefs.clone())
                                .await
                                .map_err(|e| e.to_string());
                            let _ = notices.send(UiEvent::SettingsResult(id, prefs, result));
                        }
                        Persist::GlobalSettings(settings) => {
                            let result = storage
                                .settings
                                .save(&settings)
                                .await
                                .map_err(|e| e.to_string());
                            if result.is_ok() {
                                for provider in PROVIDERS {
                                    let prefs = settings.provider(provider);
                                    storage.providers.configure(
                                        provider,
                                        crate::startup::provider_executable(provider, &prefs),
                                        prefs.enabled,
                                    );
                                }
                            }
                            let _ = notices.send(UiEvent::GlobalSaved(settings, result));
                        }
                        Persist::StartChat(input) => {
                            let id = input.chat_id;
                            let draft = input.draft.clone();
                            let result = storage
                                .chats
                                .start_chat(input)
                                .await
                                .map_err(|e| e.to_string());
                            let saved = result.is_ok();
                            let _ = notices.send(UiEvent::Started(id, draft, result));
                            if saved {
                                storage.queue.resume(id);
                            }
                        }
                        Persist::DeleteChat(id) => {
                            storage
                                .chats
                                .delete_chat(id)
                                .await
                                .map_err(|e| e.to_string())?;
                            let _ = notices.send(UiEvent::Deleted(id));
                        }
                        Persist::Rename(id, title) => {
                            let chat = storage
                                .chats
                                .rename_chat(id, title)
                                .await
                                .map_err(|e| e.to_string())?;
                            let _ = notices.send(UiEvent::Renamed(chat));
                        }
                        Persist::Project(project) => {
                            storage
                                .workspace
                                .save_project(project)
                                .await
                                .map_err(|e| e.to_string())?;
                            let _ = notices.send(UiEvent::Projects(
                                storage
                                    .workspace
                                    .list_projects()
                                    .await
                                    .map_err(|e| e.to_string())?,
                            ));
                        }
                        Persist::RemoveProject(id) => {
                            storage
                                .workspace
                                .remove_project(id)
                                .await
                                .map_err(|e| e.to_string())?;
                            let _ = notices.send(UiEvent::Projects(
                                storage
                                    .workspace
                                    .list_projects()
                                    .await
                                    .map_err(|e| e.to_string())?,
                            ));
                        }
                        Persist::Barrier(done) => {
                            let _ = done.send(());
                        }
                    }
                    Ok(())
                }
                .await;
                if let Err(error) = result {
                    let _ = notices.send(UiEvent::StorageError(error));
                }
            }
        });
        cx.spawn(async move |this, cx| {
            while let Some(event) = receiver.recv().await {
                if this
                    .update(cx, |this, cx| {
                        this.apply_event(event, cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        if let Some((id, draft)) = drafts::recover_started_chat(&mut session, &chats) {
            let _ = persist.send(Persist::Draft(id, draft));
        }
        session.open_tabs.retain(|id| {
            chats.iter().any(|c| c.id.to_string() == *id)
                || session
                    .new_chat
                    .as_ref()
                    .is_some_and(|draft| draft.id == *id)
        });
        let active = session
            .active_tab
            .as_ref()
            .and_then(|id| Uuid::parse_str(id).ok())
            .filter(|id| session.open_tabs.contains(&id.to_string()))
            .or_else(|| {
                session
                    .open_tabs
                    .last()
                    .and_then(|s| Uuid::parse_str(s).ok())
            });
        let search = cx.new(Composer::new_single_line);
        search.update(cx, |s, _| {
            s.set_placeholder("Search chats…");
            s.set_font_size(13.);
        });
        cx.subscribe(&search, |this, _, _: &composer::Changed, cx| {
            this.search_changed(cx)
        })
        .detach();
        cx.subscribe(&search, |this, _, _: &composer::Canceled, cx| {
            this.clear_search(cx);
            this.focus_composer = true;
        })
        .detach();
        let mut view = Self {
            benchmark: None,
            instructions: instructions::InstructionUi::new(cx),
            services,
            chats,
            projects,
            states: HashMap::new(),
            queues: HashMap::new(),
            metadata: HashMap::new(),
            session,
            active,
            popup: None,
            skill_ui: Default::default(),
            catalogs: HashMap::new(),
            rewriting: Default::default(),
            repository: repository::RepositoryUi::new(cx),
            app_settings,
            settings_open: false,
            settings_section: settings::SettingsSection::default(),
            global_settings_save: settings::GlobalSettingsSave::default(),
            settings_focus: None,
            model_search: cx.new(Composer::new_single_line),
            model_query: String::new(),
            model_cancelled: false,
            picker_requested: None,
            provider_status: HashMap::new(),
            catalog_generations: HashMap::new(),
            path_editors: HashMap::new(),
            expanded_providers: Default::default(),
            search,
            search_generation: 0,
            search_query: String::new(),
            search_results: vec![],
            search_loading: false,
            pending_reveal: HashMap::new(),
            name_editor: cx.new(Composer::new_single_line),
            folder_editor: cx.new(Composer::new_single_line),
            banner: None,
            banner_details: false,
            focus: cx.focus_handle(),
            focus_composer: true,
            tabs_scroll: ScrollHandle::new(),
            previews: HashMap::new(),
            picker_bounds: HashMap::new(),
            popup_index: 0,
            context_anchor: gpui::point(px(0.), px(0.)),
            popup_scroll: ScrollHandle::new(),
            popup_focus: cx.focus_handle(),
            popup_return_focus: None,
            sender,
            persist,
            last_width: px(1.),
            detail_width: px(800.),
            content_inset: px(24.),
            last_window_width: px(0.),
        };
        view.model_search
            .update(cx, |editor, _| editor.set_placeholder("Search models…"));
        cx.subscribe(
            &view.instructions.editor,
            |this, editor, _: &composer::Changed, cx| {
                if !this.instructions.loading && !this.instructions.read_only {
                    this.instructions.draft.text = editor.read(cx).content();
                    this.instructions.remember();
                    this.instructions.notice = None;
                }
                cx.notify();
            },
        )
        .detach();
        cx.subscribe(
            &view.model_search,
            |this, editor, _: &composer::Changed, cx| {
                this.model_query = editor.read(cx).content();
                if matches!(this.popup, Some(Popup::Skills)) {
                    this.skill_ui.query = this.model_query.clone();
                }
                this.popup_index = 0;
                this.popup_scroll.set_offset(gpui::point(px(0.), px(0.)));
                cx.notify();
            },
        )
        .detach();
        cx.subscribe(&view.model_search, |this, _, _: &composer::Canceled, cx| {
            this.model_cancelled = true;
            cx.notify();
        })
        .detach();
        cx.subscribe(
            &view.model_search,
            |this, _, _: &composer::Submitted, cx| {
                if matches!(this.popup, Some(Popup::Models | Popup::Skills)) {
                    this.picker_requested = this
                        .picker_choices()
                        .get(this.popup_index)
                        .map(|c| c.action.clone());
                    cx.notify();
                }
            },
        )
        .detach();
        cx.subscribe(
            &view.model_search,
            |this, _, event: &composer::Navigate, cx| {
                if matches!(this.popup, Some(Popup::Models | Popup::Skills)) {
                    let count = this.picker_choices().len();
                    if count > 0 {
                        this.popup_index = (this.popup_index as isize + event.0 as isize)
                            .rem_euclid(count as isize)
                            as usize;
                        this.popup_scroll.scroll_to_item(this.popup_index);
                    }
                    cx.notify();
                }
            },
        )
        .detach();
        for provider in PROVIDERS {
            let editor = cx.new(Composer::new_single_line);
            editor.update(cx, |e, _| {
                e.set_font_size(13.);
                e.set_placeholder("Automatic executable discovery")
            });
            view.path_editors.insert(provider, editor);
        }
        view.folder_editor
            .update(cx, |s, _| s.set_placeholder("Optional working folder"));
        cx.subscribe(&view.name_editor, |this, _, _: &composer::Submitted, cx| {
            match this.popup {
                Some(Popup::Rename(id)) => {
                    let _ = this
                        .persist
                        .send(Persist::Rename(id, this.name_editor.read(cx).content()));
                    this.popup = None;
                }
                Some(Popup::Project(id)) => this.save_project(id, cx),
                _ => {}
            }
            cx.notify();
        })
        .detach();
        cx.subscribe(
            &view.folder_editor,
            |this, _, _: &composer::Submitted, cx| {
                if let Some(Popup::Project(id)) = this.popup {
                    this.save_project(id, cx);
                }
            },
        )
        .detach();
        for editor in [&view.name_editor, &view.folder_editor] {
            cx.subscribe(editor, |this, _, _: &composer::Canceled, cx| {
                this.popup = None;
                this.focus_composer = true;
                cx.notify();
            })
            .detach();
        }
        let ids: Vec<_> = view
            .session
            .open_tabs
            .iter()
            .filter_map(|s| Uuid::parse_str(s).ok())
            .collect();
        for id in ids {
            view.load_chat(id, cx);
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                if this
                    .update(cx, |this, cx| {
                        if this.repository.open && this.repository.focused && !this.settings_open {
                            this.refresh_repository(false);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        view.discover(Provider::Codex);
        view.discover(Provider::Claude);
        let services = view.services.clone();
        let notices = view.sender.clone();
        let chat_ids: Vec<_> = view.chats.iter().map(|c| c.id).collect();
        view.services.runtime.spawn(async move {
            match services.workspace.chat_previews().await {
                Ok(items) => {
                    let _ = notices.send(UiEvent::Previews(items));
                }
                Err(e) => {
                    let _ = notices.send(UiEvent::StorageError(e.to_string()));
                }
            }
            let mut metadata = Vec::new();
            for id in chat_ids {
                services.queue.refresh(id);
                match services.workspace.preferences(id).await {
                    Ok(p) => metadata.push((id, p)),
                    Err(e) => {
                        let _ = notices.send(UiEvent::StorageError(e.to_string()));
                    }
                }
            }
            let _ = notices.send(UiEvent::Metadata(metadata));
        });
        view
    }

    fn discover(&mut self, provider: Provider) {
        let generation = self.catalog_generations.entry(provider).or_default();
        *generation += 1;
        let generation = *generation;
        let enabled = self.app_settings.provider(provider).enabled;
        if enabled {
            self.catalogs.remove(&provider);
        }
        self.provider_status.remove(&provider);
        let control = self.services.providers.clone();
        let rewriter = self.services.rewriter.clone();
        let sender = self.sender.clone();
        self.services.runtime.spawn(async move {
            let status = control.readiness(provider).await;
            let result = if enabled && status == ProviderReadiness::Ready {
                Some(control.refresh(provider).await.map_err(|e| e.to_string()))
            } else {
                None
            };
            let support = if enabled && status == ProviderReadiness::Ready {
                rewriter.support(provider).await.map_err(|e| e.to_string())
            } else {
                Err("Enable this provider and sign in to check rewrite support.".into())
            };
            let _ = sender.send(UiEvent::RewriteSupport(provider, generation, support));
            let _ = sender.send(UiEvent::Catalog(provider, generation, status, result));
        });
    }

    fn load_chat(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.states.get(&id).is_some_and(|s| s.loaded) {
            return;
        }
        if !self.states.contains_key(&id) {
            let composer = cx.new(Composer::new);
            composer.update(cx, |editor, _| editor.set_font_size(15.));
            cx.subscribe(&composer, move |this, _, _: &composer::Submitted, cx| {
                if matches!(this.popup, Some(Popup::Skills)) {
                    this.picker_requested = this
                        .picker_choices()
                        .get(this.popup_index)
                        .map(|c| c.action.clone());
                    cx.notify();
                    return;
                }
                this.send(id, cx)
            })
            .detach();
            cx.subscribe(&composer, move |this, _, _: &composer::Changed, cx| {
                this.composer_skills_changed(id, cx)
            })
            .detach();
            cx.subscribe(&composer, |this, _, event: &composer::Navigate, cx| {
                if matches!(this.popup, Some(Popup::Skills)) {
                    let count = this.picker_choices().len();
                    if count > 0 {
                        this.popup_index = (this.popup_index as isize + event.0 as isize)
                            .rem_euclid(count as isize)
                            as usize;
                    }
                    cx.notify();
                }
            })
            .detach();
            cx.subscribe(&composer, move |this, _, _: &composer::Canceled, cx| {
                if let Some(state) = this.states.get(&id) {
                    state
                        .composer
                        .update(cx, |e, _| e.set_picker_navigation(false));
                }
                this.popup = None;
                cx.notify();
            })
            .detach();
            let list = ListState::new(0, ListAlignment::Bottom, px(300.)).measure_all();
            let weak = cx.entity().downgrade();
            list.set_scroll_handler(move |event, _, cx| {
                let follow = !event.is_scrolled;
                let weak = weak.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if let Some(state) = this.states.get_mut(&id) {
                            state.follow = follow;
                        }
                        this.save_session();
                        cx.notify();
                    });
                });
            });
            self.states.insert(
                id,
                ChatViewState {
                    queue_edit_pending: false,
                    composer,
                    attachments: vec![],
                    files: HashMap::new(),
                    messages: vec![],
                    preferences: self.metadata.get(&id).cloned().unwrap_or_default(),
                    list,
                    follow: true,
                    loaded: false,
                    activity: None,
                    error: None,
                    error_details: false,
                    copied: None,
                    copied_code: None,
                    pending: None,
                    submitted: None,
                    retry_of: None,
                    submitted_user: None,
                    settings_pending: false,
                    settings_error: false,
                    markdown: MarkdownCache::default(),
                    rich: HashMap::new(),
                    horizontal: HashMap::new(),
                    interaction_scroll: ScrollHandle::new(),
                    importing: false,
                    enqueue_pending: false,
                },
            );
        }
        if let Some(draft) = self
            .session
            .new_chat
            .as_ref()
            .filter(|draft| draft.id == id.to_string())
        {
            let state = self.states.get_mut(&id).expect("draft view exists");
            state.preferences = draft.preferences.clone();
            state.attachments = draft.draft.attachments.clone();
            state
                .composer
                .update(cx, |editor, cx| editor.load_draft(&draft.draft, cx));
            state.loaded = true;
            return;
        }
        let service = self.services.clone();
        let sender = self.sender.clone();
        let (barrier, ready) = oneshot::channel();
        let _ = self.persist.send(Persist::Barrier(barrier));
        self.services.runtime.spawn(async move {
            let _ = ready.await;
            let result = async {
                let messages = service
                    .conversation
                    .list_messages(id)
                    .await
                    .map_err(|e| e.to_string())?;
                let last_turn = service
                    .workspace
                    .last_turn(id)
                    .await
                    .map_err(|e| e.to_string())?;
                let mut skill_messages = std::collections::BTreeSet::new();
                if let Some(turn) = &last_turn {
                    skill_messages.insert(turn.user_message_id);
                }
                if let Some(message) = messages.last().filter(|m| m.role == MessageRole::User) {
                    skill_messages.insert(message.id);
                }
                let mut skills = HashMap::new();
                for message in skill_messages {
                    skills.insert(
                        message,
                        service
                            .skill_queries
                            .message_skills(id, message)
                            .await
                            .map_err(|e| e.to_string())?,
                    );
                }
                Ok::<_, String>(LoadedChat {
                    skills,
                    queued_ids: service
                        .queue
                        .queued_ids(id)
                        .await
                        .map_err(|error| error.to_string())?,
                    messages,
                    preferences: service
                        .workspace
                        .preferences(id)
                        .await
                        .map_err(|e| e.to_string())?,
                    draft: service
                        .workspace
                        .draft(id)
                        .await
                        .map_err(|e| e.to_string())?,
                    files: service
                        .workspace
                        .message_attachments(id)
                        .await
                        .map_err(|e| e.to_string())?,
                    last_turn,
                })
            }
            .await;
            let _ = sender.send(UiEvent::Loaded(id, result));
        });
    }

    fn apply_event(&mut self, event: UiEvent, cx: &mut Context<Self>) {
        self.benchmark_event(&event);
        match event {
            UiEvent::QueueEdited(id, result) => {
                if let Some(state) = self.states.get_mut(&id) {
                    state.queue_edit_pending = false;
                    state.composer.update(cx, |e, cx| e.set_editable(true, cx));
                    match result {
                        Ok(draft) => {
                            state.attachments = draft.attachments.clone();
                            state.composer.update(cx, |e, cx| e.load_draft(&draft, cx));
                            self.focus_composer = true;
                        }
                        Err(error) => state.error = Some(error),
                    }
                }
                self.draft_changed(id, cx);
            }
            UiEvent::Skills(generation, context, result) => {
                self.skills_loaded(generation, context, result, cx)
            }
            UiEvent::SkillPreview(generation, text) => {
                if generation == self.skill_ui.generation {
                    self.skill_ui.preview = Some(text);
                }
            }
            UiEvent::InstructionsLoaded(generation, result) => {
                self.instructions_loaded(generation, result, cx)
            }
            UiEvent::ProjectInstructionsSaved(id, value, result) => self.instruction_save_result(
                instructions::InstructionScope::Project(id),
                value,
                result,
                cx,
            ),
            UiEvent::Enqueued(id, draft, result) => {
                if let Some(state) = self.states.get_mut(&id) {
                    state.enqueue_pending = false;
                    match result {
                        Ok(()) => {
                            state.attachments.retain(|file| {
                                !draft.attachments.iter().any(|sent| sent.id == file.id)
                            });
                            if state.composer.read(cx).content() == draft.text
                                && state.composer.read(cx).skills() == draft.skills
                            {
                                state.composer.update(cx, |s, cx| s.reset(cx));
                            }
                        }
                        Err(error) => state.error = Some(error),
                    }
                }
                self.draft_changed(id, cx);
            }
            UiEvent::Queue(event) => match event {
                QueueEvent::Snapshot(id, queue) => {
                    self.queues.insert(id, queue);
                }
                QueueEvent::Started(id) => {
                    if let Some(state) = self.states.get_mut(&id) {
                        state.activity = Some(Activity::Starting);
                        state.error = None;
                    }
                }
                QueueEvent::Conversation(event) => {
                    if let ConversationEvent::UserMessage(message) = &event
                        && let Some(entry) = self
                            .queues
                            .get(&message.chat_id)
                            .and_then(|q| q.entries.iter().find(|e| e.id == message.id))
                        && let Some(state) = self.states.get_mut(&message.chat_id)
                    {
                        state
                            .files
                            .insert(message.id, entry.draft.attachments.clone());
                    }
                    self.apply_conversation(event, cx);
                }
                QueueEvent::Finished(id, result) => {
                    self.apply_event(
                        UiEvent::Finished(id, result.map_err(|error| error.to_string())),
                        cx,
                    );
                    let services = self.services.clone();
                    let notices = self.sender.clone();
                    self.services.runtime.spawn(async move {
                        if let Ok(prefs) = services.workspace.preferences(id).await {
                            let _ = notices.send(UiEvent::ExecutionPreferences(id, prefs));
                        }
                    });
                }
                QueueEvent::Error(id, error) => {
                    if let Some(state) = self.states.get_mut(&id) {
                        state.error = Some(error.to_string());
                    }
                }
            },
            UiEvent::Previews(items) => self.previews.extend(items),
            UiEvent::Loaded(id, result) => match result {
                Ok(loaded) => {
                    if let Some(message) =
                        loaded.messages.iter().find(|m| m.role == MessageRole::User)
                    {
                        self.previews
                            .insert(id, display_title("Untitled", Some(&message.body)));
                    }
                    self.metadata.insert(id, loaded.preferences.clone());
                    let Some(state) = self.states.get_mut(&id) else {
                        return;
                    };
                    // A second load must not replace a live stream or a newly edited draft.
                    if state.loaded {
                        return;
                    }
                    let live = state.activity.is_some();
                    let mut messages = loaded.messages;
                    for message in std::mem::take(&mut state.messages) {
                        upsert(&mut messages, message);
                    }
                    state.messages = messages;
                    state.preferences = loaded.preferences;
                    if state.composer.read(cx).content().is_empty() && state.attachments.is_empty()
                    {
                        state.attachments = loaded.draft.attachments.clone();
                        state
                            .composer
                            .update(cx, |s, cx| s.load_draft(&loaded.draft, cx));
                    }
                    state.files.clear();
                    for (message, file) in loaded.files {
                        state.files.entry(message).or_default().push(file);
                    }
                    state.list.reset(state.messages.len());
                    if let Some((index, offset)) =
                        self.session.scroll_positions.get(&id.to_string())
                    {
                        state.list.scroll_to(ListOffset {
                            item_ix: *index,
                            offset_in_item: px(*offset),
                        });
                        state.follow = *index >= state.messages.len();
                    }
                    let failed_user = loaded
                        .last_turn
                        .as_ref()
                        .filter(|turn| turn.status != "completed")
                        .and_then(|turn| {
                            state.messages.iter().find(|m| m.id == turn.user_message_id)
                        });
                    if let Some(last) = failed_user
                        .or_else(|| {
                            state
                                .messages
                                .last()
                                .filter(|m| m.role == MessageRole::User)
                        })
                        .filter(|m| !live && !loaded.queued_ids.contains(&m.id))
                    {
                        state.error = Some(loaded.last_turn.as_ref().and_then(|t| t.error.clone()).unwrap_or_else(|| "The previous response did not complete. Your saved message is still here.".into()));
                        if loaded.draft.text.is_empty() && state.attachments.is_empty() {
                            let draft = Draft {
                                skills: loaded.skills.get(&last.id).cloned().unwrap_or_default(),
                                text: last.body.clone(),
                                attachments: state.files.get(&last.id).cloned().unwrap_or_default(),
                                selection_start: last.body.len(),
                                selection_end: last.body.len(),
                            };
                            state.attachments = draft.attachments.clone();
                            state.composer.update(cx, |s, cx| s.load_draft(&draft, cx));
                            state.retry_of = Some(last.id);
                        } else if loaded.draft.text == last.body
                            && loaded.draft.skills
                                == loaded.skills.get(&last.id).cloned().unwrap_or_default()
                        {
                            state.retry_of = Some(last.id);
                        }
                    }
                    if let Some(message) = self.pending_reveal.remove(&id)
                        && let Some(index) = state.messages.iter().position(|m| m.id == message)
                    {
                        state.list.scroll_to(ListOffset {
                            item_ix: index,
                            offset_in_item: px(0.),
                        });
                        state.follow = false;
                    }
                    state.loaded = true;
                    self.load_response_display(id);
                }
                Err(error) => {
                    if let Some(state) = self.states.get_mut(&id) {
                        state.error = Some(error);
                    }
                }
            },
            UiEvent::Metadata(items) => {
                for (id, prefs) in items {
                    self.metadata.entry(id).or_insert(prefs);
                }
            }
            UiEvent::Catalog(provider, generation, status, result) => {
                if self.catalog_generations.get(&provider) == Some(&generation) {
                    self.provider_status.insert(provider, status);
                    if let Some(result) = result {
                        self.catalogs.insert(provider, result);
                    }
                }
            }
            UiEvent::RewriteSupport(provider, generation, result) => {
                if self.catalog_generations.get(&provider) == Some(&generation) {
                    self.rewriting.support.insert(provider, result);
                }
            }
            UiEvent::DisplayLoaded(chat, result) => self.response_display_loaded(chat, result, cx),
            UiEvent::Rewritten(chat, message, result) => {
                self.rewrite_result(chat, message, result, cx)
            }
            UiEvent::Repository(generation, result) => {
                self.repository_result(generation, result, cx)
            }
            UiEvent::RepositoryDiff(generation, path, result) => {
                self.repository_diff_result(generation, path, result, cx)
            }
            UiEvent::PullRequest(generation, url) => {
                if generation == self.repository.context_generation {
                    self.repository.pull_request = url;
                }
            }
            UiEvent::GlobalSaved(settings, result) => {
                self.instruction_save_result(
                    instructions::InstructionScope::Global,
                    settings.instructions.clone(),
                    result.clone(),
                    cx,
                );
                let (_, pending) = self.global_settings_save.acknowledge();
                match result {
                    Ok(()) => {
                        if self.app_settings.rewrite_model != settings.rewrite_model {
                            self.rewriting.displayed.clear();
                            self.rewriting.variants.clear();
                        }
                        let changed: Vec<_> = PROVIDERS
                            .into_iter()
                            .filter(|provider| {
                                let old = self.app_settings.provider(*provider);
                                let new = settings.provider(*provider);
                                old.enabled != new.enabled || old.executable != new.executable
                            })
                            .collect();
                        self.app_settings = settings;
                        for provider in changed {
                            self.discover(provider);
                        }
                    }
                    Err(error) => self.banner = Some(error),
                }
                if let Some(pending) = pending
                    && self.persist.send(Persist::GlobalSettings(pending)).is_err()
                {
                    self.global_settings_save.send_failed();
                    self.banner = Some("Settings could not be saved".into());
                }
            }
            UiEvent::Search(generation, result) if generation == self.search_generation => {
                self.search_loading = false;
                match result {
                    Ok(hits) => self.search_results = hits,
                    Err(e) => self.banner = Some(e),
                }
            }
            UiEvent::Search(_, _) => {}
            UiEvent::Started(id, draft, result) => self.started_chat(id, draft, result, cx),
            UiEvent::Deleted(id) => {
                self.chats.retain(|c| c.id != id);
                // Deletion already removed the persisted draft; do not save it on close.
                self.release_response_display(id);
                self.states.remove(&id);
                self.close_tab(id, cx);
                self.metadata.remove(&id);
                self.queues.remove(&id);
                self.session.scroll_positions.remove(&id.to_string());
                self.save_session();
            }
            UiEvent::Renamed(chat) => {
                if let Some(old) = self.chats.iter_mut().find(|c| c.id == chat.id) {
                    *old = chat;
                }
            }
            UiEvent::Projects(projects) => {
                self.projects = projects;
                for prefs in self.metadata.values_mut() {
                    if prefs
                        .project_id
                        .as_ref()
                        .is_some_and(|id| !self.projects.iter().any(|p| p.id.to_string() == *id))
                    {
                        prefs.project_id = None;
                        prefs.new_session = true;
                    }
                }
                for (id, state) in &mut self.states {
                    if let Some(p) = self.metadata.get(id) {
                        state.preferences.project_id = p.project_id.clone();
                    }
                }
            }
            UiEvent::Imported(id, result) => {
                if let Some(state) = self.states.get_mut(&id) {
                    state.importing = false;
                    match result {
                        Ok(files) => {
                            let count = state.attachments.len() + files.len();
                            let bytes: u64 = state
                                .attachments
                                .iter()
                                .chain(&files)
                                .filter(|a| a.is_image())
                                .map(|a| a.byte_len)
                                .sum();
                            if count > 100 || bytes > 80 * 1024 * 1024 {
                                state.error = Some(
                                    "Attach at most 100 files and 80 MiB of images per message."
                                        .into(),
                                );
                            } else {
                                state.attachments.extend(files);
                            }
                        }
                        Err(error) => state.error = Some(error),
                    }
                    self.draft_changed(id, cx);
                }
            }
            UiEvent::StorageError(error) => {
                self.banner = Some(error);
                self.banner_details = false;
            }
            UiEvent::SettingsResult(id, expected, result) => {
                if let Some(state) = self.states.get_mut(&id) {
                    if state.preferences.execution == expected.execution
                        && state.preferences.project_id == expected.project_id
                    {
                        state.settings_pending = false;
                        state.settings_error = result.is_err();
                        if let Err(error) = result {
                            state.error = Some(format!(
                                "Could not save settings: {error}. Select your settings again before sending."
                            ));
                        }
                    }
                } else if let Err(error) = result {
                    self.banner = Some(error);
                }
            }
            UiEvent::ExecutionPreferences(id, prefs) => {
                if let Some(state) = self.states.get_mut(&id) {
                    state.preferences.binding_workspace = prefs.binding_workspace.clone();
                    state.preferences.new_session = prefs.new_session;
                }
                if let Some(metadata) = self.metadata.get_mut(&id) {
                    metadata.binding_workspace = prefs.binding_workspace;
                    metadata.new_session = prefs.new_session;
                }
            }
            UiEvent::Finished(id, result) => {
                let completed = result.is_ok();
                if self.active == Some(id) {
                    self.refresh_repository(false);
                }
                if let Some(state) = self.states.get_mut(&id) {
                    state.activity = None;
                    if let Some(request_id) = state.pending.as_ref().and_then(|pending| {
                        matches!(&pending.request.kind, InteractionKind::Approval(_))
                            .then(|| pending.request.id.clone())
                    }) {
                        clear_approval_text(state, &request_id);
                    }
                    state.pending = None;
                    if let Err(error) = result {
                        state.error = Some(error);
                        if state.composer.read(cx).content().is_empty()
                            && state.attachments.is_empty()
                            && let Some(draft) = state.submitted.take()
                        {
                            state.attachments = draft.attachments.clone();
                            state.composer.update(cx, |s, cx| s.load_draft(&draft, cx));
                            state.retry_of = state.submitted_user;
                        }
                    } else {
                        state.submitted = None;
                        state.retry_of = None;
                    }
                    self.draft_changed(id, cx);
                    if completed {
                        self.maybe_shorten_latest(id, cx);
                    }
                    if !self.session.open_tabs.contains(&id.to_string()) {
                        self.release_response_display(id);
                    }
                    if !self.session.open_tabs.contains(&id.to_string())
                        && let Some(state) = self.states.get_mut(&id)
                    {
                        state.messages.clear();
                        state.list.reset(0);
                        state.files.clear();
                        state.rich.clear();
                        state.horizontal.clear();
                        state.markdown = MarkdownCache::default();
                        state.loaded = false;
                    }
                }
            }
        }
    }

    fn apply_conversation(&mut self, event: ConversationEvent, cx: &mut Context<Self>) {
        if let ConversationEvent::SkillsChanged { chat_id } = &event {
            self.skill_catalog_changed(*chat_id, cx);
            return;
        }
        if let ConversationEvent::FinalAnswer { answer, .. } = &event {
            self.rewriting
                .sources
                .insert(answer.message_id, answer.text.clone());
            return;
        }
        if matches!(event, ConversationEvent::SteeringAvailable { .. }) {
            return;
        }
        let id = match &event {
            ConversationEvent::UserMessage(m) | ConversationEvent::AssistantMessage(m) => m.chat_id,
            ConversationEvent::AssistantDelta { chat_id, .. }
            | ConversationEvent::AssistantSnapshot { chat_id, .. }
            | ConversationEvent::SkillsChanged { chat_id }
            | ConversationEvent::SteeringAvailable { chat_id, .. }
            | ConversationEvent::Activity { chat_id, .. }
            | ConversationEvent::FinalAnswer { chat_id, .. }
            | ConversationEvent::Interaction { chat_id, .. } => *chat_id,
        };
        let Some(state) = self.states.get_mut(&id) else {
            return;
        };
        let position = state.list.logical_scroll_top();
        let old_len = state.messages.len();
        let draft_changed = matches!(&event, ConversationEvent::UserMessage(_));
        let changed;
        match event {
            ConversationEvent::SkillsChanged { .. }
            | ConversationEvent::SteeringAvailable { .. }
            | ConversationEvent::FinalAnswer { .. } => {
                return;
            }
            ConversationEvent::UserMessage(message) => {
                self.previews
                    .entry(id)
                    .or_insert_with(|| display_title("Untitled", Some(&message.body)));
                state.submitted_user = Some(message.id);
                if let Some(draft) = &state.submitted {
                    state.files.insert(message.id, draft.attachments.clone());
                    state
                        .attachments
                        .retain(|file| !draft.attachments.iter().any(|sent| sent.id == file.id));
                    if state.composer.read(cx).content() == draft.text
                        && state.composer.read(cx).skills() == draft.skills
                    {
                        state.composer.update(cx, |s, cx| s.reset(cx));
                    }
                }
                changed = upsert(&mut state.messages, message);
            }
            ConversationEvent::AssistantMessage(message) => {
                changed = upsert(&mut state.messages, message);
            }
            ConversationEvent::AssistantDelta {
                message_id, delta, ..
            } => {
                changed =
                    if let Some(index) = state.messages.iter().position(|m| m.id == message_id) {
                        state.messages[index].body.push_str(&delta);
                        index
                    } else {
                        state.messages.push(Message {
                            id: message_id,
                            chat_id: id,
                            role: MessageRole::Assistant,
                            body: delta,
                            created_at: chrono::Utc::now(),
                        });
                        state.messages.len() - 1
                    };
            }
            ConversationEvent::AssistantSnapshot {
                message_id, text, ..
            } => {
                changed =
                    if let Some(index) = state.messages.iter().position(|m| m.id == message_id) {
                        state.messages[index].body = text;
                        index
                    } else {
                        state.messages.push(Message {
                            id: message_id,
                            chat_id: id,
                            role: MessageRole::Assistant,
                            body: text,
                            created_at: chrono::Utc::now(),
                        });
                        state.messages.len() - 1
                    };
            }
            ConversationEvent::Activity { activity, .. } => {
                state.activity = Some(activity);
                return;
            }
            ConversationEvent::Interaction { interaction, .. } => {
                if let Some(request_id) = state.pending.as_ref().and_then(|pending| {
                    matches!(&pending.request.kind, InteractionKind::Approval(_))
                        .then(|| pending.request.id.clone())
                }) {
                    clear_approval_text(state, &request_id);
                }
                state.interaction_scroll = ScrollHandle::new();
                let count = match &interaction.kind {
                    InteractionKind::Questions(q) => q.len(),
                    _ => 0,
                };
                let editors = (0..count)
                    .map(|_| {
                        let editor = cx.new(Composer::new_single_line);
                        editor.update(cx, |s, _| {
                            s.set_placeholder("Your answer…");
                            s.set_font_size(13.);
                        });
                        cx.subscribe(&editor, move |this, _, _: &composer::Submitted, cx| {
                            this.respond(id, true, cx)
                        })
                        .detach();
                        editor
                    })
                    .collect();
                state.pending = Some(PendingInteraction {
                    details: false,
                    request: interaction,
                    editors,
                    answers: BTreeMap::new(),
                });
                state.activity = Some(Activity::Waiting);
                self.focus_composer = true;
                return;
            }
        }
        let new_len = state.messages.len();
        state
            .markdown
            .retain_messages(state.messages.iter().map(|m| m.id));
        if changed < old_len {
            state.list.splice(changed..changed + 1, 1);
        }
        if new_len > old_len {
            state.list.splice(old_len..old_len, new_len - old_len);
        }
        if state.follow {
            state.list.scroll_to(ListOffset {
                item_ix: new_len,
                offset_in_item: px(0.),
            });
        } else {
            state.list.scroll_to(position);
        }
        if draft_changed {
            self.draft_changed(id, cx);
        }
    }

    pub(crate) fn open_chat(&mut self, id: Uuid, message: Option<Uuid>, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.settings_focus = None;
        if self.last_window_width < px(720.) {
            self.session.sidebar_collapsed = true;
        }

        if !self.session.open_tabs.contains(&id.to_string()) {
            self.session.open_tabs.push(id.to_string());
        }
        self.active = Some(id);
        self.session.active_tab = Some(id.to_string());
        self.popup = None;
        self.focus_composer = true;
        if let Some(index) = self
            .session
            .open_tabs
            .iter()
            .position(|tab| *tab == id.to_string())
        {
            self.tabs_scroll.scroll_to_item(index);
        }
        self.load_chat(id, cx);
        if let Some(message) = message
            && let Some(state) = self.states.get_mut(&id)
        {
            if let Some(index) = state.messages.iter().position(|m| m.id == message) {
                state.list.scroll_to(ListOffset {
                    item_ix: index,
                    offset_in_item: px(0.),
                });
                state.follow = false;
            } else {
                self.pending_reveal.insert(id, message);
            }
        }
        self.save_session();
        cx.notify();
    }

    fn close_tab(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.draft_changed(id, cx);
        let Some(index) = self
            .session
            .open_tabs
            .iter()
            .position(|s| *s == id.to_string())
        else {
            return;
        };
        self.capture_scroll();
        self.session.open_tabs.remove(index);
        self.focus_composer = true;
        if self.active == Some(id) {
            self.active = self
                .session
                .open_tabs
                .get(index.min(self.session.open_tabs.len().saturating_sub(1)))
                .and_then(|s| Uuid::parse_str(s).ok());
        }
        self.session.active_tab = self.active.map(|id| id.to_string());
        if self.states.get(&id).is_some_and(|s| s.activity.is_none()) {
            self.release_response_display(id);
        }
        if let Some(state) = self.states.get_mut(&id)
            && state.activity.is_none()
        {
            state.messages.clear();
            state.list.reset(0);
            state.rich.clear();
            state.files.clear();
            state.markdown = MarkdownCache::default();
            state.horizontal.clear();
            state.loaded = false;
        }
        self.popup = None;
        self.save_session();
        cx.notify();
    }

    fn draft_changed(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.states.get(&id).is_some_and(|s| s.queue_edit_pending) {
            cx.notify();
            return;
        }
        if self.is_new_chat(id) {
            self.save_new_chat_draft(cx);
            self.save_session();
            cx.notify();
            return;
        }
        if let Some(state) = self.states.get_mut(&id)
            && state.loaded
        {
            let draft = state.composer.read(cx).draft(state.attachments.clone());
            if state.retry_of.is_some()
                && state
                    .messages
                    .iter()
                    .rfind(|m| m.role == MessageRole::User)
                    .is_none_or(|m| m.body != draft.text)
            {
                state.retry_of = None;
            }
            let _ = self.persist.send(Persist::Draft(id, draft));
        }
        cx.notify();
    }

    fn capture_scroll(&mut self) {
        for (id, state) in &self.states {
            if !state.loaded {
                continue;
            }
            let position = state.list.logical_scroll_top();
            let offset: f32 = position.offset_in_item.into();
            self.session.scroll_positions.insert(
                id.to_string(),
                (
                    if state.follow {
                        state.messages.len()
                    } else {
                        position.item_ix
                    },
                    offset,
                ),
            );
        }
    }
    pub(crate) fn save_session(&mut self) {
        self.capture_scroll();
        let _ = self.persist.send(Persist::Session(self.session.clone()));
    }
    pub fn persist_all(&mut self, cx: &mut Context<Self>) -> oneshot::Receiver<()> {
        let ids: Vec<_> = self.states.keys().copied().collect();
        for id in ids {
            self.draft_changed(id, cx);
        }
        self.save_session();
        let (done, receiver) = oneshot::channel();
        let _ = self.persist.send(Persist::Barrier(done));
        receiver
    }

    fn send(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if !self.states.contains_key(&id) || !self.prepare_skill_send(id, cx) {
            return;
        }
        if self.global_settings_save.is_saving() {
            return;
        }
        if let Some(state) = self.states.get_mut(&id)
            && !self
                .app_settings
                .provider(state.preferences.execution.provider)
                .enabled
        {
            state.error = Some(
                "This provider is disabled. Enable it in Settings or choose another model.".into(),
            );
            cx.notify();
            return;
        }

        if let Some(state) = self.states.get_mut(&id)
            && !matches!(
                self.provider_status
                    .get(&state.preferences.execution.provider),
                Some(ProviderReadiness::Ready)
            )
        {
            state.error = Some("Check this provider’s installation and sign-in in Settings, then Refresh before sending.".into());
            cx.notify();
            return;
        }
        let no_visible = self.states.get(&id).is_some_and(|state| {
            state.preferences.execution.model.is_none()
                && self
                    .catalogs
                    .get(&state.preferences.execution.provider)
                    .and_then(|r| r.as_ref().ok())
                    .is_none_or(|c| {
                        !c.models
                            .iter()
                            .any(|m| self.app_settings.visible(c.provider, &m.id))
                    })
        });
        if no_visible {
            if let Some(state) = self.states.get_mut(&id) {
                state.error = Some("Enable a visible model in Settings before sending.".into());
            }
            cx.notify();
            return;
        }
        let Some(state) = self.states.get_mut(&id) else {
            return;
        };
        if !state.loaded
            || state.queue_edit_pending
            || state.importing
            || state.settings_pending
            || state.settings_error
            || state.enqueue_pending
        {
            return;
        }
        let draft = state.composer.read(cx).draft(state.attachments.clone());
        if draft.text.trim().is_empty() && draft.attachments.is_empty() {
            return;
        }
        state.enqueue_pending = true;
        state.error = None;
        let command = if let Some(new_chat) = self
            .session
            .new_chat
            .as_mut()
            .filter(|new_chat| new_chat.id == id.to_string())
        {
            new_chat.submitted = Some(draft.clone());
            let preferences = state.preferences.clone();
            self.save_new_chat_draft(cx);
            self.save_session();
            Persist::StartChat(StartChatInput {
                chat_id: id,
                preferences,
                draft,
            })
        } else {
            Persist::Enqueue(id, draft)
        };
        if self.persist.send(command).is_err() {
            let state = self.states.get_mut(&id).expect("chat view exists");
            state.enqueue_pending = false;
            state.error = Some("The message queue is unavailable. Your draft is retained.".into());
        }
        cx.notify();
    }

    fn search_changed(&mut self, cx: &mut Context<Self>) {
        self.search_query = self.search.read(cx).content();
        self.search_generation += 1;
        let generation = self.search_generation;
        let query = self.search_query.clone();
        self.search_loading = !query.trim().is_empty();
        if !self.search_loading {
            self.search_results.clear();
            cx.notify();
            return;
        }
        let service = self.services.workspace.clone();
        let sender = self.sender.clone();
        // One cancellable debounce task per edit, rather than issuing every query.
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(180))
                .await;
            let current = this
                .read_with(cx, |this, _| this.search_generation == generation)
                .unwrap_or(false);
            if current {
                let runtime = this
                    .read_with(cx, |this, _| this.services.runtime.clone())
                    .ok();
                if let Some(runtime) = runtime {
                    runtime.spawn(async move {
                        let result = service.search(&query).await.map_err(|e| e.to_string());
                        let _ = sender.send(UiEvent::Search(generation, result));
                    });
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn import_files(&mut self, id: Uuid, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(state) = self.states.get_mut(&id) else {
            return;
        };
        if state.importing || state.queue_edit_pending {
            return;
        }
        state.importing = true;
        state.error = None;
        let storage = self.services.attachments.clone();
        let sender = self.sender.clone();
        self.services.runtime.spawn(async move {
            let result = storage.import(paths).await.map_err(|e| e.to_string());
            let _ = sender.send(UiEvent::Imported(id, result));
        });
        cx.notify();
    }

    fn pick_files(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach to chat".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await {
                let _ = this.update(cx, |this, cx| this.import_files(id, paths, cx));
            }
        })
        .detach();
    }

    pub(crate) fn save_preferences(
        &mut self,
        id: Uuid,
        mut prefs: ChatPreferences,
        cx: &mut Context<Self>,
    ) {
        if self.states.get(&id).is_some_and(|s| s.activity.is_some()) {
            return;
        }
        let old = self.metadata.get(&id).cloned().unwrap_or_default();
        if old.execution.provider != prefs.execution.provider || old.project_id != prefs.project_id
        {
            prefs.new_session = true;
        }
        self.metadata.insert(id, prefs.clone());
        let new_chat = self.is_new_chat(id);
        if let Some(state) = self.states.get_mut(&id) {
            state.preferences = prefs.clone();
            state.settings_pending = !new_chat;
            state.settings_error = false;
        }
        if new_chat {
            self.draft_changed(id, cx);
        } else {
            let _ = self.persist.send(Persist::Preferences(id, prefs));
        }
        if self.popup.is_some() {
            self.focus_composer = true;
        }
        self.popup = None;
        cx.notify();
    }

    pub(crate) fn respond(&mut self, id: Uuid, approved: bool, cx: &mut Context<Self>) {
        let Some(state) = self.states.get_mut(&id) else {
            return;
        };
        let Some(pending) = state.pending.take() else {
            return;
        };
        if matches!(&pending.request.kind, InteractionKind::Approval(_)) {
            clear_approval_text(state, &pending.request.id);
        }
        self.focus_composer = true;
        let mut answers = pending.answers;
        if let InteractionKind::Questions(questions) = &pending.request.kind {
            for (index, question) in questions.iter().enumerate() {
                let text = pending.editors[index].read(cx).content().trim().to_owned();
                if !text.is_empty() {
                    answers.entry(question.id.clone()).or_default().push(text);
                }
            }
            if questions
                .iter()
                .any(|q| answers.get(&q.id).is_none_or(|a| a.is_empty()))
            {
                state.error = Some("Answer each question before continuing.".into());
                state.interaction_scroll = ScrollHandle::new();
                state.pending = Some(PendingInteraction {
                    details: false,
                    request: pending.request,
                    editors: pending.editors,
                    answers,
                });
                return;
            }
        }
        if matches!(pending.request.kind, InteractionKind::LeavePlan { .. }) && approved {
            state.preferences.execution.plan_mode = false;
            self.metadata.insert(id, state.preferences.clone());
            let _ = self
                .persist
                .send(Persist::Preferences(id, state.preferences.clone()));
        }
        {
            let _ = self.persist.send(Persist::Respond(
                id,
                InteractionResponse {
                    id: pending.request.id,
                    approved,
                    answers,
                },
            ));
        }
        state.activity = Some(Activity::Working);
        state.error = None;
        cx.notify();
    }

    fn begin_project(&mut self, id: Option<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        let project = id.and_then(|id| self.projects.iter().find(|p| p.id == id));
        self.name_editor.update(cx, |s, cx| {
            s.set_content(
                project.map(|p| p.name.clone()).unwrap_or_default(),
                window,
                cx,
            )
        });
        self.folder_editor.update(cx, |s, cx| {
            s.set_content(
                project
                    .and_then(|p| p.folder.as_ref().map(|p| p.display().to_string()))
                    .unwrap_or_default(),
                window,
                cx,
            )
        });
        self.name_editor.focus_handle(cx).focus(window);
        self.popup = Some(Popup::Project(id));
        cx.notify();
    }

    fn save_project(&mut self, id: Option<Uuid>, cx: &mut Context<Self>) {
        if id.is_some_and(|id| {
            self.states
                .values()
                .any(|s| s.activity.is_some() && s.preferences.project_id == Some(id.to_string()))
        }) {
            self.banner =
                Some("Wait for this project’s active chats before changing its folder.".into());
            return;
        }
        let name = self.name_editor.read(cx).content();
        let folder = self.folder_editor.read(cx).content();
        if name.trim().is_empty() {
            self.banner = Some("A project name cannot be empty.".into());
            cx.notify();
            return;
        }
        let folder = folder.trim();
        let folder = if folder.is_empty() {
            None
        } else if let Some(rest) = folder.strip_prefix("~/") {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(rest))
        } else {
            Some(PathBuf::from(folder))
        };
        if folder.as_ref().is_some_and(|p| !p.is_dir()) {
            self.banner =
                Some("The working folder does not exist. Choose an existing directory.".into());
            cx.notify();
            return;
        }
        let _ = self.persist.send(Persist::Project(Project {
            id: id.unwrap_or_else(Uuid::new_v4),
            name,
            folder,
        }));
        self.popup = None;
        self.focus_composer = true;
        cx.notify();
    }

    fn switch_tab(&mut self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.session.open_tabs.len();
        if len == 0 {
            return;
        }
        let index = self
            .active
            .and_then(|id| {
                self.session
                    .open_tabs
                    .iter()
                    .position(|s| *s == id.to_string())
            })
            .unwrap_or(0);
        let next = (index as isize + direction).rem_euclid(len as isize) as usize;
        if let Ok(id) = Uuid::parse_str(&self.session.open_tabs[next]) {
            self.open_chat(id, None, cx);
            if let Some(s) = self.states.get(&id) {
                s.composer.focus_handle(cx).focus(window);
            }
        }
    }

    fn display_title(&self, chat: &Chat) -> String {
        display_title(&chat.title, self.previews.get(&chat.id).map(String::as_str))
    }

    fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.reset(cx));
        self.search_changed(cx);
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.session.sidebar_collapsed = false;
        self.search.focus_handle(cx).focus(window);
        self.save_session();
        cx.notify();
    }
}

fn upsert(messages: &mut Vec<Message>, message: Message) -> usize {
    if let Some(index) = messages.iter().position(|m| m.id == message.id) {
        messages[index] = message;
        index
    } else {
        messages.push(message);
        messages.len() - 1
    }
}

pub(crate) fn element_id(kind: &'static str, value: impl ToString) -> gpui::ElementId {
    (gpui::ElementId::from(kind), value.to_string()).into()
}

impl ChatsView {
    pub(crate) fn selected_model(&self, id: Uuid) -> Option<&ModelCapability> {
        let prefs = &self.states.get(&id)?.preferences.execution;
        let catalog = self.catalogs.get(&prefs.provider)?.as_ref().ok()?;
        match &prefs.model {
            Some(model) => catalog.models.iter().find(|m| m.id == *model),
            None => catalog
                .models
                .iter()
                .find(|m| m.is_default)
                .or_else(|| catalog.models.first()),
        }
    }
}

impl Render for ChatsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.benchmark_frame(window, cx);
        if self.model_cancelled {
            self.model_cancelled = false;
            if self.settings_open {
                self.close_settings(window, cx);
            } else {
                self.dismiss_popup(window, cx);
            }
        }
        if let Some(action) = self.picker_requested.take() {
            self.choose_picker(action, window, cx);
        }

        let window_width = window.bounds().size.width;
        if window_width < px(720.)
            && (self.last_window_width == px(0.) || self.last_window_width >= px(720.))
        {
            self.session.sidebar_collapsed = true;
            self.save_session();
        }
        self.last_window_width = window_width;
        let compact = window_width < px(720.);
        self.content_inset = px(if compact { 12. } else { 24. });
        self.sync_repository(window, cx);
        let sidebar = self.render_sidebar(cx);
        let (sidebar, navigation_overlay) = if compact && !self.session.sidebar_collapsed {
            let overlay = div()
                .id("compact-navigation-backdrop")
                .absolute()
                .inset_0()
                .occlude()
                .bg(gpui::rgba(0x00000044))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.session.sidebar_collapsed = true;
                    this.save_session();
                    cx.notify();
                }))
                .child(
                    div()
                        .id("compact-navigation-panel")
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(SIDEBAR_WIDTH))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(sidebar),
                );
            (
                div()
                    .w(px(SIDEBAR_RAIL_WIDTH))
                    .flex_shrink_0()
                    .into_any_element(),
                Some(overlay.into_any_element()),
            )
        } else {
            (sidebar, None)
        };
        let weak = cx.entity().downgrade();
        let mut detail = div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(self.render_tabs(cx))
            .child(self.render_repository_header(cx))
            .on_children_prepainted(move |bounds, _, cx| {
                if let Some(bounds) = bounds.first() {
                    let width = bounds.size.width;
                    let weak = weak.clone();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |this, cx| {
                            if this.detail_width != width {
                                this.detail_width = width;
                                cx.notify();
                            }
                        });
                    });
                }
            });
        let width = (self.detail_width - self.content_inset * 2.)
            .max(px(1.))
            .min(px(800.));
        if width != self.last_width {
            self.last_width = width;
            for state in self.states.values() {
                let offset = state.list.logical_scroll_top();
                state.list.reset(state.messages.len());
                if state.follow {
                    state.list.scroll_to(ListOffset {
                        item_ix: state.messages.len(),
                        offset_in_item: px(0.),
                    });
                } else {
                    state.list.scroll_to(offset);
                }
            }
        }
        if let Some(id) = self.active.filter(|id| self.states.contains_key(id)) {
            let pending = self.states[&id].pending.is_some();
            if self.focus_composer && self.states[&id].loaded && !self.settings_open {
                if pending && window.viewport_size().height < px(600.) {
                    self.focus.focus(window);
                } else {
                    self.states[&id].composer.focus_handle(cx).focus(window);
                }
                self.focus_composer = false;
            }
            self.states[&id].composer.update(cx, |composer, cx| {
                composer.set_max_rows(
                    composer_rows(f32::from(window.viewport_size().height), pending),
                    cx,
                )
            });
            detail = detail.child(self.render_transcript(id, window, cx));
            let compact_request = pending && window.viewport_size().height < px(600.);
            let interaction_height = if compact_request {
                (window.viewport_size().height * 0.35).clamp(px(140.), px(180.))
            } else {
                (window.viewport_size().height * 0.3).clamp(px(180.), px(280.))
            };
            if let Some(interaction) = self.render_interaction(id, interaction_height, cx) {
                detail = detail.child(
                    div()
                        .flex_shrink_0()
                        .id("interaction-viewport")
                        .max_h(interaction_height)
                        .w_full()
                        .flex()
                        .justify_center()
                        .px(self.content_inset)
                        .child(interaction),
                );
            }
            let state = &self.states[&id];
            let status = state
                .activity
                .filter(|_| state.pending.is_none())
                .map(|a| a.label().to_owned());
            let error = state.error.clone();
            if let Some(status) = status.filter(|_| !compact_request) {
                detail = detail.child(
                    div()
                        .flex_shrink_0()
                        .px(self.content_inset)
                        .py_1()
                        .text_size(px(13.))
                        .text_color(rgb(ACCENT))
                        .w_full()
                        .flex()
                        .justify_center()
                        .child(div().w_full().max_w(px(800.)).child(status)),
                );
            }
            if let Some(error) = error {
                let expanded = self.states[&id].error_details;
                let summary = error_summary(&error, self.states[&id].settings_error, pending);
                detail = detail.child(
                    div()
                        .w_full()
                        .flex()
                        .justify_center()
                        .px(self.content_inset)
                        .py_2()
                        .flex_shrink_0()
                        .child(
                            div()
                                .id("chat-error")
                                .w_full()
                                .max_w(px(800.))
                                .max_h(px(if compact_request { 36. } else { 120. }))
                                .overflow_y_scroll()
                                .text_sm()
                                .child(
                                    row()
                                        .child(
                                            div().flex_1().text_color(rgb(DANGER)).child(summary),
                                        )
                                        .child(
                                            button(
                                                "chat-error-details",
                                                if expanded { "Hide details" } else { "Details" },
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    if let Some(s) = this.states.get_mut(&id) {
                                                        s.error_details = !s.error_details;
                                                    }
                                                    cx.notify();
                                                }),
                                            ),
                                        ),
                                )
                                .when(expanded, |d| {
                                    d.child(div().text_color(rgb(MUTED)).child(error))
                                }),
                        ),
                );
            }
            if let Some(queue) = self.render_queue(id, window.viewport_size().height, cx) {
                detail = detail.child(
                    div()
                        .flex_shrink_0()
                        .w_full()
                        .flex()
                        .justify_center()
                        .px(self.content_inset)
                        .child(queue),
                );
            }
            // Keep a compact composer available for queuing while an approval is pending.
            {
                detail = detail.child(
                    div()
                        .flex_shrink_0()
                        .w_full()
                        .flex()
                        .justify_center()
                        .px(self.content_inset)
                        .pt_2()
                        .pb_2()
                        .child(self.render_composer(id, window, cx)),
                );
            }
            if pending && !compact_request {
                detail = detail.child(
                    div()
                        .flex_shrink_0()
                        .px_3()
                        .pb_3()
                        .text_center()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("Your draft is saved."),
                );
            }
        } else {
            if self.focus_composer && !self.settings_open {
                self.focus.focus(window);
                self.focus_composer = false;
            }
            detail = detail.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child("What’s on your mind?"),
                    )
                    .child(muted("Open a chat from the sidebar, or start a new one."))
                    .child(
                        button("empty-new-chat", "+  New chat")
                            .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx))),
                    ),
            );
        }
        if let Some(error) = &self.banner {
            detail = detail.child(
                row()
                    .px_4()
                    .py_2()
                    .bg(rgb(PANEL))
                    .child(div().flex_1().text_sm().text_color(rgb(DANGER)).child(
                        "Couldn’t save a workspace change. Check the details before trying again.",
                    ))
                    .child(
                        button(
                            "storage-details",
                            if self.banner_details {
                                "Hide details"
                            } else {
                                "Details"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.banner_details = !this.banner_details;
                            cx.notify();
                        })),
                    )
                    .when(self.banner_details, |view| {
                        view.child(div().flex_1().text_sm().child(error.clone()))
                    })
                    .child(
                        button("dismiss-error", "×").on_click(cx.listener(|this, _, _, cx| {
                            this.banner = None;
                            cx.notify();
                        })),
                    ),
            );
        }
        if self.settings_open {
            detail = self.render_settings(window, cx);
        }
        let popup = self.render_popup(window, cx);
        let review = if self.repository.open && !self.settings_open {
            Some(self.render_repository_panel(window, cx))
        } else {
            None
        };
        let review_overlay = window.viewport_size().width < px(1100.);
        let (review_panel, review_overlay) = if review_overlay {
            (None, review)
        } else {
            (review, None)
        };
        div()
            .relative()
            .size_full()
            .flex()
            .overflow_hidden()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .font_family(".AppleSystemUIFont")
            .text_size(px(15.))
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                this.handle_workspace_key(event, window, cx);
            }))
            .capture_key_up(cx.listener(|this, event: &gpui::KeyUpEvent, window, cx| {
                this.handle_settings_key_up(event, window, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::OpenSettings, window, cx| {
                this.open_settings(window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::NewChat, _, cx| this.new_chat(cx)))
            .on_action(cx.listener(|this, _: &crate::CloseTab, window, cx| {
                if this.settings_open {
                    this.close_settings(window, cx);
                    return;
                }
                if let Some(id) = this.active {
                    this.close_tab(id, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &crate::NextTab, window, cx| this.switch_tab(1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &crate::PreviousTab, window, cx| {
                this.switch_tab(-1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleSidebar, _, cx| {
                this.session.sidebar_collapsed = !this.session.sidebar_collapsed;
                this.save_session();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::SearchChats, window, cx| {
                if this.settings_open {
                    if matches!(this.popup, Some(Popup::Settings(_))) {
                        this.dismiss_popup(window, cx);
                    }
                    this.settings_section = settings::SettingsSection::Providers;
                    cx.notify();
                }
                if matches!(this.popup, Some(Popup::Models | Popup::Skills)) || this.settings_open {
                    this.model_search.focus_handle(cx).focus(window);
                } else {
                    this.focus_search(window, cx);
                }
            }))
            .child(sidebar)
            .child(detail)
            .children(review_panel)
            .children(review_overlay)
            .children(navigation_overlay)
            .children(popup)
    }
}
