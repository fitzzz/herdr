//! Release a prompt that Codex never read.
//!
//! Codex's terminal input reader (crossterm with edge-triggered epoll) can miss
//! the readiness wake-up for a write. The prompt then sits unread in the PTY's
//! kernel input queue until some later write arrives. Live captures held the
//! whole prompt there for 15 s to over 60 s while every Codex thread slept, and
//! a resend released it, which delivered the prompt twice. One focus-in report
//! is a later write that Codex parses as an inert focus event, so it releases
//! the queued prompt without a duplicate.

use crate::api::schema::AgentTarget;
use crate::api::{AgentInputWakeup, AgentInputWakeupProjectionResponse};
use crate::app::App;
use bytes::Bytes;

use super::responses::encode_error;

/// Nudge only bytes that are provably still queued, and only with a report the
/// program asked for: with focus reporting off, `\e[I` would be ordinary input.
pub(super) fn should_send_wakeup(unread_input_bytes: Option<usize>, focus_reporting: bool) -> bool {
    unread_input_bytes.is_some_and(|bytes| bytes > 0) && focus_reporting
}

impl App {
    /// Measure unread input for the prompted Codex and, when asked and when
    /// `should_send_wakeup` allows it, write exactly one focus-in report. Text,
    /// Enter and Esc are never written here. The caller makes at most one
    /// `nudge` request per prompt.
    pub(super) fn handle_agent_input_wakeup(
        &mut self,
        id: String,
        target: AgentTarget,
        expected_terminal_id: &str,
        nudge: bool,
    ) -> String {
        let not_eligible = |id: String, reason: &str| {
            encode_error(id, "agent_wakeup_not_eligible", reason.to_string())
        };
        let Ok(resolved) = self.resolve_agent_target(&target.target) else {
            return not_eligible(id, "agent not found");
        };
        if resolved.terminal_id != expected_terminal_id {
            return not_eligible(id, "the target is no longer the prompted terminal");
        }
        let Some(terminal) = self.state.terminals.get(resolved.terminal_id.as_str()) else {
            return not_eligible(id, "agent not found");
        };
        if terminal.effective_known_agent() != Some(crate::detect::Agent::Codex) {
            return not_eligible(id, "the agent is not Codex");
        }
        if terminal.state != crate::detect::AgentState::Idle {
            return not_eligible(id, "the agent is not idle");
        }
        let Some(runtime) = self.lookup_runtime_sender(resolved.ws_idx, resolved.pane_id) else {
            return not_eligible(id, "agent not found");
        };
        if !super::super::agents::runtime_hosts_agent(runtime, crate::detect::Agent::Codex) {
            return not_eligible(id, "Codex is no longer the pane foreground process");
        }

        let unread_input_bytes = runtime.unread_input_bytes();
        let focus_reporting = runtime.focus_reporting_enabled();
        let mut nudged = false;
        if nudge && should_send_wakeup(unread_input_bytes, focus_reporting) {
            match crate::ghostty::encode_focus(crate::ghostty::FocusEvent::Gained) {
                Ok(report) => match runtime.try_send_bytes(Bytes::from(report)) {
                    Ok(()) => nudged = true,
                    Err(err) => {
                        tracing::warn!(err = %err, "failed to write the Codex input wake-up")
                    }
                },
                Err(err) => tracing::warn!(err = %err, "failed to encode the Codex input wake-up"),
            }
        }
        let response = AgentInputWakeupProjectionResponse {
            id: id.clone(),
            result: AgentInputWakeup {
                unread_input_bytes,
                focus_reporting,
                nudged,
            },
        };
        serde_json::to_string(&response)
            .unwrap_or_else(|error| encode_error(id, "serialization_error", error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::should_send_wakeup;

    #[test]
    fn wakeup_needs_queued_bytes_and_focus_reporting() {
        assert!(should_send_wakeup(Some(1571), true));
        assert!(
            !should_send_wakeup(Some(0), true),
            "an empty queue is never nudged"
        );
        assert!(
            !should_send_wakeup(None, true),
            "an unmeasured queue is never nudged"
        );
        assert!(
            !should_send_wakeup(Some(1571), false),
            "without focus reporting the report would be ordinary input"
        );
    }
}
