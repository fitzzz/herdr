use super::*;

fn wheel(kind: protocol::ClientMouseKind) -> protocol::ClientPaneInputEvent {
    protocol::ClientPaneInputEvent::Mouse {
        kind,
        position: protocol::ClientMousePosition::Cell { column: 2, row: 2 },
        geometry: None,
        modifiers: 0,
        lines: 3,
    }
}

fn setup_scrolling_shell(
    cols: u16,
    rows: u16,
) -> (
    HeadlessServer,
    crate::layout::PaneId,
    String,
    std::sync::mpsc::Receiver<Vec<u8>>,
    tokio::sync::mpsc::Receiver<Bytes>,
) {
    let mut server = test_headless_server();
    let mut workspace = crate::workspace::Workspace::test_new("shell-scroll");
    let pane_id = workspace.tabs[0].root_pane;
    let history = (0..300)
        .map(|n| format!("history row {n:03}\r\n"))
        .collect::<String>();
    let (runtime, input) = crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
        cols,
        rows,
        65536,
        history.as_bytes(),
        8,
    );
    workspace.insert_test_runtime(pane_id, runtime);
    server.app.state.workspaces = vec![workspace];
    server.app.state.active = Some(0);
    server.app.state.mode = crate::app::Mode::Terminal;
    let public_pane = server.app.public_pane_id(0, pane_id).unwrap();
    let (_control, render) = connect_test_shell(&mut server, 7, cols, rows);
    server.render_and_stream();
    let _ = recv_pane_surface(&render, "shell baseline");
    server.app.render_dirty.take();
    (server, pane_id, public_pane, render, input)
}

#[tokio::test]
async fn shell_scroll_without_layout_change_retains_rows_at_small_and_large_geometry() {
    for (cols, rows) in [(120, 40), (483, 128)] {
        let (mut server, pane_id, public_pane, render, mut input) =
            setup_scrolling_shell(cols, rows);
        let controller = server.tab_geometry_controllers.clone();
        let page_up = client_page_key(
            protocol::ClientKeyCode::PageUp,
            crossterm::event::KeyModifiers::empty(),
            protocol::ClientKeyKind::Press,
        );
        for event in [
            wheel(protocol::ClientMouseKind::ScrollUp),
            page_up,
            protocol::ClientPaneInputEvent::TextCommit("x".into()),
        ] {
            let impact =
                server.handle_server_event_with_render_impact(ServerEvent::ClientShellPaneInput {
                    client_id: 7,
                    pane_id: public_pane.clone(),
                    events: vec![event],
                });
            assert_eq!(
                impact,
                RenderImpact::None,
                "a viewport change with stable shell geometry must not force the complete renderer"
            );
            assert_eq!(server.foreground_client_id, Some(7));
            assert_eq!(server.tab_geometry_controllers, controller);
            let request = server.app.render_dirty.take();
            assert!(!request.generic);
            assert_eq!(request.pty_sources, HashSet::from([pane_id]));
            assert!(server.render_retained_pane_surface_and_stream(&request.pty_sources));
            let _ = recv_pane_surface_patch(&render, "scroll row patch");
            let retained = server.clients[&7]
                .render_state
                .last_pane_surface()
                .unwrap()
                .clone();
            server
                .clients
                .get_mut(&7)
                .unwrap()
                .render_state
                .request_repaint();
            server.render_and_stream();
            let complete = recv_pane_surface(&render, "complete renderer parity");
            assert_eq!(retained.frame, complete.frame);
            assert_eq!(retained.panes, complete.panes);
        }
        assert_eq!(
            input.try_recv().expect("text forwarded"),
            Bytes::from_static(b"x")
        );
        assert!(
            input.try_recv().is_err(),
            "host scroll must not generate child input"
        );
        shutdown_test_runtimes(&mut server);
    }
}

#[tokio::test]
async fn shell_scroll_that_promotes_a_client_still_requires_complete_rendering() {
    let (mut server, _pane_id, public_pane, render, _input) = setup_scrolling_shell(120, 40);
    server.foreground_client_id = None;
    let impact = server.handle_server_event_with_render_impact(ServerEvent::ClientShellPaneInput {
        client_id: 7,
        pane_id: public_pane,
        events: vec![wheel(protocol::ClientMouseKind::ScrollUp)],
    });
    assert_eq!(impact, RenderImpact::Full);
    assert_eq!(server.foreground_client_id, Some(7));
    assert!(
        render.try_recv().is_err(),
        "invalidation must precede presentation"
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn shell_scroll_that_claims_another_geometry_still_requires_complete_rendering() {
    let (mut server, pane_id, public_pane, render, _input) = setup_scrolling_shell(483, 128);
    let (_control, small_render) = connect_test_shell(&mut server, 8, 120, 40);
    server.render_and_stream();
    let _ = recv_pane_surface(&small_render, "small peer baseline");
    // The large surface may be unchanged, so consume it only when present.
    while render.try_recv().is_ok() {}
    let before = server
        .app
        .state
        .runtime_for_pane_in_workspace(&server.app.terminal_runtimes, 0, pane_id)
        .unwrap()
        .current_size();
    server.foreground_client_id = Some(8);
    let impact = server.handle_server_event_with_render_impact(ServerEvent::ClientShellPaneInput {
        client_id: 8,
        pane_id: public_pane,
        events: vec![wheel(protocol::ClientMouseKind::ScrollUp)],
    });
    let after = server
        .app
        .state
        .runtime_for_pane_in_workspace(&server.app.terminal_runtimes, 0, pane_id)
        .unwrap()
        .current_size();
    assert_eq!(impact, RenderImpact::Full);
    assert_ne!(
        before, after,
        "the control case must actually change terminal dimensions"
    );
    assert_eq!(server.foreground_client_id, Some(8));
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn writer_recovery_semantic_viewport_change_retries_oversized_surface_without_pty_damage() {
    let (mut server, pane_id, _public_pane, frames, _input) = setup_scrolling_shell(80, 23);
    let baseline = server.clients[&7]
        .render_state
        .last_pane_surface()
        .unwrap()
        .clone();
    let cap = HeadlessServer::frame_server_message_with_max(
        &ServerMessage::PaneSurface(baseline.clone()),
        MAX_GRAPHICS_FRAME_SIZE,
    )
    .unwrap()
    .len()
        + 512;
    server.test_render_frame_limit = cap;
    let geometry = (
        server.clients[&7].terminal_size,
        server.app.state.workspaces[0].tabs[0].layout.focused(),
        server.app.state.workspaces[0].tabs[0].panes.len(),
    );
    let runtime = server
        .app
        .state
        .runtime_for_pane_in_workspace(&server.app.terminal_runtimes, 0, pane_id)
        .unwrap();
    let (rows, cols) = runtime.current_size();
    let mut dense = String::from("\x1b[2J\x1b[H");
    for row in 1..=rows {
        dense.push_str(&format!("\x1b[{row};1H{}", "é".repeat(usize::from(cols))));
    }
    runtime.test_process_pty_bytes(dense.as_bytes());
    server.clients.get_mut(&7).unwrap().defer_full_render();
    server.handle_server_event(ServerEvent::ClientWriterDrained { client_id: 7 });
    assert!(server.render_pending_work(false, false, &HashSet::new()));
    assert!(server.render_recoveries_waiting_for_change.contains_key(&7));
    assert_eq!(
        server.clients[&7].render_state.last_pane_surface(),
        Some(&baseline)
    );
    let attempts = server.render_recovery_attempts;
    server.app.render_dirty.take();
    for _ in 0..15 {
        assert!(!server.render_pending_work(false, false, &HashSet::new()));
    }
    assert_eq!(server.render_recovery_attempts, attempts);

    // Only the host viewport changes. There is no producer output or outer resize.
    server
        .app
        .state
        .runtime_for_pane_in_workspace(&server.app.terminal_runtimes, 0, pane_id)
        .unwrap()
        .scroll_up(50);
    assert_eq!(
        (
            server.clients[&7].terminal_size,
            server.app.state.workspaces[0].tabs[0].layout.focused(),
            server.app.state.workspaces[0].tabs[0].panes.len(),
        ),
        geometry
    );
    assert!(server.app.render_dirty.take().pty_sources.is_empty());
    assert!(server.render_pending_work(true, false, &HashSet::new()));
    let recovered = recv_pane_surface(&frames, "semantic viewport recovery");
    assert!(frame_text(&recovered.frame).contains("history row"));
    assert!(server.pending_render_recoveries.is_empty());
    assert!(server.render_recoveries_waiting_for_change.is_empty());
    shutdown_test_runtimes(&mut server);
}
