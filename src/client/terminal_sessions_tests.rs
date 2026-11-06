use super::*;
use crate::protocol::terminal_cells::{self, CellsWelcome};

fn welcome(codec: Option<&str>, error: Option<&str>) -> ServerMessage {
    terminal_cells::welcome_message(&CellsWelcome {
        codec: codec.map(str::to_owned),
        error: error.map(str::to_owned),
    })
    .unwrap()
}

fn terminal(seq: u64, full: bool, bytes: &[u8]) -> ServerMessage {
    ServerMessage::Terminal(TerminalFrame {
        seq,
        width: 3,
        height: 2,
        full,
        bytes: bytes.to_vec(),
    })
}

fn shutdown(reason: Option<&str>) -> ServerMessage {
    ServerMessage::ServerShutdown {
        reason: reason.map(str::to_owned),
    }
}

fn cells_record() -> Vec<u8> {
    let buffer = ratatui::buffer::Buffer::with_lines(["abc", "def"]);
    let frame = crate::protocol::FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);
    let encoder = terminal_cells::CellsEncoder::new();
    encoder.encode(&frame, false, 1, 0).unwrap().bytes
}

/// Feeds `messages` and returns stdout plus whether the stream closed.
fn run(
    output: TerminalSessionOutput,
    messages: Vec<ServerMessage>,
) -> (Vec<u8>, io::Result<OutputFlow>) {
    let mut writer = TerminalOutputWriter::new(output);
    let mut out = Vec::new();
    let mut flow = Ok(OutputFlow::Open);
    for message in messages {
        flow = writer.write(&mut out, message);
        if !matches!(flow, Ok(OutputFlow::Open)) {
            break;
        }
    }
    (out, flow)
}

/// Splits stdout into its first JSON line and the bytes after it.
fn stream_line(out: &[u8]) -> (serde_json::Value, &[u8]) {
    let end = out
        .iter()
        .position(|&byte| byte == b'\n')
        .expect("stream line");
    (
        serde_json::from_slice(&out[..end]).unwrap(),
        &out[end + 1..],
    )
}

fn records(mut bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut records = Vec::new();
    while !bytes.is_empty() {
        let len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        records.push((bytes[4], bytes[5..5 + len].to_vec()));
        bytes = &bytes[5 + len..];
    }
    records
}

fn json_lines(bytes: &[u8]) -> Vec<serde_json::Value> {
    bytes
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

#[test]
fn ansi_output_is_unchanged_json_lines() {
    let (out, flow) = run(
        TerminalSessionOutput::Ansi,
        vec![terminal(1, true, b"hi"), shutdown(Some("bye"))],
    );
    assert_eq!(flow.unwrap(), OutputFlow::Closed);
    let lines = json_lines(&out);
    assert_eq!(
        lines,
        [
            serde_json::json!({
                "type": "terminal.frame",
                "seq": 1,
                "encoding": "ansi",
                "width": 3,
                "height": 2,
                "full": true,
                "bytes": "aGk=",
            }),
            serde_json::json!({"type": "terminal.closed", "reason": "bye"}),
        ]
    );
}

#[test]
fn accepted_cells_stream_writes_records_verbatim() {
    let record = cells_record();
    let (out, flow) = run(
        TerminalSessionOutput::Cells,
        vec![
            welcome(Some(terminal_cells::CODEC), None),
            terminal_cells::frame_message(&record),
            shutdown(Some("server stopping")),
        ],
    );
    assert_eq!(flow.unwrap(), OutputFlow::Closed);
    let (line, rest) = stream_line(&out);
    assert_eq!(
        line,
        serde_json::json!({
            "type": "terminal.stream",
            "encoding": "cells",
            "codec": terminal_cells::CODEC,
        })
    );
    let records = records(rest);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0], (RECORD_CELLS, record));
    assert_eq!(
        records[1],
        (
            RECORD_CLOSED,
            [&[1u8][..], &b"server stopping"[..]].concat()
        )
    );
}

#[test]
fn ansi_fallback_frames_become_ansi_records() {
    let (out, _) = run(
        TerminalSessionOutput::Cells,
        vec![
            welcome(Some(terminal_cells::CODEC), None),
            terminal(7, true, b"\x1b[2Jx"),
            shutdown(None),
        ],
    );
    let (_, rest) = stream_line(&out);
    let records = records(rest);
    let mut expected = 7u64.to_le_bytes().to_vec();
    expected.extend_from_slice(&3u16.to_le_bytes());
    expected.extend_from_slice(&2u16.to_le_bytes());
    expected.push(1);
    expected.extend_from_slice(b"\x1b[2Jx");
    assert_eq!(records[0], (RECORD_ANSI, expected));
    assert_eq!(records[1], (RECORD_CLOSED, vec![0]));
}

#[test]
fn refused_cells_continue_as_ansi_json_lines() {
    let (out, _) = run(
        TerminalSessionOutput::Cells,
        vec![
            welcome(None, Some("no supported terminal cells codec")),
            terminal(1, true, b"hi"),
        ],
    );
    let lines = json_lines(&out);
    assert_eq!(
        lines[0],
        serde_json::json!({
            "type": "terminal.stream",
            "encoding": "ansi",
            "reason": "no supported terminal cells codec",
        })
    );
    assert_eq!(lines[1]["type"], "terminal.frame");
    assert_eq!(lines.len(), 2);
}

#[test]
fn a_server_without_cells_streams_ansi_after_the_stream_line() {
    let (out, _) = run(
        TerminalSessionOutput::Cells,
        vec![terminal(1, true, b"hi"), terminal(2, false, b"!")],
    );
    let lines = json_lines(&out);
    assert_eq!(lines[0]["type"], "terminal.stream");
    assert_eq!(lines[0]["encoding"], "ansi");
    assert_eq!(lines[1]["seq"], 1);
    assert_eq!(lines[2]["seq"], 2);

    let (out, flow) = run(TerminalSessionOutput::Cells, vec![shutdown(Some("gone"))]);
    assert_eq!(flow.unwrap(), OutputFlow::Closed);
    let lines = json_lines(&out);
    assert_eq!(lines[0]["encoding"], "ansi");
    assert_eq!(
        lines[1],
        serde_json::json!({"type": "terminal.closed", "reason": "gone"})
    );
}

#[test]
fn cells_frames_without_an_accepted_welcome_are_errors() {
    let frame = terminal_cells::frame_message(&cells_record());
    let (out, flow) = run(TerminalSessionOutput::Cells, vec![frame.clone()]);
    assert!(flow.is_err());
    assert!(out.is_empty(), "nothing reaches stdout before the welcome");

    let (_, flow) = run(
        TerminalSessionOutput::Cells,
        vec![welcome(None, Some("refused")), frame.clone()],
    );
    assert!(flow.is_err());

    let (_, flow) = run(TerminalSessionOutput::Ansi, vec![frame]);
    assert!(flow.is_err());
}

#[test]
fn oversized_or_malformed_cells_frames_are_errors() {
    let oversized = ServerMessage::EndpointControl {
        kind: terminal_cells::FRAME_KIND.to_owned(),
        data: "A".repeat(terminal_cells::MAX_RECORD_BYTES / 3 * 4 + 8),
    };
    let malformed = ServerMessage::EndpointControl {
        kind: terminal_cells::FRAME_KIND.to_owned(),
        data: "not base64!".to_owned(),
    };
    for frame in [oversized, malformed] {
        let (out, flow) = run(
            TerminalSessionOutput::Cells,
            vec![welcome(Some(terminal_cells::CODEC), None), frame],
        );
        assert!(flow.is_err());
        let (_, rest) = stream_line(&out);
        assert!(rest.is_empty(), "no partial record is written");
    }
}

#[test]
fn unrelated_messages_before_the_welcome_are_ignored() {
    let (out, flow) = run(
        TerminalSessionOutput::Cells,
        vec![
            ServerMessage::EndpointControl {
                kind: "future.kind".to_owned(),
                data: String::new(),
            },
            welcome(Some(terminal_cells::CODEC), None),
        ],
    );
    assert_eq!(flow.unwrap(), OutputFlow::Open);
    let (line, rest) = stream_line(&out);
    assert_eq!(line["encoding"], "cells");
    assert!(rest.is_empty());
}

fn framed(messages: &[ServerMessage]) -> io::Cursor<Vec<u8>> {
    let mut bytes = Vec::new();
    for message in messages {
        protocol::write_message(&mut bytes, message).unwrap();
    }
    io::Cursor::new(bytes)
}

fn admission_answer(admitted: bool, code: Option<&str>, message: Option<&str>) -> ServerMessage {
    protocol::terminal_launch_guard::answer_message(&AdmissionAnswer {
        admitted,
        code: code.map(str::to_owned),
        message: message.map(str::to_owned),
    })
    .unwrap()
}

#[test]
fn an_accepted_admission_keeps_what_arrived_ahead_of_it_in_order() {
    let early = read_guarded_admission_answer(&mut framed(&[
        welcome(Some(terminal_cells::CODEC), None),
        admission_answer(true, None, None),
        terminal(1, true, b"after"),
    ]))
    .unwrap();
    assert_eq!(early.len(), 1);
    assert!(matches!(
        &early[0],
        ServerMessage::EndpointControl { kind, .. } if kind == terminal_cells::WELCOME_KIND
    ));
}

#[test]
fn a_refused_admission_carries_the_servers_typed_error() {
    let error = read_guarded_admission_answer(&mut framed(&[admission_answer(
        false,
        Some("terminal_launch_mismatch"),
        Some("terminal t runs launch a"),
    )]))
    .unwrap_err();
    assert_eq!(error.code, "terminal_launch_mismatch");
    assert_eq!(error.message, "terminal t runs launch a");
}

#[test]
fn a_connection_closed_without_an_answer_is_not_admitted() {
    let shutdown_error = read_guarded_admission_answer(&mut framed(&[shutdown(Some(
        "terminal attach failed: terminal t already has an attached client; retry with --takeover",
    ))]))
    .unwrap_err();
    assert_eq!(shutdown_error.code, ADMISSION_FAILED_CODE);
    assert!(shutdown_error.message.contains("--takeover"));

    let closed = read_guarded_admission_answer(&mut framed(&[])).unwrap_err();
    assert_eq!(closed.code, ADMISSION_FAILED_CODE);

    let invalid = read_guarded_admission_answer(&mut framed(&[ServerMessage::EndpointControl {
        kind: protocol::terminal_launch_guard::ANSWER_KIND.into(),
        data: "not json".into(),
    }]))
    .unwrap_err();
    assert_eq!(invalid.code, ADMISSION_FAILED_CODE);
}

#[test]
fn a_server_that_never_answers_is_not_admitted() {
    // What a read timeout on the socket reports, from a server that ignored
    // the unknown control and attached nothing.
    struct TimedOut;
    impl io::Read for TimedOut {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::TimedOut.into())
        }
    }
    let error = read_guarded_admission_answer(&mut TimedOut).unwrap_err();
    assert_eq!(error.code, ADMISSION_UNANSWERED_CODE);
}
