//! Drives existing UI handlers and records semantic events only in explicit benchmark mode.
use super::*;
use crate::benchmark::{EventLog, Session};
use serde_json::{Value, json};

pub(super) struct State {
    log: Arc<EventLog>,
    ready: bool,
    submitted: bool,
    first_content: bool,
    completion: Option<Uuid>,
    frame_pending: Option<Value>,
    frame_scheduled: bool,
}

impl ChatsView {
    pub(crate) fn attach_benchmark(&mut self, session: Session, cx: &mut Context<Self>) {
        let Session { log, mut commands } = session;
        self.benchmark = Some(State {
            log,
            ready: false,
            submitted: false,
            first_content: false,
            completion: None,
            frame_pending: None,
            frame_scheduled: false,
        });
        cx.spawn(async move |this, cx| {
            while let Some(command) = commands.recv().await {
                if this
                    .update(cx, |this, cx| this.benchmark_command(command, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn benchmark_command(&mut self, command: Value, cx: &mut Context<Self>) {
        let Some(benchmark) = self.benchmark.as_mut() else {
            return;
        };
        let request = command["id"].clone();
        match command["command"].as_str() {
            Some("quit") => {
                benchmark.log.emit("quitting", json!({}));
                cx.quit();
            }
            Some("send") if benchmark.ready && !benchmark.submitted => {
                benchmark.submitted = true;
                let Some(id) = self.active else {
                    return;
                };
                benchmark.log.emit(
                    "submitted",
                    json!({"chat_id":id.to_string(),"request":request}),
                );
                self.send(id, cx);
                if !self.states.get(&id).is_some_and(|s| s.enqueue_pending) {
                    self.benchmark.as_ref().unwrap().log.emit(
                        "error",
                        json!({"message":"UI rejected benchmark send",
                        "details":self.states.get(&id).and_then(|s| s.error.clone())}),
                    );
                }
            }
            Some("open") => {
                let id = command["chat_id"]
                    .as_str()
                    .and_then(|id| Uuid::parse_str(id).ok());
                if let Some(id) = id.filter(|id| self.chats.iter().any(|chat| chat.id == *id)) {
                    self.open_chat(id, None, cx);
                    self.benchmark.as_mut().unwrap().frame_pending =
                        Some(json!({"request":request,"action":"open","chat_id":id.to_string()}));
                } else {
                    self.benchmark
                        .as_ref()
                        .unwrap()
                        .log
                        .emit("error", json!({"message":"unknown benchmark chat"}));
                }
            }
            Some("close") => {
                if let Some(id) = self.active {
                    self.close_tab(id, cx);
                }
                self.benchmark.as_mut().unwrap().frame_pending =
                    Some(json!({"request":request,"action":"close"}));
            }
            Some("scroll") => {
                if let Some(state) = self.active.and_then(|id| self.states.get_mut(&id)) {
                    let index = if command["end"] == true {
                        state.messages.len()
                    } else {
                        0
                    };
                    state.follow = command["end"] == true;
                    state.list.scroll_to(ListOffset {
                        item_ix: index,
                        offset_in_item: px(0.),
                    });
                }
                self.benchmark.as_mut().unwrap().frame_pending =
                    Some(json!({"request":request,"action":"scroll"}));
            }
            _ => benchmark.log.emit(
                "error",
                json!({"message":"invalid benchmark command or state","request":request}),
            ),
        }
        cx.notify();
    }

    pub(super) fn benchmark_event(&mut self, event: &UiEvent) {
        let Some(benchmark) = self.benchmark.as_mut() else {
            return;
        };
        match event {
            UiEvent::Queue(QueueEvent::Started(id)) => benchmark
                .log
                .emit("execution_started", json!({"chat_id":id.to_string()})),
            UiEvent::Queue(QueueEvent::Conversation(ConversationEvent::AssistantDelta {
                delta,
                ..
            })) if !delta.is_empty() && !benchmark.first_content => {
                benchmark.first_content = true;
                benchmark.log.emit("first_content", json!({}));
            }
            UiEvent::Queue(QueueEvent::Conversation(ConversationEvent::AssistantSnapshot {
                text,
                ..
            })) if !text.is_empty() && !benchmark.first_content => {
                benchmark.first_content = true;
                benchmark.log.emit("first_content", json!({}));
            }
            UiEvent::Finished(id, result) => {
                benchmark.log.emit("execution_finished", json!({"chat_id":id.to_string(),"ok":result.is_ok(),"error":result.as_ref().err()}));
                if result.is_ok() {
                    benchmark.completion = Some(*id);
                } else {
                    benchmark.log.emit(
                        "error",
                        json!({"message":"execution failed","details":result.as_ref().err()}),
                    );
                }
            }
            UiEvent::StorageError(error) => {
                benchmark
                    .log
                    .emit("error", json!({"message":error.to_string()}));
            }
            UiEvent::Queue(QueueEvent::Error(_, error)) => benchmark
                .log
                .emit("error", json!({"message":error.to_string()})),
            _ => {}
        }
    }

    pub(super) fn benchmark_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(benchmark) = self.benchmark.as_ref() else {
            return;
        };
        if benchmark.frame_scheduled {
            return;
        }
        if benchmark
            .completion
            .is_some_and(|id| self.active != Some(id))
        {
            benchmark.log.emit(
                "error",
                json!({"message":"completed benchmark chat is not the active view"}),
            );
            return;
        }
        let loaded = self
            .active
            .is_none_or(|id| self.states.get(&id).is_some_and(|s| s.loaded));
        let provider_ready = matches!(
            self.provider_status.get(&Provider::Codex),
            Some(ProviderReadiness::Ready)
        );
        if !benchmark.ready && !provider_ready {
            if let Some(
                status @ (ProviderReadiness::Unavailable(_)
                | ProviderReadiness::Missing
                | ProviderReadiness::SignedOut),
            ) = self.provider_status.get(&Provider::Codex)
            {
                benchmark.log.emit(
                    "error",
                    json!({"message":format!("provider unavailable: {status:?}")}),
                );
            }
            return;
        }
        if loaded
            && (!benchmark.ready
                || benchmark.completion.is_some()
                || benchmark.frame_pending.is_some())
        {
            self.benchmark.as_mut().unwrap().frame_scheduled = true;
            cx.on_next_frame(window, |this, _, _| {
                let benchmark = this.benchmark.as_mut().unwrap();
                benchmark.frame_scheduled = false;
                if !benchmark.ready {
                    benchmark.ready = true;
                    benchmark.log.emit("ui_ready", json!({"chat_id":this.active.map(|id|id.to_string())}));
                }
                if let Some(id) = benchmark.completion.take() {
                    let bytes: usize = this.states.get(&id).map(|s| s.messages.iter()
                        .filter(|m|m.role == MessageRole::Assistant).map(|m|m.body.len()).sum()).unwrap_or(0);
                    benchmark.log.emit("rendered_complete", json!({"chat_id":id.to_string(),"assistant_bytes":bytes,
                        "evidence":"GPUI callback after completed view frame; not a pixel assertion"}));
                }
                if let Some(mut data) = benchmark.frame_pending.take() {
                    data["active_chat_id"] = json!(this.active.map(|id|id.to_string()));
                    if let Some(state) = this.active.and_then(|id|this.states.get(&id)) {
                        data["message_count"] = json!(state.messages.len());
                        data["scroll_item"] = json!(state.list.logical_scroll_top().item_ix);
                    }
                    benchmark.log.emit("view_rendered", data);
                }
            });
        }
    }
}
