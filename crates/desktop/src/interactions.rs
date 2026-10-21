use crate::{
    shell::{ChatsView, element_id},
    theme::*,
};
use gpui::{AnyElement, Context, IntoElement, div, prelude::*, px, rgb};
use smaller_talk_domain::{ApprovalAction, InteractionKind};
use uuid::Uuid;

impl ChatsView {
    pub(crate) fn render_interaction(
        &mut self,
        id: Uuid,
        max_height: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (request, answers, editors, expanded, interaction_scroll, submitted_user) = {
            let state = self.states.get(&id)?;
            let pending = state.pending.as_ref()?;
            (
                pending.request.clone(),
                pending.answers.clone(),
                pending.editors.clone(),
                pending.details,
                state.interaction_scroll.clone(),
                state.submitted_user.unwrap_or_default(),
            )
        };
        let card = div()
            .id(element_id(
                "provider-interaction",
                format!("{id}:{}:{}", submitted_user, request.id),
            ))
            .w_full()
            .max_w(px(800.))
            .max_h(max_height)
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(quiet_border())
            .bg(rgb(PANEL))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_shrink_0()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .child(request.title.clone()),
            );
        // Only the request body scrolls. Keep its title and native decision actions
        // reachable together, including when the composer shares a short window.
        let mut details = div()
            .id("interaction-body")
            .w_full()
            .min_h(px(0.))
            .max_h((max_height - px(76.)).max(px(40.)))
            .overflow_y_scroll()
            .track_scroll(&interaction_scroll)
            .px_3()
            .py_1()
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(13.))
            .line_height(px(20.));
        let mut actions = row().flex_shrink_0().flex_wrap().gap_1().px_2().py_1();
        match &request.kind {
            InteractionKind::Questions(questions) => {
                for (index, question) in questions.iter().enumerate() {
                    details = details.child(div().child(question.prompt.clone()));
                    let mut options = row().gap_1().flex_wrap();
                    for option in &question.options {
                        let selected = answers
                            .get(&question.id)
                            .is_some_and(|a| a.contains(option));
                        let question_id = question.id.clone();
                        let option = option.clone();
                        let multiple = question.multiple;
                        options = options.child(
                            compact_button(
                                element_id("question-option", format!("{index}:{option}")),
                                option.clone(),
                            )
                            .border_1()
                            .border_color(if selected {
                                rgb(ACCENT)
                            } else {
                                quiet_border()
                            })
                            .when(selected, |b| b.bg(rgb(PANEL_HOVER)))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(pending) =
                                        this.states.get_mut(&id).and_then(|s| s.pending.as_mut())
                                    {
                                        let answers =
                                            pending.answers.entry(question_id.clone()).or_default();
                                        if multiple {
                                            if let Some(i) =
                                                answers.iter().position(|a| *a == option)
                                            {
                                                answers.remove(i);
                                            } else {
                                                answers.push(option.clone());
                                            }
                                        } else {
                                            *answers = vec![option.clone()];
                                        }
                                    }
                                    cx.notify();
                                },
                            )),
                        );
                    }
                    details = details.child(options).child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(quiet_border())
                            .bg(rgb(BACKGROUND))
                            .child(editors[index].clone()),
                    );
                }
                actions = actions.child(
                    compact_button("submit-answers", "Continue")
                        .bg(rgb(PANEL_HOVER))
                        .on_click(cx.listener(move |this, _, _, cx| this.respond(id, true, cx))),
                );
            }
            InteractionKind::Approval(presentation) => {
                let arguments_index =
                    presentation.fields.len() + usize::from(presentation.explanation.is_some());
                for field in &presentation.fields {
                    let selectable_command =
                        presentation.action == ApprovalAction::Command && field.label == "Command";
                    let command = field.value.clone();
                    let command_view = selectable_command.then(|| {
                        self.text_view(
                            id,
                            (submitted_user, format!("approval-command:{}", request.id)),
                            command,
                            vec![],
                            vec![],
                            cx,
                        )
                    });
                    details = details.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .line_height(px(16.))
                                    .text_color(rgb(MUTED))
                                    .child(field.label.clone()),
                            )
                            .child(
                                div()
                                    .when(field.monospace, |d| d.font_family("Menlo"))
                                    .when(selectable_command, |d| {
                                        d.px_2().py_1().rounded_md().bg(rgb(BACKGROUND))
                                    })
                                    .child(if let Some(text) = command_view {
                                        text.into_any_element()
                                    } else {
                                        div().child(field.value.clone()).into_any_element()
                                    }),
                            ),
                    );
                }
                if let Some(explanation) = &presentation.explanation {
                    details =
                        details.child(div().text_color(rgb(MUTED)).child(explanation.clone()));
                }
                actions = actions
                    .child(
                        compact_button("approve-request", presentation.allow_label.clone())
                            .bg(rgb(PANEL_HOVER))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.respond(id, true, cx)),
                            ),
                    )
                    .child(
                        compact_button("decline-request", "Decline")
                            .text_color(rgb(MUTED))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.respond(id, false, cx)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        compact_button("approval-details", "")
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(
                                icon(if expanded { "chevron-down" } else { "chevron" })
                                    .size(px(12.)),
                            )
                            .child(if expanded { "Hide details" } else { "Details" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let reveal = if let Some(pending) =
                                    this.states.get_mut(&id).and_then(|s| s.pending.as_mut())
                                {
                                    pending.details = !pending.details;
                                    pending.details
                                } else {
                                    false
                                };
                                if reveal && let Some(state) = this.states.get(&id) {
                                    state
                                        .interaction_scroll
                                        .scroll_to_top_of_item(arguments_index);
                                }
                                cx.notify();
                            })),
                    );
                if expanded {
                    let arguments = presentation.raw_arguments.clone();
                    let arguments_view = self.text_view(
                        id,
                        (submitted_user, format!("approval-arguments:{}", request.id)),
                        arguments,
                        vec![],
                        vec![],
                        cx,
                    );
                    details = details.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(MUTED))
                                    .child("Original arguments"),
                            )
                            .child(
                                div()
                                    .p_2()
                                    .rounded_md()
                                    .bg(rgb(BACKGROUND))
                                    .font_family("Menlo")
                                    .text_size(px(12.))
                                    .line_height(px(18.))
                                    .child(arguments_view),
                            ),
                    );
                }
            }
            InteractionKind::LeavePlan { detail } => {
                details = details.child(div().child(detail.clone()));
                actions = actions
                    .child(
                        compact_button("leave-plan", "Leave Plan mode")
                            .bg(rgb(PANEL_HOVER))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.respond(id, true, cx)),
                            ),
                    )
                    .child(
                        compact_button("keep-planning", "Keep planning")
                            .text_color(rgb(MUTED))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.respond(id, false, cx)),
                            ),
                    );
            }
        }
        Some(card.child(details).child(actions).into_any_element())
    }
}
