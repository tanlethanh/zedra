use std::ops::Range;

use gpui::*;
use tracing::info;

use crate::terminal::Terminal;

pub struct TerminalInputHandler {
    entity: WeakEntity<Terminal>,
    bounds: Bounds<Pixels>,
    context_rewrite_active: bool,
}

impl TerminalInputHandler {
    pub fn new(entity: WeakEntity<Terminal>, bounds: Bounds<Pixels>) -> Self {
        Self {
            entity,
            bounds,
            context_rewrite_active: false,
        }
    }

    fn offset_from_utf16(text: &str, offset: usize) -> usize {
        let mut utf16_count = 0;
        for (utf8_index, ch) in text.char_indices() {
            if utf16_count >= offset {
                return utf8_index;
            }
            utf16_count += ch.len_utf16();
        }
        text.len()
    }

    fn range_from_utf16(text: &str, range_utf16: &Range<usize>) -> Range<usize> {
        Self::offset_from_utf16(text, range_utf16.start)
            ..Self::offset_from_utf16(text, range_utf16.end)
    }

    fn accepts_text_input_policy() -> bool {
        true
    }

    fn text_input_traits_policy() -> PlatformTextInputTraits {
        PlatformTextInputTraits::keyboard_suggestions()
    }

    fn send_text_to_terminal(term: &mut Terminal, text: &str) {
        info!(text = %text, "KeyboardDebug terminal_input send_text_to_terminal");
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
        info!(
            count,
            "KeyboardDebug terminal_input send_backspaces_to_terminal"
        );
        for _ in 0..count {
            term.handle_keystroke(&Keystroke {
                modifiers: Modifiers::default(),
                key: "backspace".to_string(),
                key_char: None,
            });
        }
    }
}

impl InputHandler for TerminalInputHandler {
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection> {
        let selection = match self.entity.read_with(cx, |term, _| {
            // UIKit's deleteBackward path expects the caret to sit within the
            // document it sees via text_for_range/endOfDocument. When the terminal
            // has no active marked text, we still expose a one-code-unit placeholder
            // document so backspace can target that synthetic position.
            term.text_input_selection_range()
        }) {
            Ok(selection) => selection,
            Err(_) => {
                let pos = " ".encode_utf16().count();
                pos..pos
            }
        };
        info!(
            selection = ?selection,
            "KeyboardDebug terminal_input selected_text_range"
        );
        Some(UTF16Selection {
            range: selection,
            reversed: false,
        })
    }

    fn marked_text_range(&mut self, _window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        let range = self
            .entity
            .read_with(cx, |term, _| term.marked_text_range())
            .ok()
            .flatten();
        info!(range = ?range, "KeyboardDebug terminal_input marked_text_range");
        range
    }

    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        let doc = self
            .entity
            .read_with(cx, |term, _| term.text_input_document().to_string())
            .unwrap_or_else(|_| " ".to_string());
        let utf16_len = doc.encode_utf16().count();
        let start = range_utf16.start.min(utf16_len);
        let end = range_utf16.end.min(utf16_len);
        *adjusted_range = Some(start..end);
        let range = Self::range_from_utf16(&doc, &(start..end));
        let result = doc[range].to_string();
        info!(
            requested_range = ?range_utf16,
            adjusted_range = ?(start..end),
            document = %doc,
            result = %result,
            "KeyboardDebug terminal_input text_for_range"
        );
        Some(result)
    }

    fn replace_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let entity = self.entity.clone();
        let text = text.to_string();
        let track_redundant_context_insert = self.context_rewrite_active;
        self.context_rewrite_active = false;
        let _ = entity.update(cx, move |term, cx| {
            info!(
                replacement_range = ?replacement_range,
                text = %text,
                track_redundant_context_insert,
                dictation_active = term.is_dictation_active(),
                has_marked = term.has_uncommitted_marked_text(),
                pending_dictation_cleanup = term.has_committed_dictation_pending_cleanup(),
                marked_range = ?term.marked_text_range(),
                selection_range = ?term.text_input_selection_range(),
                document = %term.text_input_document(),
                "KeyboardDebug terminal_input replace_text_in_range start"
            );
            if replacement_range.is_some() && text.is_empty() {
                // UIKit sends a delete (empty replacement) to clear the current selection.
                if term.is_dictation_active() {
                    info!(
                        replacement_range = ?replacement_range,
                        "KeyboardDebug terminal_input replace_text_in_range delete_dictation_marked"
                    );
                    if let Some(range) = replacement_range.clone() {
                        term.replace_marked_text_in_range(Some(range), String::new(), None);
                    }
                    cx.notify();
                    return;
                }
                if term.consume_committed_dictation_cleanup_delete(replacement_range.clone()) {
                    info!(
                        replacement_range = ?replacement_range,
                        "KeyboardDebug terminal_input replace_text_in_range consume_dictation_cleanup_delete"
                    );
                    cx.notify();
                    return;
                }
                if term.has_uncommitted_marked_text() {
                    info!(
                        replacement_range = ?replacement_range,
                        marked_range = ?term.marked_text_range(),
                        document = %term.text_input_document(),
                        "KeyboardDebug terminal_input replace_text_in_range clear_marked_delete"
                    );
                    term.clear_marked_state();
                    cx.notify();
                    return;
                }
                let removed_text = if track_redundant_context_insert {
                    term.replace_text_input_context_range_from_context_rewrite(
                        replacement_range.clone(),
                        "",
                    )
                } else {
                    term.replace_text_input_context_range(replacement_range.clone(), "")
                };
                let count = removed_text.chars().count().max(1);
                info!(
                    replacement_range = ?replacement_range,
                    removed_text = %removed_text,
                    backspaces = count,
                    "KeyboardDebug terminal_input replace_text_in_range delete_context"
                );
                Self::send_backspaces_to_terminal(term, count);
            } else if !text.is_empty() {
                if term.is_dictation_active() {
                    info!(
                        replacement_range = ?replacement_range,
                        text = %text,
                        "KeyboardDebug terminal_input replace_text_in_range buffer_dictation_text"
                    );
                    term.replace_marked_text_in_range(replacement_range.clone(), text, None);
                    cx.notify();
                    return;
                }

                if term.has_uncommitted_marked_text() {
                    info!(
                        replacement_range = ?replacement_range,
                        text = %text,
                        marked_range = ?term.marked_text_range(),
                        document = %term.text_input_document(),
                        "KeyboardDebug terminal_input replace_text_in_range clear_marked_before_insert"
                    );
                    term.clear_marked_state();
                }
                let text = if replacement_range.is_none() {
                    if let Some(text_to_insert) = term.consume_pending_redundant_context_insert(&text)
                    {
                        info!(
                            original_text = %text,
                            text_to_insert = %text_to_insert,
                            "KeyboardDebug terminal_input replace_text_in_range redundant_context_insert"
                        );
                        if text_to_insert.is_empty() {
                            cx.notify();
                            return;
                        }
                        text_to_insert
                    } else {
                        text
                    }
                } else {
                    text
                };
                let removed_text = if track_redundant_context_insert {
                    term.replace_text_input_context_range_from_context_rewrite(
                        replacement_range.clone(),
                        &text,
                    )
                } else {
                    term.replace_text_input_context_range(replacement_range.clone(), &text)
                };
                info!(
                    replacement_range = ?replacement_range,
                    text = %text,
                    removed_text = %removed_text,
                    "KeyboardDebug terminal_input replace_text_in_range normal_text"
                );
                Self::send_backspaces_to_terminal(term, removed_text.chars().count());
                Self::send_text_to_terminal(term, &text);
            }
        });
    }

    fn replace_text_in_range_from_context(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.context_rewrite_active = true;
        self.replace_text_in_range(replacement_range, text, window, cx);
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
            info!(
                replacement_range = ?replacement_range,
                selected_range = ?_new_selected_range,
                text = %text,
                dictation_active = term.is_dictation_active(),
                marked_range_before = ?term.marked_text_range(),
                document_before = %term.text_input_document(),
                "KeyboardDebug terminal_input replace_and_mark_text_in_range start"
            );
            // Both IME composition and dictation hypothesis use marked text.
            term.replace_marked_text_in_range(replacement_range, text, _new_selected_range);
            info!(
                marked_range_after = ?term.marked_text_range(),
                selection_range_after = ?term.text_input_selection_range(),
                document_after = %term.text_input_document(),
                "KeyboardDebug terminal_input replace_and_mark_text_in_range end"
            );
            cx.notify();
        });
    }

    fn dictation_started(&mut self, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            info!(
                dictation_active_before = term.is_dictation_active(),
                marked_range_before = ?term.marked_text_range(),
                document_before = %term.text_input_document(),
                "KeyboardDebug terminal_input dictation_started"
            );
            term.begin_dictation();
            cx.notify();
        });
    }

    fn insert_dictation_text(&mut self, text: &str, _window: &mut Window, cx: &mut App) {
        let text = text.to_string();
        let entity = self.entity.clone();
        let _ = entity.update(cx, move |term, cx| {
            info!(
                text = %text,
                dictation_active = term.is_dictation_active(),
                pending_dictation_cleanup = term.has_committed_dictation_pending_cleanup(),
                marked_range = ?term.marked_text_range(),
                document = %term.text_input_document(),
                "KeyboardDebug terminal_input insert_dictation_text start"
            );
            if term.is_dictation_active() {
                term.replace_marked_text_in_range(None, text, None);
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
            info!(
                committed_text = ?text,
                pending_dictation_cleanup = term.has_committed_dictation_pending_cleanup(),
                marked_range_after = ?term.marked_text_range(),
                document_after = %term.text_input_document(),
                "KeyboardDebug terminal_input dictation_ended"
            );
            if let Some(text) = text {
                Self::send_text_to_terminal(term, &text);
            };
            cx.notify();
        });
    }

    fn dictation_cancelled(&mut self, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            info!(
                dictation_active_before = term.is_dictation_active(),
                marked_range_before = ?term.marked_text_range(),
                document_before = %term.text_input_document(),
                "KeyboardDebug terminal_input dictation_cancelled"
            );
            term.cancel_dictation();
            cx.notify();
        });
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            info!(
                dictation_active = term.is_dictation_active(),
                has_marked = term.has_uncommitted_marked_text(),
                pending_dictation_cleanup = term.has_committed_dictation_pending_cleanup(),
                marked_range = ?term.marked_text_range(),
                selection_range = ?term.text_input_selection_range(),
                document = %term.text_input_document(),
                "KeyboardDebug terminal_input unmark_text"
            );
            // UIKit can call unmarkText between dictation hypothesis updates
            // without first calling insertDictationResultPlaceholder on custom
            // UITextInput clients. Preserve the marked range until a real
            // commit or deletion clears it so UIDictationController can still
            // find its previous hypothesis.
            if term.is_dictation_active() {
                return;
            }
            term.clear_marked_state();
            cx.notify();
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
