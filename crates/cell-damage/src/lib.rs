//! Row damage between two packed terminal cell grids.
//!
//! Every visible attribute of a cell is packed into 24 plain bytes, so two
//! cells are equal exactly when their bytes are equal. [`cpu_row_damage`] is
//! the reference implementation; the optional CUDA backend (feature `cuda`,
//! Linux) computes the same result on the GPU and is tested against it.

#[cfg(feature = "cuda")]
#[cfg_attr(target_os = "linux", path = "cuda.rs")]
#[cfg_attr(not(target_os = "linux"), path = "cuda_unsupported.rs")]
mod cuda;
#[cfg(feature = "cuda")]
pub use cuda::CudaDamage;
#[cfg(all(feature = "cuda", target_os = "linux"))]
mod residency;
#[cfg(all(test, not(all(feature = "cuda", target_os = "linux"))))]
#[allow(dead_code)] // Only the Linux CUDA backend uses it; its tests run everywhere.
mod residency;
#[cfg(feature = "cuda")]
use std::{fmt, time::Duration};

/// One terminal cell packed for byte equality.
///
/// The packer owns the meaning of each field; equal fields must mean equal
/// cells for whatever comparison the caller needs.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackedCell {
    /// [`inline_symbol`] or [`interned_symbol`].
    pub symbol: u64,
    pub fg: u32,
    pub bg: u32,
    pub modifier: u16,
    pub flags: u16,
    pub link: u32,
}

// The CUDA kernel reads cells as three 64-bit words.
const _: () = assert!(std::mem::size_of::<PackedCell>() == 24);
const _: () = assert!(std::mem::align_of::<PackedCell>() == 8);

const INLINE_SYMBOL_MAX: usize = 7;
const INTERNED_SYMBOL_TAG: u64 = 0xff << 56;

/// Packs symbols of up to seven UTF-8 bytes exactly: the bytes plus their length.
pub fn inline_symbol(bytes: &[u8]) -> Option<u64> {
    if bytes.len() > INLINE_SYMBOL_MAX {
        return None;
    }
    let mut word = [0; 8];
    word[..bytes.len()].copy_from_slice(bytes);
    word[7] = bytes.len() as u8;
    Some(u64::from_le_bytes(word))
}

/// Symbol key for a longer symbol, by an id the caller interns consistently
/// for both grids. Its tag byte never collides with an inline length.
pub fn interned_symbol(id: u32) -> u64 {
    INTERNED_SYMBOL_TAG | u64::from(id)
}

/// Changed columns of one row: `start..end`, empty when the row is equal.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowDamage {
    pub start: u32,
    pub end: u32,
}

impl RowDamage {
    pub const UNCHANGED: Self = Self {
        start: u32::MAX,
        end: 0,
    };

    pub fn is_changed(self) -> bool {
        self.start < self.end
    }
}

/// A packed row-major grid. Grids with the same `generation` must hold the
/// same cells; the CUDA backend keeps grids on the device by generation.
#[derive(Clone, Copy)]
pub struct GridRef<'a> {
    pub generation: u64,
    pub width: usize,
    pub cells: &'a [PackedCell],
}

/// Why two grids cannot be compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeError {
    Mismatch,
    RowOutOfRange,
}

/// Grid height, after checking both grids have the same shape.
pub fn checked_height(prev: GridRef<'_>, next: GridRef<'_>) -> Result<usize, ShapeError> {
    if prev.width != next.width
        || prev.cells.len() != next.cells.len()
        || (next.width == 0 && !next.cells.is_empty())
        || (next.width != 0 && next.cells.len() % next.width != 0)
    {
        return Err(ShapeError::Mismatch);
    }
    Ok(next.cells.len().checked_div(next.width).unwrap_or(0))
}

/// Computes the damage of every row in `rows` (all rows when `None`) into
/// `out`, one entry per grid row. Rows not listed are reported unchanged: the
/// caller guarantees they are equal, for example by row revision.
pub fn cpu_row_damage(
    prev: GridRef<'_>,
    next: GridRef<'_>,
    rows: Option<&[u32]>,
    out: &mut Vec<RowDamage>,
) -> Result<(), ShapeError> {
    let height = checked_height(prev, next)?;
    if rows.is_some_and(|rows| rows.iter().any(|&row| row as usize >= height)) {
        return Err(ShapeError::RowOutOfRange);
    }
    out.clear();
    out.resize(height, RowDamage::UNCHANGED);
    let width = next.width;
    let mut damage_row = |row: usize| {
        let range = row * width..(row + 1) * width;
        let (old, new) = (&prev.cells[range.clone()], &next.cells[range]);
        let first = old.iter().zip(new).position(|(a, b)| a != b);
        if let Some(first) = first {
            let last = old
                .iter()
                .zip(new)
                .rposition(|(a, b)| a != b)
                .unwrap_or(first);
            out[row] = RowDamage {
                start: first as u32,
                end: last as u32 + 1,
            };
        }
    };
    match rows {
        Some(rows) => rows.iter().for_each(|&row| damage_row(row as usize)),
        None => (0..height).for_each(damage_row),
    }
    Ok(())
}

#[cfg(feature = "cuda")]
/// A failed CUDA, NVRTC, or library-loading step.
#[derive(Debug, Clone)]
pub struct CudaError {
    pub step: &'static str,
    pub detail: String,
}

#[cfg(feature = "cuda")]
impl fmt::Display for CudaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.step, self.detail)
    }
}

#[cfg(feature = "cuda")]
impl std::error::Error for CudaError {}

#[cfg(feature = "cuda")]
impl From<ShapeError> for CudaError {
    fn from(error: ShapeError) -> Self {
        Self {
            step: "grid shape",
            detail: format!("{error:?}"),
        }
    }
}

#[cfg(feature = "cuda")]
/// Host wall time of each device phase of one comparison.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceTimings {
    /// Baseline and candidate-row uploads, the row list, and the device copy
    /// that seeds the next resident grid.
    pub upload: Duration,
    /// Kernel launch until the device is idle.
    pub launch_sync: Duration,
    /// Copying the per-row damage back to the host.
    pub readback: Duration,
    pub uploaded_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(symbol: &str, fg: u32) -> PackedCell {
        PackedCell {
            symbol: inline_symbol(symbol.as_bytes()).unwrap(),
            fg,
            ..PackedCell::default()
        }
    }

    fn grid(width: usize, rows: &[&str]) -> Vec<PackedCell> {
        let mut cells = Vec::new();
        for row in rows {
            assert_eq!(row.len(), width);
            cells.extend(row.chars().map(|ch| cell(&ch.to_string(), 0)));
        }
        cells
    }

    #[test]
    fn inline_symbols_are_exact_and_distinct_from_interned_ids() {
        assert_ne!(inline_symbol(b""), inline_symbol(b"\0"));
        assert_ne!(inline_symbol(b"a"), inline_symbol(b"a\0"));
        assert_eq!(inline_symbol("é".as_bytes()), inline_symbol("é".as_bytes()));
        assert_eq!(inline_symbol(b"12345678"), None);
        let longest = inline_symbol(b"1234567").unwrap();
        assert_ne!(longest, interned_symbol(0));
        assert_ne!(longest, interned_symbol(u32::MAX));
        assert_ne!(interned_symbol(1), interned_symbol(2));
    }

    #[test]
    fn damage_spans_first_to_last_changed_column() {
        let prev = grid(5, &["abcde", "fghij", "klmno"]);
        let next = grid(5, &["abcde", "fXhYj", "Klmno"]);
        let mut out = Vec::new();
        cpu_row_damage(
            GridRef {
                generation: 1,
                width: 5,
                cells: &prev,
            },
            GridRef {
                generation: 2,
                width: 5,
                cells: &next,
            },
            None,
            &mut out,
        )
        .unwrap();
        assert_eq!(
            out,
            [
                RowDamage::UNCHANGED,
                RowDamage { start: 1, end: 4 },
                RowDamage { start: 0, end: 1 },
            ]
        );
    }

    #[test]
    fn every_packed_field_participates_in_equality() {
        let base = cell("a", 1);
        let variants = [
            PackedCell {
                symbol: base.symbol + 1,
                ..base
            },
            PackedCell { fg: 2, ..base },
            PackedCell { bg: 3, ..base },
            PackedCell {
                modifier: 4,
                ..base
            },
            PackedCell { flags: 5, ..base },
            PackedCell { link: 6, ..base },
        ];
        for variant in variants {
            let mut out = Vec::new();
            cpu_row_damage(
                GridRef {
                    generation: 1,
                    width: 1,
                    cells: &[base],
                },
                GridRef {
                    generation: 2,
                    width: 1,
                    cells: &[variant],
                },
                None,
                &mut out,
            )
            .unwrap();
            assert_eq!(out, [RowDamage { start: 0, end: 1 }], "{variant:?}");
        }
    }

    #[test]
    fn candidate_rows_limit_the_comparison() {
        let prev = grid(2, &["ab", "cd", "ef"]);
        let next = grid(2, &["Xb", "cd", "eY"]);
        let mut out = Vec::new();
        cpu_row_damage(
            GridRef {
                generation: 1,
                width: 2,
                cells: &prev,
            },
            GridRef {
                generation: 2,
                width: 2,
                cells: &next,
            },
            Some(&[2, 1]),
            &mut out,
        )
        .unwrap();
        assert_eq!(
            out,
            [
                RowDamage::UNCHANGED,
                RowDamage::UNCHANGED,
                RowDamage { start: 1, end: 2 },
            ]
        );
    }

    #[test]
    fn shape_errors_are_reported() {
        let three = grid(3, &["abc"]);
        let six = grid(3, &["abc", "def"]);
        let mut out = Vec::new();
        let at = |cells: &[PackedCell], width| GridRef {
            generation: 0,
            width,
            cells,
        };
        assert_eq!(
            cpu_row_damage(at(&three, 3), at(&six, 3), None, &mut out),
            Err(ShapeError::Mismatch)
        );
        assert_eq!(
            cpu_row_damage(at(&six, 2), at(&six, 2), None, &mut out),
            Ok(())
        );
        assert_eq!(
            cpu_row_damage(at(&three, 3), at(&three, 3), Some(&[1]), &mut out),
            Err(ShapeError::RowOutOfRange)
        );
        assert_eq!(
            cpu_row_damage(at(&[], 0), at(&[], 0), None, &mut out),
            Ok(())
        );
        assert!(out.is_empty());
    }
}
