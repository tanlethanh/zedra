# Syntax Highlighting Examples

A reference document covering all languages supported by Zedra's syntax highlighter.

## Supported Languages

| Language   | Extension(s)                     | Tree-sitter Crate          |
|------------|----------------------------------|----------------------------|
| Rust       | `.rs`                            | `tree-sitter-rust`         |
| Python     | `.py`, `.pyi`                    | `tree-sitter-python`       |
| Go         | `.go`                            | `tree-sitter-go`           |
| JavaScript | `.js`, `.mjs`, `.cjs`, `.jsx`    | `tree-sitter-javascript`   |
| TypeScript | `.ts`, `.mts`, `.cts`            | `tree-sitter-typescript`   |
| TSX        | `.tsx`                           | `tree-sitter-typescript`   |
| C          | `.c`, `.h`                       | `tree-sitter-c`            |
| C++        | `.cpp`, `.cc`, `.hpp`            | `tree-sitter-cpp`          |
| CSS        | `.css`                           | `tree-sitter-css`          |
| JSON       | `.json`, `.jsonc`                | `tree-sitter-json`         |
| YAML       | `.yaml`, `.yml`                  | `tree-sitter-yaml`         |
| Bash       | `.sh`, `.bash`, `.zsh`           | `tree-sitter-bash`         |
| Markdown   | `.md`, `.markdown`               | `tree-sitter-md`           |
| HTML       | `.html`, `.htm`                  | `tree-sitter-html`         |
| Ruby       | `.rb`, `.rake`, `.gemspec`       | `tree-sitter-ruby`         |
| Java       | `.java`                          | `tree-sitter-java`         |
| C#         | `.cs`                            | `tree-sitter-c-sharp`      |
| PHP        | `.php`, `.phtml`                 | `tree-sitter-php`          |

## How Highlighting Works

1. **Detection** — `Language::from_filename()` maps the file extension to a `Language` variant.
2. **Parsing** — `Highlighter::parse()` builds a concrete syntax tree via tree-sitter.
3. **Queries** — `.highlights()` runs the language's `highlights.scm` query over the requested byte range.
4. **Theming** — `SyntaxTheme::get()` maps capture names (`keyword`, `function`, `type`, …) to `HighlightStyle` colours using longest-prefix matching.

## Inline Code Samples

```rust
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}
```

```python
def greet(name: str) -> str:
    return f"Hello, {name}!"
```

```go
func greet(name string) string {
    return "Hello, " + name + "!"
}
```

## Testing Checklist

- [ ] Keywords render in purple
- [ ] Strings render in green
- [ ] Comments render in gray
- [ ] Types render in yellow
- [ ] Functions render in blue
- [ ] Numbers render in orange
- [ ] Operators render in purple
- [ ] Scroll performance stays at 60 FPS on large files
