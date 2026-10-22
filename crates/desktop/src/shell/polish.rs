use super::*;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone)]
pub(super) enum PickerAction {
    Skill(SkillReference),
    Model(Provider, String),
    Effort(Option<String>),
    Retry(Provider),
    SettingsModel(settings::SettingsPicker, Provider, String),
    SettingsEffort(Option<String>),
}

pub(super) struct PickerChoice {
    pub label: String,
    pub selected: bool,
    pub action: PickerAction,
}

pub(super) fn display_title(title: &str, preview: Option<&str>) -> String {
    if !title.trim().is_empty() && title != "Untitled" {
        return title.to_owned();
    }
    let text = preview
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return "Untitled".into();
    }
    let mut parts = text.graphemes(true);
    let mut title: String = parts.by_ref().take(64).collect();
    if parts.next().is_some() {
        title.push('…');
    }
    title
}

pub(super) fn effort_label(effort: &str) -> String {
    match effort {
        "low" => "Low".into(),
        "medium" => "Medium".into(),
        "high" => "High".into(),
        "xhigh" => "Extra high".into(),
        "max" => "Maximum".into(),
        "ultra" => "Ultra".into(),
        "minimal" => "Minimal".into(),
        "none" => "None".into(),
        other => other.to_owned(),
    }
}

impl ChatsView {
    pub(super) fn open_context(
        &mut self,
        popup: Popup,
        anchor: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_anchor = anchor;
        self.popup_return_focus = window.focused(cx);
        self.popup = Some(popup);
        self.popup_focus.focus(window);
        cx.notify();
    }

    pub(super) fn picker_choices(&self) -> Vec<PickerChoice> {
        if matches!(self.popup, Some(Popup::Skills)) {
            return self.skill_choices();
        }
        if let Some(Popup::Settings(picker)) = self.popup {
            return self.settings_picker_choices(picker);
        }
        let Some(id) = self.active else {
            return Vec::new();
        };
        let settings = &self.states[&id].preferences.execution;
        match self.popup {
            Some(Popup::Models) => [Provider::Codex, Provider::Claude]
                .into_iter()
                .filter(|provider| self.app_settings.provider(*provider).enabled)
                .flat_map(|provider| match self.catalogs.get(&provider) {
                    Some(Ok(catalog)) => catalog
                        .models
                        .iter()
                        .filter(|model| {
                            self.app_settings.visible(provider, &model.id)
                                && model_matches(
                                    &self.model_query,
                                    provider,
                                    &model.label,
                                    &model.id,
                                )
                        })
                        .map(|model| PickerChoice {
                            label: format!("{} · {}", provider.label(), model.label),
                            selected: settings.provider == provider
                                && settings
                                    .model
                                    .as_ref()
                                    .map_or(model.is_default, |m| *m == model.id),
                            action: PickerAction::Model(provider, model.id.clone()),
                        })
                        .collect(),
                    Some(Err(_)) => vec![PickerChoice {
                        label: format!("Retry {} discovery", provider.label()),
                        selected: false,
                        action: PickerAction::Retry(provider),
                    }],
                    None => Vec::new(),
                })
                .collect(),
            Some(Popup::Effort) => {
                let model = self.selected_model(id);
                let default = model
                    .and_then(|m| m.default_effort.as_deref())
                    .map(|s| format!("Default ({})", effort_label(s)))
                    .unwrap_or_else(|| "Provider default".into());
                let mut choices = vec![PickerChoice {
                    label: default,
                    selected: settings.effort.is_none(),
                    action: PickerAction::Effort(None),
                }];
                if let Some(model) = model {
                    choices.extend(model.efforts.iter().map(|effort| PickerChoice {
                        label: effort_label(effort),
                        selected: settings.effort.as_ref() == Some(effort),
                        action: PickerAction::Effort(Some(effort.clone())),
                    }));
                }
                choices
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn open_picker(
        &mut self,
        popup: Popup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.popup_return_focus = window.focused(cx);
        if matches!(popup, Popup::Models | Popup::Skills) {
            self.model_query.clear();
            self.model_search.update(cx, |e, cx| {
                e.set_picker_navigation(true);
                e.set_font_size(13.);
                e.set_content(String::new(), window, cx);
            });
        }
        self.popup = Some(popup);
        self.popup_scroll = ScrollHandle::new();
        self.popup_index = self
            .picker_choices()
            .iter()
            .position(|c| c.selected)
            .unwrap_or(0);
        self.popup_scroll.scroll_to_item(self.popup_index);
        if matches!(popup, Popup::Models | Popup::Skills) {
            self.model_search.focus_handle(cx).focus(window);
        } else {
            self.popup_focus.focus(window);
        }
        cx.notify();
    }

    pub(super) fn dismiss_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let return_to_composer = matches!(
            self.popup,
            Some(
                Popup::Models
                    | Popup::Effort
                    | Popup::Skills
                    | Popup::Attachments
                    | Popup::SkillDetails
            )
        ) && !self.settings_open;
        self.popup = None;
        if let Some(id) = self.active
            && let Some(state) = self.states.get(&id)
        {
            state
                .composer
                .update(cx, |e, _| e.set_picker_navigation(false));
        }
        if return_to_composer {
            self.popup_return_focus = None;
            if let Some(id) = self.active
                && let Some(state) = self.states.get(&id)
            {
                state.composer.focus_handle(cx).focus(window);
            } else {
                self.focus.focus(window);
            }
        } else if let Some(focus) = self.popup_return_focus.take() {
            focus.focus(window);
        } else {
            self.focus_composer = true;
        }
        cx.notify();
    }

    pub(super) fn choose_picker(
        &mut self,
        action: PickerAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let PickerAction::Skill(reference) = &action {
            self.select_skill(reference.clone(), window, cx);
            return;
        }
        self.dismiss_popup(window, cx);
        if matches!(
            action,
            PickerAction::SettingsModel(..) | PickerAction::SettingsEffort(_)
        ) {
            self.choose_settings(action, cx);
            return;
        }
        let Some(id) = self.active else {
            return;
        };
        let mut prefs = self.states[&id].preferences.clone();
        match action {
            PickerAction::Skill(_) => unreachable!(),
            PickerAction::Retry(provider) => {
                self.discover(provider);
                self.open_picker(Popup::Models, window, cx);
                return;
            }
            PickerAction::Model(provider, model) => {
                // Re-selecting the current model must not reset its effort.
                if prefs.execution.provider == provider
                    && self
                        .selected_model(id)
                        .is_some_and(|selected| selected.id == model)
                {
                    return;
                }
                prefs.execution.provider = provider;
                prefs.execution.model = Some(model);
                prefs.execution.effort = None;
            }
            PickerAction::Effort(effort) => prefs.execution.effort = effort,
            PickerAction::SettingsModel(..) | PickerAction::SettingsEffort(_) => unreachable!(),
        }
        self.save_preferences(id, prefs, cx);
    }

    pub(super) fn handle_workspace_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        if matches!(self.popup, Some(Popup::Skills))
            && self
                .active
                .and_then(|id| self.states.get(&id))
                .is_some_and(|s| s.composer.read(cx).composing())
        {
            return;
        }

        if self.settings_open
            && self.settings_section == settings::SettingsSection::Instructions
            && event.keystroke.modifiers.platform
            && key == "s"
        {
            self.save_instructions(cx);
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if self.settings_open && self.popup.is_none() && key == "escape" {
            self.close_settings(window, cx);
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if self.popup.is_some() && key == "escape" {
            self.dismiss_popup(window, cx);
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        // Commit Settings choices on release so restoring focus cannot reopen
        // the selector through GPUI's automatic keyboard click.
        if matches!(self.popup, Some(Popup::Settings(_))) && matches!(key, "enter" | "space") {
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if matches!(
            self.popup,
            Some(Popup::Models | Popup::Effort | Popup::Skills | Popup::Settings(_))
        ) {
            let searching = matches!(self.popup, Some(Popup::Models | Popup::Skills));
            if searching && !matches!(key, "up" | "down" | "enter" | "tab") {
                return;
            }
            let choices = self.picker_choices();
            if !choices.is_empty() {
                self.popup_index = self.popup_index.min(choices.len() - 1);
                match key {
                    "down" | "tab" if !event.keystroke.modifiers.shift => {
                        self.popup_index = (self.popup_index + 1) % choices.len()
                    }
                    "up" | "tab" => {
                        self.popup_index = (self.popup_index + choices.len() - 1) % choices.len()
                    }
                    "home" => self.popup_index = 0,
                    "end" => self.popup_index = choices.len() - 1,
                    "enter" | "space" => {
                        self.choose_picker(choices[self.popup_index].action.clone(), window, cx)
                    }
                    _ => {}
                }
                self.popup_scroll.scroll_to_item(self.popup_index);
            }
            window.prevent_default();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let context_menu = matches!(
            self.popup,
            Some(Popup::Chat(_) | Popup::Assign(_) | Popup::ProjectMenu(_))
        );
        if key == "tab" || (context_menu && matches!(key, "up" | "down")) {
            for _ in 0..128 {
                if event.keystroke.modifiers.shift || key == "up" {
                    window.focus_prev();
                } else {
                    window.focus_next();
                }
                if self.popup.is_none() || self.popup_focus.contains_focused(window, cx) {
                    break;
                }
            }
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    pub(super) fn picker_control(
        &self,
        key: &'static str,
        control: impl IntoElement,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let weak = cx.entity().downgrade();
        div()
            .child(control)
            .on_children_prepainted(move |bounds, _, cx| {
                if let Some(bounds) = bounds.first() {
                    let _ = weak.update(cx, |this, _| {
                        this.picker_bounds.insert(key, *bounds);
                    });
                }
            })
    }
}

// Leave room for history and fixed controls even at the minimum window height.
pub(super) fn composer_rows(height: f32, pending: bool) -> usize {
    let rows = if height < 600. {
        3
    } else if height < 760. {
        5
    } else {
        7
    };
    if pending && height < 600. {
        1
    } else if pending {
        rows.min(2)
    } else {
        rows
    }
}

pub(super) fn error_summary(error: &str, settings: bool, pending: bool) -> String {
    if settings {
        return "Couldn’t save chat settings. Choose the model or mode again before sending."
            .into();
    }
    if pending {
        return "This request needs attention. Review the details and answer above to continue."
            .into();
    }
    if error.len() <= 200
        && !error.contains("database")
        && !error.contains("(code:")
        && !error.contains("{\"")
    {
        return error.into();
    }
    "Couldn’t complete this operation. Your message is retained. Review the details before trying again.".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_titles_preserve_explicit_names_and_unicode() {
        assert_eq!(display_title("Named", Some("different")), "Named");
        assert_eq!(
            display_title("Untitled", Some("  First\n message  ")),
            "First message"
        );
        assert_eq!(display_title("", None), "Untitled");
        let result = display_title("Untitled", Some(&"👩‍💻".repeat(65)));
        assert_eq!(result.graphemes(true).count(), 65);
        assert!(result.ends_with('…'));
    }
    #[test]
    fn readable_effort_labels_preserve_unknown_provider_values() {
        assert_eq!(effort_label("xhigh"), "Extra high");
        assert_eq!(effort_label("provider-specific"), "provider-specific");
    }
}
