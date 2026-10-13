//! The CUDA backend against the CPU reference, on a real device.
//!
//! These tests need an NVIDIA driver and NVRTC at run time and run only when
//! `HERDR_TEST_CUDA=1`; once set, a missing device is a failure, not a skip.
#![cfg(all(feature = "cuda", target_os = "linux"))]

use cell_damage::{
    cpu_row_damage, inline_symbol, interned_symbol, CudaDamage, GridRef, PackedCell, RowDamage,
};
use std::time::{Duration, Instant};

fn device() -> Option<CudaDamage> {
    if std::env::var_os("HERDR_TEST_CUDA").is_none_or(|value| value != "1") {
        eprintln!("skipped: set HERDR_TEST_CUDA=1 to run on a CUDA device");
        return None;
    }
    let device = CudaDamage::open().expect("HERDR_TEST_CUDA=1 but CUDA is unavailable");
    eprintln!(
        "device: {} (compute {:?})",
        device.device_name(),
        device.compute_capability()
    );
    Some(device)
}

/// Deterministic xorshift, so a failure reproduces from its seed.
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
}

fn random_cell(rng: &mut Rng) -> PackedCell {
    let symbol = match rng.below(3) {
        0 => inline_symbol(&[b'a' + rng.below(26) as u8]).unwrap(),
        1 => inline_symbol("漢".as_bytes()).unwrap(),
        _ => interned_symbol(rng.below(4) as u32),
    };
    PackedCell {
        symbol,
        fg: rng.below(4) as u32,
        bg: rng.below(4) as u32,
        modifier: rng.below(4) as u16,
        flags: rng.below(2) as u16,
        link: rng.below(3) as u32,
    }
}

fn random_grid(rng: &mut Rng, cells: usize) -> Vec<PackedCell> {
    (0..cells).map(|_| random_cell(rng)).collect()
}

/// Changes a single field of one cell, so every field is exercised alone.
fn mutate(rng: &mut Rng, cell: &mut PackedCell) {
    match rng.below(6) {
        0 => cell.symbol ^= 1,
        1 => cell.fg ^= 1 << 24,
        2 => cell.bg ^= 1,
        3 => cell.modifier ^= 1 << 12,
        4 => cell.flags ^= 1,
        _ => cell.link ^= 1,
    }
}

/// Edits `rows` of a copy of `prev`, some cells per row, and sometimes the
/// first or last column so the span edges are covered.
fn edited(rng: &mut Rng, prev: &[PackedCell], width: usize, rows: &[u32]) -> Vec<PackedCell> {
    let mut next = prev.to_vec();
    for &row in rows {
        let base = row as usize * width;
        match rng.below(4) {
            0 => {}
            1 => mutate(rng, &mut next[base]),
            2 => mutate(rng, &mut next[base + width - 1]),
            _ => {
                for _ in 0..1 + rng.below(4) {
                    let x = rng.below(width);
                    mutate(rng, &mut next[base + x]);
                }
            }
        }
    }
    next
}

fn grid(generation: u64, width: usize, cells: &[PackedCell]) -> GridRef<'_> {
    GridRef {
        generation,
        width,
        cells,
    }
}

fn cpu(prev: GridRef<'_>, next: GridRef<'_>, rows: Option<&[u32]>) -> Vec<RowDamage> {
    let mut out = Vec::new();
    cpu_row_damage(prev, next, rows, &mut out).unwrap();
    out
}

fn assert_parity(
    device: &mut CudaDamage,
    prev: GridRef<'_>,
    next: GridRef<'_>,
    rows: Option<&[u32]>,
    context: &str,
) -> cell_damage::DeviceTimings {
    let mut gpu = Vec::new();
    let timings = device
        .row_damage(prev, next, rows, &mut gpu)
        .unwrap_or_else(|error| panic!("{context}: {error}"));
    assert_eq!(gpu, cpu(prev, next, rows), "{context}");
    timings
}

fn candidate_rows(rng: &mut Rng, height: usize) -> Vec<u32> {
    let mut rows: Vec<u32> = (0..height as u32).filter(|_| rng.below(4) == 0).collect();
    if rows.is_empty() {
        rows.push(rng.below(height) as u32);
    }
    rows
}

#[test]
fn full_grids_match_the_cpu_reference() {
    let Some(mut device) = device() else { return };
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut generation = 0;
    // Widths below, at, and above one 128-thread block, and the two profile sizes.
    for (width, height) in [
        (1, 1),
        (1, 7),
        (7, 3),
        (128, 4),
        (129, 5),
        (94, 39),
        (457, 127),
    ] {
        for _ in 0..4 {
            let prev = random_grid(&mut rng, width * height);
            let all: Vec<u32> = (0..height as u32).collect();
            let next = edited(&mut rng, &prev, width, &all);
            generation += 2;
            assert_parity(
                &mut device,
                grid(generation - 1, width, &prev),
                grid(generation, width, &next),
                None,
                &format!("{width}x{height}"),
            );
        }
    }
}

#[test]
fn chained_candidate_rows_reuse_the_resident_grid_exactly() {
    let Some(mut device) = device() else { return };
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let (width, height) = (94, 39);
    let mut prev = random_grid(&mut rng, width * height);
    for (generation, step) in (1..).zip(0..200) {
        let rows = candidate_rows(&mut rng, height);
        let next = edited(&mut rng, &prev, width, &rows);
        let timings = assert_parity(
            &mut device,
            grid(generation, width, &prev),
            grid(generation + 1, width, &next),
            Some(&rows[..]),
            &format!("step {step}"),
        );
        if step > 0 {
            // The baseline is the grid the previous step left on the device,
            // so only the candidate rows and the row list are uploaded.
            let expected = rows.len() * width * 24 + rows.len() * 4;
            assert_eq!(timings.uploaded_bytes, expected as u64, "step {step}");
        }
        prev = next;
    }
    assert!(device.resident_grids() <= 16);
}

#[test]
fn a_resident_next_grid_is_compared_without_upload() {
    let Some(mut device) = device() else { return };
    let mut rng = Rng(7);
    let (width, height) = (40, 10);
    let prev = random_grid(&mut rng, width * height);
    let all: Vec<u32> = (0..height as u32).collect();
    let next = edited(&mut rng, &prev, width, &all);
    let (a, b) = (grid(1, width, &prev), grid(2, width, &next));
    assert_parity(&mut device, a, b, None, "first");
    let rows = [3, 0, 9, 3];
    let timings = assert_parity(&mut device, a, b, Some(&rows[..]), "again");
    assert_eq!(timings.uploaded_bytes, (rows.len() * 4) as u64);
}

#[test]
fn a_reused_generation_with_another_shape_is_uploaded_again() {
    let Some(mut device) = device() else { return };
    let mut rng = Rng(11);
    let first = random_grid(&mut rng, 80 * 24);
    let first_next = random_grid(&mut rng, 80 * 24);
    assert_parity(
        &mut device,
        grid(1, 80, &first),
        grid(2, 80, &first_next),
        None,
        "80x24",
    );
    // Same generations, same cell count, another width; then a resize.
    for (width, height) in [(120, 16), (100, 30)] {
        let prev = random_grid(&mut rng, width * height);
        let next = random_grid(&mut rng, width * height);
        assert_parity(
            &mut device,
            grid(1, width, &prev),
            grid(2, width, &next),
            None,
            &format!("{width}x{height}"),
        );
    }
}

#[test]
fn eviction_beyond_the_grid_bound_stays_exact() {
    let Some(mut device) = device() else { return };
    let mut rng = Rng(13);
    let (width, height) = (33, 9);
    let grids: Vec<Vec<PackedCell>> = (0..40)
        .map(|_| random_grid(&mut rng, width * height))
        .collect();
    for round in 0..3 {
        for (index, pair) in grids.windows(2).enumerate() {
            let rows = candidate_rows(&mut rng, height);
            // Unlisted rows must be equal, so derive next from prev here.
            let next = edited(&mut rng, &pair[0], width, &rows);
            let generation = 1000 * round + 2 * index as u64;
            assert_parity(
                &mut device,
                grid(generation, width, &pair[0]),
                grid(generation + 1, width, &next),
                Some(&rows[..]),
                &format!("round {round} pair {index}"),
            );
        }
        assert!(device.resident_grids() <= 16);
    }
}

#[test]
fn grids_over_the_byte_bound_are_refused_and_the_device_recovers() {
    let Some(mut device) = device() else { return };
    // Each grid is 72 MiB: one fits the 128 MiB bound, two do not.
    let (width, height) = (1000, 3000);
    let prev = vec![PackedCell::default(); width * height];
    let next = prev.clone();
    let mut out = Vec::new();
    let error = device
        .row_damage(grid(1, width, &prev), grid(2, width, &next), None, &mut out)
        .expect_err("two 72 MiB grids exceed the bound");
    assert_eq!(error.step, "device residency");
    assert_eq!(device.resident_grids(), 0);
    let mut rng = Rng(17);
    let small = random_grid(&mut rng, 20 * 5);
    let small_next = random_grid(&mut rng, 20 * 5);
    assert_parity(
        &mut device,
        grid(3, 20, &small),
        grid(4, 20, &small_next),
        None,
        "after refusal",
    );
}

#[test]
fn shape_errors_and_empty_inputs_match_the_cpu_reference() {
    let Some(mut device) = device() else { return };
    let cells = vec![PackedCell::default(); 6];
    let mut out = Vec::new();
    let error = device
        .row_damage(grid(1, 3, &cells), grid(2, 2, &cells), None, &mut out)
        .expect_err("width mismatch");
    assert_eq!(error.step, "grid shape");
    let error = device
        .row_damage(
            grid(1, 3, &cells),
            grid(2, 3, &cells),
            Some(&[2][..]),
            &mut out,
        )
        .expect_err("row out of range");
    assert_eq!(error.step, "grid shape");
    assert_parity(
        &mut device,
        grid(5, 3, &cells),
        grid(6, 3, &cells),
        Some(&[0u32; 0][..]),
        "no rows",
    );
    assert_parity(&mut device, grid(7, 0, &[]), grid(8, 0, &[]), None, "empty");
}

fn percentiles(samples: &mut [Duration]) -> String {
    samples.sort_unstable();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
    format!(
        "p50 {:.1?} p95 {:.1?} p99 {:.1?}",
        at(0.50),
        at(0.95),
        at(0.99)
    )
}

/// Host wall time of a whole device comparison (upload, launch, sync,
/// readback) next to the CPU reference on the same packed grids. Packing is
/// not included on either side; the Herdr profile measures it.
#[test]
#[ignore = "profile; run with HERDR_TEST_CUDA=1 -- --ignored --nocapture"]
fn cuda_row_damage_profile() {
    let Some(mut device) = device() else { return };
    let samples = 500;
    // Generations stay unique across scenarios: one device keeps grids by
    // generation, and a reused one would be read as the earlier grid.
    let mut first_generation = 1u64;
    for (width, height) in [(94, 39), (457, 127)] {
        for (scenario, rows_per_frame) in [("typing", 1), ("scroll", height), ("rewrite", height)] {
            let mut rng = Rng(19);
            let mut prev = random_grid(&mut rng, width * height);
            let (mut cpu_times, mut gpu_times) = (Vec::new(), Vec::new());
            let (mut upload, mut launch, mut readback) = (Vec::new(), Vec::new(), Vec::new());
            let mut out = Vec::new();
            for generation in (first_generation..).take(samples) {
                let rows: Vec<u32> = if rows_per_frame == 1 {
                    vec![rng.below(height) as u32]
                } else {
                    (0..height as u32).collect()
                };
                let next = match scenario {
                    "scroll" => {
                        let mut next = prev[width..].to_vec();
                        next.extend(random_grid(&mut rng, width));
                        next
                    }
                    "rewrite" => random_grid(&mut rng, width * height),
                    _ => edited(&mut rng, &prev, width, &rows),
                };
                let hint = (rows_per_frame == 1).then_some(rows.as_slice());
                let (a, b) = (
                    grid(generation, width, &prev),
                    grid(generation + 1, width, &next),
                );
                let started = Instant::now();
                cpu_row_damage(a, b, hint, &mut out).unwrap();
                cpu_times.push(started.elapsed());
                let started = Instant::now();
                let timings = device.row_damage(a, b, hint, &mut out).unwrap();
                gpu_times.push(started.elapsed());
                upload.push(timings.upload);
                launch.push(timings.launch_sync);
                readback.push(timings.readback);
                prev = next;
            }
            first_generation += samples as u64 + 1;
            println!(
                "{width}x{height} {scenario:<7} cpu reference      {}",
                percentiles(&mut cpu_times)
            );
            println!(
                "{width}x{height} {scenario:<7} cuda complete      {}",
                percentiles(&mut gpu_times)
            );
            println!(
                "{width}x{height} {scenario:<7} cuda upload        {}",
                percentiles(&mut upload)
            );
            println!(
                "{width}x{height} {scenario:<7} cuda launch+sync   {}",
                percentiles(&mut launch)
            );
            println!(
                "{width}x{height} {scenario:<7} cuda readback      {}",
                percentiles(&mut readback)
            );
        }
    }
}
