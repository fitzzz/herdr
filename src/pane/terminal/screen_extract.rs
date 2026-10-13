//! Refreshes a pane's retained `TerminalScreen` from Ghostty's render state.
//!
//! The refresh is the only consumer of Ghostty's render dirty flags while the
//! snapshot path is enabled. It re-reads dirty rows (all rows after a full
//! redraw, a resize, or a style-context change), shares rows whose content did
//! not change, and records each consumer's progress as row revisions instead
//! of competing for the dirty flags.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use ratatui::style::{Color, Style};
use unicode_width::UnicodeWidthStr;

use super::screen::{
    next_row_revision, next_screen_generation, ScreenCell, ScreenRow, TerminalScreen, CELL_CLUSTER,
    CELL_LINK_DETACHED,
};
use super::{
    cursor_state_from_render_state, ghostty_blank_symbol_for_width, ghostty_buffer_symbol_into,
    ghostty_cell_color, ghostty_cell_style, ghostty_cell_symbol, ghostty_color, ghostty_default_bg,
    ghostty_default_fg, is_halfwidth_katakana_voiced_grapheme, GhosttyPaneCore, PaletteOverrides,
    TerminalDirtyPatch, TerminalDirtyPatchOutcome,
};
use crate::ghostty::{CellColor, CellStyle, CellWide, RawCell, RawContentTag};

/// A retained screen nobody has read for this long is dropped, so hidden
/// panes keep parsing output without holding a display grid.
pub(crate) const IDLE_SCREEN_RETENTION: Duration = Duration::from_secs(2);

/// `HERDR_SCREEN_SNAPSHOT=0` restores the legacy per-cell render path, for
/// matched A/B measurement in one binary.
pub(super) fn screen_snapshot_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var_os("HERDR_SCREEN_SNAPSHOT").is_none_or(|value| value != "0"))
}

/// Everything outside the cell grid that changes how cells resolve. Herdr's
/// host theme is not a Ghostty dirty source, so it is compared explicitly.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StyleContext {
    default_fg: Option<Color>,
    default_bg: Option<Color>,
    resolved_fg: Option<Color>,
    resolved_bg: Option<Color>,
    palette_overrides: Option<Box<PaletteOverrides>>,
    hide_kitty_placeholders: bool,
}

impl StyleContext {
    fn read(core: &GhosttyPaneCore) -> Self {
        let colors = core.render_state.colors().ok();
        Self {
            default_fg: colors.and_then(|c| {
                ghostty_default_fg(
                    c.foreground,
                    core.host_terminal_theme,
                    core.initial_default_foreground,
                )
            }),
            default_bg: colors.and_then(|c| {
                ghostty_default_bg(
                    c.background,
                    core.host_terminal_theme,
                    core.initial_default_background,
                )
            }),
            resolved_fg: colors.map(|c| ghostty_color(c.foreground)),
            resolved_bg: colors.map(|c| ghostty_color(c.background)),
            palette_overrides: colors
                .zip(core.terminal.default_palette().ok())
                .and_then(|(colors, default)| PaletteOverrides::new(&colors.palette, &default))
                .map(Box::new),
            hide_kitty_placeholders: crate::kitty_graphics::is_enabled(),
        }
    }

    fn blank(&self) -> ScreenCell {
        ScreenCell::blank(
            crate::protocol::color_to_u32(self.default_fg.unwrap_or(Color::Reset)),
            crate::protocol::color_to_u32(self.default_bg.unwrap_or(Color::Reset)),
        )
    }

    /// `ghostty_cell_style` from a decoded style. Its accessor fallbacks only
    /// apply when the style has no color of its own, where Ghostty reports no
    /// color either, so they are not needed here.
    fn resolve(&self, style: &CellStyle, content_bg: Option<CellColor>) -> ScreenCell {
        let palette = self.palette_overrides.as_deref();
        let mut fg = style
            .fg_color
            .map(|color| ghostty_cell_color(color, palette))
            .or(self.default_fg);
        let mut bg = content_bg
            .or(style.bg_color)
            .map(|color| ghostty_cell_color(color, palette))
            .or(self.default_bg);
        if style.invisible {
            fg = bg.or(self.default_bg);
        }
        if style.inverse {
            if bg.is_none() {
                bg = self.resolved_bg;
            }
            if fg.is_none() {
                fg = self.resolved_fg;
            }
            std::mem::swap(&mut fg, &mut bg);
        }
        let mut modifiers = ratatui::style::Modifier::empty();
        for (enabled, modifier) in [
            (style.bold, ratatui::style::Modifier::BOLD),
            (style.italic, ratatui::style::Modifier::ITALIC),
            (style.faint, ratatui::style::Modifier::DIM),
            (style.blink, ratatui::style::Modifier::SLOW_BLINK),
            (style.underlined, ratatui::style::Modifier::UNDERLINED),
            (style.strikethrough, ratatui::style::Modifier::CROSSED_OUT),
        ] {
            if enabled {
                modifiers |= modifier;
            }
        }
        let modifiers = crate::protocol::modifier_with_underline_style(modifiers, style.underline);
        ScreenCell {
            fg: crate::protocol::color_to_u32(fg.unwrap_or(Color::Reset)),
            bg: crate::protocol::color_to_u32(bg.unwrap_or(Color::Reset)),
            underline_color: crate::protocol::color_to_u32(
                style
                    .underline_color
                    .map(|color| ghostty_cell_color(color, palette))
                    .unwrap_or(Color::Reset),
            ),
            modifier: crate::protocol::modifier_to_u16(modifiers),
            ..ScreenCell::default()
        }
    }
}

fn screen_cell_from_style(style: Style) -> ScreenCell {
    ScreenCell {
        fg: crate::protocol::color_to_u32(style.fg.unwrap_or(Color::Reset)),
        bg: crate::protocol::color_to_u32(style.bg.unwrap_or(Color::Reset)),
        underline_color: crate::protocol::color_to_u32(
            style.underline_color.unwrap_or(Color::Reset),
        ),
        modifier: crate::protocol::modifier_to_u16(style.add_modifier),
        ..ScreenCell::default()
    }
}

/// Reusable buffers for one refresh. A dirty row whose content turns out
/// unchanged costs no allocation.
#[derive(Default)]
struct RowScratch {
    cells: Vec<ScreenCell>,
    clusters: Vec<Box<str>>,
    links: Vec<Arc<str>>,
    link_symbols: Vec<(u16, Box<str>)>,
    styles: Vec<(u16, CellStyle, ScreenCell)>,
    grapheme_bytes: Vec<u8>,
    text: String,
    link_text: String,
}

impl RowScratch {
    fn clear(&mut self) {
        self.cells.clear();
        self.clusters.clear();
        self.links.clear();
        self.link_symbols.clear();
        self.styles.clear();
    }

    fn matches(&self, row: &ScreenRow, selection: Option<(u16, u16)>) -> bool {
        *row.cells == *self.cells
            && row.clusters == self.clusters
            && row.links == self.links
            && row.link_symbols == self.link_symbols
            && row.selection == selection
    }

    fn to_row(&self, selection: Option<(u16, u16)>) -> ScreenRow {
        ScreenRow {
            revision: next_row_revision(),
            cells: self.cells.clone().into_boxed_slice(),
            clusters: self.clusters.clone(),
            links: self.links.clone(),
            link_symbols: self.link_symbols.clone(),
            selection,
        }
    }

    /// Stores `self.text` as the cell's display symbol.
    fn set_symbol(&mut self, cell: &mut ScreenCell) {
        let mut chars = self.text.chars();
        match (chars.next(), chars.next()) {
            (None, _) => cell.content = 0,
            (Some(ch), None) => cell.content = u32::from(ch),
            (Some(_), Some(_)) => {
                cell.content = self.clusters.len() as u32;
                cell.flags |= CELL_CLUSTER;
                self.clusters.push(self.text.as_str().into());
            }
        }
    }

    fn intern_link(&mut self, uri: String) -> u32 {
        let index = match self.links.iter().position(|link| **link == *uri) {
            Some(index) => index,
            None => {
                self.links.push(uri.into());
                self.links.len() - 1
            }
        };
        index as u32 + 1
    }

    /// Attaches `uri`; `self.link_text` holds the OSC 8 symbol.
    fn set_link(&mut self, x: u16, cell: &mut ScreenCell, uri: String) {
        cell.link = self.intern_link(uri);
        let mut scratch = [0u8; 4];
        let display = if cell.flags & CELL_CLUSTER != 0 {
            self.clusters
                .get(cell.content as usize)
                .map_or(" ", |cluster| cluster)
        } else if cell.content == 0 {
            ""
        } else {
            char::from_u32(cell.content).map_or(" ", |ch| ch.encode_utf8(&mut scratch))
        };
        if display != self.link_text {
            cell.flags |= CELL_LINK_DETACHED;
            self.link_symbols.push((x, self.link_text.as_str().into()));
        }
    }
}

/// `ghostty_buffer_symbol_into` width normalization, applied to `text`.
fn normalize_display_width(text: &mut String, wide: CellWide) {
    let expected_width = match wide {
        CellWide::Wide => 2,
        CellWide::Narrow | CellWide::SpacerHead => 1,
        CellWide::SpacerTail => 0,
    };
    let actual_width = text.width();
    if actual_width != expected_width
        && !(wide == CellWide::Narrow && actual_width == 2)
        && !(wide == CellWide::Wide && is_halfwidth_katakana_voiced_grapheme(text))
    {
        text.clear();
        text.push_str(ghostty_blank_symbol_for_width(wide));
    }
}

/// Reads one row through the packed-cell view: one native call for the row,
/// plus style and grapheme reads for distinct style ids and multi-scalar
/// cells, plus one URI read per OSC 8 cell.
fn extract_row_raw(
    rows: &mut crate::ghostty::RowIter<'_>,
    row_cells: &mut crate::ghostty::RowCells,
    terminal: &crate::ghostty::Terminal,
    y: u16,
    cols: u16,
    context: &StyleContext,
    scratch: &mut RowScratch,
) -> Result<(), crate::ghostty::Error> {
    let raw_cells = rows.cells_raw()?;
    let mut cells = rows.populate_cells(row_cells)?;
    let blank = context.blank();
    for (x, raw) in raw_cells
        .iter()
        .copied()
        .take(usize::from(cols))
        .enumerate()
    {
        // Bounded by `cols`.
        let x = x as u16;
        if RawCell::is_default_blank(raw) {
            scratch.cells.push(blank);
            continue;
        }
        let decoded = RawCell::decode(raw);
        let mut cell = if decoded.style_id == 0 && decoded.bg.is_none() {
            blank
        } else if decoded.style_id == 0 {
            context.resolve(&CellStyle::default(), decoded.bg)
        } else {
            let cached = scratch
                .styles
                .iter()
                .find(|(id, _, _)| *id == decoded.style_id)
                .map(|(_, style, resolved)| (*style, *resolved));
            let (style, resolved) = match cached {
                Some(cached) => cached,
                None => {
                    cells.select(x)?;
                    let style = cells.style()?;
                    let resolved = context.resolve(&style, None);
                    scratch.styles.push((decoded.style_id, style, resolved));
                    (style, resolved)
                }
            };
            if decoded.bg.is_some() {
                context.resolve(&style, decoded.bg)
            } else {
                resolved
            }
        };

        // Printable ASCII in a narrow cell needs no text or width work.
        if decoded.content_tag == RawContentTag::Codepoint
            && decoded.wide == CellWide::Narrow
            && !decoded.has_hyperlink
            && (0x20..0x7f).contains(&decoded.codepoint)
        {
            cell.content = decoded.codepoint;
            scratch.cells.push(cell);
            continue;
        }

        // Raw text first: OSC 8 symbols use it without display normalization.
        scratch.text.clear();
        match decoded.wide {
            CellWide::SpacerTail | CellWide::SpacerHead => {}
            CellWide::Narrow | CellWide::Wide => match decoded.content_tag {
                RawContentTag::Codepoint => {
                    if let Some(ch) =
                        char::from_u32(decoded.codepoint).filter(|_| decoded.codepoint != 0)
                    {
                        scratch.text.push(ch);
                    }
                }
                RawContentTag::CodepointGrapheme => {
                    cells.select(x)?;
                    cells.grapheme_text_into(&mut scratch.grapheme_bytes, &mut scratch.text)?;
                }
                RawContentTag::BgPalette | RawContentTag::BgRgb => {}
            },
        }
        let first = scratch.text.chars().next().map(u32::from);
        if decoded.has_hyperlink {
            scratch.link_text.clear();
            if decoded.wide != CellWide::SpacerTail {
                if first == Some(crate::ghostty::KITTY_UNICODE_PLACEHOLDER)
                    || scratch.text.is_empty()
                {
                    scratch.link_text.push(' ');
                } else {
                    scratch.link_text.push_str(&scratch.text);
                }
            }
        }
        match decoded.wide {
            CellWide::SpacerTail => {}
            CellWide::SpacerHead => scratch.text.push(' '),
            CellWide::Narrow | CellWide::Wide => {
                let hidden_placeholder = context.hide_kitty_placeholders
                    && first == Some(crate::ghostty::KITTY_UNICODE_PLACEHOLDER);
                if hidden_placeholder || scratch.text.is_empty() {
                    scratch.text.clear();
                    scratch.text.push(' ');
                }
            }
        }
        normalize_display_width(&mut scratch.text, decoded.wide);
        scratch.set_symbol(&mut cell);

        if decoded.has_hyperlink {
            if let Some(uri) = terminal.viewport_hyperlink_uri(x, y.into())? {
                scratch.set_link(x, &mut cell, uri);
            }
        }
        scratch.cells.push(cell);
    }
    scratch.cells.resize(usize::from(cols), blank);
    Ok(())
}

/// Reads one row through per-cell accessors, exactly as the legacy renderer
/// and hyperlink scan did. Used when the packed layout is not verified.
fn extract_row_accessor(
    rows: &mut crate::ghostty::RowIter<'_>,
    row_cells: &mut crate::ghostty::RowCells,
    terminal: &crate::ghostty::Terminal,
    y: u16,
    cols: u16,
    context: &StyleContext,
    scratch: &mut RowScratch,
) -> Result<(), crate::ghostty::Error> {
    let mut cells = rows.populate_cells(row_cells)?;
    let mut x = 0u16;
    while x < cols && cells.next() {
        let basic = cells.basic_data().unwrap_or_default();
        let style = ghostty_cell_style(
            &cells,
            &basic,
            context.default_fg,
            context.default_bg,
            context.resolved_fg,
            context.resolved_bg,
            context.palette_overrides.as_deref(),
        );
        let mut cell = screen_cell_from_style(style);
        if ghostty_buffer_symbol_into(
            &cells,
            basic.wide,
            context.hide_kitty_placeholders,
            &mut scratch.grapheme_bytes,
            &mut scratch.text,
        )
        .is_err()
        {
            scratch.text.clear();
            scratch
                .text
                .push_str(ghostty_blank_symbol_for_width(basic.wide));
        }
        scratch.set_symbol(&mut cell);
        if cells.has_hyperlink()? {
            if let Some(uri) = terminal.viewport_hyperlink_uri(x, y.into())? {
                scratch.link_text = ghostty_cell_symbol(&cells)?;
                scratch.set_link(x, &mut cell, uri);
            }
        }
        scratch.cells.push(cell);
        x += 1;
    }
    scratch.cells.resize(usize::from(cols), context.blank());
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExtractPath {
    Raw,
    Accessor,
}

fn default_extract_path() -> ExtractPath {
    if crate::ghostty::raw_cell_layout_verified() {
        ExtractPath::Raw
    } else {
        ExtractPath::Accessor
    }
}

/// Refreshes and returns the pane's screen. On error the retained screen is
/// dropped, so the next refresh rebuilds every row.
pub(super) fn refresh_screen(
    core: &mut GhosttyPaneCore,
) -> Result<Arc<TerminalScreen>, crate::ghostty::Error> {
    refresh_screen_with(core, default_extract_path())
}

pub(super) fn refresh_screen_with(
    core: &mut GhosttyPaneCore,
    path: ExtractPath,
) -> Result<Arc<TerminalScreen>, crate::ghostty::Error> {
    let started = crate::render_prof::timer();
    core.screen_used_at = Instant::now();
    let result = refresh_screen_inner(core, path);
    if result.is_err() {
        core.screen = None;
        crate::render_prof::event("screen.refresh_error");
    }
    crate::render_prof::duration_since("screen.refresh", started);
    result
}

fn refresh_screen_inner(
    core: &mut GhosttyPaneCore,
    path: ExtractPath,
) -> Result<Arc<TerminalScreen>, crate::ghostty::Error> {
    core.render_state.update(&core.terminal)?;
    let dirty = core.render_state.dirty()?;
    let cols = core.render_state.cols()?;
    let row_count = core.render_state.rows()?;
    let context = StyleContext::read(core);
    let cursor = cursor_state_from_render_state(&mut core.render_state, &core.decscusr_tracker);
    let previous = core
        .screen
        .take()
        .filter(|previous| previous.cols == cols && previous.rows == row_count);
    // Resolved colors may change without a Ghostty dirty flag (host theme), so
    // every row is re-read; rows that resolve the same are still shared.
    let context_changed = previous
        .as_ref()
        .is_some_and(|previous| previous.context != context);

    if dirty == crate::ghostty::Dirty::Clean && !context_changed {
        if let Some(previous) = previous {
            let screen = if previous.cursor == cursor {
                previous
            } else {
                Arc::new(TerminalScreen {
                    generation: next_screen_generation(),
                    cursor,
                    ..TerminalScreen::clone(&previous)
                })
            };
            core.screen = Some(screen.clone());
            return Ok(screen);
        }
    }
    let rebuild_all = dirty == crate::ghostty::Dirty::Full || context_changed || previous.is_none();

    let GhosttyPaneCore {
        terminal,
        render_state,
        ..
    } = &mut *core;
    let mut row_iterator = crate::ghostty::RowIterator::new()?;
    let mut row_cells = crate::ghostty::RowCells::new()?;
    let mut scratch = RowScratch::default();
    let mut rows_data = Vec::with_capacity(usize::from(row_count));
    let mut extracted = 0u64;
    let mut changed = false;
    {
        let mut rows = render_state.populate_row_iterator(&mut row_iterator)?;
        let mut y = 0u16;
        while y < row_count && rows.next() {
            let selection = rows.selection()?.map(|s| (s.start_x, s.end_x));
            let old = previous
                .as_ref()
                .and_then(|previous| previous.rows_data.get(usize::from(y)));
            let row_dirty = rebuild_all || rows.dirty()?;
            let row = match old {
                Some(old) if !row_dirty => {
                    if old.selection == selection {
                        old.clone()
                    } else {
                        changed = true;
                        Arc::new(ScreenRow {
                            revision: next_row_revision(),
                            selection,
                            ..ScreenRow::clone(old)
                        })
                    }
                }
                _ => {
                    scratch.clear();
                    match path {
                        ExtractPath::Raw => extract_row_raw(
                            &mut rows,
                            &mut row_cells,
                            terminal,
                            y,
                            cols,
                            &context,
                            &mut scratch,
                        )?,
                        ExtractPath::Accessor => extract_row_accessor(
                            &mut rows,
                            &mut row_cells,
                            terminal,
                            y,
                            cols,
                            &context,
                            &mut scratch,
                        )?,
                    }
                    extracted += 1;
                    match old {
                        Some(old) if scratch.matches(old, selection) => old.clone(),
                        _ => {
                            changed = true;
                            Arc::new(scratch.to_row(selection))
                        }
                    }
                }
            };
            rows.clear_dirty()?;
            rows_data.push(row);
            y += 1;
        }
        // A short iterator leaves blank rows, matching the renderer's padding.
        while y < row_count {
            scratch.clear();
            scratch.cells.resize(usize::from(cols), context.blank());
            let old = previous
                .as_ref()
                .and_then(|previous| previous.rows_data.get(usize::from(y)));
            rows_data.push(match old {
                Some(old) if scratch.matches(old, None) => old.clone(),
                _ => {
                    changed = true;
                    Arc::new(scratch.to_row(None))
                }
            });
            y += 1;
        }
    }
    render_state.set_dirty(crate::ghostty::Dirty::Clean)?;
    crate::render_prof::counter("screen.rows_extracted", extracted);
    #[cfg(test)]
    {
        core.rows_extracted += extracted;
    }

    let screen = match previous {
        Some(previous) if !changed && !context_changed && previous.cursor == cursor => previous,
        _ => Arc::new(TerminalScreen {
            cols,
            rows: row_count,
            generation: next_screen_generation(),
            default_fg: crate::protocol::color_to_u32(context.default_fg.unwrap_or(Color::Reset)),
            default_bg: crate::protocol::color_to_u32(context.default_bg.unwrap_or(Color::Reset)),
            cursor,
            rows_data,
            context,
        }),
    };
    core.screen = Some(screen.clone());
    Ok(screen)
}

/// Drops the retained screen when nobody read it within `retention`.
pub(super) fn release_idle_screen(
    core: &mut GhosttyPaneCore,
    now: Instant,
    retention: Duration,
) -> bool {
    if core.screen.is_none() || now.saturating_duration_since(core.screen_used_at) < retention {
        return false;
    }
    core.screen = None;
    true
}

/// What a dirty-patch consumer last received: one revision per viewport row
/// (0 = never delivered) and the geometry and padding style it was built for.
#[derive(Debug, Default)]
pub(super) struct PatchBaseline {
    cols: u16,
    area_width: u16,
    default_fg: u32,
    default_bg: u32,
    revisions: Vec<u64>,
}

/// Rows changed since the previous dirty patch, from the shared screen.
/// Rows outside `area_height`, and rows of a fallback, stay pending.
pub(super) fn screen_dirty_patch(
    core: &mut GhosttyPaneCore,
    area_width: u16,
    area_height: u16,
) -> TerminalDirtyPatchOutcome {
    let started = crate::render_prof::timer();
    let outcome = screen_dirty_patch_inner(core, area_width, area_height);
    if let Some(started) = started {
        crate::render_prof::duration("dirty_collect.total", started.elapsed());
        match &outcome {
            TerminalDirtyPatchOutcome::Clean => crate::render_prof::event("dirty_collect.clean"),
            TerminalDirtyPatchOutcome::Fallback => {
                crate::render_prof::event("dirty_collect.fallback")
            }
            TerminalDirtyPatchOutcome::Patch(patch) => {
                crate::render_prof::event("dirty_collect.patch");
                crate::render_prof::counter("dirty_collect.rows", patch.rows.len() as u64);
                let cells = patch.rows.iter().map(|(_, cells)| cells.len() as u64).sum();
                crate::render_prof::counter("dirty_collect.cells", cells);
            }
        }
    }
    outcome
}

fn screen_dirty_patch_inner(
    core: &mut GhosttyPaneCore,
    area_width: u16,
    area_height: u16,
) -> TerminalDirtyPatchOutcome {
    let Ok(screen) = refresh_screen(core) else {
        crate::render_prof::event("dirty_fallback.screen_refresh_error");
        return TerminalDirtyPatchOutcome::Fallback;
    };
    let baseline = core
        .patch_baseline
        .get_or_insert_with(PatchBaseline::default);
    if baseline.cols != screen.cols
        || baseline.area_width != area_width
        || baseline.default_fg != screen.default_fg
        || baseline.default_bg != screen.default_bg
        || baseline.revisions.len() != screen.rows_data.len()
    {
        *baseline = PatchBaseline {
            cols: screen.cols,
            area_width,
            default_fg: screen.default_fg,
            default_bg: screen.default_bg,
            revisions: vec![0; screen.rows_data.len()],
        };
    }
    let changed = screen
        .rows_data
        .iter()
        .zip(&baseline.revisions)
        .take(usize::from(area_height))
        .enumerate()
        .filter(|(_, (row, delivered))| row.revision != **delivered)
        .map(|(y, (row, _))| (y, row))
        .collect::<Vec<_>>();
    if changed.is_empty() {
        return TerminalDirtyPatchOutcome::Clean;
    }
    if changed.iter().any(|(_, row)| row.selection.is_some()) {
        crate::render_prof::event("dirty_fallback.row_selection_present");
        return TerminalDirtyPatchOutcome::Fallback;
    }
    let mut rows = Vec::with_capacity(changed.len());
    let mut hyperlinks = Vec::new();
    for (y, row) in changed {
        // Bounded by `area_height`.
        let y16 = y as u16;
        rows.push((y16, screen.patch_cells(y16, area_width, &mut hyperlinks)));
        baseline.revisions[y] = row.revision;
    }
    TerminalDirtyPatchOutcome::Patch(TerminalDirtyPatch { rows, hyperlinks })
}
