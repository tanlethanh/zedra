use std::io::IsTerminal;

/// Print a yellow warning line to stderr when attached to a terminal.
/// Falls back to plain text for redirected output.
pub fn eprintln_warn(message: impl AsRef<str>) {
    let message = message.as_ref();
    if std::io::stderr().is_terminal() {
        eprintln!("\x1b[33m{}\x1b[0m", message);
    } else {
        eprintln!("{message}");
    }
}
