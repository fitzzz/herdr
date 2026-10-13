//! Replays the recorded rich fixtures and builds, after every segment, the frame
//! the server sends a direct terminal client (render_terminal_virtual plus the
//! visible hyperlinks). Every such frame must take the terminal cells codec, not
//! the ANSI fallback, and the applied records must show exactly that frame.
use super::*;
use crate::protocol::render_ansi::{cell_width, BlitEncoder};
use crate::protocol::terminal_cells::{CellsEncoder, ReferenceGrid};
use crate::protocol::{CursorState, FrameData};
use std::collections::BTreeMap;

#[derive(serde::Deserialize)]
struct Fixture {
    width: u16,
    height: u16,
    stages: Vec<Stage>,
}

#[derive(serde::Deserialize)]
struct Stage {
    segments: Vec<String>,
}

#[derive(Debug, Default)]
struct Counts {
    frames: usize,
    full: usize,
    patches: usize,
    fallbacks: BTreeMap<&'static str, usize>,
    cells_record_bytes: usize,
    ansi_bytes: usize,
}

fn session_frame(pane: &GhosttyPaneTerminal, area: Rect) -> FrameData {
    let backend = ratatui::backend::TestBackend::new(area.width, area.height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| pane.render(frame, area, true))
        .unwrap();
    let cursor = pane.cursor_state().map(|cursor| CursorState {
        x: cursor.x,
        y: cursor.y,
        visible: cursor.visible,
        shape: cursor.shape,
    });
    FrameData::from_ratatui_buffer_with_hyperlinks(
        terminal.backend().buffer(),
        cursor,
        &pane.visible_hyperlinks(area),
    )
}

fn assert_grid_shows(grid: &ReferenceGrid, frame: &FrameData, context: &str) {
    for y in 0..frame.height {
        let mut covered = false;
        for x in 0..frame.width {
            let cell = &frame.cells[usize::from(y) * usize::from(frame.width) + usize::from(x)];
            let shown = grid.cell(x, y);
            if covered {
                assert_eq!(shown.width, 0, "{context} cell {x},{y}");
                covered = false;
                continue;
            }
            assert_eq!(
                (shown.text.as_str(), usize::from(shown.width)),
                (cell.symbol.as_str(), cell_width(cell)),
                "{context} cell {x},{y}"
            );
            covered = cell_width(cell) == 2;
        }
    }
}

fn replay(name: &str, fixture_json: &str) -> Vec<(bool, Counts)> {
    let fixture: Fixture = serde_json::from_str(fixture_json).unwrap();
    let area = Rect::new(0, 0, fixture.width, fixture.height);
    let mut results = Vec::new();
    // A native width one wider clips wide characters at the right edge.
    for clip_right_edge in [false, true] {
        let (tx, _rx) = mpsc::channel(4);
        let terminal = crate::ghostty::Terminal::new(
            fixture.width + u16::from(clip_right_edge),
            fixture.height,
            0,
        )
        .unwrap();
        let pane = GhosttyPaneTerminal::new(terminal, tx).unwrap();
        let mut cells = CellsEncoder::new();
        let mut ansi = BlitEncoder::new();
        let mut grid = ReferenceGrid::default();
        let mut seq = 0u64;
        let mut counts = Counts::default();
        for (stage_index, stage) in fixture.stages.iter().enumerate() {
            for (segment_index, segment) in stage.segments.iter().enumerate() {
                let synchronized = {
                    let mut core = pane.core.lock().unwrap();
                    core.terminal.write(segment.as_bytes());
                    core.decscusr_tracker.observe(segment.as_bytes());
                    core.cursor_settle_state = CursorPositionSettleState::default();
                    core.terminal
                        .mode_get(crate::ghostty::MODE_SYNCHRONIZED_OUTPUT)
                        .unwrap()
                };
                // The server does not render a terminal inside a synchronized update.
                if synchronized {
                    continue;
                }
                let frame = session_frame(&pane, area);
                if cells.is_current(&frame) {
                    continue;
                }
                counts.frames += 1;
                let blit = ansi.encode(&frame, false);
                counts.ansi_bytes += blit.bytes.len();
                ansi.commit(frame.clone(), blit);
                match cells.encode(&frame, false, seq + 1, seq) {
                    Ok(encoded) => {
                        seq += 1;
                        if encoded.full {
                            counts.full += 1;
                        } else {
                            counts.patches += 1;
                        }
                        counts.cells_record_bytes += encoded.bytes.len();
                        grid.apply(&encoded.bytes).unwrap();
                        cells.commit(frame.clone(), encoded);
                        let context = format!(
                            "{name} clip={clip_right_edge} stage {stage_index} segment {segment_index}"
                        );
                        assert_grid_shows(&grid, &frame, &context);
                    }
                    Err(reason) => *counts.fallbacks.entry(reason.0).or_default() += 1,
                }
            }
        }
        results.push((clip_right_edge, counts));
    }
    results
}

#[test]
fn recorded_rich_screens_take_the_cells_path() {
    for (name, fixture) in [
        ("rich-94x39", include_str!("fixtures/rich-94x39.json")),
        ("rich-457x127", include_str!("fixtures/rich-457x127.json")),
    ] {
        for (clip_right_edge, counts) in replay(name, fixture) {
            eprintln!("terminal cells eligibility {name} clip={clip_right_edge}: {counts:?}");
            assert!(
                counts.fallbacks.is_empty(),
                "{name} clip={clip_right_edge} fell back to ANSI: {counts:?}"
            );
            assert_eq!(counts.full, 1, "{name} clip={clip_right_edge}: {counts:?}");
            assert!(
                counts.patches > 0,
                "{name} clip={clip_right_edge}: {counts:?}"
            );
        }
    }
}
