use std::ops::Range;

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator, Tree};

/// Highlight queries for Rust, embedded from vendor/zed's language definitions.
const RUST_HIGHLIGHTS_SCM: &str =
    include_str!("../../../vendor/zed/crates/languages/src/rust/highlights.scm");

/// Wraps tree-sitter parsing and highlight query execution.
pub struct Highlighter {
    parser: Parser,
    query: Query,
    tree: Option<Tree>,
}

impl Highlighter {
    /// Create a highlighter for the given language and query source.
    pub fn new(language: Language, query_source: &str) -> Result<Self, tree_sitter::QueryError> {
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .expect("language version mismatch");
        let query = Query::new(&language, query_source)?;
        Ok(Self {
            parser,
            query,
            tree: None,
        })
    }

    /// Create a highlighter configured for Rust syntax.
    pub fn rust() -> Self {
        Self::new(tree_sitter_rust::LANGUAGE.into(), RUST_HIGHLIGHTS_SCM)
            .expect("built-in Rust highlight query should be valid")
    }

    /// Parse the full source text, storing the resulting tree for queries.
    pub fn parse(&mut self, source: &str) {
        self.tree = self.parser.parse(source, self.tree.as_ref());
    }

    /// Return highlight spans for the given byte range of the source.
    ///
    /// Each span is `(byte_range, capture_name)` where `capture_name` is the
    /// tree-sitter query capture (e.g. `"keyword"`, `"function"`, `"type"`).
    pub fn highlights<'a>(&'a self, source: &'a str, range: Range<usize>) -> Vec<(Range<usize>, &'a str)> {
        let tree = match &self.tree {
            Some(tree) => tree,
            None => return Vec::new(),
        };

        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(range.clone());

        let mut result = Vec::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source.as_bytes());
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                let capture_name = &self.query.capture_names()[capture.index as usize];
                let node_range = capture.node.byte_range();
                // Only include captures that overlap our requested range
                if node_range.start < range.end && node_range.end > range.start {
                    result.push((node_range, &**capture_name));
                }
            }
        }

        // Sort by start position, then by longest span first (outer captures first)
        result.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(b.0.end.cmp(&a.0.end)));
        result
    }
}
