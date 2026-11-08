use serde::{Deserialize, Serialize};

use super::agents::AgentInfo;
use super::common::{ClientWindowTitleReason, NotificationShowReason};
use super::events::EventEnvelope;
use super::integrations::{
    IntegrationInstallResult, IntegrationTarget, IntegrationUninstallResult,
};
use super::panes::{
    LayoutDescription, PaneEdgesResult, PaneFocusDirectionResult, PaneInfo, PaneLayoutSnapshot,
    PaneMoveResult, PaneNeighborResult, PaneProcessInfo, PaneReadResult, PaneResizeResult,
    PaneSwapResult, PaneTextPoint, PaneTextRange, PaneZoomResult,
};
use super::plugins::{
    InstalledPluginInfo, PluginActionInfo, PluginCommandLogInfo, PluginInvocationContext,
    PluginPaneInfo,
};
use super::server::ServerCapabilities;
use super::session::SessionSnapshot;
use super::tabs::TabInfo;
use super::workspaces::WorkspaceInfo;
use super::worktrees::{WorktreeInfo, WorktreeSourceInfo};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SuccessResponse {
    pub id: String,
    pub result: ResponseResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ErrorResponse {
    pub id: String,
    pub error: ErrorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

/// How `agent.prompt` observed acceptance when the submission alone did not show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentPromptEvidence {
    /// The prompt sat unread in the agent's terminal input queue until Herdr
    /// wrote one focus-in report; the agent then read it and started its turn.
    ReleasedAfterWakeup,
    /// Evidence named by a newer server. The prompt was still accepted.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseResult {
    Pong {
        version: String,
        protocol: u32,
        #[serde(default)]
        capabilities: Option<ServerCapabilities>,
    },
    SessionSnapshot {
        snapshot: Box<SessionSnapshot>,
    },
    WorkspaceInfo {
        workspace: WorkspaceInfo,
    },
    WorkspaceCreated {
        workspace: WorkspaceInfo,
        tab: TabInfo,
        root_pane: PaneInfo,
    },
    WorkspaceList {
        workspaces: Vec<WorkspaceInfo>,
    },
    WorktreeList {
        source: WorktreeSourceInfo,
        worktrees: Vec<WorktreeInfo>,
    },
    WorktreeCreated {
        workspace: WorkspaceInfo,
        tab: TabInfo,
        root_pane: PaneInfo,
        worktree: WorktreeInfo,
    },
    WorktreeOpened {
        workspace: WorkspaceInfo,
        tab: TabInfo,
        root_pane: PaneInfo,
        worktree: WorktreeInfo,
        already_open: bool,
    },
    WorktreeRemoved {
        workspace_id: String,
        path: String,
        forced: bool,
    },
    TabInfo {
        tab: TabInfo,
    },
    TabCreated {
        tab: TabInfo,
        root_pane: PaneInfo,
    },
    TabList {
        tabs: Vec<TabInfo>,
    },
    AgentInfo {
        agent: AgentInfo,
    },
    AgentStarted {
        agent: AgentInfo,
        argv: Vec<String>,
        /// The token of this admission. Record it now: `agent.launch_incarnation`
        /// reports it only once Herdr has seen the launch's own process, and
        /// never again after that process is gone. Absent where Herdr cannot
        /// attest a launch (Windows).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        launch_incarnation: Option<String>,
    },
    AgentStartedWithLauncher {
        agent: AgentInfo,
        argv: Vec<String>,
        launcher: super::agents::AgentLauncher,
        /// As for `AgentStarted`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        launch_incarnation: Option<String>,
    },
    AgentLaunchStatus {
        launch_incarnation: String,
        state: super::agents::AgentLaunchState,
        /// The terminal keeping the launch; absent when `state` is `none`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        terminal_id: Option<String>,
        /// The pane showing that terminal now; a move changes it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<String>,
        /// The agent in that pane. It carries the token only when `state` is
        /// `attested`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentInfo>,
        /// When the restore that found the launch gone ran, in milliseconds
        /// since the Unix epoch; only for `ended_at_restore`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        restored_at: Option<u64>,
        /// What showed that restore the launch's process was gone; only for
        /// `ended_at_restore`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evidence: Option<super::agents::LaunchEndEvidence>,
    },
    /// The helper's claim bound the admission to its process: the token that
    /// process is the launch of.
    AgentLaunchAttested {
        launch_incarnation: String,
    },
    AgentLauncherInfo {
        agent: AgentInfo,
        binding: Option<crate::agent_launcher::BoundAgentLauncher>,
        confirmed: bool,
    },
    AgentPrompted {
        agent: AgentInfo,
        /// Present only when acceptance needed more than the submission itself.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evidence: Option<AgentPromptEvidence>,
    },
    AgentList {
        agents: Vec<AgentInfo>,
    },
    AgentView {
        active: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    PaneInfo {
        pane: PaneInfo,
    },
    PaneList {
        panes: Vec<PaneInfo>,
    },
    PaneCurrent {
        pane: PaneInfo,
    },
    PaneSwap {
        swap: PaneSwapResult,
    },
    PaneMove {
        move_result: PaneMoveResult,
    },
    PaneZoom {
        zoom: PaneZoomResult,
    },
    PaneLayout {
        layout: PaneLayoutSnapshot,
    },
    PaneProcessInfo {
        process_info: PaneProcessInfo,
    },
    LayoutExport {
        layout: LayoutDescription,
    },
    LayoutApply {
        layout: LayoutDescription,
    },
    LayoutSplitRatioSet {
        layout: LayoutDescription,
    },
    PaneNeighbor {
        neighbor: PaneNeighborResult,
    },
    PaneEdges {
        edges: PaneEdgesResult,
    },
    PaneFocusDirection {
        focus: PaneFocusDirectionResult,
    },
    PaneResize {
        resize: PaneResizeResult,
    },
    PaneRead {
        read: PaneReadResult,
    },
    TerminalInputStream {
        stream: super::TerminalInputStreamInfo,
    },
    TerminalInputWritten {
        write: super::TerminalInputWriteAck,
    },
    PaneSelection {
        pane_id: String,
        text: String,
    },
    PaneCopyMotion {
        pane_id: String,
        cursor: PaneTextPoint,
        content_revision: u64,
    },
    PaneCopySearch {
        pane_id: String,
        content_revision: u64,
        matches: Vec<PaneTextRange>,
        total: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_global: Option<u64>,
    },
    AgentExplain {
        explain: serde_json::Value,
    },
    SubscriptionStarted {},
    WaitMatched {
        event: EventEnvelope,
    },
    OutputMatched {
        pane_id: String,
        revision: u64,
        matched_line: Option<String>,
        read: PaneReadResult,
    },
    NotificationShow {
        shown: bool,
        reason: NotificationShowReason,
    },
    ClientWindowTitle {
        changed: bool,
        reason: ClientWindowTitleReason,
    },
    IntegrationList {
        integrations: Vec<super::integrations::IntegrationInfo>,
    },
    IntegrationInstall {
        target: IntegrationTarget,
        details: IntegrationInstallResult,
    },
    IntegrationUninstall {
        target: IntegrationTarget,
        details: IntegrationUninstallResult,
    },
    AgentManifestReload {
        manifests: Vec<AgentManifestInfo>,
    },
    AgentManifestStatus {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_check_unix: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_result: Option<String>,
        manifests: Vec<AgentManifestInfo>,
    },
    PluginLinked {
        plugin: InstalledPluginInfo,
    },
    PluginList {
        plugins: Vec<InstalledPluginInfo>,
    },
    PluginUnlinked {
        plugin_id: String,
        removed: bool,
    },
    PluginEnabled {
        plugin: InstalledPluginInfo,
    },
    PluginDisabled {
        plugin: InstalledPluginInfo,
    },
    PluginActionList {
        actions: Vec<PluginActionInfo>,
    },
    PluginActionInvoked {
        action: PluginActionInfo,
        context: PluginInvocationContext,
        log: PluginCommandLogInfo,
    },
    PaneLinkResolved {
        regions: Vec<super::panes::PaneLinkRegion>,
    },
    PaneLinkActivated {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        handled: bool,
    },
    PluginLogList {
        logs: Vec<PluginCommandLogInfo>,
    },
    PluginPaneOpened {
        plugin_pane: PluginPaneInfo,
    },
    PluginPaneFocused {
        plugin_pane: PluginPaneInfo,
    },
    PluginPaneClosed {
        pane_id: String,
    },
    ConfigReload {
        status: crate::config::ConfigReloadStatus,
        diagnostics: Vec<String>,
    },
    /// Acknowledgement for the client-shell surface interest lease. This method is new on the
    /// endpoint protocol, so its revision-bearing result can establish an activation floor.
    ClientShellSurfaceSet {
        active: bool,
        projection_revision: u64,
    },
    Ok {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentManifestInfo {
    pub agent: String,
    pub source: String,
    pub source_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_remote_version: Option<String>,
    pub local_override_shadowing_remote: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_update_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_update_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_last_checked_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}
