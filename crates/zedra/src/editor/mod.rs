// Editor: text buffer, syntax highlighting, code editor, git diff

pub mod code_editor;
pub mod git_diff_view;
pub mod git_sidebar;
pub mod syntax_highlighter;
pub mod syntax_theme;
pub mod text_buffer;

pub use syntax_highlighter::Language;
