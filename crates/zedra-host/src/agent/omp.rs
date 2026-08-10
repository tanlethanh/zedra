use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use zedra_rpc::proto::*;

use super::utils::{
    command_on_path, cwd_matches, file_size_bytes, home_path, info_field, parse_rfc3339,
    resume_summary, session_title, sorted_jsonl_candidates, spawn_blocking_opt, string_field,
    user_message_text,
};
use super::{
    hook_file_mentions_zedra, hooks_enabled, setup_status, ActorFuture, AgentActor,
    AgentSetupSummary, HookContext, ScanCtx, SessionCounts as ActorSessionCounts,
};

const LIST_HEAD_SCAN_MAX_LINES: usize = 32;

#[derive(Debug, Clone)]
struct OmpSessionFile {
    path: PathBuf,
    session_id: String,
    cwd: Option<String>,
    created_at: Option<DateTime<Utc>>,
    last_activity_at: Option<DateTime<Utc>>,
    title: Option<String>,
}

impl OmpActor {
    pub fn cli_available() -> bool {
        command_on_path("omp") || Self::omp_agent_dir().is_dir()
    }

    pub fn session_counts(workdir: &Path) -> Result<ActorSessionCounts, String> {
        let (files, total) =
            Self::collect_session_files(workdir, Some(1)).map_err(|e| e.to_string())?;
        let latest = files.first();
        Ok(ActorSessionCounts::from_latest(
            total,
            latest.map(|f| f.session_id.clone()),
            latest.and_then(|f| f.title.clone()),
            latest.and_then(|f| f.last_activity_at),
        ))
    }

    pub fn sessions(
        workdir: &Path,
        _cli: &AgentCliSummary,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        let (files, total) =
            Self::collect_session_files(workdir, Some(limit)).map_err(|e| e.to_string())?;
        let summaries = files.iter().map(Self::session_summary).collect();
        Ok((summaries, total))
    }

    /// Title of a single omp session by id within the workdir. Used to fill the
    /// notification body on a `Stop` hook.
    pub fn title_for_session(workdir: &Path, session_id: &str) -> Option<String> {
        let (files, _) = Self::collect_session_files(workdir, None).ok()?;
        let file = files.into_iter().find(|f| f.session_id == session_id)?;
        session_title(file.title)
    }

    pub fn account_fields(workdir: &Path) -> Vec<AgentInfoField> {
        let mut fields = Vec::new();
        let config = Self::read_yaml(&Self::omp_agent_dir().join("config.yml"));
        if let Some(model) = config.and_then(|cfg| Self::default_model(&cfg)) {
            fields.push(info_field("Default model", &model));
        }
        let models = Self::read_yaml(&Self::omp_agent_dir().join("models.yml"));
        if let Some(count) = models
            .as_ref()
            .map(Self::custom_providers)
            .filter(|n| *n > 0)
        {
            fields.push(info_field("Custom providers", &format!("{count}")));
        }
        fields.push(info_field(
            "Project config",
            if workdir.join(".omp").join("config.yml").is_file() {
                "yes"
            } else {
                "no"
            },
        ));
        fields
    }

    // ---------------------------------------------------------------------------
    // File-system scan
    // ---------------------------------------------------------------------------

    fn omp_agent_dir() -> PathBuf {
        home_path(&[".omp", "agent"])
    }

    fn omp_sessions_root() -> PathBuf {
        Self::omp_agent_dir().join("sessions")
    }

    /// Newest sessions for `workdir` (by header `cwd`) plus the matching total.
    /// Scans every project bucket so bucket-name encoding drift across omp
    /// versions (legacy path-based, the reverted hashed scheme, migrations)
    /// never hides sessions; the recorded cwd is the authoritative scope.
    fn collect_session_files(
        workdir: &Path,
        limit: Option<usize>,
    ) -> Result<(Vec<OmpSessionFile>, usize)> {
        Self::collect_session_files_from(&Self::omp_sessions_root(), workdir, limit)
    }

    fn collect_session_files_from(
        root: &Path,
        workdir: &Path,
        limit: Option<usize>,
    ) -> Result<(Vec<OmpSessionFile>, usize)> {
        let mut files = Vec::new();
        for bucket in std::fs::read_dir(root).into_iter().flatten().flatten() {
            if !bucket.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            for (path, mtime) in sorted_jsonl_candidates(&bucket.path())? {
                let Ok(file) = Self::read_session_file(&path, mtime) else {
                    continue;
                };
                if cwd_matches(workdir, file.cwd.as_deref()) {
                    files.push(file);
                }
            }
        }
        let total = files.len();
        files.sort_by(|a, b| b.last_activity_at.cmp(&a.last_activity_at));
        if let Some(limit) = limit {
            files.truncate(limit);
        }
        Ok((files, total))
    }

    /// Parse a session JSONL head. Current omp files begin with a fixed-width
    /// `type: "title"` slot (a skipped record here), then the `type: "session"`
    /// header and entries; legacy files start at the header directly.
    fn read_session_file(path: &Path, mtime_unix_secs: Option<u64>) -> Result<OmpSessionFile> {
        let file = File::open(path)
            .with_context(|| format!("failed to read omp transcript {}", path.display()))?;
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
                    if title.is_none() {
                        title = string_field(&record, &["title"]).map(str::to_string);
                    }
                    if created_at.is_none() {
                        created_at = parse_rfc3339(string_field(&record, &["timestamp"]));
                    }
                }
                // Fixed-width title slot written at the physical file head.
                "title" => {
                    if title.is_none() {
                        title = string_field(&record, &["title", "name"]).map(str::to_string);
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
                        // Length is clamped centrally in `session_title`.
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

        Ok(OmpSessionFile {
            path: path.to_path_buf(),
            session_id,
            cwd,
            created_at,
            last_activity_at,
            title,
        })
    }

    fn session_summary(file: &OmpSessionFile) -> AgentSessionSummary {
        AgentSessionSummary {
            slug: "omp".to_string(),
            session_id: file.session_id.clone(),
            title: session_title(file.title.clone()),
            cwd: file.cwd.clone(),
            created_at: file.created_at,
            last_activity_at: file.last_activity_at,
            resume: resume_summary("omp", &file.session_id),
            git: None,
            usage: None,
            transcript_size_bytes: file_size_bytes(&file.path),
        }
    }

    // ---------------------------------------------------------------------------
    // Config / auth (account info)
    // ---------------------------------------------------------------------------

    fn read_yaml(path: &Path) -> Option<serde_yaml::Value> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_yaml::from_str(&raw).ok())
    }

    /// `modelRoles.default` (e.g. `spark/minimax-m3`) from `config.yml`.
    fn default_model(config: &serde_yaml::Value) -> Option<String> {
        config
            .get("modelRoles")?
            .get("default")?
            .as_str()
            .map(str::to_string)
            .filter(|value| !value.is_empty())
    }

    /// Custom provider count from `models.yml` `providers` map.
    fn custom_providers(models: &serde_yaml::Value) -> usize {
        models
            .get("providers")
            .and_then(serde_yaml::Value::as_mapping)
            .map_or(0, |map| map.len())
    }

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    fn first_user_text(record: &Value) -> Option<String> {
        user_message_text(record.get("message").unwrap_or(record))
    }
}

// ---------------------------------------------------------------------------
// Global hook extension written by `AgentActor::setup` and `zedra setup omp`
// ---------------------------------------------------------------------------

impl OmpActor {
    /// Writes the global omp hook extension; shared by `setup` and `setup_cli`.
    fn write_hook_extension(force: bool, cli_path: &str) -> Result<PathBuf> {
        let path = Self::hook_extension_path();
        let contents = Self::hook_extension_contents(cli_path);
        super::utils::write_file_checked(&path, &contents, force, "omp hook extension")?;
        Ok(path)
    }

    fn hook_extension_path() -> PathBuf {
        home_path(&[".omp", "agent", "extensions", "zedra-agent-hooks.ts"])
    }

    fn hook_extension_contents(cli_path: &str) -> String {
        let cli_json =
            serde_json::to_string(cli_path).unwrap_or_else(|_| format!("\"{}\"", cli_path));
        format!(
            r#"import type {{ ExtensionAPI }} from "@oh-my-pi/pi-coding-agent";
import {{ spawn }} from "node:child_process";

// Zedra hook extension for omp: forwards lifecycle events to the daemon for
// state + push notifications. Active only inside a Zedra terminal
// (ZEDRA_TERMINAL_ID); failures are swallowed so hooks never break omp.
export default function (omp: ExtensionAPI) {{
  if (!process.env.ZEDRA_TERMINAL_ID) return;

  const CLI = process.env.ZEDRA_CLI || {cli};

  const fire = (hookEventName: string, sessionId?: string) => {{
    try {{
      const child = spawn(
        CLI,
        ["agent", "hook", "receive", "--agent", "omp", "--quiet"],
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

  // Gate on ctx.hasUI: skip non-interactive (print / RPC / subagent) runs.
  // Check `=== false` so older omp versions without hasUI still fire hooks.
  const skip = (ctx: {{ hasUI?: boolean }}) => ctx.hasUI === false;

  omp.on("before_agent_start", (event, ctx) => {{
    if (skip(ctx)) return;
    fire("UserPromptSubmit", (event as any)?.sessionId);
  }});

  omp.on("agent_end", (event, ctx) => {{
    if (skip(ctx)) return;
    fire("Stop", (event as any)?.sessionId);
  }});

  // Fires on Ctrl+C, SIGTERM, /quit, /reload, /new, /resume, /fork.
  // Ensures Running indicator clears if omp is killed mid-turn.
  omp.on("session_shutdown", (event, ctx) => {{
    if (skip(ctx)) return;
    fire("Stop", (event as any)?.sessionId);
  }});
}}
"#,
            cli = cli_json
        )
    }
}

pub(super) struct OmpActor;

impl AgentActor for OmpActor {
    fn shows_detail(&self) -> bool {
        true
    }

    fn slug(&self) -> &'static str {
        "omp"
    }
    fn display_name(&self) -> &'static str {
        "Oh My Pi"
    }
    fn icon_name(&self) -> &'static str {
        "omp"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["omp"]
    }
    // Short token: match only as the entire command so `curl .../omp.sh/install`
    // and word-boundary hits never latch an agent identity (pi.rs pattern).
    fn detect_exact(&self) -> &'static [&'static str] {
        &["omp"]
    }
    fn detect_aliases(&self) -> &'static [&'static str] {
        &["@oh-my-pi/pi-coding-agent"]
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
                    ".omp",
                    "agent",
                    "extensions",
                    "zedra-agent-hooks.ts",
                ])),
            None,
        )
    }

    fn resume_launch_command(&self, quoted: &str) -> Option<String> {
        Some(format!("omp --resume {quoted}"))
    }

    // No remote plan/usage endpoint: `subscription_plan`/`account_usage` keep
    // the trait's None defaults.

    fn supports_hooks(&self) -> bool {
        true
    }

    // omp exposes no approval hook (default `tools.approvalMode: yolo`), so
    // there is no WaitingApproval transition.
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

    // omp stores transcripts per workdir; look up the session title for the body.
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
                    ctx.section("Setting up omp");
                    let binary = ctx.hook_binary()?;
                    let path = Self::write_hook_extension(true, &binary)?;
                    ctx.step("hooks");
                    ctx.detail(&format!("write {}", path.display()));
                    ctx.message("omp setup complete.");
                }
                super::SetupAction::Remove => {
                    ctx.message("Removing Zedra lifecycle-hook extension for omp:");
                    let path = Self::hook_extension_path();
                    if ctx.remove_path(&path)? {
                        ctx.step("hooks");
                        ctx.detail(&format!("remove {}", path.display()));
                    }
                    ctx.message("");
                    ctx.message("omp setup removed.");
                    ctx.message("Restart any running omp session to apply the change.");
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_title_slot_header_and_first_user_message() {
        let dir = tempfile::tempdir().unwrap();
        let bucket = dir.path().join("-dev-project");
        std::fs::create_dir_all(&bucket).unwrap();
        let path = bucket.join("2026-05-28_abc.jsonl");
        // Physical title slot (fixed-width in real files; a plain record here),
        // then the session header and a user message.
        std::fs::write(
            &path,
            r#"{"type":"title","title":"Fix terminal paste","source":"auto"}
{"type":"session","version":3,"id":"abc","timestamp":"2026-05-28T10:00:00Z","cwd":"/Users/me/project"}
{"type":"message","id":"a","message":{"role":"user","content":[{"type":"text","text":"Refactor terminal scrollback"}]}}
{"type":"message","id":"b","message":{"role":"assistant","content":[]}}
"#,
        )
        .unwrap();

        let file = OmpActor::read_session_file(&path, None).unwrap();
        assert_eq!(file.session_id, "abc");
        assert_eq!(file.cwd.as_deref(), Some("/Users/me/project"));
        assert_eq!(file.title.as_deref(), Some("Fix terminal paste"));
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
        let file = OmpActor::read_session_file(&path, None).unwrap();
        assert_eq!(file.session_id, "2026-05-28_xyz");
    }

    #[test]
    fn scan_filters_sessions_to_workdir_across_buckets() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let workdir = Path::new("/Users/me/project");

        let mine = root.join("-dev-project");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(
            mine.join("2026-05-28_a.jsonl"),
            r#"{"type":"session","id":"a","timestamp":"2026-05-28T10:00:00Z","cwd":"/Users/me/project"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"mine"}]}}
"#,
        )
        .unwrap();

        let other = root.join("--tmp-other--");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            other.join("2026-05-29_b.jsonl"),
            r#"{"type":"session","id":"b","timestamp":"2026-05-29T10:00:00Z","cwd":"/tmp/other"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"other"}]}}
"#,
        )
        .unwrap();

        let (files, total) = OmpActor::collect_session_files_from(root, workdir, None).unwrap();
        assert_eq!(total, 1);
        assert_eq!(files[0].session_id, "a");
    }

    #[test]
    fn config_yaml_surfaces_model_roles_and_providers() {
        let config: serde_yaml::Value =
            serde_yaml::from_str("modelRoles:\n  default: spark/minimax-m3\n").unwrap();
        assert_eq!(
            OmpActor::default_model(&config).as_deref(),
            Some("spark/minimax-m3")
        );

        let models: serde_yaml::Value =
            serde_yaml::from_str("providers:\n  spark: { models: [] }\n  ollama: { models: [] }\n")
                .unwrap();
        assert_eq!(OmpActor::custom_providers(&models), 2);
        assert_eq!(OmpActor::custom_providers(&serde_yaml::Value::Null), 0);
    }

    #[test]
    fn hook_state_maps_lifecycle_events() {
        let actor = OmpActor;
        assert_eq!(
            actor.hook_state("UserPromptSubmit", &Value::Null),
            Some(AgentState::Running)
        );
        assert_eq!(
            actor.hook_state("Stop", &Value::Null),
            Some(AgentState::Completed)
        );
        assert_eq!(actor.hook_state("WaitingApproval", &Value::Null), None);
    }

    #[test]
    fn hook_extension_contents_embed_cli_and_slug() {
        let contents = OmpActor::hook_extension_contents("/usr/local/bin/zedra");
        assert!(contents.contains("@oh-my-pi/pi-coding-agent"));
        assert!(contents.contains("\"--agent\", \"omp\""));
        assert!(contents.contains("/usr/local/bin/zedra"));
        assert!(contents.contains("before_agent_start"));
        assert!(contents.contains("session_shutdown"));
    }
}
