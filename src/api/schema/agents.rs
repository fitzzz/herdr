use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::common::{AgentStatus, ReadFormat, ReadSource};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentReadParams {
    pub target: String,
    pub source: ReadSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u32>,
    #[serde(default)]
    pub format: ReadFormat,
    #[serde(default = "super::common::default_true")]
    pub strip_ansi: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentSendKeysParams {
    pub target: String,
    pub keys: Vec<String>,
    /// The native agent session the caller expects the target to occupy: the
    /// `agent_session.value` that `agent.get` reports. When present, the server
    /// compares it with the occupant's current session in the same step that
    /// writes the keys and refuses with `agent_session_mismatch`, writing
    /// nothing, when they differ or no session is known. Advertised by
    /// `ServerCapabilities::agent_send_keys_expected_session`; older servers
    /// ignore the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_session: Option<String>,
    /// The launch the caller expects the target to run: the
    /// `launch_incarnation` its `agent.start` or `agent.start_with_launcher`
    /// reply returned. When present, the server compares it, byte for byte,
    /// with the target's current launch incarnation in the same step that
    /// checks `expected_session` and writes the keys, and refuses with
    /// `agent_launch_mismatch`, writing nothing, when they differ or no launch
    /// is attested. Advertised by
    /// `ServerCapabilities::agent_send_keys_expected_launch_incarnation`;
    /// older servers ignore the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_launch_incarnation: Option<String>,
}

/// Sent by the helper Herdr types for an admitted agent start, from the
/// process that start runs, before it executes the agent. Not for clients:
/// the nonce is spent by its first use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentAttestLaunchParams {
    pub nonce: String,
    /// Sent instead of a claim when the pane's shell resolved the started
    /// command itself, through an alias, a function or a builtin: the start
    /// runs unattested and the nonce is spent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resolved_by_shell: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentWaitParams {
    pub target: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub until: Vec<AgentStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentPromptWaitOptions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub until: Vec<AgentStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) submission_deadline: Option<std::time::Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentRenameParams {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentViewSetParams {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<AgentViewFilter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<AgentViewSort>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct AgentViewClearParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AgentViewFilter {
    All {
        filters: Vec<AgentViewFilter>,
    },
    Any {
        filters: Vec<AgentViewFilter>,
    },
    Not {
        filter: Box<AgentViewFilter>,
    },
    Eq {
        field: AgentViewField,
        value: AgentViewValue,
    },
    In {
        field: AgentViewField,
        values: Vec<AgentViewValue>,
    },
    Exists {
        field: AgentViewField,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum AgentViewField {
    Builtin(AgentViewBuiltinField),
    Token { token: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentViewBuiltinField {
    Status,
    WorkspaceId,
    TabId,
    PaneId,
    Agent,
    Seen,
    StateChangeSeq,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum AgentViewValue {
    String(String),
    Bool(bool),
    Number(u64),
    Context { context: AgentViewContext },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentViewContext {
    CurrentWorkspaceId,
    CurrentTabId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentViewSort {
    pub field: AgentViewSortField,
    #[serde(default)]
    pub order: AgentViewSortOrder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum AgentViewSortField {
    Builtin(AgentViewBuiltinSortField),
    Token { token: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentViewBuiltinSortField {
    WorkspaceOrder,
    TabOrder,
    PaneOrder,
    Attention,
    Status,
    Agent,
    Seen,
    StateChangeSeq,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentViewSortOrder {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentStartParams {
    pub name: String,
    pub kind: String,
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Startup timeout in milliseconds. Values must be greater than 3000 and at most 300000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// A process-scoped launcher. Native agent arguments and identity stay separate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentLauncher {
    pub executable: String,
    pub prefix_args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
}

/// Separately advertised so older servers cannot silently ignore the launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentStartWithLauncherParams {
    pub name: String,
    pub kind: String,
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    pub launcher: AgentLauncher,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentPromptParams {
    pub target: String,
    pub text: String,
    /// The launch the caller expects the target to run: the
    /// `launch_incarnation` its `agent.start` or `agent.start_with_launcher`
    /// reply returned. When present, the server compares it, byte for byte,
    /// with the target's current launch incarnation in the same step that
    /// queues the prompt, and refuses with `agent_launch_mismatch`, typing
    /// nothing, when they differ or no launch is attested. The terminal writer
    /// proves the launch's process again before it writes the text and before
    /// it writes Enter; a check that fails after some of the text was written
    /// answers `agent_prompt_interrupted`, and Enter is not sent. Advertised by
    /// `ServerCapabilities::agent_prompt_expected_launch_incarnation`; older
    /// servers ignore the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_launch_incarnation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<AgentPromptWaitOptions>,
}

/// Asks what this server knows of one admitted launch, by its token alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentLaunchStatusParams {
    pub launch_incarnation: String,
}

/// What a server knows of an admitted launch, by its token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentLaunchState {
    /// Its process is proved now: the terminal's agent process, leading the
    /// foreground, read once for the reply's `agent`, which carries the token.
    /// A terminal no pane shows is `held`.
    Attested,
    /// A terminal keeps the launch and it is not proved now: not yet claimed
    /// or detected, carried by a live handoff the next server has not attested
    /// yet, or its process does not lead the foreground. It may be running.
    Held,
    /// A cold restore found the launch's process gone: the last session save
    /// recorded the launch bound to a process, and at the restore the host had
    /// booted again or no process had that id and start time. The reply names
    /// the terminal restored in its pane, when (`restored_at`) and the
    /// `evidence`. Nothing else, an observed exit or a revoked token included,
    /// gives this answer.
    EndedAtRestore,
    /// No terminal keeps it and no restore found it gone: its process exited,
    /// its pane closed or respawned, another process or agent kind took the
    /// terminal, a new start was admitted there, or this server never had it
    /// (a restart or restore does not carry a launch; a live handoff does).
    /// This is not evidence that its process stopped. A token is never
    /// reused, and one in no terminal never comes back.
    None,
}

/// What showed a cold restore that a launch's process was gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LaunchEndEvidence {
    /// The host booted again since the save that recorded the launch.
    BootChanged,
    /// In the same boot, no process had the launch's process id and start time.
    ProcessAbsent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentInfo {
    pub terminal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_title_stripped: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_agent: Option<String>,
    pub agent_status: AgentStatus,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub screen_detection_skipped: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub state_labels: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    #[schemars(schema_with = "super::common::metadata_token_values_schema")]
    pub tokens: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<AgentSessionInfo>,
    /// The admitted `agent.start` launch this agent is: present only while
    /// the process that launch started, identified by its process id and
    /// start time, is the terminal's agent process. Absent before Herdr has
    /// seen that process, and for good once it exits, the pane respawns,
    /// another process takes the terminal or a new start is admitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_incarnation: Option<String>,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub focused: bool,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub launch_pending: bool,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub interactive_ready: bool,
    #[serde(default)]
    pub state_change_seq: u64,
    /// Sequence comparisons are valid only within the same observation epoch.
    /// Absent on ordinary terminals; present after an admitted legacy handoff
    /// that preserved terminal identity without old completion watermarks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_change_epoch: Option<String>,
    /// The current idle transition completed work, independently of who has viewed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_cwd: Option<String>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentSessionInfo {
    pub source: String,
    pub agent: String,
    pub kind: crate::agent_resume::AgentSessionRefKind,
    pub value: String,
}
