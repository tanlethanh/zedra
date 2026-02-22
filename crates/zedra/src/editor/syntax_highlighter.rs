use std::ops::Range;

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator, Tree};

/// Highlight queries for Rust, embedded from vendor/zed's language definitions.
const RUST_HIGHLIGHTS_SCM: &str =
    include_str!("../../../../vendor/zed/crates/languages/src/rust/highlights.scm");

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
    /// When parsing different source texts (like individual diff lines), we
    /// don't reuse the old tree to avoid stale node references.
    pub fn parse(&mut self, source: &str) {
        // Always create a fresh parse to avoid issues with stale tree references
        // when parsing unrelated snippets (like individual diff lines)
        self.tree = self.parser.parse(source, None);
    }

    /// Return highlight spans for the given byte range of the source.
    ///
    /// Each span is `(byte_range, capture_name)` where `capture_name` is the
    /// tree-sitter query capture (e.g. `"keyword"`, `"function"`, `"type"`).
    pub fn highlights<'a>(
        &'a self,
        source: &'a str,
        range: Range<usize>,
    ) -> Vec<(Range<usize>, &'a str)> {
        let tree = match &self.tree {
            Some(tree) => tree,
            None => return Vec::new(),
        };

        let source_len = source.len();
        // Clamp the requested range to source bounds
        let safe_range = range.start.min(source_len)..range.end.min(source_len);
        if safe_range.is_empty() {
            return Vec::new();
        }

        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(safe_range.clone());

        let mut result = Vec::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source.as_bytes());
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                let capture_name = &self.query.capture_names()[capture.index as usize];
                let node_range = capture.node.byte_range();
                // Clamp node range to source bounds to prevent out-of-bounds access
                let clamped_start = node_range.start.min(source_len);
                let clamped_end = node_range.end.min(source_len);
                // Only include captures that overlap our requested range and have valid bounds
                if clamped_start < clamped_end
                    && clamped_start < safe_range.end
                    && clamped_end > safe_range.start
                {
                    result.push((clamped_start..clamped_end, &**capture_name));
                }
            }
        }

        // Sort by start position, then by longest span first (outer captures first)
        result.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(b.0.end.cmp(&a.0.end)));
        result
    }
}
