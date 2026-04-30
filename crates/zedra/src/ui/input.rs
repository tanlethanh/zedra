/// Text Input component for Android with soft keyboard support
///
/// Provides a focusable text input field that:
/// - Shows/hides the Android soft keyboard on focus/blur
/// - Displays a blinking cursor when focused
/// - Handles keyboard input to update text content
use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::theme;

fn clamp_byte_index(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Multiline wrapped text with a caret at `cursor_byte`, using the same layout as `SharedString`.
struct MultilineInputText {
    text: SharedString,
    cursor_byte: usize,
    draw_caret: bool,
}

impl Element for MultilineInputText {
    type RequestLayoutState = TextLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let state = TextLayout::default();
        let layout_id = state.uniform_request_layout(self.text.clone(), window, cx);
        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        text_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        text_layout.uniform_prepaint(bounds, self.text.as_ref());
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        text_layout: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        text_layout.uniform_paint(self.text.as_ref(), window, cx);
        if !self.draw_caret {
            return;
        }
        let ix = clamp_byte_index(self.text.as_ref(), self.cursor_byte);
        let origin = if self.text.is_empty() {
            bounds.origin
        } else {
            text_layout.position_for_index(ix).unwrap_or(bounds.origin)
        };
        let line_height = if self.text.is_empty() {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            style
                .line_height
                .to_pixels(font_size.into(), window.rem_size())
        } else {
            text_layout.line_height()
        };
        let caret = fill(
            Bounds::new(origin, size(px(2.0), line_height)),
            rgb(theme::TEXT_SECONDARY),
        );
        window.paint_quad(caret);
    }
}

impl IntoElement for MultilineInputText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct ImeInputHandlerElement {
    input: Entity<Input>,
}

impl IntoElement for ImeInputHandlerElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ImeInputHandlerElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
    }
}

/// Event emitted when the input value changes
#[derive(Clone, Debug)]
pub struct InputChanged {
    pub value: String,
}

/// Event emitted when the user presses Enter/Return to submit the input
#[derive(Clone, Debug)]
pub struct InputSubmit {
    pub value: String,
}

/// Text input component with Android keyboard integration
pub struct Input {
    /// Current text value
    value: String,
    /// Cached display value — bullet-masked for secure inputs; updated on every `value` change.
    display_value: String,
    /// Placeholder text shown when empty
    placeholder: String,
    /// Whether to obscure text (for passwords)
    secure: bool,
    /// Whether to ask the platform keyboard for native inline suggestions.
    native_suggestions: bool,
    /// Focus handle for GPUI focus management
    focus_handle: FocusHandle,
    /// Last time a key was pressed (for cursor blink pause)
    last_keystroke: Option<Instant>,
    /// Compact mode for denser tool-style inputs.
    compact: bool,
    /// Extra space reserved on the right inside the field (e.g. trailing icon button).
    trailing_gutter: f32,
    /// When true, text wraps and Enter inserts a newline instead of submitting.
    multiline: bool,
    /// When set (multiline only), inner text area scrolls after this many lines of height.
    max_lines: Option<usize>,
    /// UTF-8 byte index of the caret (multiline only).
    cursor_byte: usize,
    /// UTF-8 byte range selected by the native text system.
    selected_range: Option<Range<usize>>,
    /// UTF-8 byte range for native IME marked/preedit text.
    marked_range: Option<Range<usize>>,
    /// True while UIKit is streaming native dictation hypotheses into the input.
    dictation_active: bool,
    /// UTF-8 byte range UIKit may delete after committing a dictation hypothesis.
    committed_dictation_cleanup_range: Option<Range<usize>>,
}

impl Input {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            value: String::new(),
            display_value: String::new(),
            placeholder: String::new(),
            secure: false,
            native_suggestions: false,
            focus_handle: cx.focus_handle(),
            last_keystroke: None,
            compact: false,
            trailing_gutter: 0.0,
            multiline: false,
            max_lines: None,
            cursor_byte: 0,
            selected_range: None,
            marked_range: None,
            dictation_active: false,
            committed_dictation_cleanup_range: None,
        }
    }

    /// Recompute `display_value` from the current `value` and `secure` flag.
    fn refresh_display_value(&mut self) {
        self.display_value = if self.secure && !self.value.is_empty() {
            "\u{2022}".repeat(self.value.len())
        } else {
            self.value.clone()
        };
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

    /// Enable native platform keyboard suggestions when supported.
    pub fn native_suggestions(mut self, native_suggestions: bool) -> Self {
        self.native_suggestions = native_suggestions;
        self
    }

    fn text_input_traits_policy(native_suggestions: bool, secure: bool) -> PlatformTextInputTraits {
        if native_suggestions && !secure {
            PlatformTextInputTraits::keyboard_suggestions()
        } else {
            PlatformTextInputTraits::default()
        }
    }

    /// Use a smaller, denser layout for compact toolbars and sidebars.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// Reserve horizontal space on the right for a control overlaid inside the field.
    pub fn trailing_gutter(mut self, px: f32) -> Self {
        self.trailing_gutter = px;
        self
    }

    pub fn multiline(mut self, multiline: bool) -> Self {
        self.multiline = multiline;
        self
    }

    /// Cap vertical growth (scrolls inside). Only applies with `multiline(true)`.
    pub fn max_lines(mut self, lines: usize) -> Self {
        self.max_lines = Some(lines);
        self
    }

    /// Set the initial value
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self.refresh_display_value();
        self.cursor_byte = self.value.len();
        self.selected_range = None;
        self.marked_range = None;
        self.dictation_active = false;
        self.committed_dictation_cleanup_range = None;
        self
    }

    /// Get current value
    pub fn get_value(&self) -> &str {
        &self.value
    }

    /// Set current value
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.refresh_display_value();
        self.cursor_byte = self.value.len();
        self.selected_range = None;
        self.marked_range = None;
        self.dictation_active = false;
        self.committed_dictation_cleanup_range = None;
    }

    fn cursor_byte(&self) -> usize {
        clamp_byte_index(&self.value, self.cursor_byte)
    }

    fn byte_offset_from_utf16(text: &str, utf16_offset: usize) -> usize {
        let mut utf16_count = 0;
        for (byte_idx, ch) in text.char_indices() {
            if utf16_count >= utf16_offset {
                return byte_idx;
            }
            utf16_count += ch.len_utf16();
            if utf16_count > utf16_offset {
                return byte_idx;
            }
        }
        text.len()
    }

    fn utf16_to_byte_offset(&self, utf16_offset: usize) -> usize {
        Self::byte_offset_from_utf16(&self.value, utf16_offset)
    }

    fn utf16_range_to_byte_range(&self, range: &Range<usize>) -> Range<usize> {
        self.utf16_to_byte_offset(range.start)..self.utf16_to_byte_offset(range.end)
    }

    fn byte_to_utf16_offset_in(text: &str, byte_offset: usize) -> usize {
        text[..clamp_byte_index(text, byte_offset)]
            .encode_utf16()
            .count()
    }

    fn byte_to_utf16_offset(&self, byte_offset: usize) -> usize {
        Self::byte_to_utf16_offset_in(&self.value, byte_offset)
    }

    fn byte_range_to_utf16_range(&self, range: &Range<usize>) -> Range<usize> {
        self.byte_to_utf16_offset(range.start)..self.byte_to_utf16_offset(range.end)
    }

    fn active_replacement_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        let marked_range = (!self.has_committed_dictation_pending_cleanup()
            || self.dictation_active)
            .then(|| self.marked_range.clone())
            .flatten();
        range_utf16
            .as_ref()
            .map(|range| self.utf16_range_to_byte_range(range))
            .or(marked_range)
            .or(self.selected_range.clone())
            .unwrap_or_else(|| {
                let cursor = self.cursor_byte();
                cursor..cursor
            })
    }

    fn set_cursor_from_utf16_range(&mut self, range_utf16: Range<usize>) {
        let range = self.utf16_range_to_byte_range(&range_utf16);
        self.selected_range = Some(range.clone());
        self.cursor_byte = range.end;
    }

    fn has_committed_dictation_pending_cleanup(&self) -> bool {
        self.committed_dictation_cleanup_range.is_some()
    }

    fn clear_committed_dictation_cleanup(&mut self) {
        self.committed_dictation_cleanup_range = None;
        if !self.dictation_active {
            self.marked_range = None;
        }
    }

    fn replace_range_with_text(
        &mut self,
        replacement_range: Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let start = clamp_byte_index(&self.value, replacement_range.start);
        let end = clamp_byte_index(&self.value, replacement_range.end.max(start));
        self.value.replace_range(start..end, text);
        self.cursor_byte = start + text.len();
        self.selected_range = None;
        self.marked_range = None;
        self.dictation_active = false;
        self.committed_dictation_cleanup_range = None;
        self.refresh_display_value();
        self.last_keystroke = Some(Instant::now());
        cx.emit(InputChanged {
            value: self.value.clone(),
        });
        cx.notify();
    }

    fn replace_range_with_marked_text(
        &mut self,
        replacement_range: Range<usize>,
        text: &str,
        selected_range_utf16: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        let start = clamp_byte_index(&self.value, replacement_range.start);
        let end = clamp_byte_index(&self.value, replacement_range.end.max(start));
        self.value.replace_range(start..end, text);
        let marked_end = start + text.len();
        self.marked_range = (!text.is_empty()).then_some(start..marked_end);
        self.committed_dictation_cleanup_range = None;
        let selected_range = selected_range_utf16.map(|range| {
            let selected_start =
                (start + Self::byte_offset_from_utf16(text, range.start)).min(marked_end);
            let selected_end =
                (start + Self::byte_offset_from_utf16(text, range.end)).min(marked_end);
            selected_start.min(selected_end)..selected_start.max(selected_end)
        });
        self.cursor_byte = selected_range
            .as_ref()
            .map_or(marked_end, |range| range.end);
        self.selected_range = selected_range;
        self.refresh_display_value();
        self.last_keystroke = Some(Instant::now());
        cx.emit(InputChanged {
            value: self.value.clone(),
        });
        cx.notify();
    }

    fn begin_dictation(&mut self, cx: &mut Context<Self>) {
        self.dictation_active = true;
        self.committed_dictation_cleanup_range = None;
        if self.marked_range.is_none() {
            let cursor = self.cursor_byte();
            self.marked_range = Some(cursor..cursor);
            self.selected_range = Some(cursor..cursor);
        }
        cx.notify();
    }

    fn insert_live_dictation_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.dictation_active {
            self.begin_dictation(cx);
        }
        let range = self.active_replacement_range(None);
        let selected = text.encode_utf16().count();
        self.replace_range_with_marked_text(range, text, Some(selected..selected), cx);
    }

    fn finish_dictation(&mut self, cx: &mut Context<Self>) {
        if !self.dictation_active {
            return;
        }

        self.dictation_active = false;
        // Critical: UIKit deletes its committed hypothesis after dictation ends.
        // Preserve the visible value and consume that synthetic delete later.
        self.committed_dictation_cleanup_range = self.marked_range.clone();
        cx.notify();
    }

    fn cancel_dictation(&mut self, cx: &mut Context<Self>) {
        self.dictation_active = false;
        self.committed_dictation_cleanup_range = None;
        if let Some(range) = self.marked_range.take() {
            self.replace_range_with_text(range, "", cx);
        } else {
            cx.notify();
        }
    }

    fn consume_or_reconcile_dictation_cleanup(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(cleanup_range) = self.committed_dictation_cleanup_range.clone() else {
            return false;
        };
        let Some(native_range) = range_utf16
            .as_ref()
            .map(|range| self.utf16_range_to_byte_range(range))
        else {
            self.clear_committed_dictation_cleanup();
            return false;
        };
        let covers_cleanup =
            native_range.start <= cleanup_range.start && native_range.end >= cleanup_range.end;

        if text.is_empty() {
            if covers_cleanup {
                self.clear_committed_dictation_cleanup();
                self.selected_range = Some(cleanup_range.end..cleanup_range.end);
                self.cursor_byte = cleanup_range.end.min(self.value.len());
                cx.notify();
                return true;
            }

            self.clear_committed_dictation_cleanup();
            return false;
        }

        if covers_cleanup {
            let start = clamp_byte_index(&self.value, cleanup_range.start);
            let end = clamp_byte_index(&self.value, cleanup_range.end.max(start));
            self.value.replace_range(start..end, text);
            let inserted_range = start..start + text.len();
            self.cursor_byte = inserted_range.end;
            self.selected_range = Some(inserted_range.end..inserted_range.end);
            self.marked_range = Some(inserted_range.clone());
            self.committed_dictation_cleanup_range = Some(inserted_range);
            self.refresh_display_value();
            self.last_keystroke = Some(Instant::now());
            cx.emit(InputChanged {
                value: self.value.clone(),
            });
            cx.notify();
            return true;
        }

        self.clear_committed_dictation_cleanup();
        false
    }

    fn handle_press(&mut self, _event: &PressEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        window.show_soft_keyboard();
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke.key;
        tracing::debug!("Input key down: {:?}", event.keystroke);

        // Record keystroke time to pause cursor blinking
        self.last_keystroke = Some(Instant::now());

        match key.as_str() {
            "backspace" => {
                if self.multiline {
                    let i = clamp_byte_index(&self.value, self.cursor_byte);
                    if i == 0 {
                        return;
                    }
                    let prev = self.value[..i].chars().next_back().unwrap();
                    let start = i - prev.len_utf8();
                    self.value.replace_range(start..i, "");
                    self.cursor_byte = start;
                    self.selected_range = None;
                    self.marked_range = None;
                    self.dictation_active = false;
                    self.committed_dictation_cleanup_range = None;
                    self.refresh_display_value();
                    cx.emit(InputChanged {
                        value: self.value.clone(),
                    });
                    cx.notify();
                } else if !self.value.is_empty() {
                    self.value.pop();
                    self.selected_range = None;
                    self.marked_range = None;
                    self.dictation_active = false;
                    self.committed_dictation_cleanup_range = None;
                    self.refresh_display_value();
                    cx.emit(InputChanged {
                        value: self.value.clone(),
                    });
                    cx.notify();
                }
            }
            "enter" => {
                if self.multiline {
                    let i = clamp_byte_index(&self.value, self.cursor_byte);
                    self.value.insert_str(i, "\n");
                    self.cursor_byte = i + 1;
                    self.selected_range = None;
                    self.marked_range = None;
                    self.dictation_active = false;
                    self.committed_dictation_cleanup_range = None;
                    self.refresh_display_value();
                    cx.emit(InputChanged {
                        value: self.value.clone(),
                    });
                    cx.notify();
                } else {
                    cx.emit(InputSubmit {
                        value: self.value.clone(),
                    });
                }
            }
            "left" => {
                if !self.multiline {
                    return;
                }
                let i = clamp_byte_index(&self.value, self.cursor_byte);
                if i == 0 {
                    return;
                }
                let prev = self.value[..i].chars().next_back().unwrap();
                self.cursor_byte = i - prev.len_utf8();
                cx.notify();
            }
            "right" => {
                if !self.multiline {
                    return;
                }
                let i = clamp_byte_index(&self.value, self.cursor_byte);
                if i >= self.value.len() {
                    return;
                }
                let c = self.value[i..].chars().next().unwrap();
                self.cursor_byte = i + c.len_utf8();
                cx.notify();
            }
            _ => {
                if let Some(ch) = &event.keystroke.key_char {
                    if self.multiline {
                        let i = clamp_byte_index(&self.value, self.cursor_byte);
                        self.value.insert_str(i, ch);
                        self.cursor_byte = i + ch.len();
                        self.selected_range = None;
                        self.marked_range = None;
                        self.dictation_active = false;
                        self.committed_dictation_cleanup_range = None;
                        self.refresh_display_value();
                        cx.emit(InputChanged {
                            value: self.value.clone(),
                        });
                        cx.notify();
                    } else {
                        self.value.push_str(ch);
                        self.selected_range = None;
                        self.marked_range = None;
                        self.dictation_active = false;
                        self.committed_dictation_cleanup_range = None;
                        self.refresh_display_value();
                        cx.emit(InputChanged {
                            value: self.value.clone(),
                        });
                        cx.notify();
                    }
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
            .h(px(if self.compact { 14.0 } else { 18.0 }))
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
impl EventEmitter<InputSubmit> for Input {}

impl EntityInputHandler for Input {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let start = self.utf16_to_byte_offset(range_utf16.start);
        let end = self.utf16_to_byte_offset(range_utf16.end);
        *adjusted_range = Some(self.byte_to_utf16_offset(start)..self.byte_to_utf16_offset(end));
        Some(self.value[start..end].to_string())
    }

    fn text_len_utf16(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        Some(self.value.encode_utf16().count())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = self
            .selected_range
            .as_ref()
            .map(|range| self.byte_range_to_utf16_range(range))
            .unwrap_or_else(|| {
                let cursor = self.byte_to_utf16_offset(self.cursor_byte());
                cursor..cursor
            });
        Some(UTF16Selection {
            range,
            reversed: false,
        })
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_cursor_from_utf16_range(range_utf16);
        cx.notify();
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.byte_range_to_utf16_range(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.dictation_active || self.has_committed_dictation_pending_cleanup() {
            return;
        }

        if self.marked_range.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dictation_active {
            let range = self.active_replacement_range(range_utf16);
            self.replace_range_with_marked_text(range, text, None, cx);
            return;
        }

        if self.consume_or_reconcile_dictation_cleanup(range_utf16.clone(), text, cx) {
            return;
        }

        if !self.multiline && text.chars().any(|ch| ch == '\n' || ch == '\r') {
            cx.emit(InputSubmit {
                value: self.value.clone(),
            });
            return;
        }

        let range = self.active_replacement_range(range_utf16);
        self.replace_range_with_text(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dictation_active && self.has_committed_dictation_pending_cleanup() {
            self.clear_committed_dictation_cleanup();
        }

        // Critical: native IMEs keep provisional composition in marked text.
        // Replacing it as committed text breaks Vietnamese/Japanese updates.
        let range = self.active_replacement_range(range_utf16);
        self.replace_range_with_marked_text(range, new_text, new_selected_range_utf16, cx);
    }

    fn dictation_started(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.begin_dictation(cx);
    }

    fn insert_dictation_text(&mut self, text: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.insert_live_dictation_text(text, cx);
    }

    fn dictation_ended(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.finish_dictation(cx);
    }

    fn dictation_cancelled(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_dictation(cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.byte_to_utf16_offset(self.cursor_byte()))
    }

    fn text_input_traits(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> PlatformTextInputTraits {
        Self::text_input_traits_policy(self.native_suggestions, self.secure)
    }
}

impl Focusable for Input {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::Input;
    use gpui::{
        AppContext as _, PlatformTextAutocapitalization, PlatformTextInputTrait,
        PlatformTextInputTraits, TestAppContext,
    };

    fn assert_mutating_traits_disabled(traits: PlatformTextInputTraits) {
        assert_eq!(
            traits.autocapitalization,
            PlatformTextAutocapitalization::None
        );
        assert_eq!(traits.spell_checking, PlatformTextInputTrait::Disabled);
        assert_eq!(traits.smart_quotes, PlatformTextInputTrait::Disabled);
        assert_eq!(traits.smart_dashes, PlatformTextInputTrait::Disabled);
        assert_eq!(traits.smart_insert_delete, PlatformTextInputTrait::Disabled);
    }

    #[test]
    fn input_text_input_traits_default_to_disabled() {
        assert_eq!(
            Input::text_input_traits_policy(false, false),
            PlatformTextInputTraits::default()
        );
    }

    #[test]
    fn input_native_suggestions_opt_in_to_keyboard_suggestions() {
        let traits = Input::text_input_traits_policy(true, false);

        assert_eq!(traits, PlatformTextInputTraits::keyboard_suggestions());
        assert_eq!(traits.inline_prediction, PlatformTextInputTrait::Enabled);
        assert_eq!(traits.autocorrection, PlatformTextInputTrait::Enabled);
        assert_mutating_traits_disabled(traits);
    }

    #[test]
    fn secure_input_disables_native_suggestions_even_when_requested() {
        assert_eq!(
            Input::text_input_traits_policy(true, true),
            PlatformTextInputTraits::default()
        );
    }

    #[test]
    fn utf16_offsets_round_to_valid_utf8_boundaries() {
        let text = "aê🙂b";

        assert_eq!(Input::byte_offset_from_utf16(text, 0), 0);
        assert_eq!(Input::byte_offset_from_utf16(text, 1), "a".len());
        assert_eq!(Input::byte_offset_from_utf16(text, 2), "aê".len());

        // Critical: UIKit ranges are UTF-16 based. If a native IME probes
        // inside a surrogate pair, keep the byte index on a valid char
        // boundary instead of slicing through UTF-8.
        assert_eq!(Input::byte_offset_from_utf16(text, 3), "aê".len());
        assert_eq!(Input::byte_offset_from_utf16(text, 4), "aê🙂".len());

        assert_eq!(Input::byte_to_utf16_offset_in(text, "aê".len()), 2);
        assert_eq!(Input::byte_to_utf16_offset_in(text, "aê🙂".len()), 4);
        assert_eq!(Input::byte_to_utf16_offset_in(text, text.len()), 5);
    }

    #[test]
    fn marked_text_replaces_active_composition_until_commit() {
        let mut cx = TestAppContext::single();
        let input = cx.update(|cx| cx.new(Input::new));

        input.update(&mut cx, |input, cx| {
            input.replace_range_with_marked_text(0..0, "vo", Some(2..2), cx);
            assert_eq!(input.value, "vo");
            assert_eq!(input.marked_range, Some(0.."vo".len()));

            let range = input.active_replacement_range(None);
            input.replace_range_with_marked_text(range, "vơi", Some(3..3), cx);
            assert_eq!(input.value, "vơi");
            assert_eq!(input.marked_range, Some(0.."vơi".len()));

            let range = input.active_replacement_range(None);
            input.replace_range_with_text(range, "với", cx);
            assert_eq!(input.value, "với");
            assert_eq!(input.marked_range, None);
        });
    }

    #[test]
    fn marked_text_commit_and_native_suggestion_replacement_stay_independent() {
        let mut cx = TestAppContext::single();
        let input = cx.update(|cx| cx.new(Input::new));

        input.update(&mut cx, |input, cx| {
            input.replace_range_with_marked_text(0..0, "かな", Some(2..2), cx);
            let range = input.active_replacement_range(None);
            input.replace_range_with_text(range, "仮名", cx);
            assert_eq!(input.value, "仮名");
            assert_eq!(input.marked_range, None);

            let range = input.active_replacement_range(Some(0..2));
            input.replace_range_with_text(range, "かな", cx);
            assert_eq!(input.value, "かな");
            assert_eq!(input.marked_range, None);
            assert_eq!(input.cursor_byte, "かな".len());
        });
    }

    #[test]
    fn native_selection_updates_cursor_with_utf16_offsets() {
        let mut cx = TestAppContext::single();
        let input = cx.update(|cx| cx.new(Input::new));

        input.update(&mut cx, |input, _cx| {
            input.value = "aê🙂b".to_string();
            input.set_cursor_from_utf16_range(2..4);

            assert_eq!(input.cursor_byte, "aê🙂".len());
            assert_eq!(input.selected_range, Some("aê".len().."aê🙂".len()));
        });
    }

    #[test]
    fn native_selection_is_used_for_plain_replacement() {
        let mut cx = TestAppContext::single();
        let input = cx.update(|cx| cx.new(Input::new));

        input.update(&mut cx, |input, cx| {
            input.value = "abc".to_string();
            input.set_cursor_from_utf16_range(1..2);

            let range = input.active_replacement_range(None);
            input.replace_range_with_text(range, "X", cx);

            assert_eq!(input.value, "aXc");
            assert_eq!(input.selected_range, None);
        });
    }

    #[test]
    fn committed_dictation_cleanup_delete_does_not_clear_input() {
        let mut cx = TestAppContext::single();
        let input = cx.update(|cx| cx.new(Input::new));

        input.update(&mut cx, |input, cx| {
            input.begin_dictation(cx);
            input.insert_live_dictation_text("Hello", cx);
            input.finish_dictation(cx);

            assert_eq!(input.value, "Hello");
            assert_eq!(input.marked_range, Some(0.."Hello".len()));
            assert_eq!(
                input.committed_dictation_cleanup_range,
                Some(0.."Hello".len())
            );

            assert!(input.consume_or_reconcile_dictation_cleanup(Some(0..5), "", cx));
            assert_eq!(input.value, "Hello");
            assert_eq!(input.marked_range, None);
            assert_eq!(input.committed_dictation_cleanup_range, None);
        });
    }

    #[test]
    fn committed_dictation_reconciles_late_final_text_before_cleanup_delete() {
        let mut cx = TestAppContext::single();
        let input = cx.update(|cx| cx.new(Input::new));

        input.update(&mut cx, |input, cx| {
            input.begin_dictation(cx);
            input.insert_live_dictation_text("hello worl", cx);
            input.finish_dictation(cx);

            assert!(input.consume_or_reconcile_dictation_cleanup(
                Some(0.."hello worl".encode_utf16().count()),
                "hello world",
                cx,
            ));
            assert_eq!(input.value, "hello world");
            assert_eq!(input.marked_range, Some(0.."hello world".len()));
            assert_eq!(
                input.committed_dictation_cleanup_range,
                Some(0.."hello world".len())
            );

            assert!(input.consume_or_reconcile_dictation_cleanup(
                Some(0.."hello world".encode_utf16().count()),
                "",
                cx,
            ));
            assert_eq!(input.value, "hello world");
            assert_eq!(input.committed_dictation_cleanup_range, None);
        });
    }

    #[test]
    fn typing_after_dictation_commit_appends_instead_of_replacing_hypothesis() {
        let mut cx = TestAppContext::single();
        let input = cx.update(|cx| cx.new(Input::new));

        input.update(&mut cx, |input, cx| {
            input.begin_dictation(cx);
            input.insert_live_dictation_text("hello", cx);
            input.finish_dictation(cx);

            assert!(!input.consume_or_reconcile_dictation_cleanup(None, "!", cx));
            let range = input.active_replacement_range(None);
            input.replace_range_with_text(range, "!", cx);

            assert_eq!(input.value, "hello!");
            assert_eq!(input.marked_range, None);
            assert_eq!(input.committed_dictation_cleanup_range, None);
        });
    }

    #[test]
    fn cancelled_dictation_removes_uncommitted_hypothesis() {
        let mut cx = TestAppContext::single();
        let input = cx.update(|cx| cx.new(Input::new));

        input.update(&mut cx, |input, cx| {
            input.value = "prefix ".to_string();
            input.refresh_display_value();
            input.cursor_byte = input.value.len();

            input.begin_dictation(cx);
            input.insert_live_dictation_text("draft", cx);
            assert_eq!(input.value, "prefix draft");

            input.cancel_dictation(cx);
            assert_eq!(input.value, "prefix ");
            assert_eq!(input.marked_range, None);
            assert_eq!(input.committed_dictation_cleanup_range, None);
        });
    }
}

impl Render for Input {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_focused = self.focus_handle.is_focused(window);

        let display_value = self.display_value.clone();

        // Show placeholder only when unfocused and empty
        let show_placeholder = self.value.is_empty() && !is_focused;

        let text_color = if show_placeholder {
            rgb(theme::TEXT_MUTED)
        } else {
            rgb(theme::TEXT_SECONDARY)
        };

        let border_color = if is_focused {
            rgb(theme::BORDER_ACTIVE)
        } else {
            rgb(theme::BORDER_DEFAULT)
        };
        let text_size = if self.compact {
            theme::FONT_DETAIL
        } else {
            theme::FONT_BODY
        };
        let min_height = if self.compact { 36.0 } else { 44.0 };
        let horizontal_padding = if self.compact { 10.0 } else { 12.0 };
        let vertical_padding = if self.compact { 8.0 } else { 10.0 };
        let line_height = text_size * 1.45;
        let inner_max_h_px = self
            .multiline
            .then(|| self.max_lines)
            .flatten()
            .map(|n| n as f32 * line_height);

        // Build display text
        let display_text = if show_placeholder {
            self.placeholder.clone()
        } else {
            display_value
        };

        let shell = div()
            .id(("input", cx.entity_id()))
            .relative()
            .track_focus(&self.focus_handle)
            .on_pointer_down(|_, _, cx| {
                cx.stop_propagation();
            })
            .on_press(cx.listener(Self::handle_press))
            .on_key_down(cx.listener(Self::handle_key_down))
            .pl(px(horizontal_padding))
            .pr(px(horizontal_padding + self.trailing_gutter))
            .py(px(vertical_padding))
            .w_full()
            .min_h(px(min_height))
            .bg(rgb(theme::BG_SURFACE))
            .rounded(px(6.0))
            .border_1()
            .border_color(border_color)
            .text_color(text_color)
            .text_size(px(text_size))
            .cursor_text();

        let ime_overlay = div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .child(ImeInputHandlerElement { input: cx.entity() });

        if self.multiline {
            let scroll_child: AnyElement = if is_focused {
                MultilineInputText {
                    text: SharedString::from(display_text.clone()),
                    cursor_byte: clamp_byte_index(self.display_value.as_str(), self.cursor_byte),
                    draw_caret: true,
                }
                .into_any_element()
            } else {
                div()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .text_left()
                    .child(display_text)
                    .into_any_element()
            };
            let mut body = div()
                .id(("input-multiline-scroll", cx.entity_id()))
                .w_full();
            if let Some(h) = inner_max_h_px {
                body = body.max_h(px(h)).overflow_y_scroll();
            }
            shell
                .flex()
                .flex_col()
                .items_stretch()
                .child(body.child(scroll_child))
                .child(ime_overlay)
        } else {
            shell
                .flex()
                .flex_row()
                .items_center()
                .child(display_text)
                .when(is_focused, |this| this.child(self.render_cursor(cx)))
                .child(ime_overlay)
        }
    }
}
