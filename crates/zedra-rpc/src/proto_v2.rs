// Frozen `zedra/rpc/2` wire schema, for clients built before the `zedra/rpc/3`
// ALPN bump. The host advertises both ALPNs and serves a `zedra/rpc/2` client by
// decoding requests with `ZedraProtoV2`, lifting them into `proto::ZedraMessage`
// via `into_live`, and reusing the live dispatch; the only seam is the per-RPC
// channel wrap that re-encodes responses to the `zedra/rpc/2` shape.
//
// Wire contract for shipped binaries — do not edit types or variant order. Only
// types that diverged at `zedra/rpc/3` are defined here; the rest reuse `proto`
// (pinned by the roundtrip tests below). A future wire change must bump the ALPN
// (§2.4) and re-freeze the reused types.

use chrono::{DateTime, Utc};
use irpc::channel::{mpsc, oneshot};
use irpc::rpc_requests;
use serde::{Deserialize, Serialize};

use crate::proto;

/// Previous ALPN, advertised alongside `ZEDRA_ALPN`.
pub const ZEDRA_ALPN_V2: &[u8] = b"zedra/rpc/2";

// ---------------------------------------------------------------------------
// Protocol enum (frozen at the final `zedra/rpc/2` variant set and order)
// ---------------------------------------------------------------------------

#[rpc_requests(message = ZedraMessageV2)]
#[derive(Serialize, Deserialize, Debug)]
pub enum ZedraProtoV2 {
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
}

// ---------------------------------------------------------------------------
// Divergent types — frozen at their final `zedra/rpc/2` wire shape
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
}

/// Lowest common denominator across shipped `zedra/rpc/2` builds (some predate
/// `Pi`). Only these three are emitted/accepted; `Pi`/`Hermes` are filtered out.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ManagedAgentKind {
    Claude,
    Codex,
    OpenCode,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentLiveSummary {
    pub active_terminal_ids: Vec<String>,
    pub pending_action_count: usize,
    pub latest_event: Option<AgentEventSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEventSummary {
    pub kind: AgentEventKind,
    pub status: AgentLifecycleStatus,
    pub at: Option<DateTime<Utc>>,
    pub terminal_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AgentEventKind {
    SessionStarted,
    SessionUpdated,
    TurnStarted,
    TurnCompleted,
    TurnFailed,
    TaskCreated,
    TaskCompleted,
    TaskFailed,
    ToolStarted,
    ToolCompleted,
    ToolFailed,
    PermissionRequested,
    PermissionResolved,
    Notification,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub enum AgentLifecycleStatus {
    #[default]
    Unknown,
    Starting,
    Running,
    WaitingForUser,
    WaitingForPermission,
    Idle,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    pub kind: ManagedAgentKind,
    pub display_name: String,
    pub cli: proto::AgentCliSummary,
    pub setup: proto::AgentSetupSummary,
    pub capabilities: proto::AgentCapabilities,
    pub workspace: proto::AgentWorkspaceSummary,
    pub sessions: AgentSessionCounts,
    pub live: AgentLiveSummary,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub data_sources: Vec<proto::AgentDataSource>,
    pub warnings: Vec<proto::AgentWarning>,
    pub account: proto::AgentAccountSummary,
    pub usage: Option<proto::AgentUsageSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentListResult {
    pub agents: Vec<AgentSummary>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionCounts {
    pub total: usize,
    pub resumable: usize,
    pub active_live: usize,
    pub latest_session_id: Option<String>,
    pub latest_session_title: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentSessionLiveSummary {
    pub terminal_id: Option<String>,
    pub status: AgentLifecycleStatus,
    pub pending_action_count: usize,
    pub current_turn_id: Option<String>,
    pub latest_event: Option<AgentEventSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentProviderSessionInfo {
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub cli_version: Option<String>,
    pub origin: Option<String>,
    pub source: Option<String>,
    pub entrypoint: Option<String>,
    pub native_project_id: Option<String>,
    pub model_provider: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentSessionCounters {
    pub record_count: u64,
    pub message_count: u64,
    pub turn_count: u64,
    pub tool_count: u64,
    pub tool_failure_count: u64,
    pub hook_success_count: u64,
    pub hook_failure_count: u64,
    pub malformed_record_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentSessionFlags {
    pub is_sidechain: bool,
    pub is_subagent: bool,
    pub is_archived: bool,
    pub historical_only: bool,
    pub live_bound: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionSummary {
    pub kind: ManagedAgentKind,
    pub session_id: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub resume: proto::AgentResumeSummary,
    pub live: AgentSessionLiveSummary,
    pub provider: AgentProviderSessionInfo,
    pub git: Option<proto::AgentGitSummary>,
    pub usage: Option<proto::AgentUsageSnapshot>,
    pub counters: AgentSessionCounters,
    pub flags: AgentSessionFlags,
    pub data_sources: Vec<proto::AgentDataSource>,
    pub warnings: Vec<proto::AgentWarning>,
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
    pub kind: ManagedAgentKind,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResumeReq {
    pub kind: ManagedAgentKind,
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

// ---------------------------------------------------------------------------
// Live (v3) -> `zedra/rpc/2` response conversions
// ---------------------------------------------------------------------------

impl From<proto::TerminalSyncEntry> for TerminalSyncEntry {
    fn from(t: proto::TerminalSyncEntry) -> Self {
        // Drops the fields appended at `zedra/rpc/3`: agent_command, shell_state,
        // last_exit_code, agent_state.
        Self {
            id: t.id,
            position: t.position,
            last_seq: t.last_seq,
            title: t.title,
            cwd: t.cwd,
            icon_name: t.icon_name,
        }
    }
}

impl From<proto::SyncSessionResult> for SyncSessionResult {
    fn from(s: proto::SyncSessionResult) -> Self {
        // Drops delta_pubkey (added at `zedra/rpc/3`).
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

/// `Pi`/`Hermes` have no safe `v2` representation; they yield `None` and callers
/// drop those agents/sessions.
fn managed_agent_kind(kind: proto::AgentKind) -> Option<ManagedAgentKind> {
    match kind {
        proto::AgentKind::Claude => Some(ManagedAgentKind::Claude),
        proto::AgentKind::Codex => Some(ManagedAgentKind::Codex),
        proto::AgentKind::OpenCode => Some(ManagedAgentKind::OpenCode),
        proto::AgentKind::Pi | proto::AgentKind::Hermes | proto::AgentKind::Maki => None,
    }
}

/// Maps a `v2` agent kind up to the live kind (always representable).
fn agent_kind_to_v3(kind: ManagedAgentKind) -> proto::AgentKind {
    match kind {
        ManagedAgentKind::Claude => proto::AgentKind::Claude,
        ManagedAgentKind::Codex => proto::AgentKind::Codex,
        ManagedAgentKind::OpenCode => proto::AgentKind::OpenCode,
    }
}

impl From<proto::AgentSessionCounts> for AgentSessionCounts {
    fn from(c: proto::AgentSessionCounts) -> Self {
        // `active_live` was removed at `zedra/rpc/3`; synthesize 0.
        Self {
            total: c.total,
            resumable: c.resumable,
            active_live: 0,
            latest_session_id: c.latest_session_id,
            latest_session_title: c.latest_session_title,
        }
    }
}

// Request lift: `v2` client -> live handler. Only `kind` widened.
impl From<AgentSessionsReq> for proto::AgentSessionsReq {
    fn from(r: AgentSessionsReq) -> Self {
        Self {
            kind: agent_kind_to_v3(r.kind),
            refresh: r.refresh,
            limit: r.limit,
        }
    }
}

impl From<AgentResumeReq> for proto::AgentResumeReq {
    fn from(r: AgentResumeReq) -> Self {
        Self {
            kind: agent_kind_to_v3(r.kind),
            session_id: r.session_id,
            cols: r.cols,
            rows: r.rows,
        }
    }
}

/// `None` for kinds absent in `v2`. Fields removed at `zedra/rpc/3` (live,
/// provider, counters, flags, data_sources, warnings) get empty defaults.
fn agent_session_summary_v2(s: proto::AgentSessionSummary) -> Option<AgentSessionSummary> {
    Some(AgentSessionSummary {
        kind: managed_agent_kind(s.kind)?,
        session_id: s.session_id,
        title: s.title,
        cwd: s.cwd,
        created_at: s.created_at,
        last_activity_at: s.last_activity_at,
        resume: s.resume,
        live: AgentSessionLiveSummary::default(),
        provider: AgentProviderSessionInfo::default(),
        git: s.git,
        usage: s.usage,
        counters: AgentSessionCounters::default(),
        flags: AgentSessionFlags::default(),
        data_sources: Vec::new(),
        warnings: Vec::new(),
        transcript_size_bytes: s.transcript_size_bytes,
    })
}

impl From<proto::AgentSessionsResult> for AgentSessionsResult {
    fn from(r: proto::AgentSessionsResult) -> Self {
        Self {
            sessions: r
                .sessions
                .into_iter()
                .filter_map(agent_session_summary_v2)
                .collect(),
            total: r.total,
            error: r.error,
        }
    }
}

/// `None` for kinds absent in `v2`. The `live` subtree, removed at `zedra/rpc/3`,
/// is sent empty.
fn agent_summary_v2(a: proto::AgentSummary) -> Option<AgentSummary> {
    Some(AgentSummary {
        kind: managed_agent_kind(a.kind)?,
        display_name: a.display_name,
        cli: a.cli,
        setup: a.setup,
        capabilities: a.capabilities,
        workspace: a.workspace,
        sessions: a.sessions.into(),
        live: AgentLiveSummary::default(),
        last_activity_at: a.last_activity_at,
        updated_at: a.updated_at,
        data_sources: a.data_sources,
        warnings: a.warnings,
        account: a.account,
        usage: a.usage,
    })
}

impl From<proto::AgentListResult> for AgentListResult {
    fn from(r: proto::AgentListResult) -> Self {
        Self {
            agents: r.agents.into_iter().filter_map(agent_summary_v2).collect(),
            error: r.error,
        }
    }
}

/// `None` drops events the old client can't decode: the `v3` variants, and
/// `AgentInfoChanged` for a filtered agent kind.
fn host_event_v2(e: proto::HostEvent) -> Option<HostEvent> {
    match e {
        proto::HostEvent::TerminalCreated { id, launch_cmd } => {
            Some(HostEvent::TerminalCreated { id, launch_cmd })
        }
        proto::HostEvent::GitChanged => Some(HostEvent::GitChanged),
        proto::HostEvent::FsChanged { path } => Some(HostEvent::FsChanged { path }),
        proto::HostEvent::AgentInfoChanged { info } => {
            agent_summary_v2(info).map(|info| HostEvent::AgentInfoChanged { info })
        }
        proto::HostEvent::AgentHookReceived { .. } | proto::HostEvent::AgentStateChanged { .. } => {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Lift a decoded `zedra/rpc/2` request into the live message, wrapping each
// response channel to re-encode to `zedra/rpc/2` bytes on send.
// ---------------------------------------------------------------------------

impl ZedraMessageV2 {
    pub fn into_live(self) -> proto::ZedraMessage {
        use proto::ZedraMessage as M;
        match self {
            // Byte-identical: same concrete channel types, rebuilt under M.
            ZedraMessageV2::Register(m) => M::Register((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::Authenticate(m) => M::Authenticate((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::Ping(m) => M::Ping((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::GetSessionInfo(m) => M::GetSessionInfo((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::ListSessions(m) => M::ListSessions((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::SwitchSession(m) => M::SwitchSession((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::FsList(m) => M::FsList((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::FsRead(m) => M::FsRead((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::FsWrite(m) => M::FsWrite((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::FsStat(m) => M::FsStat((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::TermCreate(m) => M::TermCreate((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::TermAttach(m) => M::TermAttach((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::TermResize(m) => M::TermResize((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::TermClose(m) => M::TermClose((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::TermList(m) => M::TermList((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::GitStatus(m) => M::GitStatus((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::GitDiff(m) => M::GitDiff((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::GitLog(m) => M::GitLog((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::GitCommit(m) => M::GitCommit((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::GitStage(m) => M::GitStage((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::GitUnstage(m) => M::GitUnstage((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::GitBranches(m) => M::GitBranches((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::GitCheckout(m) => M::GitCheckout((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::AiPrompt(m) => M::AiPrompt((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::LspDiagnostics(m) => M::LspDiagnostics((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::LspHover(m) => M::LspHover((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::FsWatch(m) => M::FsWatch((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::FsUnwatch(m) => M::FsUnwatch((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::SubscribeHostInfo(m) => {
                M::SubscribeHostInfo((m.inner, m.tx, m.rx).into())
            }
            ZedraMessageV2::TermReorder(m) => M::TermReorder((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::FsDocsTree(m) => M::FsDocsTree((m.inner, m.tx, m.rx).into()),
            ZedraMessageV2::AgentInstalledList(m) => {
                M::AgentInstalledList((m.inner, m.tx, m.rx).into())
            }
            ZedraMessageV2::TermCreateV2(m) => M::TermCreateV2((m.inner, m.tx, m.rx).into()),

            // Agent RPCs: lift the request kind and map the result back. The
            // `AgentResume` result is wire-identical, so only its request lifts.
            ZedraMessageV2::AgentResume(m) => M::AgentResume((m.inner.into(), m.tx, m.rx).into()),
            ZedraMessageV2::AgentSessions(m) => M::AgentSessions(
                (
                    m.inner.into(),
                    m.tx.with_map(AgentSessionsResult::from),
                    m.rx,
                )
                    .into(),
            ),

            // Divergent responses: map the live result back to `zedra/rpc/2` bytes.
            ZedraMessageV2::Connect(m) => {
                M::Connect((m.inner, m.tx.with_map(ConnectResult::from), m.rx).into())
            }
            ZedraMessageV2::AuthProve(m) => {
                M::AuthProve((m.inner, m.tx.with_map(AuthProveResult::from), m.rx).into())
            }
            ZedraMessageV2::SyncSession(m) => {
                M::SyncSession((m.inner, m.tx.with_map(SyncSessionResult::from), m.rx).into())
            }
            ZedraMessageV2::AgentList(m) => {
                M::AgentList((m.inner, m.tx.with_map(AgentListResult::from), m.rx).into())
            }
            // Stream: drop events the old client cannot decode.
            ZedraMessageV2::Subscribe(m) => {
                M::Subscribe((m.inner, m.tx.with_filter_map(host_event_v2), m.rx).into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asserts `v3` bytes decode as the reused `v2` type — guards against a
    /// reused `proto` type drifting without an ALPN bump.
    fn assert_v3_decodes_as<V3, V2>(value: &V3)
    where
        V3: Serialize,
        V2: for<'de> Deserialize<'de>,
    {
        let bytes = postcard::to_stdvec(value).unwrap();
        postcard::from_bytes::<V2>(&bytes).expect("v3 bytes must decode as v2 type");
    }

    #[test]
    fn reused_results_are_wire_identical() {
        assert_v3_decodes_as::<_, proto::PongResult>(&proto::PongResult { timestamp_ms: 7 });
        assert_v3_decodes_as::<_, proto::FsWriteResult>(&proto::FsWriteResult { ok: true });
    }

    #[test]
    fn divergent_sync_session_drops_v3_fields() {
        let v3 = proto::SyncSessionResult {
            session_id: "s".into(),
            session_token: [1u8; 32],
            hostname: "h".into(),
            workdir: "/w".into(),
            username: "u".into(),
            home_dir: None,
            os: None,
            arch: None,
            os_version: None,
            host_version: None,
            delta_pubkey: [2u8; 32],
            terminals: Vec::new(),
        };
        let v2: SyncSessionResult = v3.into();
        // Re-encode under v2 and ensure it is self-consistent.
        let bytes = postcard::to_stdvec(&v2).unwrap();
        let _: SyncSessionResult = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(v2.session_token, [1u8; 32]);
    }
}
