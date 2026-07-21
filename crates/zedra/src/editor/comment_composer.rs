//! CommentComposer - floating multi-line input for the diff-view "Comment"
//! selection action. Presented via `platform_bridge::show_custom_sheet`.

use gpui::*;

use crate::theme;
use crate::ui::input::Input;
use crate::ui::{InputChanged, InputSubmit};
use crate::workspace_editor::EditorSelection;

#[derive(Clone, Debug)]
pub enum CommentComposerEvent {
    /// "Comment" was pressed — mark this comment pending, don't send yet.
    SavePending { text: String },
    /// "Submit" was pressed — send this single comment now.
    SubmitNow { text: String },
}

pub struct CommentComposer {
    selection: EditorSelection,
    input: Entity<Input>,
    _subscriptions: Vec<Subscription>,
}

impl CommentComposer {
    pub fn new(selection: EditorSelection, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            Input::new(cx)
                .placeholder("Add a comment…")
                .multiline(true)
                .native_suggestions(true)
                .max_lines(8)
        });
        let mut subscriptions = Vec::new();
        // `Input` owns the text; just re-render so `can_send`'s opacity tracks it.
        subscriptions.push(cx.subscribe(
            &input,
            |_this: &mut Self, _input, _event: &InputChanged, cx| {
                cx.notify();
            },
        ));
        // Submitting from the keyboard behaves like tapping "Comment" — it
        // marks the comment pending rather than sending immediately, since
        // sending needs an explicit target-agent pick.
        subscriptions.push(cx.subscribe(
            &input,
            |this: &mut Self, _input, _event: &InputSubmit, cx| {
                this.emit_save_pending(cx);
            },
        ));

        Self {
            selection,
            input,
            _subscriptions: subscriptions,
        }
    }

    fn trimmed_text(&self, cx: &App) -> String {
        self.input.read(cx).get_value().trim().to_string()
    }

    fn can_send(&self, cx: &App) -> bool {
        !self.trimmed_text(cx).is_empty()
    }

    fn emit_save_pending(&mut self, cx: &mut Context<Self>) {
        let text = self.trimmed_text(cx);
        if text.is_empty() {
            return;
        }
        cx.emit(CommentComposerEvent::SavePending { text });
    }

    fn emit_submit_now(&mut self, cx: &mut Context<Self>) {
        let text = self.trimmed_text(cx);
        if text.is_empty() {
            return;
        }
        cx.emit(CommentComposerEvent::SubmitNow { text });
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = if self.selection.start == self.selection.end {
            format!("{}:L{}", self.selection.path, self.selection.start)
        } else {
            format!(
                "{}:L{}-L{}",
                self.selection.path, self.selection.start, self.selection.end
            )
        };
        div()
            .w_full()
            .px(px(theme::DRAWER_PADDING))
            .pt(px(theme::SPACING_SM))
            .text_size(px(theme::FONT_DETAIL))
            .text_color(rgb(theme::text_muted(cx)))
            .truncate()
            .child(label)
    }

    fn render_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_send = self.can_send(cx);
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(theme::SPACING_SM))
            .px(px(theme::DRAWER_PADDING))
            .pb(px(theme::SPACING_SM))
            .child(
                div()
                    .id("comment-composer-comment")
                    .px(px(theme::SPACING_MD))
                    .py(px(6.0))
                    .opacity(if can_send { 1.0 } else { 0.35 })
                    .cursor_pointer()
                    .text_color(rgb(theme::text_secondary(cx)))
                    .child("Comment")
                    .on_press(cx.listener(|this, _, _, cx| {
                        this.emit_save_pending(cx);
                    })),
            )
            .child(
                div()
                    .id("comment-composer-submit")
                    .px(px(theme::SPACING_MD))
                    .py(px(6.0))
                    .opacity(if can_send { 1.0 } else { 0.35 })
                    .cursor_pointer()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::accent_blue(cx)))
                    .child("Submit")
                    .on_press(cx.listener(|this, _, _, cx| {
                        this.emit_submit_now(cx);
                    })),
            )
    }
}

impl EventEmitter<CommentComposerEvent> for CommentComposer {}

impl Render for CommentComposer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme::bg_primary(cx)))
            .child(self.render_header(cx))
            .child(
                div()
                    .w_full()
                    .px(px(theme::DRAWER_PADDING))
                    .pt(px(theme::SPACING_SM))
                    .child(self.input.clone()),
            )
            .child(self.render_actions(cx))
    }
}
