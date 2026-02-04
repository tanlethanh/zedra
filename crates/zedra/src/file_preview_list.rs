// FilePreviewList — grid of preview cards for sample code files.
// Tapping a card emits PreviewSelected so the parent can push an EditorView.

use gpui::prelude::FluentBuilder;
use gpui::*;

/// Metadata for a sample file shown in the preview grid.
pub struct SampleFile {
    pub filename: &'static str,
    pub language: &'static str,
    pub content: &'static str,
    pub line_count: usize,
}

const SAMPLE_CACHE: &str = r#"use std::collections::HashMap;

/// A simple key-value store with expiration.
pub struct Cache<V> {
    entries: HashMap<String, (V, Option<std::time::Instant>)>,
    default_ttl: std::time::Duration,
}

impl<V: Clone> Cache<V> {
    pub fn new(default_ttl: std::time::Duration) -> Self {
        Self {
            entries: HashMap::new(),
            default_ttl,
        }
    }

    pub fn insert(&mut self, key: String, value: V) {
        let expires_at = std::time::Instant::now() + self.default_ttl;
        self.entries.insert(key, (value, Some(expires_at)));
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        match self.entries.get(key) {
            Some((value, Some(exp))) if *exp > std::time::Instant::now() => Some(value),
            Some((value, None)) => Some(value),
            _ => None,
        }
    }

    pub fn evict_expired(&mut self) {
        let now = std::time::Instant::now();
        self.entries.retain(|_, (_, exp)| exp.map_or(true, |e| e > now));
    }
}

fn main() {
    let mut cache = Cache::new(std::time::Duration::from_secs(60));
    cache.insert("greeting".to_string(), "Hello, world!");
    if let Some(v) = cache.get("greeting") {
        println!("Found: {}", v);
    }
}"#;

const SAMPLE_PARSER: &str = r#"use std::iter::Peekable;
use std::str::Chars;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(f64),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Eof,
}

pub struct Lexer<'a> {
    chars: Peekable<Chars<'a>>,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self { chars: input.chars().peekable() }
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace();
        match self.chars.peek() {
            None => Token::Eof,
            Some(&c) => match c {
                '+' => { self.chars.next(); Token::Plus }
                '-' => { self.chars.next(); Token::Minus }
                '*' => { self.chars.next(); Token::Star }
                '/' => { self.chars.next(); Token::Slash }
                '(' => { self.chars.next(); Token::LParen }
                ')' => { self.chars.next(); Token::RParen }
                '0'..='9' | '.' => self.read_number(),
                _ => { self.chars.next(); self.next_token() }
            },
        }
    }

    fn skip_whitespace(&mut self) {
        while self.chars.peek().map_or(false, |c| c.is_whitespace()) {
            self.chars.next();
        }
    }

    fn read_number(&mut self) -> Token {
        let mut s = String::new();
        while self.chars.peek().map_or(false, |c| c.is_ascii_digit() || *c == '.') {
            s.push(self.chars.next().unwrap());
        }
        Token::Number(s.parse().unwrap_or(0.0))
    }
}"#;

const SAMPLE_SERVER: &str = r#"use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

type Db = Arc<Mutex<HashMap<String, String>>>;

fn handle_client(stream: TcpStream, db: Db) -> io::Result<()> {
    let mut reader = io::BufReader::new(&stream);
    let mut writer = io::BufWriter::new(&stream);

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 { break; }

        let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
        let response = match parts.as_slice() {
            ["GET", key] => {
                let db = db.lock().unwrap();
                db.get(*key).cloned().unwrap_or_else(|| "(nil)".into())
            }
            ["SET", key, value] => {
                let mut db = db.lock().unwrap();
                db.insert(key.to_string(), value.to_string());
                "OK".into()
            }
            ["DEL", key] => {
                let mut db = db.lock().unwrap();
                if db.remove(*key).is_some() { "1" } else { "0" }.into()
            }
            _ => "ERR unknown command".into(),
        };
        writeln!(writer, "{}", response)?;
        writer.flush()?;
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:7878")?;
    let db: Db = Arc::new(Mutex::new(HashMap::new()));
    println!("Listening on :7878");

    for stream in listener.incoming().flatten() {
        let db = db.clone();
        thread::spawn(move || { let _ = handle_client(stream, db); });
    }
    Ok(())
}"#;

const SAMPLE_CONFIG: &str = r#"use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub enum Value {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Array(Vec<Value>),
    Table(HashMap<String, Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self { Value::String(s) => Some(s), _ => None }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self { Value::Integer(n) => Some(*n), _ => None }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self { Value::Boolean(b) => Some(*b), _ => None }
    }
}

pub struct Config {
    values: HashMap<String, Value>,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::parse(&content)
    }

    pub fn parse(input: &str) -> Result<Self, String> {
        let mut values = HashMap::new();
        for line in input.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let (key, val) = line.split_once('=')
                .ok_or_else(|| format!("invalid line: {}", line))?;
            values.insert(
                key.trim().to_string(),
                Value::String(val.trim().to_string()),
            );
        }
        Ok(Self { values })
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.values.get(key).and_then(|v| v.as_str())
    }
}"#;

pub const SAMPLE_FILES: &[SampleFile] = &[
    SampleFile {
        filename: "cache.rs",
        language: "Rust",
        content: SAMPLE_CACHE,
        line_count: 56,
    },
    SampleFile {
        filename: "parser.rs",
        language: "Rust",
        content: SAMPLE_PARSER,
        line_count: 60,
    },
    SampleFile {
        filename: "server.rs",
        language: "Rust",
        content: SAMPLE_SERVER,
        line_count: 52,
    },
    SampleFile {
        filename: "config.rs",
        language: "Rust",
        content: SAMPLE_CONFIG,
        line_count: 58,
    },
];

/// Event emitted when a preview card is tapped.
#[derive(Clone, Debug)]
pub struct PreviewSelected {
    pub index: usize,
}

pub struct FilePreviewList {
    focus_handle: FocusHandle,
}

impl FilePreviewList {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl EventEmitter<PreviewSelected> for FilePreviewList {}

impl Focusable for FilePreviewList {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FilePreviewList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut grid = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_3()
            .p_4();

        for (idx, sample) in SAMPLE_FILES.iter().enumerate() {
            // First 6 lines of code, truncated to ~24 chars each
            let preview_lines: Vec<String> = sample
                .content
                .lines()
                .take(6)
                .map(|l| {
                    if l.len() > 24 {
                        format!("{}…", &l[..24])
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            let preview_text = preview_lines.join("\n");
            let line_count_label = format!("{} lines", sample.line_count);
            let filename: SharedString = sample.filename.into();
            let language: SharedString = sample.language.into();

            grid = grid.child(
                div()
                    .w(px(155.0))
                    .h(px(180.0))
                    .bg(rgb(0x282c34))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(rgb(0x3e4451))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.border_color(rgb(0x61afef)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _event, _window, cx| {
                            cx.emit(PreviewSelected { index: idx });
                        }),
                    )
                    // Filename
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xabb2bf))
                            .child(filename),
                    )
                    // Language badge
                    .child(
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(rgb(0x3e4451))
                            .text_xs()
                            .text_color(rgb(0xe5c07b))
                            .child(language),
                    )
                    // Code preview
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_xs()
                            .text_color(rgb(0x5c6370))
                            .child(preview_text),
                    )
                    // Line count
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x4b5263))
                            .child(line_count_label),
                    ),
            );
        }

        div()
            .id("file-preview-list")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .overflow_y_scroll()
            .child(
                div()
                    .p_4()
                    .child(
                        div()
                            .text_color(rgb(0x61afef))
                            .text_lg()
                            .child("Code Samples"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x5c6370))
                            .text_sm()
                            .mt_1()
                            .child("Tap a file to open in editor"),
                    ),
            )
            .child(grid)
    }
}
