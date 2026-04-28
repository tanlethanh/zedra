use std::borrow::Cow;
use std::cmp::min;
use std::ops::{Index, Range};
use std::path::{Path, PathBuf};
use tracing::*;

use alacritty_terminal::event::{Event as AlacTermEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Config;
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, Processor};
use gpui::{Context, Keystroke, Pixels, ScrollDelta, ScrollWheelEvent, Task, px};
use tokio::sync::{broadcast, mpsc};
use zedra_osc::{OscEvent, OscScanner};

const REMOTE_TOUCH_SCROLL_STEP_PX: f32 = 12.0;

use crate::keys::to_esc_str;

/// Events emitted by the terminal to observers.
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    RequestResize {
        cols: u16,
        rows: u16,
    },
    TitleChanged(Option<String>),
    OscEvent(OscEvent),
    OpenHyperlink(TerminalHyperlink),
    AltScreenChanged(bool),
    DictationPreviewChanged(Option<String>),
    ScrollbackPositionChanged {
        display_offset: usize,
        history_size: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalHyperlink {
    pub label: String,
    pub target: TerminalHyperlinkTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalHyperlinkTarget {
    Url {
        url: String,
    },
    File {
        path: String,
        relative_path: String,
        line: Option<u32>,
        column: Option<u32>,
    },
}

/// Event listener that captures alacritty title events, queuing them as TerminalEvents.
#[derive(Clone)]
pub struct ZedraListener {
    event_tx: broadcast::Sender<TerminalEvent>,
}

impl ZedraListener {
    fn new(event_tx: broadcast::Sender<TerminalEvent>) -> Self {
        Self { event_tx }
    }
}

impl EventListener for ZedraListener {
    fn send_event(&self, event: AlacTermEvent) {
        let terminal_event = match event {
            AlacTermEvent::Title(t) => TerminalEvent::TitleChanged(Some(t)),
            AlacTermEvent::ResetTitle => TerminalEvent::TitleChanged(None),
            _ => return,
        };
        if let Err(e) = self.event_tx.send(terminal_event) {
            error!("failed to send terminal event: {:?}", e);
        }
    }
}

/// A link detected in plain terminal text (no OSC 8 encoding).
/// `start` and `end` are inclusive alacritty grid points.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectedLink {
    pub start: Point,
    pub end: Point,
    pub text: String,
    pub kind: DetectedLinkKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectedLinkKind {
    Url,
    FilePath,
}

/// Snapshot of terminal grid content for rendering
#[derive(Clone)]
pub struct TerminalContent {
    pub cells: Vec<IndexedCell>,
    pub mode: TermMode,
    pub display_offset: usize,
    pub cursor: CursorState,
    pub cursor_char: char,
    pub grid_rows: usize,
    pub grid_cols: usize,
    pub detected_links: Vec<DetectedLink>,
}

/// A terminal cell with its grid position
#[derive(Clone, Debug)]
pub struct IndexedCell {
    pub point: Point,
    pub cell: Cell,
}

/// Cursor rendering state
#[derive(Clone, Debug)]
pub struct CursorState {
    pub point: Point,
    pub shape: CursorShape,
}

/// Terminal size in cells and pixels
#[derive(Clone, Copy, Debug)]
pub struct TerminalSize {
    pub cell_width: Pixels,
    pub line_height: Pixels,
    pub columns: usize,
    pub rows: usize,
}

/// Simple Dimensions implementation for terminal sizing
struct SimpleDimensions {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for SimpleDimensions {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

#[derive(Default)]
pub struct IMEState {
    /// Synthetic document exposed to native text input APIs.
    pub document_text: String,
    /// Composing/marked text. When `dictation_active` is true this holds the
    /// live hypothesis; otherwise it holds the active IME composition string.
    pub marked_text: String,
    /// UTF-16 range of the marked text inside `document_text`.
    pub marked_range: Option<Range<usize>>,
    /// UTF-16 selection range inside `document_text`.
    pub selected_range: Option<Range<usize>>,
    /// True while a dictation session is in progress.
    pub dictation_active: bool,
    /// True after a dictation hypothesis has been committed while keeping the
    /// synthetic text store available for UIKit's trailing dictation queries.
    pub committed_dictation_pending_cleanup: bool,
}

/// Minimal terminal state wrapping alacritty_terminal::Term
pub struct Terminal {
    term: Term<ZedraListener>,
    /// VTE processor — persisted across advance_bytes calls so that
    /// escape sequences split across network packets are parsed correctly.
    processor: Processor,
    mode: TermMode,
    size: TerminalSize,
    ime_state: Option<IMEState>,
    scanner: OscScanner,
    event_tx: broadcast::Sender<TerminalEvent>,
    input_tx: Option<mpsc::Sender<Vec<u8>>>,
    output_task: Option<Task<()>>,
}

impl Terminal {
    /// Create a new terminal with the given grid dimensions
    pub fn new(columns: usize, rows: usize, cell_width: Pixels, line_height: Pixels) -> Self {
        let (event_tx, _) = broadcast::channel(100);
        let listener = ZedraListener::new(event_tx.clone());
        let config = Config::default();
        let term_size = SimpleDimensions {
            columns,
            screen_lines: rows,
        };
        let term = Term::new(config, &term_size, listener);

        Self {
            term,
            processor: Processor::new(),
            mode: TermMode::empty(),
            size: TerminalSize {
                cell_width,
                line_height,
                columns,
                rows,
            },
            ime_state: None,
            scanner: OscScanner::new(),
            event_tx,
            input_tx: None,
            output_task: None,
        }
    }

    pub fn is_channel_attached(&self) -> bool {
        self.input_tx.is_some() && self.output_task.is_some()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<TerminalEvent> {
        self.event_tx.subscribe()
    }

    /// Attach a channel for input and output bytes to the terminal emulator
    pub fn attach_channel(
        &mut self,
        input_tx: mpsc::Sender<Vec<u8>>,
        mut output_rx: mpsc::Receiver<Vec<u8>>,
        cx: &mut Context<Self>,
    ) {
        self.input_tx = Some(input_tx);

        if let Some(prev_task) = self.output_task.take() {
            info!("drop output task when reattach a new one");
            drop(prev_task);
        }
        let output_task = cx.spawn(async move |this, cx| {
            while let Some(bytes) = output_rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.advance_bytes(&bytes);
                    this.feed_osc_bytes(&bytes);
                    cx.notify();
                });
            }
        });
        self.output_task = Some(output_task);
    }

    pub fn input_sender(&self) -> Option<mpsc::Sender<Vec<u8>>> {
        self.input_tx.clone()
    }

    pub async fn send_bytes(&mut self, bytes: Vec<u8>) {
        if let Some(tx) = &self.input_tx {
            if let Err(e) = tx.send(bytes).await {
                error!("failed to send input: {:?}", e)
            }
        }
    }

    pub async fn send_input(&mut self, text: String) {
        self.send_bytes(text.into_bytes()).await;
    }

    /// Feed bytes from PTY output buffer into the terminal emulator
    pub fn advance_bytes(&mut self, bytes: &[u8]) {
        let was_alt = self.mode.contains(TermMode::ALT_SCREEN);
        let previous_display_offset = self.display_offset();
        self.processor.advance(&mut self.term, bytes);
        self.mode = *self.term.mode();
        let is_alt = self.mode.contains(TermMode::ALT_SCREEN);
        if is_alt != was_alt {
            let _ = self.event_tx.send(TerminalEvent::AltScreenChanged(is_alt));
        }
        self.emit_scrollback_position_if_changed(previous_display_offset);
    }

    /// Feed bytes from PTY output buffer into the OSC scanner
    /// and emit events to the event channel.
    pub fn feed_osc_bytes(&mut self, bytes: &[u8]) {
        let osc_events = self.scanner.feed(bytes);
        if !osc_events.is_empty() {
            for event in osc_events {
                if let Err(e) = self.event_tx.send(TerminalEvent::OscEvent(event)) {
                    error!("failed to send osc event: {:?}", e);
                }
            }
        }
    }

    /// Get a snapshot of the terminal content for rendering
    pub fn content(&self) -> TerminalContent {
        let content = self.term.renderable_content();
        let mut cells = Vec::new();

        for ic in content.display_iter {
            cells.push(IndexedCell {
                point: ic.point,
                cell: ic.cell.clone(),
            });
        }

        let cursor_point = content.cursor.point;
        let cursor_char = self.term.grid()[cursor_point].c;

        let detected_links = self.detect_plain_links();
        TerminalContent {
            cells,
            mode: content.mode,
            display_offset: content.display_offset,
            cursor: CursorState {
                point: cursor_point,
                shape: content.cursor.shape,
            },
            cursor_char,
            grid_rows: self.size.rows,
            grid_cols: self.size.columns,
            detected_links,
        }
    }

    /// Handle a keystroke, converting to escape sequence and sending via SSH or RPC session
    pub fn handle_keystroke(&mut self, keystroke: &Keystroke) {
        // Try to convert keystroke to terminal escape sequence
        if let Some(bytes) = self.try_keystroke(keystroke) {
            self.send_bytes_sync(bytes);
        } else if let Some(ref key_char) = keystroke.key_char {
            // For plain characters, send the character directly
            if !keystroke.modifiers.control
                && !keystroke.modifiers.alt
                && !keystroke.modifiers.platform
            {
                self.send_bytes_sync(key_char.as_bytes().to_vec());
            }
        }
    }

    /// Convert a GPUI keystroke to terminal escape sequence bytes
    pub fn try_keystroke(&self, keystroke: &gpui::Keystroke) -> Option<Vec<u8>> {
        let esc = to_esc_str(keystroke, &self.mode, false);
        esc.map(|s| match s {
            Cow::Borrowed(string) => string.as_bytes().to_vec(),
            Cow::Owned(string) => string.into_bytes(),
        })
    }

    /// Resize the terminal grid
    pub fn resize(&mut self, columns: usize, rows: usize, cell_width: Pixels, line_height: Pixels) {
        self.size = TerminalSize {
            cell_width,
            line_height,
            columns,
            rows,
        };
        let term_size = SimpleDimensions {
            columns,
            screen_lines: rows,
        };
        self.term.resize(term_size);
    }

    /// Get current terminal size
    pub fn size(&self) -> TerminalSize {
        self.size
    }

    /// Get current terminal mode
    pub fn mode(&self) -> TermMode {
        self.mode
    }

    pub fn is_alt_screen(&self) -> bool {
        self.mode.contains(TermMode::ALT_SCREEN)
    }

    fn mouse_mode(&self, event: &ScrollWheelEvent) -> bool {
        self.input_tx.is_some()
            && !event.modifiers.shift
            && self.mode().intersects(TermMode::MOUSE_MODE)
    }

    pub fn should_send_alt_scroll(&self, event: &ScrollWheelEvent) -> bool {
        if self.mouse_mode(event) || self.input_tx.is_none() || event.modifiers.shift {
            return false;
        }
        self.mode
            .contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
    }

    pub fn scroll_step_px(&self, event: &ScrollWheelEvent) -> f32 {
        if self.mouse_mode(event) || self.should_send_alt_scroll(event) {
            REMOTE_TOUCH_SCROLL_STEP_PX
        } else {
            (self.size.line_height / px(1.0)) as f32
        }
    }

    pub fn should_snap_touch_release(&self, event: &ScrollWheelEvent) -> bool {
        !self.mouse_mode(event) && !self.should_send_alt_scroll(event)
    }

    fn scroll_point(
        &self,
        event: &ScrollWheelEvent,
        grid_origin: Option<gpui::Point<Pixels>>,
    ) -> Option<Point> {
        let columns = self.size.columns.max(1);
        let rows = self.size.rows.max(1);

        let (column, line) = if matches!(event.delta, ScrollDelta::Pixels(_)) {
            (columns / 2, (rows / 2) as i32)
        } else {
            let origin = grid_origin?;
            let relative = event.position - origin;
            let x = relative.x.max(px(0.0));
            let y = relative.y.max(px(0.0));
            (
                min(
                    (x / self.size.cell_width) as usize,
                    columns.saturating_sub(1),
                ),
                min((y / self.size.line_height) as usize, rows.saturating_sub(1)) as i32,
            )
        };

        let line = line - self.display_offset() as i32;
        Some(Point::new(Line(line), Column(column)))
    }

    pub fn send_bytes_sync(&self, bytes: Vec<u8>) {
        if let Some(tx) = &self.input_tx {
            if let Err(e) = tx.try_send(bytes) {
                error!("failed to send bytes: {:?}", e);
            }
        }
    }

    /// Scroll the terminal by a number of lines (positive = up)
    pub fn scroll(&mut self, lines: i32) {
        let previous_display_offset = self.display_offset();
        let scroll = Scroll::Delta(lines);
        self.term.scroll_display(scroll);
        self.emit_scrollback_position_if_changed(previous_display_offset);
    }

    pub fn scroll_to_bottom(&mut self) {
        let previous_display_offset = self.display_offset();
        self.term.scroll_display(Scroll::Bottom);
        self.emit_scrollback_position_if_changed(previous_display_offset);
    }

    /// Current display offset (0 = bottom, history_size = top)
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Get total history size
    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    fn emit_scrollback_position_if_changed(&self, previous_display_offset: usize) {
        if self.display_offset() == previous_display_offset {
            return;
        }

        let _ = self
            .event_tx
            .send(TerminalEvent::ScrollbackPositionChanged {
                display_offset: self.display_offset(),
                history_size: self.history_size(),
            });
    }

    fn emit_dictation_preview_changed(&self, text: Option<String>) {
        let _ = self
            .event_tx
            .send(TerminalEvent::DictationPreviewChanged(text));
    }

    // --- IME / Input Composition ---

    fn ime_state_mut(&mut self) -> &mut IMEState {
        self.ime_state.get_or_insert_with(IMEState::default)
    }

    fn utf16_len(text: &str) -> usize {
        text.encode_utf16().count()
    }

    fn byte_offset_from_utf16(text: &str, offset: usize) -> usize {
        let mut utf16_count = 0;
        for (utf8_index, ch) in text.char_indices() {
            if utf16_count >= offset {
                return utf8_index;
            }
            utf16_count += ch.len_utf16();
        }
        text.len()
    }

    fn byte_range_from_utf16(text: &str, range_utf16: &Range<usize>) -> Range<usize> {
        Self::byte_offset_from_utf16(text, range_utf16.start)
            ..Self::byte_offset_from_utf16(text, range_utf16.end)
    }

    fn clamp_utf16_range(text: &str, range: Range<usize>) -> Range<usize> {
        let len = Self::utf16_len(text);
        let start = range.start.min(len);
        let end = range.end.min(len);
        start.min(end)..start.max(end)
    }

    fn replace_utf16_range(
        document: &str,
        range: Range<usize>,
        replacement: &str,
    ) -> (String, Range<usize>) {
        let range = Self::clamp_utf16_range(document, range);
        let byte_range = Self::byte_range_from_utf16(document, &range);
        let mut result = document.to_string();
        result.replace_range(byte_range, replacement);

        let replacement_end = range.start + Self::utf16_len(replacement);
        (result, range.start..replacement_end)
    }

    /// Set the marked/composing text. When dictation is active this is the live hypothesis.
    pub fn set_marked_text(&mut self, text: String) {
        self.replace_marked_text_in_range(None, text, None);
    }

    /// Current marked text, if any.
    pub fn marked_text(&self) -> Option<&str> {
        let text = &self.ime_state.as_ref()?.marked_text;
        if text.is_empty() {
            None
        } else {
            Some(text.as_str())
        }
    }

    /// Synthetic text document exposed to native text input APIs.
    pub fn text_input_document(&self) -> &str {
        match self.ime_state.as_ref() {
            Some(state)
                if state.dictation_active
                    || state.marked_range.is_some()
                    || !state.document_text.is_empty() =>
            {
                state.document_text.as_str()
            }
            _ => " ",
        }
    }

    pub fn text_input_selection_range(&self) -> Range<usize> {
        if let Some(selection) = self
            .ime_state
            .as_ref()
            .and_then(|state| state.selected_range.clone())
        {
            selection
        } else {
            let len = Self::utf16_len(self.text_input_document());
            len..len
        }
    }

    /// Clear the marked text.
    pub fn clear_marked_state(&mut self) {
        if let Some(state) = self.ime_state.as_mut() {
            state.marked_text.clear();
            state.marked_range = None;
            state.selected_range = None;
            state.committed_dictation_pending_cleanup = false;
            if !state.dictation_active {
                state.document_text.clear();
            }
        }
    }

    /// Range (in UTF-16 code units) of the current marked text, if any.
    pub fn marked_text_range(&self) -> Option<Range<usize>> {
        self.ime_state
            .as_ref()
            .and_then(|state| state.marked_range.clone())
    }

    pub fn is_dictation_active(&self) -> bool {
        self.ime_state.as_ref().is_some_and(|s| s.dictation_active)
    }

    pub fn has_committed_dictation_pending_cleanup(&self) -> bool {
        self.ime_state
            .as_ref()
            .is_some_and(|state| state.committed_dictation_pending_cleanup)
    }

    pub fn replace_marked_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: String,
        selected_range: Option<Range<usize>>,
    ) -> bool {
        let dictation_active = {
            let state = self.ime_state_mut();
            if state.dictation_active && state.document_text.is_empty() {
                state.document_text.push(' ');
            }

            let document_len = Self::utf16_len(&state.document_text);
            let replacement_range = replacement_range
                .or_else(|| state.marked_range.clone())
                .or_else(|| state.selected_range.clone())
                .unwrap_or(document_len..document_len);
            let (document_text, inserted_range) =
                Self::replace_utf16_range(&state.document_text, replacement_range, &text);

            state.document_text = document_text;
            state.marked_text = text.clone();
            state.committed_dictation_pending_cleanup = false;
            state.marked_range = if text.is_empty() && !state.dictation_active {
                None
            } else {
                Some(inserted_range.clone())
            };

            let text_len = Self::utf16_len(&text);
            let selected_range = selected_range
                .map(|range| {
                    inserted_range.start + range.start.min(text_len)
                        ..inserted_range.start + range.end.min(text_len)
                })
                .unwrap_or_else(|| inserted_range.end..inserted_range.end);
            state.selected_range = Some(selected_range);
            state.dictation_active
        };

        if dictation_active {
            self.emit_dictation_preview_changed(Some(text));
        }

        dictation_active
    }

    /// Begin a dictation session with a stable marked range in the text store.
    pub fn begin_dictation(&mut self) {
        let state = self.ime_state_mut();
        if state.committed_dictation_pending_cleanup {
            state.document_text.clear();
            state.marked_text.clear();
            state.marked_range = None;
            state.selected_range = None;
            state.committed_dictation_pending_cleanup = false;
        }
        let already_active = state.dictation_active;
        state.dictation_active = true;
        state.committed_dictation_pending_cleanup = false;
        if !already_active {
            if state.document_text.is_empty() {
                state.document_text.push(' ');
            }
            let insertion = state
                .selected_range
                .as_ref()
                .map(|range| range.end)
                .unwrap_or_else(|| Self::utf16_len(&state.document_text));
            state.marked_text.clear();
            state.marked_range = Some(insertion..insertion);
            state.selected_range = Some(insertion..insertion);
            self.emit_dictation_preview_changed(Some(String::new()));
        } else if state.marked_range.is_none() {
            let insertion = state
                .selected_range
                .as_ref()
                .map(|range| range.end)
                .unwrap_or_else(|| Self::utf16_len(&state.document_text));
            state.marked_range = Some(insertion..insertion);
            state.selected_range = Some(insertion..insertion);
        }
    }

    /// Finish dictation and return the current marked text, if any.
    pub fn finish_dictation(&mut self) -> Option<String> {
        let result = if let Some(state) = self.ime_state.as_mut() {
            if !state.dictation_active {
                return None;
            }

            state.dictation_active = false;
            // Critical: UIKit may keep asking markedTextRange/textInRange after
            // dictation finalization while UIDictationController reconciles its
            // last hypothesis. Do not clear the synthetic document or marked
            // range here; the next normal input/cancel/start path owns cleanup.
            // This preserved store is read-only: repeated finish/final-result
            // callbacks must not commit the same hypothesis to the terminal.
            let text = state.marked_text.clone();
            state.committed_dictation_pending_cleanup = !text.is_empty();
            if text.is_empty() { None } else { Some(text) }
        } else {
            None
        };

        self.emit_dictation_preview_changed(None);
        result
    }

    pub fn cancel_dictation(&mut self) {
        if let Some(state) = self.ime_state.as_mut() {
            state.dictation_active = false;
            state.committed_dictation_pending_cleanup = false;
        }
        self.clear_marked_state();
        self.emit_dictation_preview_changed(None);
    }

    /// End dictation, committing the current marked text (hypothesis) to the PTY.
    pub fn end_dictation(&mut self) {
        if let Some(text) = self.finish_dictation() {
            self.send_bytes_sync(text.into_bytes());
        }
    }

    /// Send text directly to the PTY.
    pub fn handle_ime_text(&mut self, text: &str) {
        self.send_bytes_sync(text.as_bytes().to_vec());
    }

    pub fn hyperlink_at(
        &self,
        position: gpui::Point<Pixels>,
        grid_origin: Option<gpui::Point<Pixels>>,
        workdir: Option<&str>,
    ) -> Option<TerminalHyperlink> {
        let point = self.grid_point_at(position, grid_origin)?;
        self.hyperlink_at_point(point, workdir)
    }

    fn grid_point_at(
        &self,
        position: gpui::Point<Pixels>,
        grid_origin: Option<gpui::Point<Pixels>>,
    ) -> Option<Point> {
        let origin = grid_origin?;
        let relative = position - origin;
        if relative.x < px(0.0) || relative.y < px(0.0) {
            return None;
        }

        let columns = self.size.columns.max(1);
        let rows = self.size.rows.max(1);
        let column = min(
            (relative.x / self.size.cell_width) as usize,
            columns.saturating_sub(1),
        );
        let viewport_line = min(
            (relative.y / self.size.line_height) as usize,
            rows.saturating_sub(1),
        ) as i32;
        let line = viewport_line - self.display_offset() as i32;
        Some(Point::new(Line(line), Column(column)))
    }

    fn hyperlink_at_point(&self, point: Point, workdir: Option<&str>) -> Option<TerminalHyperlink> {
        // Allow scrollback (negative lines) — alacritty's grid index supports
        // negative lines down to `topmost_line()`. Reject anything below that
        // to avoid an out-of-bounds index panic.
        let topmost = self.term.grid().topmost_line();
        let bottom = Line(self.size.rows as i32 - 1);
        if point.line < topmost || point.line > bottom {
            return None;
        }

        self.hyperlink_from_osc8(point, workdir)
            .or_else(|| self.plain_hyperlink_at_point(point, workdir))
    }

    fn plain_hyperlink_at_point(
        &self,
        point: Point,
        workdir: Option<&str>,
    ) -> Option<TerminalHyperlink> {
        let links = self.detect_plain_links();
        let link = links.into_iter().find(|l| point_in_link(point, l))?;
        match link.kind {
            DetectedLinkKind::Url => Some(TerminalHyperlink {
                label: link.text.clone(),
                target: TerminalHyperlinkTarget::Url { url: link.text },
            }),
            DetectedLinkKind::FilePath => {
                let (path, line_num, col_num) = Self::split_file_position(&link.text);
                let relative_path = Self::resolve_relative_path(path, workdir)?;
                Some(TerminalHyperlink {
                    label: link.text.clone(),
                    target: TerminalHyperlinkTarget::File {
                        path: path.to_string(),
                        relative_path,
                        line: line_num,
                        column: col_num,
                    },
                })
            }
        }
    }

    /// Hot path: runs once per render frame via `content()`. Keep it lean —
    /// see `tail_looks_like_cut_off_path` for the in-place trick.
    pub fn detect_plain_links(&self) -> Vec<DetectedLink> {
        use alacritty_terminal::term::cell::Flags;

        let display_offset = self.display_offset() as i32;
        let rows = self.size.rows as i32;
        let cols = self.size.columns;
        if rows == 0 || cols == 0 {
            return Vec::new();
        }

        let top = -display_offset;
        let bottom = rows - 1 - display_offset;

        let mut links = Vec::new();
        let mut logical_line: Vec<(Point, char)> = Vec::new();
        // True when the previous physical line ended without WRAPLINE but its
        // last visible char was a path-char — likely a Claude-style word-wrap
        // with hard newline + leading-space indent on the next line.
        let mut continuation_pending = false;

        let mut line_idx = top;
        while line_idx <= bottom {
            let alac_line = Line(line_idx);
            let row = &self.term.grid()[alac_line];

            // WRAPLINE on last cell = soft wrap (alacritty filled the row);
            // absent = hard newline.
            let is_wrapped = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);

            // Cell↔char must stay 1:1: skip wide-char spacers, NUL→space, so
            // match offsets later map back to grid points correctly.
            // Continuation lines drop leading whitespace so a hard-wrap with
            // indent (e.g. Claude `Update(/.../zedra-t\n        erminal/...`)
            // joins seamlessly.
            let mut leading_skipped = !continuation_pending;
            let mut row_cells: Vec<(Point, char)> = Vec::new();
            let mut last_visible: Option<char> = None;
            for col in 0..cols {
                let col_idx = Column(col);
                let cell = &row[col_idx];
                let flags = cell.flags;
                if flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
                    || flags.contains(Flags::WIDE_CHAR_SPACER)
                {
                    continue;
                }
                let ch = match cell.c {
                    '\0' => ' ',
                    c => c,
                };
                if !leading_skipped {
                    if ch.is_whitespace() {
                        continue;
                    }
                    leading_skipped = true;
                }
                row_cells.push((Point::new(alac_line, col_idx), ch));
                if !ch.is_whitespace() {
                    last_visible = Some(ch);
                }
            }

            // Soft-wrapped rows are content-full — never trim. Non-WRAPLINE
            // rows carry padding spaces past visible text; trim so a join to
            // the next line doesn't include a giant gap.
            if !is_wrapped {
                while row_cells
                    .last()
                    .map(|(_, c)| c.is_whitespace())
                    .unwrap_or(false)
                {
                    row_cells.pop();
                }
            }

            logical_line.extend(row_cells);

            // Hard-wrap-with-indent join: only when tail looks cut mid-path.
            // Loosening this (e.g., "ends in path-char") glues legit text like
            // "Referenced file" onto the next line's path.
            let _ = last_visible;
            let cut_off_tail =
                !is_wrapped && line_idx < bottom && tail_looks_like_cut_off_path(&logical_line);
            let join_with_next = is_wrapped || cut_off_tail;

            if !join_with_next {
                detect_links_in_chars(&logical_line, &mut links);
                logical_line.clear();
                continuation_pending = false;
            } else {
                continuation_pending = !is_wrapped;
            }

            line_idx += 1;
        }

        // Flush any trailing logical line (e.g., bottom row had path-like tail).
        if !logical_line.is_empty() {
            detect_links_in_chars(&logical_line, &mut links);
        }

        links
    }

    fn hyperlink_from_osc8(
        &self,
        point: Point,
        workdir: Option<&str>,
    ) -> Option<TerminalHyperlink> {
        let link = self.term.grid().index(point).hyperlink()?;
        let label = self
            .osc8_label_at_point(point)
            .unwrap_or_else(|| link.uri().to_string());
        Self::parse_osc8_uri(link.uri(), label, workdir)
    }

    fn osc8_label_at_point(&self, point: Point) -> Option<String> {
        let target = self.term.grid().index(point).hyperlink()?.clone();
        let line_start = self.term.line_search_left(point);
        let line_end = self.term.line_search_right(point);
        let mut text = String::new();
        let mut in_target_run = false;
        let mut run_contains_point = false;

        for cell in self.term.grid().iter_from(line_start) {
            if cell.point > line_end {
                break;
            }

            let cell_link = cell.hyperlink();
            if cell_link.as_ref() != Some(&target) {
                if in_target_run {
                    if run_contains_point {
                        break;
                    }

                    text.clear();
                    in_target_run = false;
                }
                continue;
            }

            in_target_run = true;
            if cell.point == point {
                run_contains_point = true;
            }

            let flags = cell.flags;
            if flags.contains(alacritty_terminal::term::cell::Flags::LEADING_WIDE_CHAR_SPACER)
                || flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER)
            {
                continue;
            }

            let ch = match cell.c {
                '\t' => ' ',
                c => c,
            };
            text.push(ch);
        }

        if !run_contains_point {
            return None;
        }

        let label = text.trim();
        (!label.is_empty()).then(|| label.to_string())
    }

    fn parse_osc8_uri(
        uri: &str,
        label: String,
        workdir: Option<&str>,
    ) -> Option<TerminalHyperlink> {
        let uri = uri.trim();
        if uri.is_empty() {
            return None;
        }

        if let Some(path) = Self::strip_file_uri(uri) {
            return Self::file_hyperlink_from_osc8(path, label, workdir);
        }

        if Self::looks_like_external_uri(uri) {
            return Some(TerminalHyperlink {
                label,
                target: TerminalHyperlinkTarget::Url {
                    url: uri.to_string(),
                },
            });
        }

        Self::file_hyperlink_from_osc8(uri, label, workdir)
    }

    fn file_hyperlink_from_osc8(
        raw: &str,
        label: String,
        workdir: Option<&str>,
    ) -> Option<TerminalHyperlink> {
        let target = raw.trim();
        if target.is_empty() {
            return None;
        }

        let (path, line, column) = Self::split_file_position(target);
        if path.is_empty() {
            return None;
        }

        let relative_path = Self::resolve_relative_path(path, workdir)?;
        Some(TerminalHyperlink {
            label: if label.is_empty() {
                target.to_string()
            } else {
                label
            },
            target: TerminalHyperlinkTarget::File {
                path: path.to_string(),
                relative_path,
                line,
                column,
            },
        })
    }

    fn strip_file_uri(uri: &str) -> Option<&str> {
        if let Some(path) = uri.strip_prefix("file://") {
            return Some(path.strip_prefix("localhost").unwrap_or(path));
        }

        uri.strip_prefix("file:")
    }

    fn looks_like_windows_drive_path(path: &str) -> bool {
        let bytes = path.as_bytes();
        bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/')
    }

    fn looks_like_external_uri(uri: &str) -> bool {
        if Self::looks_like_windows_drive_path(uri) {
            return false;
        }

        let Some((scheme, _rest)) = uri.split_once(':') else {
            return false;
        };

        let mut chars = scheme.chars();
        let Some(first) = chars.next() else {
            return false;
        };

        first.is_ascii_alphabetic()
            && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
    }

    fn split_file_position(token: &str) -> (&str, Option<u32>, Option<u32>) {
        let mut pieces = token.rsplit(':');
        let last = pieces.next();
        let second = pieces.next();

        let parse_u32 = |value: Option<&str>| value.and_then(|v| v.parse::<u32>().ok());
        let last_num = parse_u32(last);
        let second_num = parse_u32(second);

        if let (Some(column), Some(line), Some(last), Some(second)) =
            (last_num, second_num, last, second)
        {
            let suffix_len = last.len() + second.len() + 2;
            let path_end = token.len().saturating_sub(suffix_len);
            if path_end > 0 {
                return (&token[..path_end], Some(line), Some(column));
            }
        }

        if let (Some(line), Some(last)) = (last_num, last) {
            let suffix_len = last.len() + 1;
            let path_end = token.len().saturating_sub(suffix_len);
            if path_end > 0 {
                return (&token[..path_end], Some(line), None);
            }
        }

        (token, None, None)
    }

    fn resolve_relative_path(path: &str, workdir: Option<&str>) -> Option<String> {
        let path = path.trim();
        if path.is_empty() {
            return None;
        }

        let raw = Path::new(path);
        let candidate = if raw.is_absolute() {
            PathBuf::from(raw)
        } else if let Some(workdir) = workdir {
            Path::new(workdir).join(raw)
        } else {
            PathBuf::from(raw)
        };

        if let Some(workdir) = workdir {
            let workdir_path = Path::new(workdir);
            if let Ok(relative) = candidate.strip_prefix(workdir_path) {
                return Some(relative.to_string_lossy().to_string());
            }
        }

        Some(candidate.to_string_lossy().to_string())
    }

    fn send_mouse_scroll(
        &self,
        lines: i32,
        event: &ScrollWheelEvent,
        grid_origin: Option<gpui::Point<Pixels>>,
    ) -> bool {
        let Some(point) = self.scroll_point(event, grid_origin) else {
            return false;
        };
        let Some(report) = scroll_report_bytes(point, event, self.mode) else {
            return false;
        };

        for _ in 0..lines.unsigned_abs() {
            self.send_bytes_sync(report.clone());
        }

        true
    }

    pub fn commit_scroll_lines(
        &mut self,
        lines: i32,
        event: &ScrollWheelEvent,
        grid_origin: Option<gpui::Point<Pixels>>,
    ) -> bool {
        if lines == 0 {
            return false;
        }

        if self.mouse_mode(event) {
            return self.send_mouse_scroll(lines, event, grid_origin);
        }

        if self.should_send_alt_scroll(event) {
            self.send_bytes_sync(alt_scroll_bytes(lines));
            return true;
        }

        let before = self.display_offset();
        self.scroll(lines);
        self.display_offset() != before
    }
}

fn alt_scroll_bytes(lines: i32) -> Vec<u8> {
    let command = if lines > 0 { b'A' } else { b'B' };
    let mut bytes = Vec::with_capacity(lines.unsigned_abs() as usize * 3);

    for _ in 0..lines.abs() {
        bytes.push(0x1b);
        bytes.push(b'O');
        bytes.push(command);
    }

    bytes
}

fn scroll_report_bytes(point: Point, event: &ScrollWheelEvent, mode: TermMode) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) || point.line < Line(0) {
        return None;
    }

    let mut button = if scroll_is_up(event) { 64 } else { 65 };
    if event.modifiers.shift {
        button += 4;
    }
    if event.modifiers.alt {
        button += 8;
    }
    if event.modifiers.control {
        button += 16;
    }

    if mode.contains(TermMode::SGR_MOUSE) {
        Some(
            format!(
                "\x1b[<{};{};{}M",
                button,
                point.column.0 + 1,
                point.line.0 + 1
            )
            .into_bytes(),
        )
    } else {
        normal_mouse_scroll_report(point, button, mode.contains(TermMode::UTF8_MOUSE))
    }
}

fn scroll_is_up(event: &ScrollWheelEvent) -> bool {
    match event.delta {
        ScrollDelta::Pixels(delta) => delta.y > px(0.0),
        ScrollDelta::Lines(delta) => delta.y > 0.0,
    }
}

fn normal_mouse_scroll_report(point: Point, button: u8, utf8: bool) -> Option<Vec<u8>> {
    let line = point.line.0;
    let column = point.column.0;
    let max_point = if utf8 { 2015usize } else { 223usize };

    if line < 0 || line as usize >= max_point || column >= max_point {
        return None;
    }

    let mut report = vec![b'\x1b', b'[', b'M', 32 + button];

    if utf8 && column >= 95 {
        report.extend(encode_mouse_position(column));
    } else {
        report.push(32 + 1 + column as u8);
    }

    let line = line as usize;
    if utf8 && line >= 95 {
        report.extend(encode_mouse_position(line));
    } else {
        report.push(32 + 1 + line as u8);
    }

    Some(report)
}

fn encode_mouse_position(position: usize) -> [u8; 2] {
    let position = 32 + 1 + position;
    [(0xC0 + position / 64) as u8, (0x80 + (position & 63)) as u8]
}

// ---------------------------------------------------------------------------
// Plain-text link detection
// ---------------------------------------------------------------------------

// Conservative-by-design: paths need `/` AND a known extension; bare filenames
// (`README`, `package.json`) deliberately rejected. Loosening invites false
// positives. URL detection separately requires `http(s)://`.
const FILE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "swift", "kt", "java", "c", "cc",
    "cpp", "cxx", "h", "hh", "hpp", "hxx", "cs", "rb", "lua", "php", "zig", "dart", "ex", "exs",
    "json", "json5", "toml", "yaml", "yml", "xml", "html", "htm", "css", "scss", "sass", "vue",
    "md", "mdx", "rst", "txt", "sh", "bash", "zsh", "fish", "ps1", "bat", "lock", "mod", "sum",
    "env", "ini", "cfg", "conf",
];

/// Returns true if alacritty grid point `p` falls within the inclusive span [link.start, link.end].
pub fn point_in_link(p: Point, link: &DetectedLink) -> bool {
    let start = link.start;
    let end = link.end;
    if p.line < start.line || p.line > end.line {
        return false;
    }
    if p.line == start.line && p.column < start.column {
        return false;
    }
    if p.line == end.line && p.column > end.column {
        return false;
    }
    true
}

fn detect_links_in_chars(cells: &[(Point, char)], links: &mut Vec<DetectedLink>) {
    if cells.is_empty() {
        return;
    }
    let chars: Vec<char> = cells.iter().map(|(_, c)| *c).collect();
    detect_urls_in_chars(&chars, cells, links);
    detect_file_paths_in_chars(&chars, cells, links);
}

fn detect_urls_in_chars(chars: &[char], cells: &[(Point, char)], links: &mut Vec<DetectedLink>) {
    let len = chars.len();
    let mut i = 0;
    while i < len {
        let prefix = if chars_at_match(chars, i, "https://") {
            8
        } else if chars_at_match(chars, i, "http://") {
            7
        } else {
            i += 1;
            continue;
        };

        let start = i;
        let mut end = start + prefix;

        while end < len && !is_url_terminator(chars[end]) {
            end += 1;
        }
        // Strip trailing punctuation
        while end > start + prefix && is_url_trailing_punct(chars[end - 1]) {
            end -= 1;
        }

        if end > start + prefix && end <= cells.len() {
            let text: String = chars[start..end].iter().collect();
            links.push(DetectedLink {
                start: cells[start].0,
                end: cells[end - 1].0,
                text,
                kind: DetectedLinkKind::Url,
            });
        }

        i = end.max(start + prefix);
    }
}

fn detect_file_paths_in_chars(
    chars: &[char],
    cells: &[(Point, char)],
    links: &mut Vec<DetectedLink>,
) {
    // Scan around each '/' outward, terminating at whitespace or surrounding
    // punctuation. This correctly handles paths embedded in tokens like
    // `Update(/Users/.../file.rs)`, `error: src/main.rs`, or `(./foo.rs:1:2)`.
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] != '/' {
            i += 1;
            continue;
        }

        // Walk left to find the path's start.
        let mut start = i;
        while start > 0 && is_path_char(chars[start - 1]) {
            start -= 1;
        }
        // Walk right to find the path's end.
        let mut end = i + 1;
        while end < len && is_path_char(chars[end]) {
            end += 1;
        }
        // Strip ONLY trailing sentence punctuation `.,;!?`. Leading `.` is
        // valid path syntax (`./foo.rs`, `../bar.rs`, `.hidden/file.rs`) and
        // must be preserved.
        while end > start && is_path_trailing_punct(chars[end - 1]) {
            end -= 1;
        }

        let next_i = end.max(i + 1);
        if start >= end {
            i = next_i;
            continue;
        }

        let segment: String = chars[start..end].iter().collect();

        // URLs run first; skip `://` segments here so a URL ending in `.html`
        // isn't double-counted as a file path.
        if segment.contains("://") {
            i = next_i;
            continue;
        }

        // Must still contain a slash after stripping.
        if !segment.contains('/') {
            i = next_i;
            continue;
        }

        // Parse optional :line:col suffix to isolate the actual path.
        let (path_part, _line_num, _col_num) = split_file_position_chars(&segment);
        if path_part.is_empty() {
            i = next_i;
            continue;
        }

        // Must have a known file extension.
        let ext = std::path::Path::new(path_part)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !FILE_EXTENSIONS.contains(&ext) {
            i = next_i;
            continue;
        }

        if start < cells.len() && end - 1 < cells.len() {
            links.push(DetectedLink {
                start: cells[start].0,
                end: cells[end - 1].0,
                text: segment,
                kind: DetectedLinkKind::FilePath,
            });
        }

        i = next_i;
    }
}

fn is_path_char(c: char) -> bool {
    !c.is_whitespace() && !is_surrounding_punct(c)
}

fn chars_at_match(chars: &[char], start: usize, pattern: &str) -> bool {
    let mut pattern_chars = pattern.chars();
    let mut i = start;
    while let Some(pc) = pattern_chars.next() {
        if i >= chars.len() || chars[i] != pc {
            return false;
        }
        i += 1;
    }
    true
}

fn is_url_terminator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | ']' | ')' | '}')
}

fn is_url_trailing_punct(c: char) -> bool {
    matches!(
        c,
        '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '\''
    )
}

fn is_surrounding_punct(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
    )
}

/// Tail token has `/` but no completed extension → likely cut mid-path.
/// Drives the hard-newline+indent join in `detect_plain_links`.
///
/// Hot path: walks the cell slice in place, allocates only the small
/// last-component string. Do NOT regress to stringifying the full logical
/// line (was O(N²) over multi-line cut-off chains).
fn tail_looks_like_cut_off_path(cells: &[(Point, char)]) -> bool {
    let len = cells.len();

    // Walk back over trailing whitespace.
    let mut end = len;
    while end > 0 && cells[end - 1].1.is_whitespace() {
        end -= 1;
    }
    if end == 0 {
        return false;
    }

    // Walk back over the trailing non-whitespace token.
    let mut start = end;
    while start > 0 && !cells[start - 1].1.is_whitespace() {
        start -= 1;
    }

    // Locate the last `/` inside [start, end).
    let mut last_slash: Option<usize> = None;
    for i in start..end {
        if cells[i].1 == '/' {
            last_slash = Some(i);
        }
    }
    let Some(slash_pos) = last_slash else {
        return false;
    };

    // Token ends with `/` — last segment is empty, definitely no extension.
    let comp_start = slash_pos + 1;
    if comp_start >= end {
        return true;
    }

    // Extract just the last path component (small alloc).
    let comp: String = cells[comp_start..end].iter().map(|(_, c)| *c).collect();
    let (path_only, _, _) = split_file_position_chars(&comp);
    let ext = std::path::Path::new(path_only)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    !FILE_EXTENSIONS.contains(&ext)
}

fn is_path_trailing_punct(c: char) -> bool {
    matches!(c, '.' | ',' | ';' | '!' | '?')
}

fn split_file_position_chars(token: &str) -> (&str, Option<u32>, Option<u32>) {
    let mut pieces = token.rsplit(':');
    let last = pieces.next();
    let second = pieces.next();

    let parse_u32 = |value: Option<&str>| value.and_then(|v| v.parse::<u32>().ok());
    let last_num = parse_u32(last);
    let second_num = parse_u32(second);

    if let (Some(column), Some(line), Some(last), Some(second)) =
        (last_num, second_num, last, second)
    {
        let suffix_len = last.len() + second.len() + 2;
        let path_end = token.len().saturating_sub(suffix_len);
        if path_end > 0 {
            return (&token[..path_end], Some(line), Some(column));
        }
    }

    if let (Some(line), Some(last)) = (last_num, last) {
        let suffix_len = last.len() + 1;
        let path_end = token.len().saturating_sub(suffix_len);
        if path_end > 0 {
            return (&token[..path_end], Some(line), None);
        }
    }

    (token, None, None)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use alacritty_terminal::index::{Column, Line, Point};
    use gpui::px;

    use super::{Terminal, TerminalHyperlink, TerminalHyperlinkTarget};

    fn terminal_with_output(output: &[u8]) -> Terminal {
        let mut terminal = Terminal::new(160, 8, px(10.0), px(20.0));
        terminal.advance_bytes(output);
        terminal
    }

    fn terminal_with_history() -> Terminal {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));
        for line in 0..12 {
            terminal.advance_bytes(format!("line {line}\r\n").as_bytes());
        }
        terminal
    }

    fn point_for_substring(line: &str, needle: &str) -> Point {
        let start = line.find(needle).expect("substring should exist");
        let hovered_column = start + needle.len().saturating_sub(1) / 2;
        Point::new(Line(0), Column(hovered_column))
    }

    fn file_target(
        hyperlink: TerminalHyperlink,
    ) -> (String, String, String, Option<u32>, Option<u32>) {
        match hyperlink.target {
            TerminalHyperlinkTarget::File {
                path,
                relative_path,
                line,
                column,
            } => (hyperlink.label, path, relative_path, line, column),
            other => panic!("expected file hyperlink, got {other:?}"),
        }
    }

    fn url_target(hyperlink: TerminalHyperlink) -> (String, String) {
        match hyperlink.target {
            TerminalHyperlinkTarget::Url { url } => (hyperlink.label, url),
            other => panic!("expected url hyperlink, got {other:?}"),
        }
    }

    #[test]
    fn finish_dictation_returns_marked_text_and_preserves_text_input_store() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));
        terminal.begin_dictation();
        terminal.set_marked_text("echo hello".to_string());

        assert_eq!(terminal.finish_dictation(), Some("echo hello".to_string()));
        assert!(!terminal.is_dictation_active());
        assert!(terminal.has_committed_dictation_pending_cleanup());
        assert_eq!(terminal.marked_text(), Some("echo hello"));
        assert_eq!(terminal.text_input_document(), " echo hello");
        assert_eq!(terminal.marked_text_range(), Some(1..11));
    }

    #[test]
    fn finish_dictation_does_not_recommit_preserved_text_input_store() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));
        terminal.begin_dictation();
        terminal.set_marked_text("echo hello".to_string());

        assert_eq!(terminal.finish_dictation(), Some("echo hello".to_string()));
        assert_eq!(terminal.finish_dictation(), None);
        assert!(terminal.has_committed_dictation_pending_cleanup());
        assert_eq!(terminal.marked_text(), Some("echo hello"));
        assert_eq!(terminal.text_input_document(), " echo hello");
        assert_eq!(terminal.marked_text_range(), Some(1..11));
    }

    #[test]
    fn clear_marked_state_removes_committed_dictation_store_after_late_queries() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));
        terminal.begin_dictation();
        terminal.set_marked_text("echo hello".to_string());
        terminal.finish_dictation();

        terminal.clear_marked_state();

        assert_eq!(terminal.marked_text(), None);
        assert_eq!(terminal.marked_text_range(), None);
        assert_eq!(terminal.text_input_document(), " ");
    }

    #[test]
    fn new_dictation_session_does_not_reuse_committed_dictation_store() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));
        terminal.begin_dictation();
        terminal.set_marked_text("echo hello".to_string());
        terminal.finish_dictation();

        terminal.begin_dictation();

        assert!(terminal.is_dictation_active());
        assert_eq!(terminal.text_input_document(), " ");
        assert_eq!(terminal.marked_text(), None);
        assert_eq!(terminal.marked_text_range(), Some(1..1));
        assert_eq!(terminal.text_input_selection_range(), 1..1);
    }

    #[test]
    fn cancel_dictation_clears_text_input_store() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));
        terminal.begin_dictation();
        terminal.set_marked_text("echo hello".to_string());

        terminal.cancel_dictation();

        assert!(!terminal.is_dictation_active());
        assert_eq!(terminal.marked_text(), None);
        assert_eq!(terminal.marked_text_range(), None);
        assert_eq!(terminal.text_input_document(), " ");
    }

    #[test]
    fn finish_dictation_returns_none_for_empty_hypothesis() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));
        terminal.begin_dictation();

        assert_eq!(terminal.finish_dictation(), None);
        assert!(!terminal.is_dictation_active());
    }

    #[test]
    fn dictation_keeps_empty_hypothesis_visible_to_text_input() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));

        assert_eq!(terminal.text_input_document(), " ");
        assert_eq!(terminal.marked_text_range(), None);

        terminal.begin_dictation();

        assert_eq!(terminal.text_input_document(), " ");
        assert_eq!(terminal.marked_text_range(), Some(1..1));
        assert_eq!(terminal.text_input_selection_range(), 1..1);
    }

    #[test]
    fn dictation_marked_range_tracks_real_hypothesis_in_document() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));

        terminal.begin_dictation();
        terminal.set_marked_text("hello".to_string());

        assert_eq!(terminal.text_input_document(), " hello");
        assert_eq!(terminal.marked_text_range(), Some(1..6));
        assert_eq!(terminal.text_input_selection_range(), 6..6);
    }

    #[test]
    fn dictation_replaces_existing_hypothesis_range() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));

        terminal.begin_dictation();
        terminal.replace_marked_text_in_range(Some(1..1), "Hello".to_string(), None);
        assert_eq!(terminal.text_input_document(), " Hello");
        assert_eq!(terminal.marked_text_range(), Some(1..6));

        terminal.replace_marked_text_in_range(Some(1..6), "Hello, how".to_string(), None);
        assert_eq!(terminal.text_input_document(), " Hello, how");
        assert_eq!(terminal.marked_text_range(), Some(1..11));
        assert_eq!(terminal.text_input_selection_range(), 11..11);
    }

    #[test]
    fn repeated_begin_dictation_preserves_live_hypothesis() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));

        terminal.begin_dictation();
        terminal.set_marked_text("hello".to_string());
        terminal.begin_dictation();

        assert_eq!(terminal.text_input_document(), " hello");
        assert_eq!(terminal.marked_text_range(), Some(1..6));
    }

    #[test]
    fn dictation_preview_events_track_live_hypothesis() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));
        let mut events = terminal.subscribe_events();

        terminal.begin_dictation();
        match events.try_recv().expect("expected dictation start event") {
            super::TerminalEvent::DictationPreviewChanged(Some(text)) => assert_eq!(text, ""),
            event => panic!("expected dictation preview start event, got {event:?}"),
        }

        terminal.set_marked_text("echo hello".to_string());
        match events.try_recv().expect("expected dictation update event") {
            super::TerminalEvent::DictationPreviewChanged(Some(text)) => {
                assert_eq!(text, "echo hello");
            }
            event => panic!("expected dictation preview update event, got {event:?}"),
        }

        assert_eq!(terminal.finish_dictation(), Some("echo hello".to_string()));
        match events.try_recv().expect("expected dictation end event") {
            super::TerminalEvent::DictationPreviewChanged(None) => {}
            event => panic!("expected dictation preview end event, got {event:?}"),
        }
    }

    #[test]
    fn detects_plain_file_links_from_grid_point() {
        let line = "Visit src/main.rs:12:3 now";
        let terminal = terminal_with_output(b"Visit src/main.rs:12:3 now\r\n");

        let hyperlink = terminal
            .hyperlink_at_point(point_for_substring(line, "src/main.rs"), Some("/repo"))
            .expect("expected plain file hyperlink");
        let (_label, path, relative_path, line_num, col_num) = file_target(hyperlink);
        assert_eq!(path, "src/main.rs");
        assert_eq!(relative_path, "src/main.rs");
        assert_eq!(line_num, Some(12));
        assert_eq!(col_num, Some(3));
    }

    #[test]
    fn scroll_to_bottom_resets_display_offset_and_emits_position() {
        let mut terminal = terminal_with_history();
        terminal.scroll(5);
        assert!(terminal.display_offset() > 0);

        let mut events = terminal.subscribe_events();
        terminal.scroll_to_bottom();

        assert_eq!(terminal.display_offset(), 0);
        match events
            .try_recv()
            .expect("expected scrollback position event")
        {
            super::TerminalEvent::ScrollbackPositionChanged {
                display_offset,
                history_size,
            } => {
                assert_eq!(display_offset, 0);
                assert!(history_size > 0);
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    fn detects_plain_file_links_stripped_of_surrounding_punctuation() {
        let line = r#"Open ("src/main.rs:12:3") next"#;
        let terminal = terminal_with_output(b"Open (\"src/main.rs:12:3\") next\r\n");

        let hyperlink = terminal
            .hyperlink_at_point(point_for_substring(line, "src/main.rs"), Some("/repo"))
            .expect("expected plain file hyperlink stripped of surrounding punctuation");
        let (_label, path, _relative_path, line_num, col_num) = file_target(hyperlink);
        assert_eq!(path, "src/main.rs");
        assert_eq!(line_num, Some(12));
        assert_eq!(col_num, Some(3));
    }

    #[test]
    fn detects_plain_url_links_from_grid_point() {
        let line = "Visit https://zedra.dev now";
        let terminal = terminal_with_output(b"Visit https://zedra.dev now\r\n");

        let hyperlink = terminal
            .hyperlink_at_point(point_for_substring(line, "zedra.dev"), Some("/repo"))
            .expect("expected plain URL hyperlink");
        let (_label, url) = url_target(hyperlink);
        assert_eq!(url, "https://zedra.dev");
    }

    #[test]
    fn ignores_shell_prompt_tokens_from_grid_point() {
        let line = "git:(refactor-app-session-architecture";
        let terminal = terminal_with_output(b"git:(refactor-app-session-architecture\r\n");

        assert_eq!(
            terminal.hyperlink_at_point(
                point_for_substring(line, "refactor-app-session-architecture"),
                Some("/repo"),
            ),
            None
        );
    }

    #[test]
    fn ignores_version_like_tokens_from_grid_point() {
        let line = "v0.112.0 gpt-5.4 /model";
        let terminal = terminal_with_output(b"v0.112.0 gpt-5.4 /model\r\n");

        assert_eq!(
            terminal.hyperlink_at_point(point_for_substring(line, "v0.112.0"), Some("/repo")),
            None
        );
        assert_eq!(
            terminal.hyperlink_at_point(point_for_substring(line, "gpt-5.4"), Some("/repo")),
            None
        );
        assert_eq!(
            terminal.hyperlink_at_point(point_for_substring(line, "/model"), Some("/repo")),
            None
        );
    }

    #[test]
    fn ignores_readme_from_grid_point() {
        let line = "README";
        let terminal = terminal_with_output(b"README\r\n");

        assert_eq!(
            terminal.hyperlink_at_point(point_for_substring(line, "README"), Some("/repo")),
            None
        );
    }

    #[test]
    fn detects_osc8_url_hyperlinks_from_grid_point() {
        let line = "Visit zedra.dev now";
        let terminal = terminal_with_output(
            b"Visit \x1b]8;;https://zedra.dev\x1b\\zedra.dev\x1b]8;;\x1b\\ now\r\n",
        );

        let hyperlink = terminal
            .hyperlink_at_point(point_for_substring(line, "zedra.dev"), Some("/repo"))
            .expect("expected OSC 8 url hyperlink");
        let (label, url) = url_target(hyperlink);

        assert_eq!(label, "zedra.dev");
        assert_eq!(url, "https://zedra.dev");
    }

    #[test]
    fn detects_osc8_file_hyperlinks_from_grid_point() {
        let line = "Open docs/guide.md now";
        let terminal = terminal_with_output(
            b"Open \x1b]8;;file:///repo/docs/guide.md:12:3\x1b\\docs/guide.md\x1b]8;;\x1b\\ now\r\n",
        );

        let hyperlink = terminal
            .hyperlink_at_point(point_for_substring(line, "docs/guide.md"), Some("/repo"))
            .expect("expected OSC 8 file hyperlink");
        let (_label, path, relative_path, line, column) = file_target(hyperlink);

        assert_eq!(Path::new(&path), Path::new("/repo/docs/guide.md"));
        assert_eq!(Path::new(&relative_path), Path::new("docs/guide.md"));
        assert_eq!(line, Some(12));
        assert_eq!(column, Some(3));
    }

    #[test]
    fn detects_osc8_relative_file_hyperlinks_from_grid_point() {
        let line = "Open source now";
        let terminal = terminal_with_output(
            b"Open \x1b]8;;src/main.rs:12:3\x1b\\source\x1b]8;;\x1b\\ now\r\n",
        );

        let hyperlink = terminal
            .hyperlink_at_point(point_for_substring(line, "source"), Some("/repo"))
            .expect("expected OSC 8 relative file hyperlink");
        let (label, path, relative_path, line, column) = file_target(hyperlink);

        assert_eq!(label, "source");
        assert_eq!(Path::new(&path), Path::new("src/main.rs"));
        assert_eq!(Path::new(&relative_path), Path::new("src/main.rs"));
        assert_eq!(line, Some(12));
        assert_eq!(column, Some(3));
    }

    #[test]
    fn detects_osc8_bare_file_targets_from_grid_point() {
        let line = "Read README now";
        let terminal =
            terminal_with_output(b"Read \x1b]8;;README\x1b\\README\x1b]8;;\x1b\\ now\r\n");

        let hyperlink = terminal
            .hyperlink_at_point(point_for_substring(line, "README"), Some("/repo"))
            .expect("expected OSC 8 bare file hyperlink");
        let (label, path, relative_path, line, column) = file_target(hyperlink);

        assert_eq!(label, "README");
        assert_eq!(Path::new(&path), Path::new("README"));
        assert_eq!(Path::new(&relative_path), Path::new("README"));
        assert_eq!(line, None);
        assert_eq!(column, None);
    }

    // -- Plain link wrap detection ------------------------------------------------

    fn narrow_terminal(cols: usize, rows: usize, output: &[u8]) -> Terminal {
        let mut terminal = Terminal::new(cols, rows, px(10.0), px(20.0));
        terminal.advance_bytes(output);
        terminal
    }

    #[test]
    fn detects_url_wrapped_across_two_lines() {
        // 20-col terminal forces wrap. URL is 35 chars, total line is 41 chars.
        // "Visit " = 6, "https://example.com/very/long/path" = 34 chars. 6+34 = 40 → wraps.
        let terminal = narrow_terminal(20, 8, b"Visit https://example.com/very/long/path now\r\n");

        let links = terminal.detect_plain_links();
        assert_eq!(
            links.len(),
            1,
            "expected exactly one URL link, got {:?}",
            links
        );
        assert_eq!(links[0].kind, super::DetectedLinkKind::Url);
        assert_eq!(links[0].text, "https://example.com/very/long/path");
        // Start point should be on the first physical line at column 6.
        assert_eq!(links[0].start.column.0, 6);
    }

    #[test]
    fn detects_file_path_wrapped_across_two_lines() {
        // 20-col terminal forces wrap. "Edit crates/zedra-host/src/main.rs:42:5 now"
        // "Edit " = 5, path with line:col = 38 chars → wraps after col 19.
        let terminal = narrow_terminal(20, 8, b"Edit crates/zedra-host/src/main.rs:42:5 now\r\n");

        let links = terminal.detect_plain_links();
        assert_eq!(links.len(), 1, "expected one file link, got {:?}", links);
        assert_eq!(links[0].kind, super::DetectedLinkKind::FilePath);
        assert_eq!(links[0].text, "crates/zedra-host/src/main.rs:42:5");
        // The link spans at least two physical lines.
        assert!(
            links[0].end.line > links[0].start.line,
            "expected link to span multiple lines: start={:?} end={:?}",
            links[0].start,
            links[0].end,
        );
    }

    #[test]
    fn hyperlink_at_point_resolves_wrapped_url_from_continuation_line() {
        // 20-col terminal, URL wraps. Tap the continuation line.
        let terminal = narrow_terminal(20, 8, b"Visit https://example.com/very/long/path now\r\n");
        // Column 0 of line 1 is 'g' (continuation of `.../long/...`).
        let point = Point::new(Line(1), Column(0));
        let hyperlink = terminal
            .hyperlink_at_point(point, Some("/repo"))
            .expect("expected URL hyperlink on continuation line");
        let (_label, url) = url_target(hyperlink);
        assert_eq!(url, "https://example.com/very/long/path");
    }

    #[test]
    fn hyperlink_at_point_resolves_wrapped_file_path_from_continuation_line() {
        let terminal = narrow_terminal(20, 8, b"Edit crates/zedra-host/src/main.rs:42:5 now\r\n");
        // Column 5 of line 1 should fall inside the wrapped path.
        let point = Point::new(Line(1), Column(5));
        let hyperlink = terminal
            .hyperlink_at_point(point, Some("/repo"))
            .expect("expected file hyperlink on continuation line");
        let (_label, path, _rel, line_num, col_num) = file_target(hyperlink);
        assert_eq!(path, "crates/zedra-host/src/main.rs");
        assert_eq!(line_num, Some(42));
        assert_eq!(col_num, Some(5));
    }

    #[test]
    fn does_not_detect_bare_filename_without_slash() {
        let terminal = terminal_with_output(b"Read package.json now\r\n");
        let links = terminal.detect_plain_links();
        assert!(
            links.is_empty(),
            "should not detect bare package.json, got {:?}",
            links
        );
    }

    #[test]
    fn does_not_detect_unknown_extension() {
        let terminal = terminal_with_output(b"Read src/foo.xyz now\r\n");
        let links = terminal.detect_plain_links();
        assert!(
            links.is_empty(),
            "should not detect unknown ext, got {:?}",
            links
        );
    }

    #[test]
    fn detects_path_embedded_in_function_call_token() {
        // Claude Code emits `Update(/abs/path/to/file.rs)` — path glued to
        // tool name with `(` and to closing `)` with no whitespace.
        let terminal = terminal_with_output(b"Update(/Users/me/repo/src/main.rs)\r\n");
        let links = terminal.detect_plain_links();
        assert_eq!(links.len(), 1, "expected one path link, got {:?}", links);
        assert_eq!(links[0].kind, super::DetectedLinkKind::FilePath);
        assert_eq!(links[0].text, "/Users/me/repo/src/main.rs");
    }

    #[test]
    fn detects_path_embedded_in_function_call_token_wrapped() {
        // 30-col terminal forces wrap mid-path inside `Update(...)`.
        let terminal = narrow_terminal(30, 8, b"Update(/Users/me/repo/src/terminal.rs)\r\n");
        let links = terminal.detect_plain_links();
        assert_eq!(
            links.len(),
            1,
            "expected one wrapped path link, got {:?}",
            links
        );
        assert_eq!(links[0].text, "/Users/me/repo/src/terminal.rs");
    }

    #[test]
    fn detects_path_in_scrollback_after_scroll_up() {
        // Push enough lines to send the first path into scrollback, then
        // scroll up so it becomes visible at a negative alacritty line.
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));
        terminal.advance_bytes(b"Read(crates/foo/file_a.rs)\r\n");
        for i in 0..20 {
            terminal.advance_bytes(format!("filler line {i}\r\n").as_bytes());
        }
        // Now first link is well above the screen. Scroll up to bring it back.
        terminal.scroll(20);
        assert!(terminal.display_offset() > 0, "should be scrolled");

        let links = terminal.detect_plain_links();
        let recovered = links.iter().find(|l| l.text == "crates/foo/file_a.rs");
        assert!(
            recovered.is_some(),
            "expected to detect scrollback path, got {:?}",
            links,
        );
        let link = recovered.unwrap();
        assert!(
            link.start.line < Line(0),
            "link should be in scrollback (negative line), got {:?}",
            link.start,
        );

        // Tap on that scrollback line should resolve the link.
        let hl = terminal
            .hyperlink_at_point(link.start, Some("/repo"))
            .expect("expected hyperlink at scrollback point");
        let (_l, path, _r, _ln, _c) = file_target(hl);
        assert_eq!(path, "crates/foo/file_a.rs");
    }

    #[test]
    fn detects_multiple_paths_on_separate_lines() {
        let terminal = terminal_with_output(
            b"Read(crates/foo/file1.rs)\r\nUpdate(crates/foo/file2.rs)\r\nWrite(crates/foo/file3.rs)\r\n",
        );
        let links = terminal.detect_plain_links();
        assert_eq!(links.len(), 3, "expected 3 links, got {:?}", links);
        assert_eq!(links[0].text, "crates/foo/file1.rs");
        assert_eq!(links[1].text, "crates/foo/file2.rs");
        assert_eq!(links[2].text, "crates/foo/file3.rs");

        // Each should be tappable from a point on its own line.
        for (i, expected_text) in [
            "crates/foo/file1.rs",
            "crates/foo/file2.rs",
            "crates/foo/file3.rs",
        ]
        .iter()
        .enumerate()
        {
            let line_idx = i as i32;
            let hl = terminal
                .hyperlink_at_point(Point::new(Line(line_idx), Column(10)), Some("/repo"))
                .unwrap_or_else(|| panic!("no link at line {}", line_idx));
            let (_l, path, _r, _ln, _c) = file_target(hl);
            assert_eq!(path, *expected_text, "line {} path mismatch", line_idx);
        }
    }

    #[test]
    fn detects_path_after_label_with_colon() {
        // `error: src/main.rs:12:3` — `error:` precedes path with space.
        let terminal = terminal_with_output(b"error: src/main.rs:12:3\r\n");
        let links = terminal.detect_plain_links();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].text, "src/main.rs:12:3");
    }

    #[test]
    fn detects_path_split_by_hard_newline_with_indent() {
        // Claude Code word-wrap: line 0 ends mid-path with no trailing space,
        // then `\n` followed by indent spaces, then path continues. WRAPLINE
        // is NOT set on line 0 (hard newline). Detection must still work.
        // Output: "Update(/Users/me/repo/src/zedra-t\n        erminal/src/main.rs)"
        let terminal = terminal_with_output(
            b"Update(/Users/me/repo/src/zedra-t\r\n        erminal/src/main.rs)\r\n",
        );
        let links = terminal.detect_plain_links();
        assert_eq!(
            links.len(),
            1,
            "expected one joined path link, got {:?}",
            links,
        );
        assert_eq!(links[0].kind, super::DetectedLinkKind::FilePath);
        assert_eq!(
            links[0].text,
            "/Users/me/repo/src/zedra-terminal/src/main.rs"
        );
        // Span must cross physical lines.
        assert!(links[0].end.line > links[0].start.line);
    }

    #[test]
    fn hyperlink_at_point_resolves_hard_wrapped_path_on_continuation_line() {
        // Tap inside the indented continuation should resolve the full path.
        let terminal = terminal_with_output(
            b"Update(/Users/me/repo/src/zedra-t\r\n        erminal/src/main.rs)\r\n",
        );
        // Line 1, column 12 lands inside `erminal/src/main.rs`.
        let point = Point::new(Line(1), Column(12));
        let hyperlink = terminal
            .hyperlink_at_point(point, Some("/Users/me/repo"))
            .expect("expected file hyperlink on hard-wrapped continuation");
        let (_label, path, _rel, _line, _col) = file_target(hyperlink);
        assert_eq!(path, "/Users/me/repo/src/zedra-terminal/src/main.rs");
    }

    fn cells_from(s: &str) -> Vec<(super::Point, char)> {
        s.chars()
            .enumerate()
            .map(|(i, c)| (super::Point::new(super::Line(0), super::Column(i)), c))
            .collect()
    }

    #[test]
    fn tail_cut_off_detects_incomplete_path() {
        // Path cut mid-segment, no extension yet.
        let cells = cells_from("Update(/Users/me/repo/src/zedra-t");
        assert!(super::tail_looks_like_cut_off_path(&cells));
    }

    #[test]
    fn tail_cut_off_detects_path_ending_with_slash() {
        // Token ends with `/` — last component is empty → cut off.
        let cells = cells_from("see crates/zedra/src/");
        assert!(super::tail_looks_like_cut_off_path(&cells));
    }

    #[test]
    fn tail_cut_off_rejects_completed_path() {
        let cells = cells_from("see crates/zedra/src/main.rs");
        assert!(!super::tail_looks_like_cut_off_path(&cells));
    }

    #[test]
    fn tail_cut_off_rejects_completed_path_with_line_col() {
        let cells = cells_from("see crates/zedra/src/main.rs:42:5");
        assert!(!super::tail_looks_like_cut_off_path(&cells));
    }

    #[test]
    fn tail_cut_off_rejects_token_without_slash() {
        let cells = cells_from("Referenced file");
        assert!(!super::tail_looks_like_cut_off_path(&cells));
    }

    #[test]
    fn tail_cut_off_rejects_empty_input() {
        let cells: Vec<(super::Point, char)> = Vec::new();
        assert!(!super::tail_looks_like_cut_off_path(&cells));
    }

    #[test]
    fn tail_cut_off_rejects_all_whitespace() {
        let cells = cells_from("       ");
        assert!(!super::tail_looks_like_cut_off_path(&cells));
    }

    #[test]
    fn tail_cut_off_ignores_text_before_last_token() {
        // Earlier tokens with completed paths shouldn't matter — only the tail.
        let cells = cells_from("ok foo/done.rs then bar/wip-");
        assert!(super::tail_looks_like_cut_off_path(&cells));
    }

    #[test]
    fn tail_cut_off_handles_trailing_whitespace() {
        // Trailing spaces shouldn't fool the detector — it walks past them.
        let cells = cells_from("Update(/Users/repo/zedra-t   ");
        assert!(super::tail_looks_like_cut_off_path(&cells));
    }

    #[test]
    fn detects_dot_slash_relative_path() {
        // `./scripts/run-ios.sh` — leading `.` must not be stripped.
        let terminal = terminal_with_output(b"Run ./scripts/run-ios.sh now\r\n");
        let links = terminal.detect_plain_links();
        assert_eq!(links.len(), 1, "expected one link, got {:?}", links);
        assert_eq!(links[0].text, "./scripts/run-ios.sh");
    }

    #[test]
    fn detects_dot_dot_slash_relative_path() {
        let terminal = terminal_with_output(b"see ../parent/file.rs now\r\n");
        let links = terminal.detect_plain_links();
        assert_eq!(links.len(), 1, "got {:?}", links);
        assert_eq!(links[0].text, "../parent/file.rs");
    }

    #[test]
    fn does_not_glue_referenced_file_label_to_indented_path() {
        // Pattern: "-> Referenced file\n   crates/foo/file.rs" should detect
        // the path on the second line WITHOUT eating the leading whitespace
        // and gluing "file" to "crates".
        let terminal = terminal_with_output(
            b"-> Referenced file\r\n   crates/zedra-terminal/src/element.rs\r\n",
        );
        let links = terminal.detect_plain_links();
        assert_eq!(links.len(), 1, "got {:?}", links);
        assert_eq!(links[0].text, "crates/zedra-terminal/src/element.rs");
    }

    #[test]
    fn detects_each_referenced_file_in_repeated_block() {
        // Verifies the full pattern the user reported: 3+ "Referenced file\n
        // <indented path>" entries each get their own link, not just the last.
        let terminal = terminal_with_output(
            b"-> Referenced file\r\n   crates/foo/a.rs\r\n\
              -> Referenced file\r\n   crates/foo/b.rs\r\n\
              -> Referenced file\r\n   crates/foo/c.rs\r\n",
        );
        let links = terminal.detect_plain_links();
        let texts: Vec<_> = links.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["crates/foo/a.rs", "crates/foo/b.rs", "crates/foo/c.rs"],
            "got {:?}",
            links,
        );
    }

    #[test]
    fn does_not_redetect_url_as_file_path() {
        // URL with .html extension shouldn't be double-counted as file path.
        let terminal = terminal_with_output(b"Visit https://example.com/page.html now\r\n");
        let links = terminal.detect_plain_links();
        assert_eq!(links.len(), 1, "expected only URL match, got {:?}", links);
        assert_eq!(links[0].kind, super::DetectedLinkKind::Url);
    }
}
