#[cfg(unix)]
use serde::{Deserialize, Serialize};

/// Long-lived pane runtime transferred during server replacement.
///
/// Handoff preserves server-owned session state such as PTYs, processes, agent
/// identity, and durable plugin/session metadata. It intentionally does not
/// preserve transient coordination such as in-flight requests, waits,
/// subscriptions, client sockets, or pane-to-pane messages; clients reconnect
/// and retry those operations after replacement.
#[cfg(unix)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HandoffRuntimeState {
    pub pane_id: u32,
    pub child_pid: u32,
    pub rows: u16,
    pub cols: u16,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
    #[serde(default)]
    pub keyboard_protocol_flags: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_protocol_ansi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_state: Option<crate::pane::InputState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_history_ansi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_state: Option<crate::terminal::state::HandoffAgentState>,
    /// Acknowledged input stream state. Exported only after the PTY writer
    /// quiesced, so every admitted write is already on the PTY.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_streams: Option<Box<crate::terminal::input_streams::HandoffInputStreams>>,
}

#[cfg(unix)]
impl HandoffRuntimeState {
    pub fn with_pane_id(mut self, pane_id: crate::layout::PaneId) -> Self {
        self.pane_id = pane_id.raw();
        self
    }
}

#[derive(Debug)]
pub(crate) struct ImportedHandoffRuntime {
    /// Present only after explicit live identity admission. Never read from a
    /// cold snapshot or inferred from a runtime/PID.
    #[cfg(unix)]
    pub preserved_terminal_id: Option<crate::terminal::TerminalId>,
    #[cfg(unix)]
    pub observation_epoch: Option<String>,
    #[cfg(unix)]
    pub master_fd: std::os::fd::OwnedFd,
    #[cfg(unix)]
    pub state: HandoffRuntimeState,
}

#[cfg(unix)]
pub(crate) const SAVED_DIRECTORY_UNAVAILABLE: &str =
    "Saved directory is unavailable. Restore the directory and restart this session.";

/// Explicit capture of an existing unavailable pane, with no PTY or process.
#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UnavailableHandoffState {
    pub restore_error: String,
    pub cwd: std::path::PathBuf,
    pub label: Option<String>,
}

#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct ImportedHandoffPlaceholder {
    pub terminal_id: crate::terminal::TerminalId,
    pub observation_epoch: String,
    pub state: UnavailableHandoffState,
}

/// A continuing live runtime or a captured unavailable placeholder. Absence
/// is explicit and can never be inferred from a missing live-runtime entry.
#[derive(Debug)]
pub(crate) enum HandoffImport {
    Runtime(ImportedHandoffRuntime),
    #[cfg(unix)]
    Unavailable(ImportedHandoffPlaceholder),
}

impl HandoffImport {
    #[cfg(unix)]
    pub(crate) fn preserved_terminal_id(&self) -> Option<&crate::terminal::TerminalId> {
        match self {
            Self::Runtime(runtime) => runtime.preserved_terminal_id.as_ref(),
            Self::Unavailable(placeholder) => Some(&placeholder.terminal_id),
        }
    }

    #[cfg(unix)]
    pub(crate) fn observation_epoch(&self) -> Option<&str> {
        match self {
            Self::Runtime(runtime) => runtime.observation_epoch.as_deref(),
            Self::Unavailable(placeholder) => Some(&placeholder.observation_epoch),
        }
    }
}
