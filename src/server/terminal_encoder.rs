//! Per-connection terminal frame encoder: ANSI, or negotiated terminal cells.

use crate::protocol::render_ansi::{BlitEncoder, EncodedBlit, RowRevisions};
use crate::protocol::terminal_cells::{CellsEncoder, EncodedCells};
use crate::protocol::{CursorState, FrameData, PaneSurfacePatchRow};

/// Encodes terminal frames for one direct terminal connection. A connection
/// starts with ANSI and switches to cells only through the negotiated hello,
/// before its first frame. A frame the cell codec cannot carry exactly is sent
/// as a full ANSI repaint instead, and committing it switches the connection to
/// ANSI for the rest of its life.
pub(crate) enum TerminalEncoder {
    Ansi(BlitEncoder),
    Cells(CellsEncoder),
}

pub(crate) enum TerminalEncoded {
    Ansi(EncodedBlit),
    Cells(EncodedCells),
}

impl TerminalEncoded {
    pub(crate) fn full(&self) -> bool {
        match self {
            Self::Ansi(encoded) => encoded.full,
            Self::Cells(encoded) => encoded.full,
        }
    }
}

impl TerminalEncoder {
    pub(crate) fn ansi() -> Self {
        Self::Ansi(BlitEncoder::new())
    }

    /// Switches a connection that has not received a frame yet to cells.
    pub(crate) fn enable_cells(&mut self) -> bool {
        if matches!(self, Self::Ansi(encoder) if encoder.last_frame().is_none()) {
            *self = Self::Cells(CellsEncoder::new());
        }
        matches!(self, Self::Cells(_))
    }

    /// Drops the committed baseline; the next frame is a full frame of the same kind.
    pub(crate) fn reset(&mut self) {
        *self = match *self {
            Self::Ansi(_) => Self::ansi(),
            Self::Cells(_) => Self::Cells(CellsEncoder::new()),
        };
    }

    pub(crate) fn is_current(&self, frame: &FrameData) -> bool {
        match self {
            Self::Ansi(encoder) => encoder.is_current(frame),
            Self::Cells(encoder) => encoder.is_current(frame),
        }
    }

    pub(crate) fn last_frame(&self) -> Option<&FrameData> {
        match self {
            Self::Ansi(encoder) => encoder.last_frame(),
            Self::Cells(encoder) => encoder.last_frame(),
        }
    }

    /// Encodes `frame` as sequence `seq`; `base_seq` is the committed frame's sequence.
    pub(crate) fn encode(
        &self,
        frame: &FrameData,
        repaint: bool,
        seq: u64,
        base_seq: u64,
    ) -> TerminalEncoded {
        self.encode_with_optional_revisions(frame, repaint, seq, base_seq, None)
    }

    /// Like [`Self::encode`] for a frame whose rows carry screen revisions.
    /// ANSI row damage then compares only rows whose revision changed. The
    /// cell codec diffs every cell and does not use them, but an ANSI
    /// fallback repaint keeps them so the ANSI diffs after it can.
    pub(crate) fn encode_with_row_revisions(
        &self,
        frame: &FrameData,
        repaint: bool,
        seq: u64,
        base_seq: u64,
        revisions: RowRevisions,
    ) -> TerminalEncoded {
        self.encode_with_optional_revisions(frame, repaint, seq, base_seq, Some(revisions))
    }

    fn encode_with_optional_revisions(
        &self,
        frame: &FrameData,
        repaint: bool,
        seq: u64,
        base_seq: u64,
        revisions: Option<RowRevisions>,
    ) -> TerminalEncoded {
        let encode_ansi = |encoder: &BlitEncoder, repaint, revisions| match revisions {
            Some(revisions) => encoder.encode_with_row_revisions(frame, repaint, revisions),
            None => encoder.encode(frame, repaint),
        };
        match self {
            Self::Ansi(encoder) => TerminalEncoded::Ansi(encode_ansi(encoder, repaint, revisions)),
            Self::Cells(encoder) => match encoder.encode(frame, repaint, seq, base_seq) {
                Ok(encoded) => TerminalEncoded::Cells(encoded),
                Err(reason) => {
                    tracing::info!(
                        reason = reason.0,
                        "terminal frame needs ANSI; switching connection from cells"
                    );
                    // A fresh encoder clears and repaints every cell.
                    TerminalEncoded::Ansi(encode_ansi(&BlitEncoder::new(), false, revisions))
                }
            },
        }
    }

    pub(crate) fn commit(&mut self, frame: FrameData, encoded: TerminalEncoded) {
        match encoded {
            TerminalEncoded::Ansi(encoded) => {
                // An ANSI frame on a cells connection is the fallback repaint.
                if matches!(self, Self::Cells(_)) {
                    *self = Self::ansi();
                }
                if let Self::Ansi(encoder) = self {
                    encoder.commit(frame, encoded);
                }
            }
            TerminalEncoded::Cells(encoded) => match self {
                Self::Cells(encoder) => encoder.commit(frame, encoded),
                Self::Ansi(_) => tracing::warn!("dropping a cells frame prepared for ANSI"),
            },
        }
    }

    /// Encodes retained row damage; `None` sends the caller to the complete renderer.
    pub(crate) fn encode_patch(
        &self,
        rows: &[PaneSurfacePatchRow],
        cursor: Option<CursorState>,
        seq: u64,
        base_seq: u64,
    ) -> Option<TerminalEncoded> {
        match self {
            Self::Ansi(encoder) => encoder
                .encode_patch(rows, cursor, false)
                .map(TerminalEncoded::Ansi),
            Self::Cells(encoder) => encoder
                .encode_patch(rows, cursor, seq, base_seq)
                .map(TerminalEncoded::Cells),
        }
    }

    pub(crate) fn commit_patch(
        &mut self,
        rows: &[PaneSurfacePatchRow],
        cursor: Option<CursorState>,
        encoded: TerminalEncoded,
    ) -> bool {
        match (self, encoded) {
            (Self::Ansi(encoder), TerminalEncoded::Ansi(encoded)) => {
                encoder.commit_patch(rows, cursor, encoded)
            }
            (Self::Cells(encoder), TerminalEncoded::Cells(encoded)) => {
                encoder.commit_patch(rows, cursor, encoded)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
#[path = "terminal_encoder_tests.rs"]
mod tests;
