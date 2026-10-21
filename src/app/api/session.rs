use crate::api::schema::{ResponseResult, SessionSnapshot};
use crate::app::App;

use super::responses::encode_success;

#[cfg(test)]
thread_local! {
    static SESSION_SNAPSHOT_OBSERVATIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

impl App {
    #[cfg(test)]
    pub(crate) fn test_metadata_observation_counts(&self) -> (u64, u64) {
        (
            SESSION_SNAPSHOT_OBSERVATIONS.with(std::cell::Cell::get),
            crate::app::creation::PANE_INFO_OBSERVATIONS.with(std::cell::Cell::get),
        )
    }

    pub(super) fn handle_session_snapshot(&mut self, id: String) -> String {
        encode_success(
            id,
            ResponseResult::SessionSnapshot {
                snapshot: Box::new(self.session_snapshot()),
            },
        )
    }

    pub(crate) fn session_snapshot(&self) -> SessionSnapshot {
        let started = crate::render_prof::timer();
        #[cfg(test)]
        SESSION_SNAPSHOT_OBSERVATIONS.with(|count| count.set(count.get() + 1));
        let focused_workspace_id = self
            .state
            .active
            .map(|ws_idx| self.public_workspace_id(ws_idx));
        let focused_tab_id = self.state.active.and_then(|ws_idx| {
            let ws = self.state.workspaces.get(ws_idx)?;
            self.public_tab_id(ws_idx, ws.active_tab)
        });
        let focused_pane_id = self.state.active.and_then(|ws_idx| {
            let ws = self.state.workspaces.get(ws_idx)?;
            self.public_pane_id(ws_idx, ws.focused_pane_id()?)
        });

        let mut workspaces = Vec::new();
        let mut tabs = Vec::new();
        let mut layouts = Vec::new();
        for (ws_idx, ws) in self.state.workspaces.iter().enumerate() {
            workspaces.push(self.workspace_info(ws_idx));
            for tab_idx in 0..ws.tabs.len() {
                if let Some(tab) = self.tab_info(ws_idx, tab_idx) {
                    tabs.push(tab);
                }
                if let Some(layout) = self.pane_layout_snapshot(ws_idx, tab_idx) {
                    layouts.push(layout);
                }
            }
        }

        // Observe each pane once so its agent projection shares the same live cwd,
        // scroll and presentation observation instead of repeating runtime queries.
        let panes = self.collect_panes_for_workspace(None).unwrap_or_default();
        let agents = panes
            .iter()
            .filter_map(|pane| self.agent_info_from_pane(pane))
            .collect();
        let snapshot = SessionSnapshot {
            version: crate::build_info::version(),
            protocol: crate::protocol::PROTOCOL_VERSION,
            focused_workspace_id,
            focused_tab_id,
            focused_pane_id,
            workspaces,
            tabs,
            panes,
            layouts,
            agents,
        };
        crate::render_prof::duration_since("metadata.session_snapshot", started);
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use crate::api::schema::{EmptyParams, Method, ResponseResult, SuccessResponse};
    use crate::{config::Config, workspace::Workspace};

    fn app_with_two_tabs() -> crate::app::App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = crate::app::App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let mut workspace = Workspace::test_new("snapshot");
        workspace.test_add_tab(None);
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app
    }

    #[test]
    fn metadata_session_agents_reuse_pane_observations_and_keep_authority() {
        let mut app = app_with_two_tabs();
        let first = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].terminal_id(first).unwrap().clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.agent_name = Some("metadata-agent".into());
        terminal.last_agent_state_change_seq = Some(19);
        terminal.last_agent_completion_seq = Some(17);
        terminal.state_change_epoch = Some("metadata-epoch".into());
        let expected = app.collect_agent_infos();
        let before = app.test_metadata_observation_counts();
        let snapshot = app.session_snapshot();
        let after = app.test_metadata_observation_counts();
        assert_eq!(after.0 - before.0, 1);
        assert_eq!(after.1 - before.1, snapshot.panes.len() as u64);
        assert_eq!(snapshot.agents, expected);
        assert_eq!(snapshot.agents.len(), 1, "nonagent panes remain excluded");
        assert_eq!(
            snapshot.agents[0].state_change_epoch.as_deref(),
            Some("metadata-epoch")
        );
        assert_eq!(snapshot.agents[0].completion_seq, Some(17));
        assert_eq!(snapshot.agents[0].state_change_seq, 19);
        assert_eq!(snapshot.agents[0].cwd, snapshot.panes[0].cwd);
        assert_eq!(
            snapshot.agents[0].foreground_cwd,
            snapshot.panes[0].foreground_cwd
        );
    }

    #[test]
    fn session_snapshot_bootstraps_runtime_resources() {
        let mut app = app_with_two_tabs();
        let response = app.handle_api_request(crate::api::schema::Request {
            id: "req_snapshot".into(),
            method: Method::SessionSnapshot(EmptyParams::default()),
        });

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::SessionSnapshot { snapshot } = success.result else {
            panic!("expected session snapshot response");
        };
        assert_eq!(success.id, "req_snapshot");
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.tabs.len(), 2);
        assert_eq!(snapshot.panes.len(), 2);
        assert_eq!(snapshot.layouts.len(), 2);
        assert_eq!(
            snapshot.focused_workspace_id.as_deref(),
            Some(snapshot.workspaces[0].workspace_id.as_str())
        );
        assert_eq!(
            snapshot.focused_tab_id.as_deref(),
            Some(snapshot.tabs[0].tab_id.as_str())
        );
        assert_eq!(
            snapshot.focused_pane_id.as_deref(),
            Some(snapshot.panes[0].pane_id.as_str())
        );
    }
}
