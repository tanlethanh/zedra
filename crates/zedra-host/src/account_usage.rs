//! Shared parsing for structured provider usage API responses (Claude OAuth, Codex).

use chrono::DateTime;
use serde_json::Value;

/// Parse `resets_at` / `reset_at` from a provider usage window object.
pub fn parse_usage_window_resets_at(window: &Value) -> Option<i64> {
    let raw = window.get("resets_at").or_else(|| window.get("reset_at"))?;
    if let Some(secs) = raw.as_i64() {
        return Some(normalize_unix_seconds(secs));
    }
    if let Some(f) = raw.as_f64() {
        return Some(normalize_unix_seconds(f as i64));
    }
    let text = raw.as_str()?;
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return Some(dt.timestamp());
    }
    DateTime::parse_from_str(text, "%+")
        .ok()
        .map(|dt| dt.timestamp())
}

fn normalize_unix_seconds(secs: i64) -> i64 {
    if secs > 1_000_000_000_000 {
        secs / 1000
    } else {
        secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    #[test]
    fn parse_usage_window_resets_at_accepts_unix_and_rfc3339() {
        let unix = serde_json::json!({ "resets_at": 1_700_000_000 });
        assert_eq!(parse_usage_window_resets_at(&unix), Some(1_700_000_000));
        let ms = serde_json::json!({ "resets_at": 1_700_000_000_000_i64 });
        assert_eq!(parse_usage_window_resets_at(&ms), Some(1_700_000_000));
        let rfc = serde_json::json!({ "resets_at": "2026-01-02T22:59:00Z" });
        assert_eq!(
            parse_usage_window_resets_at(&rfc),
            Some(
                DateTime::parse_from_rfc3339("2026-01-02T22:59:00Z")
                    .unwrap()
                    .timestamp()
            )
        );
    }
}
