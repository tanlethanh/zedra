use gpui::prelude::FluentBuilder;
use gpui::*;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::editor::merge_highlights;
use crate::fonts;
use crate::native_presentation;
use crate::platform_bridge;
use crate::theme;

pub struct MarkdownView {
    source: SharedString,
    scroll_handle: ScrollHandle,
}

impl MarkdownView {
    pub fn new(source: impl Into<SharedString>) -> Self {
        Self {
            source: source.into(),
            scroll_handle: ScrollHandle::new(),
        }
    }

    pub fn set_source(&mut self, source: impl Into<SharedString>) {
        self.source = source.into();
    }
}

#[derive(Clone, Debug, Default)]
struct MarkdownDocument {
    blocks: Vec<Block>,
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
        info: Option<String>,
        text: String,
    },
    Table(TableBlock),
    Html(String),
    Rule,
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let document = parse_document(self.source.as_ref());
        update_sheet_scroll_boundary(&self.scroll_handle);

        div()
            .id("markdown-preview-scroll")
            .size_full()
            .min_h_0()
            .track_scroll(&self.scroll_handle)
            .overflow_y_scroll()
            .on_scroll_wheel(cx.listener(|this, _event, _window, _cx| {
                update_sheet_scroll_boundary(&this.scroll_handle);
            }))
            .child(
                div()
                    .w_full()
                    .p(px(theme::SPACING_LG))
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACING_MD))
                    .children(
                        document
                            .blocks
                            .iter()
                            .enumerate()
                            .map(|(ix, block)| render_block(block, format!("md-{ix}"), window, cx)),
                    ),
            )
    }
}

fn update_sheet_scroll_boundary(scroll_handle: &ScrollHandle) {
    let offset_y = f32::from(scroll_handle.offset().y);
    let is_at_top = offset_y >= -0.5;
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

    let events = Parser::new_ext(source, options).collect::<Vec<_>>();
    let mut cursor = 0;
    MarkdownDocument {
        blocks: parse_blocks(&events, &mut cursor, None),
    }
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
            Event::Start(Tag::CodeBlock(kind)) => {
                let info = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(info) => {
                        let value = info.trim();
                        (!value.is_empty()).then(|| value.to_string())
                    }
                };
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
                blocks.push(Block::CodeBlock { info, text });
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
                            .child(marker),
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
        Block::CodeBlock { info, text } => {
            let mut container = div()
                .w_full()
                .bg(rgb(theme::BG_CARD))
                .border_1()
                .border_color(rgb(theme::BORDER_DEFAULT))
                .rounded(px(6.0))
                .p(px(theme::SPACING_MD))
                .flex()
                .flex_col()
                .gap(px(6.0));

            if let Some(info) = info {
                container = container.child(
                    div()
                        .text_color(rgb(theme::TEXT_MUTED))
                        .text_size(px(theme::FONT_DETAIL - 1.0))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .child(info.clone()),
                );
            }

            container
                .children(text.lines().map(|line| {
                    div()
                        .w_full()
                        .text_color(rgb(theme::TEXT_PRIMARY))
                        .text_size(px(theme::FONT_DETAIL))
                        .line_height(px(theme::FONT_DETAIL + 5.0))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .whitespace_nowrap()
                        .child(if line.is_empty() {
                            " ".to_string()
                        } else {
                            line.to_string()
                        })
                }))
                .into_any_element()
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

fn render_table(table: &TableBlock, key: String) -> AnyElement {
    let mut rows = Vec::new();
    if !table.headers.is_empty() {
        rows.push(table.headers.clone());
    }
    rows.extend(table.rows.clone());

    div()
        .w_full()
        .bg(rgb(theme::BG_CARD))
        .border_1()
        .border_color(rgb(theme::BORDER_DEFAULT))
        .rounded(px(6.0))
        .overflow_hidden()
        .flex()
        .flex_col()
        .children(rows.into_iter().enumerate().map(|(row_ix, row)| {
            let is_last_row =
                row_ix + 1 == table.rows.len() + usize::from(!table.headers.is_empty());
            div()
                .w_full()
                .flex()
                .border_b_1()
                .when(is_last_row, |this| {
                    this.border_color(rgb(theme::BORDER_SUBTLE))
                })
                .border_color(rgb(theme::BORDER_SUBTLE))
                .children(row.into_iter().enumerate().map(|(cell_ix, cell)| {
                    div()
                        .flex_1()
                        .min_w(px(96.0))
                        .p(px(theme::SPACING_MD))
                        .border_r_1()
                        .border_color(rgb(theme::BORDER_SUBTLE))
                        .child(render_inline_block(
                            &cell,
                            if row_ix == 0 && !table.headers.is_empty() {
                                InlineBlockStyle::TableHeader
                            } else {
                                InlineBlockStyle::TableCell
                            },
                            format!("{key}-row-{row_ix}-cell-{cell_ix}"),
                        ))
                }))
        }))
        .into_any_element()
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
