use gpui::prelude::FluentBuilder;
use gpui::*;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use crate::editor::merge_highlights;
use crate::fonts;
use crate::native_presentation;
use crate::platform_bridge;
use crate::theme;
use crate::workspace_action::AddSelectionToChat;

// Enough offscreen content to keep fast mobile scrolls smooth without
// measuring the full markdown document.
const MARKDOWN_LIST_OVERDRAW_PX: f32 = 1200.0;
const MARKDOWN_BOTTOM_INSET_MIN: f32 = 100.0;
const MARKDOWN_LINK_HIT_SLOP: f32 = 8.0;
const CODE_BLOCK_FONT_SIZE: f32 = theme::EDITOR_FONT_SIZE;
const CODE_BLOCK_LINE_HEIGHT: f32 = theme::EDITOR_LINE_HEIGHT;
const CODE_BLOCK_CHAR_WIDTH_FACTOR: f32 = 0.6;
const CODE_BLOCK_TAB_WIDTH: usize = 4;
const CODE_BLOCK_PADDING_X: f32 = theme::SPACING_SM;
const CODE_BLOCK_PADDING_Y: f32 = 6.0;
const TABLE_CELL_MIN_WIDTH: f32 = 80.0;
const TABLE_CELL_MAX_WIDTH: f32 = 220.0;
const TABLE_CELL_CHAR_WIDTH_FACTOR: f32 = 0.62;
const TABLE_CELL_PADDING_X: f32 = CODE_BLOCK_PADDING_X;
const TABLE_CELL_PADDING_Y: f32 = CODE_BLOCK_PADDING_Y;
pub const MARKDOWN_SELECTION_AREA_ID: &str = "markdown-preview-selection";

pub struct MarkdownView {
    document: MarkdownDocument,
    list_state: ListState,
    track_sheet_scroll_boundary: bool,
    focus_handle: Option<FocusHandle>,
}

impl MarkdownView {
    pub fn new(source: impl Into<SharedString>) -> Self {
        let source = source.into();
        let document = parse_document(source.as_ref());
        let list_state = ListState::new(
            markdown_list_item_count(document.blocks.len()),
            ListAlignment::Top,
            px(MARKDOWN_LIST_OVERDRAW_PX),
        );
        Self {
            document,
            list_state,
            track_sheet_scroll_boundary: false,
            focus_handle: None,
        }
    }

    pub fn new_for_sheet(source: impl Into<SharedString>) -> Self {
        let mut this = Self::new(source);
        this.track_sheet_scroll_boundary = true;
        this.list_state.set_scroll_handler(|event, _window, _cx| {
            native_presentation::set_sheet_content_at_top(!event.is_scrolled);
        });
        this
    }

    pub fn set_source(&mut self, source: impl Into<SharedString>) {
        let source = source.into();
        self.replace_document(parse_document(source.as_ref()));
    }

    pub fn line_range_for_selection(&self, range_utf16: Range<usize>) -> Option<(u32, u32)> {
        self.document.line_range_for_selection(range_utf16)
    }

    pub(crate) fn set_parsed_source(&mut self, parsed: ParsedMarkdownSource) {
        self.replace_document(parsed.document);
    }

    fn replace_document(&mut self, document: MarkdownDocument) {
        self.document = document;
        // ListState caches row measurements. Reset it whenever the parsed block
        // tree changes, or scroll position and item heights can be reused from
        // the previous file.
        self.list_state
            .reset(markdown_list_item_count(self.document.blocks.len()));
    }
}

fn markdown_list_item_count(block_count: usize) -> usize {
    block_count + 1
}

pub(crate) struct ParsedMarkdownSource {
    document: MarkdownDocument,
}

pub(crate) fn parse_markdown_source(source: String) -> ParsedMarkdownSource {
    ParsedMarkdownSource {
        document: parse_document(&source),
    }
}

pub fn is_markdown_path(path: &str) -> bool {
    let path = Path::new(path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();

    let is_markdown_extension = matches!(
        extension.to_ascii_lowercase().as_str(),
        "md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdtxt"
    );
    let is_bare_readme = extension.is_empty() && file_name.eq_ignore_ascii_case("readme");

    is_markdown_extension || is_bare_readme
}

#[derive(Clone, Debug)]
struct MarkdownDocument {
    source: Arc<str>,
    blocks: Arc<[Block]>,
    selection_map: Arc<[MarkdownSelectionSegment]>,
}

impl MarkdownDocument {
    fn line_range_for_selection(&self, range_utf16: Range<usize>) -> Option<(u32, u32)> {
        line_range_for_selection_map(&self.source, &self.selection_map, range_utf16)
    }

    #[cfg(test)]
    fn total_block_count(&self) -> usize {
        self.blocks.iter().map(Block::total_block_count).sum()
    }
}

impl Default for MarkdownDocument {
    fn default() -> Self {
        Self {
            source: Arc::from(""),
            blocks: Vec::new().into(),
            selection_map: Vec::new().into(),
        }
    }
}

#[derive(Clone, Debug)]
struct MarkdownSelectionSegment {
    len_utf16: usize,
    source_range: Range<usize>,
}

#[derive(Clone, Debug)]
enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: HeadingLevel,
        content: Vec<Inline>,
    },
    BlockQuote(Vec<Block>),
    List {
        ordered: bool,
        start: usize,
        items: Vec<Vec<Block>>,
    },
    CodeBlock {
        text: String,
    },
    Table(TableBlock),
    Html(String),
    Rule,
}

impl Block {
    #[cfg(test)]
    fn total_block_count(&self) -> usize {
        1 + match self {
            Block::BlockQuote(children) => children.iter().map(Block::total_block_count).sum(),
            Block::List { items, .. } => items
                .iter()
                .flat_map(|item| item.iter())
                .map(Block::total_block_count)
                .sum(),
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct TableBlock {
    headers: Vec<Vec<Inline>>,
    rows: Vec<Vec<Vec<Inline>>>,
}

#[derive(Clone, Debug)]
enum Inline {
    Text(String),
    Code(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Link { url: String, content: Vec<Inline> },
    Html(String),
    SoftBreak,
    HardBreak,
    TaskMarker(bool),
}

#[derive(Clone, Debug)]
struct StyledRun {
    range: std::ops::Range<usize>,
    style: HighlightStyle,
}

#[derive(Clone, Debug)]
struct LinkRun {
    range: std::ops::Range<usize>,
    url: String,
}

#[derive(Clone, Debug, Default)]
struct InlineRenderBuffer {
    text: String,
    highlights: Vec<StyledRun>,
    links: Vec<LinkRun>,
}

impl InlineRenderBuffer {
    fn push_text(&mut self, text: &str) -> std::ops::Range<usize> {
        let start = self.text.len();
        self.text.push_str(text);
        start..self.text.len()
    }
}

impl Render for MarkdownView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.track_sheet_scroll_boundary {
            update_sheet_scroll_boundary(&self.list_state);
        }

        let blocks = Arc::clone(&self.document.blocks);
        let block_count = blocks.len();
        let bottom_inset = f32::max(
            platform_bridge::home_indicator_inset(),
            MARKDOWN_BOTTOM_INSET_MIN,
        );
        let focus_handle = self
            .focus_handle
            .get_or_insert_with(|| _cx.focus_handle())
            .clone();
        let press_focus_handle = focus_handle.clone();
        // Keep each top-level markdown block as one variable-height list row.
        // Eagerly collecting every block here makes scrolling large READMEs
        // rebuild the whole document on every frame.
        let markdown_list = list(self.list_state.clone(), move |ix, window, cx| {
            if let Some(block) = blocks.get(ix) {
                div()
                    .w_full()
                    .px(px(theme::SPACING_LG))
                    .when(ix == 0, |this| this.pt(px(theme::SPACING_LG)))
                    .pb(if ix + 1 == blocks.len() {
                        px(theme::SPACING_LG)
                    } else {
                        px(theme::SPACING_MD)
                    })
                    .child(render_block(block, format!("md-{ix}"), window, cx))
                    .into_any_element()
            } else if ix == block_count {
                div().h(px(bottom_inset)).into_any_element()
            } else {
                Empty.into_any_element()
            }
        })
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .size_full();

        div()
            .id("markdown-preview-scroll")
            .size_full()
            .min_h_0()
            // Empty markdown taps should move focus and dismiss read-only selection.
            .track_focus(&focus_handle)
            .on_press(move |event, window, cx| {
                if event.completed() && window.active_read_only_selection().is_some() {
                    window.blur();
                    press_focus_handle.focus(window, cx);
                }
            })
            .child(
                selection_area(markdown_list)
                    .id(MARKDOWN_SELECTION_AREA_ID)
                    .action("Add to Chat", AddSelectionToChat)
                    .into_any_element(),
            )
    }
}

fn update_sheet_scroll_boundary(list_state: &ListState) {
    let scroll_top = list_state.logical_scroll_top();
    let is_at_top = scroll_top.item_ix == 0 && scroll_top.offset_in_item <= px(0.5);
    native_presentation::set_sheet_content_at_top(is_at_top);
}

fn parse_document(source: &str) -> MarkdownDocument {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options.insert(Options::ENABLE_GFM);

    let offset_events = Parser::new_ext(source, options)
        .into_offset_iter()
        .collect::<Vec<_>>();
    let events = offset_events
        .iter()
        .map(|(event, _)| event.clone())
        .collect::<Vec<_>>();
    let mut cursor = 0;
    let mut selection_cursor = 0;
    let mut selection_map = Vec::new();
    build_selection_blocks(
        source,
        &offset_events,
        &mut selection_cursor,
        None,
        &mut selection_map,
    );

    MarkdownDocument {
        source: Arc::from(source),
        blocks: parse_blocks(&events, &mut cursor, None).into(),
        selection_map: selection_map.into(),
    }
}

fn build_selection_blocks(
    source: &str,
    events: &[(Event<'_>, Range<usize>)],
    cursor: &mut usize,
    end: Option<TagEnd>,
    map: &mut Vec<MarkdownSelectionSegment>,
) -> Option<Range<usize>> {
    let mut source_range = None;

    while *cursor < events.len() {
        let (event, event_range) = &events[*cursor];
        match event {
            Event::End(tag_end) if Some(*tag_end) == end => {
                merge_source_range(&mut source_range, event_range.clone());
                *cursor += 1;
                break;
            }
            Event::Start(Tag::Paragraph) => {
                let start_range = event_range.clone();
                *cursor += 1;
                let block_range = build_selection_inlines(events, cursor, TagEnd::Paragraph, map)
                    .unwrap_or(start_range);
                push_selection_segment(map, "\n", block_range.clone());
                merge_source_range(&mut source_range, block_range);
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let level = *level;
                let start_range = event_range.clone();
                *cursor += 1;
                let block_range =
                    build_selection_inlines(events, cursor, TagEnd::Heading(level), map)
                        .unwrap_or(start_range);
                push_selection_segment(map, "\n", block_range.clone());
                merge_source_range(&mut source_range, block_range);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                let end_tag = match event {
                    Event::Start(Tag::BlockQuote(kind)) => TagEnd::BlockQuote(*kind),
                    _ => unreachable!(),
                };
                let start_range = event_range.clone();
                *cursor += 1;
                let block_range =
                    build_selection_blocks(source, events, cursor, Some(end_tag), map)
                        .unwrap_or(start_range);
                merge_source_range(&mut source_range, block_range);
            }
            Event::Start(Tag::List(start)) => {
                let ordered = start.is_some();
                let start_number = start.unwrap_or(1) as usize;
                let list_start_range = event_range.clone();
                let mut list_range = Some(list_start_range);
                let mut item_ix = 0;
                *cursor += 1;

                while *cursor < events.len() {
                    let (event, event_range) = &events[*cursor];
                    match event {
                        Event::End(TagEnd::List(_)) => {
                            merge_source_range(&mut list_range, event_range.clone());
                            *cursor += 1;
                            break;
                        }
                        Event::Start(Tag::Item) => {
                            let item_range = event_range.clone();
                            let marker = if ordered {
                                format!("{}.", start_number + item_ix)
                            } else {
                                "•".to_string()
                            };
                            push_selection_segment(map, &marker, item_range.clone());
                            push_selection_segment(map, " ", item_range.clone());
                            *cursor += 1;

                            let child_range = build_selection_blocks(
                                source,
                                events,
                                cursor,
                                Some(TagEnd::Item),
                                map,
                            )
                            .unwrap_or_else(|| item_range.clone());
                            merge_source_range(&mut list_range, item_range);
                            merge_source_range(&mut list_range, child_range);
                            item_ix += 1;
                        }
                        _ => *cursor += 1,
                    }
                }

                if let Some(list_range) = list_range {
                    merge_source_range(&mut source_range, list_range);
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let block_start_range = event_range.clone();
                let is_fenced = matches!(kind, CodeBlockKind::Fenced(_));

                *cursor += 1;
                let mut text = String::new();
                let mut text_range = None;
                while *cursor < events.len() {
                    let (event, event_range) = &events[*cursor];
                    match event {
                        Event::End(TagEnd::CodeBlock) => {
                            merge_source_range(&mut text_range, event_range.clone());
                            *cursor += 1;
                            break;
                        }
                        Event::Text(value)
                        | Event::Code(value)
                        | Event::Html(value)
                        | Event::InlineHtml(value) => {
                            merge_source_range(&mut text_range, event_range.clone());
                            text.push_str(value);
                            *cursor += 1;
                        }
                        Event::SoftBreak | Event::HardBreak => {
                            merge_source_range(&mut text_range, event_range.clone());
                            text.push('\n');
                            *cursor += 1;
                        }
                        _ => *cursor += 1,
                    }
                }

                let text_start = code_content_start(source, &block_start_range, is_fenced);
                push_code_text_selection_segments(map, &text, text_start);
                merge_source_range(&mut source_range, block_start_range);
                if let Some(text_range) = text_range {
                    merge_source_range(&mut source_range, text_range);
                }
            }
            Event::Start(Tag::Table(_)) => {
                let table_start_range = event_range.clone();
                *cursor += 1;
                let table_range =
                    build_selection_table(events, cursor, map).unwrap_or(table_start_range);
                merge_source_range(&mut source_range, table_range);
            }
            Event::Rule => {
                merge_source_range(&mut source_range, event_range.clone());
                *cursor += 1;
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                push_selection_segment(map, html, event_range.clone());
                push_selection_segment(map, "\n", event_range.clone());
                merge_source_range(&mut source_range, event_range.clone());
                *cursor += 1;
            }
            Event::SoftBreak
            | Event::HardBreak
            | Event::Text(_)
            | Event::Code(_)
            | Event::TaskListMarker(_) => {
                let block_range = build_selection_inlines_loose(events, cursor, map)
                    .unwrap_or_else(|| event_range.clone());
                push_selection_segment(map, "\n", block_range.clone());
                merge_source_range(&mut source_range, block_range);
            }
            _ => {
                merge_source_range(&mut source_range, event_range.clone());
                *cursor += 1;
            }
        }
    }

    source_range
}

fn build_selection_table(
    events: &[(Event<'_>, Range<usize>)],
    cursor: &mut usize,
    map: &mut Vec<MarkdownSelectionSegment>,
) -> Option<Range<usize>> {
    let mut source_range = None;

    while *cursor < events.len() {
        let (event, event_range) = &events[*cursor];
        match event {
            Event::End(TagEnd::Table) => {
                merge_source_range(&mut source_range, event_range.clone());
                *cursor += 1;
                break;
            }
            Event::Start(Tag::TableHead) => {
                merge_source_range(&mut source_range, event_range.clone());
                *cursor += 1;
            }
            Event::End(TagEnd::TableHead) => {
                merge_source_range(&mut source_range, event_range.clone());
                *cursor += 1;
            }
            Event::Start(Tag::TableRow) => {
                merge_source_range(&mut source_range, event_range.clone());
                *cursor += 1;
            }
            Event::End(TagEnd::TableRow) => {
                merge_source_range(&mut source_range, event_range.clone());
                *cursor += 1;
            }
            Event::Start(Tag::TableCell) => {
                let cell_start_range = event_range.clone();
                *cursor += 1;
                let cell_range = build_selection_inlines(events, cursor, TagEnd::TableCell, map)
                    .unwrap_or(cell_start_range);
                push_selection_segment(map, "\t", cell_range.clone());
                merge_source_range(&mut source_range, cell_range);
            }
            _ => {
                merge_source_range(&mut source_range, event_range.clone());
                *cursor += 1;
            }
        }
    }

    source_range
}

fn build_selection_inlines_loose(
    events: &[(Event<'_>, Range<usize>)],
    cursor: &mut usize,
    map: &mut Vec<MarkdownSelectionSegment>,
) -> Option<Range<usize>> {
    let mut source_range = None;

    while *cursor < events.len() {
        match &events[*cursor].0 {
            Event::Start(Tag::Paragraph)
            | Event::Start(Tag::Heading { .. })
            | Event::Start(Tag::BlockQuote(_))
            | Event::Start(Tag::List(_))
            | Event::Start(Tag::CodeBlock(_))
            | Event::Start(Tag::Table(_))
            | Event::Rule
            | Event::End(_) => break,
            _ => {
                let inline_range = build_selection_inline_event(events, cursor, map);
                merge_optional_source_range(&mut source_range, inline_range);
            }
        }
    }

    source_range
}

fn build_selection_inlines(
    events: &[(Event<'_>, Range<usize>)],
    cursor: &mut usize,
    end: TagEnd,
    map: &mut Vec<MarkdownSelectionSegment>,
) -> Option<Range<usize>> {
    let mut source_range = None;

    while *cursor < events.len() {
        if let Event::End(tag_end) = &events[*cursor].0
            && *tag_end == end
        {
            merge_source_range(&mut source_range, events[*cursor].1.clone());
            *cursor += 1;
            break;
        }

        let inline_range = build_selection_inline_event(events, cursor, map);
        merge_optional_source_range(&mut source_range, inline_range);
    }

    source_range
}

fn build_selection_inline_event(
    events: &[(Event<'_>, Range<usize>)],
    cursor: &mut usize,
    map: &mut Vec<MarkdownSelectionSegment>,
) -> Option<Range<usize>> {
    let (event, event_range) = &events[*cursor];
    match event {
        Event::Text(text) | Event::Code(text) | Event::Html(text) | Event::InlineHtml(text) => {
            push_selection_segment(map, text, event_range.clone());
            *cursor += 1;
            Some(event_range.clone())
        }
        Event::SoftBreak => {
            push_selection_segment(map, " ", event_range.clone());
            *cursor += 1;
            Some(event_range.clone())
        }
        Event::HardBreak => {
            push_selection_segment(map, "\n", event_range.clone());
            *cursor += 1;
            Some(event_range.clone())
        }
        Event::TaskListMarker(checked) => {
            push_selection_segment(
                map,
                if *checked { "[x]" } else { "[ ]" },
                event_range.clone(),
            );
            push_selection_segment(map, " ", event_range.clone());
            *cursor += 1;
            Some(event_range.clone())
        }
        Event::Start(Tag::Emphasis) => {
            let start_range = event_range.clone();
            *cursor += 1;
            let mut range = Some(start_range);
            merge_optional_source_range(
                &mut range,
                build_selection_inlines(events, cursor, TagEnd::Emphasis, map),
            );
            range
        }
        Event::Start(Tag::Strong) => {
            let start_range = event_range.clone();
            *cursor += 1;
            let mut range = Some(start_range);
            merge_optional_source_range(
                &mut range,
                build_selection_inlines(events, cursor, TagEnd::Strong, map),
            );
            range
        }
        Event::Start(Tag::Strikethrough) => {
            let start_range = event_range.clone();
            *cursor += 1;
            let mut range = Some(start_range);
            merge_optional_source_range(
                &mut range,
                build_selection_inlines(events, cursor, TagEnd::Strikethrough, map),
            );
            range
        }
        Event::Start(Tag::Link { .. }) => {
            let start_range = event_range.clone();
            *cursor += 1;
            let mut range = Some(start_range);
            merge_optional_source_range(
                &mut range,
                build_selection_inlines(events, cursor, TagEnd::Link, map),
            );
            range
        }
        Event::Start(_) | Event::End(_) => {
            let range = event_range.clone();
            *cursor += 1;
            Some(range)
        }
        _ => {
            let range = event_range.clone();
            *cursor += 1;
            Some(range)
        }
    }
}

fn push_code_text_selection_segments(
    map: &mut Vec<MarkdownSelectionSegment>,
    text: &str,
    source_start: usize,
) {
    if text.is_empty() {
        return;
    }

    let mut byte_offset = 0;
    for raw_line in text.split_inclusive('\n') {
        let line = raw_line
            .strip_suffix('\n')
            .unwrap_or(raw_line)
            .strip_suffix('\r')
            .unwrap_or_else(|| raw_line.strip_suffix('\n').unwrap_or(raw_line));
        let line_source_start = source_start + byte_offset;
        let line_source_end = line_source_start + line.len();
        let source_range = line_source_start..line_source_end;
        let rendered_line = if line.is_empty() { " " } else { line };

        push_selection_segment(map, rendered_line, source_range.clone());
        push_selection_segment(map, "\n", source_range);
        byte_offset += raw_line.len();
    }
}

fn code_content_start(source: &str, block_range: &Range<usize>, is_fenced: bool) -> usize {
    let source_len = source.len();
    let start = block_range.start.min(source_len);
    if !is_fenced {
        return start;
    }

    let end = block_range.end.min(source_len);
    source.as_bytes()[start..end]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|newline_offset| start + newline_offset + 1)
        .unwrap_or(start)
}

fn push_selection_segment(
    map: &mut Vec<MarkdownSelectionSegment>,
    text: &str,
    source_range: Range<usize>,
) {
    let len_utf16 = text.encode_utf16().count();
    if len_utf16 == 0 {
        return;
    }
    if let Some(last) = map.last_mut()
        && last.source_range == source_range
    {
        last.len_utf16 += len_utf16;
        return;
    }
    map.push(MarkdownSelectionSegment {
        len_utf16,
        source_range,
    });
}

fn merge_optional_source_range(current: &mut Option<Range<usize>>, range: Option<Range<usize>>) {
    if let Some(range) = range {
        merge_source_range(current, range);
    }
}

fn merge_source_range(current: &mut Option<Range<usize>>, range: Range<usize>) {
    if let Some(current) = current {
        current.start = current.start.min(range.start);
        current.end = current.end.max(range.end);
    } else {
        *current = Some(range);
    }
}

fn line_range_for_selection_map(
    source: &str,
    map: &[MarkdownSelectionSegment],
    range_utf16: Range<usize>,
) -> Option<(u32, u32)> {
    if source.is_empty() || map.is_empty() || range_utf16.is_empty() {
        return None;
    }

    let selection_start = range_utf16.start;
    let selection_end = range_utf16.end.saturating_sub(1);
    let mut offset = 0;
    let mut start_line = None;

    for segment in map {
        let segment_end = offset + segment.len_utf16;
        if start_line.is_none() && selection_start < segment_end {
            start_line = Some(line_number_for_source_range_start(
                source,
                &segment.source_range,
            ));
        }

        if selection_end < segment_end {
            let end_line = line_number_for_source_range_end(source, &segment.source_range);
            return start_line.map(|start| (start, end_line.max(start)));
        }

        offset = segment_end;
    }

    start_line.map(|start| (start, line_number_for_byte_offset(source, source.len())))
}

fn line_number_for_source_range_start(source: &str, range: &Range<usize>) -> u32 {
    line_number_for_byte_offset(source, range.start)
}

fn line_number_for_source_range_end(source: &str, range: &Range<usize>) -> u32 {
    let byte_offset = if range.end > range.start {
        range.end.saturating_sub(1)
    } else {
        range.start
    };
    line_number_for_byte_offset(source, byte_offset)
}

fn line_number_for_byte_offset(source: &str, byte_offset: usize) -> u32 {
    let byte_offset = byte_offset.min(source.len());
    source.as_bytes()[..byte_offset]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count() as u32
        + 1
}

fn parse_blocks(events: &[Event<'_>], cursor: &mut usize, end: Option<TagEnd>) -> Vec<Block> {
    let mut blocks = Vec::new();

    while *cursor < events.len() {
        match &events[*cursor] {
            Event::End(tag_end) if Some(*tag_end) == end => {
                *cursor += 1;
                break;
            }
            Event::Start(Tag::Paragraph) => {
                *cursor += 1;
                blocks.push(Block::Paragraph(parse_inlines(
                    events,
                    cursor,
                    TagEnd::Paragraph,
                )));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let level = *level;
                *cursor += 1;
                blocks.push(Block::Heading {
                    level,
                    content: parse_inlines(events, cursor, TagEnd::Heading(level)),
                });
            }
            Event::Start(Tag::BlockQuote(_)) => {
                let end_tag = match &events[*cursor] {
                    Event::Start(Tag::BlockQuote(kind)) => TagEnd::BlockQuote(*kind),
                    _ => unreachable!(),
                };
                *cursor += 1;
                blocks.push(Block::BlockQuote(parse_blocks(
                    events,
                    cursor,
                    Some(end_tag),
                )));
            }
            Event::Start(Tag::List(start)) => {
                let ordered = start.is_some();
                let start_number = start.unwrap_or(1) as usize;
                *cursor += 1;
                let mut items = Vec::new();
                while *cursor < events.len() {
                    match &events[*cursor] {
                        Event::End(TagEnd::List(_)) => {
                            *cursor += 1;
                            break;
                        }
                        Event::Start(Tag::Item) => {
                            *cursor += 1;
                            items.push(parse_blocks(events, cursor, Some(TagEnd::Item)));
                        }
                        _ => *cursor += 1,
                    }
                }
                blocks.push(Block::List {
                    ordered,
                    start: start_number,
                    items,
                });
            }
            Event::Start(Tag::CodeBlock(_)) => {
                *cursor += 1;
                let mut text = String::new();
                while *cursor < events.len() {
                    match &events[*cursor] {
                        Event::End(TagEnd::CodeBlock) => {
                            *cursor += 1;
                            break;
                        }
                        Event::Text(value)
                        | Event::Code(value)
                        | Event::Html(value)
                        | Event::InlineHtml(value) => {
                            text.push_str(value);
                            *cursor += 1;
                        }
                        Event::SoftBreak | Event::HardBreak => {
                            text.push('\n');
                            *cursor += 1;
                        }
                        _ => *cursor += 1,
                    }
                }
                blocks.push(Block::CodeBlock { text });
            }
            Event::Start(Tag::Table(_)) => {
                *cursor += 1;
                blocks.push(Block::Table(parse_table(events, cursor)));
            }
            Event::Rule => {
                blocks.push(Block::Rule);
                *cursor += 1;
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                blocks.push(Block::Html(html.to_string()));
                *cursor += 1;
            }
            Event::SoftBreak
            | Event::HardBreak
            | Event::Text(_)
            | Event::Code(_)
            | Event::TaskListMarker(_) => {
                blocks.push(Block::Paragraph(parse_inlines_loose(events, cursor)));
            }
            _ => {
                *cursor += 1;
            }
        }
    }

    blocks
}

fn parse_table(events: &[Event<'_>], cursor: &mut usize) -> TableBlock {
    let mut table = TableBlock::default();
    let mut in_head = false;

    while *cursor < events.len() {
        match &events[*cursor] {
            Event::End(TagEnd::Table) => {
                *cursor += 1;
                break;
            }
            Event::Start(Tag::TableHead) => {
                in_head = true;
                *cursor += 1;
            }
            Event::End(TagEnd::TableHead) => {
                in_head = false;
                *cursor += 1;
            }
            Event::Start(Tag::TableRow) => {
                *cursor += 1;
                let row = parse_table_row(events, cursor);
                if in_head {
                    table.headers = row;
                } else {
                    table.rows.push(row);
                }
            }
            Event::Start(Tag::TableCell) => {
                *cursor += 1;
                let cell = parse_inlines(events, cursor, TagEnd::TableCell);
                if in_head {
                    table.headers.push(cell);
                } else if let Some(row) = table.rows.last_mut() {
                    row.push(cell);
                } else {
                    table.rows.push(vec![cell]);
                }
            }
            _ => *cursor += 1,
        }
    }

    table
}

fn parse_table_row(events: &[Event<'_>], cursor: &mut usize) -> Vec<Vec<Inline>> {
    let mut row = Vec::new();
    while *cursor < events.len() {
        match &events[*cursor] {
            Event::End(TagEnd::TableRow) => {
                *cursor += 1;
                break;
            }
            Event::Start(Tag::TableCell) => {
                *cursor += 1;
                row.push(parse_inlines(events, cursor, TagEnd::TableCell));
            }
            _ => *cursor += 1,
        }
    }
    row
}

fn parse_inlines_loose(events: &[Event<'_>], cursor: &mut usize) -> Vec<Inline> {
    let mut inlines = Vec::new();
    while *cursor < events.len() {
        match &events[*cursor] {
            Event::Start(Tag::Paragraph)
            | Event::Start(Tag::Heading { .. })
            | Event::Start(Tag::BlockQuote(_))
            | Event::Start(Tag::List(_))
            | Event::Start(Tag::CodeBlock(_))
            | Event::Start(Tag::Table(_))
            | Event::Rule
            | Event::End(_) => break,
            _ => {
                inlines.extend(parse_inline_event(events, cursor));
            }
        }
    }
    inlines
}

fn parse_inlines(events: &[Event<'_>], cursor: &mut usize, end: TagEnd) -> Vec<Inline> {
    let mut inlines = Vec::new();

    while *cursor < events.len() {
        if let Event::End(tag_end) = &events[*cursor]
            && *tag_end == end
        {
            *cursor += 1;
            break;
        }

        inlines.extend(parse_inline_event(events, cursor));
    }

    inlines
}

fn parse_inline_event(events: &[Event<'_>], cursor: &mut usize) -> Vec<Inline> {
    match &events[*cursor] {
        Event::Text(text) => {
            *cursor += 1;
            vec![Inline::Text(text.to_string())]
        }
        Event::Code(text) => {
            *cursor += 1;
            vec![Inline::Code(text.to_string())]
        }
        Event::Html(text) | Event::InlineHtml(text) => {
            *cursor += 1;
            vec![Inline::Html(text.to_string())]
        }
        Event::SoftBreak => {
            *cursor += 1;
            vec![Inline::SoftBreak]
        }
        Event::HardBreak => {
            *cursor += 1;
            vec![Inline::HardBreak]
        }
        Event::TaskListMarker(checked) => {
            let checked = *checked;
            *cursor += 1;
            vec![Inline::TaskMarker(checked), Inline::Text(" ".into())]
        }
        Event::Start(Tag::Emphasis) => {
            *cursor += 1;
            vec![Inline::Emphasis(parse_inlines(
                events,
                cursor,
                TagEnd::Emphasis,
            ))]
        }
        Event::Start(Tag::Strong) => {
            *cursor += 1;
            vec![Inline::Strong(parse_inlines(
                events,
                cursor,
                TagEnd::Strong,
            ))]
        }
        Event::Start(Tag::Strikethrough) => {
            *cursor += 1;
            vec![Inline::Strikethrough(parse_inlines(
                events,
                cursor,
                TagEnd::Strikethrough,
            ))]
        }
        Event::Start(Tag::Link { dest_url, .. }) => {
            let url = dest_url.to_string();
            *cursor += 1;
            vec![Inline::Link {
                url,
                content: parse_inlines(events, cursor, TagEnd::Link),
            }]
        }
        Event::Start(_) => {
            *cursor += 1;
            Vec::new()
        }
        Event::End(_) => {
            *cursor += 1;
            Vec::new()
        }
        _ => {
            *cursor += 1;
            Vec::new()
        }
    }
}

fn render_block(block: &Block, key: String, window: &mut Window, cx: &mut App) -> AnyElement {
    match block {
        Block::Paragraph(content) => render_inline_block(content, InlineBlockStyle::Body, key),
        Block::Heading { level, content } => {
            let style = match level {
                HeadingLevel::H1 => InlineBlockStyle::Title,
                HeadingLevel::H2 => InlineBlockStyle::Section,
                _ => InlineBlockStyle::Heading,
            };
            render_inline_block(content, style, key)
        }
        Block::BlockQuote(children) => {
            div()
                .w_full()
                .pl(px(theme::SPACING_MD))
                .border_l_1()
                .border_color(rgb(theme::BORDER_DEFAULT))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .children(children.iter().enumerate().map(|(ix, child)| {
                    render_block(child, format!("{key}-quote-{ix}"), window, cx)
                }))
                .into_any_element()
        }
        Block::List {
            ordered,
            start,
            items,
        } => div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .children(items.iter().enumerate().map(|(ix, item)| {
                let marker = if *ordered {
                    format!("{}.", start + ix)
                } else {
                    "•".to_string()
                };
                div()
                    .w_full()
                    .flex()
                    .items_start()
                    .gap(px(4.0))
                    .child(
                        div()
                            .flex_shrink_0()
                            .min_w(px(16.0))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_BODY))
                            .line_height(px(theme::FONT_BODY + 6.0))
                            .font_family(fonts::MONO_FONT_FAMILY)
                            .child(markdown_text(StyledText::new(marker), " ")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .w_0()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .children(item.iter().enumerate().map(|(child_ix, child)| {
                                render_block(
                                    child,
                                    format!("{key}-item-{ix}-{child_ix}"),
                                    window,
                                    cx,
                                )
                            })),
                    )
            }))
            .into_any_element(),
        Block::CodeBlock { text } => {
            let code_width = code_block_content_min_width(text);
            let code_lines = div()
                .min_w(px(code_width))
                .px(px(CODE_BLOCK_PADDING_X))
                .py(px(CODE_BLOCK_PADDING_Y))
                .flex()
                .flex_col()
                .children(text.lines().map(|line| {
                    let line = if line.is_empty() {
                        " ".to_string()
                    } else {
                        line.to_string()
                    };
                    div()
                        .w_full()
                        .text_color(rgb(theme::TEXT_PRIMARY))
                        .text_size(px(CODE_BLOCK_FONT_SIZE))
                        .line_height(px(CODE_BLOCK_LINE_HEIGHT))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .whitespace_nowrap()
                        .child(markdown_text(StyledText::new(line), "\n"))
                }));

            let mut container = div()
                .id(format!("{key}-code-scroll"))
                .w_full()
                .bg(rgb(theme::BG_CARD))
                .border_1()
                .border_color(rgb(theme::BORDER_DEFAULT))
                .rounded(px(6.0))
                .overflow_x_scroll()
                .child(code_lines);
            container.style().restrict_scroll_to_axis = Some(true);

            container.into_any_element()
        }
        Block::Table(table) => render_table(table, key),
        Block::Html(text) => {
            render_inline_block(&[Inline::Html(text.clone())], InlineBlockStyle::Html, key)
        }
        Block::Rule => div()
            .w_full()
            .h(px(1.0))
            .bg(rgb(theme::BORDER_SUBTLE))
            .into_any_element(),
    }
}

fn code_block_content_min_width(text: &str) -> f32 {
    let columns = text
        .lines()
        .map(code_block_display_columns)
        .max()
        .unwrap_or(1)
        .max(1);
    (columns as f32 * CODE_BLOCK_FONT_SIZE * CODE_BLOCK_CHAR_WIDTH_FACTOR).ceil()
}

fn code_block_display_columns(line: &str) -> usize {
    let mut columns = 0;
    for ch in line.chars() {
        if ch == '\t' {
            columns += CODE_BLOCK_TAB_WIDTH - (columns % CODE_BLOCK_TAB_WIDTH);
        } else {
            columns += 1;
        }
    }
    columns
}

fn render_table(table: &TableBlock, key: String) -> AnyElement {
    let column_widths = table_column_widths(table);
    let table_width = column_widths.iter().sum::<f32>().max(TABLE_CELL_MIN_WIDTH);
    let rows = (!table.headers.is_empty())
        .then_some((true, table.headers.as_slice()))
        .into_iter()
        .chain(table.rows.iter().map(|row| (false, row.as_slice())));
    let row_count = table.rows.len() + usize::from(!table.headers.is_empty());

    let table_body = div()
        .w_full()
        .min_w(px(table_width))
        .flex()
        .flex_col()
        .children(
            rows.into_iter()
                .enumerate()
                .map(|(row_ix, (is_header, row))| {
                    let is_last_row = row_ix + 1 == row_count;
                    div()
                        .w_full()
                        .flex()
                        .border_b_1()
                        .when(is_last_row, |this| {
                            this.border_color(rgb(theme::BORDER_SUBTLE))
                        })
                        .border_color(rgb(theme::BORDER_SUBTLE))
                        .children(row.iter().enumerate().map(|(cell_ix, cell)| {
                            let cell_width = column_widths
                                .get(cell_ix)
                                .copied()
                                .unwrap_or(TABLE_CELL_MIN_WIDTH);
                            div()
                                .flex_none()
                                .w(px(cell_width))
                                .min_w(px(cell_width))
                                .max_w(px(TABLE_CELL_MAX_WIDTH))
                                .px(px(TABLE_CELL_PADDING_X))
                                .py(px(TABLE_CELL_PADDING_Y))
                                .border_r_1()
                                .border_color(rgb(theme::BORDER_SUBTLE))
                                .child(render_inline_block(
                                    &cell,
                                    if is_header {
                                        InlineBlockStyle::TableHeader
                                    } else {
                                        InlineBlockStyle::TableCell
                                    },
                                    format!("{key}-row-{row_ix}-cell-{cell_ix}"),
                                ))
                        }))
                }),
        );

    let mut container = div()
        .id(format!("{key}-table-scroll"))
        .w_full()
        .bg(rgb(theme::BG_CARD))
        .border_1()
        .border_color(rgb(theme::BORDER_DEFAULT))
        .rounded(px(6.0))
        .overflow_x_scroll()
        .child(table_body);
    container.style().restrict_scroll_to_axis = Some(true);

    container.into_any_element()
}

fn table_column_widths(table: &TableBlock) -> Vec<f32> {
    let column_count = table
        .headers
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or_default());
    let mut widths = vec![TABLE_CELL_MIN_WIDTH; column_count.max(1)];

    for (ix, cell) in table.headers.iter().enumerate() {
        widths[ix] = widths[ix].max(table_cell_min_width(cell));
    }
    for row in &table.rows {
        for (ix, cell) in row.iter().enumerate() {
            widths[ix] = widths[ix].max(table_cell_min_width(cell));
        }
    }

    widths
}

fn table_cell_min_width(cell: &[Inline]) -> f32 {
    let columns = inline_display_columns(cell).max(1);
    (columns as f32 * theme::FONT_BODY * TABLE_CELL_CHAR_WIDTH_FACTOR + TABLE_CELL_PADDING_X * 2.0)
        .ceil()
        .clamp(TABLE_CELL_MIN_WIDTH, TABLE_CELL_MAX_WIDTH)
}

fn inline_display_columns(inlines: &[Inline]) -> usize {
    let mut current = 0;
    let mut max = 0;
    add_inline_display_columns(inlines, &mut current, &mut max);
    max.max(current)
}

fn add_inline_display_columns(inlines: &[Inline], current: &mut usize, max: &mut usize) {
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::Code(text) | Inline::Html(text) => {
                add_display_text_columns(text, current, max);
            }
            Inline::Emphasis(children)
            | Inline::Strong(children)
            | Inline::Strikethrough(children) => {
                add_inline_display_columns(children, current, max);
            }
            Inline::Link { content, .. } => {
                add_inline_display_columns(content, current, max);
            }
            Inline::SoftBreak => {
                *current += 1;
            }
            Inline::HardBreak => {
                *max = (*max).max(*current);
                *current = 0;
            }
            Inline::TaskMarker(_) => {
                *current += 3;
            }
        }
    }
}

fn add_display_text_columns(text: &str, current: &mut usize, max: &mut usize) {
    for ch in text.chars() {
        match ch {
            '\n' => {
                *max = (*max).max(*current);
                *current = 0;
            }
            '\t' => {
                *current += CODE_BLOCK_TAB_WIDTH - (*current % CODE_BLOCK_TAB_WIDTH);
            }
            _ => *current += 1,
        }
    }
}

fn markdown_text(text: StyledText, selection_separator_after: &'static str) -> StyledText {
    text.selectable()
        .selection_separator_after(selection_separator_after)
}

#[derive(Clone, Copy)]
enum InlineBlockStyle {
    Title,
    Section,
    Heading,
    Body,
    TableHeader,
    TableCell,
    Html,
}

fn render_inline_block(content: &[Inline], style: InlineBlockStyle, key: String) -> AnyElement {
    let mut buffer = InlineRenderBuffer::default();
    flatten_inlines(content, &mut buffer, InlineMarks::default());

    let base = block_style(style);
    let styled = StyledText::new(buffer.text.clone()).with_highlights(merge_highlights(
        buffer
            .highlights
            .into_iter()
            .map(|run| (run.range, run.style))
            .collect(),
    ));
    let styled = markdown_text(styled, selection_separator_after(style));

    let link_ranges = buffer
        .links
        .iter()
        .map(|link| link.range.clone())
        .collect::<Vec<_>>();
    let link_urls = buffer
        .links
        .iter()
        .map(|link| link.url.clone())
        .collect::<Vec<_>>();
    let text: AnyElement = if link_ranges.is_empty() {
        styled.into_any_element()
    } else {
        InteractiveText::new(key, styled)
            .hit_slop(px(MARKDOWN_LINK_HIT_SLOP))
            .on_click(link_ranges, move |ix, _window, _cx| {
                if let Some(url) = link_urls.get(ix) {
                    platform_bridge::bridge().open_url(url);
                }
            })
            .into_any_element()
    };

    let base_weight = base.weight;
    div()
        .w_full()
        .text_color(base.color)
        .text_size(px(base.size))
        .line_height(px(base.line_height))
        .font_family(base.font_family)
        .when_some(base_weight, |this, weight| this.font_weight(weight))
        .whitespace_normal()
        .child(text)
        .into_any_element()
}

#[derive(Clone, Copy, Default)]
struct InlineMarks {
    emphasis: bool,
    strong: bool,
    strike: bool,
    code: bool,
    html: bool,
}

fn flatten_inlines(content: &[Inline], out: &mut InlineRenderBuffer, marks: InlineMarks) {
    for inline in content {
        match inline {
            Inline::Text(text) => push_text_with_marks(out, text, marks),
            Inline::Code(text) => {
                let mut next = marks;
                next.code = true;
                push_text_with_marks(out, text, next);
            }
            Inline::Html(text) => {
                let mut next = marks;
                next.html = true;
                push_text_with_marks(out, text, next);
            }
            Inline::Emphasis(children) => {
                let mut next = marks;
                next.emphasis = true;
                flatten_inlines(children, out, next);
            }
            Inline::Strong(children) => {
                let mut next = marks;
                next.strong = true;
                flatten_inlines(children, out, next);
            }
            Inline::Strikethrough(children) => {
                let mut next = marks;
                next.strike = true;
                flatten_inlines(children, out, next);
            }
            Inline::Link { url, content } => {
                let start = out.text.len();
                flatten_inlines(content, out, marks);
                let end = out.text.len();
                if start != end {
                    let range = start..end;
                    out.highlights.push(StyledRun {
                        range: range.clone(),
                        style: HighlightStyle {
                            color: Some(rgb(theme::ACCENT_BLUE).into()),
                            underline: Some(UnderlineStyle {
                                color: Some(rgb(theme::ACCENT_BLUE).into()),
                                thickness: px(1.0),
                                wavy: false,
                            }),
                            ..Default::default()
                        },
                    });
                    out.links.push(LinkRun {
                        range,
                        url: url.clone(),
                    });
                }
            }
            Inline::SoftBreak => {
                out.push_text(" ");
            }
            Inline::HardBreak => {
                out.push_text("\n");
            }
            Inline::TaskMarker(checked) => {
                out.push_text(if *checked { "[x]" } else { "[ ]" });
            }
        }
    }
}

fn push_text_with_marks(out: &mut InlineRenderBuffer, text: &str, marks: InlineMarks) {
    if text.is_empty() {
        return;
    }
    let range = out.push_text(text);
    let mut style = HighlightStyle::default();
    let mut has_style = false;

    if marks.emphasis {
        style.font_style = Some(FontStyle::Italic);
        has_style = true;
    }
    if marks.strong {
        style.font_weight = Some(FontWeight::BOLD);
        has_style = true;
    }
    if marks.strike {
        style.strikethrough = Some(StrikethroughStyle {
            color: Some(rgb(theme::TEXT_SECONDARY).into()),
            thickness: px(1.0),
        });
        has_style = true;
    }
    if marks.code {
        style.background_color = Some(rgb(theme::BG_CARD).into());
        style.color = Some(rgb(theme::TEXT_PRIMARY).into());
        has_style = true;
    }
    if marks.html {
        style.background_color = Some(rgb(theme::BG_CARD).into());
        style.color = Some(rgb(theme::TEXT_MUTED).into());
        has_style = true;
    }

    if has_style {
        out.highlights.push(StyledRun { range, style });
    }
}

struct BlockStyleSpec {
    size: f32,
    line_height: f32,
    color: Hsla,
    font_family: &'static str,
    weight: Option<FontWeight>,
}

fn block_style(style: InlineBlockStyle) -> BlockStyleSpec {
    match style {
        InlineBlockStyle::Title => BlockStyleSpec {
            size: theme::FONT_TITLE,
            line_height: theme::FONT_TITLE + 8.0,
            color: rgb(theme::TEXT_PRIMARY).into(),
            font_family: fonts::HEADING_FONT_FAMILY,
            weight: Some(FontWeight::MEDIUM),
        },
        InlineBlockStyle::Section => BlockStyleSpec {
            size: theme::FONT_HEADING + 2.0,
            line_height: theme::FONT_HEADING + 8.0,
            color: rgb(theme::TEXT_PRIMARY).into(),
            font_family: fonts::HEADING_FONT_FAMILY,
            weight: Some(FontWeight::MEDIUM),
        },
        InlineBlockStyle::Heading => BlockStyleSpec {
            size: theme::FONT_HEADING,
            line_height: theme::FONT_HEADING + 6.0,
            color: rgb(theme::TEXT_PRIMARY).into(),
            font_family: fonts::HEADING_FONT_FAMILY,
            weight: Some(FontWeight::MEDIUM),
        },
        InlineBlockStyle::TableHeader => BlockStyleSpec {
            size: theme::FONT_BODY,
            line_height: theme::FONT_BODY + 6.0,
            color: rgb(theme::TEXT_PRIMARY).into(),
            font_family: fonts::MONO_FONT_FAMILY,
            weight: Some(FontWeight::MEDIUM),
        },
        InlineBlockStyle::TableCell | InlineBlockStyle::Body => BlockStyleSpec {
            size: theme::FONT_BODY,
            line_height: theme::FONT_BODY + 6.0,
            color: rgb(theme::TEXT_SECONDARY).into(),
            font_family: fonts::MONO_FONT_FAMILY,
            weight: None,
        },
        InlineBlockStyle::Html => BlockStyleSpec {
            size: theme::FONT_DETAIL,
            line_height: theme::FONT_DETAIL + 5.0,
            color: rgb(theme::TEXT_MUTED).into(),
            font_family: fonts::MONO_FONT_FAMILY,
            weight: None,
        },
    }
}

fn selection_separator_after(style: InlineBlockStyle) -> &'static str {
    match style {
        InlineBlockStyle::TableHeader | InlineBlockStyle::TableCell => "\t",
        InlineBlockStyle::Title
        | InlineBlockStyle::Section
        | InlineBlockStyle::Heading
        | InlineBlockStyle::Body
        | InlineBlockStyle::Html => "\n",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Block, Inline, MarkdownView, TABLE_CELL_MAX_WIDTH, TABLE_CELL_MIN_WIDTH, TableBlock,
        code_block_display_columns, inline_display_columns, is_markdown_path,
        markdown_list_item_count, parse_document, table_column_widths,
    };

    #[test]
    fn detects_markdown_paths_for_preview_and_workspace_editor() {
        assert!(is_markdown_path("/repo/README.md"));
        assert!(is_markdown_path("/repo/readme"));
        assert!(is_markdown_path("/repo/ReadMe.MD"));
        assert!(is_markdown_path("/repo/docs/guide.markdown"));
        assert!(is_markdown_path("/repo/notes.MKD"));
        assert!(is_markdown_path("/repo/notes.mdtxt"));

        assert!(!is_markdown_path("/repo/src/main.rs"));
        assert!(!is_markdown_path("/repo/README_BACKUP"));
        assert!(!is_markdown_path("/repo/README.txt"));
        assert!(!is_markdown_path("/repo/Makefile"));
    }

    #[test]
    fn parses_markdown_into_top_level_virtualization_rows() {
        let document = parse_document(
            r#"# Title

Body paragraph.

> Quote paragraph.
>
> - Nested quote item

1. First
2. Second

```rust
fn main() {}
```

| A | B |
| - | - |
| 1 | 2 |
"#,
        );

        assert_eq!(document.blocks.len(), 6);
        assert!(matches!(document.blocks[0], Block::Heading { .. }));
        assert!(matches!(document.blocks[1], Block::Paragraph(_)));
        assert!(matches!(document.blocks[2], Block::BlockQuote(_)));
        assert!(matches!(document.blocks[3], Block::List { .. }));
        assert!(matches!(document.blocks[4], Block::CodeBlock { .. }));
        assert!(matches!(document.blocks[5], Block::Table(_)));
        assert!(document.total_block_count() > document.blocks.len());
    }

    #[test]
    fn maps_markdown_selection_to_source_lines() {
        let document = parse_document("# Title\n\nBody paragraph.\n");

        assert_eq!(document.line_range_for_selection(0..5), Some((1, 1)));
        assert_eq!(document.line_range_for_selection(0..11), Some((1, 3)));
    }

    #[test]
    fn maps_markdown_code_block_selection_to_source_lines() {
        let document = parse_document("```rust\nfn main() {}\n\n```\n");

        assert_eq!(document.line_range_for_selection(0..12), Some((2, 2)));
        assert_eq!(document.line_range_for_selection(13..14), Some((3, 3)));
    }

    #[test]
    fn markdown_code_block_selection_omits_fenced_language_label() {
        let document = parse_document("```bash\necho ok\n```\n");

        assert_eq!(document.line_range_for_selection(0..4), Some((2, 2)));
    }

    #[test]
    fn code_block_display_columns_expands_tabs() {
        assert_eq!(code_block_display_columns("a\tb"), 5);
        assert_eq!(code_block_display_columns("\tindented"), 12);
    }

    #[test]
    fn maps_markdown_task_list_selection_to_source_lines() {
        let document = parse_document("- [x] Done\n- Todo\n");

        assert_eq!(document.line_range_for_selection(0..11), Some((1, 1)));
        assert_eq!(document.line_range_for_selection(11..18), Some((2, 2)));
        assert_eq!(document.line_range_for_selection(0..18), Some((1, 2)));
    }

    #[test]
    fn maps_markdown_table_selection_to_source_lines() {
        let document = parse_document("| A | B |\n| - | - |\n| 1 | 2 |\n");

        assert_eq!(document.line_range_for_selection(0..4), Some((1, 1)));
        assert_eq!(document.line_range_for_selection(4..8), Some((3, 3)));
        assert_eq!(document.line_range_for_selection(0..8), Some((1, 3)));
    }

    #[test]
    fn parses_markdown_table_header_cells() {
        let document = parse_document("| Header | Status |\n| - | - |\n| alpha | ok |\n");

        let Block::Table(table) = &document.blocks[0] else {
            panic!("expected table block");
        };

        assert_eq!(table.headers.len(), 2);
        assert_eq!(inline_display_columns(&table.headers[0]), 6);
        assert_eq!(inline_display_columns(&table.headers[1]), 6);
        assert_eq!(table.rows.len(), 1);
    }

    #[test]
    fn table_column_widths_cap_long_content() {
        let table = TableBlock {
            headers: vec![vec![Inline::Text("Name".to_string())]],
            rows: vec![vec![vec![Inline::Text(
                "very-long-value-that-needs-horizontal-scroll".to_string(),
            )]]],
        };

        let widths = table_column_widths(&table);

        assert_eq!(widths.len(), 1);
        assert!(widths[0] > TABLE_CELL_MIN_WIDTH);
        assert_eq!(widths[0], TABLE_CELL_MAX_WIDTH);
    }

    #[test]
    fn markdown_view_resets_virtualized_rows_when_source_changes() {
        let mut view = MarkdownView::new("# Title\n\nBody paragraph.");
        assert_eq!(
            view.list_state.item_count(),
            markdown_list_item_count(view.document.blocks.len())
        );
        assert_eq!(view.document.blocks.len(), 2);

        view.set_source(
            r#"# Updated

- First
- Second

```rust
fn main() {}
```
"#,
        );

        assert_eq!(view.document.blocks.len(), 3);
        assert_eq!(
            view.list_state.item_count(),
            markdown_list_item_count(view.document.blocks.len())
        );
    }
}
