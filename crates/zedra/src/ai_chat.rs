// AI Chat view: Claude Code integration for Zedra.
//
// Provides a chat-style interface to interact with Claude Code running
// on the desktop host. Sends prompts via RPC, displays streamed responses.

use gpui::prelude::FluentBuilder;
use gpui::*;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub struct AiPromptSubmitted {
    pub prompt: String,
    pub context: Option<String>,
}

// ---------------------------------------------------------------------------
// AiChatView
// ---------------------------------------------------------------------------

pub struct AiChatView {
    messages: Vec<ChatMessage>,
    input_text: String,
    is_loading: bool,
    focus_handle: FocusHandle,
}

impl EventEmitter<AiPromptSubmitted> for AiChatView {}

impl AiChatView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            messages: vec![ChatMessage {
                role: Role::Assistant,
                text: "Welcome to Zedra Code. I can help you with your codebase. \
                       Ask me anything or give me a task."
                    .into(),
            }],
            input_text: String::new(),
            is_loading: false,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn push_user_message(&mut self, text: String, cx: &mut Context<Self>) {
        self.messages.push(ChatMessage {
            role: Role::User,
            text,
        });
        self.is_loading = true;
        cx.notify();
    }

    pub fn push_assistant_message(&mut self, text: String, cx: &mut Context<Self>) {
        self.messages.push(ChatMessage {
            role: Role::Assistant,
            text,
        });
        self.is_loading = false;
        cx.notify();
    }

    pub fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        self.is_loading = loading;
        cx.notify();
    }

    fn submit_prompt(&mut self, cx: &mut Context<Self>) {
        let text = self.input_text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let prompt = text.clone();
        self.push_user_message(text, cx);
        self.input_text.clear();
        cx.emit(AiPromptSubmitted {
            prompt,
            context: None,
        });
    }
}

impl Focusable for AiChatView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AiChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut messages_list = div().flex().flex_col().gap_3().p_4();

        for (i, msg) in self.messages.iter().enumerate() {
            let (bg, text_color, align) = match msg.role {
                Role::User => (rgb(0x61afef), rgb(0x1e1e1e), "end"),
                Role::Assistant => (rgb(0x282c34), rgb(0xabb2bf), "start"),
            };

            let bubble = div()
                .max_w(rems(20.0))
                .px_3()
                .py_2()
                .bg(bg)
                .rounded(px(12.0))
                .text_color(text_color)
                .text_sm()
                .child(msg.text.clone());

            let row = div()
                .id(ElementId::Name(format!("msg-{}", i).into()))
                .flex()
                .flex_row();

            let row = if align == "end" {
                row.justify_end().child(bubble)
            } else {
                row.justify_start().child(bubble)
            };

            messages_list = messages_list.child(row);
        }

        // Loading indicator
        if self.is_loading {
            messages_list = messages_list.child(
                div()
                    .flex()
                    .flex_row()
                    .justify_start()
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .bg(rgb(0x282c34))
                            .rounded(px(12.0))
                            .text_color(rgb(0x5c6370))
                            .text_sm()
                            .child("Thinking..."),
                    ),
            );
        }

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e1e))
            // Messages area
            .child(
                div()
                    .id("ai-chat-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(messages_list),
            )
            // Input area
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(0x3e4451))
                    .child(
                        div()
                            .flex_1()
                            .px_3()
                            .py_2()
                            .bg(rgb(0x282c34))
                            .rounded(px(8.0))
                            .text_color(if self.input_text.is_empty() {
                                rgb(0x5c6370)
                            } else {
                                rgb(0xabb2bf)
                            })
                            .text_sm()
                            .child(if self.input_text.is_empty() {
                                "Ask Claude Code...".to_string()
                            } else {
                                self.input_text.clone()
                            }),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .bg(if self.is_loading {
                                rgb(0x5c6370)
                            } else {
                                rgb(0x61afef)
                            })
                            .rounded(px(8.0))
                            .text_color(rgb(0x1e1e1e))
                            .text_sm()
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _event, _window, cx| {
                                    if !this.is_loading {
                                        this.submit_prompt(cx);
                                    }
                                }),
                            )
                            .child("Send"),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_roles() {
        let user = ChatMessage {
            role: Role::User,
            text: "hello".into(),
        };
        let assistant = ChatMessage {
            role: Role::Assistant,
            text: "hi".into(),
        };
        assert_eq!(user.role, Role::User);
        assert_eq!(assistant.role, Role::Assistant);
    }

    #[test]
    fn ai_prompt_submitted_debug() {
        let event = AiPromptSubmitted {
            prompt: "fix the bug".into(),
            context: Some("src/lib.rs".into()),
        };
        let dbg = format!("{:?}", event);
        assert!(dbg.contains("fix the bug"));
        assert!(dbg.contains("src/lib.rs"));
    }

    #[test]
    fn default_welcome_message() {
        // Verify the welcome message structure
        let msg = ChatMessage {
            role: Role::Assistant,
            text: "Welcome to Zedra Code.".into(),
        };
        assert_eq!(msg.role, Role::Assistant);
        assert!(msg.text.starts_with("Welcome"));
    }
}
