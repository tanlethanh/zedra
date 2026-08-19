use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use zedra_rpc::proto::*;

use super::utils::{
    command_on_path, command_output_with_timeout, cwd_matches, file_size_bytes, home_path,
    info_field, resume_summary, session_title, spawn_blocking_opt,
};
use super::{AgentActor, ScanCtx, SessionCounts as ActorSessionCounts};

/// `fx sessions --json` and the on-disk index share this shape, so one type
/// parses both the CLI page and the fallback store read.
#[derive(Debug, Deserialize)]
struct SessionPage {
    #[serde(default)]
    sessions: Vec<IndexEntry>,
    #[serde(default)]
    has_more: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct IndexEntry {
    id: String,
    #[serde(default)]
    workspace_root: Option<String>,
    #[serde(default)]
    created_at_ms: Option<i64>,
    #[serde(default)]
    updated_at_ms: Option<i64>,
    #[serde(default)]
    title: Option<String>,
}

fn timestamp(ms: Option<i64>) -> Option<DateTime<Utc>> {
    ms.and_then(DateTime::<Utc>::from_timestamp_millis)
}

/// fx pages sessions at 100; a wider request is an error, not a bigger page.
const SESSION_PAGE_MAX: usize = 100;
const CLI_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const NETWORK_CLI_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub(super) struct FxActor;

impl FxActor {
    fn state_dir() -> PathBuf {
        home_path(&[".fx"])
    }

    fn sessions_dir() -> PathBuf {
        Self::state_dir().join("sessions")
    }

    fn cli_available() -> bool {
        command_on_path("fx") || Self::state_dir().is_dir()
    }

    /// Run an `fx` subcommand and parse its `--json` output.
    fn run_json(
        args: &[&str],
        cwd: Option<&Path>,
        timeout: std::time::Duration,
    ) -> Result<serde_json::Value, String> {
        let output = command_output_with_timeout("fx", args, cwd, timeout)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            return Err(if stderr.is_empty() {
                format!("`fx {}` exited with {}", args.join(" "), output.status)
            } else {
                format!(
                    "`fx {}` exited with {}: {stderr}",
                    args.join(" "),
                    output.status
                )
            });
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|err| format!("failed to parse `fx {}` output: {err}", args.join(" ")))
    }

    /// Sessions for `workdir`, newest first. `fx sessions` is the source of
    /// truth — it is workspace-scoped by cwd and answers in single-digit ms —
    /// with the on-disk index as the fallback when the CLI is missing or fails.
    fn fetch_sessions(workdir: &Path, limit: usize) -> Result<(Vec<IndexEntry>, usize), String> {
        let page_limit = limit.clamp(1, SESSION_PAGE_MAX);
        let cli_error = match Self::cli_sessions(workdir, page_limit) {
            Ok((entries, has_more)) => {
                let count = entries.len();
                if !has_more {
                    return Ok((entries, count));
                }
                // The page is capped, so the exact total comes from the index;
                // a missing index degrades to "at least this page".
                let total = Self::scan_sessions(&Self::sessions_dir(), workdir, None)
                    .map(|(_, total)| total)
                    .unwrap_or(count);
                return Ok((entries, total.max(count)));
            }
            Err(error) => error,
        };
        Self::scan_sessions(&Self::sessions_dir(), workdir, Some(limit))
            .map_err(|fallback| format!("{cli_error}; session index fallback failed: {fallback}"))
    }

    fn cli_sessions(workdir: &Path, limit: usize) -> Result<(Vec<IndexEntry>, bool), String> {
        let limit = limit.to_string();
        let value = Self::run_json(
            &["sessions", "--json", "--limit", &limit],
            Some(workdir),
            CLI_TIMEOUT,
        )?;
        let page: SessionPage = serde_json::from_value(value)
            .map_err(|err| format!("failed to parse `fx sessions` output: {err}"))?;
        let entries = page
            .sessions
            .into_iter()
            .filter(|entry| !entry.id.trim().is_empty())
            .collect();
        Ok((entries, page.has_more))
    }

    /// Index entries for `workdir`, newest first. `total` is every match; the
    /// returned vec is capped by `limit`.
    fn scan_sessions(
        dir: &Path,
        workdir: &Path,
        limit: Option<usize>,
    ) -> Result<(Vec<IndexEntry>, usize), String> {
        let path = dir.join("index.json");
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
            Err(err) => return Err(format!("failed to read {}: {err}", path.display())),
        };
        let index: SessionPage = serde_json::from_str(&contents)
            .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;

        let mut matched: Vec<IndexEntry> = index
            .sessions
            .into_iter()
            .filter(|entry| {
                !entry.id.trim().is_empty() && cwd_matches(workdir, entry.workspace_root.as_deref())
            })
            .collect();
        matched.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| b.id.cmp(&a.id))
        });

        let total = matched.len();
        if let Some(limit) = limit {
            matched.truncate(limit);
        }
        Ok((matched, total))
    }

    fn session_summary(&self, entry: &IndexEntry) -> AgentSessionSummary {
        let transcript = Self::sessions_dir().join(&entry.id).join("events.jsonl");
        AgentSessionSummary {
            slug: "fx".to_string(),
            session_id: entry.id.clone(),
            title: session_title(entry.title.clone()),
            cwd: entry.workspace_root.clone(),
            created_at: timestamp(entry.created_at_ms),
            last_activity_at: timestamp(entry.updated_at_ms),
            resume: resume_summary("fx", &entry.id),
            git: None,
            usage: None,
            transcript_size_bytes: file_size_bytes(&transcript),
        }
    }

    /// `fx status --json`: resolved model, auth mode, Vercel team, permission
    /// mode. One local call, no store parsing.
    fn status(workdir: &Path) -> Option<serde_json::Value> {
        match Self::run_json(&["status", "--json"], Some(workdir), CLI_TIMEOUT) {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::info!("fx: status probe failed: {error}");
                None
            }
        }
    }

    /// Auth mode reported by `fx status`; falls back to the stored credential
    /// source so a missing CLI still shows how the user signed in.
    fn auth_label(status: Option<&serde_json::Value>) -> Option<String> {
        if let Some(auth) = status
            .and_then(|status| status.get("auth"))
            .and_then(serde_json::Value::as_str)
            .filter(|auth| !auth.is_empty())
        {
            return Some(auth.to_string());
        }
        let contents = std::fs::read_to_string(Self::state_dir().join("settings.json")).ok()?;
        let settings: serde_json::Value = serde_json::from_str(&contents).ok()?;
        let source = settings.get("credential_source")?.as_str()?;
        Some(
            match source {
                "fx_login" => "fx login",
                "api_key" => "AI Gateway API key",
                other => other,
            }
            .to_string(),
        )
    }

    /// `fx credits --json`: AI Gateway balance and, when the account has one,
    /// its plan. This is the only network call the actor makes.
    fn credits_fields() -> Option<Vec<AgentInfoField>> {
        let value = match Self::run_json(&["credits", "--json"], None, NETWORK_CLI_TIMEOUT) {
            Ok(value) => value,
            Err(error) => {
                tracing::info!("fx: credits probe failed: {error}");
                return None;
            }
        };
        Self::credits_fields_from(&value)
    }

    fn credits_fields_from(value: &serde_json::Value) -> Option<Vec<AgentInfoField>> {
        let mut fields = Vec::new();
        if let Some(plan) = string_or_number(value.get("plan")) {
            fields.push(info_field(
                "Plan",
                &super::utils::humanize_plan_token(&plan),
            ));
        }
        if let Some(balance) = string_or_number(value.get("balance")) {
            fields.push(info_field("Gateway credits", &balance));
        }
        if let Some(used) = string_or_number(value.get("used")) {
            fields.push(info_field("Credits used", &used));
        }
        (!fields.is_empty()).then_some(fields)
    }

    /// `fx usage --json`: locally recorded token spend. fx has no rate-limit
    /// windows, so the snapshot carries counters only.
    fn usage_snapshot() -> Option<AgentUsageSnapshot> {
        let value = match Self::run_json(&["usage", "--json", "--period", "30d"], None, CLI_TIMEOUT)
        {
            Ok(value) => value,
            Err(error) => {
                tracing::info!("fx: usage probe failed: {error}");
                return None;
            }
        };
        Self::usage_snapshot_from(&value)
    }

    fn usage_snapshot_from(value: &serde_json::Value) -> Option<AgentUsageSnapshot> {
        let totals = value.get("totals")?;
        let number = |key: &str| totals.get(key).and_then(serde_json::Value::as_f64);

        let mut extra = Vec::new();
        if let Some(tokens) = number("total_tokens") {
            extra.push(info_field("Tokens (30d)", &format_count(tokens)));
        }
        if let Some(requests) = number("request_count") {
            extra.push(info_field("Requests (30d)", &format_count(requests)));
        }
        if let Some(spend) = number("spend") {
            extra.push(info_field("Spend (30d)", &format!("${spend:.2}")));
        }
        // fx records usage per machine and only from the moment it was
        // installed; flag a window it cannot fully cover.
        if value
            .get("completeness")
            .and_then(serde_json::Value::as_str)
            == Some("incomplete")
        {
            extra.push(info_field("Window", "partial"));
        }
        (!extra.is_empty()).then(|| AgentUsageSnapshot {
            rate_limit_five_hour_used_percent: None,
            rate_limit_seven_day_used_percent: None,
            rate_limit_five_hour_resets_at: None,
            rate_limit_seven_day_resets_at: None,
            extra,
        })
    }
}

/// fx emits numeric fields as either JSON numbers or strings (`"balance":"5"`).
fn string_or_number(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn format_count(value: f64) -> String {
    let value = value.max(0.0);
    if value >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}K", value / 1_000.0)
    } else {
        format!("{value:.0}")
    }
}

impl AgentActor for FxActor {
    fn shows_detail(&self) -> bool {
        true
    }

    fn slug(&self) -> &'static str {
        "fx"
    }
    fn display_name(&self) -> &'static str {
        "fx"
    }
    fn icon_name(&self) -> &'static str {
        "fx"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["fx"]
    }

    // `fx` is too short to match inside a command line without false hits.
    fn detect_exact(&self) -> &'static [&'static str] {
        &["fx"]
    }

    fn cli_available(&self, _workdir: &Path) -> bool {
        Self::cli_available()
    }

    fn session_counts(&self, ctx: &ScanCtx) -> Result<ActorSessionCounts, String> {
        let (entries, total) = Self::fetch_sessions(ctx.workdir, 1)?;
        let latest = entries.into_iter().next();
        Ok(ActorSessionCounts::from_latest(
            total,
            latest.as_ref().map(|entry| entry.id.clone()),
            latest.as_ref().and_then(|entry| entry.title.clone()),
            latest.and_then(|entry| timestamp(entry.updated_at_ms)),
        ))
    }

    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        let (entries, total) = Self::fetch_sessions(ctx.workdir, limit)?;
        let summaries = entries
            .iter()
            .map(|entry| self.session_summary(entry))
            .collect();
        Ok((summaries, total))
    }

    fn account_fields(&self, workdir: &Path) -> Vec<AgentInfoField> {
        let status = Self::status(workdir);
        let mut fields = Vec::new();
        if let Some(auth) = Self::auth_label(status.as_ref()) {
            fields.push(info_field("Auth", &auth));
        }
        let string = |key: &str| {
            status
                .as_ref()
                .and_then(|status| status.get(key))
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        if let Some(team) = string("team") {
            fields.push(info_field("Team", &team));
        }
        if let Some(model) = string("model") {
            fields.push(info_field("Model", &model));
        }
        if let Some(mode) = string("permission_mode") {
            fields.push(info_field("Permissions", &mode));
        }
        fields
    }

    fn subscription_plan<'a>(&'a self) -> super::ActorFuture<'a, Option<Vec<AgentInfoField>>> {
        spawn_blocking_opt(Self::credits_fields)
    }

    fn account_usage<'a>(&'a self) -> super::ActorFuture<'a, Option<AgentUsageSnapshot>> {
        spawn_blocking_opt(Self::usage_snapshot)
    }

    // fx reads the permission mode from the environment; `yolo` is its bypass.
    fn default_launch_command(&self) -> Option<String> {
        self.resolved_program()
            .map(|program| format!("FX_PERMISSION_MODE=yolo {program}"))
    }

    fn resume_launch_command(&self, quoted: &str) -> Option<String> {
        Some(format!("FX_PERMISSION_MODE=yolo fx --resume {quoted}"))
    }
}

#[cfg(test)]
mod tests {
    use super::super::utils::shell_quote;
    use super::*;

    fn write_index(dir: &Path, entries: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("index.json"),
            format!("{{\"schema_version\":3,\"sessions\":[{entries}]}}"),
        )
        .unwrap();
    }

    #[test]
    fn scans_index_filtered_by_workspace_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        write_index(
            dir.path(),
            r#"
            {"id":"a","workspace_root":"/repo","created_at_ms":1000,"updated_at_ms":2000,"title":"alpha"},
            {"id":"b","workspace_root":"/repo","created_at_ms":1000,"updated_at_ms":5000,"title":"beta"},
            {"id":"c","workspace_root":"/other","created_at_ms":1000,"updated_at_ms":9000,"title":"gamma"}
            "#,
        );

        let (entries, total) =
            FxActor::scan_sessions(dir.path(), Path::new("/repo"), None).unwrap();
        assert_eq!(total, 2);
        assert_eq!(entries[0].id, "b");
        assert_eq!(entries[1].id, "a");

        let (entries, total) =
            FxActor::scan_sessions(dir.path(), Path::new("/repo"), Some(1)).unwrap();
        assert_eq!(total, 2);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "b");
    }

    #[test]
    fn missing_index_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (entries, total) =
            FxActor::scan_sessions(dir.path(), Path::new("/repo"), None).unwrap();
        assert!(entries.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn parses_credits_plan_and_balance() {
        let value = serde_json::json!({"kind":"credits","balance":"5","used":null,"plan":"pro"});
        let fields = FxActor::credits_fields_from(&value).unwrap();
        assert_eq!(fields[0].label, "Plan");
        assert_eq!(fields[0].value, "Pro");
        assert_eq!(fields[1].label, "Gateway credits");
        assert_eq!(fields[1].value, "5");
        // A response with nothing usable yields no plan section at all.
        assert!(
            FxActor::credits_fields_from(&serde_json::json!({"balance":null,"plan":null}))
                .is_none()
        );
    }

    #[test]
    fn usage_snapshot_reports_counters_not_rate_limits() {
        let value = serde_json::json!({
            "kind": "usage",
            "completeness": "incomplete",
            "totals": {"total_tokens": 1_608_639, "request_count": 41, "spend": 0.0},
        });
        let usage = FxActor::usage_snapshot_from(&value).unwrap();
        assert!(usage.rate_limit_five_hour_used_percent.is_none());
        let labels: Vec<_> = usage.extra.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Tokens (30d)", "Requests (30d)", "Spend (30d)", "Window"]
        );
        assert_eq!(usage.extra[0].value, "1.6M");
        assert_eq!(usage.extra[2].value, "$0.00");
        assert_eq!(usage.extra[3].value, "partial");
    }

    #[test]
    fn resume_command_keeps_permission_bypass() {
        assert_eq!(
            FxActor.resume_launch_command(&shell_quote("s 1")).unwrap(),
            "FX_PERMISSION_MODE=yolo fx --resume 's 1'"
        );
    }
}
