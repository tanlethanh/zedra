use std::borrow::Cow;
use std::cmp::min;
use std::ops::Index;
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
    RequestResize { cols: u16, rows: u16 },
    TitleChanged(Option<String>),
    OscEvent(OscEvent),
    OpenHyperlink(TerminalHyperlink),
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
        if point.line < Line(0) {
            return None;
        }

        if let Some(hyperlink) = self.hyperlink_from_osc8(point, workdir) {
            return Some(hyperlink);
        }

        let token = self.token_at_point(point)?;
        Self::parse_terminal_link(&token, workdir)
    }

    fn hyperlink_from_osc8(
        &self,
        point: Point,
        workdir: Option<&str>,
    ) -> Option<TerminalHyperlink> {
        let link = self.term.grid().index(point).hyperlink()?;
        let url = link.uri().to_owned();
        let label = self.bounds_word(point)?;
        if Self::looks_like_url(&url) {
            return Some(TerminalHyperlink {
                label,
                target: TerminalHyperlinkTarget::Url { url },
            });
        }

        // Strip file:// so the absolute path is handled correctly by resolve_path.
        let raw = url.strip_prefix("file://").unwrap_or(&url);
        Self::parse_terminal_link(raw, workdir).map(|mut hyperlink| {
            if hyperlink.label.is_empty() {
                hyperlink.label = label;
            }
            hyperlink
        })
    }

    fn token_at_point(&self, point: Point) -> Option<String> {
        let line = self.bounds_word(point)?;
        let trimmed = Self::trim_token(&line)?;
        Some(trimmed.to_string())
    }

    fn bounds_word(&self, point: Point) -> Option<String> {
        let line_start = self.term.line_search_left(point);
        let line_end = self.term.line_search_right(point);
        let mut text = String::new();
        let mut hovered_offset = None;
        let mut prev_len = 0usize;

        for cell in self.term.grid().iter_from(line_start) {
            if cell.point > line_end {
                break;
            }

            let flags = cell.flags;
            if flags.contains(alacritty_terminal::term::cell::Flags::LEADING_WIDE_CHAR_SPACER)
                || flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER)
            {
                if cell.point == point {
                    hovered_offset = Some(prev_len);
                }
                continue;
            }

            prev_len = text.len();
            let ch = match cell.c {
                '\t' => ' ',
                c => c,
            };
            text.push(ch);

            if cell.point == point {
                hovered_offset = Some(prev_len);
            }
        }

        let hovered_offset = hovered_offset?;
        if hovered_offset >= text.len() {
            return None;
        }

        let bytes = text.as_bytes();
        let mut start = hovered_offset;
        while start > 0 && !bytes[start - 1].is_ascii_whitespace() {
            start -= 1;
        }
        let mut end = hovered_offset;
        while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
            end += 1;
        }
        if start >= end {
            return None;
        }
        Some(text[start..end].to_string())
    }

    pub(crate) fn parse_terminal_link(
        raw: &str,
        workdir: Option<&str>,
    ) -> Option<TerminalHyperlink> {
        let token = Self::trim_token(raw)?;
        if token.is_empty() {
            return None;
        }

        if Self::looks_like_url(token) {
            return Some(TerminalHyperlink {
                label: token.to_string(),
                target: TerminalHyperlinkTarget::Url {
                    url: token.to_string(),
                },
            });
        }

        let (path, line, column) = Self::split_file_position(token);
        if !Self::looks_like_file_path(path) {
            return None;
        }
        let relative_path = Self::resolve_relative_path(path, workdir)?;
        Some(TerminalHyperlink {
            label: token.to_string(),
            target: TerminalHyperlinkTarget::File {
                path: path.to_string(),
                relative_path,
                line,
                column,
            },
        })
    }

    fn looks_like_url(token: &str) -> bool {
        token.starts_with("http://") || token.starts_with("https://")
    }

    fn looks_like_file_path(path: &str) -> bool {
        let path = path.trim();
        if path.is_empty() || Self::looks_like_url(path) {
            return false;
        }

        if path.contains(':') && !Self::looks_like_windows_drive_path(path) {
            return false;
        }

        if path.starts_with("./") || path.starts_with("../") || path.starts_with("~/") {
            return true;
        }

        if path.starts_with('/') {
            return Self::looks_like_absolute_path(path);
        }

        if path.contains('/') || path.contains('\\') {
            return true;
        }

        let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        if file_name.starts_with('.') && file_name.len() > 1 {
            return true;
        }

        if Self::has_file_like_extension(file_name) {
            return true;
        }

        Self::is_common_bare_filename(file_name)
    }

    fn looks_like_absolute_path(path: &str) -> bool {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return false;
        }

        if path.contains('/') || path.contains('\\') {
            return true;
        }

        let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        (file_name.starts_with('.') && file_name.len() > 1)
            || Self::has_file_like_extension(file_name)
            || Self::is_common_bare_filename(file_name)
    }

    fn has_file_like_extension(file_name: &str) -> bool {
        let Some((stem, extension)) = file_name.rsplit_once('.') else {
            return false;
        };

        !stem.is_empty()
            && !extension.is_empty()
            && extension.chars().any(|ch| ch.is_ascii_alphabetic())
    }

    fn looks_like_windows_drive_path(path: &str) -> bool {
        let bytes = path.as_bytes();
        bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/')
    }

    fn is_common_bare_filename(file_name: &str) -> bool {
        file_name.eq_ignore_ascii_case("license")
            || file_name.eq_ignore_ascii_case("licence")
            || matches!(
                file_name,
                "Makefile"
                    | "Dockerfile"
                    | "Gemfile"
                    | "Procfile"
                    | "Podfile"
                    | "Rakefile"
                    | "Brewfile"
                    | "Justfile"
                    | "justfile"
                    | "Tiltfile"
                    | "Vagrantfile"
            )
    }

    pub(crate) fn trim_token(token: &str) -> Option<&str> {
        let trimmed = token.trim_matches(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
                )
        });
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.trim_end_matches(['.', ':']))
        }
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
    fn detects_plain_file_links_from_grid_point() {
        let line = "Visit src/main.rs:12:3 now";
        let terminal = terminal_with_output(b"Visit src/main.rs:12:3 now\r\n");

        let hyperlink = terminal
            .hyperlink_at_point(point_for_substring(line, "src/main.rs"), Some("/repo"))
            .expect("expected file hyperlink");
        let (label, path, relative_path, line, column) = file_target(hyperlink);

        assert_eq!(label, "src/main.rs:12:3");
        assert_eq!(Path::new(&path), Path::new("src/main.rs"));
        assert_eq!(Path::new(&relative_path), Path::new("src/main.rs"));
        assert_eq!(line, Some(12));
        assert_eq!(column, Some(3));
    }

    #[test]
    fn trims_wrapping_punctuation_for_plain_file_links() {
        let line = r#"Open ("src/main.rs:12:3") next"#;
        let terminal = terminal_with_output(b"Open (\"src/main.rs:12:3\") next\r\n");

        let hyperlink = terminal
            .hyperlink_at_point(point_for_substring(line, "src/main.rs"), Some("/repo"))
            .expect("expected wrapped file hyperlink");
        let (label, path, relative_path, line, column) = file_target(hyperlink);

        assert_eq!(label, "src/main.rs:12:3");
        assert_eq!(Path::new(&path), Path::new("src/main.rs"));
        assert_eq!(Path::new(&relative_path), Path::new("src/main.rs"));
        assert_eq!(line, Some(12));
        assert_eq!(column, Some(3));
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
            b"Open \x1b]8;;file:///repo/docs/guide.md\x1b\\docs/guide.md\x1b]8;;\x1b\\ now\r\n",
        );

        let hyperlink = terminal
            .hyperlink_at_point(point_for_substring(line, "docs/guide.md"), Some("/repo"))
            .expect("expected OSC 8 file hyperlink");
        let (_label, path, relative_path, line, column) = file_target(hyperlink);

        assert_eq!(Path::new(&path), Path::new("/repo/docs/guide.md"));
        assert_eq!(Path::new(&relative_path), Path::new("docs/guide.md"));
        assert_eq!(line, None);
        assert_eq!(column, None);
    }

    #[test]
    fn ignores_shell_prompt_branch_tokens() {
        assert_eq!(
            Terminal::parse_terminal_link("git:(refactor-app-session-architecture", Some("/repo")),
            None
        );
        assert_eq!(Terminal::parse_terminal_link("hello", Some("/repo")), None);
    }

    #[test]
    fn ignores_version_like_tokens() {
        assert_eq!(
            Terminal::parse_terminal_link("v0.112.0", Some("/repo")),
            None
        );
        assert_eq!(
            Terminal::parse_terminal_link("gpt-5.4", Some("/repo")),
            None
        );
        assert_eq!(Terminal::parse_terminal_link("/model", Some("/repo")), None);
    }

    #[test]
    fn ignores_bare_readme() {
        assert_eq!(Terminal::parse_terminal_link("README", Some("/repo")), None);
    }

    #[test]
    fn parses_relative_file_links() {
        let hyperlink = Terminal::parse_terminal_link("src/main.rs:12:3", Some("/repo"))
            .expect("expected file hyperlink");

        assert_eq!(hyperlink.label, "src/main.rs:12:3");
        assert_eq!(
            hyperlink.target,
            TerminalHyperlinkTarget::File {
                path: "src/main.rs".into(),
                relative_path: "src/main.rs".into(),
                line: Some(12),
                column: Some(3),
            }
        );
    }

    #[test]
    fn parses_common_bare_filenames() {
        let hyperlink =
            Terminal::parse_terminal_link("Makefile:8", Some("/repo")).expect("expected Makefile");

        assert_eq!(
            hyperlink.target,
            TerminalHyperlinkTarget::File {
                path: "Makefile".into(),
                relative_path: "Makefile".into(),
                line: Some(8),
                column: None,
            }
        );
    }

    #[test]
    fn preserves_url_links() {
        let hyperlink = Terminal::parse_terminal_link("https://zedra.dev", Some("/repo"))
            .expect("expected url hyperlink");

        assert_eq!(
            hyperlink.target,
            TerminalHyperlinkTarget::Url {
                url: "https://zedra.dev".into(),
            }
        );
    }
}
