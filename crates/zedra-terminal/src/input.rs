use std::ops::Range;

use gpui::*;

use crate::terminal::Terminal;

#[derive(Clone, Debug, PartialEq, Eq)]
struct TextInputPreflight {
    text: String,
}

pub struct TerminalInputHandler {
    entity: WeakEntity<Terminal>,
    bounds: Bounds<Pixels>,
    pending_text_input_preflight: Option<TextInputPreflight>,
    text_input_rewrite_active: bool,
    text_input_rewrite_guard_active: bool,
}

impl TerminalInputHandler {
    pub fn new(entity: WeakEntity<Terminal>, bounds: Bounds<Pixels>) -> Self {
        Self {
            entity,
            bounds,
            pending_text_input_preflight: None,
            text_input_rewrite_active: false,
            text_input_rewrite_guard_active: false,
        }
    }

    fn accepts_text_input_policy() -> bool {
        true
    }

    fn text_input_traits_policy() -> PlatformTextInputTraits {
        PlatformTextInputTraits::keyboard_suggestions()
    }

    fn utf16_len(text: &str) -> usize {
        text.encode_utf16().count()
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

    fn apply_streamed_text_input_context_edit(
        term: &mut Terminal,
        replacement_range: Option<Range<usize>>,
        text: &str,
    ) {
        term.replace_streamed_text_input_context_range(replacement_range, text);
    }

    fn flush_streamed_text_input_context(term: &mut Terminal) {
        if let Some(edit) = term.flush_streamed_text_input_context() {
            Self::send_keyboard_context_edit_to_terminal(
                term,
                edit.backspaces,
                &edit.text_to_insert,
            );
        }
    }

    fn commit_marked_text(term: &mut Terminal, text: &str) {
        let edit = term.commit_marked_text_to_keyboard_context(text);
        Self::send_keyboard_context_edit_to_terminal(term, edit.backspaces, &edit.text_to_insert);
        if Self::text_resets_keyboard_context(text) {
            term.clear_text_input_context();
        }
    }

    fn apply_dictation_context_rewrite(
        term: &mut Terminal,
        replacement_range: Option<Range<usize>>,
        text: &str,
    ) -> bool {
        if !term.is_dictation_active() {
            return false;
        }

        term.update_dictation_hypothesis(replacement_range, text.to_string(), None);
        true
    }

    fn finish_dictation_or_streamed_preview(term: &mut Terminal) {
        if let Some(edit) = term.commit_streamed_text_input_context() {
            Self::send_keyboard_context_edit_to_terminal(
                term,
                edit.backspaces,
                &edit.text_to_insert,
            );
            return;
        }

        let text = term.finish_dictation();
        if let Some(text) = text {
            Self::send_text_to_terminal(term, &text);
        }
    }

    fn should_stage_unconfirmed_text_input(
        term: &Terminal,
        replacement_range: Option<&Range<usize>>,
        text: &str,
        rewrite_guard_active: bool,
    ) -> bool {
        if text.is_empty() {
            return false;
        }

        if term.is_dictation_active() || term.has_streamed_text_input_pending_commit() {
            return true;
        }

        // IME delete/rewrite flows can replay unconfirmed text after the
        // confirmed correction; do not let that bootstrap dictation preview.
        if rewrite_guard_active {
            return false;
        }

        if term.has_committed_dictation_pending_cleanup() || term.has_uncommitted_marked_text() {
            return false;
        }

        let document_utf16_len = Self::utf16_len(term.text_input_document());
        let replacement_stays_on_anchor = replacement_range
            .map(|range| range.start <= document_utf16_len && range.end <= document_utf16_len)
            .unwrap_or(true);

        // Bootstrap unconfirmed native text only from the empty anchor. Once
        // it is staged, all rewrites stay in the preview store until an
        // explicit commit/cancel boundary decides whether it reaches the PTY.
        term.keyboard_input_context_is_empty()
            && document_utf16_len <= 1
            && replacement_stays_on_anchor
    }

    fn should_keep_text_input_rewrite_active(
        pending_exists: bool,
        exact_pending_insert: bool,
        rewrite_active: bool,
    ) -> bool {
        (pending_exists || rewrite_active) && !(exact_pending_insert && !rewrite_active)
    }

    fn should_clear_text_input_rewrite_for_delete(pending: Option<&TextInputPreflight>) -> bool {
        pending.is_some_and(|pending| pending.text.is_empty())
    }

    fn should_mark_text_input_rewrite_active_for_delete(
        pending: Option<&TextInputPreflight>,
        rewrite_active: bool,
    ) -> bool {
        !Self::should_clear_text_input_rewrite_for_delete(pending)
            && (rewrite_active || pending.is_some())
    }

    fn observe_text_input_preflight(&mut self, range: Option<Range<usize>>, text: String) {
        // A delete or range rewrite often belongs to IME correction; later
        // unconfirmed inserts in the same burst are context replay.
        if text.is_empty() || range.as_ref().is_some_and(|range| !range.is_empty()) {
            self.text_input_rewrite_guard_active = true;
        }
        self.pending_text_input_preflight = Some(TextInputPreflight { text });
        self.text_input_rewrite_active = false;
    }

    fn consume_insert_text_preflight(&mut self, text: &str) -> bool {
        let pending = self.pending_text_input_preflight.take();
        let was_confirmed = pending.is_some() || self.text_input_rewrite_active;
        let exact_pending_insert = pending.as_ref().is_some_and(|pending| pending.text == text);
        self.text_input_rewrite_active = Self::should_keep_text_input_rewrite_active(
            pending.is_some(),
            exact_pending_insert,
            self.text_input_rewrite_active,
        );
        was_confirmed
    }

    fn consume_replace_range_preflight(&mut self) -> bool {
        let was_confirmed =
            self.pending_text_input_preflight.is_some() || self.text_input_rewrite_active;
        self.pending_text_input_preflight = None;
        self.text_input_rewrite_active = false;
        was_confirmed
    }

    fn observe_delete_backward(&mut self) {
        if Self::should_clear_text_input_rewrite_for_delete(
            self.pending_text_input_preflight.as_ref(),
        ) {
            self.pending_text_input_preflight = None;
            self.text_input_rewrite_active = false;
            self.text_input_rewrite_guard_active = true;
            return;
        }

        if Self::should_mark_text_input_rewrite_active_for_delete(
            self.pending_text_input_preflight.as_ref(),
            self.text_input_rewrite_active,
        ) {
            self.text_input_rewrite_active = true;
            self.text_input_rewrite_guard_active = true;
        }
    }

    fn clear_text_input_preflight(&mut self) {
        self.pending_text_input_preflight = None;
        self.text_input_rewrite_active = false;
        self.text_input_rewrite_guard_active = false;
    }

    fn replace_text_input_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        flush_pending_stream: bool,
        cx: &mut App,
    ) {
        let text = text.to_string();
        let entity = self.entity.clone();
        if Self::text_resets_keyboard_context(&text) {
            self.text_input_rewrite_guard_active = false;
        }
        let _ = entity.update(cx, move |term, cx| {
            if flush_pending_stream {
                // Confirmed input after a speculative preview must first make
                // the preview ordinary PTY text, then apply the confirmed edit.
                Self::flush_streamed_text_input_context(term);
            }
            if Self::apply_dictation_context_rewrite(term, replacement_range.clone(), &text) {
                cx.notify();
                return;
            }

            if text.is_empty() {
                if term.consume_committed_dictation_cleanup_delete(replacement_range.clone()) {
                    cx.notify();
                } else if term.has_streamed_text_input_pending_commit() {
                    term.cancel_streamed_text_input_context();
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

            if term.has_streamed_text_input_pending_commit() {
                term.cancel_streamed_text_input_context();
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
                    || term.has_committed_dictation_pending_cleanup()
                    || term.has_streamed_text_input_pending_commit();
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

    fn set_selected_text_range(&mut self, range: Range<usize>, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, move |term, cx| {
            // UIKit can select inside the synthetic text-input context while
            // rewriting IME text. Keep that selection so replayed context replaces
            // the shadow document instead of appending duplicate text to the PTY.
            term.set_text_input_selection_range(range);
            cx.notify();
        });
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
                    || term.has_committed_dictation_pending_cleanup()
                    || term.has_streamed_text_input_pending_commit();
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

    fn should_change_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        _cx: &mut App,
    ) -> bool {
        self.observe_text_input_preflight(replacement_range, text.to_string());
        true
    }

    fn insert_text(&mut self, text: &str, _window: &mut Window, cx: &mut App) {
        let text = text.to_string();
        let confirmed_by_preflight = self.consume_insert_text_preflight(&text);
        let rewrite_guard_active = self.text_input_rewrite_guard_active;
        let entity = self.entity.clone();
        let should_preview = entity
            .read_with(cx, |term, _| {
                term.is_dictation_active()
                    || (!confirmed_by_preflight
                        && Self::should_stage_unconfirmed_text_input(
                            term,
                            None,
                            &text,
                            rewrite_guard_active,
                        ))
            })
            .unwrap_or(false);
        if !should_preview {
            self.replace_text_input_range(None, &text, confirmed_by_preflight, cx);
            return;
        }

        let _ = entity.update(cx, move |term, cx| {
            if Self::apply_dictation_context_rewrite(term, None, &text) {
                cx.notify();
                return;
            }

            Self::apply_streamed_text_input_context_edit(term, None, &text);
            cx.notify();
        });
    }

    fn replace_range(
        &mut self,
        replacement_range: Range<usize>,
        text: &str,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let text = text.to_string();
        let confirmed_by_preflight = self.consume_replace_range_preflight();
        let rewrite_guard_active = self.text_input_rewrite_guard_active;
        let entity = self.entity.clone();
        let preview_range = replacement_range.clone();
        let should_preview = entity
            .read_with(cx, |term, _| {
                term.is_dictation_active()
                    || (!confirmed_by_preflight
                        && Self::should_stage_unconfirmed_text_input(
                            term,
                            Some(&preview_range),
                            &text,
                            rewrite_guard_active,
                        ))
            })
            .unwrap_or(false);
        if !should_preview {
            self.replace_text_input_range(
                Some(replacement_range),
                &text,
                confirmed_by_preflight,
                cx,
            );
            return;
        }

        let _ = entity.update(cx, move |term, cx| {
            if Self::apply_dictation_context_rewrite(term, Some(replacement_range.clone()), &text) {
                cx.notify();
                return;
            }

            Self::apply_streamed_text_input_context_edit(term, Some(replacement_range), &text);
            cx.notify();
        });
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
                } else if term.has_streamed_text_input_pending_commit() {
                    term.cancel_streamed_text_input_context();
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

            if term.has_streamed_text_input_pending_commit() {
                term.cancel_streamed_text_input_context();
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

    fn delete_backward(&mut self, _window: &mut Window, cx: &mut App) {
        self.observe_delete_backward();
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            if term.is_dictation_active() {
                term.cancel_dictation();
                cx.notify();
                return;
            }

            if term.has_streamed_text_input_pending_commit() {
                // The preview text has not reached the PTY yet, so backspace must
                // only cancel the synthetic marked store.
                term.cancel_streamed_text_input_context();
                cx.notify();
                return;
            }

            if term.has_uncommitted_marked_text() {
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
        marked_text: &str,
        selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        self.clear_text_input_preflight();
        let text = marked_text.to_string();
        let entity = self.entity.clone();
        let _ = entity.update(cx, move |term, cx| {
            // Both IME composition and dictation hypothesis use marked text.
            term.replace_marked_text_in_range(replacement_range, text, selected_range);
            cx.notify();
        });
    }

    fn insert_dictation_result_placeholder(&mut self, _window: &mut Window, cx: &mut App) {
        self.clear_text_input_preflight();
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            if term.has_streamed_text_input_pending_commit() && !term.is_dictation_active() {
                // The stream is already represented by marked text and preview;
                // a late placeholder must not clear the range UIKit still needs.
                cx.notify();
                return;
            }
            term.begin_dictation();
            cx.notify();
        });
    }

    fn insert_dictation_result(&mut self, text: &str, _window: &mut Window, cx: &mut App) {
        self.clear_text_input_preflight();
        let text = text.to_string();
        let entity = self.entity.clone();
        let _ = entity.update(cx, move |term, cx| {
            if let Some(edit) = term.reconcile_late_dictation_result_after_cleanup(&text) {
                Self::send_keyboard_context_edit_to_terminal(
                    term,
                    edit.backspaces,
                    &edit.text_to_insert,
                );
            } else if term.is_dictation_active() {
                term.update_dictation_hypothesis(None, text.clone(), None);
                Self::finish_dictation_or_streamed_preview(term);
            } else if term.has_streamed_text_input_pending_commit() {
                let replacement_range = term.marked_text_range();
                Self::apply_streamed_text_input_context_edit(term, replacement_range, &text);
                Self::finish_dictation_or_streamed_preview(term);
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

    fn remove_dictation_result_placeholder(
        &mut self,
        will_insert_result: bool,
        _window: &mut Window,
        cx: &mut App,
    ) {
        self.clear_text_input_preflight();
        let entity = self.entity.clone();
        let _ = entity.update(cx, move |term, cx| {
            if will_insert_result {
                cx.notify();
                return;
            }

            if term.is_dictation_active() || term.has_streamed_text_input_pending_commit() {
                Self::finish_dictation_or_streamed_preview(term);
            }
            cx.notify();
        });
    }

    fn dictation_recording_did_end(&mut self, _window: &mut Window, cx: &mut App) {
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            term.dictation_recording_ended();
            cx.notify();
        });
    }

    fn dictation_recognition_failed(&mut self, _window: &mut Window, cx: &mut App) {
        self.clear_text_input_preflight();
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            term.cancel_dictation();
            cx.notify();
        });
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut App) {
        self.clear_text_input_preflight();
        let entity = self.entity.clone();
        let _ = entity.update(cx, |term, cx| {
            // UIKit can call unmarkText between dictation hypothesis updates
            // without first calling insertDictationResultPlaceholder on custom
            // UITextInput clients. Preserve the marked range until a real
            // commit or deletion clears it so UIDictationController can still
            // find its previous hypothesis.
            let cleared = term.unmark_text();
            if cleared {
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
    use super::{TerminalInputHandler, TextInputPreflight};
    use crate::terminal::{Terminal, TerminalEvent};
    use gpui::{
        PlatformTextAutocapitalization, PlatformTextInputTrait, PlatformTextInputTraits, px,
    };

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

    #[test]
    fn dictation_context_rewrite_emits_preview_update() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));
        let mut events = terminal.subscribe_events();

        terminal.begin_dictation();
        events.try_recv().expect("expected empty preview");

        assert!(TerminalInputHandler::apply_dictation_context_rewrite(
            &mut terminal,
            Some(1..1),
            "Hi"
        ));
        match events.try_recv().expect("expected context rewrite preview") {
            TerminalEvent::DictationPreviewChanged(Some(text)) => assert_eq!(text, "Hi"),
            event => panic!("expected dictation preview update, got {event:?}"),
        }

        assert_eq!(terminal.text_input_document(), " Hi");
        assert_eq!(terminal.marked_text_range(), Some(1..3));
    }

    #[test]
    fn unconfirmed_text_input_stages_only_from_empty_anchor() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));

        assert!(TerminalInputHandler::should_stage_unconfirmed_text_input(
            &terminal, None, "h", false
        ));
        assert!(TerminalInputHandler::should_stage_unconfirmed_text_input(
            &terminal, None, "hey", false
        ));
        assert!(!TerminalInputHandler::should_stage_unconfirmed_text_input(
            &terminal, None, "h", true
        ));
        assert!(!TerminalInputHandler::should_stage_unconfirmed_text_input(
            &terminal, None, "hey", true
        ));

        terminal.replace_keyboard_input_context_range(None, "x");
        assert!(!TerminalInputHandler::should_stage_unconfirmed_text_input(
            &terminal,
            Some(&(0..1)),
            "xe",
            false,
        ));
    }

    #[test]
    fn unconfirmed_text_input_does_not_promote_after_context_was_committed() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));

        terminal.replace_keyboard_input_context_range(None, "h");

        assert!(!TerminalInputHandler::should_stage_unconfirmed_text_input(
            &terminal,
            Some(&(0..1)),
            "ho",
            false,
        ));
    }

    #[test]
    fn insert_text_without_preflight_preview_continues_pending_stream() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));

        terminal.replace_streamed_text_input_context_range(None, "hey");

        assert!(TerminalInputHandler::should_stage_unconfirmed_text_input(
            &terminal,
            Some(&(0..3)),
            "hey there",
            false,
        ));
    }

    #[test]
    fn text_input_rewrite_guard_blocks_unconfirmed_replay_burst() {
        let terminal = Terminal::new(80, 4, px(10.0), px(20.0));

        assert!(!TerminalInputHandler::should_stage_unconfirmed_text_input(
            &terminal, None, "d", true
        ));
        assert!(!TerminalInputHandler::should_stage_unconfirmed_text_input(
            &terminal, None, "dúng", true
        ));
    }

    #[test]
    fn insert_text_without_preflight_preview_follows_active_dictation_lifecycle() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));

        terminal.begin_dictation();

        assert!(TerminalInputHandler::should_stage_unconfirmed_text_input(
            &terminal,
            Some(&(1..1)),
            "I",
            true,
        ));
    }

    #[test]
    fn text_input_preflight_rewrite_state_stays_terminal_owned() {
        assert!(
            !TerminalInputHandler::should_keep_text_input_rewrite_active(true, true, false),
            "ordinary key insert should not leave an IME rewrite open"
        );
        assert!(
            TerminalInputHandler::should_keep_text_input_rewrite_active(true, false, false),
            "validated Telex rewrite can insert different text than shouldChangeText"
        );
        assert!(
            TerminalInputHandler::should_keep_text_input_rewrite_active(false, false, true),
            "multi-step IME rewrite remains confirmed until unmarkText"
        );
    }

    #[test]
    fn delete_preflight_without_replacement_clears_terminal_rewrite_state() {
        let plain_delete = TextInputPreflight {
            text: String::new(),
        };
        let rewrite_delete = TextInputPreflight {
            text: "o".to_string(),
        };

        assert!(
            TerminalInputHandler::should_clear_text_input_rewrite_for_delete(Some(&plain_delete))
        );
        assert!(
            !TerminalInputHandler::should_mark_text_input_rewrite_active_for_delete(
                Some(&plain_delete),
                true
            )
        );
        assert!(
            !TerminalInputHandler::should_clear_text_input_rewrite_for_delete(Some(
                &rewrite_delete
            ))
        );
        assert!(
            TerminalInputHandler::should_mark_text_input_rewrite_active_for_delete(
                Some(&rewrite_delete),
                false
            )
        );
    }
}
