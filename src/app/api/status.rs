use crate::api::schema::{PaneAgentStatusChangedEvent, PaneTarget};
use crate::api::PaneAgentStatusProjectionResponse;
use crate::app::{api_helpers::pane_agent_status, App, AppState};
use crate::layout::PaneId;

use super::responses::encode_error;

// The projection depends only on session data, never the terminal runtime
// registry. In particular it cannot wait for a PTY actor, inspect processes,
// read cwd, format terminal output or collect scroll metrics.
fn pane_agent_status_snapshot(
    state: &AppState,
    ws_idx: usize,
    pane_id: PaneId,
) -> Option<PaneAgentStatusChangedEvent> {
    let ws = state.workspaces.get(ws_idx)?;
    let pane = ws.pane_state(pane_id)?;
    let terminal = state.terminals.get(&pane.attached_terminal_id)?;
    // Retain pane.get's missing-pane semantics for incomplete identities.
    let tab_idx = ws.find_tab_index_for_pane(pane_id)?;
    ws.public_tab_number(tab_idx)?;
    let pane_number = ws.public_pane_number(pane_id)?;
    let presentation = terminal.effective_presentation();
    Some(PaneAgentStatusChangedEvent {
        pane_id: crate::workspace::public_pane_id_for_number(&ws.id, pane_number),
        workspace_id: ws.id.clone(),
        agent_status: pane_agent_status(terminal.state, pane.seen),
        agent: terminal.effective_agent_label().map(str::to_string),
        title: presentation.title,
        display_agent: presentation.display_agent,
        state_labels: presentation.state_labels,
    })
}

impl App {
    pub(super) fn handle_pane_agent_status_projection(
        &self,
        id: String,
        target: PaneTarget,
    ) -> String {
        let pane = self
            .parse_pane_id(&target.pane_id)
            .and_then(|(ws_idx, pane_id)| pane_agent_status_snapshot(&self.state, ws_idx, pane_id));
        let Some(pane) = pane else {
            return encode_error(
                id,
                "pane_not_found",
                format!("pane {} not found", target.pane_id),
            );
        };
        let response = PaneAgentStatusProjectionResponse {
            id: id.clone(),
            result: pane,
        };
        serde_json::to_string(&response)
            .unwrap_or_else(|error| encode_error(id, "serialization_error", error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{AgentStatus, ErrorResponse, Method, Request};
    use crate::detect::{Agent, AgentState};
    use crate::terminal::AgentMetadataReport;
    use std::collections::HashMap;
    use std::time::Duration;

    fn projection_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state = AppState::test_with_adversarial_identity_state();
        app.state.assert_invariants_for_test();
        app
    }

    #[test]
    fn status_projection_matches_public_status_fields_with_adversarial_identity_and_seen_state() {
        let mut app = projection_app();
        assert!(app.terminal_runtimes.is_empty());
        let pane_ids = app.state.workspaces[0]
            .tabs
            .iter()
            .flat_map(|tab| tab.panes.keys().copied())
            .collect::<Vec<_>>();
        for pane_id in pane_ids {
            let terminal_id = app.state.workspaces[0]
                .pane_state(pane_id)
                .unwrap()
                .attached_terminal_id
                .clone();
            for state in [
                AgentState::Unknown,
                AgentState::Working,
                AgentState::Blocked,
                AgentState::Idle,
            ] {
                for seen in [false, true] {
                    app.state.workspaces[0]
                        .pane_state_mut(pane_id)
                        .unwrap()
                        .seen = seen;
                    let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
                    terminal.set_detected_state(Some(Agent::Pi), state);
                    terminal.set_agent_metadata(AgentMetadataReport {
                        source: "user:status-projection".into(),
                        agent_label: Some("pi".into()),
                        applies_to_source: None,
                        title: Some("reviewing".into()),
                        display_agent: Some("Auditor".into()),
                        state_labels: HashMap::from([("working".into(), "Auditing".into())]),
                        clear_title: false,
                        clear_display_agent: false,
                        clear_state_labels: false,
                        ttl: None,
                        seq: None,
                    });
                    let pane = app.pane_info(0, pane_id).unwrap();
                    let expected = PaneAgentStatusChangedEvent {
                        pane_id: pane.pane_id.clone(),
                        workspace_id: pane.workspace_id,
                        agent_status: pane.agent_status,
                        agent: pane.agent,
                        title: pane.title,
                        display_agent: pane.display_agent,
                        state_labels: pane.state_labels,
                    };
                    // Canonical handles and legacy aliases must resolve identically.
                    for target in [pane.pane_id, format!("p_{}", pane_id.raw())] {
                        let response = app.handle_pane_agent_status_projection(
                            "status".into(),
                            PaneTarget { pane_id: target },
                        );
                        let projection: PaneAgentStatusProjectionResponse =
                            serde_json::from_str(&response).unwrap();
                        assert_eq!(projection.result, expected);
                        if state == AgentState::Idle {
                            assert_eq!(
                                projection.result.agent_status,
                                if seen {
                                    AgentStatus::Idle
                                } else {
                                    AgentStatus::Done
                                }
                            );
                        }
                    }
                }
            }
        }
        app.state.assert_invariants_for_test();
        assert!(app.terminal_runtimes.is_empty());
    }

    #[test]
    fn status_projection_observes_metadata_expiry_and_equal_status_label_changes() {
        let mut app = projection_app();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0]
            .pane_state(pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Working);
        terminal.set_agent_metadata(AgentMetadataReport {
            source: "user:status-projection".into(),
            agent_label: Some("pi".into()),
            applies_to_source: None,
            title: Some("short lived".into()),
            display_agent: Some("Auditor".into()),
            state_labels: HashMap::from([("working".into(), "Auditing".into())]),
            clear_title: false,
            clear_display_agent: false,
            clear_state_labels: false,
            // Advance the recorded deadline explicitly; do not rely on a sleep.
            ttl: Some(Duration::from_secs(60)),
            seq: None,
        });
        let deadline = terminal.next_agent_metadata_expiry().unwrap();
        let before = pane_agent_status_snapshot(&app.state, 0, pane_id).unwrap();
        assert_eq!(before.agent_status, AgentStatus::Working);
        assert_eq!(before.title.as_deref(), Some("short lived"));
        app.expire_metadata_at(deadline, deadline);
        let after = pane_agent_status_snapshot(&app.state, 0, pane_id).unwrap();
        assert_eq!(after.agent_status, before.agent_status);
        assert_eq!(after.title, None);
        assert_eq!(after.display_agent, None);
        assert!(after.state_labels.is_empty());
        assert!(app.event_hub.events_after(0).iter().any(|(_, event)| matches!(
            &event.data,
            crate::api::schema::EventData::PaneAgentStatusChanged { title: None, state_labels, .. }
                if state_labels.is_empty()
        )));
    }

    #[test]
    fn status_projection_preserves_public_missing_pane_errors() {
        let mut app = projection_app();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let public_id = app.public_pane_id(0, pane_id).unwrap();
        let terminal_id = app.state.workspaces[0]
            .pane_state(pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();
        app.state.terminals.remove(&terminal_id);
        for pane_id in ["pane_missing".to_string(), public_id] {
            let target = PaneTarget { pane_id };
            let public = app.handle_pane_get("missing".into(), target.clone());
            let projection = app.handle_pane_agent_status_projection("missing".into(), target);
            assert_eq!(projection, public);
            let error: ErrorResponse = serde_json::from_str(&projection).unwrap();
            assert_eq!(error.error.code, "pane_not_found");
        }
    }

    #[test]
    fn status_projection_app_dispatch_rejects_a_public_mutation() {
        let mut app = projection_app();
        let response = app.handle_api_projection_after_internal_events_drained(
            Request {
                id: "invalid".into(),
                method: Method::ServerStop(Default::default()),
            },
            crate::api::ApiRequestProjection::PaneAgentStatus,
        );
        let error: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "internal_error");
        assert!(!app.state.should_quit);
    }
}
