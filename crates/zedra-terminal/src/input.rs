use std::ops::Range;
use tracing::*;

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

    fn synthetic_document_len(marked_text: Option<&str>) -> usize {
        marked_text
            .map(|text| text.encode_utf16().count())
            .unwrap_or(" ".encode_utf16().count())
    }

    fn accepts_text_input_policy() -> bool {
        true
    }

    fn disable_default_keyboard_behavior_policy() -> bool {
        true
    }

    fn disable_default_focus_behavior_policy() -> bool {
        true
    }
}

impl InputHandler for TerminalInputHandler {
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection> {
        let pos = self
            .entity
            .read_with(cx, |term, _| {
                // UIKit's deleteBackward path expects the caret to sit within the
                // document it sees via text_for_range/endOfDocument. When the terminal
                // has no active marked text, we still expose a one-code-unit placeholder
                // document so backspace can target that synthetic position.
                Self::synthetic_document_len(term.marked_text())
            })
            .unwrap_or(Self::synthetic_document_len(None));
        Some(UTF16Selection {
            range: pos..pos,
            reversed: false,
        })
    }

    fn marked_text_range(&mut self, _window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        let range = self
            .entity
            .read_with(cx, |term, _| term.marked_text_range())
            .ok()
            .flatten();
        debug!("marked_text_range → {:?}", range);
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
            .read_with(cx, |term, _| term.marked_text().unwrap_or(" ").to_string())
            .unwrap_or_else(|_| " ".to_string());
        let utf16_len = doc.encode_utf16().count();
        let start = range_utf16.start.min(utf16_len);
        let end = range_utf16.end.min(utf16_len);
        *adjusted_range = Some(start..end);
        let range = Self::range_from_utf16(&doc, &(start..end));
        let result = doc[range].to_string();
        debug!(
            "text_for_range {:?} → doc={:?} result={:?}",
            range_utf16, doc, result
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
        let _ = entity.update(cx, move |term, cx| {
            if replacement_range.is_some() && text.is_empty() {
                // UIKit sends a delete (empty replacement) to clear the current selection.
                // During dictation startup the pending buffer is empty — ignore to avoid
                // forwarding a spurious backspace to the terminal.
                if term.is_dictation_active() {
                    debug!("replace_text_in_range: ignoring delete during dictation");
                    return;
                }
                debug!("replace_text_in_range: sending backspace keystroke");
                term.handle_keystroke(&Keystroke {
                    modifiers: Modifiers::default(),
                    key: "backspace".to_string(),
                    key_char: None,
                });
            } else if !text.is_empty() {
                if term.is_dictation_active() {
                    debug!(
                        "replace_text_in_range: dictation active, updating hypothesis {:?}",
                        text
                    );
                    term.set_marked_text(text);
                    cx.notify();
                    return;
                }

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
        });
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _replacement_range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let text = new_text.to_string();
        let entity = self.entity.clone();
        let _ = entity.update(cx, move |term, cx| {
            // Both IME composition and dictation hypothesis use set_marked_text —
            // marked_text serves as dictation hypothesis when dictation_active=true.
            term.set_marked_text(text);
            cx.notify();
        });
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            // UIKit calls unmarkText between dictation hypothesis updates.
            // Keep marked=true while dictation is active so the hypothesis range
            // stays visible to UIKit.
            if term.is_dictation_active() {
                debug!("unmark_text: skipped during active dictation");
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

    fn disable_default_keyboard_behavior(&mut self, _window: &mut Window, _cx: &mut App) -> bool {
        Self::disable_default_keyboard_behavior_policy()
    }

    fn disable_default_focus_behavior(&mut self, _window: &mut Window, _cx: &mut App) -> bool {
        Self::disable_default_focus_behavior_policy()
    }

    // fn dictation_started(&mut self, _window: &mut Window, cx: &mut App) {
    //     debug!("dictation_started");
    //     let entity = self.entity.clone();
    //     let _ = entity.update(cx, |term, cx| {
    //         term.begin_dictation();
    //         cx.notify();
    //     });
    // }

    // fn insert_dictation_text(&mut self, text: &str, _window: &mut Window, cx: &mut App) {
    //     debug!("insert_dictation_text {:?}", text);
    //     let text = text.to_string();
    //     let entity = self.entity.clone();
    //     let _ = entity.update(cx, move |term, cx| {
    //         term.set_marked_text(text);
    //         cx.notify();
    //     });
    // }

    // fn dictation_ended(&mut self, _window: &mut Window, cx: &mut App) {
    //     debug!("dictation_ended");
    //     let entity = self.entity.clone();
    //     let _ = entity.update(cx, |term, cx| {
    //         term.end_dictation();
    //         cx.notify();
    //     });
    // }
}

#[cfg(test)]
mod tests {
    use super::TerminalInputHandler;

    #[test]
    fn synthetic_document_len_uses_placeholder_when_empty() {
        assert_eq!(TerminalInputHandler::synthetic_document_len(None), 1);
    }

    #[test]
    fn synthetic_document_len_tracks_utf16_units_for_marked_text() {
        assert_eq!(TerminalInputHandler::synthetic_document_len(Some("abc")), 3);
        assert_eq!(TerminalInputHandler::synthetic_document_len(Some("🙂")), 2);
    }

    #[test]
    fn terminal_accepts_text_but_owns_keyboard_request() {
        assert!(TerminalInputHandler::accepts_text_input_policy());
        assert!(TerminalInputHandler::disable_default_keyboard_behavior_policy());
        assert!(TerminalInputHandler::disable_default_focus_behavior_policy());
    }
}
