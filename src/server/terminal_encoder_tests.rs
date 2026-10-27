use super::*;
use crate::cell_damage::{with_test_backend, Backend};
use crate::protocol::terminal_cells::{self, DecodedFrame};
use crate::protocol::{RenderEncoding, ServerMessage};
use crate::server::render_stream::{ClientRenderState, PreparedRender};

fn frame(lines: &[&str]) -> FrameData {
    let buffer = ratatui::buffer::Buffer::with_lines(lines.iter().copied());
    FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[])
}

fn unsupported_frame(lines: &[&str]) -> FrameData {
    let mut frame = frame(lines);
    frame.cells[0].skip = true;
    frame
}

fn cells_record(prepared: &PreparedRender) -> DecodedFrame {
    let ServerMessage::EndpointControl { kind, data } = prepared.message() else {
        panic!("expected a terminal cells control");
    };
    assert_eq!(kind, terminal_cells::FRAME_KIND);
    let record = terminal_cells::decode_frame_message(data).unwrap();
    terminal_cells::decode(&record).unwrap()
}

fn ansi_frame(prepared: &PreparedRender) -> &crate::protocol::TerminalFrame {
    let ServerMessage::Terminal(frame) = prepared.message() else {
        panic!("expected an ANSI terminal frame");
    };
    frame
}

fn cells_state() -> ClientRenderState {
    let mut state = ClientRenderState::new(RenderEncoding::TerminalAnsi);
    assert!(state.enable_terminal_cells());
    state
}

#[test]
fn ansi_connections_encode_exactly_as_before() {
    let first = frame(&["hello", "world"]);
    let second = frame(&["hellO", "world"]);
    let mut encoder = TerminalEncoder::ansi();
    let mut reference = BlitEncoder::new();
    for next in [first, second] {
        let TerminalEncoded::Ansi(encoded) = encoder.encode(&next, false, 1, 0) else {
            panic!("ANSI connection produced cells");
        };
        let expected = reference.encode(&next, false);
        assert_eq!(encoded.bytes, expected.bytes);
        reference.commit(next.clone(), expected);
        encoder.commit(next, TerminalEncoded::Ansi(encoded));
    }
}

#[test]
fn cells_are_enabled_only_before_the_first_frame() {
    let mut encoder = TerminalEncoder::ansi();
    let first = frame(&["a"]);
    let encoded = encoder.encode(&first, false, 1, 0);
    encoder.commit(first, encoded);
    assert!(!encoder.enable_cells());
    assert!(matches!(encoder, TerminalEncoder::Ansi(_)));

    let mut fresh = TerminalEncoder::ansi();
    assert!(fresh.enable_cells());
    assert!(fresh.enable_cells(), "a repeated hello keeps cells");
}

#[test]
fn render_state_accepts_cells_only_without_a_sent_frame() {
    let mut state = ClientRenderState::new(RenderEncoding::TerminalAnsi);
    let prepared = state.prepare_frame(frame(&["a"])).unwrap();
    state.commit_sent_frame(prepared);
    assert!(!state.enable_terminal_cells());

    let mut semantic = ClientRenderState::new(RenderEncoding::SemanticFrame);
    assert!(!semantic.enable_terminal_cells());
}

#[test]
fn cells_stream_sends_full_then_patches_against_the_committed_sequence() {
    let mut state = cells_state();
    let first = state.prepare_frame(frame(&["one", "two"])).unwrap();
    let record = cells_record(&first);
    assert!(record.full);
    assert_eq!((record.seq, record.base_seq), (1, 0));
    state.commit_sent_frame(first);

    // Deferred: prepared but never sent, so it must not move the baseline.
    let deferred = state.prepare_frame(frame(&["onE", "two"])).unwrap();
    assert_eq!(cells_record(&deferred).seq, 2);
    drop(deferred);

    let second = state.prepare_frame(frame(&["oNE", "two"])).unwrap();
    let record = cells_record(&second);
    assert!(!record.full);
    assert_eq!((record.seq, record.base_seq), (2, 1));
    assert_eq!(record.spans.len(), 1);
    assert_eq!((record.spans[0].x, record.spans[0].cells.len()), (1, 2));
    state.commit_sent_frame(second);

    assert!(
        state.prepare_frame(frame(&["oNE", "two"])).is_none(),
        "unchanged frame"
    );
}

#[test]
fn repaint_and_reset_resend_full_cells_frames() {
    let mut state = cells_state();
    let first = state.prepare_frame(frame(&["one"])).unwrap();
    state.commit_sent_frame(first);

    state.request_repaint();
    let repaint = state.prepare_frame(frame(&["one"])).unwrap();
    let record = cells_record(&repaint);
    assert!(record.full);
    assert_eq!(record.seq, 2);
    state.commit_sent_frame(repaint);

    state.reset_baseline();
    let reset = state.prepare_frame(frame(&["one"])).unwrap();
    let record = cells_record(&reset);
    assert!(record.full, "a reset baseline stays on cells");
    assert_eq!(record.seq, 3);
}

#[test]
fn unsupported_frames_fall_back_to_a_full_ansi_repaint_for_good() {
    let mut state = cells_state();
    let first = state.prepare_frame(frame(&["one"])).unwrap();
    state.commit_sent_frame(first);

    let fallback = state.prepare_frame(unsupported_frame(&["two"])).unwrap();
    let terminal = ansi_frame(&fallback);
    assert!(terminal.full);
    assert_eq!(terminal.seq, 2);
    assert!(
        terminal.bytes.windows(4).any(|window| window == b"\x1b[2J"),
        "the fallback clears what the cells stream painted"
    );
    state.commit_sent_frame(fallback);
    assert!(
        !state.enable_terminal_cells(),
        "the connection stays on ANSI"
    );

    let next = state.prepare_frame(frame(&["six"])).unwrap();
    let terminal = ansi_frame(&next);
    assert!(
        !terminal.full,
        "ANSI diffs continue from the fallback repaint"
    );
    assert_eq!(terminal.seq, 3);
}

#[test]
fn an_uncommitted_fallback_keeps_the_cells_baseline() {
    let mut state = cells_state();
    let first = state.prepare_frame(frame(&["one"])).unwrap();
    state.commit_sent_frame(first);

    let fallback = state.prepare_frame(unsupported_frame(&["two"])).unwrap();
    assert!(matches!(fallback.message(), ServerMessage::Terminal(_)));
    drop(fallback);

    let next = state.prepare_frame(frame(&["six"])).unwrap();
    let record = cells_record(&next);
    assert!(!record.full);
    assert_eq!((record.seq, record.base_seq), (2, 1));
}

#[test]
fn retained_row_patches_use_the_connection_codec() {
    let mut state = cells_state();
    let first = state.prepare_frame(frame(&["one", "two"])).unwrap();
    state.commit_sent_frame(first);
    let rows = vec![crate::protocol::PaneSurfacePatchRow {
        x: 0,
        y: 1,
        cells: frame(&["T"]).cells,
    }];
    let patch = state.prepare_terminal_patch(rows, None).unwrap();
    let record = cells_record(&patch);
    assert!(!record.full);
    assert_eq!((record.seq, record.base_seq), (2, 1));
    assert_eq!((record.spans[0].y, record.spans[0].x), (1, 0));
    state.commit_sent_frame(patch);
    assert_eq!(state.last_terminal_frame().unwrap().cells[3].symbol, "T");
}

fn revisions(key: u64, rows: &[u64]) -> RowRevisions {
    RowRevisions {
        key,
        rows: rows.to_vec(),
    }
}

#[test]
fn ansi_row_revisions_encode_exactly_like_the_blit_encoder() {
    with_test_backend(Some(Backend::Cpu), || {
        let frames = [
            (frame(&["one", "two"]), revisions(7, &[1, 1])),
            (frame(&["one", "TWO"]), revisions(7, &[1, 2])),
        ];
        let mut encoder = TerminalEncoder::ansi();
        let mut reference = BlitEncoder::new();
        for (seq, (next, rows)) in (1..).zip(frames) {
            let TerminalEncoded::Ansi(encoded) =
                encoder.encode_with_row_revisions(&next, false, seq, seq - 1, rows.clone())
            else {
                panic!("ANSI connection produced cells");
            };
            let expected = reference.encode_with_row_revisions(&next, false, rows);
            assert_eq!(encoded.bytes, expected.bytes);
            assert_eq!(encoded.full, expected.full);
            assert_eq!(encoded.has_row_damage(), seq > 1);
            reference.commit(next.clone(), expected);
            encoder.commit(next, TerminalEncoded::Ansi(encoded));
        }
    });
}

#[test]
fn cells_records_do_not_depend_on_row_revisions() {
    let mut with_revisions = TerminalEncoder::ansi();
    let mut without = TerminalEncoder::ansi();
    assert!(with_revisions.enable_cells() && without.enable_cells());
    let frames = [
        (frame(&["one", "two"]), revisions(7, &[1, 1])),
        // Wrong on purpose: row 1 changed but keeps its revision.
        (frame(&["one", "TWO"]), revisions(7, &[1, 1])),
    ];
    for (seq, (next, rows)) in (1..).zip(frames) {
        let encoded = with_revisions.encode_with_row_revisions(&next, false, seq, seq - 1, rows);
        let expected = without.encode(&next, false, seq, seq - 1);
        let (TerminalEncoded::Cells(cells), TerminalEncoded::Cells(expected_cells)) =
            (&encoded, &expected)
        else {
            panic!("cells connection produced ANSI");
        };
        assert_eq!(cells.bytes, expected_cells.bytes);
        with_revisions.commit(next.clone(), encoded);
        without.commit(next, expected);
    }
}

#[test]
fn an_ansi_fallback_keeps_row_revisions_for_the_diffs_after_it() {
    with_test_backend(Some(Backend::Cpu), || {
        let mut encoder = TerminalEncoder::ansi();
        assert!(encoder.enable_cells());
        let first = frame(&["one", "two"]);
        let encoded = encoder.encode_with_row_revisions(&first, false, 1, 0, revisions(3, &[1, 1]));
        assert!(matches!(encoded, TerminalEncoded::Cells(_)));
        encoder.commit(first, encoded);

        let fallback = unsupported_frame(&["one", "two"]);
        let fallback_rows = revisions(3, &[2, 1]);
        let TerminalEncoded::Ansi(repaint) =
            encoder.encode_with_row_revisions(&fallback, false, 2, 1, fallback_rows.clone())
        else {
            panic!("expected the ANSI fallback");
        };
        assert!(repaint.full);
        assert!(repaint.bytes.windows(4).any(|window| window == b"\x1b[2J"));
        let mut reference = BlitEncoder::new();
        let expected = reference.encode_with_row_revisions(&fallback, false, fallback_rows);
        assert_eq!(repaint.bytes, expected.bytes);
        reference.commit(fallback.clone(), expected);
        encoder.commit(fallback.clone(), TerminalEncoded::Ansi(repaint));
        assert!(matches!(encoder, TerminalEncoder::Ansi(_)));

        // The revisions claim row 1 is unchanged although it changed, so the
        // bytes show whether the fallback commit kept them: with them, row
        // damage skips row 1; without them, the diff writes it.
        let next = unsupported_frame(&["one", "TWO"]);
        let next_rows = revisions(3, &[2, 1]);
        let TerminalEncoded::Ansi(diff) =
            encoder.encode_with_row_revisions(&next, false, 3, 2, next_rows.clone())
        else {
            panic!("the connection stays on ANSI");
        };
        let expected = reference.encode_with_row_revisions(&next, false, next_rows);
        assert!(!diff.full);
        assert_eq!(diff.bytes, expected.bytes);
        let mut plain = BlitEncoder::new();
        let baseline = plain.encode(&fallback, false);
        plain.commit(fallback, baseline);
        assert_ne!(diff.bytes, plain.encode(&next, false).bytes);
    });
}
