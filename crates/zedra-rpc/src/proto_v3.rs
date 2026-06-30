// Frozen `zedra/rpc/3` wire schema, for clients built before the `zedra/rpc/4`
// ALPN bump. The host advertises both ALPNs and serves a `zedra/rpc/3` client by
// decoding requests with `ZedraProtoV3`, lifting them into `proto::ZedraMessage`
// via `into_live`, and reusing the live dispatch; the only seam is the per-RPC
// channel wrap that re-encodes responses to the `zedra/rpc/3` shape.
//
// Wire contract for shipped binaries — do not edit types or variant order. Only
// types that diverged at `zedra/rpc/4` are defined here; the rest reuse `proto`
// (pinned by the roundtrip tests below). A future wire change must bump the ALPN
// (§2.4) and re-freeze the reused types.
//
// `v3`->`v4` divergence: agents moved from the `AgentKind` enum to slug strings
// (`AgentCapabilities` removed), `TerminalSyncEntry` gained `agent_slug`,
// `AgentUsageSnapshot` gained `extra`, and `HostEvent` gained
// `TerminalAgentChanged`. Everything else is byte-identical.

use chrono::{DateTime, Utc};
use irpc::channel::{mpsc, oneshot};
use irpc::rpc_requests;
use serde::{Deserialize, Serialize};

use crate::proto;

/// Previous ALPN, advertised alongside `ZEDRA_ALPN`.
pub const ZEDRA_ALPN_V3: &[u8] = b"zedra/rpc/3";

// ---------------------------------------------------------------------------
// Protocol enum (frozen at the final `zedra/rpc/3` variant set and order)
// ---------------------------------------------------------------------------

#[rpc_requests(message = ZedraMessageV3)]
#[derive(Serialize, Deserialize, Debug)]
pub enum ZedraProtoV3 {
    #[rpc(tx = oneshot::Sender<proto::RegisterResult>)]
    Register(proto::RegisterReq),
    #[rpc(tx = oneshot::Sender<proto::AuthChallengeResult>)]
    Authenticate(proto::AuthReq),
    #[rpc(tx = oneshot::Sender<AuthProveResult>)]
    AuthProve(proto::AuthProveReq),
    #[rpc(tx = oneshot::Sender<ConnectResult>)]
    Connect(proto::ConnectReq),
    #[rpc(tx = oneshot::Sender<proto::PongResult>)]
    Ping(proto::PingReq),
    #[rpc(tx = oneshot::Sender<proto::SessionInfoResult>)]
    GetSessionInfo(proto::SessionInfoReq),
    #[rpc(tx = oneshot::Sender<proto::SessionListResult>)]
    ListSessions(proto::SessionListReq),
    #[rpc(tx = oneshot::Sender<proto::SessionSwitchResult>)]
    SwitchSession(proto::SessionSwitchReq),
    #[rpc(tx = oneshot::Sender<proto::FsListResult>)]
    FsList(proto::FsListReq),
    #[rpc(tx = oneshot::Sender<proto::FsReadResult>)]
    FsRead(proto::FsReadReq),
    #[rpc(tx = oneshot::Sender<proto::FsWriteResult>)]
    FsWrite(proto::FsWriteReq),
    #[rpc(tx = oneshot::Sender<proto::FsStatResult>)]
    FsStat(proto::FsStatReq),
    #[rpc(tx = oneshot::Sender<proto::TermCreateResult>)]
    TermCreate(proto::TermCreateReq),
    #[rpc(tx = mpsc::Sender<HostEvent>)]
    Subscribe(proto::SubscribeReq),
    #[rpc(rx = mpsc::Receiver<proto::TermInput>, tx = mpsc::Sender<proto::TermOutput>)]
    TermAttach(proto::TermAttachReq),
    #[rpc(tx = oneshot::Sender<proto::TermResizeResult>)]
    TermResize(proto::TermResizeReq),
    #[rpc(tx = oneshot::Sender<proto::TermCloseResult>)]
    TermClose(proto::TermCloseReq),
    #[rpc(tx = oneshot::Sender<proto::TermListResult>)]
    TermList(proto::TermListReq),
    #[rpc(tx = oneshot::Sender<proto::GitStatusResult>)]
    GitStatus(proto::GitStatusReq),
    #[rpc(tx = oneshot::Sender<proto::GitDiffResult>)]
    GitDiff(proto::GitDiffReq),
    #[rpc(tx = oneshot::Sender<proto::GitLogResult>)]
    GitLog(proto::GitLogReq),
    #[rpc(tx = oneshot::Sender<proto::GitCommitResult>)]
    GitCommit(proto::GitCommitReq),
    #[rpc(tx = oneshot::Sender<proto::GitStageResult>)]
    GitStage(proto::GitStageReq),
    #[rpc(tx = oneshot::Sender<proto::GitUnstageResult>)]
    GitUnstage(proto::GitUnstageReq),
    #[rpc(tx = oneshot::Sender<proto::GitBranchesResult>)]
    GitBranches(proto::GitBranchesReq),
    #[rpc(tx = oneshot::Sender<proto::GitCheckoutResult>)]
    GitCheckout(proto::GitCheckoutReq),
    #[rpc(tx = oneshot::Sender<proto::AiPromptResult>)]
    AiPrompt(proto::AiPromptReq),
    #[rpc(tx = oneshot::Sender<proto::LspDiagnosticsResult>)]
    LspDiagnostics(proto::LspDiagnosticsReq),
    #[rpc(tx = oneshot::Sender<proto::LspHoverResult>)]
    LspHover(proto::LspHoverReq),
    #[rpc(tx = oneshot::Sender<proto::FsWatchResult>)]
    FsWatch(proto::FsWatchReq),
    #[rpc(tx = oneshot::Sender<proto::FsUnwatchResult>)]
    FsUnwatch(proto::FsUnwatchReq),
    #[rpc(tx = oneshot::Sender<SyncSessionResult>)]
    SyncSession(proto::SyncSessionReq),
    #[rpc(tx = mpsc::Sender<proto::HostInfoSnapshot>)]
    SubscribeHostInfo(proto::SubscribeHostInfoReq),
    #[rpc(tx = oneshot::Sender<proto::TermReorderResult>)]
    TermReorder(proto::TermReorderReq),
    #[rpc(tx = oneshot::Sender<proto::FsDocsTreeResult>)]
    FsDocsTree(proto::FsDocsTreeReq),
    #[rpc(tx = oneshot::Sender<AgentListResult>)]
    AgentList(proto::AgentListReq),
    #[rpc(tx = oneshot::Sender<AgentSessionsResult>)]
    AgentSessions(AgentSessionsReq),
    #[rpc(tx = oneshot::Sender<proto::AgentResumeResult>)]
    AgentResume(AgentResumeReq),
    #[rpc(tx = oneshot::Sender<proto::AgentInstalledListResult>)]
    AgentInstalledList(proto::AgentInstalledListReq),
    #[rpc(tx = oneshot::Sender<proto::TermCreateResult>)]
    TermCreateV2(proto::TermCreateReqV2),
    #[rpc(tx = oneshot::Sender<proto::AgentFilesResult>)]
    AgentFiles(AgentFilesReq),
    #[rpc(tx = oneshot::Sender<proto::FsSearchResult>)]
    FsSearch(proto::FsSearchReq),
    #[rpc(tx = oneshot::Sender<proto::SetAppStateResult>)]
    SetAppState(proto::SetAppStateReq),
    #[rpc(tx = oneshot::Sender<proto::SetClientDeltaInfoResult>)]
    SetClientDeltaInfo(proto::SetClientDeltaInfoReq),
    #[rpc(tx = oneshot::Sender<proto::ClearClientDeltaInfoResult>)]
    ClearClientDeltaInfo(proto::ClearClientDeltaInfoReq),
}

// ---------------------------------------------------------------------------
// Divergent types — frozen at their final `zedra/rpc/3` wire shape
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSyncEntry {
    pub id: String,
    pub position: u64,
    pub last_seq: u64,
    pub title: Option<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub icon_name: Option<String>,
    #[serde(default)]
    pub agent_command: Option<String>,
    #[serde(default)]
    pub shell_state: proto::TermShellState,
    #[serde(default)]
    pub last_exit_code: Option<i32>,
    #[serde(default)]
    pub agent_state: proto::AgentState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSessionResult {
    pub session_id: String,
    pub session_token: [u8; 32],
    pub hostname: String,
    pub workdir: String,
    pub username: String,
    pub home_dir: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub os_version: Option<String>,
    pub host_version: Option<String>,
    pub delta_pubkey: [u8; 32],
    pub terminals: Vec<TerminalSyncEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ConnectResult {
    Ok(SyncSessionResult),
    Challenge {
        nonce: [u8; 32],
        #[serde(with = "crate::proto::bytes64")]
        host_signature: [u8; 64],
    },
    Unauthorized,
    NotInSessionAcl,
    SessionOccupied,
    SessionNotFound,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum AuthProveResult {
    Ok(SyncSessionResult),
    Unauthorized,
    NotInSessionAcl,
    SessionOccupied,
    SessionNotFound,
    InvalidSignature,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostEvent {
    TerminalCreated {
        id: String,
        launch_cmd: Option<String>,
    },
    GitChanged,
    FsChanged {
        path: String,
    },
    AgentInfoChanged {
        info: AgentSummary,
    },
    AgentHookReceived {
        agent_kind: AgentKind,
        event_name: String,
        payload: String,
    },
    AgentStateChanged {
        terminal_id: String,
        agent_session_id: String,
        state: proto::AgentState,
    },
}

/// Frozen `zedra/rpc/3` managed-agent enum. The live protocol replaced it with
/// slug strings at `v4`; only `claude`/`codex`/`opencode`/`pi`/`hermes` can be
/// represented here, so newer actor slugs are filtered out of frozen responses.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
    Pi,
    Hermes,
}

/// Frozen `zedra/rpc/3` capability payload. Live agent actors no longer expose a
/// capability registry; this shape remains only to preserve v3 bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilities {
    pub list_sessions: bool,
    pub resume_session: bool,
    pub live_binding: bool,
    pub confirm_action: bool,
    pub select_action: bool,
    pub lifecycle_events: bool,
    pub usage_snapshot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    pub kind: AgentKind,
    pub display_name: String,
    pub cli: proto::AgentCliSummary,
    pub setup: proto::AgentSetupSummary,
    pub capabilities: AgentCapabilities,
    pub workspace: proto::AgentWorkspaceSummary,
    pub sessions: proto::AgentSessionCounts,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub data_sources: Vec<proto::AgentDataSource>,
    pub warnings: Vec<proto::AgentWarning>,
    pub account: proto::AgentAccountSummary,
    pub usage: Option<AgentUsageSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentListResult {
    pub agents: Vec<AgentSummary>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionSummary {
    pub kind: AgentKind,
    pub session_id: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub resume: proto::AgentResumeSummary,
    pub git: Option<proto::AgentGitSummary>,
    pub usage: Option<AgentUsageSnapshot>,
    pub transcript_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionsResult {
    pub sessions: Vec<AgentSessionSummary>,
    pub total: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionsReq {
    pub kind: AgentKind,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResumeReq {
    pub kind: AgentKind,
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFilesReq {
    pub kind: AgentKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentUsageSnapshot {
    pub context_used_percent: Option<f32>,
    pub total_cost_usd: Option<f64>,
    pub total_duration_ms: Option<u64>,
    pub total_api_duration_ms: Option<u64>,
    pub lines_added: Option<i64>,
    pub lines_removed: Option<i64>,
    pub rate_limit_five_hour_used_percent: Option<f32>,
    pub rate_limit_seven_day_used_percent: Option<f32>,
    pub rate_limit_five_hour_resets_at: Option<i64>,
    pub rate_limit_seven_day_resets_at: Option<i64>,
}

// ---------------------------------------------------------------------------
// Live (v4) -> `zedra/rpc/3` response conversions
// ---------------------------------------------------------------------------

impl From<proto::TerminalSyncEntry> for TerminalSyncEntry {
    fn from(t: proto::TerminalSyncEntry) -> Self {
        // Drops `agent_slug` appended at `zedra/rpc/4`.
        Self {
            id: t.id,
            position: t.position,
            last_seq: t.last_seq,
            title: t.title,
            cwd: t.cwd,
            icon_name: t.icon_name,
            agent_command: t.agent_command,
            shell_state: t.shell_state,
            last_exit_code: t.last_exit_code,
            agent_state: t.agent_state,
        }
    }
}

impl From<proto::SyncSessionResult> for SyncSessionResult {
    fn from(s: proto::SyncSessionResult) -> Self {
        Self {
            session_id: s.session_id,
            session_token: s.session_token,
            hostname: s.hostname,
            workdir: s.workdir,
            username: s.username,
            home_dir: s.home_dir,
            os: s.os,
            arch: s.arch,
            os_version: s.os_version,
            host_version: s.host_version,
            delta_pubkey: s.delta_pubkey,
            terminals: s.terminals.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<proto::ConnectResult> for ConnectResult {
    fn from(r: proto::ConnectResult) -> Self {
        match r {
            proto::ConnectResult::Ok(s) => ConnectResult::Ok(s.into()),
            proto::ConnectResult::Challenge {
                nonce,
                host_signature,
            } => ConnectResult::Challenge {
                nonce,
                host_signature,
            },
            proto::ConnectResult::Unauthorized => ConnectResult::Unauthorized,
            proto::ConnectResult::NotInSessionAcl => ConnectResult::NotInSessionAcl,
            proto::ConnectResult::SessionOccupied => ConnectResult::SessionOccupied,
            proto::ConnectResult::SessionNotFound => ConnectResult::SessionNotFound,
        }
    }
}

impl From<proto::AuthProveResult> for AuthProveResult {
    fn from(r: proto::AuthProveResult) -> Self {
        match r {
            proto::AuthProveResult::Ok(s) => AuthProveResult::Ok(s.into()),
            proto::AuthProveResult::Unauthorized => AuthProveResult::Unauthorized,
            proto::AuthProveResult::NotInSessionAcl => AuthProveResult::NotInSessionAcl,
            proto::AuthProveResult::SessionOccupied => AuthProveResult::SessionOccupied,
            proto::AuthProveResult::SessionNotFound => AuthProveResult::SessionNotFound,
            proto::AuthProveResult::InvalidSignature => AuthProveResult::InvalidSignature,
        }
    }
}

impl From<proto::AgentUsageSnapshot> for AgentUsageSnapshot {
    fn from(u: proto::AgentUsageSnapshot) -> Self {
        // Drops `extra` appended at `zedra/rpc/4`.
        Self {
            context_used_percent: u.context_used_percent,
            total_cost_usd: u.total_cost_usd,
            total_duration_ms: u.total_duration_ms,
            total_api_duration_ms: u.total_api_duration_ms,
            lines_added: u.lines_added,
            lines_removed: u.lines_removed,
            rate_limit_five_hour_used_percent: u.rate_limit_five_hour_used_percent,
            rate_limit_seven_day_used_percent: u.rate_limit_seven_day_used_percent,
            rate_limit_five_hour_resets_at: u.rate_limit_five_hour_resets_at,
            rate_limit_seven_day_resets_at: u.rate_limit_seven_day_resets_at,
        }
    }
}

/// Only the five historical slugs can be represented by the frozen v3 enum.
/// New actors are intentionally filtered rather than changing the v3 schema.
fn agent_kind(slug: &str) -> Option<AgentKind> {
    match slug {
        "claude" => Some(AgentKind::Claude),
        "codex" => Some(AgentKind::Codex),
        "opencode" => Some(AgentKind::OpenCode),
        "pi" => Some(AgentKind::Pi),
        "hermes" => Some(AgentKind::Hermes),
        _ => None,
    }
}

/// Maps a `v3` agent kind up to the live slug.
fn agent_slug_to_live(kind: AgentKind) -> String {
    match kind {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
        AgentKind::OpenCode => "opencode",
        AgentKind::Pi => "pi",
        AgentKind::Hermes => "hermes",
    }
    .to_string()
}

/// Best-effort capabilities for the removed `v3` registry. Live actors expose
/// the underlying features directly, so this reports the full set with a
/// usage snapshot only for the providers that surfaced one.
fn legacy_agent_capabilities(kind: AgentKind, usage_snapshot: bool) -> AgentCapabilities {
    AgentCapabilities {
        list_sessions: true,
        resume_session: true,
        live_binding: true,
        confirm_action: !matches!(kind, AgentKind::Pi),
        select_action: true,
        lifecycle_events: true,
        usage_snapshot,
    }
}

// Request lift: `v3` client -> live handler. The frozen enum becomes a slug.
impl From<AgentSessionsReq> for proto::AgentSessionsReq {
    fn from(r: AgentSessionsReq) -> Self {
        Self {
            slug: agent_slug_to_live(r.kind),
            refresh: r.refresh,
            limit: r.limit,
        }
    }
}

impl From<AgentResumeReq> for proto::AgentResumeReq {
    fn from(r: AgentResumeReq) -> Self {
        Self {
            slug: agent_slug_to_live(r.kind),
            session_id: r.session_id,
            cols: r.cols,
            rows: r.rows,
        }
    }
}

impl From<AgentFilesReq> for proto::AgentFilesReq {
    fn from(r: AgentFilesReq) -> Self {
        Self {
            slug: agent_slug_to_live(r.kind),
        }
    }
}

/// `None` for slugs absent in `v3` (newer actors). The `extra` usage field added
/// at `v4` is dropped via the usage conversion.
fn agent_summary_v3(a: proto::AgentSummary) -> Option<AgentSummary> {
    let kind = agent_kind(&a.slug)?;
    // Derive usage support from the converted summary so a v3 client never sees
    // `usage: Some(..)` while `capabilities.usage_snapshot` stays false.
    let usage_snapshot = a.usage.is_some();
    Some(AgentSummary {
        kind,
        display_name: a.display_name,
        cli: a.cli,
        setup: a.setup,
        capabilities: legacy_agent_capabilities(kind, usage_snapshot),
        workspace: a.workspace,
        sessions: a.sessions,
        last_activity_at: a.last_activity_at,
        updated_at: a.updated_at,
        data_sources: a.data_sources,
        warnings: a.warnings,
        account: a.account,
        usage: a.usage.map(Into::into),
    })
}

impl From<proto::AgentListResult> for AgentListResult {
    fn from(r: proto::AgentListResult) -> Self {
        Self {
            agents: r.agents.into_iter().filter_map(agent_summary_v3).collect(),
            error: r.error,
        }
    }
}

/// `None` for slugs absent in `v3` (newer actors).
fn agent_session_summary_v3(s: proto::AgentSessionSummary) -> Option<AgentSessionSummary> {
    Some(AgentSessionSummary {
        kind: agent_kind(&s.slug)?,
        session_id: s.session_id,
        title: s.title,
        cwd: s.cwd,
        created_at: s.created_at,
        last_activity_at: s.last_activity_at,
        resume: s.resume,
        git: s.git,
        usage: s.usage.map(Into::into),
        transcript_size_bytes: s.transcript_size_bytes,
    })
}

impl From<proto::AgentSessionsResult> for AgentSessionsResult {
    fn from(r: proto::AgentSessionsResult) -> Self {
        Self {
            sessions: r
                .sessions
                .into_iter()
                .filter_map(agent_session_summary_v3)
                .collect(),
            total: r.total,
            error: r.error,
        }
    }
}

/// `None` drops events the old client can't decode: the `v4`-only
/// `TerminalAgentChanged`, and agent events for a filtered (newer) slug.
fn host_event_v3(e: proto::HostEvent) -> Option<HostEvent> {
    match e {
        proto::HostEvent::TerminalCreated { id, launch_cmd, .. } => {
            Some(HostEvent::TerminalCreated { id, launch_cmd })
        }
        proto::HostEvent::GitChanged => Some(HostEvent::GitChanged),
        proto::HostEvent::FsChanged { path } => Some(HostEvent::FsChanged { path }),
        proto::HostEvent::AgentInfoChanged { info } => {
            agent_summary_v3(info).map(|info| HostEvent::AgentInfoChanged { info })
        }
        proto::HostEvent::AgentHookReceived {
            agent_slug,
            event_name,
            payload,
        } => agent_kind(&agent_slug).map(|agent_kind| HostEvent::AgentHookReceived {
            agent_kind,
            event_name,
            payload,
        }),
        proto::HostEvent::AgentStateChanged {
            terminal_id,
            agent_session_id,
            state,
        } => Some(HostEvent::AgentStateChanged {
            terminal_id,
            agent_session_id,
            state,
        }),
        proto::HostEvent::TerminalAgentChanged { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// Lift a decoded `zedra/rpc/3` request into the live message, wrapping each
// response channel to re-encode to `zedra/rpc/3` bytes on send.
// ---------------------------------------------------------------------------

impl ZedraMessageV3 {
    pub fn into_live(self) -> proto::ZedraMessage {
        use proto::ZedraMessage as M;
        match self {
            // Byte-identical: same concrete channel types, rebuilt under M.
            ZedraMessageV3::Register(m) => M::Register((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::Authenticate(m) => M::Authenticate((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::Ping(m) => M::Ping((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::GetSessionInfo(m) => M::GetSessionInfo((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::ListSessions(m) => M::ListSessions((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::SwitchSession(m) => M::SwitchSession((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::FsList(m) => M::FsList((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::FsRead(m) => M::FsRead((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::FsWrite(m) => M::FsWrite((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::FsStat(m) => M::FsStat((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::TermCreate(m) => M::TermCreate((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::TermAttach(m) => M::TermAttach((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::TermResize(m) => M::TermResize((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::TermClose(m) => M::TermClose((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::TermList(m) => M::TermList((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::GitStatus(m) => M::GitStatus((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::GitDiff(m) => M::GitDiff((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::GitLog(m) => M::GitLog((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::GitCommit(m) => M::GitCommit((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::GitStage(m) => M::GitStage((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::GitUnstage(m) => M::GitUnstage((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::GitBranches(m) => M::GitBranches((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::GitCheckout(m) => M::GitCheckout((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::AiPrompt(m) => M::AiPrompt((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::LspDiagnostics(m) => M::LspDiagnostics((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::LspHover(m) => M::LspHover((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::FsWatch(m) => M::FsWatch((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::FsUnwatch(m) => M::FsUnwatch((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::SubscribeHostInfo(m) => {
                M::SubscribeHostInfo((m.inner, m.tx, m.rx).into())
            }
            ZedraMessageV3::TermReorder(m) => M::TermReorder((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::FsDocsTree(m) => M::FsDocsTree((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::AgentInstalledList(m) => {
                M::AgentInstalledList((m.inner, m.tx, m.rx).into())
            }
            ZedraMessageV3::TermCreateV2(m) => M::TermCreateV2((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::FsSearch(m) => M::FsSearch((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::SetAppState(m) => M::SetAppState((m.inner, m.tx, m.rx).into()),
            ZedraMessageV3::SetClientDeltaInfo(m) => {
                M::SetClientDeltaInfo((m.inner, m.tx, m.rx).into())
            }
            ZedraMessageV3::ClearClientDeltaInfo(m) => {
                M::ClearClientDeltaInfo((m.inner, m.tx, m.rx).into())
            }

            // Agent RPCs: lift the request kind to a slug. `AgentResume` and
            // `AgentFiles` results are wire-identical, so only their requests lift.
            ZedraMessageV3::AgentResume(m) => M::AgentResume((m.inner.into(), m.tx, m.rx).into()),
            ZedraMessageV3::AgentFiles(m) => M::AgentFiles((m.inner.into(), m.tx, m.rx).into()),
            ZedraMessageV3::AgentSessions(m) => M::AgentSessions(
                (
                    m.inner.into(),
                    m.tx.with_map(AgentSessionsResult::from),
                    m.rx,
                )
                    .into(),
            ),

            // Divergent responses: map the live result back to `zedra/rpc/3` bytes.
            ZedraMessageV3::Connect(m) => {
                M::Connect((m.inner, m.tx.with_map(ConnectResult::from), m.rx).into())
            }
            ZedraMessageV3::AuthProve(m) => {
                M::AuthProve((m.inner, m.tx.with_map(AuthProveResult::from), m.rx).into())
            }
            ZedraMessageV3::SyncSession(m) => {
                M::SyncSession((m.inner, m.tx.with_map(SyncSessionResult::from), m.rx).into())
            }
            ZedraMessageV3::AgentList(m) => {
                M::AgentList((m.inner, m.tx.with_map(AgentListResult::from), m.rx).into())
            }
            // Stream: drop events the old client cannot decode.
            ZedraMessageV3::Subscribe(m) => {
                M::Subscribe((m.inner, m.tx.with_filter_map(host_event_v3), m.rx).into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asserts `v4` bytes decode as the reused `v3` type — guards against a
    /// reused `proto` type drifting without an ALPN bump.
    fn assert_v4_decodes_as<V4, V3>(value: &V4)
    where
        V4: Serialize,
        V3: for<'de> Deserialize<'de>,
    {
        let bytes = postcard::to_stdvec(value).unwrap();
        postcard::from_bytes::<V3>(&bytes).expect("v4 bytes must decode as v3 type");
    }

    #[test]
    fn reused_results_are_wire_identical() {
        assert_v4_decodes_as::<_, proto::PongResult>(&proto::PongResult { timestamp_ms: 7 });
        assert_v4_decodes_as::<_, proto::FsWriteResult>(&proto::FsWriteResult { ok: true });
    }

    #[test]
    fn terminal_sync_entry_drops_v4_agent_slug() {
        let v4 = proto::TerminalSyncEntry {
            id: "t".into(),
            position: 0,
            last_seq: 1,
            title: None,
            cwd: None,
            icon_name: None,
            agent_command: Some("codex".into()),
            shell_state: proto::TermShellState::Idle,
            last_exit_code: Some(0),
            agent_state: proto::AgentState::Idle,
            agent_slug: Some("codex".into()),
        };
        let v3: TerminalSyncEntry = v4.into();
        let bytes = postcard::to_stdvec(&v3).unwrap();
        let _: TerminalSyncEntry = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(v3.agent_command.as_deref(), Some("codex"));
    }

    #[test]
    fn agent_summary_filters_unrepresentable_slug() {
        // A newer actor slug has no `v3` enum variant and must be dropped.
        assert!(agent_kind("amp").is_none());
        assert_eq!(agent_kind("hermes"), Some(AgentKind::Hermes));
    }
}
