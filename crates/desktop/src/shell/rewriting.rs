use super::*;
#[derive(Default)]
pub(super) struct RewriteUi {
    pub sources: HashMap<Uuid, String>,
    pub variants: HashMap<Uuid, ResponseVariant>,
    pub displayed: std::collections::HashSet<Uuid>,
    pending: std::collections::HashSet<Uuid>,
    errors: HashMap<Uuid, String>,
    pub support: HashMap<Provider, Result<(), String>>,
}
impl ChatsView {
    pub(super) fn release_response_display(&mut self, chat: Uuid) {
        if let Some(state) = self.states.get(&chat) {
            for message in &state.messages {
                self.rewriting.sources.remove(&message.id);
                self.rewriting.variants.remove(&message.id);
                self.rewriting.displayed.remove(&message.id);
                self.rewriting.errors.remove(&message.id);
            }
        }
    }
    pub(super) fn load_response_display(&self, chat: Uuid) {
        let store = self.services.display.clone();
        let sender = self.sender.clone();
        self.services.runtime.spawn(async move {
            let result = async {
                Ok((
                    store.final_answers(chat).await.map_err(|e| e.to_string())?,
                    store.variants(chat).await.map_err(|e| e.to_string())?,
                ))
            }
            .await;
            let _ = sender.send(UiEvent::DisplayLoaded(chat, result));
        });
    }
    pub(super) fn response_display_loaded(
        &mut self,
        chat: Uuid,
        result: Result<(Vec<FinalAnswer>, Vec<ResponseVariant>), String>,
        cx: &mut Context<Self>,
    ) {
        if !self.states.get(&chat).is_some_and(|s| s.loaded) {
            return;
        }
        match result {
            Ok((sources, variants)) => {
                for source in sources {
                    self.rewriting
                        .sources
                        .insert(source.message_id, source.text);
                }
                for variant in variants {
                    if self.variant_matches(&variant) {
                        self.rewriting.variants.insert(variant.message_id, variant);
                    }
                }
            }
            Err(error) => self.banner = Some(error),
        }
        cx.notify();
    }
    fn variant_matches(&self, v: &ResponseVariant) -> bool {
        v.prompt_version == REWRITE_PROMPT_VERSION
            && self
                .rewriting
                .sources
                .get(&v.message_id)
                .is_some_and(|s| source_hash(s) == v.source_hash)
            && self
                .app_settings
                .rewrite_model
                .as_ref()
                .is_some_and(|s| s.provider == v.provider && s.model.as_ref() == Some(&v.model))
    }
    pub(crate) fn showing_shortened(&self, message: Uuid) -> bool {
        self.rewriting.displayed.contains(&message)
            && self
                .rewriting
                .variants
                .get(&message)
                .is_some_and(|v| self.variant_matches(v))
    }
    pub(crate) fn displayed_response_body(&self, message: &Message) -> String {
        if self.rewriting.displayed.contains(&message.id)
            && let Some(variant) = self
                .rewriting
                .variants
                .get(&message.id)
                .filter(|v| self.variant_matches(v))
            && let Some(source) = self.rewriting.sources.get(&message.id)
            && let Some(prefix) = message.body.strip_suffix(source)
        {
            return format!("{prefix}{}", variant.text);
        }
        message.body.clone()
    }
    pub(super) fn maybe_shorten_latest(&mut self, chat: Uuid, cx: &mut Context<Self>) {
        if !self.app_settings.automatic_shortening {
            return;
        }
        if !self.app_settings.rewrite_model.as_ref().is_some_and(|s| {
            self.app_settings.provider(s.provider).enabled
                && self
                    .rewriting
                    .support
                    .get(&s.provider)
                    .is_some_and(|r| r.is_ok())
        }) {
            return;
        }
        if let Some(message) = self
            .states
            .get(&chat)
            .and_then(|s| s.messages.last())
            .map(|m| m.id)
            && self
                .rewriting
                .sources
                .get(&message)
                .is_some_and(|s| prose_words(s) > AUTO_SHORTEN_WORDS)
        {
            self.request_shorten(chat, message, cx);
        }
    }
    fn request_shorten(&mut self, chat: Uuid, message: Uuid, cx: &mut Context<Self>) {
        if self.rewriting.pending.contains(&message) {
            return;
        }
        let Some(settings) = self.app_settings.rewrite_model.clone() else {
            return;
        };
        let support = self
            .rewriting
            .support
            .get(&settings.provider)
            .and_then(|r| r.as_ref().err());
        if let Some(error) = support {
            self.rewriting.errors.insert(message, error.clone());
            cx.notify();
            return;
        }
        self.rewriting.pending.insert(message);
        self.rewriting.errors.remove(&message);
        let rewriter = self.services.rewriter.clone();
        let sender = self.sender.clone();
        self.services.runtime.spawn(async move {
            let result = rewriter
                .shorten(ShortenInput {
                    chat_id: chat,
                    message_id: message,
                    settings,
                })
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(UiEvent::Rewritten(chat, message, result));
        });
        cx.notify();
    }
    pub(super) fn rewrite_result(
        &mut self,
        chat: Uuid,
        message: Uuid,
        result: Result<ResponseVariant, String>,
        cx: &mut Context<Self>,
    ) {
        self.rewriting.pending.remove(&message);
        match result {
            Ok(variant) if self.variant_matches(&variant) => {
                self.rewriting.variants.insert(message, variant);
                let safe = self.active == Some(chat)
                    && !self.settings_open
                    && self.states.get(&chat).is_some_and(|s| {
                        s.follow
                            && s.activity.is_none()
                            && s.messages.last().is_some_and(|m| m.id == message)
                            && !s.rich.values().any(|view| view.read(cx).has_selection())
                    });
                if safe {
                    self.rewriting.displayed.insert(message);
                    self.reset_response_layout(chat);
                }
            }
            Ok(_) => {}
            Err(error) => {
                self.rewriting.errors.insert(message, error);
            }
        }
        cx.notify();
    }
    fn reset_response_layout(&mut self, chat: Uuid) {
        if let Some(s) = self.states.get_mut(&chat) {
            let offset = s.list.logical_scroll_top();
            s.list.reset(s.messages.len());
            if s.follow {
                s.list.scroll_to(ListOffset {
                    item_ix: s.messages.len(),
                    offset_in_item: px(0.),
                });
            } else {
                s.list.scroll_to(offset);
            }
        }
    }
    pub(crate) fn render_rewrite_controls(
        &self,
        chat: Uuid,
        message: Uuid,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if !self.rewriting.sources.contains_key(&message) {
            return div().into_any_element();
        }
        let pending = self.rewriting.pending.contains(&message);
        let variant = self
            .rewriting
            .variants
            .get(&message)
            .is_some_and(|v| self.variant_matches(v));
        let showing = self.rewriting.displayed.contains(&message) && variant;
        let label = if pending {
            "Shortening…"
        } else if showing {
            "Original"
        } else if variant {
            "Shortened version ready"
        } else if self.rewriting.errors.contains_key(&message) {
            "Retry shortening"
        } else {
            "Shorten"
        };
        let mut controls = div().flex().flex_col().gap_1().child(
            row().child(
                button(element_id("shorten-response", message), label)
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if pending {
                            return;
                        }
                        if variant {
                            if !this.rewriting.displayed.remove(&message) {
                                this.rewriting.displayed.insert(message);
                            }
                            this.reset_response_layout(chat);
                            cx.notify();
                        } else if this.app_settings.rewrite_model.is_none() {
                            this.open_settings(window, cx);
                        } else {
                            this.request_shorten(chat, message, cx);
                        }
                    })),
            ),
        );
        if let Some(error) = self.rewriting.errors.get(&message) {
            controls = controls.child(div().text_xs().text_color(rgb(MUTED)).child(error.clone()));
        }
        controls.into_any_element()
    }
}
