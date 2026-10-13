//! Retained, immutable copy of a pane's visible screen.
//!
//! The pane core refreshes one `TerminalScreen` from Ghostty's dirty rows and
//! every display consumer reads it: the TUI buffer, helper frames, hyperlink
//! scans, and retained dirty patches. Unchanged rows are shared between
//! generations, so a consumer can find what changed by comparing row
//! revisions instead of cells.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use unicode_width::UnicodeWidthStr;

use super::TerminalCursorState;
use crate::protocol::{CellData, CursorState, FrameData};

/// `ScreenCell::content` indexes `ScreenRow::clusters` instead of holding a
/// Unicode scalar.
pub(crate) const CELL_CLUSTER: u8 = 1 << 0;
/// The cell's OSC 8 symbol differs from its display symbol; the OSC 8 symbol
/// is in `ScreenRow::link_symbols`. Frames do not attach such links.
pub(crate) const CELL_LINK_DETACHED: u8 = 1 << 1;

/// One display cell. Colors use the generation-1 wire encoding
/// (`protocol::color_to_u32`, Reset = 0) and `modifier` holds
/// `protocol::modifier_to_u16` bits including the underline-style nibble.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(C)]
pub(crate) struct ScreenCell {
    /// Unicode scalar of a one-scalar symbol, 0 for the empty symbol of a wide
    /// spacer tail, or a `clusters` index with `CELL_CLUSTER`.
    pub content: u32,
    pub fg: u32,
    pub bg: u32,
    pub underline_color: u32,
    pub modifier: u16,
    pub flags: u8,
    // Keeps the cell a padding-free 24-byte POD for GPU upload; never read.
    #[allow(dead_code)]
    pub reserved: u8,
    /// 0 = no hyperlink; n = `ScreenRow::links[n - 1]`.
    pub link: u32,
}

impl ScreenCell {
    pub(crate) const fn blank(default_fg: u32, default_bg: u32) -> Self {
        Self {
            content: ' ' as u32,
            fg: default_fg,
            bg: default_bg,
            underline_color: 0,
            modifier: 0,
            flags: 0,
            reserved: 0,
            link: 0,
        }
    }

    /// Equal to ratatui's empty cell, which a fresh render diff never writes.
    fn is_ratatui_empty(&self) -> bool {
        self.content == ' ' as u32
            && self.flags & CELL_CLUSTER == 0
            && self.fg == 0
            && self.bg == 0
            && self.underline_color == 0
            && self.modifier == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScreenRow {
    /// Process-unique; a row with new content always gets a new revision.
    pub revision: u64,
    /// Exactly `TerminalScreen::cols` cells.
    pub cells: Box<[ScreenCell]>,
    /// Symbols longer than one scalar, rare.
    pub clusters: Vec<Box<str>>,
    /// OSC 8 URIs used by this row.
    pub links: Vec<Arc<str>>,
    /// OSC 8 symbols of `CELL_LINK_DETACHED` cells, by column.
    pub link_symbols: Vec<(u16, Box<str>)>,
    /// Ghostty row selection, inclusive columns.
    pub selection: Option<(u16, u16)>,
}

impl ScreenRow {
    pub(crate) fn symbol<'a>(&'a self, cell: &ScreenCell, scratch: &'a mut [u8; 4]) -> &'a str {
        if cell.flags & CELL_CLUSTER != 0 {
            return self
                .clusters
                .get(cell.content as usize)
                .map_or(" ", |cluster| cluster);
        }
        if cell.content == 0 {
            return "";
        }
        char::from_u32(cell.content).map_or(" ", |ch| ch.encode_utf8(scratch))
    }

    pub(crate) fn link_uri(&self, cell: &ScreenCell) -> Option<&Arc<str>> {
        let index = usize::try_from(cell.link).ok()?.checked_sub(1)?;
        self.links.get(index)
    }

    /// The symbol Ghostty reports for an OSC 8 cell, which skips display
    /// width normalization.
    pub(crate) fn link_symbol<'a>(
        &'a self,
        x: u16,
        cell: &ScreenCell,
        scratch: &'a mut [u8; 4],
    ) -> &'a str {
        if cell.flags & CELL_LINK_DETACHED != 0 {
            if let Some((_, symbol)) = self.link_symbols.iter().find(|(column, _)| *column == x) {
                return symbol;
            }
        }
        self.symbol(cell, scratch)
    }

    pub(crate) fn has_links(&self) -> bool {
        !self.links.is_empty()
    }

    // Used by the extraction-path agreement tests and the GPU comparison lane.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn same_content(&self, other: &Self) -> bool {
        self.cells == other.cells
            && self.clusters == other.clusters
            && self.links == other.links
            && self.link_symbols == other.link_symbols
            && self.selection == other.selection
    }
}

static NEXT_ROW_REVISION: AtomicU64 = AtomicU64::new(1);
static NEXT_SCREEN_GENERATION: AtomicU64 = AtomicU64::new(1);

pub(super) fn next_row_revision() -> u64 {
    NEXT_ROW_REVISION.fetch_add(1, Ordering::Relaxed)
}

pub(super) fn next_screen_generation() -> u64 {
    NEXT_SCREEN_GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// Identity of each row of a helper frame built by
/// `TerminalScreen::frame_data_with_row_revisions`. Two frames with equal
/// `key` whose rows hold equal values have identical cells in those rows,
/// with hyperlinks compared by URI (table indices can differ).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameRowRevisions {
    /// Frame area, grid size and padding colors.
    // Read by the server ANSI encoder's row damage (integration patch) and tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub key: u64,
    /// One value per frame row: the screen row's revision, or 0 for a row
    /// below the terminal grid, which holds only padding.
    #[cfg_attr(not(test), allow(dead_code))]
    pub rows: Vec<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct TerminalScreen {
    pub cols: u16,
    pub rows: u16,
    /// Process-unique; every new snapshot gets a new generation.
    // Read by tests and by consumers in the GPU lane, not by the CPU path.
    #[cfg_attr(not(test), allow(dead_code))]
    pub generation: u64,
    /// Padding style for cells outside the terminal grid, wire encoding.
    pub default_fg: u32,
    pub default_bg: u32,
    /// Ghostty's cursor before Herdr's settle policy.
    pub cursor: Option<TerminalCursorState>,
    pub rows_data: Vec<Arc<ScreenRow>>,
    pub(super) context: super::screen_extract::StyleContext,
}

impl TerminalScreen {
    pub(crate) fn row(&self, y: u16) -> Option<&ScreenRow> {
        self.rows_data.get(usize::from(y)).map(Arc::as_ref)
    }

    /// Rows whose content differs from `previous`, in viewport order. Every
    /// row counts as changed when the grid dimensions differ.
    // The GPU damage lane is the production consumer; tests cover it here.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn changed_rows_since<'a>(
        &'a self,
        previous: &'a TerminalScreen,
    ) -> impl Iterator<Item = u16> + 'a {
        let comparable = self.cols == previous.cols && self.rows == previous.rows;
        self.rows_data
            .iter()
            .enumerate()
            .filter(move |(y, row)| {
                !comparable
                    || previous
                        .rows_data
                        .get(*y)
                        .is_none_or(|old| old.revision != row.revision)
            })
            .filter_map(|(y, _)| u16::try_from(y).ok())
    }

    fn blank(&self) -> ScreenCell {
        ScreenCell::blank(self.default_fg, self.default_bg)
    }

    /// Writes the same cells as the legacy per-cell renderer into `area`.
    pub(crate) fn write_to_buffer(&self, buf: &mut Buffer, area: Rect) {
        let mut scratch = [0u8; 4];
        let blank = self.blank();
        for y in 0..area.height {
            let row = self.row(y);
            for x in 0..area.width {
                let (cell, symbol) =
                    match row.and_then(|row| Some((row, row.cells.get(usize::from(x))?))) {
                        Some((row, cell)) => (cell, row.symbol(cell, &mut scratch)),
                        None => (&blank, " "),
                    };
                let target = &mut buf[(area.x + x, area.y + y)];
                target.reset();
                target.set_symbol(symbol);
                target.fg = crate::protocol::u32_to_color(cell.fg);
                target.bg = crate::protocol::u32_to_color(cell.bg);
                target.underline_color = crate::protocol::u32_to_color(cell.underline_color);
                target.modifier = Modifier::from_bits_retain(cell.modifier);
            }
        }
    }

    /// OSC 8 cells inside `area`: position, OSC 8 symbol, URI.
    pub(crate) fn visible_hyperlinks(&self, area: Rect) -> Vec<((u16, u16), String, String)> {
        let mut links = Vec::new();
        let mut scratch = [0u8; 4];
        for (y, row) in self
            .rows_data
            .iter()
            .take(usize::from(area.height))
            .enumerate()
        {
            if !row.has_links() {
                continue;
            }
            for (x, cell) in row.cells.iter().take(usize::from(area.width)).enumerate() {
                let Some(uri) = row.link_uri(cell) else {
                    continue;
                };
                // Both indices are bounded by u16 area dimensions.
                let (x, y) = (x as u16, y as u16);
                links.push((
                    (area.x + x, area.y + y),
                    row.link_symbol(x, cell, &mut scratch).to_owned(),
                    uri.to_string(),
                ));
            }
        }
        links
    }

    /// Patch cells for row `y`, padded to `width` with the default blank.
    /// OSC 8 cells index `hyperlinks`, which gains any URI it lacks. A link
    /// attaches where a full render would attach it: where its symbol equals
    /// what the row's own backend diff writes there. The diff starts fresh
    /// at the row's first column, as it does after a pane's left border.
    pub(crate) fn patch_cells(
        &self,
        y: u16,
        width: u16,
        hyperlinks: &mut Vec<String>,
    ) -> Vec<CellData> {
        let mut scratch = [0u8; 4];
        let mut link_scratch = [0u8; 4];
        let blank = self.blank();
        let row = self.row(y);
        let mut diff = DiffState::default();
        (0..width)
            .map(|x| {
                let screen_cell = row.and_then(|row| Some((row, row.cells.get(usize::from(x))?)));
                let (cell, symbol) = match screen_cell {
                    Some((row, cell)) => (cell, row.symbol(cell, &mut scratch)),
                    None => (&blank, " "),
                };
                let written =
                    diff.write(cell.is_ratatui_empty(), symbol, symbol_width(cell, symbol));
                let backend_symbol = if written { symbol } else { " " };
                let hyperlink = screen_cell.and_then(|(row, cell)| {
                    let uri: &str = row.link_uri(cell)?;
                    if row.link_symbol(x, cell, &mut link_scratch) != backend_symbol {
                        return None;
                    }
                    let index = match hyperlinks.iter().position(|known| known == uri) {
                        Some(index) => index,
                        None => {
                            hyperlinks.push(uri.to_owned());
                            hyperlinks.len() - 1
                        }
                    };
                    u32::try_from(index).ok()
                });
                CellData {
                    symbol: symbol.to_owned(),
                    fg: cell.fg,
                    bg: cell.bg,
                    modifier: cell.modifier,
                    skip: false,
                    hyperlink,
                }
            })
            .collect()
    }

    /// The frame the legacy helper path produced: a full render into a fresh
    /// ratatui test backend (which only receives cells ratatui's diff emits
    /// against an empty buffer), then OSC 8 attachment where the hyperlink
    /// symbol equals the backend symbol.
    pub(crate) fn frame_data(&self, area: Rect, cursor: Option<CursorState>) -> FrameData {
        self.frame_data_with_row_revisions(area, cursor).0
    }

    /// `frame_data` plus the identity of each frame row. The revisions are
    /// `None` when ratatui's diff state carries from one row into the next
    /// (a wide cell at the right edge), because such a row also depends on
    /// the row above it.
    pub(crate) fn frame_data_with_row_revisions(
        &self,
        area: Rect,
        cursor: Option<CursorState>,
    ) -> (FrameData, Option<FrameRowRevisions>) {
        let width = area.width;
        let height = area.height;
        let total = usize::from(width) * usize::from(height);
        let blank = self.blank();
        let mut cells = Vec::with_capacity(total);
        let mut hyperlinks = Vec::<String>::new();
        let mut hyperlink_indices = HashMap::<&str, u32>::new();
        let mut scratch = [0u8; 4];
        let mut link_scratch = [0u8; 4];

        // Carried across rows like the original.
        let mut diff = DiffState::default();
        let mut row_revisions = Some(Vec::with_capacity(usize::from(height)));
        for y in 0..height {
            let row = self.row(y);
            if diff.carries() {
                row_revisions = None;
            }
            if let Some(revisions) = row_revisions.as_mut() {
                revisions.push(row.map_or(0, |row| row.revision));
            }
            for x in 0..width {
                let screen_cell = row.and_then(|row| Some((row, row.cells.get(usize::from(x))?)));
                let (cell, symbol) = match screen_cell {
                    Some((row, cell)) => (cell, row.symbol(cell, &mut scratch)),
                    None => (&blank, " "),
                };
                let written =
                    diff.write(cell.is_ratatui_empty(), symbol, symbol_width(cell, symbol));
                let backend_symbol = if written { symbol } else { " " };
                let hyperlink = screen_cell.and_then(|(row, cell)| {
                    let uri: &str = row.link_uri(cell)?;
                    if row.link_symbol(x, cell, &mut link_scratch) != backend_symbol {
                        return None;
                    }
                    Some(*hyperlink_indices.entry(uri).or_insert_with(|| {
                        let next = hyperlinks.len() as u32;
                        hyperlinks.push(uri.to_owned());
                        next
                    }))
                });
                cells.push(if written {
                    CellData {
                        symbol: symbol.to_owned(),
                        fg: cell.fg,
                        bg: cell.bg,
                        modifier: cell.modifier,
                        skip: false,
                        hyperlink,
                    }
                } else {
                    CellData {
                        symbol: " ".to_owned(),
                        fg: 0,
                        bg: 0,
                        modifier: 0,
                        skip: false,
                        hyperlink,
                    }
                });
            }
        }

        let frame = FrameData {
            cells,
            width,
            height,
            cursor,
            hyperlinks,
            graphics: Vec::new(),
        };
        let revisions = row_revisions.map(|rows| {
            let mut key = std::hash::DefaultHasher::new();
            (area.width, area.height, self.cols, self.rows).hash(&mut key);
            (self.default_fg, self.default_bg).hash(&mut key);
            FrameRowRevisions {
                key: key.finish(),
                rows,
            }
        });
        (frame, revisions)
    }
}

/// ratatui-core's `Buffer::diff` against an empty previous buffer, one cell
/// at a time in row-major order: which cells a fresh test backend receives.
#[derive(Default)]
struct DiffState {
    invalidated: usize,
    to_skip: usize,
    forced_end: usize,
    index: usize,
}

impl DiffState {
    /// Whether the next cell still depends on cells before it.
    fn carries(&self) -> bool {
        self.invalidated > 0 || self.to_skip > 0 || self.index < self.forced_end
    }

    /// Advances over one cell; true when the backend receives it.
    fn write(&mut self, empty: bool, symbol: &str, width: usize) -> bool {
        let diff_write = (!empty || self.invalidated > 0) && self.to_skip == 0;
        let written = diff_write || (self.index < self.forced_end && !empty);
        // A VS16 wide symbol also forces the following non-empty cells it covers.
        if diff_write && width > 1 && symbol.contains('\u{FE0F}') {
            self.forced_end = self.forced_end.max(self.index + width);
        }
        self.to_skip = width.saturating_sub(1);
        self.invalidated = width.max(1).max(self.invalidated) - 1;
        self.index += 1;
        written
    }
}

fn symbol_width(cell: &ScreenCell, symbol: &str) -> usize {
    if cell.flags & CELL_CLUSTER == 0 && (0x20..0x7f).contains(&cell.content) {
        1
    } else {
        symbol.width()
    }
}
