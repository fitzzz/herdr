//! Negotiated binary terminal-cell codec for terminal session helpers.
//!
//! A terminal session helper (`herdr terminal session control|observe --output cells`)
//! asks for this codec with a named `EndpointControl` hello after the private
//! handshake. Only a connection whose hello the server answered with a
//! [`WELCOME_KIND`] control ever receives [`FRAME_KIND`] controls. No existing
//! message variant, field or codec changes meaning: older servers ignore the
//! unknown hello and keep sending ANSI `Terminal` frames, which the helper
//! recognises as a fallback.
//!
//! The codec carries exactly what the ANSI encoder would paint for the same
//! `FrameData`, without truncation. A frame it cannot carry exactly is reported
//! as [`Unsupported`]; the server then switches that connection to the existing
//! ANSI `Terminal` frames, starting with a full repaint.
//!
//! The frame record is the payload a display client applies directly to its
//! cell grid, without an ANSI parser. Its layout (little endian):
//!
//! ```text
//! header  "HTC1" | u8 kind (0 full, 1 patch) | u8 flags (bit0 cursor present, bit1 visible)
//!         | u64 seq | u64 base_seq (patch: seq it applies to; full: 0)
//!         | u16 width | u16 height | u16 cursor_x | u16 cursor_y | u8 cursor_shape | u8 0
//! links   u32 count, then { u32 id (> 0) | u16 len | utf8 uri }
//! spans   u32 count, then { u16 y | u16 x | u16 cell_count | cells }
//! cell    u8 tag: bit0 style follows, bit1 link follows, bits2-3 width (0 continuation, 1, 2),
//!         bits4-7 text length 0..=14, 15 = u16 length follows
//!         [style: u32 fg | u32 bg | u32 underline color | u16 attrs] [link: u32 id] [utf8 text]
//! ```
//!
//! Colors use the generation-1 wire encoding: `0x000000NN` with NN 0 default and
//! 1..=16 the sixteen named SGR colors (30..37, 90..97 order), `0x010000NN`
//! palette index, `0x02RRGGBB` rgb. Attrs are the generation-1 modifier bits
//! unchanged (ratatui bits 0..=8, underline style in bits 12..=15).
//!
//! Style and link carry over between cells of one span; every span starts from
//! the default style and no link. A full frame has exactly one span per row,
//! in row order, covering the row. A full frame's link table replaces the
//! client table; a patch's table only adds ids. Sequence numbers increase per
//! connection but may skip; a patch applies only when `base_seq` equals the
//! last applied sequence, otherwise the client must resynchronise.

use std::collections::HashMap;

use base64::Engine;
use serde::{Deserialize, Serialize};

use super::render_ansi::{cell_width, resolve_host_cursor_state, sanitized_hyperlink_uri};
use super::{CellData, CursorState, FrameData, PaneSurfacePatchRow, ServerMessage};

pub(crate) const CODEC: &str = "herdr.terminal.cells.v1";
pub(crate) const HELLO_KIND: &str = "terminal.cells.hello.v1";
pub(crate) const WELCOME_KIND: &str = "terminal.cells.welcome.v1";
pub(crate) const FRAME_KIND: &str = "terminal.cells.frame.v1";

pub(crate) const MAGIC: &[u8; 4] = b"HTC1";
pub(crate) const KIND_FULL: u8 = 0;
pub(crate) const KIND_PATCH: u8 = 1;
const FLAG_CURSOR_PRESENT: u8 = 1 << 0;
const FLAG_CURSOR_VISIBLE: u8 = 1 << 1;
const TAG_STYLE: u8 = 1 << 0;
const TAG_LINK: u8 = 1 << 1;
const TAG_WIDTH_SHIFT: u8 = 2;
const TAG_TEXT_SHIFT: u8 = 4;
const TAG_TEXT_EXTENDED: u8 = 15;
const HEADER_BYTES: usize = 4 + 1 + 1 + 8 + 8 + 2 * 4 + 2;

/// Largest frame width or height either side accepts.
pub(crate) const MAX_DIMENSION: u16 = 4096;
/// Largest encoded frame record: its base64 control must fit one server frame.
/// Helpers use the same bound for their stdout records.
pub(crate) const MAX_RECORD_BYTES: usize = (super::MAX_FRAME_SIZE - 4096) / 4 * 3;
/// Largest number of hyperlink ids alive in one stream between full frames.
pub(crate) const MAX_LINKS: usize = 16 * 1024;
/// Largest hyperlink URI, the most its u16 length field can describe.
pub(crate) const MAX_URI_BYTES: usize = u16::MAX as usize;
/// Largest cell text, the most its u16 length field can describe.
pub(crate) const MAX_CELL_TEXT_BYTES: usize = u16::MAX as usize;

pub(crate) const COLOR_DEFAULT: u32 = 0;

/// Why a frame cannot be carried exactly; the caller falls back to ANSI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unsupported(pub(crate) &'static str);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CellsHello {
    #[serde(default)]
    pub codecs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CellsWelcome {
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CellsHello {
    pub(crate) fn current() -> Self {
        Self {
            codecs: vec![CODEC.to_owned()],
        }
    }

    pub(crate) fn accepts_current(&self) -> bool {
        self.codecs.iter().any(|codec| codec == CODEC)
    }
}

pub(crate) fn hello_message() -> serde_json::Result<super::ClientMessage> {
    Ok(super::ClientMessage::EndpointControl {
        kind: HELLO_KIND.into(),
        data: serde_json::to_string(&CellsHello::current())?,
    })
}

pub(crate) fn welcome_message(welcome: &CellsWelcome) -> serde_json::Result<ServerMessage> {
    Ok(ServerMessage::EndpointControl {
        kind: WELCOME_KIND.into(),
        data: serde_json::to_string(welcome)?,
    })
}

/// Wraps one encoded record in its named control. The record is base64 because
/// the extensible control carries text; the helper decodes it once and writes raw
/// bytes to the display client.
pub(crate) fn frame_message(record: &[u8]) -> ServerMessage {
    ServerMessage::EndpointControl {
        kind: FRAME_KIND.into(),
        data: base64::engine::general_purpose::STANDARD.encode(record),
    }
}

pub(crate) fn decode_frame_message(data: &str) -> Result<Vec<u8>, DecodeError> {
    if data.len() > MAX_RECORD_BYTES.div_ceil(3) * 4 {
        return Err(DecodeError::TooLarge);
    }
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| DecodeError::Malformed("frame record is not base64"))
}

// ---------------------------------------------------------------------------
// Cell normalisation
// ---------------------------------------------------------------------------

/// Canonical form of a generation-1 wire color. Values the ANSI encoder paints
/// as the default color become the default; every other value keeps the exact
/// bits the ANSI encoder reads.
pub(crate) fn codec_color(wire: u32) -> u32 {
    match wire >> 24 {
        0x00 => match wire & 0xFF {
            named @ 0x00..=0x10 => named,
            _ => COLOR_DEFAULT,
        },
        0x01 => 0x01_00_00_00 | (wire & 0xFF),
        0x02 => wire,
        _ => COLOR_DEFAULT,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Style {
    fg: u32,
    bg: u32,
    underline_color: u32,
    attrs: u16,
}

/// One cell as the display client stores it, borrowed from the source frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EffectiveCell<'a> {
    text: &'a str,
    width: u8,
    style: Style,
    link: Option<&'a str>,
}

/// Hyperlink URIs exactly as the ANSI encoder writes them: control characters
/// removed, and a URI that is empty afterwards leaves the cell unlinked.
fn sanitized_links(frame: &FrameData) -> Vec<Option<String>> {
    frame
        .hyperlinks
        .iter()
        .map(|uri| sanitized_hyperlink_uri(uri))
        .collect()
}

/// Resolves each column to what the ANSI encoder paints there. A cell after a
/// width-2 lead is covered by that lead and its own contents are never painted.
/// Cells the ANSI encoder leaves unpainted on purpose (skipped, empty or
/// zero-width symbols) and cells wider than two columns cannot be represented.
fn effective_row<'a>(
    cells: impl Iterator<Item = &'a CellData>,
    links: &'a [Option<String>],
    out: &mut Vec<EffectiveCell<'a>>,
) -> Result<(), Unsupported> {
    out.clear();
    let mut covering: Option<EffectiveCell<'a>> = None;
    for cell in cells {
        if let Some(lead) = covering.take() {
            // The ANSI diff path takes its next skip from the covered cell's own
            // width; only a narrow covered cell gives both paths the same result.
            if !cell.skip && cell_width(cell) > 1 {
                return Err(Unsupported("wide cell under a wide lead"));
            }
            out.push(EffectiveCell {
                text: "",
                width: 0,
                ..lead
            });
            continue;
        }
        if cell.skip {
            return Err(Unsupported("skipped cell"));
        }
        let width = match cell_width(cell) {
            1 => 1,
            2 => 2,
            0 if cell.symbol.is_empty() => return Err(Unsupported("empty cell")),
            0 => return Err(Unsupported("zero-width cell")),
            _ => return Err(Unsupported("cell wider than two columns")),
        };
        if cell.symbol.len() > MAX_CELL_TEXT_BYTES {
            return Err(Unsupported("cell text too long"));
        }
        let link = cell
            .hyperlink
            .and_then(|index| links.get(index as usize))
            .and_then(Option::as_deref);
        if link.is_some_and(|uri| uri.len() > MAX_URI_BYTES) {
            return Err(Unsupported("hyperlink too long"));
        }
        let effective = EffectiveCell {
            text: &cell.symbol,
            width,
            style: Style {
                fg: codec_color(cell.fg),
                bg: codec_color(cell.bg),
                underline_color: COLOR_DEFAULT,
                attrs: cell.modifier,
            },
            link,
        };
        out.push(effective);
        if width == 2 {
            covering = Some(effective);
        }
    }
    Ok(())
}

/// Places the cursor exactly where the ANSI encoder parks the host cursor.
fn host_cursor(
    width: u16,
    height: u16,
    cursor: Option<&CursorState>,
    last_visible: Option<(u16, u16)>,
) -> (CursorState, Option<(u16, u16)>) {
    let frame = FrameData {
        cells: Vec::new(),
        width,
        height,
        cursor: cursor.cloned(),
        hyperlinks: Vec::new(),
        graphics: Vec::new(),
    };
    let mut memory = last_visible;
    let host = resolve_host_cursor_state(&frame, &mut memory);
    let resolved = CursorState {
        x: host.position.0,
        y: host.position.1,
        visible: host.visible,
        shape: host.shape,
    };
    (resolved, memory)
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct LinkTable {
    ids: HashMap<String, u32>,
    next: u32,
}

/// Bytes produced by [`CellsEncoder`] for one frame, plus the state to commit.
#[derive(Debug)]
pub(crate) struct EncodedCells {
    pub(crate) bytes: Vec<u8>,
    pub(crate) full: bool,
    new_links: Vec<(String, u32)>,
    next_last_visible_cursor: Option<(u16, u16)>,
}

/// Per-connection encoder keeping the committed frame and live link ids. Encoding
/// never changes it; only `commit` and `commit_patch` do.
#[derive(Debug, Default)]
pub(crate) struct CellsEncoder {
    last_frame: Option<FrameData>,
    links: LinkTable,
    last_visible_cursor: Option<(u16, u16)>,
}

enum Finish {
    /// A patch needs more link ids than the stream budget; a full frame resets it.
    LinkBudget,
    Unsupported(Unsupported),
}

struct RecordWriter<'t> {
    bytes: Vec<u8>,
    links: &'t LinkTable,
    new_links: Vec<(String, u32)>,
    new_ids: HashMap<String, u32>,
    next_link: u32,
    span_count: u32,
    link_count_at: usize,
    link_overflow: bool,
}

impl<'t> RecordWriter<'t> {
    fn new(
        kind: u8,
        seq: u64,
        base_seq: u64,
        width: u16,
        height: u16,
        cursor: &CursorState,
        links: &'t LinkTable,
    ) -> Self {
        let mut bytes = Vec::with_capacity(HEADER_BYTES + 8 + usize::from(width) * 3);
        bytes.extend_from_slice(MAGIC);
        bytes.push(kind);
        let mut flags = FLAG_CURSOR_PRESENT;
        if cursor.visible {
            flags |= FLAG_CURSOR_VISIBLE;
        }
        bytes.push(flags);
        bytes.extend_from_slice(&seq.to_le_bytes());
        bytes.extend_from_slice(&base_seq.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&cursor.x.to_le_bytes());
        bytes.extend_from_slice(&cursor.y.to_le_bytes());
        bytes.push(cursor.shape);
        bytes.push(0);
        let link_count_at = bytes.len();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        Self {
            bytes,
            links,
            new_links: Vec::new(),
            new_ids: HashMap::new(),
            next_link: links.next,
            span_count: 0,
            link_count_at,
            link_overflow: false,
        }
    }

    fn link_id(&mut self, uri: &str) -> u32 {
        if let Some(id) = self.links.ids.get(uri) {
            return *id;
        }
        if let Some(id) = self.new_ids.get(uri) {
            return *id;
        }
        if self.links.ids.len() + self.new_links.len() >= MAX_LINKS {
            self.link_overflow = true;
            return 0;
        }
        self.next_link += 1;
        self.new_links.push((uri.to_owned(), self.next_link));
        self.new_ids.insert(uri.to_owned(), self.next_link);
        self.next_link
    }

    /// Appends one span. Spans accumulate after the reserved link count and are
    /// moved behind the link table in `finish`.
    fn span(&mut self, y: u16, x: u16, cells: &[EffectiveCell<'_>]) {
        self.bytes.extend_from_slice(&y.to_le_bytes());
        self.bytes.extend_from_slice(&x.to_le_bytes());
        self.bytes
            .extend_from_slice(&(cells.len() as u16).to_le_bytes());
        let mut style = Style::default();
        let mut link = 0u32;
        for cell in cells {
            let cell_link = cell.link.map_or(0, |uri| self.link_id(uri));
            let text = cell.text.as_bytes();
            let mut tag = (cell.width & 0x03) << TAG_WIDTH_SHIFT;
            if cell.style != style {
                tag |= TAG_STYLE;
            }
            if cell_link != link {
                tag |= TAG_LINK;
            }
            let extended = text.len() >= usize::from(TAG_TEXT_EXTENDED);
            tag |= if extended {
                TAG_TEXT_EXTENDED
            } else {
                text.len() as u8
            } << TAG_TEXT_SHIFT;
            self.bytes.push(tag);
            if extended {
                self.bytes
                    .extend_from_slice(&(text.len() as u16).to_le_bytes());
            }
            if cell.style != style {
                style = cell.style;
                self.bytes.extend_from_slice(&style.fg.to_le_bytes());
                self.bytes.extend_from_slice(&style.bg.to_le_bytes());
                self.bytes
                    .extend_from_slice(&style.underline_color.to_le_bytes());
                self.bytes.extend_from_slice(&style.attrs.to_le_bytes());
            }
            if cell_link != link {
                link = cell_link;
                self.bytes.extend_from_slice(&link.to_le_bytes());
            }
            self.bytes.extend_from_slice(text);
        }
        self.span_count += 1;
    }

    fn finish(
        self,
        full: bool,
        next_last_visible_cursor: Option<(u16, u16)>,
    ) -> Result<EncodedCells, Finish> {
        if self.link_overflow {
            return Err(if full {
                Finish::Unsupported(Unsupported("too many hyperlinks"))
            } else {
                Finish::LinkBudget
            });
        }
        // Layout: header | links | span count | spans. The links are known only
        // after every span was written, so the record is assembled here.
        let spans = &self.bytes[self.link_count_at + 4..];
        let links_len: usize = self.new_links.iter().map(|(uri, _)| 6 + uri.len()).sum();
        let total = self.link_count_at + 4 + links_len + 4 + spans.len();
        if total > MAX_RECORD_BYTES {
            return Err(Finish::Unsupported(Unsupported("frame record too large")));
        }
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&self.bytes[..self.link_count_at]);
        out.extend_from_slice(&(self.new_links.len() as u32).to_le_bytes());
        for (uri, id) in &self.new_links {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&(uri.len() as u16).to_le_bytes());
            out.extend_from_slice(uri.as_bytes());
        }
        out.extend_from_slice(&self.span_count.to_le_bytes());
        out.extend_from_slice(spans);
        Ok(EncodedCells {
            bytes: out,
            full,
            new_links: self.new_links,
            next_last_visible_cursor,
        })
    }
}

fn row_cells(frame: &FrameData, y: u16) -> &[CellData] {
    let width = usize::from(frame.width);
    let start = usize::from(y) * width;
    &frame.cells[start..start + width]
}

fn check_frame(frame: &FrameData) -> Result<(), Unsupported> {
    if frame.width == 0
        || frame.height == 0
        || frame.width > MAX_DIMENSION
        || frame.height > MAX_DIMENSION
        || frame.cells.len() != usize::from(frame.width) * usize::from(frame.height)
    {
        return Err(Unsupported("frame dimensions"));
    }
    if !frame.graphics.is_empty() {
        return Err(Unsupported("terminal graphics"));
    }
    Ok(())
}

/// Appends spans for every run of columns whose effective cells differ.
fn write_changed_spans(
    writer: &mut RecordWriter<'_>,
    y: u16,
    previous: &[EffectiveCell<'_>],
    next: &[EffectiveCell<'_>],
) {
    let mut col = 0;
    while col < next.len() {
        if previous.get(col) == Some(&next[col]) {
            col += 1;
            continue;
        }
        let start = col;
        while col < next.len() && previous.get(col) != Some(&next[col]) {
            col += 1;
        }
        writer.span(y, start as u16, &next[start..col]);
    }
}

impl CellsEncoder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn is_current(&self, frame: &FrameData) -> bool {
        self.last_frame.as_ref() == Some(frame)
    }

    pub(crate) fn last_frame(&self) -> Option<&FrameData> {
        self.last_frame.as_ref()
    }

    /// Encodes `frame` as a patch against the committed frame when one of the
    /// same size exists and `repaint` is false, otherwise as a full frame.
    pub(crate) fn encode(
        &self,
        frame: &FrameData,
        repaint: bool,
        seq: u64,
        base_seq: u64,
    ) -> Result<EncodedCells, Unsupported> {
        check_frame(frame)?;
        let previous = self
            .last_frame
            .as_ref()
            .filter(|_| !repaint)
            .filter(|last| last.width == frame.width && last.height == frame.height);
        if let Some(previous) = previous {
            match self.encode_diff(previous, frame, seq, base_seq) {
                Ok(encoded) => return Ok(encoded),
                Err(Finish::LinkBudget) => {}
                Err(Finish::Unsupported(reason)) => return Err(reason),
            }
        }
        self.encode_full(frame, seq)
    }

    fn encode_full(&self, frame: &FrameData, seq: u64) -> Result<EncodedCells, Unsupported> {
        let empty = LinkTable::default();
        let links = sanitized_links(frame);
        let (cursor, memory) = host_cursor(
            frame.width,
            frame.height,
            frame.cursor.as_ref(),
            self.last_visible_cursor,
        );
        let mut writer = RecordWriter::new(
            KIND_FULL,
            seq,
            0,
            frame.width,
            frame.height,
            &cursor,
            &empty,
        );
        let mut row = Vec::with_capacity(usize::from(frame.width));
        for y in 0..frame.height {
            effective_row(row_cells(frame, y).iter(), &links, &mut row)?;
            writer.span(y, 0, &row);
        }
        writer.finish(true, memory).map_err(|finish| match finish {
            Finish::Unsupported(reason) => reason,
            Finish::LinkBudget => Unsupported("too many hyperlinks"),
        })
    }

    fn encode_diff(
        &self,
        previous: &FrameData,
        frame: &FrameData,
        seq: u64,
        base_seq: u64,
    ) -> Result<EncodedCells, Finish> {
        let (cursor, memory) = host_cursor(
            frame.width,
            frame.height,
            frame.cursor.as_ref(),
            self.last_visible_cursor,
        );
        let mut writer = RecordWriter::new(
            KIND_PATCH,
            seq,
            base_seq,
            frame.width,
            frame.height,
            &cursor,
            &self.links,
        );
        let old_links = sanitized_links(previous);
        let new_links = sanitized_links(frame);
        let mut old_row = Vec::with_capacity(usize::from(frame.width));
        let mut new_row = Vec::with_capacity(usize::from(frame.width));
        for y in 0..frame.height {
            let old_cells = row_cells(previous, y);
            let new_cells = row_cells(frame, y);
            if old_cells == new_cells && old_links == new_links {
                continue;
            }
            // The committed frame was representable when it was encoded.
            effective_row(old_cells.iter(), &old_links, &mut old_row)
                .map_err(Finish::Unsupported)?;
            effective_row(new_cells.iter(), &new_links, &mut new_row)
                .map_err(Finish::Unsupported)?;
            write_changed_spans(&mut writer, y, &old_row, &new_row);
        }
        writer.finish(false, memory)
    }

    pub(crate) fn commit(&mut self, frame: FrameData, encoded: EncodedCells) {
        if encoded.full {
            self.links = LinkTable::default();
        }
        self.apply_links(encoded.new_links);
        self.last_visible_cursor = encoded.next_last_visible_cursor;
        self.last_frame = Some(frame);
    }

    fn apply_links(&mut self, links: Vec<(String, u32)>) {
        for (uri, id) in links {
            self.links.next = self.links.next.max(id);
            self.links.ids.insert(uri, id);
        }
    }

    /// Encodes retained row damage against the committed frame. Rows that overlap,
    /// leave the frame, carry hyperlink indices (which only a full frame's table
    /// can resolve) or cannot be represented return `None` so the caller takes
    /// the complete renderer, as the ANSI patch path does.
    pub(crate) fn encode_patch(
        &self,
        rows: &[PaneSurfacePatchRow],
        cursor: Option<CursorState>,
        seq: u64,
        base_seq: u64,
    ) -> Option<EncodedCells> {
        let frame = self.last_frame.as_ref()?;
        let mut touched = Vec::<u16>::new();
        for row in rows {
            let len = u16::try_from(row.cells.len()).ok()?;
            if row.y >= frame.height
                || row.x.checked_add(len)? > frame.width
                || row.cells.iter().any(|cell| cell.hyperlink.is_some())
            {
                return None;
            }
            touched.push(row.y);
        }
        touched.sort_unstable();
        touched.dedup();
        let (resolved, memory) = host_cursor(
            frame.width,
            frame.height,
            cursor.as_ref(),
            self.last_visible_cursor,
        );
        let mut writer = RecordWriter::new(
            KIND_PATCH,
            seq,
            base_seq,
            frame.width,
            frame.height,
            &resolved,
            &self.links,
        );
        let links = sanitized_links(frame);
        let mut old_row = Vec::with_capacity(usize::from(frame.width));
        let mut new_row = Vec::with_capacity(usize::from(frame.width));
        let mut overlay: Vec<&CellData> = Vec::with_capacity(usize::from(frame.width));
        let mut covered = vec![false; usize::from(frame.width)];
        for y in touched {
            let base = row_cells(frame, y);
            overlay.clear();
            overlay.extend(base.iter());
            covered.iter_mut().for_each(|covered| *covered = false);
            for row in rows.iter().filter(|row| row.y == y) {
                for (offset, cell) in row.cells.iter().enumerate() {
                    let col = usize::from(row.x) + offset;
                    if std::mem::replace(&mut covered[col], true) {
                        return None;
                    }
                    overlay[col] = cell;
                }
            }
            effective_row(base.iter(), &links, &mut old_row).ok()?;
            effective_row(overlay.iter().copied(), &links, &mut new_row).ok()?;
            write_changed_spans(&mut writer, y, &old_row, &new_row);
        }
        writer.finish(false, memory).ok()
    }

    pub(crate) fn commit_patch(
        &mut self,
        rows: &[PaneSurfacePatchRow],
        cursor: Option<CursorState>,
        encoded: EncodedCells,
    ) -> bool {
        let Some(frame) = self.last_frame.as_mut() else {
            return false;
        };
        for row in rows {
            let start = usize::from(row.y) * usize::from(frame.width) + usize::from(row.x);
            let end = start + row.cells.len();
            let Some(target) = frame.cells.get_mut(start..end) else {
                return false;
            };
            target.clone_from_slice(&row.cells);
        }
        frame.cursor = cursor;
        self.apply_links(encoded.new_links);
        self.last_visible_cursor = encoded.next_last_visible_cursor;
        true
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------
//
// The helper forwards frame records without parsing them (the display client
// validates them), so production code only unwraps the frame control. The
// record decoder below is the reference implementation the tests check the
// encoder against; display clients implement their own.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DecodeError {
    TooLarge,
    #[cfg(test)]
    Truncated,
    Malformed(&'static str),
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedCell {
    pub text: String,
    pub width: u8,
    pub fg: u32,
    pub bg: u32,
    pub underline_color: u32,
    pub attrs: u16,
    pub link: u32,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedSpan {
    pub y: u16,
    pub x: u16,
    pub cells: Vec<DecodedCell>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedFrame {
    pub full: bool,
    pub seq: u64,
    pub base_seq: u64,
    pub width: u16,
    pub height: u16,
    pub cursor: Option<CursorState>,
    pub links: Vec<(u32, String)>,
    pub spans: Vec<DecodedSpan>,
}

#[cfg(test)]
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

#[cfg(test)]
impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.at.checked_add(len).ok_or(DecodeError::Truncated)?;
        let slice = self.bytes.get(self.at..end).ok_or(DecodeError::Truncated)?;
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(raw))
    }

    fn utf8(&mut self, len: usize) -> Result<&'a str, DecodeError> {
        std::str::from_utf8(self.take(len)?).map_err(|_| DecodeError::Malformed("invalid utf-8"))
    }
}

#[cfg(test)]
fn valid_color(color: u32) -> bool {
    match color >> 24 {
        0x00 => color <= 0x10,
        0x01 => color & 0x00_FF_FF_00 == 0,
        0x02 => true,
        _ => false,
    }
}

/// Decodes and validates a complete record. Nothing is returned unless every
/// bound holds, so a client can apply the result atomically.
#[cfg(test)]
pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedFrame, DecodeError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(DecodeError::TooLarge);
    }
    let mut reader = Reader { bytes, at: 0 };
    if reader.take(4)? != MAGIC {
        return Err(DecodeError::Malformed("bad magic"));
    }
    let kind = reader.u8()?;
    let full = match kind {
        KIND_FULL => true,
        KIND_PATCH => false,
        _ => return Err(DecodeError::Malformed("unknown frame kind")),
    };
    let flags = reader.u8()?;
    if flags & !(FLAG_CURSOR_PRESENT | FLAG_CURSOR_VISIBLE) != 0 {
        return Err(DecodeError::Malformed("unknown flags"));
    }
    let seq = reader.u64()?;
    let base_seq = reader.u64()?;
    if seq == 0 || (full && base_seq != 0) || (!full && base_seq >= seq) {
        return Err(DecodeError::Malformed("invalid sequence"));
    }
    let width = reader.u16()?;
    let height = reader.u16()?;
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(DecodeError::Malformed("invalid dimensions"));
    }
    let cursor_x = reader.u16()?;
    let cursor_y = reader.u16()?;
    let shape = reader.u8()?;
    let _reserved = reader.u8()?;
    if shape > 6 {
        return Err(DecodeError::Malformed("invalid cursor shape"));
    }
    let cursor = (flags & FLAG_CURSOR_PRESENT != 0).then_some(CursorState {
        x: cursor_x,
        y: cursor_y,
        visible: flags & FLAG_CURSOR_VISIBLE != 0,
        shape,
    });

    let link_count = reader.u32()? as usize;
    if link_count > MAX_LINKS {
        return Err(DecodeError::Malformed("too many links"));
    }
    let mut links = Vec::with_capacity(link_count.min(bytes.len() / 7));
    let mut link_ids = std::collections::HashSet::with_capacity(links.capacity());
    for _ in 0..link_count {
        let id = reader.u32()?;
        let len = usize::from(reader.u16()?);
        if id == 0 || len == 0 {
            return Err(DecodeError::Malformed("invalid link"));
        }
        let uri = reader.utf8(len)?;
        if !link_ids.insert(id) {
            return Err(DecodeError::Malformed("duplicate link id"));
        }
        links.push((id, uri.to_owned()));
    }

    let span_count = reader.u32()? as usize;
    let max_spans = usize::from(width) * usize::from(height);
    if span_count > max_spans || (full && span_count != usize::from(height)) {
        return Err(DecodeError::Malformed("invalid span count"));
    }
    // Every span is at least seven bytes, so a lying count cannot force a large allocation.
    let mut spans = Vec::with_capacity(span_count.min(bytes.len() / 7));
    for index in 0..span_count {
        let y = reader.u16()?;
        let x = reader.u16()?;
        let count = reader.u16()?;
        if count == 0 || y >= height || x.checked_add(count).is_none_or(|end| end > width) {
            return Err(DecodeError::Malformed("span out of bounds"));
        }
        if full && (x != 0 || count != width || usize::from(y) != index) {
            return Err(DecodeError::Malformed("full frame must cover each row"));
        }
        let mut style = Style::default();
        let mut link = 0u32;
        let mut cells = Vec::with_capacity(usize::from(count).min(bytes.len() - reader.at));
        for _ in 0..count {
            let tag = reader.u8()?;
            let width = (tag >> TAG_WIDTH_SHIFT) & 0x03;
            if width > 2 {
                return Err(DecodeError::Malformed("invalid cell width"));
            }
            let text_len = match tag >> TAG_TEXT_SHIFT {
                TAG_TEXT_EXTENDED => usize::from(reader.u16()?),
                inline => usize::from(inline),
            };
            if (width == 0) != (text_len == 0) {
                return Err(DecodeError::Malformed("invalid cell text"));
            }
            if tag & TAG_STYLE != 0 {
                style = Style {
                    fg: reader.u32()?,
                    bg: reader.u32()?,
                    underline_color: reader.u32()?,
                    attrs: reader.u16()?,
                };
                if !valid_color(style.fg)
                    || !valid_color(style.bg)
                    || !valid_color(style.underline_color)
                {
                    return Err(DecodeError::Malformed("invalid color"));
                }
            }
            if tag & TAG_LINK != 0 {
                link = reader.u32()?;
            }
            let text = reader.utf8(text_len)?.to_owned();
            cells.push(DecodedCell {
                text,
                width,
                fg: style.fg,
                bg: style.bg,
                underline_color: style.underline_color,
                attrs: style.attrs,
                link,
            });
        }
        spans.push(DecodedSpan { y, x, cells });
    }
    if reader.at != bytes.len() {
        return Err(DecodeError::Malformed("trailing bytes"));
    }
    Ok(DecodedFrame {
        full,
        seq,
        base_seq,
        width,
        height,
        cursor,
        links,
        spans,
    })
}

/// Reference client state: applies decoded frames exactly as a display client must.
#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub(crate) struct ReferenceGrid {
    pub width: u16,
    pub height: u16,
    pub seq: u64,
    pub cells: Vec<DecodedCell>,
    pub cursor: Option<CursorState>,
    pub links: HashMap<u32, String>,
}

#[cfg(test)]
impl ReferenceGrid {
    /// Applies one record atomically; on error the grid is unchanged.
    pub(crate) fn apply(&mut self, record: &[u8]) -> Result<(), &'static str> {
        let frame = decode(record).map_err(|_| "decode")?;
        let mut next = if frame.full {
            Self {
                width: frame.width,
                height: frame.height,
                cells: vec![
                    DecodedCell {
                        text: " ".into(),
                        width: 1,
                        fg: 0,
                        bg: 0,
                        underline_color: 0,
                        attrs: 0,
                        link: 0,
                    };
                    usize::from(frame.width) * usize::from(frame.height)
                ],
                ..Self::default()
            }
        } else {
            if self.seq == 0 || frame.base_seq != self.seq {
                return Err("base sequence mismatch");
            }
            if frame.width != self.width || frame.height != self.height {
                return Err("patch size mismatch");
            }
            self.clone()
        };
        for (id, uri) in frame.links {
            if next.links.get(&id).is_some_and(|known| *known != uri) {
                return Err("link id reused");
            }
            next.links.insert(id, uri);
        }
        for span in frame.spans {
            for (offset, cell) in span.cells.into_iter().enumerate() {
                if cell.link != 0 && !next.links.contains_key(&cell.link) {
                    return Err("unknown link id");
                }
                let index =
                    usize::from(span.y) * usize::from(next.width) + usize::from(span.x) + offset;
                next.cells[index] = cell;
            }
        }
        next.cursor = frame.cursor;
        next.seq = frame.seq;
        *self = next;
        Ok(())
    }

    pub(crate) fn cell(&self, x: u16, y: u16) -> &DecodedCell {
        &self.cells[usize::from(y) * usize::from(self.width) + usize::from(x)]
    }

    pub(crate) fn link_uri(&self, cell: &DecodedCell) -> Option<&str> {
        (cell.link != 0).then(|| self.links[&cell.link].as_str())
    }
}

#[cfg(test)]
#[path = "terminal_cells_tests.rs"]
mod tests;
