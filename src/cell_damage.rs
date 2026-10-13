//! Optional packed row damage for the server's terminal-ANSI frame encoder.
//!
//! `HERDR_RENDER_ACCEL` selects a backend for the server process only: `cpu`
//! packs frames and compares them on the CPU (the measurement control), and
//! `cuda` compares them on the GPU in builds with the `cuda` feature. Unset,
//! encoders keep their existing cell-by-cell diff. Damage only tells the
//! encoder which rows are equal to its committed frame so it can skip them;
//! every byte it writes still comes from the CPU diff of the remaining rows.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use cell_damage::{inline_symbol, interned_symbol, GridRef, PackedCell, RowDamage};

use crate::protocol::render_ansi::{is_overwide, sanitized_frame_hyperlinks};
use crate::protocol::{CellData, FrameData, PaneSurfacePatchRow};

const ENV_VAR: &str = "HERDR_RENDER_ACCEL";
/// Bounds the symbol and hyperlink interner; overflowing starts a new epoch.
const MAX_INTERNED: usize = 1 << 16;
const FLAG_SKIP: u16 = 1;
/// Wider than two columns; see [`PackedFrame::overwide_rows`].
const FLAG_OVERWIDE: u16 = 2;

static SERVER_BACKEND: OnceLock<Option<Backend>> = OnceLock::new();
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
thread_local! {
    static TEST_BACKEND: std::cell::Cell<Option<Option<Backend>>> =
        const { std::cell::Cell::new(None) };
    static TEST_DAMAGED_FRAMES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Runs `test` with `backend` as the server backend for encoders created on
/// this thread, as `HERDR_RENDER_ACCEL` would select it for a server.
#[cfg(test)]
pub(crate) fn with_test_backend<R>(backend: Option<Backend>, test: impl FnOnce() -> R) -> R {
    let previous = TEST_BACKEND.with(|cell| cell.replace(Some(backend)));
    let result = test();
    TEST_BACKEND.with(|cell| cell.set(previous));
    result
}

/// Frames this thread encoded with row damage so far.
#[cfg(test)]
pub(crate) fn test_damaged_frames() -> usize {
    TEST_DAMAGED_FRAMES.with(std::cell::Cell::get)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    Cpu,
    #[cfg(feature = "cuda")]
    Cuda,
}

/// Reads `HERDR_RENDER_ACCEL` for this server process and, for CUDA, opens
/// the device on a background thread. Clients never call this, so their
/// encoders keep the existing diff.
pub(crate) fn configure_server_from_env() {
    let backend =
        *SERVER_BACKEND.get_or_init(|| parse_backend(std::env::var(ENV_VAR).ok().as_deref()));
    let Some(backend) = backend else {
        return;
    };
    tracing::info!(?backend, "render row damage enabled");
    #[cfg(feature = "cuda")]
    if backend == Backend::Cuda {
        cuda::start();
    }
}

fn parse_backend(value: Option<&str>) -> Option<Backend> {
    match value? {
        "" | "0" | "off" => None,
        "cpu" => Some(Backend::Cpu),
        #[cfg(feature = "cuda")]
        "cuda" => Some(Backend::Cuda),
        other => {
            tracing::warn!(
                value = other,
                "unsupported {ENV_VAR} (cuda needs a build with the cuda feature); row damage stays off"
            );
            None
        }
    }
}

fn server_backend() -> Option<Backend> {
    SERVER_BACKEND.get().copied().flatten()
}

fn next_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// A frame packed for byte equality. Two packed cells are equal only when the
/// encoder would treat the cells as visually identical: same symbol, colors
/// and modifier bits (underline style included), same sanitized OSC 8 target,
/// and the same ratatui skip flag.
pub(crate) struct PackedFrame {
    generation: u64,
    epoch: u64,
    width: u16,
    height: u16,
    cells: Vec<PackedCell>,
    /// Rows holding a symbol wider than two columns. The encoder's diff can
    /// rewrite cells of such a row even when it is equal, so they are always
    /// reported changed.
    overwide_rows: Vec<bool>,
}

impl PackedFrame {
    fn grid(&self) -> GridRef<'_> {
        GridRef {
            generation: self.generation,
            width: usize::from(self.width),
            cells: &self.cells,
        }
    }

    fn describes(&self, frame: &FrameData, epoch: u64) -> bool {
        self.epoch == epoch && self.width == frame.width && self.height == frame.height
    }

    /// Repacks `width` cells of `frame` from `start`, which lie in `row`.
    fn repack(
        &mut self,
        packer: &mut CellPacker,
        frame_cells: &[CellData],
        start: usize,
        row: usize,
        link_keys: &[u32],
    ) {
        let width = usize::from(self.width);
        for (packed, cell) in self.cells[start..].iter_mut().zip(frame_cells) {
            *packed = packer.cell(cell, link_keys);
        }
        let row_start = row * width;
        self.overwide_rows[row] = self.cells[row_start..row_start + width]
            .iter()
            .any(|cell| cell.flags & FLAG_OVERWIDE != 0);
    }
}

struct CellPacker {
    /// Ids for symbols longer than seven bytes and for sanitized hyperlink
    /// URIs, shared by every frame of one encoder so equal text packs equally
    /// whatever hyperlink-table index it has in either frame.
    ids: HashMap<String, u32>,
    epoch: u64,
    max_interned: usize,
}

impl Default for CellPacker {
    fn default() -> Self {
        Self {
            ids: HashMap::new(),
            epoch: 0,
            max_interned: MAX_INTERNED,
        }
    }
}

impl CellPacker {
    fn begin_frame(&mut self) {
        if self.ids.len() >= self.max_interned {
            self.ids.clear();
            self.epoch += 1;
        }
    }

    fn intern(&mut self, text: &str) -> u32 {
        if let Some(&id) = self.ids.get(text) {
            return id;
        }
        let id = self.ids.len() as u32;
        self.ids.insert(text.to_owned(), id);
        id
    }

    /// Packed link key per hyperlink-table index; 0 means no usable link.
    fn link_keys(&mut self, frame: &FrameData) -> Vec<u32> {
        sanitized_frame_hyperlinks(frame)
            .iter()
            .map(|uri| uri.as_deref().map_or(0, |uri| self.intern(uri) + 1))
            .collect()
    }

    fn cell(&mut self, cell: &CellData, link_keys: &[u32]) -> PackedCell {
        let symbol = match inline_symbol(cell.symbol.as_bytes()) {
            Some(symbol) => symbol,
            None => interned_symbol(self.intern(&cell.symbol)),
        };
        PackedCell {
            symbol,
            fg: cell.fg,
            bg: cell.bg,
            modifier: cell.modifier,
            flags: (if cell.skip { FLAG_SKIP } else { 0 }) | overwide_flag(cell),
            link: cell
                .hyperlink
                .and_then(|index| link_keys.get(index as usize).copied())
                .unwrap_or(0),
        }
    }

    fn pack(&mut self, frame: &FrameData) -> PackedFrame {
        let link_keys = self.link_keys(frame);
        let width = usize::from(frame.width).max(1);
        let mut overwide_rows = vec![false; usize::from(frame.height)];
        let cells = frame
            .cells
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                let packed = self.cell(cell, &link_keys);
                if packed.flags & FLAG_OVERWIDE != 0 {
                    overwide_rows[index / width] = true;
                }
                packed
            })
            .collect();
        PackedFrame {
            generation: next_generation(),
            epoch: self.epoch,
            width: frame.width,
            height: frame.height,
            cells,
            overwide_rows,
        }
    }

    /// `frame` packed by copying `baseline` and repacking only `rows`.
    fn pack_rows(
        &mut self,
        baseline: &PackedFrame,
        frame: &FrameData,
        rows: &[u16],
    ) -> PackedFrame {
        let link_keys = self.link_keys(frame);
        let width = usize::from(frame.width);
        let mut packed = PackedFrame {
            generation: next_generation(),
            epoch: self.epoch,
            width: frame.width,
            height: frame.height,
            cells: baseline.cells.clone(),
            overwide_rows: baseline.overwide_rows.clone(),
        };
        for &row in rows {
            let start = usize::from(row) * width;
            packed.repack(
                self,
                &frame.cells[start..start + width],
                start,
                usize::from(row),
                &link_keys,
            );
        }
        packed
    }
}

/// Row damage of one frame against the encoder's committed frame.
pub(crate) struct Damage {
    /// One entry per frame row; unchanged rows can be skipped by the encoder.
    pub(crate) rows: Vec<RowDamage>,
    /// The packed next frame, kept as the baseline once the frame is committed.
    pub(crate) packed: PackedFrame,
}

/// Per-encoder damage state, boxed and absent while row damage is off so
/// every encoder stays small.
pub(crate) struct FrameDamage {
    state: Option<Box<DamageState>>,
}

/// The backend, the interner and the packed form of the committed frame.
struct DamageState {
    backend: Backend,
    packer: Mutex<CellPacker>,
    baseline: Option<PackedFrame>,
}

impl Default for FrameDamage {
    fn default() -> Self {
        #[cfg(test)]
        if let Some(backend) = TEST_BACKEND.with(std::cell::Cell::get) {
            return Self::with_backend(backend);
        }
        Self::with_backend(server_backend())
    }
}

impl FrameDamage {
    pub(crate) fn with_backend(backend: Option<Backend>) -> Self {
        Self {
            state: backend.map(|backend| {
                Box::new(DamageState {
                    backend,
                    packer: Mutex::default(),
                    baseline: None,
                })
            }),
        }
    }

    /// A tiny interner bound, so tests cross epochs within a few frames.
    #[cfg(test)]
    fn with_interner_bound(backend: Backend, max_interned: usize) -> Self {
        Self {
            state: Some(Box::new(DamageState {
                backend,
                packer: Mutex::new(CellPacker {
                    max_interned,
                    ..CellPacker::default()
                }),
                baseline: None,
            })),
        }
    }

    /// Damage of `next` against `prev`, the committed frame. Rows outside
    /// `changed_rows`, when given, must equal `prev` and are not packed or
    /// compared. `None` means the caller keeps its full diff.
    pub(crate) fn damage(
        &self,
        prev: &FrameData,
        next: &FrameData,
        changed_rows: Option<&[u16]>,
    ) -> Option<Damage> {
        let state = self.state.as_deref()?;
        let backend = state.backend;
        let cells = usize::from(next.width) * usize::from(next.height);
        if prev.width != next.width
            || prev.height != next.height
            || prev.cells.len() != cells
            || next.cells.len() != cells
            || changed_rows.is_some_and(|rows| rows.iter().any(|&row| row >= next.height))
        {
            return None;
        }
        if !backend_ready(backend) {
            crate::render_prof::event("cell_damage.backend_not_ready");
            return None;
        }
        let started = crate::render_prof::timer();
        let mut packer = state.packer.lock().ok()?;
        packer.begin_frame();
        let epoch = packer.epoch;
        let packed_prev;
        let prev_packed = match state.baseline.as_ref().filter(|b| b.describes(prev, epoch)) {
            Some(baseline) => baseline,
            None => {
                crate::render_prof::event("cell_damage.pack_baseline");
                packed_prev = packer.pack(prev);
                &packed_prev
            }
        };
        let packed = match changed_rows {
            Some(rows) => packer.pack_rows(prev_packed, next, rows),
            None => packer.pack(next),
        };
        drop(packer);
        crate::render_prof::duration_since("cell_damage.pack", started);
        let rows =
            changed_rows.map(|rows| rows.iter().map(|&row| u32::from(row)).collect::<Vec<_>>());
        let mut damage = compare(backend, prev_packed, &packed, rows.as_deref())?;
        let full_row = RowDamage {
            start: 0,
            end: u32::from(next.width),
        };
        for (damage, &overwide) in damage.iter_mut().zip(&packed.overwide_rows) {
            if overwide {
                *damage = full_row;
            }
        }
        crate::render_prof::duration_since("cell_damage.total", started);
        #[cfg(test)]
        TEST_DAMAGED_FRAMES.with(|count| count.set(count.get() + 1));
        Some(Damage {
            rows: damage,
            packed,
        })
    }

    /// Records the packed form of the frame the encoder just committed, or
    /// forgets the baseline when the frame was encoded without damage.
    pub(crate) fn commit(&mut self, packed: Option<PackedFrame>) {
        if let Some(state) = self.state.as_deref_mut() {
            state.baseline = packed;
        }
    }

    /// Keeps the baseline equal to the committed frame after `rows` were
    /// written into it.
    pub(crate) fn apply_patch(&mut self, frame: &FrameData, rows: &[PaneSurfacePatchRow]) {
        let Some(state) = self.state.as_deref_mut() else {
            return;
        };
        let Some(mut baseline) = state.baseline.take() else {
            return;
        };
        let Ok(mut packer) = state.packer.lock() else {
            return;
        };
        if !baseline.describes(frame, packer.epoch) {
            return;
        }
        let link_keys = packer.link_keys(frame);
        let width = usize::from(frame.width);
        for row in rows {
            let start = usize::from(row.y) * width + usize::from(row.x);
            if row.y >= frame.height || usize::from(row.x) + row.cells.len() > width {
                return;
            }
            baseline.repack(
                &mut packer,
                &row.cells,
                start,
                usize::from(row.y),
                &link_keys,
            );
        }
        baseline.generation = next_generation();
        drop(packer);
        state.baseline = Some(baseline);
    }
}

/// [`FLAG_OVERWIDE`] when the encoder treats `cell` as wider than two
/// columns.
fn overwide_flag(cell: &CellData) -> u16 {
    if is_overwide(cell) {
        FLAG_OVERWIDE
    } else {
        0
    }
}

fn backend_ready(backend: Backend) -> bool {
    match backend {
        Backend::Cpu => true,
        #[cfg(feature = "cuda")]
        Backend::Cuda => cuda::ready(),
    }
}

fn compare(
    backend: Backend,
    prev: &PackedFrame,
    next: &PackedFrame,
    rows: Option<&[u32]>,
) -> Option<Vec<RowDamage>> {
    let mut out = Vec::new();
    match backend {
        Backend::Cpu => {
            let started = crate::render_prof::timer();
            cell_damage::cpu_row_damage(prev.grid(), next.grid(), rows, &mut out).ok()?;
            crate::render_prof::duration_since("cell_damage.cpu", started);
        }
        #[cfg(feature = "cuda")]
        Backend::Cuda => {
            cuda::row_damage(prev.grid(), next.grid(), rows, &mut out)?;
        }
    }
    Some(out)
}

#[cfg(feature = "cuda")]
mod cuda {
    use std::sync::{Mutex, OnceLock};

    use cell_damage::{CudaDamage, DeviceTimings, GridRef, RowDamage};

    enum State {
        Starting,
        Ready(Box<CudaDamage>),
        Unavailable,
    }

    static STATE: OnceLock<Mutex<State>> = OnceLock::new();

    fn state() -> &'static Mutex<State> {
        STATE.get_or_init(|| Mutex::new(State::Starting))
    }

    fn opened() -> State {
        match CudaDamage::open() {
            Ok(cuda) => {
                tracing::info!(
                    device = cuda.device_name(),
                    compute_capability = ?cuda.compute_capability(),
                    "CUDA row damage ready"
                );
                State::Ready(Box::new(cuda))
            }
            Err(error) => {
                tracing::warn!(%error, "CUDA row damage unavailable; using the CPU frame diff");
                State::Unavailable
            }
        }
    }

    /// Opens the device off the render path. Frames keep the existing diff
    /// until it is ready, and for good if it cannot open.
    pub(super) fn start() {
        state();
        let spawned = std::thread::Builder::new()
            .name("herdr-cuda-init".into())
            .spawn(|| {
                let opened = opened();
                if let Ok(mut state) = state().lock() {
                    *state = opened;
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not start CUDA initialization");
            if let Ok(mut state) = state().lock() {
                *state = State::Unavailable;
            }
        }
    }

    /// Opens the device on the calling thread if no attempt has finished.
    #[cfg(test)]
    pub(crate) fn open_blocking() -> Result<String, String> {
        let mut state = state().lock().map_err(|error| error.to_string())?;
        if matches!(*state, State::Starting) {
            *state = opened();
        }
        match &*state {
            State::Ready(cuda) => Ok(cuda.device_name().to_owned()),
            _ => Err("CUDA row damage did not open; see the log".into()),
        }
    }

    pub(super) fn ready() -> bool {
        matches!(state().lock().as_deref(), Ok(State::Ready(_)))
    }

    pub(super) fn row_damage(
        prev: GridRef<'_>,
        next: GridRef<'_>,
        rows: Option<&[u32]>,
        out: &mut Vec<RowDamage>,
    ) -> Option<DeviceTimings> {
        let mut state = state().lock().ok()?;
        let State::Ready(cuda) = &mut *state else {
            return None;
        };
        let started = crate::render_prof::timer();
        match cuda.row_damage(prev, next, rows, out) {
            Ok(timings) => {
                crate::render_prof::duration("cell_damage.cuda.upload", timings.upload);
                crate::render_prof::duration("cell_damage.cuda.alloc_free", timings.alloc_free);
                crate::render_prof::duration("cell_damage.cuda.device_copy", timings.device_copy);
                crate::render_prof::duration("cell_damage.cuda.launch_sync", timings.launch_sync);
                crate::render_prof::duration("cell_damage.cuda.readback", timings.readback);
                crate::render_prof::counter(
                    "cell_damage.cuda.upload_bytes",
                    timings.uploaded_bytes,
                );
                crate::render_prof::duration_since("cell_damage.cuda.total", started);
                Some(timings)
            }
            Err(error) => {
                tracing::warn!(%error, "CUDA row damage failed; using the CPU frame diff from now on");
                *state = State::Unavailable;
                None
            }
        }
    }
}

#[cfg(all(test, feature = "cuda"))]
pub(crate) use cuda::open_blocking as open_cuda_for_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::render_ansi::{BlitEncoder, EncodedBlit, RowRevisions};
    use crate::protocol::CursorState;

    /// Narrow, wide (emoji, CJK, half-width voiced kana), empty, combining,
    /// interned (longer than seven bytes), control, and wider than two
    /// columns ("abc").
    const SYMBOLS: &[&str] = &[
        "a",
        "b",
        " ",
        "",
        "💡",
        "漢",
        "ｶ\u{ff9e}",
        "👩\u{200d}👩\u{200d}👧",
        "🏳\u{fe0f}\u{200d}🌈",
        "e\u{301}",
        "abc",
        "\u{7}",
    ];
    /// Includes URIs that sanitize to the same target and one that sanitizes
    /// to no link at all.
    const URIS: &[&str] = &[
        "https://a.example",
        "https://b.example",
        "https://a.example\u{1b}",
        "\u{7}",
        "https://c.example\u{0}",
    ];
    const COLORS: &[u32] = &[0, 0x01, 0x0100_0010, 0x0200_8040];
    /// Plain, bold, reversed, and underline styles in the high bits.
    const MODIFIERS: &[u16] = &[0, 1, 1 << 6, 1 << 3, 1 << 12, 2 << 12, 3 << 12];

    struct Rng(u64);

    impl Rng {
        fn next_u64(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, bound: usize) -> usize {
            (self.next_u64() % bound as u64) as usize
        }

        fn pick<T: Copy>(&mut self, values: &[T]) -> T {
            values[self.below(values.len())]
        }
    }

    fn random_cell(rng: &mut Rng, links: usize, with_link: bool) -> CellData {
        CellData {
            symbol: rng.pick(SYMBOLS).to_owned(),
            fg: rng.pick(COLORS),
            bg: rng.pick(COLORS),
            modifier: rng.pick(MODIFIERS),
            skip: rng.below(8) == 0,
            // Index `links` is out of range on purpose: it renders unlinked.
            hyperlink: (with_link && rng.below(3) == 0).then(|| rng.below(links + 1) as u32),
        }
    }

    fn random_frame(rng: &mut Rng, width: u16, height: u16) -> FrameData {
        let hyperlinks: Vec<String> = (0..rng.below(URIS.len() + 1))
            .map(|_| rng.pick(URIS).to_owned())
            .collect();
        let cells = (0..usize::from(width) * usize::from(height))
            .map(|_| random_cell(rng, hyperlinks.len(), true))
            .collect();
        FrameData {
            cells,
            width,
            height,
            cursor: random_cursor(rng, width, height),
            hyperlinks,
            graphics: Vec::new(),
        }
    }

    fn random_cursor(rng: &mut Rng, width: u16, height: u16) -> Option<CursorState> {
        (rng.below(3) != 0).then(|| CursorState {
            x: rng.below(usize::from(width)) as u16,
            y: rng.below(usize::from(height)) as u16,
            visible: rng.below(2) == 0,
            shape: rng.below(7) as u8,
        })
    }

    /// `frame` with some cells of `rows` replaced.
    fn edited(rng: &mut Rng, frame: &FrameData, rows: &[u16]) -> FrameData {
        let mut next = frame.clone();
        let width = usize::from(frame.width);
        for &row in rows {
            for _ in 0..1 + rng.below(3) {
                let index = usize::from(row) * width + rng.below(width);
                next.cells[index] = random_cell(rng, next.hyperlinks.len(), true);
            }
        }
        next.cursor = random_cursor(rng, frame.width, frame.height);
        next
    }

    /// The same picture with a reordered hyperlink table and every cell
    /// reindexed into it.
    fn rebound(rng: &mut Rng, frame: &FrameData) -> FrameData {
        let mut order: Vec<usize> = (0..frame.hyperlinks.len()).collect();
        for index in (1..order.len()).rev() {
            order.swap(index, rng.below(index + 1));
        }
        let mut next = frame.clone();
        next.hyperlinks = order
            .iter()
            .map(|&old| frame.hyperlinks[old].clone())
            .collect();
        next.hyperlinks.push(rng.pick(URIS).to_owned());
        for cell in &mut next.cells {
            if let Some(index) = cell.hyperlink {
                cell.hyperlink = Some(
                    order
                        .iter()
                        .position(|&old| old as u32 == index)
                        .map_or(next.hyperlinks.len() as u32, |new| new as u32),
                );
            }
        }
        next
    }

    fn rows_of(rng: &mut Rng, height: u16) -> Vec<u16> {
        (0..height).filter(|_| rng.below(3) == 0).collect()
    }

    /// An encoder with the existing diff and one with row damage, fed the
    /// same frames; every frame must produce the same bytes.
    struct Pair {
        off: BlitEncoder,
        on: BlitEncoder,
        damaged: usize,
        skipped_rows: usize,
    }

    impl Pair {
        fn new(damage: FrameDamage) -> Self {
            Self {
                off: BlitEncoder::with_damage(FrameDamage::with_backend(None)),
                on: BlitEncoder::with_damage(damage),
                damaged: 0,
                skipped_rows: 0,
            }
        }

        fn frame(&mut self, frame: &FrameData, repaint: bool, changed_rows: Option<&[u16]>) {
            let expected = self.off.encode(frame, repaint);
            let actual = match changed_rows {
                Some(rows) => self.on.encode_with_changed_rows(frame, repaint, rows),
                None => self.on.encode(frame, repaint),
            };
            self.check_and_commit(frame, expected, actual);
        }

        fn frame_with_revisions(&mut self, frame: &FrameData, revisions: RowRevisions) {
            let expected = self.off.encode(frame, false);
            let actual = self.on.encode_with_row_revisions(frame, false, revisions);
            self.check_and_commit(frame, expected, actual);
        }

        fn check_and_commit(
            &mut self,
            frame: &FrameData,
            expected: EncodedBlit,
            actual: EncodedBlit,
        ) {
            assert_eq!(
                String::from_utf8_lossy(&actual.bytes),
                String::from_utf8_lossy(&expected.bytes)
            );
            assert_eq!(actual.full, expected.full);
            self.damaged += usize::from(actual.has_row_damage());
            self.skipped_rows += actual.rows_skipped;
            self.off.commit(frame.clone(), expected);
            self.on.commit(frame.clone(), actual);
        }

        fn patch(&mut self, rows: &[PaneSurfacePatchRow], cursor: Option<CursorState>) -> bool {
            let expected = self.off.encode_patch(rows, cursor.clone(), false);
            let actual = self.on.encode_patch(rows, cursor.clone(), false);
            match (expected, actual) {
                (Some(expected), Some(actual)) => {
                    assert_eq!(actual.bytes, expected.bytes);
                    assert!(self.off.commit_patch(rows, cursor.clone(), expected));
                    assert!(self.on.commit_patch(rows, cursor, actual));
                    true
                }
                (None, None) => false,
                _ => panic!("only one encoder accepted the patch"),
            }
        }
    }

    /// Resizes, repaints, hyperlink rebinding and retargeting, patches, and
    /// row edits with and without a changed-row hint.
    fn run_random_parity(damage: FrameDamage, seed: u64, steps: usize) -> usize {
        let mut rng = Rng(seed);
        let mut pair = Pair::new(damage);
        let (mut width, mut height) = (12u16, 6u16);
        let mut frame = random_frame(&mut rng, width, height);
        pair.frame(&frame, false, None);
        for _ in 0..steps {
            match rng.below(20) {
                0 => {
                    width = 1 + rng.below(16) as u16;
                    height = 1 + rng.below(8) as u16;
                    frame = random_frame(&mut rng, width, height);
                    pair.frame(&frame, false, None);
                }
                1 => pair.frame(&frame, true, None),
                2 => {
                    frame = rebound(&mut rng, &frame);
                    pair.frame(&frame, false, None);
                }
                3 if !frame.hyperlinks.is_empty() => {
                    // Same cells, another target: every linked cell changes.
                    let index = rng.below(frame.hyperlinks.len());
                    frame.hyperlinks[index] = rng.pick(URIS).to_owned();
                    pair.frame(&frame, false, None);
                }
                4 => {
                    let y = rng.below(usize::from(height)) as u16;
                    let x = rng.below(usize::from(width));
                    let cells: Vec<CellData> = (0..1 + rng.below(usize::from(width) - x))
                        .map(|_| random_cell(&mut rng, 0, false))
                        .collect();
                    let cursor = random_cursor(&mut rng, width, height);
                    let row = PaneSurfacePatchRow {
                        x: x as u16,
                        y,
                        cells: cells.clone(),
                    };
                    if pair.patch(&[row], cursor.clone()) {
                        let start = usize::from(y) * usize::from(width) + x;
                        frame.cells[start..start + cells.len()].clone_from_slice(&cells);
                        frame.cursor = cursor;
                    }
                }
                _ => {
                    let rows = rows_of(&mut rng, height);
                    let next = edited(&mut rng, &frame, &rows);
                    if rng.below(2) == 0 {
                        // The hint may also list rows that did not change.
                        let mut hint = rows;
                        hint.extend(rows_of(&mut rng, height));
                        pair.frame(&next, false, Some(&hint[..]));
                    } else {
                        pair.frame(&next, false, None);
                    }
                    frame = next;
                }
            }
        }
        pair.damaged
    }

    #[test]
    fn cpu_row_damage_writes_the_same_bytes_as_the_full_diff() {
        for seed in 1..=8 {
            let damaged = run_random_parity(
                FrameDamage::with_backend(Some(Backend::Cpu)),
                seed * 0x9e37_79b9,
                400,
            );
            assert!(damaged > 200, "seed {seed}: only {damaged} damaged frames");
        }
    }

    /// Row edits, patches, changes outside the rows (a new key) and resizes,
    /// with the producer's row revisions. Returns the pair's counters.
    fn run_revision_parity(damage: FrameDamage, seed: u64, steps: usize) -> Pair {
        let mut rng = Rng(seed);
        let mut pair = Pair::new(damage);
        let (mut width, mut height) = (10u16, 5u16);
        let mut frame = random_frame(&mut rng, width, height);
        let mut revisions = RowRevisions {
            key: 1,
            rows: vec![1; usize::from(height)],
        };
        let mut next_revision = 2;
        pair.frame_with_revisions(&frame, revisions.clone());
        for step in 0..steps {
            match step % 50 {
                49 => {
                    // A change outside the rows: the producer changes the key.
                    frame.hyperlinks.push(rng.pick(URIS).to_owned());
                    frame.hyperlinks.swap_remove(0);
                    revisions.key += 1;
                }
                37 => {
                    width = 1 + rng.below(16) as u16;
                    height = 1 + rng.below(8) as u16;
                    frame = random_frame(&mut rng, width, height);
                    revisions = RowRevisions {
                        key: revisions.key + 1,
                        rows: vec![next_revision; usize::from(height)],
                    };
                    next_revision += 1;
                }
                24 => {
                    let y = rng.below(usize::from(height)) as u16;
                    let cells = vec![random_cell(&mut rng, 0, false)];
                    let row = PaneSurfacePatchRow { x: 0, y, cells };
                    if pair.patch(std::slice::from_ref(&row), frame.cursor.clone()) {
                        frame.cells[usize::from(y) * usize::from(width)] = row.cells[0].clone();
                        revisions.rows[usize::from(y)] = next_revision;
                        next_revision += 1;
                    }
                }
                _ => {
                    let rows = rows_of(&mut rng, height);
                    frame = edited(&mut rng, &frame, &rows);
                    for &row in &rows {
                        revisions.rows[usize::from(row)] = next_revision;
                        next_revision += 1;
                    }
                }
            }
            pair.frame_with_revisions(&frame, revisions.clone());
        }
        pair
    }

    #[test]
    fn row_revisions_keep_the_same_bytes() {
        for seed in 1..=4 {
            let pair = run_revision_parity(
                FrameDamage::with_backend(Some(Backend::Cpu)),
                seed * 0x5851_f42d,
                300,
            );
            assert!(
                pair.damaged > 250,
                "seed {seed}: only {} damaged frames",
                pair.damaged
            );
        }
    }

    #[test]
    fn row_revisions_without_a_backend_skip_rows_and_keep_the_same_bytes() {
        let mut skipped_rows = 0;
        for seed in 1..=8 {
            let pair =
                run_revision_parity(FrameDamage::with_backend(None), seed * 0x5851_f42d, 300);
            assert_eq!(pair.damaged, 0);
            skipped_rows += pair.skipped_rows;
        }
        // Random rows often hold a symbol wider than two columns, which is
        // never skipped; the exact count is asserted by the overwide test.
        assert!(skipped_rows > 100, "only {skipped_rows} rows skipped");
    }

    #[test]
    fn interner_epochs_keep_the_same_bytes() {
        let damaged = run_random_parity(
            FrameDamage::with_interner_bound(Backend::Cpu, 3),
            0x1234_5678,
            400,
        );
        assert!(damaged > 200, "only {damaged} damaged frames");
    }

    #[test]
    fn without_a_backend_the_encoder_keeps_its_full_diff() {
        let mut rng = Rng(5);
        let frame = random_frame(&mut rng, 8, 4);
        let next = edited(&mut rng, &frame, &[1]);
        let mut encoder = BlitEncoder::with_damage(FrameDamage::with_backend(None));
        let first = encoder.encode(&frame, false);
        encoder.commit(frame, first);
        assert!(!encoder.encode(&next, false).has_row_damage());
        assert!(!encoder
            .encode_with_changed_rows(&next, false, &[1])
            .has_row_damage());
    }

    #[test]
    fn an_equal_row_with_a_symbol_wider_than_two_columns_is_still_written() {
        let cell = |symbol: &str| CellData {
            symbol: symbol.to_owned(),
            fg: 0,
            bg: 0,
            modifier: 0,
            skip: false,
            hyperlink: None,
        };
        let rows = ["abc", "x", "y", "z", "a", "b", "c", "d"];
        let frame = FrameData {
            cells: rows.iter().map(|symbol| cell(symbol)).collect(),
            width: 4,
            height: 2,
            cursor: None,
            hyperlinks: Vec::new(),
            graphics: Vec::new(),
        };
        let mut next = frame.clone();
        next.cells[7] = cell("D");
        let mut pair = Pair::new(FrameDamage::with_backend(Some(Backend::Cpu)));
        pair.frame(&frame, false, None);
        // The existing diff rewrites "y" in the unchanged first row, after
        // the three-column symbol; row damage must not skip that row.
        let expected = pair.off.encode(&next, false);
        assert!(String::from_utf8_lossy(&expected.bytes).contains('y'));
        pair.frame(&next, false, Some(&[1][..]));
        assert_eq!(pair.damaged, 1);

        // Row revisions alone must not skip it either.
        let mut pair = Pair::new(FrameDamage::with_backend(None));
        let revisions = |rows: Vec<u64>| RowRevisions { key: 1, rows };
        pair.frame_with_revisions(&frame, revisions(vec![1, 1]));
        pair.frame_with_revisions(&next, revisions(vec![1, 2]));
        assert_eq!(pair.skipped_rows, 0);
        let mut plain = next.clone();
        plain.cells[0] = cell("a");
        pair.frame_with_revisions(&plain, revisions(vec![3, 2]));
        pair.frame_with_revisions(&next, revisions(vec![4, 2]));
        assert_eq!(pair.skipped_rows, 2);
    }

    #[test]
    fn mismatched_hints_and_shapes_fall_back_to_the_full_diff() {
        let mut rng = Rng(9);
        let frame = random_frame(&mut rng, 6, 3);
        let damage = FrameDamage::with_backend(Some(Backend::Cpu));
        assert!(damage.damage(&frame, &frame, Some(&[3][..])).is_none());
        let wider = random_frame(&mut rng, 7, 3);
        assert!(damage.damage(&frame, &wider, None).is_none());
        let mut short = frame.clone();
        short.cells.pop();
        assert!(damage.damage(&frame, &short, None).is_none());
    }

    /// Runs only with `HERDR_TEST_CUDA=1` on a machine with a CUDA device,
    /// where a device that does not open fails the test.
    #[cfg(feature = "cuda")]
    #[test]
    fn cuda_row_damage_writes_the_same_bytes_as_the_full_diff() {
        if std::env::var("HERDR_TEST_CUDA").as_deref() != Ok("1") {
            return;
        }
        let device = open_cuda_for_tests().expect("HERDR_TEST_CUDA=1 needs a CUDA device");
        eprintln!("CUDA device: {device}");
        for seed in 1..=4 {
            let damaged = run_random_parity(
                FrameDamage::with_backend(Some(Backend::Cuda)),
                seed * 0x2545_f491,
                400,
            );
            assert!(damaged > 200, "seed {seed}: only {damaged} damaged frames");
            let pair = run_revision_parity(
                FrameDamage::with_backend(Some(Backend::Cuda)),
                seed * 0x5851_f42d,
                300,
            );
            assert!(
                pair.damaged > 250,
                "seed {seed}: only {} damaged frames",
                pair.damaged
            );
        }
    }

    fn text_frame(rng: &mut Rng, width: u16, height: u16) -> FrameData {
        const TEXT: &[&str] = &["a", "e", "t", "o", "n", "s", "r", "-", "1", " ", " ", " "];
        let cells = (0..usize::from(width) * usize::from(height))
            .map(|_| CellData {
                symbol: rng.pick(TEXT).to_owned(),
                fg: if rng.below(8) == 0 {
                    rng.pick(COLORS)
                } else {
                    0
                },
                bg: 0,
                modifier: if rng.below(16) == 0 { 1 } else { 0 },
                skip: false,
                hyperlink: None,
            })
            .collect();
        FrameData {
            cells,
            width,
            height,
            cursor: None,
            hyperlinks: Vec::new(),
            graphics: Vec::new(),
        }
    }

    /// The frames of one scenario and, for each, the rows that changed.
    fn profile_frames(
        scenario: &str,
        width: u16,
        height: u16,
        samples: usize,
    ) -> Vec<(FrameData, Vec<u16>)> {
        let mut rng = Rng(23);
        let all_rows: Vec<u16> = (0..height).collect();
        let mut frames = vec![(text_frame(&mut rng, width, height), all_rows.clone())];
        for sample in 0..samples {
            let prev = &frames[frames.len() - 1].0;
            let next = match scenario {
                "typing" => {
                    let row = (sample % usize::from(height)) as u16;
                    let mut next = prev.clone();
                    let index = usize::from(row) * usize::from(width) + sample % usize::from(width);
                    next.cells[index].symbol = rng.pick(&["x", "y", "z"]).to_owned();
                    (next, vec![row])
                }
                "scroll" => {
                    let mut next = prev.clone();
                    let width = usize::from(width);
                    next.cells.drain(..width);
                    next.cells
                        .extend(text_frame(&mut rng, width as u16, 1).cells);
                    (next, all_rows.clone())
                }
                _ => (text_frame(&mut rng, width, height), all_rows.clone()),
            };
            frames.push(next);
        }
        frames
    }

    fn profile_paths() -> Vec<(&'static str, Option<Backend>, bool)> {
        #[allow(unused_mut)] // Only CUDA builds add paths.
        let mut paths = vec![
            ("full diff (existing)", None, false),
            ("revisions, no packing", None, true),
            ("cpu packed", Some(Backend::Cpu), false),
            ("cpu packed + revisions", Some(Backend::Cpu), true),
        ];
        #[cfg(feature = "cuda")]
        if std::env::var("HERDR_TEST_CUDA").as_deref() == Ok("1") {
            let device = open_cuda_for_tests().expect("HERDR_TEST_CUDA=1 needs a CUDA device");
            println!("CUDA device: {device}");
            paths.push(("cuda", Some(Backend::Cuda), false));
            paths.push(("cuda + revisions", Some(Backend::Cuda), true));
        }
        paths
    }

    fn percentiles(samples: &mut [std::time::Duration]) -> String {
        samples.sort_unstable();
        let at = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
        format!(
            "p50 {:.1?} p95 {:.1?} p99 {:.1?}",
            at(0.50),
            at(0.95),
            at(0.99)
        )
    }

    /// Host wall time of complete server ANSI encodes per path: packing,
    /// comparison (for CUDA: upload, launch, sync and readback), and the
    /// byte diff. "+ revisions" paths get row revisions bumped exactly for
    /// the changed rows, as a retained screen would report them. Every path
    /// must produce the same bytes. Raw evidence, not a claim.
    #[test]
    #[ignore = "profile; run with --ignored --nocapture, HERDR_TEST_CUDA=1 adds CUDA"]
    fn cell_damage_profile() {
        let samples = 300;
        let paths = profile_paths();
        for (width, height) in [(94u16, 39u16), (457, 127)] {
            for scenario in ["typing", "scroll", "rewrite"] {
                let frames = profile_frames(scenario, width, height, samples);
                let mut revisions = RowRevisions {
                    key: 1,
                    rows: vec![0; usize::from(height)],
                };
                let frame_revisions: Vec<RowRevisions> = (1..)
                    .zip(&frames)
                    .map(|(revision, (_, rows))| {
                        for &row in rows {
                            revisions.rows[usize::from(row)] = revision;
                        }
                        revisions.clone()
                    })
                    .collect();
                let mut expected_bytes = None;
                for &(name, backend, with_revisions) in &paths {
                    let mut encoder = BlitEncoder::with_damage(FrameDamage::with_backend(backend));
                    let encode = |encoder: &BlitEncoder, index: usize| {
                        let frame = &frames[index].0;
                        if with_revisions {
                            encoder.encode_with_row_revisions(
                                frame,
                                false,
                                frame_revisions[index].clone(),
                            )
                        } else {
                            encoder.encode(frame, false)
                        }
                    };
                    let first = encode(&encoder, 0);
                    encoder.commit(frames[0].0.clone(), first);
                    let mut times = Vec::with_capacity(samples);
                    let mut bytes = Vec::new();
                    let mut skipped_rows = 0;
                    for (index, (frame, _)) in frames.iter().enumerate().skip(1) {
                        let started = std::time::Instant::now();
                        let encoded = encode(&encoder, index);
                        times.push(started.elapsed());
                        bytes.extend_from_slice(&encoded.bytes);
                        skipped_rows += encoded.rows_skipped;
                        encoder.commit(frame.clone(), encoded);
                    }
                    let expected = expected_bytes.get_or_insert_with(|| bytes.clone());
                    assert!(*expected == bytes, "{name} wrote different bytes");
                    println!(
                        "{width}x{height} {scenario:<7} {name:<24} {} rows skipped/frame {:.1}",
                        percentiles(&mut times),
                        skipped_rows as f64 / samples as f64
                    );
                }
            }
        }
    }
}
