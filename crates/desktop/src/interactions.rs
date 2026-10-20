use crate::{
    shell::{ChatsView, element_id},
    theme::*,
};
use gpui::{AnyElement, Context, IntoElement, div, prelude::*, px, rgb};
use smaller_talk_domain::InteractionKind;
use uuid::Uuid;

impl ChatsView {
    pub(crate) fn render_interaction(
        &mut self,
        id: Uuid,
        max_height: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = self.states.get(&id)?;
        let pending = state.pending.as_ref()?;
        let mut card = div()
            .id(element_id(
                "provider-interaction",
                format!(
                    "{id}:{}:{}",
                    state.submitted_user.unwrap_or_default(),
                    pending.request.id
                ),
            ))
            .w_full()
            .max_w(px(800.))
            .max_h(max_height)
            .overflow_y_scroll()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(PANEL))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_sm()
                    .child(pending.request.title.clone()),
            );
        match &pending.request.kind {
            InteractionKind::Questions(questions) => {
                for (index, question) in questions.iter().enumerate() {
                    card = card.child(div().text_sm().child(question.prompt.clone()));
                    let mut options = row().flex_wrap();
                    for option in &question.options {
                        let selected = pending
                            .answers
                            .get(&question.id)
                            .is_some_and(|a| a.contains(option));
                        let question_id = question.id.clone();
                        let option = option.clone();
                        let multiple = question.multiple;
                        options = options.child(
                            button(
                                element_id("question-option", format!("{index}:{option}")),
                                option.clone(),
                            )
                            .border_1()
                            .border_color(rgb(if selected { ACCENT } else { BORDER }))
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
                    card = card.child(options).child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(BORDER))
                            .child(pending.editors[index].clone()),
                    );
                }
                card = card.child(
                    button("submit-answers", "Continue")
                        .bg(rgb(PANEL_HOVER))
                        .on_click(cx.listener(move |this, _, _, cx| this.respond(id, true, cx))),
                );
            }
            InteractionKind::Approval(presentation) => {
                for field in &presentation.fields {
                    card = card.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(field.label.clone()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .when(field.monospace, |d| d.font_family("Menlo"))
                                    .child(field.value.clone()),
                            ),
                    );
                }
                if let Some(explanation) = &presentation.explanation {
                    card = card.child(div().text_sm().child(explanation.clone()));
                }
                let expanded = pending.details;
                card = card.child(
                    row()
                        .child(
                            button("approve-request", presentation.allow_label.clone())
                                .bg(rgb(PANEL_HOVER))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.respond(id, true, cx)),
                                ),
                        )
                        .child(button("decline-request", "Decline").on_click(
                            cx.listener(move |this, _, _, cx| this.respond(id, false, cx)),
                        ))
                        .child(div().flex_1())
                        .child(
                            button(
                                "approval-details",
                                if expanded { "Hide details" } else { "Details" },
                            )
                            .text_xs()
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(pending) =
                                        this.states.get_mut(&id).and_then(|s| s.pending.as_mut())
                                    {
                                        pending.details = !pending.details;
                                    }
                                    cx.notify();
                                },
                            )),
                        ),
                );
                if expanded {
                    card = card.child(
                        div()
                            .id("approval-json")
                            .max_h(px(160.))
                            .overflow_y_scroll()
                            .p_2()
                            .rounded_md()
                            .bg(rgb(BACKGROUND))
                            .font_family("Menlo")
                            .text_xs()
                            .child(presentation.raw_arguments.clone()),
                    );
                }
            }
            InteractionKind::LeavePlan { detail } => {
                card = card.child(div().text_sm().child(detail.clone())).child(
                    row()
                        .child(
                            button("leave-plan", "Leave Plan mode")
                                .bg(rgb(PANEL_HOVER))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.respond(id, true, cx)),
                                ),
                        )
                        .child(button("keep-planning", "Keep planning").on_click(
                            cx.listener(move |this, _, _, cx| this.respond(id, false, cx)),
                        )),
                );
            }
        }
        Some(card.into_any_element())
    }
}
