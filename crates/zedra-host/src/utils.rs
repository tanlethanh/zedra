use std::io::IsTerminal;
use std::path::Path;

const COMMAND_STYLE: &str = "1;36";
const ERROR_STYLE: &str = "1;31";
const HEADING_STYLE: &str = "1";
const SUCCESS_STYLE: &str = "1;32";
const WARNING_STYLE: &str = "33";

pub fn stdout_is_terminal() -> bool {
    std::io::stdout().is_terminal()
}

pub fn stderr_is_terminal() -> bool {
    std::io::stderr().is_terminal()
}

pub fn stdout_color(text: impl AsRef<str>, style: &str) -> String {
    color_text(text.as_ref(), style, stdout_is_terminal())
}

pub fn stderr_color(text: impl AsRef<str>, style: &str) -> String {
    color_text(text.as_ref(), style, stderr_is_terminal())
}

pub fn command_text(command: impl AsRef<str>) -> String {
    stdout_color(command, COMMAND_STYLE)
}

pub fn shell_command_text(command: impl AsRef<str>) -> String {
    command_text(format!("$ {}", command.as_ref()))
}

pub fn success_text(text: impl AsRef<str>) -> String {
    stdout_color(text, SUCCESS_STYLE)
}

pub fn error_text(text: impl AsRef<str>) -> String {
    stdout_color(text, ERROR_STYLE)
}

pub fn warning_text(text: impl AsRef<str>) -> String {
    stdout_color(text, WARNING_STYLE)
}

pub fn heading_text(text: impl AsRef<str>) -> String {
    stdout_color(text, HEADING_STYLE)
}

pub fn println_heading(title: impl AsRef<str>) {
    println!("{}", heading_text(title));
}

pub fn eprintln_heading(title: impl AsRef<str>) {
    eprintln!("{}", stderr_color(title, HEADING_STYLE));
}

pub fn println_command(command: impl AsRef<str>) {
    println!("  {}", command_text(command));
}

pub fn println_shell_command(command: impl AsRef<str>) {
    println!("  {}", shell_command_text(command));
}

pub fn eprintln_shell_command(command: impl AsRef<str>) {
    eprintln!(
        "  {}",
        stderr_color(format!("$ {}", command.as_ref()), COMMAND_STYLE)
    );
}

pub fn render_shell_command_list(rows: &[(&str, &str)]) -> String {
    let width = rows
        .iter()
        .map(|(command, _)| format!("$ {command}").len())
        .max()
        .unwrap_or(0);

    rows.iter()
        .map(|(command, description)| {
            let plain = format!("$ {command}");
            let padding = " ".repeat(width.saturating_sub(plain.len()));
            format!("  {}{}  {description}", command_text(plain), padding)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn println_step(label: impl AsRef<str>) {
    println!("> {}", label.as_ref());
}

pub fn eprintln_step(label: impl AsRef<str>) {
    eprintln!("> {}", label.as_ref());
}

pub fn println_success(message: impl AsRef<str>) {
    println!("{}", success_text(message));
}

pub fn eprintln_success(message: impl AsRef<str>) {
    eprintln!("{}", stderr_color(message, SUCCESS_STYLE));
}

pub fn println_warn(message: impl AsRef<str>) {
    println!("{}", warning_text(message));
}

pub fn println_error(message: impl AsRef<str>) {
    println!("{}", error_text(message));
}

pub fn eprintln_error(message: impl AsRef<str>) {
    eprintln!("{}", stderr_color(message, ERROR_STYLE));
}

pub fn println_note(message: impl AsRef<str>) {
    println!("{}", message.as_ref());
}

pub fn eprintln_note(message: impl AsRef<str>) {
    eprintln!("{}", message.as_ref());
}

/// Print a yellow warning line to stderr when attached to a terminal.
/// Falls back to plain text for redirected output.
pub fn eprintln_warn(message: impl AsRef<str>) {
    eprintln!("{}", stderr_color(message, WARNING_STYLE));
}

pub fn render_key_values(rows: &[(&str, String)]) -> String {
    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    rows.iter()
        .map(|(label, value)| format!("  {label:<width$}  {value}", width = width))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn print_key_values(rows: &[(&str, String)]) {
    println!("{}", render_key_values(rows));
}

pub fn eprintln_key_values(rows: &[(&str, String)]) {
    eprintln!("{}", render_key_values(rows));
}

pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate().take(widths.len()) {
            widths[index] = widths[index].max(cell.len());
        }
    }

    let mut lines = Vec::with_capacity(rows.len() + 2);
    lines.push(format_table_row(
        &headers
            .iter()
            .map(|header| header.to_string())
            .collect::<Vec<_>>(),
        &widths,
    ));
    lines.push(
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("  "),
    );
    for row in rows {
        lines.push(format_table_row(row, &widths));
    }
    lines.join("\n")
}

pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub fn shell_arg_path(path: &Path) -> String {
    shell_arg(&path.display().to_string())
}

pub fn shell_arg(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ':' | '='))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn format_table_row(cells: &[String], widths: &[usize]) -> String {
    (0..widths.len())
        .map(|index| {
            let cell = cells.get(index).map(String::as_str).unwrap_or("");
            format!("{cell:<width$}", width = widths[index])
        })
        .collect::<Vec<_>>()
        .join("  ")
}

fn color_text(text: &str, style: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{style}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_table_uses_stable_columns() {
        assert_eq!(
            render_table(
                &["PID", "STATE", "WORKDIR"],
                &[
                    vec!["1".to_string(), "ready".to_string(), "/a".to_string()],
                    vec!["22".to_string(), "no-api".to_string(), "/long".to_string()],
                ],
            ),
            "PID  STATE   WORKDIR\n---  ------  -------\n1    ready   /a     \n22   no-api  /long  "
        );
    }

    #[test]
    fn render_key_values_aligns_labels() {
        assert_eq!(
            render_key_values(&[("PID", "123".to_string()), ("Workdir", "/repo".to_string()),]),
            "  PID      123\n  Workdir  /repo"
        );
    }

    #[test]
    fn render_shell_command_list_aligns_descriptions() {
        assert_eq!(
            render_shell_command_list(&[("zedra qr", "Print QR"), ("zedra logs", "Show logs"),]),
            "  $ zedra qr    Print QR\n  $ zedra logs  Show logs"
        );
    }

    #[test]
    fn shell_arg_quotes_only_when_needed() {
        assert_eq!(shell_arg("/repo/project"), "/repo/project");
        assert_eq!(shell_arg("/repo/with space"), "'/repo/with space'");
        assert_eq!(shell_arg("/repo/that's-it"), "'/repo/that'\\''s-it'");
    }

    #[test]
    fn format_duration_uses_compact_units() {
        assert_eq!(format_duration(59), "59s");
        assert_eq!(format_duration(65), "1m5s");
        assert_eq!(format_duration(3661), "1h1m");
    }
}
