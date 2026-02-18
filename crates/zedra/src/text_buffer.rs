use std::ops::Range;

/// A simple text buffer with line indexing.
///
/// Backed by a plain `String` with a cached line-start index for O(1) line
/// lookups. Sufficient for a mobile file viewer — no rope needed.
pub struct Buffer {
    text: String,
    /// Byte offsets of the start of each line (including line 0 at offset 0).
    line_starts: Vec<usize>,
}

impl Buffer {
    pub fn new(text: String) -> Self {
        let line_starts = Self::compute_line_starts(&text);
        Self { text, line_starts }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn line_text(&self, line: usize) -> &str {
        let range = self.line_byte_range(line);
        let end = range.end.min(self.text.len());
        // Strip trailing newline for display
        let slice = &self.text[range.start..end];
        slice.strip_suffix('\n').unwrap_or(slice)
    }

    pub fn line_byte_range(&self, line: usize) -> Range<usize> {
        let start = self.line_starts.get(line).copied().unwrap_or(self.text.len());
        let end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len());
        start..end
    }

    pub fn offset_to_point(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(insert) => insert.saturating_sub(1),
        };
        let col = offset - self.line_starts[line];
        (line, col)
    }

    pub fn point_to_offset(&self, row: usize, col: usize) -> usize {
        if row >= self.line_starts.len() {
            return self.text.len();
        }
        let line_start = self.line_starts[row];
        let line_end = self
            .line_starts
            .get(row + 1)
            .copied()
            .unwrap_or(self.text.len());
        (line_start + col).min(line_end)
    }

    /// Insert `text` at `offset`, recomputing the line index.
    pub fn insert(&mut self, offset: usize, new_text: &str) {
        let offset = offset.min(self.text.len());
        self.text.insert_str(offset, new_text);
        self.line_starts = Self::compute_line_starts(&self.text);
    }

    /// Delete the byte range, recomputing the line index.
    pub fn delete(&mut self, range: Range<usize>) {
        let start = range.start.min(self.text.len());
        let end = range.end.min(self.text.len());
        if start < end {
            self.text.drain(start..end);
            self.line_starts = Self::compute_line_starts(&self.text);
        }
    }

    /// Replace the entire buffer content.
    pub fn set_text(&mut self, text: String) {
        self.line_starts = Self::compute_line_starts(&text);
        self.text = text;
    }

    fn compute_line_starts(text: &str) -> Vec<usize> {
        let mut starts = vec![0];
        for (i, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(i + 1);
            }
        }
        starts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_count() {
        let buffer = Buffer::new("hello\nworld\n".to_string());
        assert_eq!(buffer.line_count(), 3); // "hello\n", "world\n", ""
    }

    #[test]
    fn test_line_text() {
        let buffer = Buffer::new("fn main() {\n    println!(\"hi\");\n}\n".to_string());
        assert_eq!(buffer.line_text(0), "fn main() {");
        assert_eq!(buffer.line_text(1), "    println!(\"hi\");");
        assert_eq!(buffer.line_text(2), "}");
    }

    #[test]
    fn test_offset_to_point() {
        let buffer = Buffer::new("abc\ndef\n".to_string());
        assert_eq!(buffer.offset_to_point(0), (0, 0));
        assert_eq!(buffer.offset_to_point(3), (0, 3));
        assert_eq!(buffer.offset_to_point(4), (1, 0));
        assert_eq!(buffer.offset_to_point(7), (1, 3));
    }

    #[test]
    fn test_insert_delete() {
        let mut buffer = Buffer::new("hello".to_string());
        buffer.insert(5, " world");
        assert_eq!(buffer.text(), "hello world");

        buffer.delete(5..6);
        assert_eq!(buffer.text(), "helloworld");
    }
}
