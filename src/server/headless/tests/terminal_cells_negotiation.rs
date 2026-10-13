use super::*;
use crate::protocol::terminal_cells::{self, CellsHello, CellsWelcome};

fn connect(
    server: &mut HeadlessServer,
    client_id: u64,
) -> (
    std::sync::mpsc::Receiver<Vec<u8>>,
    std::sync::mpsc::Receiver<Vec<u8>>,
) {
    let (writer, control_rx, render_rx) = test_client_writer();
    assert!(!server.handle_server_event(ServerEvent::ClientConnected {
        client_id,
        cols: 80,
        rows: 24,
        cell_width_px: 0,
        cell_height_px: 0,
        pixel_mouse: false,
        writer,
    }));
    (control_rx, render_rx)
}

fn hello(server: &mut HeadlessServer, client_id: u64, data: &str) {
    assert!(
        !server.handle_server_event(ServerEvent::TerminalCellsHello {
            client_id,
            data: data.to_owned(),
        })
    );
}

fn current_hello() -> String {
    serde_json::to_string(&CellsHello::current()).unwrap()
}

fn observe(server: &mut HeadlessServer, client_id: u64, target: &str) {
    assert!(
        server.handle_server_event(ServerEvent::ClientObserveTerminal {
            client_id,
            target: target.to_owned(),
        })
    );
}

/// The welcome among the control messages written so far.
fn welcome(control_rx: &std::sync::mpsc::Receiver<Vec<u8>>) -> CellsWelcome {
    let welcomes = control_rx
        .try_iter()
        .filter_map(|bytes| match read_server_message(bytes) {
            ServerMessage::EndpointControl { kind, data }
                if kind == terminal_cells::WELCOME_KIND =>
            {
                Some(serde_json::from_str::<CellsWelcome>(&data).expect("welcome json"))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(welcomes.len(), 1, "exactly one welcome per hello");
    welcomes.into_iter().next().unwrap()
}

#[test]
fn hello_before_attach_is_welcomed_before_any_frame_and_streams_cells() {
    with_terminal_session_test_server(|server, _, target, _| {
        let (control_rx, render_rx) = connect(server, 7);
        hello(server, 7, &current_hello());
        // The welcome is queued while the connection is still pending, so no
        // frame can exist yet; the writer also drains control before render.
        assert!(render_rx.try_recv().is_err());
        assert_eq!(
            welcome(&control_rx),
            CellsWelcome {
                codec: Some(terminal_cells::CODEC.to_owned()),
                error: None,
            }
        );

        observe(server, 7, &target);
        server.render_and_stream();
        let ServerMessage::EndpointControl { kind, data } =
            read_server_message(render_rx.recv().expect("first frame"))
        else {
            panic!("expected a terminal cells frame");
        };
        assert_eq!(kind, terminal_cells::FRAME_KIND);
        let record = terminal_cells::decode_frame_message(&data).expect("frame payload");
        let frame = terminal_cells::decode(&record).expect("cells record");
        assert!(frame.full);
        assert_eq!((frame.seq, frame.base_seq), (1, 0));
    });
}

#[test]
fn hello_after_attach_is_refused_and_the_stream_stays_ansi() {
    with_terminal_session_test_server(|server, _, target, _| {
        let (control_rx, render_rx) = connect(server, 7);
        observe(server, 7, &target);
        server.render_and_stream();
        assert!(matches!(
            read_server_message(render_rx.recv().expect("ANSI baseline")),
            ServerMessage::Terminal(_)
        ));
        let _ = control_rx.try_iter().count();

        hello(server, 7, &current_hello());
        let refused = welcome(&control_rx);
        assert_eq!(refused.codec, None);
        assert_eq!(
            refused.error.as_deref(),
            Some("terminal cells must be requested before attaching")
        );
        assert!(!server
            .clients
            .get_mut(&7)
            .expect("client")
            .render_state
            .enable_terminal_cells());
    });
}

#[test]
fn hello_without_a_known_codec_is_refused_and_frames_stay_ansi() {
    for data in [
        "not json",
        r#"{"codecs":["herdr.terminal.cells.v9"]}"#,
        "{}",
    ] {
        with_terminal_session_test_server(|server, _, target, _| {
            let (control_rx, render_rx) = connect(server, 7);
            hello(server, 7, data);
            let refused = welcome(&control_rx);
            assert_eq!(refused.codec, None, "{data}");
            assert_eq!(
                refused.error.as_deref(),
                Some("no supported terminal cells codec")
            );

            observe(server, 7, &target);
            server.render_and_stream();
            assert!(
                matches!(
                    read_server_message(render_rx.recv().expect("frame")),
                    ServerMessage::Terminal(_)
                ),
                "{data}"
            );
        });
    }
}

#[test]
fn a_repeated_hello_before_attach_is_welcomed_again() {
    with_terminal_session_test_server(|server, _, _, _| {
        let (control_rx, _render_rx) = connect(server, 7);
        hello(server, 7, &current_hello());
        assert!(welcome(&control_rx).codec.is_some());
        hello(server, 7, &current_hello());
        assert!(welcome(&control_rx).codec.is_some());
    });
}

#[test]
fn hello_for_an_unknown_client_is_ignored() {
    let mut server = test_headless_server();
    hello(&mut server, 99, &current_hello());
    assert!(server.clients.is_empty());
}
