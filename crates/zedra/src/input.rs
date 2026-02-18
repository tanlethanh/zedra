/// Text Input component for Android with soft keyboard support
///
/// Provides a focusable text input field that:
/// - Shows/hides the Android soft keyboard on focus/blur
/// - Displays a blinking cursor when focused
/// - Handles keyboard input to update text content
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::android_jni;
use crate::theme;

/// Event emitted when the input value changes
#[derive(Clone, Debug)]
pub struct InputChanged {
    pub value: String,
}

/// Text input component with Android keyboard integration
pub struct Input {
    /// Current text value
    value: String,
    /// Placeholder text shown when empty
    placeholder: String,
    /// Whether to obscure text (for passwords)
    secure: bool,
    /// Focus handle for GPUI focus management
    focus_handle: FocusHandle,
    /// Last time a key was pressed (for cursor blink pause)
    last_keystroke: Option<Instant>,
}

impl Input {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            value: String::new(),
            placeholder: String::new(),
            secure: false,
            focus_handle: cx.focus_handle(),
            last_keystroke: None,
        }
    }

    /// Set the placeholder text
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Set as a secure/password input
    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    /// Set the initial value
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    /// Get current value
    pub fn get_value(&self) -> &str {
        &self.value
    }

    /// Set current value
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
    }

    fn handle_click(
        &mut self,
        _event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        log::info!("Input clicked - focusing and requesting keyboard");
        // Focus this element
        self.focus_handle.focus(window, cx);
        // Request keyboard
        android_jni::show_keyboard();
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke.key;
        log::debug!("Input key down: {:?}", event.keystroke);

        // Record keystroke time to pause cursor blinking
        self.last_keystroke = Some(Instant::now());

        match key.as_str() {
            "backspace" => {
                if !self.value.is_empty() {
                    self.value.pop();
                    cx.emit(InputChanged {
                        value: self.value.clone(),
                    });
                    cx.notify();
                }
            }
            "enter" => {
                // Could emit a submit event here - for now, just ignore
            }
            _ => {
                // Handle character input
                if let Some(ch) = &event.keystroke.key_char {
                    self.value.push_str(ch);
                    cx.emit(InputChanged {
                        value: self.value.clone(),
                    });
                    cx.notify();
                }
            }
        }
    }

    /// Render the cursor element (solid while typing, blinking otherwise)
    fn render_cursor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cursor_id = ("cursor", cx.entity_id());

        // Keep cursor solid for 500ms after last keystroke
        let recently_typed = self
            .last_keystroke
            .map(|t| t.elapsed() < Duration::from_millis(500))
            .unwrap_or(false);

        div()
            .w(px(2.0))
            .h(px(18.0))
            .bg(rgb(theme::TEXT_SECONDARY))
            .rounded(px(1.0))
            .with_animation(
                cursor_id,
                Animation::new(Duration::from_millis(1000)).repeat(),
                move |cursor, delta| {
                    // Solid while typing, blinking otherwise
                    let opacity = if recently_typed {
                        1.0
                    } else if delta < 0.5 {
                        1.0
                    } else {
                        0.0
                    };
                    cursor.opacity(opacity)
                },
            )
    }
}

impl EventEmitter<InputChanged> for Input {}

impl Focusable for Input {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Input {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_focused = self.focus_handle.is_focused(window);

        // Determine what text to display
        let display_value = if self.secure && !self.value.is_empty() {
            "\u{2022}".repeat(self.value.len()) // Bullet points for password
        } else {
            self.value.clone()
        };

        // Show placeholder only when unfocused and empty
        let show_placeholder = self.value.is_empty() && !is_focused;

        let text_color = if show_placeholder {
            rgb(theme::TEXT_MUTED)
        } else {
            rgb(theme::TEXT_SECONDARY)
        };

        let border_color = if is_focused {
            rgb(theme::ACCENT_BLUE)
        } else {
            rgb(theme::BORDER_DEFAULT)
        };

        // Build display text
        let display_text = if show_placeholder {
            self.placeholder.clone()
        } else {
            display_value
        };

        div()
            .id(("input", cx.entity_id()))
            .track_focus(&self.focus_handle)
            .on_click(cx.listener(Self::handle_click))
            .on_key_down(cx.listener(Self::handle_key_down))
            .flex()
            .flex_row()
            .items_center()
            .px_3()
            .py_2()
            .w_full()
            .min_h(px(40.0))
            .bg(rgb(theme::BG_SURFACE))
            .rounded(px(6.0))
            .border_1()
            .border_color(border_color)
            .text_color(text_color)
            .cursor_text()
            .child(display_text)
            .when(is_focused, |this| this.child(self.render_cursor(cx)))
    }
}
