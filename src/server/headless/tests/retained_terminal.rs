use super::*;

fn connect_direct(
    server: &mut HeadlessServer,
    target: &str,
    client_id: u64,
    cols: u16,
    rows: u16,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    connect_terminal(server, target, client_id, cols, rows, true)
}

fn connect_terminal(
    server: &mut HeadlessServer,
    target: &str,
    client_id: u64,
    cols: u16,
    rows: u16,
    control: bool,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (writer, _control, render) = test_client_writer();
    assert!(!server.handle_server_event(ServerEvent::ClientConnected {
        client_id,
        cols,
        rows,
        cell_width_px: 0,
        cell_height_px: 0,
        pixel_mouse: false,
        writer,
    }));
    assert!(server.handle_server_event(if control {
        ServerEvent::ClientControlTerminal {
            client_id,
            target: target.to_owned(),
            takeover: false,
        }
    } else {
        ServerEvent::ClientObserveTerminal {
            client_id,
            target: target.to_owned(),
        }
    }));
    render
}

fn terminal_frame(render: &std::sync::mpsc::Receiver<Vec<u8>>) -> protocol::TerminalFrame {
    let ServerMessage::Terminal(frame) = read_server_message(
        render
            .recv_timeout(Duration::from_secs(1))
            .expect("terminal frame"),
    ) else {
        panic!("expected terminal ANSI frame");
    };
    frame
}

fn assert_dense_parity(server: &HeadlessServer, target: &str, client_id: u64) {
    let retained = server.clients[&client_id]
        .render_state
        .last_terminal_frame()
        .unwrap();
    let area = Rect::new(0, 0, retained.width, retained.height);
    let runtime = server.runtime_for_terminal_id_string(target).unwrap();
    let (buffer, cursor) = crate::server::render_stream::render_terminal_virtual(runtime, area);
    let expected = FrameData::from_ratatui_buffer_with_hyperlinks(
        &buffer,
        cursor,
        &runtime.visible_hyperlinks(area),
    );
    assert_eq!(
        retained, &expected,
        "retained ANSI state must match the complete renderer"
    );
}

#[test]
fn portal_direct_attachment_does_not_disable_retained_shell_updates() {
    with_terminal_session_test_server(|server, id, target, _| {
        server.app.state.active = Some(0);
        server.app.state.mode = crate::app::Mode::Terminal;
        let pane_id = server.app.state.workspaces[0].tabs[0].root_pane;
        let (_control, shell) = connect_test_shell(server, 7, 360, 100);
        let direct = connect_direct(server, &target, 8, 350, 96);
        server.render_and_stream();
        let initial_shell = recv_pane_surface(&shell, "initial shell");
        let initial_direct = terminal_frame(&direct);
        assert!(initial_direct.full);

        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .test_process_pty_bytes(b"\rMIXED_CLIENT");
        assert!(
            server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])),
            "a Portal-style direct attachment must not force the shell back to full rendering"
        );
        let shell_patch = recv_pane_surface_patch(&shell, "retained shell patch");
        assert_eq!(
            shell_patch.base_surface_revision,
            initial_shell.surface_revision
        );
        let direct_patch = terminal_frame(&direct);
        assert!(!direct_patch.full);
        assert_eq!(direct_patch.seq, initial_direct.seq + 1);
        assert!(direct_patch
            .bytes
            .windows(b"MIXED_CLIENT".len())
            .any(|w| w == b"MIXED_CLIENT"));
        assert_dense_parity(server, &target, 8);
    });
}

#[test]
fn large_direct_attachment_reuses_its_committed_ansi_grid() {
    with_terminal_session_test_server(|server, id, target, _| {
        let pane_id = server.app.state.workspaces[0].tabs[0].root_pane;
        let direct = connect_direct(server, &target, 8, 360, 100);
        server.render_and_stream();
        let initial = terminal_frame(&direct);
        assert!(initial.full);
        let initial_grid = server.clients[&8]
            .render_state
            .last_terminal_frame()
            .unwrap()
            .cells
            .as_ptr();
        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .test_process_pty_bytes(b"\x1b[50;180HX");
        assert!(
            server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])),
            "a direct attachment must render terminal dirty rows without rebuilding a dense frame"
        );
        let patch = terminal_frame(&direct);
        assert!(!patch.full);
        assert_eq!((patch.width, patch.height), (360, 100));
        assert_eq!(patch.seq, initial.seq + 1);
        assert!(patch.bytes.contains(&b'X'));
        assert!(
            patch.bytes.len() < 512,
            "one cell change must remain bounded"
        );
        assert_eq!(
            server.clients[&8]
                .render_state
                .last_terminal_frame()
                .unwrap()
                .cells
                .as_ptr(),
            initial_grid,
            "dirty updates must retain the allocated dense baseline"
        );
        assert_dense_parity(server, &target, 8);
    });
}

#[test]
fn direct_controller_and_smaller_observer_share_one_dirty_collection() {
    with_terminal_session_test_server(|server, id, target, _| {
        let pane_id = server.app.state.workspaces[0].tabs[0].root_pane;
        let controller = connect_direct(server, &target, 8, 360, 100);
        let observer = connect_terminal(server, &target, 9, 80, 24, false);
        server.render_and_stream();
        let _ = terminal_frame(&controller);
        let _ = terminal_frame(&observer);
        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .test_process_pty_bytes(b"\x1b[2;3HMULTI\x1b[80;250HDEEP");
        assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])));
        let large = terminal_frame(&controller);
        let small = terminal_frame(&observer);
        assert!(!large.full && !small.full);
        assert_eq!((large.width, large.height), (360, 100));
        assert_eq!((small.width, small.height), (80, 24));
        assert_dense_parity(server, &target, 8);
        assert_dense_parity(server, &target, 9);
    });
}

#[test]
fn retained_direct_cursor_clear_wide_cells_and_scroll_match_dense_frames() {
    with_terminal_session_test_server(|server, id, target, _| {
        server.app.terminal_runtimes.insert(
            id.clone(),
            crate::terminal::TerminalRuntime::test_with_scrollback_bytes(80, 24, 65536, b""),
        );
        let pane_id = server.app.state.workspaces[0].tabs[0].root_pane;
        let direct = connect_direct(server, &target, 8, 80, 24);
        server.render_and_stream();
        let _ = terminal_frame(&direct);
        for bytes in [
            "\x1b[3;4H界Z".as_bytes(),
            b"\x1b[3;4Hx",
            b"\x1b[3;6H\x1b[5 q\x1b[?25l",
            b"\x1b[?25h\x1b[2J\x1b[HTEXT_AFTER_CLEAR",
        ] {
            server
                .app
                .terminal_runtimes
                .get(&id)
                .unwrap()
                .test_process_pty_bytes(bytes);
            assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])));
            assert!(!terminal_frame(&direct).full);
            assert_dense_parity(server, &target, 8);
        }
        let history = (0..80)
            .map(|n| format!("line {n:03}\r\n"))
            .collect::<String>();
        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .test_process_pty_bytes(history.as_bytes());
        server.render_and_stream();
        let _ = terminal_frame(&direct);
        server.app.terminal_runtimes.get(&id).unwrap().scroll_up(8);
        assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])));
        assert!(!terminal_frame(&direct).full);
        assert!(server.clients[&8]
            .render_state
            .last_terminal_frame()
            .unwrap()
            .cursor
            .as_ref()
            .is_none_or(|cursor| !cursor.visible));
        assert_dense_parity(server, &target, 8);
        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .scroll_reset();
        assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])));
        let _ = terminal_frame(&direct);
        assert_dense_parity(server, &target, 8);
    });
}

#[test]
fn backpressured_direct_recipient_keeps_baseline_and_recovers_all_changes() {
    with_terminal_session_test_server(|server, id, target, _| {
        server.app.state.active = Some(0);
        let pane_id = server.app.state.workspaces[0].tabs[0].root_pane;
        let (_control, shell) = connect_test_shell(server, 7, 80, 24);
        let direct = connect_direct(server, &target, 8, 70, 20);
        server.render_and_stream();
        let _ = recv_pane_surface(&shell, "shell baseline");
        let initial = terminal_frame(&direct);
        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .test_process_pty_bytes(b"\rONE");
        assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])));
        let _ = recv_pane_surface_patch(&shell, "first shell patch");
        let before = server.clients[&8]
            .render_state
            .last_terminal_frame()
            .unwrap()
            .clone();
        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .test_process_pty_bytes(b"\rTWO");
        assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])));
        let _ = recv_pane_surface_patch(&shell, "second shell patch");
        assert_eq!(server.clients[&8].deferred_render(), DeferredRender::Full);
        assert_eq!(
            server.clients[&8].render_state.last_terminal_frame(),
            Some(&before)
        );
        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .test_process_pty_bytes(b"\x1b[2;1HTHREE");
        assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])));
        let _ = recv_pane_surface_patch(&shell, "third shell patch");
        assert_eq!(terminal_frame(&direct).seq, initial.seq + 1);
        assert!(server.handle_server_event(ServerEvent::ClientWriterDrained { client_id: 8 }));
        server.render_and_stream();
        assert_eq!(terminal_frame(&direct).seq, initial.seq + 2);
        assert_dense_parity(server, &target, 8);
    });
}

#[test]
fn retained_direct_waits_for_synchronized_output_and_hyperlinks_use_full_recovery() {
    with_terminal_session_test_server(|server, id, target, _| {
        let pane_id = server.app.state.workspaces[0].tabs[0].root_pane;
        let direct = connect_direct(server, &target, 8, 80, 24);
        server.render_and_stream();
        let _ = terminal_frame(&direct);
        let before = server.clients[&8]
            .render_state
            .last_terminal_frame()
            .unwrap()
            .clone();
        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .test_process_pty_bytes(b"\x1b[?2026hPARTIAL");
        assert!(!server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])));
        assert!(direct.try_recv().is_err());
        assert_eq!(
            server.clients[&8].render_state.last_terminal_frame(),
            Some(&before)
        );
        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .test_process_pty_bytes(b"\rCOMPLETE\x1b[?2026l");
        assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])));
        let _ = terminal_frame(&direct);
        assert_dense_parity(server, &target, 8);
        let before = server.clients[&8]
            .render_state
            .last_terminal_frame()
            .unwrap()
            .clone();
        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .test_process_pty_bytes(b"\x1b]8;;https://example.test\x1b\\LINK\x1b]8;;\x1b\\");
        assert!(!server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])));
        assert_eq!(
            server.clients[&8].render_state.last_terminal_frame(),
            Some(&before)
        );
        server.render_and_stream();
        let _ = terminal_frame(&direct);
        assert!(!server.clients[&8]
            .render_state
            .last_terminal_frame()
            .unwrap()
            .hyperlinks
            .is_empty());
    });
}

#[test]
fn direct_input_and_scroll_mark_only_the_target_pane_for_rendering() {
    with_terminal_session_test_server(|server, id, target, _| {
        let history = (0..80)
            .map(|n| format!("line {n:03}\r\n"))
            .collect::<String>();
        let (runtime, mut input) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80,
                24,
                65536,
                history.as_bytes(),
                4,
            );
        server.app.terminal_runtimes.insert(id.clone(), runtime);
        let pane_id = server.app.state.workspaces[0].tabs[0].root_pane;
        let direct = connect_direct(server, &target, 8, 80, 24);
        server.render_and_stream();
        let _ = terminal_frame(&direct);
        server.app.render_dirty.take();
        assert_eq!(
            server.handle_server_event_with_render_impact(ServerEvent::ClientInput {
                client_id: 8,
                data: b"x".to_vec(),
            }),
            RenderImpact::None
        );
        let request = server.app.render_dirty.take();
        assert!(!request.generic);
        assert_eq!(request.pty_sources, HashSet::from([pane_id]));
        assert_eq!(
            input
                .try_recv()
                .expect("input is delivered to its terminal"),
            Bytes::from_static(b"x")
        );
        server
            .app
            .terminal_runtimes
            .get(&id)
            .unwrap()
            .test_process_pty_bytes(b"echo");
        assert!(server.render_retained_pane_surface_and_stream(&request.pty_sources));
        let _ = terminal_frame(&direct);
        assert_eq!(
            server.handle_server_event_with_render_impact(ServerEvent::ClientAttachScroll {
                client_id: 8,
                source: AttachScrollSource::Wheel,
                direction: AttachScrollDirection::Up,
                lines: 3,
                column: Some(0),
                row: Some(0),
                modifiers: 0,
            }),
            RenderImpact::None
        );
        let request = server.app.render_dirty.take();
        assert!(!request.generic);
        assert_eq!(request.pty_sources, HashSet::from([pane_id]));
        assert!(server.render_retained_pane_surface_and_stream(&request.pty_sources));
        let _ = terminal_frame(&direct);
        assert_dense_parity(server, &target, 8);
        assert!(
            server
                .app
                .terminal_runtimes
                .get(&id)
                .unwrap()
                .scroll_metrics()
                .unwrap()
                .offset_from_bottom
                > 0
        );
        assert_eq!(
            server.handle_server_event_with_render_impact(ServerEvent::ClientInput {
                client_id: 8,
                data: b"y".to_vec(),
            }),
            RenderImpact::None
        );
        assert_eq!(input.try_recv().unwrap(), Bytes::from_static(b"y"));
        let request = server.app.render_dirty.take();
        assert!(server.render_retained_pane_surface_and_stream(&request.pty_sources));
        let _ = terminal_frame(&direct);
        assert_eq!(
            server
                .app
                .terminal_runtimes
                .get(&id)
                .unwrap()
                .scroll_metrics()
                .unwrap()
                .offset_from_bottom,
            0
        );
        assert_dense_parity(server, &target, 8);
    });
}
