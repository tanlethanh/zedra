use std::ops::Range;

use gpui::*;

use crate::terminal::Terminal;

pub struct TerminalInputHandler {
    entity: WeakEntity<Terminal>,
    bounds: Bounds<Pixels>,
}

impl TerminalInputHandler {
    pub fn new(entity: WeakEntity<Terminal>, bounds: Bounds<Pixels>) -> Self {
        Self { entity, bounds }
    }

    fn accepts_text_input_policy() -> bool {
        true
    }

    fn text_input_traits_policy() -> PlatformTextInputTraits {
        PlatformTextInputTraits::keyboard_suggestions()
    }

    fn send_text_to_terminal(term: &mut Terminal, text: &str) {
        let mut plain_text = String::new();
        for ch in text.chars() {
            // Intercept `\n`/`\r` and send as enter keystroke.
            if ch == '\n' || ch == '\r' {
                if !plain_text.is_empty() {
                    term.handle_ime_text(&plain_text);
                    plain_text.clear();
                }
                term.handle_keystroke(&Keystroke {
                    modifiers: Modifiers::default(),
                    key: "enter".to_string(),
                    key_char: None,
                });
                term.clear_text_input_context();
            } else {
                plain_text.push(ch);
            }
        }

        if !plain_text.is_empty() {
            term.handle_ime_text(&plain_text);
        }
    }

    fn send_backspaces_to_terminal(term: &mut Terminal, count: usize) {
        for _ in 0..count {
            term.handle_keystroke(&Keystroke {
                modifiers: Modifiers::default(),
                key: "backspace".to_string(),
                key_char: None,
            });
        }
    }

    fn text_resets_keyboard_context(text: &str) -> bool {
        text.chars()
            .any(|ch| ch == '\n' || ch == '\r' || ch.is_whitespace() || ch.is_ascii_punctuation())
    }

    fn send_keyboard_context_edit_to_terminal(
        term: &mut Terminal,
        backspaces: usize,
        text_to_insert: &str,
    ) {
        Self::send_backspaces_to_terminal(term, backspaces);
        Self::send_text_to_terminal(term, text_to_insert);
    }

    fn apply_keyboard_context_edit(
        term: &mut Terminal,
        replacement_range: Option<Range<usize>>,
        text: &str,
    ) {
        let reset_context = Self::text_resets_keyboard_context(text);
        let edit = term.replace_keyboard_input_context_range(replacement_range, text);
        Self::send_keyboard_context_edit_to_terminal(term, edit.backspaces, &edit.text_to_insert);
        if reset_context {
            term.clear_text_input_context();
        }
    }

    fn commit_marked_text(term: &mut Terminal, text: &str) {
        let edit = term.commit_marked_text_to_keyboard_context(text);
        Self::send_keyboard_context_edit_to_terminal(term, edit.backspaces, &edit.text_to_insert);
        if Self::text_resets_keyboard_context(text) {
            term.clear_text_input_context();
        }
    }
}

impl InputHandler for TerminalInputHandler {
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<UTF16Selection> {
        self.entity
            .read_with(_cx, |term, _| {
                let uses_text_input_document = term.is_dictation_active()
                    || term.has_uncommitted_marked_text()
                    || term.has_committed_dictation_pending_cleanup();
                let range = if uses_text_input_document {
                    term.text_input_selection_range()
                } else {
                    term.keyboard_input_context_selection_range()
                };
                Some(UTF16Selection {
                    range,
                    reversed: false,
                })
            })
            .ok()
            .flatten()
    }

    fn set_selected_text_range(
        &mut self,
        _range: Range<usize>,
        _window: &mut Window,
        _cx: &mut App,
    ) {
        // Critical: terminal input is a PTY diff stream, not a native editable
        // text field. UIKit's transient Telex selections must not become
        // multi-character terminal deletes.
    }

    fn marked_text_range(&mut self, _window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.entity
            .read_with(cx, |term, _| term.marked_text_range())
            .ok()
            .flatten()
    }

    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<String> {
        self.entity
            .read_with(_cx, |term, _| {
                let uses_text_input_document = term.is_dictation_active()
                    || term.has_uncommitted_marked_text()
                    || term.has_committed_dictation_pending_cleanup();
                let (range, text) = if uses_text_input_document {
                    term.text_input_document_text_for_range(range_utf16.clone())
                } else {
                    term.keyboard_input_context_text_for_range(range_utf16.clone())
                };
                *adjusted_range = Some(range.clone());
                Some(text)
            })
            .ok()
            .flatten()
    }

    fn replace_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let text = text.to_string();
        let entity = self.entity.clone();
        let _ = entity.update(cx, move |term, cx| {
            if term.is_dictation_active() {
                term.replace_marked_text_in_range(replacement_range, text, None);
                cx.notify();
                return;
            }

            if text.is_empty() {
                if term.consume_committed_dictation_cleanup_delete(replacement_range.clone()) {
                    cx.notify();
                } else if term.has_committed_dictation_pending_cleanup() {
                    // Critical: UIKit cleanup deletes are synthetic after a
                    // dictation commit. Stale ranges must not become PTY
                    // backspaces after a late final transcript reconciliation.
                    cx.notify();
                } else if term.has_uncommitted_marked_text() {
                    term.clear_marked_state();
                    cx.notify();
                } else if let Some(replacement_range) = replacement_range {
                    let context_was_empty = term.keyboard_input_context_is_empty();
                    let replaced_anchor = context_was_empty && !replacement_range.is_empty();
                    let edit = term
                        .replace_keyboard_input_context_range(Some(replacement_range.clone()), "");
                    let backspaces = if replaced_anchor
                        && edit.backspaces == 0
                        && edit.text_to_insert.is_empty()
                    {
                        1
                    } else {
                        edit.backspaces
                    };
                    Self::send_keyboard_context_edit_to_terminal(
                        term,
                        backspaces,
                        &edit.text_to_insert,
                    );
                    cx.notify();
                }
                return;
            }

            if term.has_committed_dictation_pending_cleanup() {
                if let Some(edit) = term.reconcile_committed_dictation_text(&text) {
                    Self::send_keyboard_context_edit_to_terminal(
                        term,
                        edit.backspaces,
                        &edit.text_to_insert,
                    );
                    cx.notify();
                }
                return;
            }

            if term.has_uncommitted_marked_text() {
                // IME commit arrives through insertText after setMarkedText; the
                // preedit was not sent to the PTY, so commit it from the shadow
                // text store rather than treating it as ordinary appended input.
                Self::commit_marked_text(term, &text);
                cx.notify();
                return;
            }
            Self::apply_keyboard_context_edit(term, replacement_range, &text);
            cx.notify();
        });
    }

    fn replace_text_in_range_from_context(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let text = text.to_string();
        let entity = self.entity.clone();
        let _ = entity.update(cx, move |term, cx| {
            if term.is_dictation_active() {
                term.replace_marked_text_in_range(replacement_range, text, None);
                cx.notify();
                return;
            }

            if text.is_empty() {
                if term.consume_committed_dictation_cleanup_delete(replacement_range.clone()) {
                    cx.notify();
                } else if term.has_committed_dictation_pending_cleanup() {
                    cx.notify();
                } else if term.has_uncommitted_marked_text() {
                    term.clear_marked_state();
                    cx.notify();
                } else {
                    Self::apply_keyboard_context_edit(term, replacement_range, &text);
                    cx.notify();
                }
                return;
            }

            if term.has_committed_dictation_pending_cleanup() {
                if let Some(edit) = term.reconcile_committed_dictation_text(&text) {
                    Self::send_keyboard_context_edit_to_terminal(
                        term,
                        edit.backspaces,
                        &edit.text_to_insert,
                    );
                    cx.notify();
                }
                return;
            }

            if term.has_uncommitted_marked_text() {
                Self::commit_marked_text(term, &text);
                cx.notify();
                return;
            }

            Self::apply_keyboard_context_edit(term, replacement_range, &text);
            cx.notify();
        });
    }

    fn delete_backward(&mut self, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            if term.is_dictation_active() || term.has_uncommitted_marked_text() {
                term.clear_marked_state();
                cx.notify();
                return;
            }

            if let Some(edit) = term.delete_keyboard_input_context_backward() {
                Self::send_keyboard_context_edit_to_terminal(
                    term,
                    edit.backspaces,
                    &edit.text_to_insert,
                );
            } else {
                Self::send_backspaces_to_terminal(term, 1);
            }
            cx.notify();
        });
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let text = new_text.to_string();
        let entity = self.entity.clone();
        let _ = entity.update(cx, move |term, cx| {
            // Both IME composition and dictation hypothesis use marked text.
            term.replace_marked_text_in_range(replacement_range, text, _new_selected_range);
            cx.notify();
        });
    }

    fn dictation_started(&mut self, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            term.begin_dictation();
            cx.notify();
        });
    }

    fn insert_dictation_text(&mut self, text: &str, _window: &mut Window, cx: &mut App) {
        let text = text.to_string();
        let entity = self.entity.clone();
        let _ = entity.update(cx, move |term, cx| {
            if term.is_dictation_active() {
                term.update_dictation_hypothesis(None, text, None);
            } else if term.has_committed_dictation_pending_cleanup() {
                // Critical: after committing a streamed dictation hypothesis,
                // UIKit can still deliver a final dictation insertion while it
                // reconciles the placeholder. The preserved synthetic document
                // is for late native reads only; do not send it to the PTY again.
                return;
            } else {
                Self::send_text_to_terminal(term, &text);
            }
            cx.notify();
        });
    }

    fn dictation_ended(&mut self, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            let text = term.finish_dictation();
            if let Some(text) = text {
                Self::send_text_to_terminal(term, &text);
            };
            cx.notify();
        });
    }

    fn dictation_recording_ended(&mut self, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            term.dictation_recording_ended();
            cx.notify();
        });
    }

    fn dictation_cancelled(&mut self, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            term.cancel_dictation();
            cx.notify();
        });
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            // UIKit can call unmarkText between dictation hypothesis updates
            // without first calling insertDictationResultPlaceholder on custom
            // UITextInput clients. Preserve the marked range until a real
            // commit or deletion clears it so UIDictationController can still
            // find its previous hypothesis.
            if term.unmark_text() {
                cx.notify();
            }
        });
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        Some(self.bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<usize> {
        Some(1)
    }

    fn accepts_text_input(&mut self, _window: &mut Window, _cx: &mut App) -> bool {
        Self::accepts_text_input_policy()
    }

    fn text_input_traits(
        &mut self,
        _window: &mut Window,
        _cx: &mut App,
    ) -> PlatformTextInputTraits {
        Self::text_input_traits_policy()
    }
}

#[cfg(test)]
mod tests {
    use super::TerminalInputHandler;
    use gpui::{PlatformTextAutocapitalization, PlatformTextInputTrait, PlatformTextInputTraits};

    #[test]
    fn terminal_accepts_text_input() {
        assert!(TerminalInputHandler::accepts_text_input_policy());
    }

    #[test]
    fn terminal_requests_native_keyboard_suggestions_without_smart_punctuation() {
        let traits = TerminalInputHandler::text_input_traits_policy();

        assert_eq!(traits, PlatformTextInputTraits::keyboard_suggestions());
        assert_eq!(
            traits.autocapitalization,
            PlatformTextAutocapitalization::None
        );
        assert_eq!(traits.inline_prediction, PlatformTextInputTrait::Enabled);
        assert_eq!(traits.autocorrection, PlatformTextInputTrait::Enabled);
        assert_eq!(traits.spell_checking, PlatformTextInputTrait::Disabled);
        assert_eq!(traits.smart_quotes, PlatformTextInputTrait::Disabled);
        assert_eq!(traits.smart_dashes, PlatformTextInputTrait::Disabled);
        assert_eq!(traits.smart_insert_delete, PlatformTextInputTrait::Disabled);
    }
}
