use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct PingParams {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ServerLiveHandoffParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_exe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_protocol: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ServerSshAgentRegisterParams {
    /// Absolute remote-host agent socket. Registration lasts until this API connection closes.
    pub socket_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ServerCapabilities {
    /// Optional JSON API methods implemented by this server, not the client binary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
    /// Managed launcher start and persisted recovery are both supported.
    #[serde(default)]
    pub agent_launcher: bool,
    pub live_handoff: bool,
    #[serde(default)]
    pub detached_server_daemon: bool,
    /// Stable client-owned endpoint generation supported by this server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_protocol_generation: Option<u32>,
    /// Whether this server supports explicit client-shell surface interest.
    #[serde(default)]
    pub surface_interest: bool,
    /// Whether this server supports endpoint health probes.
    #[serde(default)]
    pub health_check: bool,
    /// Supports connection-scoped `server.ssh_agent.register` on the local JSON API.
    #[serde(default)]
    pub ssh_agent_registration: bool,
    /// Supports `terminal.input.open`, `terminal.input.write` and
    /// `terminal.input.reconcile`.
    #[serde(default)]
    pub terminal_input_ack: bool,
    /// Supports byte-preserving UTF-8 `terminal.input.write_literal` on the
    /// same acknowledged streams as `terminal.input.write`.
    #[serde(default)]
    pub terminal_input_literal: bool,
    /// `agent.prompt` reports a truthful submission outcome: it refuses a busy
    /// composer with `agent_not_ready`, answers `agent_prompt_stalled` when
    /// acceptance evidence is missing, and answers `agent_prompt_not_submitted`
    /// when the text stayed in the composer or was rejected. A success answer
    /// means acceptance was observed.
    #[serde(default)]
    pub agent_prompt_outcome: bool,
    /// Agent kinds whose restore repeats the options the agent was launched
    /// with, read from its command line without what the prefix of the
    /// launcher it was started through passed on. Absent on servers that do
    /// not repeat them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub launch_option_replay: Vec<String>,
}
