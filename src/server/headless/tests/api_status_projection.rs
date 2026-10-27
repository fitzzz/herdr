use super::*;

fn project_status(server: &mut HeadlessServer, request: api::schema::Request) -> String {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(api::ApiRequestMessage {
        request,
        projection: Some(api::ApiRequestProjection::PaneAgentStatus),
        peer_process_id: None,
        respond_to,
        response_write_complete: None,
    });
    response_rx.recv().unwrap()
}

#[test]
fn status_projection_rejects_mutation_before_headless_side_effects() {
    let mut server = test_headless_server();
    let response = project_status(
        &mut server,
        api::schema::Request {
            id: "invalid-projection".into(),
            method: api::schema::Method::ServerStop(Default::default()),
        },
    );
    let error: api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();
    assert_eq!(error.id, "invalid-projection");
    assert_eq!(error.error.code, "internal_error");
    assert!(!server.app.state.should_quit);
    assert!(!server.shutting_down);
}

#[test]
fn status_projection_drains_queued_status_events_before_reading() {
    let hub = api::EventHub::default();
    let mut server = test_headless_server_with_event_hub(hub.clone());
    server.app.state.workspaces = vec![crate::workspace::Workspace::test_new("projection")];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    let pane_id = server.app.state.workspaces[0].tabs[0].root_pane;
    let public_pane_id = server.app.public_pane_id(0, pane_id).unwrap();
    let terminal_id = server.app.state.workspaces[0]
        .pane_state(pane_id)
        .unwrap()
        .attached_terminal_id
        .clone();
    server
        .app
        .state
        .terminals
        .get_mut(&terminal_id)
        .unwrap()
        .set_detected_state(
            Some(crate::detect::Agent::Pi),
            crate::detect::AgentState::Working,
        );
    // A backlog larger than the ordinary drain limit must be current before
    // answering, exactly as it was for pane.get.
    for index in 0..=crate::app::APP_EVENT_DRAIN_LIMIT {
        server
            .app
            .event_tx
            .try_send(AppEvent::UpdateReady {
                version: format!("4.0.{index}"),
                install_command: "herdr install".into(),
            })
            .unwrap();
    }
    server
        .app
        .event_tx
        .try_send(AppEvent::StateChanged {
            pane_id,
            agent: Some(crate::detect::Agent::Pi),
            state: crate::detect::AgentState::Blocked,
            visible_blocker: true,
            visible_working: false,
            process_exited: false,
            observed_at: Instant::now(),
        })
        .unwrap();
    let response = project_status(
        &mut server,
        api::schema::Request {
            id: "fresh-projection".into(),
            method: api::schema::Method::PaneGet(api::schema::PaneTarget {
                pane_id: public_pane_id,
            }),
        },
    );
    let response: api::PaneAgentStatusProjectionResponse = serde_json::from_str(&response).unwrap();
    assert_eq!(
        response.result.agent_status,
        api::schema::AgentStatus::Blocked
    );
    assert!(server.app.event_rx.is_empty());
    assert_eq!(
        server.app.state.update_available,
        Some(format!("4.0.{}", crate::app::APP_EVENT_DRAIN_LIMIT))
    );
    assert!(hub.events_after(0).iter().any(|(_, event)| matches!(
        event.data,
        api::schema::EventData::PaneAgentStatusChanged {
            agent_status: api::schema::AgentStatus::Blocked,
            ..
        }
    )));
}

#[tokio::test]
async fn status_projection_keeps_pending_terminal_title_processing() {
    let hub = api::EventHub::default();
    let mut server = test_headless_server_with_event_hub(hub.clone());
    server.app.state.workspaces = vec![crate::workspace::Workspace::test_new("projection")];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    let pane_id = server.app.state.workspaces[0].tabs[0].root_pane;
    let public_pane_id = server.app.public_pane_id(0, pane_id).unwrap();
    let terminal_id = server.app.state.workspaces[0]
        .pane_state(pane_id)
        .unwrap()
        .attached_terminal_id
        .clone();
    let runtime = crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 24, b"");
    runtime.test_process_pty_bytes(b"\x1b]2;current title\x1b\\");
    server
        .app
        .terminal_runtimes
        .insert(terminal_id.clone(), runtime);
    server.app.render_dirty.request_terminal_title(pane_id);
    let response = project_status(
        &mut server,
        api::schema::Request {
            id: "title-projection".into(),
            method: api::schema::Method::PaneGet(api::schema::PaneTarget {
                pane_id: public_pane_id,
            }),
        },
    );
    let _: api::PaneAgentStatusProjectionResponse = serde_json::from_str(&response).unwrap();
    assert_eq!(
        server.app.state.terminals[&terminal_id]
            .terminal_title
            .as_deref(),
        Some("current title")
    );
    assert_eq!(pane_updated_events(&hub), 1);
}
