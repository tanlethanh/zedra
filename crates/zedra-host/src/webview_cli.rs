use anyhow::Result;
use clap::Args;

use crate::terminal_cli::{api_post, resolve_workdir};

#[derive(Debug, Args)]
pub struct OpenArgs {
    /// Target to open on the phone. A bare port (`8080`), `host:port`
    /// (`localhost:8080`), or a full URL (`http://localhost:5173/app`).
    /// Loopback targets tunnel through the session; anything else opens in the
    /// system browser.
    pub target: String,

    /// Working directory of the running daemon
    #[arg(short, long, default_value = ".")]
    pub workdir: String,
}

pub async fn run(args: OpenArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let url = normalize_target(&args.target);
    let body = serde_json::json!({ "url": url });
    let resp: serde_json::Value = api_post(&workdir, "/api/webview", &body).await?;
    if resp.get("delivered").and_then(|v| v.as_bool()) == Some(true) {
        println!("Opening {url} on the connected phone");
    } else {
        println!("Queued {url} — it will open when a phone connects");
    }
    Ok(())
}

/// Turn a user target into a URL: pass through an explicit scheme, otherwise
/// treat a bare port or `host:port` as `http://` loopback.
fn normalize_target(target: &str) -> String {
    let target = target.trim();
    if target.contains("://") {
        return target.to_string();
    }
    if let Ok(port) = target.parse::<u16>() {
        return format!("http://localhost:{port}");
    }
    format!("http://{target}")
}

#[cfg(test)]
mod tests {
    use super::normalize_target;

    #[test]
    fn normalizes_targets() {
        assert_eq!(normalize_target("8080"), "http://localhost:8080");
        assert_eq!(normalize_target("localhost:5173"), "http://localhost:5173");
        assert_eq!(normalize_target("127.0.0.1:3000"), "http://127.0.0.1:3000");
        assert_eq!(
            normalize_target("http://localhost:5173/app"),
            "http://localhost:5173/app"
        );
        assert_eq!(
            normalize_target("https://localhost:8443"),
            "https://localhost:8443"
        );
    }
}
