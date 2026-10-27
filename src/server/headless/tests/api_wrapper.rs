use super::*;
use crate::api::schema::{Method, PaneTarget, ResponseResult};
use crate::detect::{Agent, AgentState};
use std::cell::Cell;

thread_local! {
    pub(crate) static NOTIFICATION_SCAN_VISITS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

fn request(server: &mut HeadlessServer, method: Method) -> serde_json::Value {
    let (respond_to, responses) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(api::ApiRequestMessage {
        request: api::schema::Request {
            id: "wrapper-regression".into(),
            method,
        },
        projection: None,
        peer_process_id: None,
        respond_to,
        response_write_complete: None,
    });
    serde_json::from_str(&responses.try_recv().expect("synchronous API response")).unwrap()
}

fn observational_methods(pane_id: &str) -> Vec<Method> {
    vec![
        Method::PaneGet(PaneTarget {
            pane_id: pane_id.into(),
        }),
        Method::PaneList(api::schema::PaneListParams::default()),
        Method::PaneCurrent(api::schema::PaneCurrentParams::default()),
        Method::AgentList(api::schema::EmptyParams::default()),
    ]
}

fn install_state(
    server: &mut HeadlessServer,
    pane_count: usize,
) -> (crate::layout::PaneId, String) {
    let mut workspace = crate::workspace::Workspace::test_new("wrapper");
    for _ in 1..pane_count {
        workspace.test_split(ratatui::layout::Direction::Horizontal);
    }
    server.app.state.workspaces = vec![workspace];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    let pane_id = server.app.state.workspaces[0].focused_pane_id().unwrap();
    let terminal_id = server.app.state.workspaces[0]
        .terminal_id(pane_id)
        .unwrap()
        .clone();
    server
        .app
        .state
        .terminals
        .get_mut(&terminal_id)
        .unwrap()
        .detected_agent = Some(Agent::Claude);
    let public_id = server.app.public_pane_id(0, pane_id).unwrap();
    (pane_id, public_id)
}

#[test]
fn observational_reads_do_no_global_notification_scans_at_retained_pane_scale() {
    for pane_count in [1, 15, 174] {
        let mut server = test_headless_server();
        let (_, public_id) = install_state(&mut server, pane_count);
        let pane_id = server.app.state.workspaces[0].focused_pane_id().unwrap();
        let terminal_id = server.app.state.workspaces[0]
            .terminal_id(pane_id)
            .unwrap()
            .clone();
        let native_session = crate::agent_resume::PersistedAgentSession {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("native-wrapper-session")
                .unwrap(),
        };
        server
            .app
            .state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .persisted_agent_session = Some(native_session.clone());
        let prefix_storage = server.app.state.prefix_keys.as_ptr();
        let key_storage = server.app.state.keybinds.help.bindings.as_ptr();
        NOTIFICATION_SCAN_VISITS.with(|visits| visits.set((0, 0)));
        for _ in 0..4 {
            for method in observational_methods(&public_id) {
                let response = request(&mut server, method);
                assert!(response.get("result").is_some(), "{response}");
            }
        }
        NOTIFICATION_SCAN_VISITS.with(|visits| assert_eq!(visits.get(), (0, 0)));
        assert_eq!(server.app.state.prefix_keys.as_ptr(), prefix_storage);
        assert_eq!(
            server.app.state.keybinds.help.bindings.as_ptr(),
            key_storage
        );
        assert_eq!(
            server.app.state.workspaces[0].public_pane_numbers.len(),
            pane_count
        );
        assert_eq!(
            server.app.state.terminals[&terminal_id].persisted_agent_session,
            Some(native_session)
        );
        let pane = request(
            &mut server,
            Method::PaneGet(PaneTarget {
                pane_id: public_id.clone(),
            }),
        );
        assert_eq!(
            pane["result"]["pane"]["agent_session"]["value"],
            "native-wrapper-session"
        );

        // A UI-neutral method may reconcile managed state. It stays on the
        // conservative path regardless of request_changes_ui's classification.
        let method = Method::AgentGet(api::schema::AgentTarget {
            target: public_id.clone(),
        });
        assert!(!api::request_changes_ui(&api::schema::Request {
            id: "classification".into(),
            method: method.clone(),
        }));
        assert!(request(&mut server, method).get("result").is_some());
        NOTIFICATION_SCAN_VISITS.with(|visits| assert_eq!(visits.get(), (pane_count, pane_count)));

        NOTIFICATION_SCAN_VISITS.with(|visits| visits.set((0, 0)));
        let missing = request(
            &mut server,
            Method::PaneGet(PaneTarget {
                pane_id: "w0:p0".into(),
            }),
        );
        assert_eq!(missing["error"]["code"], "pane_not_found");
        NOTIFICATION_SCAN_VISITS.with(|visits| assert_eq!(visits.get(), (0, 0)));
    }
}

fn queue_hook(
    server: &mut HeadlessServer,
    pane_id: crate::layout::PaneId,
    state: AgentState,
    seq: u64,
) {
    server
        .app
        .event_tx
        .try_send(AppEvent::HookStateReported {
            pane_id,
            source: "custom:pi".into(),
            agent_label: "pi".into(),
            state,
            message: None,
            seq: Some(seq),
            session_ref: None,
        })
        .unwrap();
}

fn notification_messages(
    server: &mut HeadlessServer,
    receiver: &std::sync::mpsc::Receiver<Vec<u8>>,
) -> Vec<serde_json::Value> {
    server.send_to_client(
        1,
        ServerMessage::EndpointControl {
            kind: "test.wrapper-barrier".into(),
            data: String::new(),
        },
    );
    let mut messages = Vec::new();
    loop {
        let message = read_server_message(receiver.try_recv().expect("notification or barrier"));
        if matches!(&message, ServerMessage::EndpointControl { kind, .. } if kind == "test.wrapper-barrier")
        {
            return messages;
        }
        if matches!(
            &message,
            ServerMessage::SemanticNotification(_) | ServerMessage::Notify { .. }
        ) {
            messages.push(serde_json::to_value(message).unwrap());
        }
    }
}

// Compare the optimized reads with the retained wrapper path around exactly the
// same queued transitions. Public pane/tab IDs are fixed; runtime IDs stay local.
fn queued_transition_trace(
    optimized: bool,
    active: bool,
    focused: bool,
    delay: u64,
) -> (Vec<serde_json::Value>, Vec<(u64, api::schema::EventKind)>) {
    let hub = api::EventHub::default();
    let mut server = test_headless_server_with_event_hub(hub.clone());
    let (writer, control, _render) = test_client_writer();
    let mut background = crate::workspace::Workspace::test_new("background");
    background.id = "w1".into();
    let mut foreground = crate::workspace::Workspace::test_new("foreground");
    foreground.id = "w2".into();
    let pane_id = background.tabs[0].root_pane;
    server.app.state.workspaces = vec![background, foreground];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(if active { 0 } else { 1 });
    server.app.state.toast_config.delay_seconds = delay;
    server.app.state.toast_config.delivery = crate::config::ToastDelivery::Terminal;
    let mut client = ClientConnection::new(
        (80, 24),
        Default::default(),
        1,
        RenderEncoding::SemanticFrame,
        Some(writer),
    );
    client.outer_terminal_focus = Some(focused);
    server.clients.insert(1, client);
    server.foreground_client_id = Some(1);
    let public_id = server.app.public_pane_id(0, pane_id).unwrap();

    server
        .app
        .event_tx
        .try_send(AppEvent::AgentProcessDetected {
            pane_id,
            agent: Agent::Pi,
            observed_at: Instant::now(),
        })
        .unwrap();
    // Startup blocked/idle must not manufacture completion, a real work cycle
    // must notify once, and a stale report must not insert another transition.
    for (state, seq) in [
        (AgentState::Blocked, 1),
        (AgentState::Idle, 2),
        (AgentState::Working, 3),
        (AgentState::Blocked, 4),
        (AgentState::Idle, 4),
        (AgentState::Idle, 5),
    ] {
        queue_hook(&mut server, pane_id, state, seq);
    }
    // Exceed one internal drain batch with harmless stale reports, then add a
    // fresh tail. A read must expose that tail rather than stop at the batch.
    for _ in 0..=crate::app::APP_EVENT_DRAIN_LIMIT {
        queue_hook(&mut server, pane_id, AgentState::Blocked, 1);
    }
    queue_hook(&mut server, pane_id, AgentState::Working, 6);
    let method = if optimized {
        Method::PaneGet(PaneTarget {
            pane_id: public_id.clone(),
        })
    } else {
        Method::WorkspaceList(api::schema::EmptyParams::default())
    };
    assert!(request(&mut server, method).get("result").is_some());
    assert!(server.app.event_rx.try_recv().is_err());
    let terminal_id = server.app.state.workspaces[0].terminal_id(pane_id).unwrap();
    assert_eq!(
        server.app.state.terminals[terminal_id].state,
        AgentState::Working
    );

    if let Some(deadline) = server.app.state.next_pending_agent_notification_deadline() {
        server.handle_scheduled_tasks_headless(deadline, false);
    }
    let messages = notification_messages(&mut server, &control);
    let events = hub
        .events_after(0)
        .into_iter()
        .map(|(seq, event)| (seq, event.event))
        .collect();
    (messages, events)
}

#[test]
fn observational_reads_preserve_queued_notifications_and_event_sequence() {
    for active in [false, true] {
        for focused in [false, true] {
            for delay in [0, 60] {
                let retained = queued_transition_trace(false, active, focused, delay);
                let optimized = queued_transition_trace(true, active, focused, delay);
                assert!(
                    !retained.0.is_empty(),
                    "trace must exercise notification delivery"
                );
                assert_eq!(
                    optimized, retained,
                    "active={active} focused={focused} delay={delay}"
                );
            }
        }
    }
}

#[tokio::test]
async fn observational_reads_expire_metadata_tokens_and_sync_terminal_titles() {
    for method_index in 0..4 {
        let hub = api::EventHub::default();
        let mut server = test_headless_server_with_event_hub(hub.clone());
        let (pane_id, public_id) = install_state(&mut server, 1);
        let terminal_id = server.app.state.workspaces[0]
            .terminal_id(pane_id)
            .unwrap()
            .clone();
        server.handle_internal_event_with_forwarding(AppEvent::HookMetadataReported {
            pane_id,
            source: "user:display".into(),
            agent_label: None,
            applies_to_source: None,
            title: Some("expired title".into()),
            display_agent: Some("expired display".into()),
            state_labels: HashMap::new(),
            clear_title: false,
            clear_display_agent: false,
            clear_state_labels: false,
            seq: None,
            ttl: Some(Duration::ZERO),
        });
        // Put token expiry in the past deterministically; no wall-clock wait.
        let past = Instant::now() - Duration::from_secs(1);
        server
            .app
            .state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .metadata_tokens
            .patch(
                HashMap::from([("task".into(), Some("expired pane token".into()))]),
                Some(Duration::ZERO),
                past,
            );
        server.app.state.workspaces[0].metadata_tokens.patch(
            HashMap::from([("task".into(), Some("expired workspace token".into()))]),
            Some(Duration::ZERO),
            past,
        );
        server.app.sync_agent_metadata_deadline();
        assert!(server.app.agent_metadata_deadline.is_some());
        let before = hub.current_sequence();
        // Screen-seeding writes directly to Ghostty before Herdr's OSC tracker
        // exists. Feed the title through PTY processing so it is an observation
        // the API can synchronize, as in the production reader path.
        let runtime = crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 24, b"");
        assert_eq!(runtime.terminal_title(), None);
        runtime.test_process_pty_bytes(b"\x1b]0;latest title\x07");
        assert_eq!(runtime.terminal_title().as_deref(), Some("latest title"));
        assert_eq!(
            server.app.state.terminals[&terminal_id].terminal_title,
            None
        );
        server
            .app
            .terminal_runtimes
            .insert(terminal_id.clone(), runtime);
        server.app.render_dirty.request_terminal_title(pane_id);
        let method = observational_methods(&public_id).remove(method_index);
        let response = request(&mut server, method);
        assert!(response.get("result").is_some(), "{response}");
        let terminal = &server.app.state.terminals[&terminal_id];
        assert_eq!(terminal.effective_title(), None);
        assert_eq!(terminal.effective_display_agent(), None);
        assert!(terminal.metadata_tokens.values().is_empty());
        assert!(server.app.state.workspaces[0]
            .metadata_tokens
            .values()
            .is_empty());
        assert_eq!(terminal.terminal_title.as_deref(), Some("latest title"));
        assert_eq!(server.app.agent_metadata_deadline, None);
        let events = hub.events_after(before);
        assert!(events
            .iter()
            .any(|(_, ev)| ev.event == api::schema::EventKind::PaneUpdated));
        assert!(events
            .iter()
            .any(|(_, ev)| ev.event == api::schema::EventKind::WorkspaceMetadataUpdated));
        // Verify the actual read, including native terminal identity and freshness.
        let pane = request(
            &mut server,
            Method::PaneGet(PaneTarget {
                pane_id: public_id.clone(),
            }),
        );
        assert_eq!(
            pane["result"]["pane"]["terminal_id"],
            terminal_id.to_string()
        );
        assert_eq!(pane["result"]["pane"]["pane_id"], public_id);
        assert_eq!(pane["result"]["pane"]["terminal_title"], "latest title");
        shutdown_test_runtimes(&mut server);
    }
}

#[test]
fn observational_read_follows_queued_workspace_removal_without_retaining_indexes() {
    let mut server = test_headless_server();
    let removed = crate::workspace::Workspace::test_new("removed");
    let survivor = crate::workspace::Workspace::test_new("survivor");
    let removed_pane = removed.tabs[0].root_pane;
    let survivor_pane = survivor.tabs[0].root_pane;
    server.app.state.workspaces = vec![removed, survivor];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(1);
    let survivor_public = server.app.public_pane_id(1, survivor_pane).unwrap();
    let terminal_id = server.app.state.workspaces[1]
        .terminal_id(survivor_pane)
        .unwrap()
        .clone();
    server
        .app
        .event_tx
        .try_send(AppEvent::PaneDied {
            pane_id: removed_pane,
            exit_reason: crate::platform::ChildExitReason::Exited,
        })
        .unwrap();
    queue_hook(&mut server, survivor_pane, AgentState::Working, 1);
    let response = request(
        &mut server,
        Method::PaneGet(PaneTarget {
            pane_id: survivor_public.clone(),
        }),
    );
    assert_eq!(server.app.state.workspaces.len(), 1);
    assert_eq!(response["result"]["pane"]["pane_id"], survivor_public);
    assert_eq!(
        response["result"]["pane"]["terminal_id"],
        terminal_id.to_string()
    );
    assert_eq!(response["result"]["pane"]["agent_status"], "working");
}

#[test]
fn foreground_sync_keeps_binding_storage_while_refreshing_size_focus_and_seen() {
    let mut server = test_headless_server();
    let (pane_id, _) = install_state(&mut server, 1);
    server.app.state.workspaces[0]
        .pane_state_mut(pane_id)
        .unwrap()
        .seen = false;
    let prefix_storage = server.app.state.prefix_keys.as_ptr();
    let key_storage = server.app.state.keybinds.help.bindings.as_ptr();
    for (client_id, size, endpoint_keys) in [(1, (80, 24), false), (2, (120, 40), true)] {
        let mut client = ClientConnection::new(
            size,
            Default::default(),
            client_id,
            RenderEncoding::SemanticFrame,
            None,
        );
        client.shell_uses_endpoint_keybindings = endpoint_keys;
        client.outer_terminal_focus = Some(client_id == 2);
        server.clients.insert(client_id, client);
        server.promote_client_to_foreground(client_id);
        assert_eq!(server.effective_size, size);
        assert_eq!(server.app.state.prefix_keys.as_ptr(), prefix_storage);
        assert_eq!(
            server.app.state.keybinds.help.bindings.as_ptr(),
            key_storage
        );
    }
    assert!(
        server.app.state.workspaces[0]
            .pane_state(pane_id)
            .unwrap()
            .seen
    );
    let client = server.clients.get_mut(&2).unwrap();
    client.terminal_size = (100, 31);
    client.outer_terminal_focus = Some(false);
    client.pixel_mouse = true;
    server.sync_foreground_client_state();
    assert_eq!(server.effective_size, (100, 31));
    assert_eq!(
        server.app.state.view.terminal_area,
        Rect::new(0, 0, 100, 31)
    );
    assert_eq!(server.app.state.outer_terminal_focus, Some(false));
    assert!(server.app.pixel_mouse_available);
    server.clients.remove(&2);
    server.promote_latest_remaining_client();
    assert_eq!(server.foreground_client_id, Some(1));
    assert_eq!(server.effective_size, (80, 24));
    server.clients.clear();
    server.promote_latest_remaining_client();
    assert_eq!(server.effective_size, server.headless_size);
    assert_eq!(server.app.state.outer_terminal_focus, None);
    assert_eq!(server.app.state.prefix_keys.as_ptr(), prefix_storage);
    assert_eq!(
        server.app.state.keybinds.help.bindings.as_ptr(),
        key_storage
    );
}

#[test]
fn server_config_reload_replaces_valid_bindings_and_retains_them_on_invalid_reload() {
    let _lock = crate::config::test_config_env_lock().lock().unwrap();
    let mut server = test_headless_server();
    install_state(&mut server, 1);
    let path = server
        .client_socket_path
        .with_file_name("wrapper-config.toml");
    let original = std::env::var_os(crate::config::CONFIG_PATH_ENV_VAR);
    struct RestoreConfigEnv(Option<std::ffi::OsString>);
    impl Drop for RestoreConfigEnv {
        fn drop(&mut self) {
            if let Some(value) = &self.0 {
                std::env::set_var(crate::config::CONFIG_PATH_ENV_VAR, value);
            } else {
                std::env::remove_var(crate::config::CONFIG_PATH_ENV_VAR);
            }
        }
    }
    let _restore = RestoreConfigEnv(original);
    std::env::set_var(crate::config::CONFIG_PATH_ENV_VAR, &path);
    for (contents, status, expected_prefix) in [
        (
            "[keys]\nprefix = \"ctrl+space\"\nnew_workspace = \"prefix+m\"\n",
            crate::config::ConfigReloadStatus::Applied,
            (
                crossterm::event::KeyCode::Char(' '),
                crossterm::event::KeyModifiers::CONTROL,
            ),
        ),
        (
            "[keys\n",
            crate::config::ConfigReloadStatus::Failed,
            (
                crossterm::event::KeyCode::Char(' '),
                crossterm::event::KeyModifiers::CONTROL,
            ),
        ),
    ] {
        fs::write(&path, contents).unwrap();
        let response = request(
            &mut server,
            Method::ServerReloadConfig(api::schema::EmptyParams::default()),
        );
        let response: api::schema::SuccessResponse = serde_json::from_value(response).unwrap();
        let ResponseResult::ConfigReload { status: actual, .. } = response.result else {
            panic!("config reload result");
        };
        assert_eq!(actual, status);
        assert_eq!(server.app.state.prefix_keys, vec![expected_prefix]);
        assert_eq!(server.server_keybindings.prefix, vec![expected_prefix]);
        assert_eq!(
            server.app.state.keybinds.new_workspace,
            server.server_keybindings.keybinds.new_workspace
        );
        assert_eq!(
            server.app.state.keybinds.new_workspace.label().as_deref(),
            Some("prefix+m")
        );
        let prefix_storage = server.app.state.prefix_keys.as_ptr();
        let key_storage = server.app.state.keybinds.new_workspace.bindings.as_ptr();
        server.sync_foreground_client_state();
        assert_eq!(server.app.state.prefix_keys.as_ptr(), prefix_storage);
        assert_eq!(
            server.app.state.keybinds.new_workspace.bindings.as_ptr(),
            key_storage
        );
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn direct_agent_reports_keep_scans_and_deliver_each_effective_transition_once() {
    let (writer, control, _render) = test_client_writer();
    let (mut server, pane_id) = completion_guard_server(writer);
    let public_id = server.app.public_pane_id(0, pane_id).unwrap();
    NOTIFICATION_SCAN_VISITS.with(|visits| visits.set((0, 0)));
    for (state, seq) in [
        (api::schema::PaneAgentState::Working, 1),
        (api::schema::PaneAgentState::Blocked, 2),
        (api::schema::PaneAgentState::Idle, 2),
        (api::schema::PaneAgentState::Idle, 3),
    ] {
        completion_guard_api_report(
            &mut server,
            Method::PaneReportAgent(api::schema::PaneReportAgentParams {
                pane_id: public_id.clone(),
                source: "custom:pi".into(),
                agent: "pi".into(),
                state,
                message: None,
                seq: Some(seq),
                agent_session_id: None,
                agent_session_path: None,
                resume_argv: None,
            }),
        );
    }
    NOTIFICATION_SCAN_VISITS.with(|visits| assert_eq!(visits.get(), (8, 8)));
    let notifications = completion_guard_notifications(&mut server, &control);
    assert_eq!(
        notifications,
        vec![
            protocol::SemanticNotificationKind::NeedsAttention,
            protocol::SemanticNotificationKind::Finished
        ]
    );
    // Observational dispatch after the transition must not send it again.
    for method in observational_methods(&public_id) {
        assert!(request(&mut server, method).get("result").is_some());
    }
    assert!(completion_guard_notifications(&mut server, &control).is_empty());
}
