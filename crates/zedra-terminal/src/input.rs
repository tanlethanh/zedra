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
            } else {
                plain_text.push(ch);
            }
        }

        if !plain_text.is_empty() {
            term.handle_ime_text(&plain_text);
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
        Some(UTF16Selection {
            range: selection,
            reversed: false,
        })
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
        let _ = entity.update(cx, move |term, cx| {
            if replacement_range.is_some() && text.is_empty() {
                // UIKit sends a delete (empty replacement) to clear the current selection.
                if term.is_dictation_active() {
                    if let Some(range) = replacement_range.clone() {
                        term.replace_marked_text_in_range(Some(range), String::new(), None);
                    }
                    cx.notify();
                    return;
                }
                if term.marked_text().is_some() {
                    term.clear_marked_state();
                    cx.notify();
                    return;
                }
                term.handle_keystroke(&Keystroke {
                    modifiers: Modifiers::default(),
                    key: "backspace".to_string(),
                    key_char: None,
                });
            } else if !text.is_empty() {
                if term.is_dictation_active() {
                    term.replace_marked_text_in_range(replacement_range.clone(), text, None);
                    cx.notify();
                    return;
                }

                term.clear_marked_state();
                Self::send_text_to_terminal(term, &text);
            }
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
            if let Some(text) = term.finish_dictation() {
                Self::send_text_to_terminal(term, &text);
            };
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
}

#[cfg(test)]
mod tests {
    use super::TerminalInputHandler;

    #[test]
    fn terminal_accepts_text_input() {
        assert!(TerminalInputHandler::accepts_text_input_policy());
    }
}
