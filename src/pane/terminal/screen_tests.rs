//! Retained screen: parity with the legacy renderer, incremental refresh, and
//! retention. Test names start with `terminal_screen` for focused runs.
use super::*;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

fn pane(cols: u16, rows: u16) -> GhosttyPaneTerminal {
    let (tx, _rx) = mpsc::channel(4);
    let terminal = crate::ghostty::Terminal::new(cols, rows, 100).unwrap();
    GhosttyPaneTerminal::new(terminal, tx).unwrap()
}

fn write(pane: &GhosttyPaneTerminal, bytes: &[u8]) {
    let mut core = pane.core.lock().unwrap();
    core.terminal.write(bytes);
    core.decscusr_tracker.observe(bytes);
}

fn known_host_theme() -> crate::terminal_theme::TerminalTheme {
    crate::terminal_theme::TerminalTheme {
        foreground: Some(crate::terminal_theme::RgbColor {
            r: 0xaa,
            g: 0xbb,
            b: 0xcc,
        }),
        background: Some(crate::terminal_theme::RgbColor {
            r: 0x11,
            g: 0x22,
            b: 0x33,
        }),
        ..Default::default()
    }
}

/// Frame-buffer cells and the test-backend buffer after one draw.
fn draw(
    pane: &GhosttyPaneTerminal,
    area: Rect,
    legacy: bool,
) -> (Vec<ratatui::buffer::Cell>, Buffer) {
    let backend = TestBackend::new(area.width, area.height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let mut frame_cells = Vec::new();
    terminal
        .draw(|frame| {
            if legacy {
                pane.render_legacy(frame, area, true);
            } else {
                pane.render(frame, area, true);
            }
            frame_cells = frame.buffer_mut().content.to_vec();
        })
        .unwrap();
    (frame_cells, terminal.backend().buffer().clone())
}

/// Both extraction paths must produce identical rows; the accessor path is the
/// fallback when the packed cell layout cannot be verified.
pub(super) fn assert_extract_paths_agree(pane: &GhosttyPaneTerminal, context: &str) {
    let mut core = pane.core.lock().unwrap();
    core.screen = None;
    let raw =
        screen_extract::refresh_screen_with(&mut core, screen_extract::ExtractPath::Raw).unwrap();
    core.screen = None;
    let accessor =
        screen_extract::refresh_screen_with(&mut core, screen_extract::ExtractPath::Accessor)
            .unwrap();
    assert_eq!(raw.rows_data.len(), accessor.rows_data.len(), "{context}");
    for (y, (raw, accessor)) in raw.rows_data.iter().zip(&accessor.rows_data).enumerate() {
        assert!(
            raw.same_content(accessor),
            "{context} row {y}: raw {raw:?} accessor {accessor:?}"
        );
    }
    core.screen = None;
}

const RICH_CONTENT: &[&[u8]] = &[
    b"\x1b]4;5;rgb:12/34/56\x07",
    b"plain \x1b[1mbold\x1b[0m \x1b[3mit\x1b[23m \x1b[2mdim\x1b[22m \x1b[5mbl\x1b[25m \x1b[9mst\x1b[29m\r\n",
    b"\x1b[4:3m\x1b[58;2;9;8;7mcurly\x1b[0m \x1b[58;5;5m\x1b[4mul5\x1b[0m \x1b[38;5;5mp5\x1b[0m \x1b[38;5;200mi200\x1b[0m \x1b[38;2;1;2;3mrgb\x1b[0m\r\n",
    b"\x1b[7minv\x1b[0m \x1b[8mhid\x1b[0m \x1b[7;31;42mx\x1b[0m \x1b[8;44my\x1b[0m\r\n",
    "\u{6f22}\u{5b57} e\u{301} \u{1f469}\u{200d}\u{1f4bb} \u{2764}\u{fe0f} \u{1f1ef}\u{1f1f5} \u{ff76}\u{ff9e} \u{10eeee}x\r\n"
        .as_bytes(),
    "\x1b]8;;https://example.test/a\x1b\\link text \u{6f22}\x1b]8;;\x1b\\ \x1b]8;;https://example.test/b\x1b\\\x1b[1mb\x1b[0m\x1b]8;;\x1b\\ \x1b]8;;https://example.test/a\x1b\\again\x1b]8;;\x1b\\\r\n"
        .as_bytes(),
    b"\x1b[45m\x1b[K\x1b[0m\r\n",
    b"\x1b[48;2;7;8;9m\x1b[K\x1b[0m\r\n",
    "0123456789012345678901234567890123456789012345678\u{6f22}tail".as_bytes(),
];

#[test]
fn terminal_screen_matches_legacy_render_frames_and_links_for_every_cell_kind() {
    for host_theme in [None, Some(known_host_theme())] {
        let reference = pane(50, 12);
        let candidate = pane(50, 12);
        for pane in [&reference, &candidate] {
            if let Some(theme) = host_theme {
                pane.apply_host_terminal_theme(theme);
            }
            for bytes in RICH_CONTENT {
                write(pane, bytes);
            }
        }
        for area in [
            Rect::new(0, 0, 50, 12),
            Rect::new(0, 0, 30, 8),
            Rect::new(0, 0, 61, 14),
        ] {
            let context = format!("theme={} area={area:?}", host_theme.is_some());
            let (old_frame, old_backend) = draw(&reference, area, true);
            let (new_frame, new_backend) = draw(&candidate, area, false);
            assert_eq!(new_frame, old_frame, "{context} frame cells");
            assert_eq!(new_backend, old_backend, "{context} backend cells");

            let old_links =
                ghostty_visible_hyperlinks(&mut reference.core.lock().unwrap(), area).unwrap();
            let new_links = candidate.visible_hyperlinks(area);
            assert_eq!(new_links, old_links, "{context} hyperlinks");
            if area.width == 50 {
                assert!(new_links.iter().any(|(_, symbol, _)| symbol == " "));
            }

            let old_helper = crate::protocol::FrameData::from_ratatui_buffer_with_hyperlinks(
                &old_backend,
                None,
                &old_links,
            );
            let new_helper = candidate.screen_snapshot().unwrap().frame_data(
                area,
                candidate.crop_rows_above(area.height),
                None,
            );
            assert_eq!(new_helper, old_helper, "{context} helper frame");
            if area.width == 50 {
                assert_eq!(new_helper.hyperlinks.len(), 2, "{context}");
            }
        }
        assert!(crate::ghostty::raw_cell_layout_verified());
        assert_extract_paths_agree(&candidate, "rich content");
    }
}

#[test]
fn terminal_screen_shares_unchanged_rows_between_refreshes() {
    let pane = pane(12, 4);
    write(&pane, b"one\r\ntwo\r\nthree");
    let first = pane.screen_snapshot().unwrap();
    assert!(Arc::ptr_eq(&first, &pane.screen_snapshot().unwrap()));

    write(&pane, b"\x1b[2;1HTWO");
    let second = pane.screen_snapshot().unwrap();
    assert_eq!(
        second.changed_rows_since(&first).collect::<Vec<_>>(),
        vec![1]
    );
    for y in [0, 2, 3] {
        assert!(Arc::ptr_eq(&first.rows_data[y], &second.rows_data[y]));
    }
    assert_ne!(first.rows_data[1].revision, second.rows_data[1].revision);

    // A row rewritten with identical content keeps its revision.
    write(&pane, b"\x1b[1;1Hone");
    let third = pane.screen_snapshot().unwrap();
    assert_eq!(third.changed_rows_since(&second).count(), 0);
}

#[test]
fn terminal_screen_tracks_cursor_moves_without_row_changes() {
    let pane = pane(12, 4);
    write(&pane, b"abc");
    let first = pane.screen_snapshot().unwrap();
    write(&pane, b"\x1b[3;5H");
    let second = pane.screen_snapshot().unwrap();
    assert_ne!(first.generation, second.generation);
    assert_eq!(
        second.cursor.map(|cursor| (cursor.x, cursor.y)),
        Some((4, 2))
    );
    assert_eq!(second.changed_rows_since(&first).count(), 0);
    assert!(first
        .rows_data
        .iter()
        .zip(&second.rows_data)
        .all(|(old, new)| Arc::ptr_eq(old, new)));
}

#[test]
fn terminal_screen_follows_palette_and_host_theme_changes_without_text_writes() {
    let pane = pane(8, 2);
    write(&pane, b"\x1b[38;5;1mX\x1b[0m");
    let first = pane.screen_snapshot().unwrap();
    assert_eq!(
        first.rows_data[0].cells[0].fg,
        crate::protocol::color_to_u32(Color::Indexed(1))
    );

    write(&pane, b"\x1b]4;1;rgb:12/34/56\x07");
    let second = pane.screen_snapshot().unwrap();
    assert_eq!(
        second.rows_data[0].cells[0].fg,
        crate::protocol::color_to_u32(Color::Rgb(0x12, 0x34, 0x56))
    );
    assert_eq!(
        second.changed_rows_since(&first).collect::<Vec<_>>(),
        vec![0]
    );

    pane.apply_host_terminal_theme(known_host_theme());
    let third = pane.screen_snapshot().unwrap();
    let mut reference = Buffer::empty(Rect::new(0, 0, 8, 2));
    let mut candidate = Buffer::empty(Rect::new(0, 0, 8, 2));
    third.write_to_buffer(&mut candidate, Rect::new(0, 0, 8, 2), 0);
    let backend = TestBackend::new(8, 2);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            pane.render_legacy(frame, Rect::new(0, 0, 8, 2), false);
            reference = frame.buffer_mut().clone();
        })
        .unwrap();
    assert_eq!(candidate, reference);
}

#[test]
fn terminal_screen_render_and_dirty_patches_do_not_consume_each_others_changes() {
    let pane = PaneTerminal::new(pane(8, 3));
    let area = Rect::new(0, 0, 8, 3);
    assert!(matches!(
        pane.collect_dirty_patch(8, 3),
        TerminalDirtyPatchOutcome::Patch(_)
    ));
    let _ = draw(&pane.ghostty, area, false);

    // Render first, then patch: the patch still carries the row.
    write(&pane.ghostty, b"\x1b[2;1HX");
    let (cells, _) = draw(&pane.ghostty, area, false);
    assert_eq!(cells[8].symbol(), "X");
    let TerminalDirtyPatchOutcome::Patch(patch) = pane.collect_dirty_patch(8, 3) else {
        panic!("expected the rendered row in the next patch");
    };
    assert_eq!(
        patch.rows.iter().map(|(y, _)| *y).collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(patch.rows[0].1[0].symbol, "X");

    // Patch and hyperlink scan first, then render: the render still sees it.
    write(&pane.ghostty, b"\x1b[3;1HY");
    let TerminalDirtyPatchOutcome::Patch(patch) = pane.collect_dirty_patch(8, 3) else {
        panic!("expected a patch");
    };
    assert_eq!(
        patch.rows.iter().map(|(y, _)| *y).collect::<Vec<_>>(),
        vec![2]
    );
    assert!(pane.visible_hyperlinks(area).is_empty());
    let (cells, _) = draw(&pane.ghostty, area, false);
    assert_eq!(cells[16].symbol(), "Y");
    assert!(matches!(
        pane.collect_dirty_patch(8, 3),
        TerminalDirtyPatchOutcome::Clean
    ));
}

#[test]
fn terminal_screen_patch_rows_match_full_render_cells() {
    let pane = PaneTerminal::new(pane(50, 12));
    for bytes in RICH_CONTENT {
        write(&pane.ghostty, bytes);
    }
    let area = Rect::new(0, 0, 52, 12);
    let TerminalDirtyPatchOutcome::Patch(patch) = pane.collect_dirty_patch(52, 12) else {
        panic!("expected a full first patch");
    };
    let (cells, _) = draw(&pane.ghostty, area, false);
    assert_eq!(patch.rows.len(), 12);
    for (y, row) in patch.rows {
        for (x, cell) in row.into_iter().enumerate() {
            let expected = &cells[usize::from(y) * 52 + x];
            assert_eq!(cell.symbol, expected.symbol(), "({x},{y})");
            assert_eq!(
                cell.fg,
                crate::protocol::color_to_u32(expected.fg),
                "({x},{y})"
            );
            assert_eq!(
                cell.bg,
                crate::protocol::color_to_u32(expected.bg),
                "({x},{y})"
            );
            assert_eq!(cell.modifier, expected.modifier.bits(), "({x},{y})");
        }
    }
}

#[test]
fn terminal_screen_is_released_for_quiet_hidden_panes() {
    let pane = pane(12, 4);
    write(&pane, b"visible once");
    let _ = draw(&pane, Rect::new(0, 0, 12, 4), false);
    assert!(pane.screen_is_retained());

    // The pane goes hidden and quiet: no PTY output and no reads. Only the
    // server's periodic sweep runs.
    let used_at = pane.core.lock().unwrap().screen_used_at;
    assert!(!pane.release_idle_screen(used_at, IDLE_SCREEN_RETENTION));
    assert!(pane.screen_is_retained());
    assert!(pane.release_idle_screen(used_at + IDLE_SCREEN_RETENTION, IDLE_SCREEN_RETENTION));
    assert!(!pane.screen_is_retained());

    // Shown again: the next read rebuilds the full screen.
    let (cells, _) = draw(&pane, Rect::new(0, 0, 12, 4), false);
    assert_eq!(cells[0].symbol(), "v");
    assert!(pane.screen_is_retained());
}

#[test]
fn terminal_screen_release_skips_a_busy_core() {
    let pane = pane(4, 2);
    let _ = pane.screen_snapshot().unwrap();
    let used_at = pane.core.lock().unwrap().screen_used_at;
    let guard = pane.core.lock().unwrap();
    assert!(!pane.release_idle_screen(used_at + IDLE_SCREEN_RETENTION, IDLE_SCREEN_RETENTION));
    drop(guard);
    assert!(pane.release_idle_screen(used_at + IDLE_SCREEN_RETENTION, IDLE_SCREEN_RETENTION));
}

#[test]
fn terminal_screen_is_unavailable_during_synchronized_output() {
    let pane = pane(8, 2);
    write(&pane, b"\x1b[?2026hpartial");
    assert!(pane.screen_snapshot().is_none());
    write(&pane, b"\x1b[?2026l");
    let screen = pane.screen_snapshot().unwrap();
    assert_eq!(screen.rows_data[0].cells[0].content, u32::from('p'));
}

/// Helper frame cells with hyperlinks resolved to URIs, since table indices
/// may differ between frames.
fn rows_by_uri(frame: &crate::protocol::FrameData) -> Vec<Vec<(CellData, Option<String>)>> {
    frame
        .cells
        .chunks(usize::from(frame.width))
        .map(|row| {
            row.iter()
                .map(|cell| {
                    let uri = cell
                        .hyperlink
                        .map(|index| frame.hyperlinks[index as usize].clone());
                    (
                        CellData {
                            hyperlink: None,
                            ..cell.clone()
                        },
                        uri,
                    )
                })
                .collect()
        })
        .collect()
}

/// The rows the server ANSI encoder may skip when `next` follows `committed`:
/// equal key and equal row revision. Asserts the skip contract for each one,
/// same cells and same link targets.
fn skippable_rows(
    committed: &(crate::protocol::FrameData, super::screen::FrameRowRevisions),
    next: &(crate::protocol::FrameData, super::screen::FrameRowRevisions),
) -> Vec<usize> {
    if committed.1.key != next.1.key {
        return Vec::new();
    }
    let (old_rows, new_rows) = (rows_by_uri(&committed.0), rows_by_uri(&next.0));
    (0..next.1.rows.len())
        .filter(|&y| committed.1.rows[y] == next.1.rows[y])
        .inspect(|&y| assert_eq!(new_rows[y], old_rows[y], "row {y}"))
        .collect()
}

#[test]
fn terminal_screen_frame_row_revisions_identify_equal_frame_rows() {
    let pane = pane(20, 5);
    write(
        &pane,
        "top\r\n\u{6f22}\u{5b57} wide\r\nchange me\r\n\x1b]8;;https://example.test/a\x1b\\link\x1b]8;;\x1b\\"
            .as_bytes(),
    );
    let area = Rect::new(0, 0, 20, 6);
    let (first_frame, first) = pane
        .screen_snapshot()
        .unwrap()
        .frame_data_with_row_revisions(area, 0, None);
    let first = first.unwrap();
    assert_eq!(first.rows.len(), 6);
    assert_eq!(first.rows[5], 0, "padding row below the grid");

    // Row 2 changes; a new link on row 0 renumbers row 3's hyperlink index.
    write(
        &pane,
        b"\x1b[3;1Hchanged\x1b[1;5H\x1b]8;;https://example.test/b\x1b\\b\x1b]8;;\x1b\\",
    );
    let (second_frame, second) = pane
        .screen_snapshot()
        .unwrap()
        .frame_data_with_row_revisions(area, 0, None);
    let second = second.unwrap();
    assert_eq!(second.key, first.key);
    assert_ne!(second_frame.hyperlinks, first_frame.hyperlinks);
    let (old_rows, new_rows) = (rows_by_uri(&first_frame), rows_by_uri(&second_frame));
    let mut equal_rows = Vec::new();
    for y in 0..6 {
        if first.rows[y] == second.rows[y] {
            assert_eq!(new_rows[y], old_rows[y], "row {y}");
            equal_rows.push(y);
        }
    }
    assert_eq!(equal_rows, vec![1, 3, 4, 5]);

    // Another area is another key.
    let (_, narrower) = pane
        .screen_snapshot()
        .unwrap()
        .frame_data_with_row_revisions(Rect::new(0, 0, 19, 6), 0, None);
    assert_ne!(narrower.unwrap().key, second.key);

    // A wide cell clipped at the right edge carries into the next row.
    let (_, clipped) = pane
        .screen_snapshot()
        .unwrap()
        .frame_data_with_row_revisions(Rect::new(0, 0, 1, 6), 0, None);
    assert!(clipped.is_none());
}

#[test]
fn terminal_screen_frame_row_revisions_match_only_the_same_screen_rows() {
    // A 20x3 grid in a 20x4 area: row 3 is padding.
    let area = Rect::new(0, 0, 20, 4);
    let frame = |pane: &GhosttyPaneTerminal| {
        let (frame, revisions) = pane
            .screen_snapshot()
            .unwrap()
            .frame_data_with_row_revisions(area, 0, None);
        (frame, revisions.unwrap())
    };
    let content =
        "same\r\n\x1b]8;;https://example.test/a\x1b\\text\x1b]8;;\x1b\\\r\n\x1b[38;5;1mred\x1b[0m";
    let shown = pane(20, 3);
    let other = pane(20, 3);
    // Herdr gives every pane the host's default colors. Ghostty's render
    // state only takes a new default background (OSC 11) once a default
    // foreground exists too (render.zig `update`, block `bg_fg`).
    for pane in [&shown, &other] {
        pane.apply_host_terminal_theme(known_host_theme());
    }
    write(&shown, content.as_bytes());
    write(&other, content.as_bytes());
    let first = frame(&shown);

    // Pane switch: same geometry, colors and cells give the same key, but no
    // grid row of another screen shares a revision. Only padding matches.
    let switched = frame(&other);
    assert_eq!(switched.0, first.0);
    assert_eq!(switched.1.key, first.1.key);
    assert_eq!(skippable_rows(&first, &switched), vec![3]);
    assert_eq!(skippable_rows(&switched, &first), vec![3]);

    // The cursor is frame metadata, never drawn into helper cells.
    write(&shown, b"\x1b[1;10H");
    let moved = frame(&shown);
    assert_eq!(skippable_rows(&first, &moved), vec![0, 1, 2, 3]);

    // Same text, new OSC 8 target.
    write(
        &shown,
        b"\x1b[2;1H\x1b]8;;https://example.test/b\x1b\\text\x1b]8;;\x1b\\",
    );
    let retargeted = frame(&shown);
    assert_eq!(skippable_rows(&moved, &retargeted), vec![0, 2, 3]);

    // A palette entry in use changes only the row that uses it.
    write(&shown, b"\x1b]4;1;rgb:12/34/56\x07");
    let repainted = frame(&shown);
    assert_eq!(skippable_rows(&retargeted, &repainted), vec![0, 1, 3]);

    // Eviction and recreation: identical cells under the same key, but every
    // grid row is new, with revisions above any issued before.
    let issued_before = repainted.1.rows.iter().copied().max().unwrap();
    let used_at = shown.core.lock().unwrap().screen_used_at;
    assert!(shown.release_idle_screen(used_at + IDLE_SCREEN_RETENTION, IDLE_SCREEN_RETENTION));
    let recreated = frame(&shown);
    assert_eq!(recreated.0, repainted.0);
    assert_eq!(skippable_rows(&repainted, &recreated), vec![3]);
    assert!(recreated.1.rows[..3]
        .iter()
        .all(|&revision| revision > issued_before));

    // A new default background repaints padding, so the key changes.
    write(&shown, b"\x1b]11;rgb:12/34/56\x07");
    let new_background = frame(&shown);
    let padding_bg = crate::protocol::color_to_u32(Color::Rgb(0x12, 0x34, 0x56));
    assert_ne!(recreated.0.cells[3 * 20].bg, padding_bg);
    assert_eq!(new_background.0.cells[3 * 20].bg, padding_bg);
    assert_ne!(new_background.1.key, recreated.1.key);
    assert!(skippable_rows(&recreated, &new_background).is_empty());
}

#[test]
fn terminal_screen_patch_links_attach_like_full_frames() {
    let pane = pane(50, 12);
    for bytes in RICH_CONTENT {
        write(&pane, bytes);
    }
    let TerminalDirtyPatchOutcome::Patch(patch) = pane.collect_dirty_patch(50, 12) else {
        panic!("linked rows are patched, not a fallback");
    };
    let (frame, revisions) = pane
        .screen_snapshot()
        .unwrap()
        .frame_data_with_row_revisions(Rect::new(0, 0, 50, 12), 0, None);
    // Every row starts with fresh diff state, as a patch row does.
    assert!(revisions.is_some());
    let uri = |table: &[String], cell: &CellData| {
        cell.hyperlink.map(|index| table[index as usize].clone())
    };
    let mut linked = 0;
    for (y, cells) in &patch.rows {
        for (x, cell) in cells.iter().enumerate() {
            let full = &frame.cells[usize::from(*y) * 50 + x];
            assert_eq!(
                uri(&patch.hyperlinks, cell),
                uri(&frame.hyperlinks, full),
                "({x}, {y})"
            );
            linked += usize::from(cell.hyperlink.is_some());
        }
    }
    assert!(linked > 0);
    assert_eq!(patch.hyperlinks.len(), 2);
}

#[test]
fn terminal_screen_extracts_each_change_once_for_every_consumer() {
    let pane = PaneTerminal::new(pane(20, 6));
    let area = Rect::new(0, 0, 20, 6);
    let extracted = || pane.ghostty.core.lock().unwrap().rows_extracted;
    let read_everything = || {
        // Two TUI clients, a helper frame, a hyperlink scan and a retained patch.
        let _ = draw(&pane.ghostty, area, false);
        let _ = draw(&pane.ghostty, area, false);
        let _ = pane
            .ghostty
            .screen_snapshot()
            .unwrap()
            .frame_data(area, 0, None);
        let _ = pane.ghostty.visible_hyperlinks(area);
        let _ = pane.collect_dirty_patch(20, 6);
    };

    // Output to a pane nobody displays is parsed but never collected.
    write(&pane.ghostty, b"one\r\ntwo\r\nthree");
    assert_eq!(extracted(), 0);

    // The first reader reads every row once; the other consumers share it.
    read_everything();
    assert_eq!(extracted(), 6);

    // Only Ghostty's dirty rows are read again, and only once. Moving the
    // cursor from row 2 to row 1 marks both rows dirty (Screen.cursorChangePin
    // marks the row it leaves and the row it enters), and printing marks row 1;
    // nothing sets a terminal- or screen-wide flag, so no full rebuild.
    write(&pane.ghostty, b"\x1b[2;1HTWO");
    let _ = draw(&pane.ghostty, area, false);
    let after_change = extracted();
    assert_eq!(after_change, 6 + 2);
    read_everything();
    assert_eq!(extracted(), after_change);

    // Hidden again: a burst of output reads nothing until a consumer asks.
    for _ in 0..50 {
        write(&pane.ghostty, b"hidden output\r\n");
    }
    assert_eq!(extracted(), after_change);
    let used_at = pane.ghostty.core.lock().unwrap().screen_used_at;
    assert!(pane
        .ghostty
        .release_idle_screen(used_at + IDLE_SCREEN_RETENTION, IDLE_SCREEN_RETENTION));
    assert_eq!(extracted(), after_change);
    read_everything();
    assert_eq!(extracted(), after_change + 6);
}

fn row_text(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn rows_text(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| row_text(buffer, y))
        .collect()
}

fn numbered_lines(pane: &GhosttyPaneTerminal, prefix: &str, range: std::ops::Range<usize>) {
    let lines = range
        .map(|i| format!("{prefix}{i}"))
        .collect::<Vec<_>>()
        .join("\r\n");
    write(pane, lines.as_bytes());
}

#[test]
fn terminal_screen_cropped_view_follows_the_cursor_in_both_renderers() {
    // Another client holds the grid at 10 rows; this view is 4 rows tall.
    let pane = pane(20, 10);
    numbered_lines(&pane, "L", 0..9);
    write(
        &pane,
        b"\r\n\x1b]8;;https://example.test/last\x1b\\L9\x1b]8;;\x1b\\",
    );
    let area = Rect::new(0, 0, 20, 4);
    for legacy in [true, false] {
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                if legacy {
                    pane.render_legacy(frame, area, true);
                } else {
                    pane.render(frame, area, true);
                }
            })
            .unwrap();
        assert_eq!(
            rows_text(terminal.backend().buffer()),
            ["L6", "L7", "L8", "L9"],
            "legacy={legacy}"
        );
        assert_eq!(
            ratatui::backend::Backend::get_cursor_position(terminal.backend_mut()).unwrap(),
            ratatui::layout::Position::new(2, 3),
            "legacy={legacy}"
        );
    }
    let links = pane.visible_hyperlinks(area);
    assert!(!links.is_empty());
    assert!(links
        .iter()
        .all(|((_, y), _, uri)| *y == 3 && uri == "https://example.test/last"));
    let legacy_links = ghostty_visible_hyperlinks(&mut pane.core.lock().unwrap(), area).unwrap();
    assert_eq!(legacy_links, links);
    let screen = pane.screen_snapshot().unwrap();
    let frame = screen.frame_data(area, pane.crop_rows_above(area.height), None);
    let first_row: String = frame.cells[..2]
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect();
    assert_eq!(first_row, "L6");
}

#[test]
fn terminal_screen_cropped_view_keeps_the_top_while_the_cursor_is_in_view() {
    let pane = pane(20, 10);
    write(&pane, b"A\r\nB");
    let (_, buffer) = draw(&pane, Rect::new(0, 0, 20, 4), false);
    assert_eq!(rows_text(&buffer), ["A", "B", "", ""]);
    assert_eq!(pane.crop_rows_above(4), 0);
    assert_eq!(pane.crop_rows_above(10), 0);
}

#[test]
fn terminal_screen_cropped_view_scrolled_back_stays_put_and_reaches_the_oldest_row() {
    let pane = pane(20, 10);
    numbered_lines(&pane, "L", 0..40);
    let area = Rect::new(0, 0, 20, 4);
    ghostty_set_scroll_offset_from_bottom(&mut pane.core.lock().unwrap().terminal, 5);
    let (_, before) = draw(&pane, area, false);
    write(&pane, b"\r\nN0\r\nN1\r\nN2");
    let (_, after) = draw(&pane, area, false);
    assert_eq!(rows_text(&after), rows_text(&before));
    let (_, legacy) = draw(&pane, area, true);
    assert_eq!(rows_text(&legacy), rows_text(&before));

    let max = pane.scroll_metrics().unwrap().max_offset_from_bottom;
    ghostty_set_scroll_offset_from_bottom(&mut pane.core.lock().unwrap().terminal, max);
    let (_, top) = draw(&pane, area, false);
    assert_eq!(rows_text(&top), ["L0", "L1", "L2", "L3"]);
}
