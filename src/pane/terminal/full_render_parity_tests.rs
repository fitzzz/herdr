//! Fixed replay of the rich recording producer, with a c716 renderer oracle.
//! The oracle retains c716 layout/style/width/cursor/backend behavior and reads
//! glyph codepoints through native grid refs, independent of the UTF8 wrapper.
//! Fixtures stay on the alternate screen with no host scrollback, so screen
//! coordinates are viewport coordinates. Wrapper tests separately retain the
//! exact c716 two-call algorithm and exercise its actual native-call boundary.
use super::*;
use ratatui::backend::Backend;

#[derive(serde::Deserialize)]
struct Fixture {
    width: u16,
    height: u16,
    stages: Vec<Stage>,
}

#[derive(serde::Deserialize)]
struct Stage {
    name: String,
    segments: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    frame_cells: Vec<ratatui::buffer::Cell>,
    backend_cells: Vec<ratatui::buffer::Cell>,
    backend_cursor: ratatui::layout::Position,
    cursor: Option<TerminalCursorState>,
    hyperlinks: VisibleHyperlinks,
    /// The terminal attach/observe helper frame.
    helper_frame: crate::protocol::FrameData,
}

fn observe(pane: &GhosttyPaneTerminal, area: Rect, reference: bool) -> Observation {
    let backend = ratatui::backend::TestBackend::new(area.width, area.height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let mut frame_cells = Vec::new();
    terminal
        .draw(|frame| {
            if reference {
                render_c716_reference(pane, frame, area, true);
            } else {
                pane.render(frame, area, true);
            }
            frame_cells = frame.buffer_mut().content.to_vec();
        })
        .unwrap();
    let hyperlinks = if reference {
        ghostty_visible_hyperlinks(&mut pane.core.lock().unwrap(), area).unwrap()
    } else {
        pane.visible_hyperlinks(area)
    };
    let helper_frame = if reference {
        crate::protocol::FrameData::from_ratatui_buffer_with_hyperlinks(
            terminal.backend().buffer(),
            None,
            &hyperlinks,
        )
    } else {
        let synchronized = pane
            .core
            .lock()
            .unwrap()
            .terminal
            .mode_get(crate::ghostty::MODE_SYNCHRONIZED_OUTPUT)
            .unwrap();
        match pane.screen_snapshot() {
            Some(screen) => {
                assert!(!synchronized, "screen snapshot during synchronized output");
                screen.frame_data(area, 0, None)
            }
            None => {
                // The snapshot is withheld only while synchronized output is
                // active; then the helper uses the legacy render, as
                // `render_terminal_frame` callers do.
                assert!(
                    synchronized,
                    "screen snapshot unavailable: {:?}",
                    super::screen_extract::refresh_screen(&mut pane.core.lock().unwrap()).err()
                );
                crate::protocol::FrameData::from_ratatui_buffer_with_hyperlinks(
                    terminal.backend().buffer(),
                    None,
                    &hyperlinks,
                )
            }
        }
    };
    Observation {
        frame_cells,
        backend_cells: terminal.backend().buffer().content.to_vec(),
        backend_cursor: terminal.backend_mut().get_cursor_position().unwrap(),
        cursor: pane.cursor_state(),
        hyperlinks,
        helper_frame,
    }
}

fn replay(fixture_json: &str) {
    let fixture: Fixture = serde_json::from_str(fixture_json).unwrap();
    let area = Rect::new(0, 0, fixture.width, fixture.height);
    for clip_right_edge in [false, true] {
        let native_width = fixture.width + u16::from(clip_right_edge);
        let make_pane = || {
            let (tx, _rx) = mpsc::channel(4);
            let terminal = crate::ghostty::Terminal::new(native_width, fixture.height, 0).unwrap();
            let pane = GhosttyPaneTerminal::new(terminal, tx).unwrap();
            pane.apply_host_terminal_theme(crate::terminal_theme::TerminalTheme {
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
            });
            pane
        };
        let reference = make_pane();
        let candidate = make_pane();
        for stage in &fixture.stages {
            for pane in [&reference, &candidate] {
                let mut core = pane.core.lock().unwrap();
                for segment in &stage.segments {
                    core.terminal.write(segment.as_bytes());
                    core.decscusr_tracker.observe(segment.as_bytes());
                }
                // No time-based settling enters this fixed semantic oracle.
                core.cursor_settle_state = CursorPositionSettleState::default();
                assert_eq!(
                    core.terminal.active_screen().unwrap(),
                    crate::ghostty::ActiveScreen::Alternate
                );
            }
            let old = observe(&reference, area, true);
            let new = observe(&candidate, area, false);
            let context = format!(
                "{}x{} clip={clip_right_edge} {}",
                fixture.width, fixture.height, stage.name
            );
            for (kind, actual, expected) in [
                ("frame", &new.frame_cells, &old.frame_cells),
                ("backend", &new.backend_cells, &old.backend_cells),
            ] {
                assert_eq!(actual.len(), expected.len(), "{context} {kind} length");
                for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                    assert_eq!(
                        actual,
                        expected,
                        "{context} {kind} cell ({},{})",
                        index % usize::from(fixture.width),
                        index / usize::from(fixture.width)
                    );
                }
            }
            assert_eq!(
                new.backend_cursor, old.backend_cursor,
                "{context} backend cursor"
            );
            assert_eq!(new.cursor, old.cursor, "{context} terminal cursor");
            assert_eq!(new.hyperlinks, old.hyperlinks, "{context} hyperlinks");
            assert_eq!(
                new.helper_frame.hyperlinks, old.helper_frame.hyperlinks,
                "{context} helper hyperlink table"
            );
            for (index, (actual, expected)) in new
                .helper_frame
                .cells
                .iter()
                .zip(&old.helper_frame.cells)
                .enumerate()
            {
                assert_eq!(
                    actual,
                    expected,
                    "{context} helper cell ({},{})",
                    index % usize::from(fixture.width),
                    index / usize::from(fixture.width)
                );
            }
            assert_eq!(new.helper_frame, old.helper_frame, "{context} helper frame");
            assert!(crate::ghostty::raw_cell_layout_verified());
            super::screen_tests::assert_extract_paths_agree(&candidate, &context);
            assert_eq!(
                new.frame_cells.len(),
                usize::from(fixture.width) * usize::from(fixture.height)
            );
            if stage.name == "recorded-rich-setup" {
                assert!(!new.hyperlinks.is_empty());
                assert!(new
                    .frame_cells
                    .iter()
                    .any(|cell| cell.symbol().contains('漢')));
                assert!(new
                    .frame_cells
                    .iter()
                    .any(|cell| cell.symbol().contains('👩')));
                assert!(new.frame_cells.iter().any(|cell| cell.symbol() == " "));
            }
            if clip_right_edge && stage.name == "sgr-palette-erase-and-default-colors" {
                assert_eq!(
                    candidate
                        .core
                        .lock()
                        .unwrap()
                        .terminal
                        .screen_cell(fixture.width - 1, 5)
                        .unwrap()
                        .0,
                    crate::ghostty::CellWide::Wide
                );
            }
        }
    }
}

#[test]
fn full_viewport_recorded_rich_94x39_matches_c716() {
    replay(include_str!("fixtures/rich-94x39.json"));
}

#[test]
fn full_viewport_recorded_rich_457x127_matches_c716() {
    replay(include_str!("fixtures/rich-457x127.json"));
}

fn render_c716_reference(
    pane: &GhosttyPaneTerminal,
    frame: &mut Frame,
    area: Rect,
    show_cursor: bool,
) {
    let Ok(mut core) = pane.core.lock() else {
        return;
    };
    if core
        .terminal
        .mode_get(crate::ghostty::MODE_SYNCHRONIZED_OUTPUT)
        .unwrap_or(false)
    {
        return;
    }
    let host_theme = core.host_terminal_theme;
    let initial_default_foreground = core.initial_default_foreground;
    let initial_default_background = core.initial_default_background;
    let GhosttyPaneCore {
        terminal,
        render_state,
        decscusr_tracker,
        ..
    } = &mut *core;
    if render_state.update(terminal).is_err() {
        return;
    }
    let colors = render_state.colors().ok();
    let default_bg = colors
        .and_then(|c| ghostty_default_bg(c.background, host_theme, initial_default_background));
    let default_fg = colors
        .and_then(|c| ghostty_default_fg(c.foreground, host_theme, initial_default_foreground));
    let resolved_fg = colors.map(|c| ghostty_color(c.foreground));
    let resolved_bg = colors.map(|c| ghostty_color(c.background));
    let palette_overrides = colors
        .zip(terminal.default_palette().ok())
        .and_then(|(colors, default)| PaletteOverrides::new(&colors.palette, &default));
    let hide_kitty_placeholders = crate::kitty_graphics::is_enabled();

    let mut row_iterator = match crate::ghostty::RowIterator::new() {
        Ok(iterator) => iterator,
        Err(_) => return,
    };
    let mut row_cells = match crate::ghostty::RowCells::new() {
        Ok(cells) => cells,
        Err(_) => return,
    };
    {
        let buf = frame.buffer_mut();
        let mut rows = match render_state.populate_row_iterator(&mut row_iterator) {
            Ok(rows) => rows,
            Err(_) => return,
        };
        let mut symbol_scratch = String::new();
        let mut y = 0u16;
        while y < area.height && rows.next() {
            let mut cells = match rows.populate_cells(&mut row_cells) {
                Ok(cells) => cells,
                Err(_) => break,
            };
            let mut x = 0u16;
            while x < area.width && cells.next() {
                let basic = cells.basic_data().unwrap_or_default();
                let style = ghostty_cell_style(
                    &cells,
                    &basic,
                    default_fg,
                    default_bg,
                    resolved_fg,
                    resolved_bg,
                    palette_overrides.as_ref(),
                );
                let symbol = match reference_buffer_symbol_into(
                    terminal,
                    x,
                    y,
                    basic.wide,
                    hide_kitty_placeholders,
                    &mut symbol_scratch,
                ) {
                    Ok(symbol) => symbol,
                    Err(_) => {
                        symbol_scratch.clear();
                        symbol_scratch.push_str(ghostty_blank_symbol_for_width(basic.wide));
                        symbol_scratch.as_str()
                    }
                };
                let cell = &mut buf[(area.x + x, area.y + y)];
                cell.reset();
                cell.set_symbol(symbol);
                cell.set_style(style);
                x += 1;
            }
            while x < area.width {
                let cell = &mut buf[(area.x + x, area.y + y)];
                ghostty_reset_cell(cell, default_fg, default_bg);
                x += 1;
            }
            y += 1;
        }
        while y < area.height {
            for x in 0..area.width {
                let cell = &mut buf[(area.x + x, area.y + y)];
                ghostty_reset_cell(cell, default_fg, default_bg);
            }
            y += 1;
        }
    }

    ghostty_clear_render_dirty(render_state, area.height);

    let current_cursor = cursor_state_from_render_state(render_state, decscusr_tracker);
    if show_cursor {
        if let Some(cursor) =
            effective_cursor_state(&mut core, current_cursor).filter(|cursor| cursor.visible)
        {
            if cursor.x < area.width && cursor.y < area.height {
                frame.set_cursor_position((area.x + cursor.x, area.y + cursor.y));
            }
        }
    }
}

fn reference_buffer_symbol_into<'a>(
    terminal: &crate::ghostty::Terminal,
    x: u16,
    y: u16,
    wide: crate::ghostty::CellWide,
    hide_kitty_placeholders: bool,
    symbol_scratch: &'a mut String,
) -> Result<&'a str, crate::ghostty::Error> {
    symbol_scratch.clear();
    match wide {
        crate::ghostty::CellWide::SpacerTail => {}
        crate::ghostty::CellWide::SpacerHead => symbol_scratch.push(' '),
        crate::ghostty::CellWide::Narrow | crate::ghostty::CellWide::Wide => {
            let (_, codepoints) = terminal.screen_cell(x, u32::from(y))?;
            for codepoint in codepoints {
                if let Some(ch) = char::from_u32(codepoint) {
                    symbol_scratch.push(ch);
                }
            }
            let hidden_kitty_placeholder = hide_kitty_placeholders
                && symbol_scratch.chars().next().map(u32::from)
                    == Some(crate::ghostty::KITTY_UNICODE_PLACEHOLDER);
            if hidden_kitty_placeholder || symbol_scratch.is_empty() {
                symbol_scratch.clear();
                symbol_scratch.push(' ');
            }
        }
    }

    let expected_width = match wide {
        crate::ghostty::CellWide::Wide => 2,
        crate::ghostty::CellWide::Narrow | crate::ghostty::CellWide::SpacerHead => 1,
        crate::ghostty::CellWide::SpacerTail => 0,
    };
    let actual_width = symbol_scratch.width();
    if actual_width != expected_width
        && !(wide == crate::ghostty::CellWide::Narrow && actual_width == 2)
        && !(wide == crate::ghostty::CellWide::Wide
            && is_halfwidth_katakana_voiced_grapheme(symbol_scratch))
    {
        symbol_scratch.clear();
        symbol_scratch.push_str(ghostty_blank_symbol_for_width(wide));
    }

    Ok(symbol_scratch.as_str())
}
