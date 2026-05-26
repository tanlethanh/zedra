//! Language registry. Hardcoded for now; an extension surface lands later.

use zedra_rpc::proto::LspLanguage;

/// Binary + argv for a language server. The binary is looked up on `PATH`.
pub struct LanguageBinary {
    pub command: &'static str,
    pub args: &'static [&'static str],
}

/// Resolve the language server binary for `language`. Returns `None` for
/// languages with no built-in mapping.
pub fn language_binary(language: LspLanguage) -> Option<LanguageBinary> {
    match language {
        LspLanguage::Rust => Some(LanguageBinary {
            command: "rust-analyzer",
            args: &[],
        }),
        LspLanguage::Go => Some(LanguageBinary {
            command: "gopls",
            args: &[],
        }),
        LspLanguage::TypeScript | LspLanguage::JavaScript => Some(LanguageBinary {
            command: "typescript-language-server",
            args: &["--stdio"],
        }),
        LspLanguage::Python => Some(LanguageBinary {
            command: "pyright-langserver",
            args: &["--stdio"],
        }),
    }
}

/// Languages this host can serve. Used by the CLI for `zedra lsp enable`
/// completion and validation.
pub fn supported_languages() -> &'static [LspLanguage] {
    &[
        LspLanguage::Rust,
        LspLanguage::Go,
        LspLanguage::TypeScript,
        LspLanguage::JavaScript,
        LspLanguage::Python,
    ]
}
