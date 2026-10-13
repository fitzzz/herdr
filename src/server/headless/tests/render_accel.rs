//! End to end: a headless server streaming a terminal to a direct ANSI
//! recipient with row damage (`HERDR_RENDER_ACCEL`) must show, after every
//! delivered frame, exactly what a fresh full render shows.

use super::*;
use crate::cell_damage::{test_damaged_frames, with_test_backend, Backend};

// Standalone Herdr disables autowrap; Portal's embedded libvterm keeps its
// default wrap policy.
const HOST_SETUPS: [&[u8]; 2] = [b"\x1b[?7l", b""];

/// PTY output covering text, colors, underline styles and colors, wide and
/// combining graphemes, hyperlinks (new, reused, retargeted), scrolling,
/// erasing, and cursor shape. Every step changes the screen.
const STEPS: &[&str] = &[
    "\rhello world",
    "\x1b[3;1H\x1b[31;44;1mRED\x1b[0m \x1b[4:3;58:2::255:0:0mcurly\x1b[0m \x1b[21mdouble\x1b[0m",
    "\x1b[3;20H\x1b[4:3;58:2::0:0:255mcurly\x1b[0m",
    "\x1b[5;1H界界界 💡 ｶﾞ e\u{301} 👩\u{200d}👩\u{200d}👧",
    "\x1b[5;2Hx",
    "\x1b[7;1H\x1b]8;;https://a.example\x1b\\LINK\x1b]8;;\x1b\\",
    "\x1b[9;5H\x1b]8;;https://a.example\x1b\\AGAIN\x1b]8;;\x1b\\",
    "\x1b[7;1H\x1b]8;;https://b.example\x1b\\LINK\x1b]8;;\x1b\\",
    "\x1b[12;1H\x1b[7mreverse\x1b[0m\x1b[5 q",
    "\x1b[40;1H\r\nscrolled one\r\nscrolled two\r\n",
    "\x1b[3;1H\x1b[2K",
    "\x1b[20;1H\x1b[J",
    "\x1b[1;1H\x1b[2J\x1b[H\x1b[32mfresh screen\x1b[0m",
    "\x1b[2;1Htail",
];

fn connect_direct(
    server: &mut HeadlessServer,
    target: &str,
    client_id: u64,
    cols: u16,
    rows: u16,
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
    assert!(
        server.handle_server_event(ServerEvent::ClientControlTerminal {
            client_id,
            target: target.to_owned(),
            takeover: false,
        })
    );
    render
}

fn replay_frame(
    render: &std::sync::mpsc::Receiver<Vec<u8>>,
    displays: &[crate::terminal::TerminalRuntime],
) -> protocol::TerminalFrame {
    let ServerMessage::Terminal(frame) = read_server_message(
        render
            .recv_timeout(Duration::from_secs(1))
            .expect("terminal frame"),
    ) else {
        panic!("expected terminal ANSI frame");
    };
    for display in displays {
        display.test_process_pty_bytes(&frame.bytes);
    }
    frame
}

fn display_cells(
    runtime: &crate::terminal::TerminalRuntime,
    area: Rect,
) -> Vec<crate::protocol::CellData> {
    let backend = ratatui::backend::TestBackend::new(area.width, area.height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let mut cells = Vec::new();
    terminal
        .draw(|frame| {
            runtime.render(frame, area, true);
            cells = frame
                .buffer_mut()
                .content
                .iter()
                .map(crate::protocol::CellData::from_ratatui_cell)
                .collect::<Vec<_>>();
        })
        .unwrap();
    cells
}

/// The display after the delivered frames equals a fresh terminal shown one
/// full redraw of the server's current screen.
fn assert_display_matches_full_render(
    server: &HeadlessServer,
    target: &str,
    display: &crate::terminal::TerminalRuntime,
    host_setup: &[u8],
    step: &str,
) {
    let (rows, cols) = display.current_size();
    let area = Rect::new(0, 0, cols, rows);
    let runtime = server.runtime_for_terminal_id_string(target).unwrap();
    let (buffer, cursor) = crate::server::render_stream::render_terminal_virtual(runtime, area);
    let dense = FrameData::from_ratatui_buffer_with_hyperlinks(
        &buffer,
        cursor,
        &runtime.visible_hyperlinks(area),
    );
    let full = crate::protocol::render_ansi::BlitEncoder::new().encode(&dense, true);
    let expected = crate::terminal::TerminalRuntime::test_with_screen_bytes(cols, rows, host_setup);
    expected.test_process_pty_bytes(&full.bytes);
    let actual_cells = display_cells(display, area);
    let expected_cells = display_cells(&expected, area);
    assert_eq!(actual_cells.len(), expected_cells.len());
    for (index, (actual, expected)) in actual_cells.iter().zip(&expected_cells).enumerate() {
        assert_eq!(
            actual,
            expected,
            "after {step:?}: display differs at ({}, {}), host wrap enabled: {}",
            index % usize::from(cols),
            index / usize::from(cols),
            host_setup.is_empty(),
        );
    }
}

fn stream_with_row_damage(backend: Backend) {
    with_test_backend(Some(backend), || {
        with_terminal_session_test_server(|server, id, target, _| {
            let (cols, rows) = (120, 40);
            let direct = connect_direct(server, &target, 8, cols, rows);
            let displays = HOST_SETUPS.map(|setup| {
                crate::terminal::TerminalRuntime::test_with_screen_bytes(cols, rows, setup)
            });
            server.render_and_stream();
            assert!(replay_frame(&direct, &displays).full);
            for step in STEPS {
                let damaged_before = test_damaged_frames();
                server
                    .app
                    .terminal_runtimes
                    .get(&id)
                    .unwrap()
                    .test_process_pty_bytes(step.as_bytes());
                server.render_and_stream();
                let frame = replay_frame(&direct, &displays);
                assert!(!frame.full, "after {step:?}: expected a diff frame");
                assert!(
                    test_damaged_frames() > damaged_before,
                    "after {step:?}: the frame was encoded without row damage"
                );
                for (display, setup) in displays.iter().zip(HOST_SETUPS) {
                    assert_display_matches_full_render(server, &target, display, setup, step);
                }
            }
        });
    });
}

#[test]
fn cpu_row_damage_streams_the_same_display_as_a_full_render() {
    stream_with_row_damage(Backend::Cpu);
}

/// Runs only with `HERDR_TEST_CUDA=1` on a machine with a CUDA device, where
/// a device that does not open fails the test.
#[cfg(feature = "cuda")]
#[test]
fn cuda_row_damage_streams_the_same_display_as_a_full_render() {
    if std::env::var("HERDR_TEST_CUDA").as_deref() != Ok("1") {
        return;
    }
    crate::cell_damage::open_cuda_for_tests().expect("HERDR_TEST_CUDA=1 needs a CUDA device");
    stream_with_row_damage(Backend::Cuda);
}
