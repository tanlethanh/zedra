use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use zedra_rpc::proto::*;

use super::utils::{
    command_on_path, file_size_bytes, home_path, info_field, parse_rfc3339, read_json_file,
    resume_summary, session_title, sorted_jsonl_candidates, spawn_blocking_opt, string_field,
    user_message_text,
};

const LIST_HEAD_SCAN_MAX_LINES: usize = 32;

#[derive(Debug, Clone)]
struct PiSessionFile {
    path: PathBuf,
    session_id: String,
    cwd: Option<String>,
    created_at: Option<DateTime<Utc>>,
    last_activity_at: Option<DateTime<Utc>>,
    title: Option<String>,
}

impl PiActor {
    pub fn cli_available() -> bool {
        command_on_path("pi") || Self::pi_sessions_root().is_dir()
    }

    pub fn session_counts(workdir: &Path) -> Result<super::SessionCounts, String> {
        let (files, total) =
            Self::collect_session_files(workdir, Some(1)).map_err(|e| e.to_string())?;
        let latest = files.first();
        Ok(super::SessionCounts::from_latest(
            total,
            latest.map(|f| f.session_id.clone()),
            latest.and_then(|f| f.title.clone()),
            latest.and_then(|f| f.last_activity_at),
        ))
    }

    pub fn sessions(
        workdir: &Path,
        cli: &AgentCliSummary,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        let (files, total) =
            Self::collect_session_files(workdir, Some(limit)).map_err(|e| e.to_string())?;
        let summaries = files
            .iter()
            .map(|file| Self::session_summary(file, cli))
            .collect();
        Ok((summaries, total))
    }

    /// Title of a single pi session, looked up by id within the workdir's project
    /// transcripts. Used to fill the notification body on a `Stop` hook.
    pub fn title_for_session(workdir: &Path, session_id: &str) -> Option<String> {
        let (files, _) = Self::collect_session_files(workdir, None).ok()?;
        let file = files.into_iter().find(|f| f.session_id == session_id)?;
        session_title(file.title)
    }

    pub fn account_fields(workdir: &Path) -> Vec<AgentInfoField> {
        let mut fields = Vec::new();
        let (settings, has_project) = Self::effective_settings(workdir);
        let providers = Self::provider_fields();

        // Per-provider auth replaces a single "Logged in" boolean: Pi is multi-provider.
        if providers.is_empty() {
            fields.push(info_field("Logged in", "no"));
        } else {
            fields.extend(providers);
        }

        if let Some(model) = &settings.default_model {
            fields.push(info_field("Default model", model));
        }
        if let Some(provider) = &settings.default_provider {
            fields.push(info_field("Default provider", provider));
        }
        if let Some(level) = &settings.default_thinking_level {
            fields.push(info_field("Thinking", level));
        }
        if let Some(value) = Self::extensions_value(&settings) {
            fields.push(info_field("Extensions", &value));
        }
        fields.push(info_field(
            "Project config",
            if has_project { "yes" } else { "no" },
        ));
        fields
    }

    // ---------------------------------------------------------------------------
    // File-system scan
    // ---------------------------------------------------------------------------

    fn pi_sessions_root() -> PathBuf {
        home_path(&[".pi", "agent", "sessions"])
    }

    fn encoded_project_dir(workdir: &Path) -> String {
        let encoded: String = workdir
            .to_string_lossy()
            .chars()
            .map(|ch| match ch {
                '/' | '\\' => '-',
                _ => ch,
            })
            .collect();
        format!("-{encoded}--")
    }

    fn project_dir(workdir: &Path) -> PathBuf {
        Self::pi_sessions_root().join(Self::encoded_project_dir(workdir))
    }

    /// Newest sessions by mtime plus total; the project dir is workspace-scoped,
    /// so the candidate count is the total without opening any file.
    fn collect_session_files(
        workdir: &Path,
        limit: Option<usize>,
    ) -> Result<(Vec<PiSessionFile>, usize)> {
        let candidates = sorted_jsonl_candidates(&Self::project_dir(workdir))?;
        let total = candidates.len();
        let scan_limit = limit.unwrap_or(total);
        let mut files = Vec::with_capacity(scan_limit.min(total));
        for (path, mtime) in candidates.into_iter().take(scan_limit) {
            files.push(Self::read_session_file(&path, mtime)?);
        }
        Ok((files, total))
    }

    fn read_session_file(path: &Path, mtime_unix_secs: Option<u64>) -> Result<PiSessionFile> {
        let file = File::open(path)
            .with_context(|| format!("failed to read Pi transcript {}", path.display()))?;
        let mut session_id = String::new();
        let mut cwd = None;
        let mut created_at = None;
        let mut last_timestamp: Option<DateTime<Utc>> = None;
        let mut title: Option<String> = None;
        let mut scanned_lines = 0usize;

        for line in BufReader::new(file).lines() {
            let line =
                line.with_context(|| format!("failed to read line in {}", path.display()))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let record = match serde_json::from_str::<Value>(trimmed) {
                Ok(Value::Object(record)) => Value::Object(record),
                _ => continue,
            };
            scanned_lines += 1;

            let record_type = string_field(&record, &["type"]).unwrap_or("");
            match record_type {
                "session" => {
                    if session_id.is_empty() {
                        if let Some(id) = string_field(&record, &["id"]) {
                            session_id = id.to_string();
                        }
                    }
                    if cwd.is_none() {
                        cwd = string_field(&record, &["cwd"]).map(str::to_string);
                    }
                    if created_at.is_none() {
                        created_at = parse_rfc3339(string_field(&record, &["timestamp"]));
                    }
                }
                "session_info" => {
                    if let Some(name) =
                        string_field(&record, &["displayName", "display_name", "name"])
                    {
                        title = Some(name.to_string());
                    }
                }
                "label" => {
                    if title.is_none() {
                        if let Some(label) = string_field(&record, &["label", "text", "value"]) {
                            title = Some(label.to_string());
                        }
                    }
                }
                "message" => {
                    if title.is_none() {
                        // Length is clamped centrally in `session_title`; the UI trims
                        // for display.
                        title = Self::first_user_text(&record);
                    }
                }
                _ => {}
            }

            if let Some(ts) = parse_rfc3339(string_field(&record, &["timestamp"])) {
                last_timestamp = match last_timestamp {
                    Some(current) if current >= ts => Some(current),
                    _ => Some(ts),
                };
            }

            if !session_id.is_empty()
                && title.is_some()
                && scanned_lines >= LIST_HEAD_SCAN_MAX_LINES
            {
                break;
            }
        }

        if session_id.is_empty() {
            session_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("unknown")
                .to_string();
        }

        // Head scans see only opening records; trust mtime (append time) for
        // activity, falling back to the newest scanned timestamp.
        let mtime_activity =
            mtime_unix_secs.and_then(|secs| DateTime::<Utc>::from_timestamp(secs as i64, 0));
        let last_activity_at = mtime_activity.or(last_timestamp);

        Ok(PiSessionFile {
            path: path.to_path_buf(),
            session_id,
            cwd,
            created_at,
            last_activity_at,
            title,
        })
    }

    fn session_summary(file: &PiSessionFile, _cli: &AgentCliSummary) -> AgentSessionSummary {
        AgentSessionSummary {
            slug: "pi".to_string(),
            session_id: file.session_id.clone(),
            title: session_title(file.title.clone()),
            cwd: file.cwd.clone(),
            created_at: file.created_at,
            last_activity_at: file.last_activity_at,
            resume: resume_summary("pi", &file.session_id),
            git: None,
            usage: None,
            transcript_size_bytes: file_size_bytes(&file.path),
        }
    }

    // ---------------------------------------------------------------------------
    // Config / auth (account info)
    // ---------------------------------------------------------------------------

    fn pi_agent_dir() -> PathBuf {
        home_path(&[".pi", "agent"])
    }
}

/// Subset of Pi's `settings.json` we surface. Pi stores far more, but the agent
/// info panel only needs the model defaults and the extensibility counts.
#[derive(Default, serde::Deserialize)]
struct PiSettings {
    #[serde(rename = "defaultModel")]
    default_model: Option<String>,
    #[serde(rename = "defaultProvider")]
    default_provider: Option<String>,
    #[serde(rename = "defaultThinkingLevel")]
    default_thinking_level: Option<String>,
    /// npm/git package sources (string or `{ source, .. }`).
    packages: Option<Vec<Value>>,
    /// Local extension file/dir paths.
    extensions: Option<Vec<String>>,
}

impl PiActor {
    fn read_pi_settings(path: &Path) -> PiSettings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Global `~/.pi/agent` overlaid per-field by project `<workdir>/.pi`
    /// (matching Pi's merge); also returns whether the workspace has settings.
    fn effective_settings(workdir: &Path) -> (PiSettings, bool) {
        let global = Self::read_pi_settings(&Self::pi_agent_dir().join("settings.json"));
        let project_path = workdir.join(".pi").join("settings.json");
        let has_project = project_path.is_file();
        let project = Self::read_pi_settings(&project_path);
        let merged = PiSettings {
            default_model: project.default_model.or(global.default_model),
            default_provider: project.default_provider.or(global.default_provider),
            default_thinking_level: project
                .default_thinking_level
                .or(global.default_thinking_level),
            packages: project.packages.or(global.packages),
            extensions: project.extensions.or(global.extensions),
        };
        (merged, has_project)
    }

    fn extensions_value(settings: &PiSettings) -> Option<String> {
        let packages = settings.packages.as_deref().unwrap_or(&[]);
        let locals = settings.extensions.as_deref().map(<[_]>::len).unwrap_or(0);
        let total = packages.len() + locals;
        if total == 0 {
            return None;
        }
        // Show the first package source as a hint; "configured" is honest because we
        // read settings statically and never resolve the package into its resources.
        match packages.iter().find_map(Self::package_source) {
            Some(src) if total == 1 => Some(src),
            Some(src) => Some(format!("{total} configured ({src}, …)")),
            None => Some(format!("{total} configured")),
        }
    }

    fn package_source(value: &Value) -> Option<String> {
        value
            .as_str()
            .map(str::to_string)
            .or_else(|| string_field(value, &["source"]).map(str::to_string))
    }
}

#[derive(serde::Deserialize)]
struct PiAuthEntry {
    #[serde(rename = "type")]
    kind: String,
    /// OAuth access-token expiry, Unix milliseconds.
    expires: Option<i64>,
}

impl PiActor {
    /// Providers from `auth.json` plus custom ones from `models.json` as
    /// `{provider → auth state}` rows; never exposes tokens or account ids.
    fn provider_fields() -> Vec<AgentInfoField> {
        let mut fields = Vec::new();
        if let Ok(Value::Object(map)) = read_json_file(&Self::pi_agent_dir().join("auth.json")) {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (provider, value) in entries {
                if let Ok(entry) = serde_json::from_value::<PiAuthEntry>(value) {
                    fields.push(info_field(&provider, &Self::auth_value(&entry)));
                }
            }
        }
        for (id, models) in Self::custom_providers() {
            fields.push(info_field(&id, &format!("custom · {models} models")));
        }
        fields
    }

    fn auth_value(entry: &PiAuthEntry) -> String {
        match entry.kind.as_str() {
            "oauth" => match entry.expires {
                Some(expires_ms) => {
                    let remaining_ms = expires_ms - Utc::now().timestamp_millis();
                    if remaining_ms <= 0 {
                        "OAuth · expired".to_string()
                    } else {
                        format!(
                            "OAuth · expires in {}",
                            Self::humanize_secs(remaining_ms / 1000)
                        )
                    }
                }
                None => "OAuth".to_string(),
            },
            "api_key" => "API key".to_string(),
            other => other.to_string(),
        }
    }

    fn humanize_secs(secs: i64) -> String {
        if secs >= 86_400 {
            format!("{}d", secs / 86_400)
        } else if secs >= 3_600 {
            format!("{}h", secs / 3_600)
        } else {
            format!("{}m", (secs / 60).max(1))
        }
    }

    /// Custom provider ids (e.g. `ollama`) from `models.json` and their model count.
    fn custom_providers() -> Vec<(String, usize)> {
        let Ok(file) = read_json_file(&Self::pi_agent_dir().join("models.json")) else {
            return Vec::new();
        };
        let Some(providers) = file.get("providers").and_then(Value::as_object) else {
            return Vec::new();
        };
        let mut out: Vec<(String, usize)> = providers
            .iter()
            .map(|(id, cfg)| {
                let count = cfg
                    .get("models")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                (id.clone(), count)
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    fn first_user_text(record: &Value) -> Option<String> {
        user_message_text(record.get("message").unwrap_or(record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_project_dir_with_pi_wrapping() {
        assert_eq!(
            PiActor::encoded_project_dir(Path::new("/Users/me/project")),
            "--Users-me-project--"
        );
    }

    #[test]
    fn humanize_secs_picks_coarsest_unit() {
        assert_eq!(PiActor::humanize_secs(9 * 86_400 + 5), "9d");
        assert_eq!(PiActor::humanize_secs(5 * 3_600), "5h");
        assert_eq!(PiActor::humanize_secs(120), "2m");
        assert_eq!(PiActor::humanize_secs(10), "1m"); // never "0m"
    }

    #[test]
    fn auth_value_covers_oauth_and_api_key() {
        let future = Utc::now().timestamp_millis() + 3 * 86_400 * 1000;
        assert!(PiActor::auth_value(&PiAuthEntry {
            kind: "oauth".into(),
            expires: Some(future),
        })
        .starts_with("OAuth · expires in"));
        assert_eq!(
            PiActor::auth_value(&PiAuthEntry {
                kind: "oauth".into(),
                expires: Some(0),
            }),
            "OAuth · expired"
        );
        assert_eq!(
            PiActor::auth_value(&PiAuthEntry {
                kind: "oauth".into(),
                expires: None,
            }),
            "OAuth"
        );
        assert_eq!(
            PiActor::auth_value(&PiAuthEntry {
                kind: "api_key".into(),
                expires: None,
            }),
            "API key"
        );
    }

    #[test]
    fn package_source_reads_string_and_object_forms() {
        assert_eq!(
            PiActor::package_source(&serde_json::json!("npm:foo")).as_deref(),
            Some("npm:foo")
        );
        assert_eq!(
            PiActor::package_source(&serde_json::json!({ "source": "git:bar", "skills": [] }))
                .as_deref(),
            Some("git:bar")
        );
        assert_eq!(
            PiActor::package_source(&serde_json::json!({ "skills": [] })),
            None
        );
    }

    #[test]
    fn extensions_value_counts_packages_and_locals() {
        let none = PiSettings::default();
        assert_eq!(PiActor::extensions_value(&none), None);

        let single = PiSettings {
            packages: Some(vec![serde_json::json!("npm:web-search")]),
            ..Default::default()
        };
        assert_eq!(
            PiActor::extensions_value(&single).as_deref(),
            Some("npm:web-search")
        );

        let many = PiSettings {
            packages: Some(vec![serde_json::json!("npm:a"), serde_json::json!("npm:b")]),
            extensions: Some(vec!["/local/ext".to_string()]),
            ..Default::default()
        };
        assert_eq!(
            PiActor::extensions_value(&many).as_deref(),
            Some("3 configured (npm:a, …)")
        );
    }

    #[test]
    fn reads_session_header_and_first_user_message() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let sessions = dir.path().join(PiActor::encoded_project_dir(workdir));
        std::fs::create_dir_all(&sessions).unwrap();
        let path = sessions.join("2026-05-28_abc.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session","version":3,"id":"abc","timestamp":"2026-05-28T10:00:00Z","cwd":"/Users/me/project"}
{"type":"message","id":"a","message":{"role":"user","content":[{"type":"text","text":"Refactor terminal scrollback"}]}}
{"type":"message","id":"b","message":{"role":"assistant","content":[]}}
"#,
        )
        .unwrap();

        let file = PiActor::read_session_file(&path, None).unwrap();
        assert_eq!(file.session_id, "abc");
        assert_eq!(file.cwd.as_deref(), Some("/Users/me/project"));
        assert_eq!(file.title.as_deref(), Some("Refactor terminal scrollback"));
    }

    #[test]
    fn falls_back_to_filename_when_session_id_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("2026-05-28_xyz.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}
"#,
        )
        .unwrap();
        let file = PiActor::read_session_file(&path, None).unwrap();
        assert_eq!(file.session_id, "2026-05-28_xyz");
    }

    #[test]
    fn head_scan_uses_mtime_for_activity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("2026-05-28_long.jsonl");
        let mut lines = String::from(
            r#"{"type":"session","id":"long","timestamp":"2026-05-28T10:00:00Z","cwd":"/p"}
{"type":"message","id":"u","message":{"role":"user","content":[{"type":"text","text":"start"}]}}
"#,
        );
        // Push past LIST_HEAD_SCAN_MAX_LINES so the head scan stops early.
        for i in 0..(LIST_HEAD_SCAN_MAX_LINES + 10) {
            lines.push_str(&format!(
                "{{\"type\":\"message\",\"id\":\"m{i}\",\"timestamp\":\"2026-05-28T12:00:{:02}Z\",\"message\":{{\"role\":\"assistant\",\"content\":[]}}}}\n",
                i % 60
            ));
        }
        std::fs::write(&path, lines).unwrap();

        // The scan stops before the latest turn, so activity comes from mtime.
        let file = PiActor::read_session_file(&path, Some(1_700_000_000)).unwrap();
        assert_eq!(
            file.last_activity_at,
            DateTime::<Utc>::from_timestamp(1_700_000_000, 0)
        );
    }
}

use super::hook::HookContext;
use super::{
    hook_file_mentions_zedra, hooks_enabled, setup_status, ActorFuture, AgentActor, ScanCtx,
    SessionCounts as ActorSessionCounts,
};

pub(super) struct PiActor;

impl AgentActor for PiActor {
    fn shows_detail(&self) -> bool {
        true
    }

    fn slug(&self) -> &'static str {
        "pi"
    }
    fn display_name(&self) -> &'static str {
        "Pi"
    }
    fn icon_name(&self) -> &'static str {
        "pi"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["pi"]
    }

    fn detect_aliases(&self) -> &'static [&'static str] {
        &["pi-coding-agent", "@mariozechner/pi", "picodingagent"]
    }

    fn detect_exact(&self) -> &'static [&'static str] {
        &["pi"]
    }

    fn cli_available(&self, _workdir: &Path) -> bool {
        Self::cli_available()
    }

    fn session_counts(&self, ctx: &ScanCtx) -> Result<ActorSessionCounts, String> {
        Self::session_counts(ctx.workdir)
    }

    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        Self::sessions(ctx.workdir, ctx.cli, limit)
    }

    fn account_fields(&self, workdir: &Path) -> Vec<AgentInfoField> {
        Self::account_fields(workdir)
    }

    fn setup_summary(&self, available: bool, _workdir: &Path) -> AgentSetupSummary {
        setup_status(
            available,
            false,
            false,
            hooks_enabled()
                && hook_file_mentions_zedra(&home_path(&[
                    ".pi",
                    "agent",
                    "extensions",
                    "zedra-agent-hooks.ts",
                ])),
            None,
        )
    }

    fn resume_launch_command(&self, quoted: &str) -> Option<String> {
        Some(format!("pi --session {quoted}"))
    }

    // No remote plan/usage endpoint: `subscription_plan`/`account_usage` keep
    // the trait's None defaults.

    fn supports_hooks(&self) -> bool {
        true
    }

    // The pi extension normalizes pi's native events to Claude-compatible names.
    // Pi exposes no approval hook, so there is no WaitingApproval transition.
    fn hook_state(&self, event_name: &str, _payload: &Value) -> Option<AgentState> {
        match event_name {
            "UserPromptSubmit" => Some(AgentState::Running),
            "Stop" => Some(AgentState::Completed),
            _ => None,
        }
    }

    // Only notify on completion — Stop is the single user-meaningful turn boundary.
    fn hook_notify_title(&self, event_name: &str) -> Option<String> {
        (event_name == "Stop").then(|| format!("{} completed", self.display_name()))
    }

    // Pi stores transcripts per workdir; look up the session title for the body.
    fn hook_notify_body(
        &self,
        ctx: &HookContext,
        agent_session_id: Option<String>,
    ) -> ActorFuture<'static, Option<String>> {
        let workdir = ctx.workdir.clone();
        spawn_blocking_opt(move || {
            agent_session_id
                .as_deref()
                .and_then(|id| Self::title_for_session(&workdir, id))
        })
    }

    fn supports_setup(&self) -> bool {
        true
    }

    fn setup(&self, _workdir: &Path, force: bool) -> anyhow::Result<Vec<PathBuf>> {
        let cli = std::env::current_exe().context("failed to resolve current zedra binary")?;
        Ok(vec![Self::write_hook_extension(
            force,
            &cli.display().to_string(),
        )?])
    }

    fn supports_setup_cli(&self) -> bool {
        true
    }

    fn setup_cli<'a>(
        &'a self,
        action: super::SetupAction,
        ctx: super::SetupCliCtx,
    ) -> ActorFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            match action {
                super::SetupAction::Install => {
                    ctx.section("Setting up Pi");
                    let binary = ctx.hook_binary()?;
                    let path = Self::write_hook_extension(true, &binary)?;
                    ctx.step("hooks");
                    ctx.detail(&format!("write {}", path.display()));
                    ctx.message("pi setup complete.");
                }
                super::SetupAction::Remove => {
                    ctx.message("Removing Zedra lifecycle-hook extension for pi:");
                    let path = Self::hook_extension_path();
                    if ctx.remove_path(&path)? {
                        ctx.step("hooks");
                        ctx.detail(&format!("remove {}", path.display()));
                    }
                    ctx.message("");
                    ctx.message("pi setup removed.");
                    ctx.message("Restart any running pi session to apply the change.");
                }
            }
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Global hook extension written by `AgentActor::setup` and `zedra setup pi`
// ---------------------------------------------------------------------------

impl PiActor {
    /// Writes the global pi hook extension; shared by `setup` and `setup_cli`.
    fn write_hook_extension(force: bool, cli_path: &str) -> Result<PathBuf> {
        let path = Self::hook_extension_path();
        let contents = Self::hook_extension_contents(cli_path);
        super::utils::write_file_checked(&path, &contents, force, "Pi hook extension")?;
        Ok(path)
    }

    fn hook_extension_path() -> PathBuf {
        home_path(&[".pi", "agent", "extensions", "zedra-agent-hooks.ts"])
    }

    fn hook_extension_contents(cli_path: &str) -> String {
        let cli_json =
            serde_json::to_string(cli_path).unwrap_or_else(|_| format!("\"{}\"", cli_path));
        format!(
            r#"import type {{ ExtensionAPI }} from "@mariozechner/pi-coding-agent";
import {{ spawn }} from "node:child_process";

// Zedra hook extension for pi: forwards lifecycle events to the daemon for
// state + push notifications. Active only inside a Zedra terminal
// (ZEDRA_TERMINAL_ID); failures are swallowed so hooks never break pi.
export default function (pi: ExtensionAPI) {{
  if (!process.env.ZEDRA_TERMINAL_ID) return;

  const CLI = process.env.ZEDRA_CLI || {cli};

  const fire = (hookEventName: string, sessionId?: string) => {{
    try {{
      const child = spawn(
        CLI,
        ["agent", "hook", "receive", "--agent", "pi", "--quiet"],
        {{
          stdio: ["pipe", "ignore", "ignore"],
          detached: true,
          // ZEDRA_TERMINAL_ID and ZEDRA_WORKDIR are inherited from process.env
          // and picked up by `agent hook receive` as --terminal-id / --workdir.
        }},
      );
      child.on("error", () => {{}});
      child.stdin?.on("error", () => {{}});
      const payload: Record<string, string> = {{ hook_event_name: hookEventName }};
      if (sessionId) payload.session_id = sessionId;
      child.stdin?.end(JSON.stringify(payload));
      child.unref();
    }} catch {{
      // spawn() can throw synchronously (EACCES, ENOENT). Stay silent.
    }}
  }};

  // Gate on ctx.hasUI: skip non-interactive (print / JSON / subagent) runs.
  // Check `=== false` so older pi versions without hasUI still fire hooks.
  const skip = (ctx: {{ hasUI?: boolean }}) => ctx.hasUI === false;

  pi.on("before_agent_start", (event, ctx) => {{
    if (skip(ctx)) return;
    fire("UserPromptSubmit", (event as any)?.sessionId);
  }});

  pi.on("tool_execution_end", (event, ctx) => {{
    if (skip(ctx)) return;
    fire("PostToolUse", (event as any)?.sessionId);
  }});

  pi.on("agent_end", (event, ctx) => {{
    if (skip(ctx)) return;
    fire("Stop", (event as any)?.sessionId);
  }});

  // Fires on Ctrl+C, SIGTERM, /quit, /reload, /new, /resume, /fork.
  // Ensures Running indicator clears if pi is killed mid-turn.
  pi.on("session_shutdown", (event, ctx) => {{
    if (skip(ctx)) return;
    fire("Stop", (event as any)?.sessionId);
  }});
}}
"#,
            cli = cli_json
        )
    }
}
