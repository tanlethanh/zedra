use std::borrow::Cow;
use std::cmp::min;
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

const REMOTE_TOUCH_SCROLL_STEP_PX: f32 = 12.0;

use crate::keys::to_esc_str;
use crate::osc::{OscEvent, OscScanner};

/// Events emitted by the terminal to observers.
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    RequestResize { cols: u16, rows: u16 },
    TitleChanged(Option<String>),
    OscEvent(OscEvent),
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
    /// Composing/marked text. When `dictation_active` is true this holds the
    /// live hypothesis; otherwise it holds the active IME composition string.
    pub marked_text: String,
    /// True while a dictation session is in progress.
    pub dictation_active: bool,
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
        self.processor.advance(&mut self.term, bytes);
        self.mode = *self.term.mode();
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
        let scroll = Scroll::Delta(lines);
        self.term.scroll_display(scroll);
    }

    /// Current display offset (0 = bottom, history_size = top)
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Get total history size
    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    // --- IME / Input Composition ---

    fn ime_state_mut(&mut self) -> &mut IMEState {
        self.ime_state.get_or_insert_with(IMEState::default)
    }

    /// Set the marked/composing text. When dictation is active this is the live hypothesis.
    pub fn set_marked_text(&mut self, text: String) {
        self.ime_state_mut().marked_text = text;
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

    /// Clear the marked text.
    pub fn clear_marked_state(&mut self) {
        if let Some(state) = self.ime_state.as_mut() {
            state.marked_text.clear();
        }
    }

    /// Range (in UTF-16 code units) of the current marked text, if any.
    pub fn marked_text_range(&self) -> Option<std::ops::Range<usize>> {
        let state = self.ime_state.as_ref()?;
        if state.marked_text.is_empty() {
            return None;
        }
        let len = state.marked_text.encode_utf16().count();
        Some(0..len)
    }

    pub fn is_dictation_active(&self) -> bool {
        self.ime_state.as_ref().is_some_and(|s| s.dictation_active)
    }

    /// Begin a dictation session. Clears marked text.
    pub fn begin_dictation(&mut self) {
        let state = self.ime_state_mut();
        state.dictation_active = true;
        state.marked_text.clear();
    }

    /// End dictation, committing the current marked text (hypothesis) to the PTY.
    pub fn end_dictation(&mut self) {
        let bytes = if let Some(state) = self.ime_state.as_mut() {
            state.dictation_active = false;
            let text = std::mem::take(&mut state.marked_text);
            if text.is_empty() {
                None
            } else {
                Some(text.into_bytes())
            }
        } else {
            None
        };
        if let Some(bytes) = bytes {
            self.send_bytes_sync(bytes);
        }
    }

    /// Send text directly to the PTY.
    pub fn handle_ime_text(&mut self, text: &str) {
        self.send_bytes_sync(text.as_bytes().to_vec());
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
