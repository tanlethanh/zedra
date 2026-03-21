use std::ops::Range;

use tree_sitter::{Language as TSLanguage, Parser, Query, QueryCursor, StreamingIterator, Tree};

const RUST_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/rust/highlights.scm");
const PYTHON_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/python/highlights.scm");
const GO_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/go/highlights.scm");
const JAVASCRIPT_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/javascript/highlights.scm");
const TYPESCRIPT_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/typescript/highlights.scm");
const TSX_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/tsx/highlights.scm");
const C_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/c/highlights.scm");
const CPP_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/cpp/highlights.scm");
const CSS_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/css/highlights.scm");
const JSON_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/json/highlights.scm");
const YAML_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/yaml/highlights.scm");
const BASH_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/bash/highlights.scm");
const MARKDOWN_HIGHLIGHTS: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/markdown/highlights.scm");
const CSHARP_HIGHLIGHTS: &str = include_str!("queries/csharp/highlights.scm");

/// Supported programming languages for syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Python,
    Go,
    JavaScript,
    TypeScript,
    Tsx,
    C,
    Cpp,
    Css,
    Json,
    Yaml,
    Bash,
    Markdown,
    Html,
    Ruby,
    Java,
    CSharp,
    Php,
    PlainText,
}

impl Language {
    /// Detect language from filename extension.
    pub fn from_filename(filename: &str) -> Self {
        let ext = filename.rsplit('.').next().unwrap_or("");
        match ext.to_lowercase().as_str() {
            "rs" => Language::Rust,
            "py" | "pyi" | "pyw" => Language::Python,
            "go" => Language::Go,
            "js" | "mjs" | "cjs" => Language::JavaScript,
            "ts" | "mts" | "cts" => Language::TypeScript,
            "tsx" => Language::Tsx,
            "jsx" => Language::JavaScript,
            "c" | "h" => Language::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => Language::Cpp,
            "css" => Language::Css,
            "json" | "jsonc" => Language::Json,
            "yaml" | "yml" => Language::Yaml,
            "sh" | "bash" | "zsh" => Language::Bash,
            "md" | "markdown" => Language::Markdown,
            "html" | "htm" => Language::Html,
            "rb" | "rake" | "gemspec" => Language::Ruby,
            "java" => Language::Java,
            "cs" => Language::CSharp,
            "php" | "phtml" | "php3" | "php4" | "php5" => Language::Php,
            _ => Language::PlainText,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Language::Rust => "Rust",
            Language::Python => "Python",
            Language::Go => "Go",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Tsx => "TSX",
            Language::C => "C",
            Language::Cpp => "C++",
            Language::Css => "CSS",
            Language::Json => "JSON",
            Language::Yaml => "YAML",
            Language::Bash => "Bash",
            Language::Markdown => "Markdown",
            Language::Html => "HTML",
            Language::Ruby => "Ruby",
            Language::Java => "Java",
            Language::CSharp => "C#",
            Language::Php => "PHP",
            Language::PlainText => "Plain Text",
        }
    }

    fn grammar_and_query(&self) -> Option<(TSLanguage, &'static str)> {
        match self {
            Language::Rust => Some((tree_sitter_rust::LANGUAGE.into(), RUST_HIGHLIGHTS)),
            Language::Python => Some((tree_sitter_python::LANGUAGE.into(), PYTHON_HIGHLIGHTS)),
            Language::Go => Some((tree_sitter_go::LANGUAGE.into(), GO_HIGHLIGHTS)),
            Language::JavaScript => Some((
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                JAVASCRIPT_HIGHLIGHTS,
            )),
            Language::TypeScript => Some((
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                TYPESCRIPT_HIGHLIGHTS,
            )),
            Language::Tsx => {
                Some((tree_sitter_typescript::LANGUAGE_TSX.into(), TSX_HIGHLIGHTS))
            }
            Language::C => Some((tree_sitter_c::LANGUAGE.into(), C_HIGHLIGHTS)),
            Language::Cpp => Some((tree_sitter_cpp::LANGUAGE.into(), CPP_HIGHLIGHTS)),
            Language::Css => Some((tree_sitter_css::LANGUAGE.into(), CSS_HIGHLIGHTS)),
            Language::Json => Some((tree_sitter_json::LANGUAGE.into(), JSON_HIGHLIGHTS)),
            Language::Yaml => Some((tree_sitter_yaml::LANGUAGE.into(), YAML_HIGHLIGHTS)),
            Language::Bash => Some((tree_sitter_bash::LANGUAGE.into(), BASH_HIGHLIGHTS)),
            Language::Markdown => {
                Some((tree_sitter_md::LANGUAGE.into(), MARKDOWN_HIGHLIGHTS))
            }
            Language::Html => Some((
                tree_sitter_html::LANGUAGE.into(),
                tree_sitter_html::HIGHLIGHTS_QUERY,
            )),
            Language::Ruby => Some((
                tree_sitter_ruby::LANGUAGE.into(),
                tree_sitter_ruby::HIGHLIGHTS_QUERY,
            )),
            Language::Java => Some((
                tree_sitter_java::LANGUAGE.into(),
                tree_sitter_java::HIGHLIGHTS_QUERY,
            )),
            Language::CSharp => Some((
                tree_sitter_c_sharp::LANGUAGE.into(),
                CSHARP_HIGHLIGHTS,
            )),
            Language::Php => Some((
                tree_sitter_php::LANGUAGE_PHP.into(),
                tree_sitter_php::HIGHLIGHTS_QUERY,
            )),
            Language::PlainText => None,
        }
    }
}

/// Wraps tree-sitter parsing and highlight query execution.
pub struct Highlighter {
    parser: Option<Parser>,
    query: Option<Query>,
    tree: Option<Tree>,
    language: Language,
}

impl Highlighter {
    /// Create a highlighter for the specified language.
    pub fn new(language: Language) -> Self {
        match language.grammar_and_query() {
            Some((ts_lang, query_source)) => {
                let mut parser = Parser::new();
                parser
                    .set_language(&ts_lang)
                    .expect("language version mismatch");

                match Query::new(&ts_lang, query_source) {
                    Ok(query) => Self {
                        parser: Some(parser),
                        query: Some(query),
                        tree: None,
                        language,
                    },
                    Err(e) => {
                        tracing::warn!("Failed to parse highlight query for {:?}: {}", language, e);
                        Self {
                            parser: None,
                            query: None,
                            tree: None,
                            language,
                        }
                    }
                }
            }
            None => Self {
                parser: None,
                query: None,
                tree: None,
                language,
            },
        }
    }

    /// Create a highlighter configured for Rust syntax.
    pub fn rust() -> Self {
        Self::new(Language::Rust)
    }

    /// Create a highlighter based on filename extension.
    pub fn from_filename(filename: &str) -> Self {
        Self::new(Language::from_filename(filename))
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// Parse the full source text, storing the resulting tree for queries.
    pub fn parse(&mut self, source: &str) {
        if let Some(ref mut parser) = self.parser {
            self.tree = parser.parse(source, self.tree.as_ref());
        }
    }

    /// Parse without reusing the previous tree. Use when the new source is
    /// unrelated to the previous parse (e.g. a different file or diff line),
    /// to avoid stale byte offsets from the old tree causing out-of-bounds panics.
    pub fn parse_fresh(&mut self, source: &str) {
        if let Some(ref mut parser) = self.parser {
            self.tree = parser.parse(source, None);
        }
    }

    /// Return highlight spans for the given byte range of the source.
    ///
    /// Each span is `(byte_range, capture_name)`.
    pub fn highlights<'a>(
        &'a self,
        source: &'a str,
        range: Range<usize>,
    ) -> Vec<(Range<usize>, &'a str)> {
        let (tree, query) = match (&self.tree, &self.query) {
            (Some(tree), Some(query)) => (tree, query),
            _ => return Vec::new(),
        };

        let source_len = source.len();
        let safe_range = range.start.min(source_len)..range.end.min(source_len);
        if safe_range.is_empty() {
            return Vec::new();
        }

        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(safe_range.clone());

        let mut result = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                let capture_name = &query.capture_names()[capture.index as usize];
                let node_range = capture.node.byte_range();
                let clamped_start = node_range.start.min(source_len);
                let clamped_end = node_range.end.min(source_len);
                if clamped_start < clamped_end
                    && clamped_start < safe_range.end
                    && clamped_end > safe_range.start
                {
                    result.push((clamped_start..clamped_end, &**capture_name));
                }
            }
        }

        result.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(b.0.end.cmp(&a.0.end)));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_query(name: &str, lang: TSLanguage, query_src: &'static str) -> bool {
        match Query::new(&lang, query_src) {
            Ok(_)  => { println!("{name}: OK"); true }
            Err(e) => { println!("{name}: ERROR - {e}"); false }
        }
    }

    #[test]
    fn all_queries_parse() {
        let mut ok = true;
        ok &= check_query("rust",       tree_sitter_rust::LANGUAGE.into(),       RUST_HIGHLIGHTS);
        ok &= check_query("python",     tree_sitter_python::LANGUAGE.into(),     PYTHON_HIGHLIGHTS);
        ok &= check_query("go",         tree_sitter_go::LANGUAGE.into(),         GO_HIGHLIGHTS);
        ok &= check_query("javascript", tree_sitter_typescript::LANGUAGE_TSX.into(), JAVASCRIPT_HIGHLIGHTS);
        ok &= check_query("typescript", tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), TYPESCRIPT_HIGHLIGHTS);
        ok &= check_query("tsx",        tree_sitter_typescript::LANGUAGE_TSX.into(), TSX_HIGHLIGHTS);
        ok &= check_query("c",          tree_sitter_c::LANGUAGE.into(),          C_HIGHLIGHTS);
        ok &= check_query("cpp",        tree_sitter_cpp::LANGUAGE.into(),        CPP_HIGHLIGHTS);
        ok &= check_query("css",        tree_sitter_css::LANGUAGE.into(),        CSS_HIGHLIGHTS);
        ok &= check_query("json",       tree_sitter_json::LANGUAGE.into(),       JSON_HIGHLIGHTS);
        ok &= check_query("yaml",       tree_sitter_yaml::LANGUAGE.into(),       YAML_HIGHLIGHTS);
        ok &= check_query("bash",       tree_sitter_bash::LANGUAGE.into(),       BASH_HIGHLIGHTS);
        ok &= check_query("markdown",   tree_sitter_md::LANGUAGE.into(),         MARKDOWN_HIGHLIGHTS);
        ok &= check_query("html",       tree_sitter_html::LANGUAGE.into(),       tree_sitter_html::HIGHLIGHTS_QUERY);
        ok &= check_query("ruby",       tree_sitter_ruby::LANGUAGE.into(),       tree_sitter_ruby::HIGHLIGHTS_QUERY);
        ok &= check_query("java",       tree_sitter_java::LANGUAGE.into(),       tree_sitter_java::HIGHLIGHTS_QUERY);
        ok &= check_query("csharp",     tree_sitter_c_sharp::LANGUAGE.into(),    CSHARP_HIGHLIGHTS);
        ok &= check_query("php",        tree_sitter_php::LANGUAGE_PHP.into(),    tree_sitter_php::HIGHLIGHTS_QUERY);
        assert!(ok, "one or more highlight queries failed to parse");
    }

    #[test]
    fn highlight_js_and_cpp_produce_captures() {
        let js_src = "function greet(name) { return 'Hello ' + name; }";
        let mut js_hl = Highlighter::new(Language::JavaScript);
        js_hl.parse(js_src);
        let js_caps: Vec<_> = js_hl.highlights(js_src, 0..js_src.len());
        println!("JS captures ({}):", js_caps.len());
        for (range, name) in &js_caps {
            println!("  {:?} -> {name}", &js_src[range.clone()]);
        }
        assert!(!js_caps.is_empty(), "JS produced no highlights");

        let cpp_src = "#include <vector>\nint main() { return 0; }";
        let mut cpp_hl = Highlighter::new(Language::Cpp);
        cpp_hl.parse(cpp_src);
        let cpp_caps: Vec<_> = cpp_hl.highlights(cpp_src, 0..cpp_src.len());
        println!("C++ captures ({}):", cpp_caps.len());
        for (range, name) in &cpp_caps {
            println!("  {:?} -> {name}", &cpp_src[range.clone()]);
        }
        assert!(!cpp_caps.is_empty(), "C++ produced no highlights");

        // Test with more complex JS to exercise class/arrow/template patterns
        let complex_js = r#"class Foo extends Bar {
  constructor(x) { this.x = x; }
  greet = (name) => `Hello ${name}`;
  static create() { return new Foo(0); }
}
const MAX = 100;
import { foo } from './bar';
"#;
        let mut js2 = Highlighter::new(Language::JavaScript);
        js2.parse(complex_js);
        let caps2: Vec<_> = js2.highlights(complex_js, 0..complex_js.len());
        println!("Complex JS captures ({}):", caps2.len());
        for (range, name) in &caps2 {
            println!("  {:?} -> {name}", &complex_js[range.clone()]);
        }
        assert!(caps2.len() > 14, "Complex JS should produce more than 14 captures, got {}", caps2.len());
    }
}
