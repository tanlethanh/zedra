use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const LIST_HEAD_SCAN_MAX_LINES: usize = 64;
const LIST_HEAD_SCAN_MAX_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeSessionList {
    pub workdir: PathBuf,
    pub claude_config_dir: PathBuf,
    pub project_dir: PathBuf,
    pub total: usize,
    pub sessions: Vec<ClaudeSessionMetadata>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeSessionMetadata {
    pub session_id: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub worktree: Option<String>,
    pub created_at: Option<String>,
    pub last_activity_at: Option<String>,
    pub transcript_path: PathBuf,
    #[serde(skip_serializing)]
    sort_mtime_unix_secs: Option<u64>,
}

#[derive(Debug, Clone)]
struct TranscriptCandidate {
    path: PathBuf,
    worktree: Option<String>,
    sort_mtime_unix_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct SessionsIndexFile {
    entries: Vec<SessionsIndexEntry>,
}

#[derive(Debug, Deserialize)]
struct SessionsIndexEntry {
    #[serde(rename = "sessionId")]
    session_id: String,
    summary: Option<String>,
    created: Option<String>,
    modified: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    #[serde(rename = "projectPath")]
    project_path: Option<String>,
}

impl ClaudeActor {
    pub fn list_sessions_limited(workdir: &Path, limit: usize) -> Result<ClaudeSessionList> {
        let config_dir = Self::claude_config_dir()?;
        Self::list_sessions_in_config(workdir, &config_dir, Some(limit))
    }

    pub fn session_count_summary(workdir: &Path) -> Result<(usize, Option<ClaudeSessionMetadata>)> {
        let config_dir = Self::claude_config_dir()?;
        Self::session_count_summary_in_config(workdir, &config_dir)
    }

    fn sorted_transcript_candidates(
        claude_config_dir: &Path,
        workdir: &Path,
    ) -> Result<Vec<TranscriptCandidate>> {
        let project_dirs = Self::project_dirs_for_workdir(claude_config_dir, workdir);
        let mut candidates = Self::collect_transcript_candidates(&project_dirs)?;
        candidates.sort_by(|a, b| {
            b.sort_mtime_unix_secs
                .cmp(&a.sort_mtime_unix_secs)
                .then_with(|| b.path.cmp(&a.path))
        });
        Ok(candidates)
    }

    fn session_count_summary_in_config(
        workdir: &Path,
        claude_config_dir: &Path,
    ) -> Result<(usize, Option<ClaudeSessionMetadata>)> {
        let candidates = Self::sorted_transcript_candidates(claude_config_dir, workdir)?;
        let total = candidates.len();
        let latest = candidates
            .first()
            .map(|candidate| {
                let mut metadata = Self::read_transcript_list_metadata(
                    &candidate.path,
                    candidate.worktree.as_deref(),
                )?;
                if let Some(project_dir) = candidate.path.parent() {
                    let index = Self::load_sessions_index(project_dir);
                    Self::apply_sessions_index(&mut metadata, &index);
                }
                Ok::<_, anyhow::Error>(metadata)
            })
            .transpose()?;
        Ok((total, latest))
    }

    fn list_sessions_in_config(
        workdir: &Path,
        claude_config_dir: &Path,
        limit: Option<usize>,
    ) -> Result<ClaudeSessionList> {
        let project_dir = Self::project_dir_for_workdir(claude_config_dir, workdir);
        let candidates = Self::sorted_transcript_candidates(claude_config_dir, workdir)?;
        let total = candidates.len();

        let read_limit = limit.unwrap_or(total);
        let mut index_cache: HashMap<PathBuf, HashMap<String, SessionsIndexEntry>> = HashMap::new();
        let mut sessions = candidates
            .into_iter()
            .take(read_limit)
            .map(|candidate| {
                let mut metadata = Self::read_transcript_list_metadata(
                    &candidate.path,
                    candidate.worktree.as_deref(),
                )?;
                if let Some(project_dir) = candidate.path.parent() {
                    let index = index_cache
                        .entry(project_dir.to_path_buf())
                        .or_insert_with(|| Self::load_sessions_index(project_dir));
                    Self::apply_sessions_index(&mut metadata, index);
                }
                Ok::<_, anyhow::Error>(metadata)
            })
            .collect::<Result<Vec<_>>>()?;

        sessions.sort_by(|a, b| {
            b.last_activity_at
                .cmp(&a.last_activity_at)
                .then_with(|| b.sort_mtime_unix_secs.cmp(&a.sort_mtime_unix_secs))
                .then_with(|| b.transcript_path.cmp(&a.transcript_path))
        });

        Ok(ClaudeSessionList {
            workdir: workdir.to_path_buf(),
            claude_config_dir: claude_config_dir.to_path_buf(),
            project_dir,
            total,
            sessions,
        })
    }

    pub fn project_dir_for_workdir(claude_config_dir: &Path, workdir: &Path) -> PathBuf {
        claude_config_dir
            .join("projects")
            .join(Self::encoded_project_name(workdir))
    }

    pub fn project_dirs_for_workdir(claude_config_dir: &Path, workdir: &Path) -> Vec<PathBuf> {
        let projects_root = claude_config_dir.join("projects");
        let mut dirs = Vec::new();
        let project_dir = Self::project_dir_for_workdir(claude_config_dir, workdir);
        if project_dir.is_dir() {
            dirs.push(project_dir);
        }

        let claude_worktree_prefix =
            format!("{}--claude-worktrees-", Self::encoded_project_name(workdir));
        let Ok(entries) = std::fs::read_dir(&projects_root) else {
            return dirs;
        };
        for entry in entries.flatten() {
            let claude_worktree_dir = entry.path();
            if !claude_worktree_dir.is_dir() {
                continue;
            }
            let Some(name) = claude_worktree_dir
                .file_name()
                .and_then(|name| name.to_str())
            else {
                continue;
            };
            if name.starts_with(&claude_worktree_prefix) {
                dirs.push(claude_worktree_dir);
            }
        }

        dirs
    }

    fn collect_transcript_candidates(project_dirs: &[PathBuf]) -> Result<Vec<TranscriptCandidate>> {
        let mut candidates = Vec::new();
        for project_dir in project_dirs {
            let worktree = Self::worktree_from_project_dir(project_dir);
            for (path, sort_mtime_unix_secs) in sorted_jsonl_candidates(project_dir)? {
                if Self::is_subagent_transcript(&path) {
                    continue;
                }
                candidates.push(TranscriptCandidate {
                    path,
                    worktree: worktree.clone(),
                    sort_mtime_unix_secs,
                });
            }
        }
        Ok(candidates)
    }

    fn claude_config_dir() -> Result<PathBuf> {
        if let Some(value) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|value| !value.is_empty())
        {
            return Ok(PathBuf::from(value));
        }

        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        Ok(home.join(".claude"))
    }

    /// Like `claude_config_dir()` but infallible, falling back to `~/.claude`.
    fn claude_config_dir_or_home() -> PathBuf {
        Self::claude_config_dir().unwrap_or_else(|_| home_path(&[".claude"]))
    }

    fn encoded_project_name(workdir: &Path) -> String {
        workdir
            .to_string_lossy()
            .chars()
            .map(|ch| match ch {
                '/' | '\\' => '-',
                _ => ch,
            })
            .collect()
    }

    fn worktree_from_project_dir(project_dir: &Path) -> Option<String> {
        let name = project_dir.file_name()?.to_str()?;
        let marker = "--claude-worktrees-";
        name.find(marker)
            .map(|index| name[index + marker.len()..].to_string())
    }

    pub fn is_subagent_transcript(path: &Path) -> bool {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.starts_with("agent-"))
    }

    /// Session title from a transcript file; `None` if unreadable or untitled.
    pub fn title_from_transcript_path(path: &Path) -> Option<String> {
        Self::read_transcript_list_metadata(path, None)
            .ok()
            .and_then(|meta| meta.title)
    }

    fn read_transcript_list_metadata(
        path: &Path,
        worktree: Option<&str>,
    ) -> Result<ClaudeSessionMetadata> {
        let file = File::open(path)
            .with_context(|| format!("failed to read transcript {}", path.display()))?;
        let sort_mtime_unix_secs = mtime_unix_secs(path);
        let mut metadata = ClaudeSessionMetadata {
            session_id: String::new(),
            title: None,
            cwd: None,
            git_branch: None,
            worktree: worktree.map(str::to_string),
            created_at: None,
            last_activity_at: None,
            transcript_path: path.to_path_buf(),
            sort_mtime_unix_secs,
        };

        let mut scanned_lines = 0usize;
        let mut scanned_bytes = 0usize;
        for line in BufReader::new(file).lines() {
            let line = line
                .with_context(|| format!("failed to read transcript line in {}", path.display()))?;
            if line.trim().is_empty() {
                continue;
            }

            scanned_lines += 1;
            scanned_bytes += line.len() + 1;

            let record = match serde_json::from_str::<Value>(&line) {
                Ok(Value::Object(record)) => Value::Object(record),
                Ok(_) | Err(_) => continue,
            };

            if metadata.session_id.is_empty() {
                if let Some(session_id) = string_field(&record, &["sessionId", "session_id"]) {
                    metadata.session_id = session_id.to_string();
                }
            }
            Self::set_latest_string(&mut metadata.cwd, string_field(&record, &["cwd"]));
            Self::set_latest_string(
                &mut metadata.git_branch,
                string_field(&record, &["gitBranch", "git_branch"]),
            );
            if metadata.created_at.is_none() {
                if let Some(timestamp) =
                    string_field(&record, &["timestamp", "createdAt", "created_at"])
                {
                    metadata.created_at = Some(timestamp.to_string());
                }
            }
            if let Some(title) = string_field(&record, &["aiTitle", "ai_title"]) {
                metadata.title = Some(title.to_string());
            }
            // The head-scan cap is enforced after parsing the current record, not
            // before, so a single oversized head line (e.g. a large system prompt)
            // still contributes its sessionId/cwd/title before the scan stops.
            if Self::list_head_scan_complete(&metadata, scanned_lines)
                || scanned_lines >= LIST_HEAD_SCAN_MAX_LINES
                || scanned_bytes >= LIST_HEAD_SCAN_MAX_BYTES
            {
                break;
            }
        }

        if metadata.session_id.is_empty() {
            metadata.session_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("unknown")
                .to_string();
        }
        metadata.last_activity_at = sort_mtime_unix_secs.and_then(|secs| {
            DateTime::<Utc>::from_timestamp(secs as i64, 0).map(|dt| dt.to_rfc3339())
        });

        if metadata.title.is_none() {
            Self::fill_title_from_transcript_remainder(path, &mut metadata)?;
        }

        Ok(metadata)
    }

    fn fill_title_from_transcript_remainder(
        path: &Path,
        metadata: &mut ClaudeSessionMetadata,
    ) -> Result<()> {
        let file = File::open(path)
            .with_context(|| format!("failed to read transcript {}", path.display()))?;
        let mut last_prompt = None;
        let mut slug = None;
        let mut first_user = None;

        for line in BufReader::new(file).lines() {
            let line = line
                .with_context(|| format!("failed to read transcript line in {}", path.display()))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let needs_parse = trimmed.contains("aiTitle")
                || trimmed.contains("ai_title")
                || trimmed.contains("lastPrompt")
                || trimmed.contains("last_prompt")
                || trimmed.contains("\"slug\"")
                || (first_user.is_none() && trimmed.contains("\"type\":\"user\""));
            if !needs_parse {
                continue;
            }

            let record = match serde_json::from_str::<Value>(trimmed) {
                Ok(Value::Object(record)) => Value::Object(record),
                Ok(_) | Err(_) => continue,
            };

            if let Some(title) = string_field(&record, &["aiTitle", "ai_title"]) {
                metadata.title = Some(title.to_string());
            }
            if string_field(&record, &["type"]) == Some("last-prompt") {
                Self::set_latest_string(
                    &mut last_prompt,
                    string_field(&record, &["lastPrompt", "last_prompt"]),
                );
            }
            Self::set_latest_string(&mut slug, string_field(&record, &["slug"]));
            if first_user.is_none() {
                if let Some(title) = Self::user_message_title(&record) {
                    first_user = Some(title);
                }
            }
        }

        if metadata.title.is_none() {
            metadata.title = last_prompt
                .filter(|title| Self::is_usable_fallback_title(title))
                .or_else(|| {
                    slug.filter(|slug| Self::is_usable_fallback_title(slug))
                        .map(|slug| Self::humanize_slug(&slug))
                })
                .or(first_user.filter(|title| Self::is_usable_fallback_title(title)));
        }

        Ok(())
    }

    fn is_usable_fallback_title(title: &str) -> bool {
        let title = title.trim();
        !title.is_empty() && !title.eq_ignore_ascii_case("no prompt")
    }

    fn user_message_title(record: &Value) -> Option<String> {
        if string_field(record, &["type"]) != Some("user") {
            return None;
        }
        let message = record.get("message")?;
        let content = message.get("content")?;
        let text = if let Some(text) = content.as_str() {
            text
        } else {
            content.as_array()?.iter().find_map(|part| {
                if string_field(part, &["type"]) == Some("text") {
                    string_field(part, &["text"])
                } else {
                    None
                }
            })?
        };
        let text = text.trim();
        if text.starts_with('<') || text.starts_with('[') {
            return None;
        }
        // Length is clamped centrally in `session_title`; the UI trims for display.
        Some(text.to_string())
    }

    fn humanize_slug(slug: &str) -> String {
        slug.split('-')
            .filter(|segment| !segment.is_empty())
            .map(|segment| {
                let mut chars = segment.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => {
                        let mut word = first.to_uppercase().to_string();
                        word.push_str(chars.as_str());
                        word
                    }
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn set_latest_string(slot: &mut Option<String>, value: Option<&str>) {
        if let Some(value) = value {
            *slot = Some(value.to_string());
        }
    }

    fn list_head_scan_complete(metadata: &ClaudeSessionMetadata, _scanned_lines: usize) -> bool {
        let core_metadata = !metadata.session_id.is_empty()
            && metadata.cwd.is_some()
            && metadata.created_at.is_some();
        // Do not stop after N lines without a title: ai-title records often appear after
        // large hook/system preamble lines in newer Claude Code transcripts.
        core_metadata && metadata.title.is_some()
    }

    fn load_sessions_index(project_dir: &Path) -> HashMap<String, SessionsIndexEntry> {
        let path = project_dir.join("sessions-index.json");
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return HashMap::new();
        };
        let Ok(index) = serde_json::from_str::<SessionsIndexFile>(&contents) else {
            return HashMap::new();
        };
        index
            .entries
            .into_iter()
            .map(|entry| (entry.session_id.clone(), entry))
            .collect()
    }

    fn apply_sessions_index(
        metadata: &mut ClaudeSessionMetadata,
        index: &HashMap<String, SessionsIndexEntry>,
    ) {
        let Some(entry) = index.get(&metadata.session_id) else {
            return;
        };
        if let Some(summary) = entry.summary.as_deref().filter(|title| !title.is_empty()) {
            metadata.title = Some(summary.to_string());
        }
        if metadata.created_at.is_none() {
            metadata.created_at = entry.created.clone();
        }
        if metadata.last_activity_at.is_none() {
            metadata.last_activity_at = entry.modified.clone();
        }
        if metadata.git_branch.is_none() {
            metadata.git_branch = entry.git_branch.clone();
        }
        if metadata.cwd.is_none() {
            metadata.cwd = entry.project_path.clone();
        }
    }
}

// ---------- agent dispatcher integration ----------

use super::utils::{
    file_size_bytes, home_path, humanize_plan_token, info_field, mtime_unix_secs, parse_rfc3339,
    parse_usage_window_resets_at, push_json_string, read_json_file, resume_summary, session_title,
    sorted_jsonl_candidates, spawn_blocking_opt, string_field,
};
use zedra_rpc::proto::*;

impl ClaudeActor {
    fn session_summary(
        session: &ClaudeSessionMetadata,
        _cli: &AgentCliSummary,
    ) -> AgentSessionSummary {
        AgentSessionSummary {
            slug: "claude".to_string(),
            session_id: session.session_id.clone(),
            title: session_title(session.title.clone()),
            cwd: session.cwd.clone(),
            created_at: parse_rfc3339(session.created_at.as_deref()),
            last_activity_at: parse_rfc3339(session.last_activity_at.as_deref()),
            resume: resume_summary("claude", &session.session_id),
            git: Some(AgentGitSummary {
                branch: session.git_branch.clone(),
                worktree: session.worktree.clone(),
                commit_hash: None,
                repository_url: None,
                pr_number: None,
                pr_url: None,
                pr_repository: None,
            }),
            usage: None,
            transcript_size_bytes: file_size_bytes(&session.transcript_path),
        }
    }
}

// ---------- account / plan / usage ----------

impl ClaudeActor {
    fn daily_message_count_today(value: &Value) -> Option<u64> {
        let activity = value.get("dailyActivity")?.as_array()?;
        let today_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mut messages = 0;
        for entry in activity {
            let date = entry.get("date")?.as_str()?;
            if date != today_str {
                continue;
            }
            messages += entry
                .get("messageCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }
        Some(messages)
    }

    fn total_cost_usd(value: &Value) -> Option<f64> {
        let usage = value.get("modelUsage")?.as_object()?;
        let mut total = 0.0;
        for model in usage.values() {
            if let Some(cost) = model.get("costUSD").and_then(|v| v.as_f64()) {
                total += cost;
            }
        }
        (total > 0.0).then_some(total)
    }

    fn append_auth_plan_fields(fields: &mut Vec<AgentInfoField>) {
        let creds = Self::read_credentials();
        let logged_in = creds
            .as_ref()
            .and_then(ClaudeCredentials::valid_token)
            .is_some();
        fields.push(AgentInfoField {
            label: "Logged in".to_string(),
            value: if logged_in { "yes" } else { "no" }.to_string(),
        });
        if let Some(plan) = creds.as_ref().and_then(Self::plan_from_credentials) {
            fields.push(AgentInfoField {
                label: "Plan".to_string(),
                value: plan,
            });
        }
    }

    fn plan_from_credentials(creds: &ClaudeCredentials) -> Option<String> {
        Self::format_plan_label(
            creds.subscription_type.as_deref().unwrap_or(""),
            creds.rate_limit_tier.as_deref(),
        )
    }

    pub fn format_plan_label(
        subscription_type: &str,
        rate_limit_tier: Option<&str>,
    ) -> Option<String> {
        let trimmed = subscription_type.trim();
        if !trimmed.is_empty() {
            return Some(humanize_plan_token(trimmed));
        }
        super::utils::plan_label_from_token(rate_limit_tier?)
    }

    pub async fn fetch_subscription_plan() -> Option<Vec<AgentInfoField>> {
        if Self::read_oauth_token().is_some() {
            if let Some(fields) = Self::fetch_oauth_profile().await {
                return Some(fields);
            }
            tracing::debug!(
                target: "zedra_host::agent",
                "claude oauth profile unavailable; trying cli pty"
            );
        }
        #[cfg(unix)]
        if let Some(fields) = super::claude_probe::fetch_plan_fields().await {
            return Some(fields);
        }
        spawn_blocking_opt(Self::subscription_plan_fields_from_disk).await
    }

    async fn fetch_oauth_profile() -> Option<Vec<AgentInfoField>> {
        let token = Self::read_oauth_token()?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .ok()?;
        let resp = client
            .get("https://api.anthropic.com/api/oauth/profile")
            .header("Authorization", format!("Bearer {token}"))
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("Accept", "application/json")
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            tracing::debug!("claude oauth profile returned {}", resp.status());
            return None;
        }
        let body: Value = resp.json().await.ok()?;
        let mut fields = Vec::new();
        fields.push(AgentInfoField {
            label: "Logged in".to_string(),
            value: "yes".to_string(),
        });
        let subscription_type = body
            .get("subscription_type")
            .or_else(|| body.get("subscriptionType"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let rate_limit_tier = body
            .get("rate_limit_tier")
            .or_else(|| body.get("rateLimitTier"))
            .and_then(|v| v.as_str());
        if let Some(plan) = Self::format_plan_label(subscription_type, rate_limit_tier) {
            fields.push(AgentInfoField {
                label: "Plan".to_string(),
                value: plan,
            });
        }
        if let Some(org) = body
            .get("organization")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .filter(|name| !name.is_empty())
        {
            fields.push(AgentInfoField {
                label: "Organization".to_string(),
                value: org.to_string(),
            });
        }
        Some(fields)
    }

    fn subscription_plan_fields_from_disk() -> Option<Vec<AgentInfoField>> {
        let mut fields = Vec::new();
        Self::append_auth_plan_fields(&mut fields);
        if fields.len() == 1 && fields[0].label == "Logged in" && fields[0].value == "no" {
            return None;
        }
        (!fields.is_empty()).then_some(fields)
    }

    pub async fn fetch_account_usage() -> Option<AgentUsageSnapshot> {
        if Self::read_oauth_token().is_some() {
            if let Some(snap) = Self::fetch_oauth_usage().await {
                return Some(snap);
            }
            tracing::debug!(
                target: "zedra_host::agent",
                "claude oauth usage unavailable; trying cli pty"
            );
        }
        #[cfg(unix)]
        {
            super::claude_probe::fetch_usage().await
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    async fn fetch_oauth_usage() -> Option<AgentUsageSnapshot> {
        let token = Self::read_oauth_token()?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .ok()?;
        let resp = client
            .get("https://api.anthropic.com/api/oauth/usage")
            .header("Authorization", format!("Bearer {token}"))
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("Accept", "application/json")
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            tracing::debug!("claude usage API returned {}", resp.status());
            return None;
        }
        let body: Value = resp.json().await.ok()?;
        let window_pct = |obj: Option<&Value>| -> Option<f32> {
            obj.and_then(|w| w.get("utilization"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
        };
        let five_hour_obj = body.get("five_hour");
        let seven_day_obj = body.get("seven_day");
        let five_hour = window_pct(five_hour_obj);
        let seven_day = window_pct(seven_day_obj);
        let five_hour_resets_at = five_hour_obj.and_then(parse_usage_window_resets_at);
        let seven_day_resets_at = seven_day_obj.and_then(parse_usage_window_resets_at);

        // Per-model weekly windows and extra credits, surfaced as `extra`.
        let mut extra = Vec::new();
        if let Some(pct) = window_pct(body.get("seven_day_opus")) {
            extra.push(info_field("Opus weekly", &format!("{pct:.0}%")));
        }
        if let Some(pct) = window_pct(body.get("seven_day_sonnet")) {
            extra.push(info_field("Sonnet weekly", &format!("{pct:.0}%")));
        }
        let credits = body.get("extra_usage");
        let used_credits = credits
            .and_then(|e| e.get("used_credits"))
            .and_then(|v| v.as_f64());
        let credit_limit = credits
            .and_then(|e| e.get("monthly_limit"))
            .and_then(|v| v.as_f64());
        if let Some(used) = used_credits {
            let value = match credit_limit {
                Some(limit) => format!("${used:.2} / ${limit:.2}"),
                None => format!("${used:.2}"),
            };
            extra.push(info_field("Extra credits", &value));
        }

        Some(AgentUsageSnapshot {
            rate_limit_five_hour_used_percent: five_hour,
            rate_limit_seven_day_used_percent: seven_day,
            rate_limit_five_hour_resets_at: five_hour_resets_at,
            rate_limit_seven_day_resets_at: seven_day_resets_at,
            extra,
        })
    }

    /// Reads and parses `~/.claude/.credentials.json` once; `None` if missing,
    /// malformed, or lacking the `claudeAiOauth` block.
    fn read_credentials() -> Option<ClaudeCredentials> {
        let path = Self::claude_config_dir_or_home().join(".credentials.json");
        let contents = std::fs::read_to_string(&path).ok()?;
        let root: Value = serde_json::from_str(&contents).ok()?;
        let oauth = root.get("claudeAiOauth")?;
        Some(ClaudeCredentials {
            access_token: oauth
                .get("accessToken")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            subscription_type: oauth
                .get("subscriptionType")
                .or_else(|| oauth.get("subscription_type"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            rate_limit_tier: oauth
                .get("rateLimitTier")
                .or_else(|| oauth.get("rate_limit_tier"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            expires_at: oauth
                .get("expiresAt")
                .and_then(|v| v.as_f64())
                .map(|v| v as i64),
        })
    }

    /// Claude OAuth access token; `None` if missing, malformed, or expired.
    fn read_oauth_token() -> Option<String> {
        Self::read_credentials()?.valid_token().map(str::to_string)
    }
}

/// Parsed `~/.claude/.credentials.json` OAuth fields; absent fields stay `None`.
struct ClaudeCredentials {
    access_token: Option<String>,
    subscription_type: Option<String>,
    rate_limit_tier: Option<String>,
    expires_at: Option<i64>,
}

impl ClaudeCredentials {
    /// Access token when present and unexpired; logs and returns `None` when the
    /// token has expired.
    fn valid_token(&self) -> Option<&str> {
        if let Some(expires_ms) = self.expires_at {
            let expires =
                std::time::UNIX_EPOCH + std::time::Duration::from_millis(expires_ms as u64);
            if std::time::SystemTime::now() > expires {
                tracing::debug!("claude oauth token expired; skipping usage probe");
                return None;
            }
        }
        self.access_token.as_deref()
    }
}

#[cfg(test)]
mod account_tests {
    use super::*;

    #[test]
    fn format_plan_prefers_subscription_type() {
        assert_eq!(
            ClaudeActor::format_plan_label("max", Some("default_claude_pro")),
            Some("Max".to_string())
        );
        assert_eq!(
            ClaudeActor::format_plan_label("claude_pro", None),
            Some("Pro".to_string())
        );
        assert_eq!(
            ClaudeActor::format_plan_label("", Some("default_claude_max_20x")),
            Some("Max".to_string())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_uses_claude_path_encoding() {
        assert_eq!(
            ClaudeActor::project_dir_for_workdir(
                Path::new("/config"),
                Path::new("/Users/me/project")
            ),
            PathBuf::from("/config/projects/-Users-me-project")
        );
        assert_eq!(
            ClaudeActor::encoded_project_name(Path::new(r"C:\Users\me\project")),
            "C:-Users-me-project"
        );
    }

    #[test]
    fn list_sessions_reads_workspace_transcripts() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let project_dir = ClaudeActor::project_dir_for_workdir(config.path(), workdir);
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("older.jsonl"),
            r#"{"sessionId":"older-session","cwd":"/Users/me/project","gitBranch":"main","version":"1.0.0","timestamp":"2026-05-09T10:00:00Z","isSidechain":false}
"#,
        )
        .unwrap();
        std::fs::write(
            project_dir.join("newer.jsonl"),
            r#"not-json
{"sessionId":"newer-session","cwd":"/Users/me/project","gitBranch":"feature","version":"1.0.1","timestamp":"2026-05-09T09:00:00Z","isSidechain":false}
{"cwd":"/Users/me/project","gitBranch":"feature-2","version":"1.0.2","timestamp":"2026-05-09T11:00:00Z","isSidechain":true}
"#,
        )
        .unwrap();
        // List mode orders by file mtime, so pin them to make "newer" genuinely newer.
        filetime::set_file_mtime(
            project_dir.join("older.jsonl"),
            filetime::FileTime::from_unix_time(1_700_000_000, 0),
        )
        .unwrap();
        filetime::set_file_mtime(
            project_dir.join("newer.jsonl"),
            filetime::FileTime::from_unix_time(1_800_000_000, 0),
        )
        .unwrap();

        let list = ClaudeActor::list_sessions_in_config(workdir, config.path(), None).unwrap();

        assert_eq!(list.sessions.len(), 2);
        assert_eq!(list.sessions[0].session_id, "newer-session");
        assert_eq!(list.sessions[0].git_branch.as_deref(), Some("feature-2"));
        assert_eq!(list.sessions[1].session_id, "older-session");
    }

    #[test]
    fn list_sessions_omits_transcript_content_from_metadata() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let project_dir = ClaudeActor::project_dir_for_workdir(config.path(), workdir);
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("session.jsonl"),
            r#"{"sessionId":"session","cwd":"/Users/me/project","timestamp":"2026-05-09T10:00:00Z","prompt":"SECRET"}
"#,
        )
        .unwrap();

        let session =
            ClaudeActor::read_transcript_list_metadata(&project_dir.join("session.jsonl"), None)
                .unwrap();

        // List scanning never captures prompt content, so the serialized metadata
        // must not leak it (telemetry-privacy rule).
        let serialized = serde_json::to_string(&session).unwrap();
        assert!(!serialized.contains("SECRET"));
    }

    #[test]
    fn list_sessions_falls_back_to_filename_session_id() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let project_dir = ClaudeActor::project_dir_for_workdir(config.path(), workdir);
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("session-from-file.jsonl"),
            r#"{"cwd":"/Users/me/project","timestamp":"2026-05-09T10:00:00Z"}
"#,
        )
        .unwrap();

        let list = ClaudeActor::list_sessions_in_config(workdir, config.path(), None).unwrap();

        assert_eq!(list.sessions[0].session_id, "session-from-file");
    }

    #[test]
    fn worktree_from_project_dir_reads_marker_suffix() {
        let dir = Path::new("/config/projects/-Users-me-project--claude-worktrees-feature-a");
        assert_eq!(
            ClaudeActor::worktree_from_project_dir(dir).as_deref(),
            Some("feature-a")
        );
    }

    #[test]
    fn project_dirs_exclude_sibling_workdir_project_dirs() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project-main");
        let projects_root = config.path().join("projects");
        std::fs::create_dir_all(projects_root.join("-Users-me-project-main")).unwrap();
        std::fs::create_dir_all(projects_root.join("-Users-me-project")).unwrap();
        std::fs::create_dir_all(projects_root.join("-Users-me-project-ios")).unwrap();
        std::fs::create_dir_all(
            projects_root.join("-Users-me-project--claude-worktrees-feature-a"),
        )
        .unwrap();

        let dirs = ClaudeActor::project_dirs_for_workdir(config.path(), workdir);
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0], projects_root.join("-Users-me-project-main"));
    }

    #[test]
    fn project_dirs_include_claude_worktree_suffix_dirs() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let projects_root = config.path().join("projects");
        std::fs::create_dir_all(projects_root.join("-Users-me-project")).unwrap();
        std::fs::create_dir_all(
            projects_root.join("-Users-me-project--claude-worktrees-feature-a"),
        )
        .unwrap();

        let dirs = ClaudeActor::project_dirs_for_workdir(config.path(), workdir);
        assert_eq!(dirs.len(), 2);
        assert!(dirs.iter().any(|dir| {
            dir.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("--claude-worktrees-feature-a"))
        }));
    }

    #[test]
    fn list_sessions_reads_late_ai_title_after_hook_preamble() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let project_dir = ClaudeActor::project_dir_for_workdir(config.path(), workdir);
        std::fs::create_dir_all(&project_dir).unwrap();
        let mut contents = String::new();
        for index in 0..9 {
            contents.push_str(&format!(
                r#"{{"sessionId":"session","cwd":"/Users/me/project","timestamp":"2026-05-09T10:00:{index:02}Z","type":"attachment"}}
"#
            ));
        }
        contents.push_str(
            r#"{"type":"ai-title","aiTitle":"Update terminal actions UI design","sessionId":"session"}
"#,
        );
        std::fs::write(project_dir.join("session.jsonl"), contents).unwrap();

        let session =
            ClaudeActor::read_transcript_list_metadata(&project_dir.join("session.jsonl"), None)
                .unwrap();
        assert_eq!(
            session.title.as_deref(),
            Some("Update terminal actions UI design")
        );
    }

    #[test]
    fn list_sessions_reads_sessions_index_summary() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let project_dir = ClaudeActor::project_dir_for_workdir(config.path(), workdir);
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("session.jsonl"),
            r#"{"sessionId":"session","cwd":"/Users/me/project","timestamp":"2026-05-09T10:00:00Z"}
"#,
        )
        .unwrap();
        std::fs::write(
            project_dir.join("sessions-index.json"),
            r#"{
  "version": 1,
  "entries": [
    {
      "sessionId": "session",
      "summary": "Agent session history titles",
      "created": "2026-05-09T10:00:00Z",
      "modified": "2026-05-09T11:00:00Z",
      "gitBranch": "main",
      "messageCount": 12
    }
  ]
}"#,
        )
        .unwrap();

        let list = ClaudeActor::list_sessions_in_config(workdir, config.path(), Some(10)).unwrap();
        let session = &list.sessions[0];
        assert_eq!(
            session.title.as_deref(),
            Some("Agent session history titles")
        );
        assert_eq!(session.git_branch.as_deref(), Some("main"));
    }

    #[test]
    fn list_sessions_reads_ai_title_beyond_head_byte_limit() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let project_dir = ClaudeActor::project_dir_for_workdir(config.path(), workdir);
        std::fs::create_dir_all(&project_dir).unwrap();
        let mut contents = String::from(
            r#"{"sessionId":"session","cwd":"/Users/me/project","timestamp":"2026-05-09T10:00:00Z"}"#,
        );
        for index in 0..12 {
            contents.push('\n');
            contents.push_str(&format!(
                r#"{{"sessionId":"session","cwd":"/Users/me/project","timestamp":"2026-05-09T10:00:{index:02}Z","attachment":{{"content":"{padding}"}}}}"#,
                padding = "x".repeat(3_000)
            ));
        }
        contents.push_str(
            r#"
{"type":"ai-title","aiTitle":"Plan commit without coauthor","sessionId":"session"}
"#,
        );
        std::fs::write(project_dir.join("session.jsonl"), contents).unwrap();

        let session =
            ClaudeActor::read_transcript_list_metadata(&project_dir.join("session.jsonl"), None)
                .unwrap();
        assert_eq!(
            session.title.as_deref(),
            Some("Plan commit without coauthor")
        );
    }

    #[test]
    fn list_sessions_falls_back_to_last_prompt() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let project_dir = ClaudeActor::project_dir_for_workdir(config.path(), workdir);
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("session.jsonl"),
            r#"{"sessionId":"session","cwd":"/Users/me/project","timestamp":"2026-05-09T10:00:00Z"}
{"type":"user","message":{"role":"user","content":"clear"},"timestamp":"2026-05-09T10:00:01Z","sessionId":"session","cwd":"/Users/me/project"}
{"type":"last-prompt","lastPrompt":"clear","sessionId":"session"}
"#,
        )
        .unwrap();

        let session =
            ClaudeActor::read_transcript_list_metadata(&project_dir.join("session.jsonl"), None)
                .unwrap();
        assert_eq!(session.title.as_deref(), Some("clear"));
    }

    #[test]
    fn list_sessions_reads_ai_title() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let project_dir = ClaudeActor::project_dir_for_workdir(config.path(), workdir);
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("session.jsonl"),
            r#"{"type":"summary","aiTitle":"Review pending changes"}
{"sessionId":"session","cwd":"/Users/me/project","timestamp":"2026-05-09T10:00:00Z"}
"#,
        )
        .unwrap();

        let list = ClaudeActor::list_sessions_in_config(workdir, config.path(), Some(10)).unwrap();
        let session = &list.sessions[0];
        assert_eq!(session.title.as_deref(), Some("Review pending changes"));
    }

    #[test]
    fn list_mode_scans_only_transcript_head() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let project_dir = ClaudeActor::project_dir_for_workdir(config.path(), workdir);
        std::fs::create_dir_all(&project_dir).unwrap();
        let mut contents = String::from(
            r#"{"sessionId":"session","cwd":"/Users/me/project","timestamp":"2026-05-09T10:00:00Z"}"#,
        );
        for index in 0..200 {
            contents.push('\n');
            contents.push_str(&format!(
                r#"{{"sessionId":"session","cwd":"/Users/me/project","timestamp":"2026-05-09T10:00:{index:02}Z"}}"#
            ));
        }
        std::fs::write(project_dir.join("large.jsonl"), contents).unwrap();

        let session =
            ClaudeActor::read_transcript_list_metadata(&project_dir.join("large.jsonl"), None)
                .unwrap();
        assert_eq!(session.session_id, "session");
    }

    #[test]
    fn session_count_summary_reads_only_latest_transcript() {
        let config = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project-count");
        let project_dir = ClaudeActor::project_dir_for_workdir(config.path(), workdir);
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("older.jsonl"),
            r#"{"sessionId":"older","cwd":"/Users/me/project-count","timestamp":"2026-05-09T10:00:00Z","aiTitle":"Older"}
"#,
        )
        .unwrap();
        let newer_path = project_dir.join("newer.jsonl");
        std::fs::write(
            &newer_path,
            r#"{"sessionId":"newer","cwd":"/Users/me/project-count","timestamp":"2026-05-09T11:00:00Z","aiTitle":"Newer"}
"#,
        )
        .unwrap();
        filetime::set_file_mtime(
            project_dir.join("older.jsonl"),
            filetime::FileTime::from_unix_time(1_700_000_000, 0),
        )
        .unwrap();
        filetime::set_file_mtime(
            &newer_path,
            filetime::FileTime::from_unix_time(1_800_000_000, 0),
        )
        .unwrap();

        let (total, latest) =
            ClaudeActor::session_count_summary_in_config(workdir, config.path()).unwrap();
        assert_eq!(total, 2);
        assert_eq!(latest.unwrap().session_id, "newer");
    }

    #[test]
    fn missing_project_dir_returns_empty_list() {
        let config = tempfile::tempdir().unwrap();
        let list = ClaudeActor::list_sessions_in_config(
            Path::new("/Users/me/missing"),
            config.path(),
            None,
        )
        .unwrap();

        assert!(list.sessions.is_empty());
    }
}

use super::hook::HookContext;
use super::{
    hook_file_mentions_zedra, hooks_enabled, setup_status, ActorFuture, AgentActor, ScanCtx,
    SessionCounts as ActorSessionCounts,
};
use tracing::warn;

pub(super) struct ClaudeActor;

impl ClaudeActor {
    /// Notification body for a Claude hook: the session title read from the
    /// transcript referenced in the payload.
    async fn hook_notification_body(payload: &Value) -> Option<String> {
        let transcript_path = payload
            .get("transcript_path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        spawn_blocking_opt(move || {
            transcript_path
                .as_deref()
                .and_then(Self::title_from_transcript_path)
        })
        .await
    }
}

impl AgentActor for ClaudeActor {
    fn shows_detail(&self) -> bool {
        true
    }

    fn slug(&self) -> &'static str {
        "claude"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn icon_name(&self) -> &'static str {
        "claude"
    }

    fn programs(&self) -> &'static [&'static str] {
        &["claude"]
    }

    fn detect_aliases(&self) -> &'static [&'static str] {
        &["claude", "claudecode"]
    }

    fn cli_available(&self, workdir: &Path) -> bool {
        super::utils::command_on_path("claude")
            || Self::project_dir_for_workdir(&Self::claude_config_dir_or_home(), workdir).is_dir()
    }

    fn session_counts(&self, ctx: &ScanCtx) -> Result<ActorSessionCounts, String> {
        let (total, latest) =
            Self::session_count_summary(ctx.workdir).map_err(|e| e.to_string())?;
        Ok(ActorSessionCounts::from_latest(
            total,
            latest.as_ref().map(|s| s.session_id.clone()),
            latest.as_ref().and_then(|s| s.title.clone()),
            latest.and_then(|s| parse_rfc3339(s.last_activity_at.as_deref())),
        ))
    }

    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        let list = Self::list_sessions_limited(ctx.workdir, limit).map_err(|e| e.to_string())?;
        let sessions = list
            .sessions
            .iter()
            .map(|session| Self::session_summary(session, ctx.cli))
            .collect();
        Ok((sessions, list.total))
    }

    fn account_fields(&self, _workdir: &Path) -> Vec<AgentInfoField> {
        let mut fields = Vec::new();
        Self::append_auth_plan_fields(&mut fields);
        let config_dir = Self::claude_config_dir_or_home();
        let settings_path = config_dir.join("settings.json");
        if let Ok(value) = read_json_file(&settings_path) {
            push_json_string(&mut fields, "Model", &value, &["model"]);
            push_json_string(&mut fields, "Effort", &value, &["effortLevel"]);
            push_json_string(
                &mut fields,
                "Permission mode",
                &value,
                &["permissions", "defaultMode"],
            );
        }
        let stats_path = config_dir.join("stats-cache.json");
        if let Ok(value) = read_json_file(&stats_path) {
            if let Some(total_cost) = Self::total_cost_usd(&value) {
                fields.push(AgentInfoField {
                    label: "Total cost (USD)".to_string(),
                    value: format!("{total_cost:.4}"),
                });
            }
            if let Some(messages) = Self::daily_message_count_today(&value) {
                let today_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
                if value.get("lastComputedDate").and_then(|v| v.as_str())
                    == Some(today_str.as_str())
                {
                    fields.push(AgentInfoField {
                        label: "Today msgs".to_string(),
                        value: messages.to_string(),
                    });
                }
            }
        }
        fields
    }

    fn setup_summary(&self, available: bool, workdir: &Path) -> AgentSetupSummary {
        let path = Self::claude_config_dir_or_home()
            .join("plugins")
            .join("installed_plugins.json");
        let (plugin_installed, plugin_hooks, error) = match std::fs::read_to_string(path) {
            Ok(contents) => Self::claude_setup_status_from_contents(&contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, false, None),
            Err(error) => (false, false, Some(error.to_string())),
        };
        let hooks_installed = hooks_enabled()
            && (plugin_hooks
                || hook_file_mentions_zedra(&workdir.join(".claude/settings.local.json")));
        setup_status(available, false, plugin_installed, hooks_installed, error)
    }

    fn resume_launch_command(&self, quoted: &str) -> Option<String> {
        Some(format!("claude --resume {quoted}"))
    }

    fn subscription_plan<'a>(&'a self) -> ActorFuture<'a, Option<Vec<AgentInfoField>>> {
        Box::pin(Self::fetch_subscription_plan())
    }

    fn account_usage<'a>(&'a self) -> ActorFuture<'a, Option<AgentUsageSnapshot>> {
        Box::pin(Self::fetch_account_usage())
    }

    fn receive_hook<'a>(&'a self, ctx: HookContext) -> ActorFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            // Claude Code pipes hook JSON with `hook_event_name` and snake_case `session_id`.
            let Some(event_name) = super::utils::payload_string(&ctx.payload, "hook_event_name")
            else {
                // Do not log ctx.payload: it can carry user content (telemetry-privacy rule).
                warn!("claude: hook payload missing or empty hook_event_name; ignoring");
                return Ok(());
            };
            let agent_session_id = super::utils::payload_string(&ctx.payload, "session_id");
            let agent_state = match event_name.as_str() {
                "UserPromptSubmit" => Some(AgentState::Running),
                "PermissionRequest" => Some(AgentState::WaitingApproval),
                "PostToolUse" => Some(AgentState::Running),
                "Stop" => Some(AgentState::Completed),
                _ => None,
            };
            ctx.apply(
                "claude",
                &event_name,
                agent_state,
                agent_session_id.as_deref(),
            )
            .await;

            let name = self.display_name();
            let Some(title) = (match event_name.as_str() {
                "PermissionRequest" => Some(format!("{name} requires approval")),
                "Stop" => Some(format!("{name} finished")),
                _ => None,
            }) else {
                return Ok(());
            };
            ctx.notify(
                name,
                &event_name,
                title,
                Self::hook_notification_body(&ctx.payload),
            )
            .await
        })
    }

    fn supports_setup(&self) -> bool {
        true
    }

    fn setup(&self, workdir: &Path, force: bool) -> anyhow::Result<Vec<PathBuf>> {
        let script_path = super::cli::write_hook_script(workdir, force)?;
        let config_path = workdir.join(".claude/settings.local.json");
        super::utils::write_json_file_checked(
            &config_path,
            &super::utils::hook_config_from_events(&script_path, "claude", Self::HOOK_EVENTS),
            force,
            "Claude local hook config",
        )?;
        Ok(vec![script_path, config_path])
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
            ctx.require_command("claude")?;
            match action {
                super::SetupAction::Install => {
                    ctx.install_plugin_and_hooks(&Self::plugin_setup(&ctx)?)
                }
                super::SetupAction::Remove => {
                    ctx.remove_plugin_and_hooks(&Self::plugin_setup(&ctx)?)
                }
            }
        })
    }

    fn hook_test_payload(&self, event_name: &str, workdir: &Path) -> serde_json::Value {
        serde_json::json!({
            "hook_event_name": event_name,
            "session_id": "zedra-test-session",
            "transcript_path": "/tmp/zedra-test-session.jsonl",
            "cwd": workdir,
            "tool_name": "Bash",
            "tool_use_id": "toolu_zedra_test",
        })
    }
}

impl ClaudeActor {
    pub(crate) fn claude_setup_status_from_contents(
        contents: &str,
    ) -> (bool, bool, Option<String>) {
        let value: serde_json::Value = match serde_json::from_str(contents) {
            Ok(value) => value,
            Err(error) => return (false, false, Some(error.to_string())),
        };
        let Some(path) = value
            .get("plugins")
            .and_then(|plugins| plugins.get("zedra@zedra"))
            .and_then(|entries| entries.as_array())
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.get("installPath"))
            .and_then(|path| path.as_str())
        else {
            return (false, false, None);
        };
        (
            true,
            Path::new(path).join("hooks/hooks.json").is_file(),
            None,
        )
    }
}

// ---------------------------------------------------------------------------
// Interactive `zedra setup claude`
// ---------------------------------------------------------------------------

const CLAUDE_PLUGIN: &str = "zedra@zedra";

impl ClaudeActor {
    fn plugin_setup(ctx: &super::SetupCliCtx) -> Result<super::setup::PluginSetup<'static>> {
        Ok(super::setup::PluginSetup {
            display: "Claude",
            program: "claude",
            install_args: &["plugin", "install", "--scope", "user", CLAUDE_PLUGIN],
            uninstall_args: &["plugin", "uninstall", "--scope", "user", CLAUDE_PLUGIN],
            hooks_path: Self::claude_settings_path(ctx)?,
            events: Self::HOOK_EVENTS,
            agent: "claude",
            start_in: "Claude Code",
            start_command: "/zedra-start",
            reload_note: "In Claude Code, reload plugins to apply the change:",
            reload_command: Some("/reload-plugins"),
        })
    }

    fn claude_settings_path(ctx: &super::SetupCliCtx) -> Result<PathBuf> {
        Ok(ctx.home_dir()?.join(".claude").join("settings.json"))
    }
}

// ---------------------------------------------------------------------------
// Workdir-scoped hook config written by `AgentActor::setup`
// ---------------------------------------------------------------------------

impl ClaudeActor {
    // Single source for hook registration; the receive_hook state map consumes the same names.
    const HOOK_EVENTS: &'static [(&'static str, Option<&'static str>, u64)] = &[
        ("UserPromptSubmit", None, 2),
        ("PermissionRequest", Some("*"), 2),
        ("PostToolUse", Some("*"), 2),
        ("Stop", None, 2),
    ];
}

#[cfg(test)]
mod hook_config_tests {
    use super::*;

    #[test]
    fn claude_hook_config_includes_turn_tool_and_session_events() {
        let config = crate::agent::utils::hook_config_from_events(
            Path::new("/tmp/zedra-hook.sh"),
            "claude",
            ClaudeActor::HOOK_EVENTS,
        );
        let hooks = config["hooks"].as_object().unwrap();

        for &(event, _, _) in ClaudeActor::HOOK_EVENTS {
            assert!(hooks.contains_key(event), "missing {event}");
        }
        assert!(!hooks.contains_key("PreToolUse"));
        assert!(!hooks.contains_key("SessionEnd"));
    }
}
