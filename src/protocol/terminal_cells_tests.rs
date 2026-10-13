use super::*;
use crate::protocol::render_ansi::BlitEncoder;

fn cell(symbol: &str) -> CellData {
    CellData {
        symbol: symbol.to_owned(),
        fg: 0,
        bg: 0,
        modifier: 0,
        skip: false,
        hyperlink: None,
    }
}

fn styled(symbol: &str, fg: u32, bg: u32, modifier: u16) -> CellData {
    CellData {
        fg,
        bg,
        modifier,
        ..cell(symbol)
    }
}

fn linked(symbol: &str, index: u32) -> CellData {
    CellData {
        hyperlink: Some(index),
        ..cell(symbol)
    }
}

fn visible_cursor(x: u16, y: u16) -> Option<CursorState> {
    Some(CursorState {
        x,
        y,
        visible: true,
        shape: 6,
    })
}

fn frame_from_rows(rows: &[Vec<CellData>], hyperlinks: &[&str]) -> FrameData {
    FrameData {
        cells: rows.iter().flatten().cloned().collect(),
        width: rows[0].len() as u16,
        height: rows.len() as u16,
        cursor: visible_cursor(1, 0),
        hyperlinks: hyperlinks.iter().map(|uri| (*uri).to_owned()).collect(),
        graphics: Vec::new(),
    }
}

fn text_row(text: &str, width: usize) -> Vec<CellData> {
    let mut row: Vec<CellData> = text.chars().map(|ch| cell(&ch.to_string())).collect();
    row.resize_with(width, || cell(" "));
    row
}

type Resolved = (String, u8, u32, u32, u32, u16, Option<String>);

fn resolved(grid: &ReferenceGrid) -> Vec<Resolved> {
    grid.cells
        .iter()
        .map(|cell| {
            (
                cell.text.clone(),
                cell.width,
                cell.fg,
                cell.bg,
                cell.underline_color,
                cell.attrs,
                grid.link_uri(cell).map(str::to_owned),
            )
        })
        .collect()
}

fn full_grid(frame: &FrameData) -> ReferenceGrid {
    let encoded = CellsEncoder::new().encode(frame, false, 1, 0).unwrap();
    let mut grid = ReferenceGrid::default();
    grid.apply(&encoded.bytes).unwrap();
    grid
}

/// Encodes, applies and commits `frame` as the next record of one stream.
fn send(encoder: &mut CellsEncoder, grid: &mut ReferenceGrid, frame: &FrameData, seq: u64) -> bool {
    let encoded = encoder.encode(frame, false, seq, grid.seq).unwrap();
    let full = encoded.full;
    grid.apply(&encoded.bytes).unwrap();
    encoder.commit(frame.clone(), encoded);
    full
}

fn unsupported(frame: &FrameData) -> &'static str {
    CellsEncoder::new()
        .encode(frame, false, 1, 0)
        .map(|_| ())
        .unwrap_err()
        .0
}

/// A frame whose first `count` cells each link a distinct URI.
fn many_links(count: usize) -> FrameData {
    let width = 256;
    let uris: Vec<String> = (0..count).map(|n| format!("https://l/{n}")).collect();
    let refs: Vec<&str> = uris.iter().map(String::as_str).collect();
    let rows: Vec<Vec<CellData>> = (0..count.div_ceil(width))
        .map(|y| {
            (0..width)
                .map(|x| match y * width + x {
                    index if index < count => linked("x", index as u32),
                    _ => cell("x"),
                })
                .collect()
        })
        .collect();
    frame_from_rows(&rows, &refs)
}

#[test]
fn full_frame_round_trips_text_style_and_cursor() {
    let mut row = text_row("hi", 4);
    let curly_bold = crate::protocol::modifier_with_underline_style(
        ratatui::style::Modifier::BOLD | ratatui::style::Modifier::UNDERLINED,
        3,
    )
    .bits();
    row[0] = styled("h", 0x00_00_00_02, 0x02_10_20_30, curly_bold);
    let frame = frame_from_rows(&[row, text_row("ok", 4)], &[]);
    let encoded = CellsEncoder::new().encode(&frame, false, 7, 0).unwrap();
    assert!(encoded.full);
    let decoded = decode(&encoded.bytes).unwrap();
    assert!(decoded.full);
    assert_eq!((decoded.seq, decoded.base_seq), (7, 0));
    assert_eq!((decoded.width, decoded.height), (4, 2));
    assert_eq!(decoded.cursor, frame.cursor);
    assert_eq!(decoded.spans.len(), 2);
    let first = &decoded.spans[0].cells[0];
    assert_eq!(first.text, "h");
    assert_eq!(first.fg, 0x00_00_00_02, "named red keeps its SGR identity");
    assert_eq!(first.bg, 0x02_10_20_30);
    assert_eq!(
        first.attrs, curly_bold,
        "modifier bits pass through unchanged"
    );
    assert_eq!(first.underline_color, COLOR_DEFAULT);
    assert_eq!(decoded.spans[1].cells[1].text, "k");
    assert_eq!(decoded.spans[1].cells[1].fg, COLOR_DEFAULT);
}

#[test]
fn ascii_cells_with_shared_style_cost_two_bytes() {
    let frame = frame_from_rows(&[text_row(&"x".repeat(200), 200)], &[]);
    let encoded = CellsEncoder::new().encode(&frame, false, 1, 0).unwrap();
    // header + link count + span count + span header + 200 * (tag + byte)
    assert_eq!(encoded.bytes.len(), HEADER_BYTES + 4 + 4 + 6 + 400);
}

#[test]
fn colors_keep_every_value_the_ansi_encoder_distinguishes() {
    for named in 0..=0x10 {
        assert_eq!(codec_color(named), named);
    }
    assert_eq!(
        codec_color(0x00_00_00_11),
        COLOR_DEFAULT,
        "ANSI paints it as 39"
    );
    assert_eq!(
        codec_color(0x00_12_00_02),
        0x00_00_00_02,
        "ANSI reads the low byte"
    );
    assert_eq!(codec_color(0x01_00_00_C8), 0x01_00_00_C8);
    assert_eq!(
        codec_color(0x01_AB_00_C8),
        0x01_00_00_C8,
        "ANSI reads the low byte"
    );
    assert_eq!(codec_color(0x02_AB_CD_EF), 0x02_AB_CD_EF);
    assert_eq!(
        codec_color(0x07_00_00_00),
        COLOR_DEFAULT,
        "ANSI paints it as 39"
    );
    for color in [
        0,
        0x10,
        0x01_00_00_FF,
        0x02_FF_FF_FF,
        0x00_12_00_02,
        0x07_00_00_00,
    ] {
        assert!(valid_color(codec_color(color)));
    }
}

#[test]
fn every_modifier_bit_is_preserved() {
    let mut row = text_row("", 16);
    for (bit, cell) in row.iter_mut().enumerate() {
        *cell = styled("m", 0, 0, 1 << bit);
    }
    let grid = full_grid(&frame_from_rows(&[row], &[]));
    for bit in 0..16 {
        assert_eq!(grid.cell(bit, 0).attrs, 1 << bit);
    }
}

#[test]
fn wide_cells_send_a_continuation_with_the_lead_style() {
    let mut row = text_row("", 4);
    row[0] = styled("界", 0, 0x01_00_00_04, 0);
    row[1] = cell("");
    row[2] = cell("a");
    let grid = full_grid(&frame_from_rows(&[row], &[]));
    assert_eq!(
        (grid.cell(0, 0).text.as_str(), grid.cell(0, 0).width),
        ("界", 2)
    );
    let tail = grid.cell(1, 0);
    assert_eq!((tail.text.as_str(), tail.width), ("", 0));
    assert_eq!(
        tail.bg, 0x01_00_00_04,
        "the tail carries the lead background"
    );
    assert_eq!(
        (grid.cell(2, 0).text.as_str(), grid.cell(2, 0).width),
        ("a", 1)
    );
}

#[test]
fn covered_cell_contents_are_hidden_like_the_ansi_encoder() {
    // The ANSI encoder never paints a narrow cell after a wide lead, whatever it holds.
    let mut row = text_row("", 3);
    row[0] = cell("界");
    row[1] = cell("x");
    let grid = full_grid(&frame_from_rows(&[row], &[]));
    assert_eq!(
        (grid.cell(1, 0).text.as_str(), grid.cell(1, 0).width),
        ("", 0)
    );
    let mut skipped = text_row("", 3);
    skipped[0] = cell("界");
    skipped[1].skip = true;
    let grid = full_grid(&frame_from_rows(&[skipped], &[]));
    assert_eq!(grid.cell(1, 0).width, 0);
}

#[test]
fn clipped_wide_edge_keeps_its_authoritative_width() {
    let mut row = text_row("ab", 3);
    row[2] = cell("界");
    let grid = full_grid(&frame_from_rows(&[row], &[]));
    assert_eq!(
        (grid.cell(2, 0).text.as_str(), grid.cell(2, 0).width),
        ("界", 2)
    );
}

#[test]
fn halfwidth_voiced_katakana_uses_the_ansi_grapheme_width() {
    let mut row = text_row("", 3);
    row[0] = cell("ｶﾞ");
    row[1] = cell("");
    let grid = full_grid(&frame_from_rows(&[row], &[]));
    assert_eq!(grid.cell(0, 0).width, 2);
    assert_eq!(grid.cell(1, 0).width, 0);
}

#[test]
fn combining_and_zwj_clusters_round_trip_as_one_cell() {
    let family = "👨\u{200d}👩\u{200d}👧";
    let mut row = text_row("", 5);
    row[0] = cell("e\u{301}");
    row[1] = cell(family);
    row[2] = cell("");
    let grid = full_grid(&frame_from_rows(&[row], &[]));
    assert_eq!(grid.cell(0, 0).text, "e\u{301}");
    assert_eq!(grid.cell(0, 0).width, 1);
    assert_eq!(grid.cell(1, 0).text, family);
    assert_eq!(grid.cell(1, 0).width, 2);
    assert_eq!(grid.cell(2, 0).width, 0);
}

#[test]
fn long_cluster_text_is_carried_whole() {
    let zalgo = format!("a{}", "\u{301}".repeat(2000));
    let mut row = text_row("", 2);
    row[0] = cell(&zalgo);
    let grid = full_grid(&frame_from_rows(&[row], &[]));
    assert_eq!(grid.cell(0, 0).text, zalgo);

    let mut too_long = text_row("", 2);
    too_long[0] = cell(&format!(
        "a{}",
        "\u{301}".repeat(MAX_CELL_TEXT_BYTES / 2 + 1)
    ));
    assert_eq!(
        unsupported(&frame_from_rows(&[too_long], &[])),
        "cell text too long"
    );
}

#[test]
fn hyperlinks_match_the_ansi_encoder_sanitising() {
    let long = format!("https://example.com/{}", "a".repeat(10_000));
    let row = vec![
        linked("a", 0),
        linked("b", 0),
        linked("c", 1),
        linked("d", 2),
        linked("e", 3),
        linked("f", 9),
    ];
    let frame = frame_from_rows(
        &[row],
        &[
            "https://one.example",
            &long,
            "",
            "https://two\x07.example\x1b",
        ],
    );
    let grid = full_grid(&frame);
    assert_eq!(grid.link_uri(grid.cell(0, 0)), Some("https://one.example"));
    assert_eq!(grid.cell(0, 0).link, grid.cell(1, 0).link);
    assert_eq!(
        grid.link_uri(grid.cell(2, 0)),
        Some(long.as_str()),
        "long URIs stay whole"
    );
    assert_eq!(
        grid.link_uri(grid.cell(3, 0)),
        None,
        "ANSI writes no link for an empty URI"
    );
    assert_eq!(grid.link_uri(grid.cell(4, 0)), Some("https://two.example"));
    assert_eq!(
        grid.link_uri(grid.cell(5, 0)),
        None,
        "ANSI ignores a missing table entry"
    );

    let oversized = format!("https://example.com/{}", "a".repeat(MAX_URI_BYTES));
    let frame = frame_from_rows(&[vec![linked("a", 0)]], &[&oversized]);
    assert_eq!(unsupported(&frame), "hyperlink too long");
}

#[test]
fn unrepresentable_frames_are_reported_instead_of_altered() {
    let base = || frame_from_rows(&[text_row("abc", 4)], &[]);

    let mut skipped = base();
    skipped.cells[1].skip = true;
    assert_eq!(unsupported(&skipped), "skipped cell");

    let mut empty = base();
    empty.cells[1].symbol.clear();
    assert_eq!(unsupported(&empty), "empty cell");

    let mut zero_width = base();
    zero_width.cells[1].symbol = "\u{301}".into();
    assert_eq!(unsupported(&zero_width), "zero-width cell");

    let mut triple = base();
    triple.cells[0].symbol = "界界".into();
    assert_eq!(unsupported(&triple), "cell wider than two columns");

    let mut stacked = base();
    stacked.cells[0].symbol = "界".into();
    stacked.cells[1].symbol = "界".into();
    assert_eq!(unsupported(&stacked), "wide cell under a wide lead");

    let mut graphics = base();
    graphics.graphics = b"\x1b_Gq=2;\x1b\\".to_vec();
    assert_eq!(unsupported(&graphics), "terminal graphics");

    let mut short = base();
    short.cells.pop();
    assert_eq!(unsupported(&short), "frame dimensions");

    let wide = FrameData {
        cells: vec![cell(" "); usize::from(MAX_DIMENSION) + 1],
        width: MAX_DIMENSION + 1,
        height: 1,
        cursor: None,
        hyperlinks: Vec::new(),
        graphics: Vec::new(),
    };
    assert_eq!(unsupported(&wide), "frame dimensions");
}

#[test]
fn oversized_records_are_reported_instead_of_split() {
    let symbol = format!("a{}", "\u{301}".repeat(4000));
    let row_count = (MAX_RECORD_BYTES / symbol.len()).div_ceil(40) + 1;
    let rows: Vec<Vec<CellData>> = (0..row_count).map(|_| vec![cell(&symbol); 40]).collect();
    let frame = frame_from_rows(&rows, &[]);
    assert_eq!(unsupported(&frame), "frame record too large");
}

#[test]
fn encode_rejection_leaves_the_committed_baseline_unchanged() {
    let mut encoder = CellsEncoder::new();
    let mut grid = ReferenceGrid::default();
    let first = frame_from_rows(&[vec![linked("a", 0), cell("b")]], &["https://one"]);
    send(&mut encoder, &mut grid, &first, 1);

    let mut rejected = first.clone();
    rejected.cells[1].skip = true;
    assert!(encoder.encode(&rejected, false, 2, 1).is_err());
    assert!(encoder.is_current(&first));

    let next = frame_from_rows(&[vec![linked("a", 0), cell("c")]], &["https://one"]);
    let encoded = encoder.encode(&next, false, 3, 1).unwrap();
    assert!(!encoded.full, "the old baseline still diffs");
    assert!(
        decode(&encoded.bytes).unwrap().links.is_empty(),
        "link ids survive"
    );
    grid.apply(&encoded.bytes).unwrap();
    encoder.commit(next.clone(), encoded);
    assert_eq!(resolved(&grid), resolved(&full_grid(&next)));
}

#[test]
fn dropped_and_deferred_frames_do_not_move_the_baseline() {
    let mut encoder = CellsEncoder::new();
    let mut grid = ReferenceGrid::default();
    let first = frame_from_rows(&[text_row("one", 4)], &["https://x"]);
    send(&mut encoder, &mut grid, &first, 1);

    // Prepared but never committed: deferred behind a full render lane, or dropped.
    let mut dropped = frame_from_rows(
        &[vec![linked("t", 0), cell("w"), cell("o"), cell(" ")]],
        &["https://new"],
    );
    dropped.cursor = None;
    let _ = encoder.encode(&dropped, false, 2, 1).unwrap();
    let _ = encoder.encode(&dropped, true, 2, 0).unwrap();

    let next = frame_from_rows(&[text_row("thr", 4)], &[]);
    let encoded = encoder.encode(&next, false, 2, 1).unwrap();
    assert!(!encoded.full);
    grid.apply(&encoded.bytes).unwrap();
    encoder.commit(next.clone(), encoded);
    assert_eq!(resolved(&grid), resolved(&full_grid(&next)));
    assert_eq!(grid.cursor, next.cursor);
}

#[test]
fn cursor_is_parked_where_the_ansi_encoder_parks_it() {
    let mut encoder = CellsEncoder::new();
    let mut grid = ReferenceGrid::default();
    let mut frame = frame_from_rows(&[text_row("abc", 4), text_row("", 4)], &[]);
    frame.cursor = Some(CursorState {
        x: 9,
        y: 9,
        visible: true,
        shape: 9,
    });
    send(&mut encoder, &mut grid, &frame, 1);
    assert_eq!(
        grid.cursor,
        Some(CursorState {
            x: 3,
            y: 1,
            visible: true,
            shape: 0,
        }),
        "position clamped and an unknown shape becomes the default"
    );

    frame.cursor = visible_cursor(2, 0);
    send(&mut encoder, &mut grid, &frame, 2);
    frame.cursor = None;
    send(&mut encoder, &mut grid, &frame, 3);
    assert_eq!(
        grid.cursor,
        Some(CursorState {
            x: 2,
            y: 0,
            visible: false,
            shape: 0,
        }),
        "a missing cursor hides at the last visible position"
    );

    assert_eq!(
        full_grid(&frame).cursor,
        Some(CursorState {
            x: 3,
            y: 1,
            visible: false,
            shape: 0,
        }),
        "without history it parks at the bottom-right cell"
    );
}

#[test]
fn patches_reuse_known_link_ids_and_add_new_ones() {
    let mut encoder = CellsEncoder::new();
    let mut grid = ReferenceGrid::default();
    let first = frame_from_rows(&[vec![linked("a", 0), cell(" ")]], &["https://one"]);
    send(&mut encoder, &mut grid, &first, 1);
    let first_id = grid.cell(0, 0).link;

    // Same URI under a different frame-table index plus a new URI.
    let second = frame_from_rows(
        &[vec![linked("a", 1), linked("b", 0)]],
        &["https://two", "https://one"],
    );
    let encoded = encoder.encode(&second, false, 2, grid.seq).unwrap();
    assert!(!encoded.full);
    let decoded = decode(&encoded.bytes).unwrap();
    assert_eq!(decoded.links.len(), 1, "only the new URI is sent");
    assert_eq!(decoded.spans.len(), 1, "column 0 is unchanged");
    grid.apply(&encoded.bytes).unwrap();
    encoder.commit(second, encoded);
    assert_eq!(grid.cell(0, 0).link, first_id);
    assert_eq!(grid.link_uri(grid.cell(1, 0)), Some("https://two"));
}

#[test]
fn link_budget_overflow_in_a_patch_falls_back_to_a_full_frame() {
    let mut encoder = CellsEncoder::new();
    let mut grid = ReferenceGrid::default();
    let frame = many_links(MAX_LINKS);
    assert!(send(&mut encoder, &mut grid, &frame, 1));
    assert_eq!(grid.links.len(), MAX_LINKS);

    let mut next = frame.clone();
    next.hyperlinks.push("https://l/new".into());
    next.cells[0].hyperlink = Some(MAX_LINKS as u32);
    assert!(
        send(&mut encoder, &mut grid, &next, 2),
        "a full frame resets the table"
    );
    assert_eq!(grid.link_uri(grid.cell(0, 0)), Some("https://l/new"));
    assert_eq!(resolved(&grid), resolved(&full_grid(&next)));
}

#[test]
fn more_distinct_links_than_one_table_holds_are_reported() {
    assert_eq!(
        unsupported(&many_links(MAX_LINKS + 1)),
        "too many hyperlinks"
    );
}

#[test]
fn diff_sends_only_changed_runs() {
    let mut encoder = CellsEncoder::new();
    let mut grid = ReferenceGrid::default();
    let before = frame_from_rows(&[text_row("hello world", 12), text_row("x", 12)], &[]);
    send(&mut encoder, &mut grid, &before, 1);
    let after = frame_from_rows(&[text_row("hello wOrld", 12), text_row("x", 12)], &[]);
    let encoded = encoder.encode(&after, false, 2, 1).unwrap();
    let decoded = decode(&encoded.bytes).unwrap();
    assert!(!decoded.full);
    assert_eq!(decoded.base_seq, 1);
    assert_eq!(decoded.spans.len(), 1);
    assert_eq!((decoded.spans[0].y, decoded.spans[0].x), (0, 7));
    assert_eq!(decoded.spans[0].cells.len(), 1);
}

#[test]
fn width_transition_resends_the_former_continuation_cell() {
    let mut encoder = CellsEncoder::new();
    let mut grid = ReferenceGrid::default();
    let mut wide = text_row("", 4);
    wide[0] = cell("界");
    wide[1] = cell("");
    let before = frame_from_rows(&[wide], &[]);
    send(&mut encoder, &mut grid, &before, 1);
    let after = frame_from_rows(&[text_row("ab", 4)], &[]);
    send(&mut encoder, &mut grid, &after, 2);
    assert_eq!(
        (grid.cell(1, 0).text.as_str(), grid.cell(1, 0).width),
        ("b", 1)
    );
    assert_eq!(resolved(&grid), resolved(&full_grid(&after)));
}

#[test]
fn size_change_and_repaint_encode_full_frames() {
    let mut encoder = CellsEncoder::new();
    let frame = frame_from_rows(&[text_row("a", 3)], &[]);
    let encoded = encoder.encode(&frame, false, 1, 0).unwrap();
    encoder.commit(frame, encoded);
    let same = frame_from_rows(&[text_row("b", 3)], &[]);
    assert!(encoder.encode(&same, true, 2, 1).unwrap().full);
    let bigger = frame_from_rows(&[text_row("b", 4)], &[]);
    assert!(encoder.encode(&bigger, false, 2, 1).unwrap().full);
}

/// Deterministic generator for adversarial frames.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }
}

const URIS: &[&str] = &["https://a", "https://b", "", "https://c\x07d"];

/// Restores the shape Herdr's terminal renderer produces: an empty spacer after
/// every wide lead and a visible symbol everywhere else.
fn normalize(frame: &mut FrameData, narrow_last_column: bool) {
    let width = usize::from(frame.width);
    for row in frame.cells.chunks_mut(width) {
        let mut covered = false;
        for (col, cell) in row.iter_mut().enumerate() {
            if covered {
                cell.symbol.clear();
                covered = false;
                continue;
            }
            if cell.symbol.is_empty()
                || (narrow_last_column && col + 1 == width && cell_width(cell) == 2)
            {
                cell.symbol = "a".into();
            }
            covered = cell_width(cell) == 2;
        }
    }
}

fn random_frame(rng: &mut Lcg, width: u16, height: u16, symbols: &[&str]) -> FrameData {
    let hyperlinks: Vec<String> = URIS
        .iter()
        .skip(rng.next(2) as usize)
        .map(|uri| (*uri).to_owned())
        .collect();
    let cells = (0..usize::from(width) * usize::from(height))
        .map(|_| CellData {
            symbol: symbols[rng.next(symbols.len() as u64) as usize].to_owned(),
            fg: [0, 0x00_00_00_03, 0x01_00_00_20, 0x02_01_02_03][rng.next(4) as usize],
            bg: [0, 0x00_00_00_10, 0x02_FF_00_00][rng.next(3) as usize],
            modifier: [0, 1, 0x48, 0x3008][rng.next(4) as usize],
            skip: false,
            // One index past the table exercises a missing entry.
            hyperlink: (rng.next(4) == 0).then(|| rng.next(hyperlinks.len() as u64 + 1) as u32),
        })
        .collect();
    FrameData {
        cells,
        width,
        height,
        cursor: (rng.next(4) != 0).then(|| CursorState {
            x: rng.next(u64::from(width) + 2) as u16,
            y: rng.next(u64::from(height)) as u16,
            visible: rng.next(2) == 0,
            shape: rng.next(8) as u8,
        }),
        hyperlinks,
        graphics: Vec::new(),
    }
}

fn mutate(rng: &mut Lcg, frame: &FrameData, symbols: &[&str]) -> FrameData {
    let mut next = random_frame(rng, frame.width, frame.height, symbols);
    // Keep most cells so diffs are sparse, as in typing and scrolling.
    for (index, cell) in next.cells.iter_mut().enumerate() {
        if rng.next(8) != 0 {
            *cell = frame.cells[index].clone();
            if let Some(uri) = cell
                .hyperlink
                .and_then(|link| frame.hyperlinks.get(link as usize))
            {
                cell.hyperlink = next
                    .hyperlinks
                    .iter()
                    .position(|known| known == uri)
                    .map(|index| index as u32);
            }
        }
    }
    next
}

const ALL_SYMBOLS: &[&str] = &[
    " ", "a", "Z", "界", "", "e\u{301}", "👍", "ｶﾞ", "─", "🇧🇷", "x",
];

fn expected_cursor(frame: &FrameData, last_visible: &mut Option<(u16, u16)>) -> CursorState {
    let host = resolve_host_cursor_state(frame, last_visible);
    CursorState {
        x: host.position.0,
        y: host.position.1,
        visible: host.visible,
        shape: host.shape,
    }
}

#[test]
fn patched_stream_matches_a_fresh_full_frame_for_random_sequences() {
    let mut rng = Lcg(0x5eed);
    for _ in 0..40 {
        let width = 1 + rng.next(30) as u16;
        let height = 1 + rng.next(6) as u16;
        let mut encoder = CellsEncoder::new();
        let mut grid = ReferenceGrid::default();
        let mut last_visible = None;
        let mut frame = random_frame(&mut rng, width, height, ALL_SYMBOLS);
        normalize(&mut frame, false);
        let mut seq = 1;
        send(&mut encoder, &mut grid, &frame, seq);
        assert_eq!(
            grid.cursor,
            Some(expected_cursor(&frame, &mut last_visible))
        );
        for _ in 0..12 {
            frame = mutate(&mut rng, &frame, ALL_SYMBOLS);
            normalize(&mut frame, false);
            // Sequence numbers may skip; only the base must match.
            seq += 1 + rng.next(3);
            send(&mut encoder, &mut grid, &frame, seq);
            assert_eq!(grid.seq, seq);
            assert_eq!(
                grid.cursor,
                Some(expected_cursor(&frame, &mut last_visible))
            );
            assert_eq!(resolved(&grid), resolved(&full_grid(&frame)));
        }
    }
}

#[test]
fn retained_row_patch_matches_a_fresh_full_frame() {
    let narrow = &[" ", "a", "Z", "e\u{301}", "─", "x"];
    let mut rng = Lcg(42);
    for _ in 0..30 {
        let mut encoder = CellsEncoder::new();
        let mut grid = ReferenceGrid::default();
        let mut frame = random_frame(&mut rng, 20, 4, narrow);
        for cell in &mut frame.cells {
            cell.hyperlink = None;
        }
        send(&mut encoder, &mut grid, &frame, 1);
        let replacement = random_frame(&mut rng, 20, 4, narrow);
        let rows: Vec<PaneSurfacePatchRow> = (0..3)
            .map(|y| {
                let x = rng.next(10) as u16;
                let len = 1 + rng.next(9) as usize;
                PaneSurfacePatchRow {
                    x,
                    y,
                    cells: replacement.cells[..len]
                        .iter()
                        .cloned()
                        .map(|cell| CellData {
                            hyperlink: None,
                            ..cell
                        })
                        .collect(),
                }
            })
            .collect();
        let cursor = visible_cursor(3, 2);
        let encoded = encoder.encode_patch(&rows, cursor.clone(), 5, 1).unwrap();
        grid.apply(&encoded.bytes).unwrap();
        assert!(encoder.commit_patch(&rows, cursor.clone(), encoded));
        let patched = encoder.last_frame().unwrap().clone();
        assert_eq!(patched.cursor, cursor);
        assert_eq!(grid.cursor, cursor);
        assert_eq!(resolved(&grid), resolved(&full_grid(&patched)));
        assert_eq!(grid.seq, 5);
    }
}

#[test]
fn retained_row_patch_rejects_unrepresentable_rows() {
    let mut encoder = CellsEncoder::new();
    let frame = frame_from_rows(&[text_row("abc", 4), text_row("def", 4)], &[]);
    let encoded = encoder.encode(&frame, false, 1, 0).unwrap();
    encoder.commit(frame, encoded);
    let row = |x, y, cells: Vec<CellData>| PaneSurfacePatchRow { x, y, cells };
    let mut skipped = cell("a");
    skipped.skip = true;
    let cases = [
        vec![row(3, 0, vec![cell("a"), cell("b")])],
        vec![row(0, 2, vec![cell("a")])],
        vec![row(0, 0, vec![linked("a", 0)])],
        vec![
            row(0, 0, vec![cell("a"), cell("b")]),
            row(1, 0, vec![cell("c")]),
        ],
        vec![row(1, 1, vec![skipped])],
        vec![row(1, 1, vec![cell("")])],
    ];
    for rows in cases {
        assert!(
            encoder.encode_patch(&rows, None, 2, 1).is_none(),
            "{rows:?}"
        );
    }
    assert!(CellsEncoder::new()
        .encode_patch(&[row(0, 0, vec![cell("a")])], None, 2, 1)
        .is_none());
}

/// Replays the ANSI encoder's bytes for the same frames through Ghostty and
/// compares what each path displays, cell by cell, under both host wrap policies.
#[test]
fn display_matches_the_ansi_encoder_through_a_real_terminal() {
    // Symbols whose width every terminal agrees on.
    let symbols = &[" ", "a", "Z", "界", "", "e\u{301}", "─", "x", "é"];
    for host_setup in [&b"\x1b[?7l"[..], &b""[..]] {
        let mut rng = Lcg(0xA115);
        for _ in 0..12 {
            let width = 2 + rng.next(24) as u16;
            let height = 1 + rng.next(5) as u16;
            let mut terminal = crate::ghostty::Terminal::new(width, height, 0).unwrap();
            terminal.write(host_setup);
            let mut ansi = BlitEncoder::new();
            let mut encoder = CellsEncoder::new();
            let mut grid = ReferenceGrid::default();
            let mut frame = random_frame(&mut rng, width, height, symbols);
            for seq in 1..10 {
                if seq > 1 {
                    frame = mutate(&mut rng, &frame, symbols);
                }
                normalize(&mut frame, true);
                if host_setup == b"\x1b[?7l" {
                    keep_marks_off_the_column_before_the_last(&mut frame);
                }
                let encoded = ansi.encode(&frame, false);
                terminal.write(&encoded.bytes);
                ansi.commit(frame.clone(), encoded);
                send(&mut encoder, &mut grid, &frame, seq);
                assert_display_parity(&terminal, &grid);
            }
        }
    }
}

/// Without autowrap, Ghostty cannot tell a cursor parked in the last column
/// after printing into the column before it from one that printed the last
/// column itself: a combining mark that follows attaches to the last column
/// whenever that column already holds text (Terminal.zig `print`, grapheme
/// branch). The ANSI bytes then display the mark one cell too far right, so
/// that host cannot judge a cluster in that column. The autowrap pass keeps
/// such clusters in every column.
fn keep_marks_off_the_column_before_the_last(frame: &mut FrameData) {
    let width = usize::from(frame.width);
    for row in frame.cells.chunks_mut(width) {
        let cell = &mut row[width - 2];
        if cell.symbol.chars().count() > 1 && cell_width(cell) == 1 {
            cell.symbol = "a".into();
        }
    }
}

fn assert_display_parity(terminal: &crate::ghostty::Terminal, grid: &ReferenceGrid) {
    let blank = |text: &str| {
        if text.is_empty() {
            " ".to_owned()
        } else {
            text.to_owned()
        }
    };
    for y in 0..grid.height {
        for x in 0..grid.width {
            let (wide, graphemes) = terminal.screen_cell(x, u32::from(y)).unwrap();
            let text: String = graphemes
                .iter()
                .filter_map(|c| char::from_u32(*c))
                .collect();
            let cell = grid.cell(x, y);
            let at = format!("cell {x},{y}");
            match wide {
                crate::ghostty::CellWide::SpacerTail => assert_eq!(cell.width, 0, "{at}"),
                crate::ghostty::CellWide::Wide => {
                    assert_eq!((cell.width, cell.text.as_str()), (2, text.as_str()), "{at}");
                }
                crate::ghostty::CellWide::Narrow => {
                    assert_eq!(cell.width, 1, "{at}");
                    assert_eq!(blank(&cell.text), blank(&text), "{at}");
                }
                crate::ghostty::CellWide::SpacerHead => panic!("unexpected spacer head at {at}"),
            }
            if cell.width != 0 {
                let uri = terminal.viewport_hyperlink_uri(x, u32::from(y)).unwrap();
                assert_eq!(uri.as_deref(), grid.link_uri(cell), "{at}");
            }
        }
    }
}

fn sample_record() -> Vec<u8> {
    let mut row = text_row("ab", 4);
    row[2] = linked("界", 0);
    row[3] = cell("");
    // One grapheme cluster per cell: text_row would split the combining mark
    // into a zero-width cell of its own, which the codec reports as unsupported.
    let combining = vec![cell("e\u{301}"), cell(" "), cell(" "), cell(" ")];
    let frame = frame_from_rows(&[row, combining], &["https://x"]);
    CellsEncoder::new()
        .encode(&frame, false, 3, 0)
        .unwrap()
        .bytes
}

#[test]
fn every_truncated_prefix_is_rejected_without_panicking() {
    let record = sample_record();
    assert!(decode(&record).is_ok());
    for len in 0..record.len() {
        assert!(decode(&record[..len]).is_err(), "prefix {len}");
    }
    let mut trailing = record.clone();
    trailing.push(0);
    assert_eq!(
        decode(&trailing),
        Err(DecodeError::Malformed("trailing bytes"))
    );
    assert_eq!(
        decode(&vec![0; MAX_RECORD_BYTES + 1]),
        Err(DecodeError::TooLarge)
    );
}

#[test]
fn corrupt_records_are_rejected() {
    let record = sample_record();
    let mutate = |at: usize, value: &[u8]| {
        let mut bytes = record.clone();
        bytes[at..at + value.len()].copy_from_slice(value);
        decode(&bytes)
    };
    assert!(mutate(0, b"XTC1").is_err(), "magic");
    assert!(mutate(4, &[2]).is_err(), "kind");
    assert!(mutate(5, &[0x80]).is_err(), "flags");
    assert!(mutate(6, &0u64.to_le_bytes()).is_err(), "zero seq");
    assert!(
        mutate(14, &1u64.to_le_bytes()).is_err(),
        "full frame with base"
    );
    assert!(mutate(22, &0u16.to_le_bytes()).is_err(), "zero width");
    assert!(
        mutate(24, &(MAX_DIMENSION + 1).to_le_bytes()).is_err(),
        "height"
    );
    assert!(mutate(30, &[7]).is_err(), "cursor shape");
    // Link table: count, then id | len | uri.
    assert!(
        mutate(HEADER_BYTES + 4, &0u32.to_le_bytes()).is_err(),
        "link id 0"
    );
    assert!(mutate(HEADER_BYTES, &(MAX_LINKS as u32 + 1).to_le_bytes()).is_err());
    assert!(
        mutate(HEADER_BYTES, &2u32.to_le_bytes()).is_err(),
        "lying link count"
    );
    let uri_at = HEADER_BYTES + 4 + 4 + 2;
    assert!(mutate(uri_at, &[0xFF]).is_err(), "invalid utf-8 uri");
    let spans_at = uri_at + "https://x".len();
    assert!(
        mutate(spans_at, &1u32.to_le_bytes()).is_err(),
        "full span count"
    );
    assert!(
        mutate(spans_at, &u32::MAX.to_le_bytes()).is_err(),
        "huge span count"
    );
    // First span header: y | x | count.
    assert!(
        mutate(spans_at + 4, &1u16.to_le_bytes()).is_err(),
        "row order"
    );
    assert!(
        mutate(spans_at + 6, &1u16.to_le_bytes()).is_err(),
        "x offset"
    );
    assert!(
        mutate(spans_at + 8, &5u16.to_le_bytes()).is_err(),
        "span width"
    );
    // First cell tag: width 3.
    let tag = record[spans_at + 10];
    assert!(mutate(spans_at + 10, &[tag | 0x0C]).is_err(), "width 3");
}

fn one_cell_patch(cell: &[u8]) -> Result<DecodedFrame, DecodeError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&[KIND_PATCH, 0]);
    bytes.extend_from_slice(&2u64.to_le_bytes());
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(cell);
    decode(&bytes)
}

#[test]
fn malformed_cell_payloads_are_rejected() {
    let width_1 = 1 << TAG_WIDTH_SHIFT;
    let one_byte = 1 << TAG_TEXT_SHIFT;
    assert!(one_cell_patch(&[width_1 | one_byte, b'a']).is_ok());
    assert!(
        one_cell_patch(&[one_byte, b'a']).is_err(),
        "continuation text"
    );
    assert!(
        one_cell_patch(&[width_1]).is_err(),
        "visible cell without text"
    );
    assert!(one_cell_patch(&[width_1 | (2 << TAG_TEXT_SHIFT), 0xC3, 0x28]).is_err());

    let styled_cell = |fg: u32| {
        let mut bytes = vec![width_1 | one_byte | TAG_STYLE];
        bytes.extend_from_slice(&fg.to_le_bytes());
        bytes.extend_from_slice(&[0; 10]);
        bytes.push(b'a');
        one_cell_patch(&bytes)
    };
    assert!(styled_cell(0x00_00_00_10).is_ok());
    assert!(styled_cell(0x02_12_34_56).is_ok());
    assert!(styled_cell(0x00_00_00_11).is_err(), "named color range");
    assert!(styled_cell(0x09_00_00_00).is_err(), "color tag");
    assert!(styled_cell(0x01_00_01_00).is_err(), "indexed color range");
}

#[test]
fn patches_apply_atomically_and_only_to_their_base() {
    let mut encoder = CellsEncoder::new();
    let mut grid = ReferenceGrid::default();
    let first = frame_from_rows(&[text_row("one", 4)], &[]);
    send(&mut encoder, &mut grid, &first, 1);
    let snapshot = resolved(&grid);

    let second = frame_from_rows(&[text_row("two", 4)], &[]);
    let wrong_base = encoder.encode(&second, false, 9, 8).unwrap();
    assert_eq!(grid.apply(&wrong_base.bytes), Err("base sequence mismatch"));
    assert_eq!((resolved(&grid), grid.seq), (snapshot.clone(), 1));

    let linked_frame = frame_from_rows(
        &[vec![cell("t"), cell("w"), linked("o", 0), cell(" ")]],
        &["https://u"],
    );
    let diff = encoder.encode(&linked_frame, false, 2, 1).unwrap();
    let mut stripped = decode(&diff.bytes).unwrap();
    assert_eq!(stripped.links.len(), 1);
    let announced = stripped.links.clone();
    stripped.links.clear();
    assert_eq!(grid.apply(&reencode(&stripped)), Err("unknown link id"));
    assert_eq!(
        resolved(&grid),
        snapshot,
        "a rejected patch changes nothing"
    );

    grid.apply(&diff.bytes).unwrap();
    encoder.commit(linked_frame, diff);
    let after_link = resolved(&grid);
    let mut reused = decode(&encoder.encode(&second, false, 3, 2).unwrap().bytes).unwrap();
    reused.links = vec![(announced[0].0, "https://other".into())];
    assert_eq!(grid.apply(&reencode(&reused)), Err("link id reused"));
    assert_eq!(resolved(&grid), after_link);

    let full_again = encoder.encode(&second, true, 20, 0).unwrap();
    grid.apply(&full_again.bytes).unwrap();
    assert_eq!(grid.seq, 20, "a full frame resynchronises at any sequence");
    assert_eq!(resolved(&grid), resolved(&full_grid(&second)));
}

/// Test-only writer for a decoded frame, used to build adversarial variants.
fn reencode(frame: &DecodedFrame) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.push(if frame.full { KIND_FULL } else { KIND_PATCH });
    let mut flags = 0;
    if let Some(cursor) = &frame.cursor {
        flags |= FLAG_CURSOR_PRESENT;
        if cursor.visible {
            flags |= FLAG_CURSOR_VISIBLE;
        }
    }
    bytes.push(flags);
    bytes.extend_from_slice(&frame.seq.to_le_bytes());
    bytes.extend_from_slice(&frame.base_seq.to_le_bytes());
    bytes.extend_from_slice(&frame.width.to_le_bytes());
    bytes.extend_from_slice(&frame.height.to_le_bytes());
    let (x, y, shape) = frame
        .cursor
        .as_ref()
        .map_or((0, 0, 0), |cursor| (cursor.x, cursor.y, cursor.shape));
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes.extend_from_slice(&[shape, 0]);
    bytes.extend_from_slice(&(frame.links.len() as u32).to_le_bytes());
    for (id, uri) in &frame.links {
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&(uri.len() as u16).to_le_bytes());
        bytes.extend_from_slice(uri.as_bytes());
    }
    bytes.extend_from_slice(&(frame.spans.len() as u32).to_le_bytes());
    for span in &frame.spans {
        bytes.extend_from_slice(&span.y.to_le_bytes());
        bytes.extend_from_slice(&span.x.to_le_bytes());
        bytes.extend_from_slice(&(span.cells.len() as u16).to_le_bytes());
        for cell in &span.cells {
            let text = cell.text.as_bytes();
            let extended = text.len() >= usize::from(TAG_TEXT_EXTENDED);
            let len_bits = if extended {
                TAG_TEXT_EXTENDED
            } else {
                text.len() as u8
            };
            bytes.push(
                TAG_STYLE
                    | TAG_LINK
                    | ((cell.width & 0x03) << TAG_WIDTH_SHIFT)
                    | (len_bits << TAG_TEXT_SHIFT),
            );
            if extended {
                bytes.extend_from_slice(&(text.len() as u16).to_le_bytes());
            }
            bytes.extend_from_slice(&cell.fg.to_le_bytes());
            bytes.extend_from_slice(&cell.bg.to_le_bytes());
            bytes.extend_from_slice(&cell.underline_color.to_le_bytes());
            bytes.extend_from_slice(&cell.attrs.to_le_bytes());
            bytes.extend_from_slice(&cell.link.to_le_bytes());
            bytes.extend_from_slice(text);
        }
    }
    bytes
}

#[test]
fn reencoded_records_decode_identically() {
    let record = sample_record();
    let decoded = decode(&record).unwrap();
    assert_eq!(decode(&reencode(&decoded)).unwrap(), decoded);
}

#[test]
fn frame_controls_round_trip_through_base64() {
    let record = sample_record();
    let ServerMessage::EndpointControl { kind, data } = frame_message(&record) else {
        panic!("frame control");
    };
    assert_eq!(kind, FRAME_KIND);
    assert_eq!(decode_frame_message(&data).unwrap(), record);
    assert!(decode_frame_message("not base64!").is_err());
}

#[test]
fn negotiation_messages_use_named_controls() {
    let crate::protocol::ClientMessage::EndpointControl { kind, data } = hello_message().unwrap()
    else {
        panic!("hello control");
    };
    assert_eq!(kind, HELLO_KIND);
    let hello: CellsHello = serde_json::from_str(&data).unwrap();
    assert!(hello.accepts_current());
    let future: CellsHello =
        serde_json::from_str(r#"{"codecs":["herdr.terminal.cells.v9"],"extra":true}"#).unwrap();
    assert!(!future.accepts_current());
    let empty: CellsHello = serde_json::from_str("{}").unwrap();
    assert!(!empty.accepts_current());

    let ServerMessage::EndpointControl { kind, data } = welcome_message(&CellsWelcome {
        codec: Some(CODEC.into()),
        error: None,
    })
    .unwrap() else {
        panic!("welcome control");
    };
    assert_eq!(kind, WELCOME_KIND);
    assert_eq!(data, r#"{"codec":"herdr.terminal.cells.v1"}"#);
}
