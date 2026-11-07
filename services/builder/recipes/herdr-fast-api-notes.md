# Notes on the herdr-fast-api build recipe

`herdr-fast-api.json` and its lock are what Portal's build service reads from this repository's main.

## What it covers, and what it does not

This recipe is the **Linux CI gate**. It is not all of `just check` and it does not replace GitHub CI.

It runs `just lint`, `just ci-tests`, `just docs-contract-test` and the scalar libghostty check of the Linux CI job.

It does not run:

- `just windows-lint`, the fourth part of `just check` on Linux. It needs the Windows SDK from `xwin`, and that needs
  Microsoft's SDK license accepted. That has not been decided for the build machines.
- The native Windows and ConPTY jobs. A cross-compile lint would not replace these either.
- The macOS jobs and the other GitHub workflows.

So a Windows-only or macOS-only break passes this recipe and is caught only by GitHub CI. Keep those GitHub checks
required until a decision says otherwise.

If the license is accepted, add `windows-lint`: the `x86_64-pc-windows-msvc` target on the rustup line, a pinned `xwin`,
an image-time `xwin --accept-license splat` into a fixed folder, `LIBGHOSTTY_VT_WINDOWS_LIBC` pointing at its `libc.txt`,
and `add_test(windows-lint ...)`.

## Time limits

The recipe sets no time limit on a test: no `tests.timeout_s`, and `TIMEOUT 0` on every test, so CTest never stops one
for taking long.

The service's own liveness rule still applies. It stops a test step whose output has not grown for 780 seconds (its
default 480 plus 300). CTest prints nothing while a test runs and these tests are whole `just` recipes, so that silence
is one test's running time: a healthy check that runs longer than 780 seconds is stopped, requeued once, then lost.
This is the limit to look at first if a check on a slow machine is reported stuck.

## Compile jobs

Tests can compile, because a machine that receives the build gets a fresh source checkout. `compile-jobs.sh`, written by
the build step, sets `CARGO_BUILD_JOBS` on the machine running the test: its vCPUs, at most its memory at 2 GiB a job.
Keep that 2 GiB equal to `build_job_memory_gib`. The `cargo` resource lock lets one check compile at a time.

## Changing the toolchain

Rust runs through rustup, so `rust-toolchain.toml` still decides the version. A channel the image does not have fails
the check, so a toolchain bump needs a recipe update in the same change.
