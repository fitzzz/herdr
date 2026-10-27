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
    /// Absent from runtimes written before this field existed, and when no
    /// process was bound to the pane's launch yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_launch: Option<crate::terminal::state::HandoffAgentLaunch>,
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
// One import is made per pane, once per handoff, so the size difference
// between its variants costs nothing worth a box.
#[allow(clippy::large_enum_variant)]
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

#[cfg(all(test, unix))]
mod tests {
    use super::HandoffRuntimeState;
    use crate::terminal::state::HandoffAgentLaunch;

    #[test]
    fn a_runtime_state_written_before_the_agent_launch_field_still_loads() {
        let older = serde_json::json!({
            "pane_id": 1,
            "child_pid": 2,
            "rows": 24,
            "cols": 80,
            "cell_width_px": 0,
            "cell_height_px": 0,
        });
        let state: HandoffRuntimeState =
            serde_json::from_value(older).expect("an older runtime state should still load");
        assert_eq!(state.agent_launch, None);

        let confirmed = Some(HandoffAgentLaunch::Confirmed {
            process_id: 4242,
            process_started: Some(1234),
        });
        let state = HandoffRuntimeState {
            agent_launch: confirmed,
            ..state
        };
        let loaded: HandoffRuntimeState =
            serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        assert_eq!(loaded.agent_launch, confirmed);

        // A launch written before the start time was carried loads with none.
        let older_launch = serde_json::json!({"assumed": {"process_id": 4242}});
        assert_eq!(
            serde_json::from_value::<HandoffAgentLaunch>(older_launch).unwrap(),
            HandoffAgentLaunch::Assumed {
                process_id: 4242,
                process_started: None,
            }
        );
        // A launch with no start time read is written as before.
        assert_eq!(
            serde_json::to_value(HandoffAgentLaunch::Confirmed {
                process_id: 4242,
                process_started: None,
            })
            .unwrap(),
            serde_json::json!({"confirmed": {"process_id": 4242}})
        );
    }
}
