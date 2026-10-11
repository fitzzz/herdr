# Downstream fast API package and recovery rehearsal

This Linux-only package supplements upstream v0.9.3 at
`7b116c05bfda646af39d2524c54e70c751f57ee8`. It does not install over `herdr`,
attach to the user's server, import existing sessions, or migrate conversations.
A next-day live swap remains a separate action requiring Fitz's explicit approval.

## Namespace and update protection

The packaged `bin/herdr-fast-api` launcher verifies the artifact hash, removes
inherited Herdr routing and pane identities, removes common provider-home and
SSH-agent overrides, and sets a private HOME and XDG config/state/data/cache/runtime
hierarchy. It always supplies `--session fast`. These are existing supported
settings; no wire protocol changes are needed. Its default hierarchy is
`$HOME/.local/share/herdr-fast-api/main/`; `--namespace NAME` chooses another
private hierarchy. No original provider configuration, transcript, session JSON,
or agent resume command is copied. Synthetic tests do not run native chat consumers.

The launcher rejects command-line remote/machine/session routing and upstream
update/channel commands. Its private PATH places `bin/herdr` first so integration
hooks launched by its own panes use the same silo. Each pane retains the environment
created by Herdr; a new CLI process deliberately discards the parent's implicit
pane target and requires an explicit target. A child's private HOME marker keeps
its CLI in the same silo. The launcher refuses symlinked namespace directories and
paths too long for Linux sockets. An intentional external program or an explicit
absolute path to the system `herdr` can bypass a launcher; this is an operational
isolation boundary, not a sandbox against programs the user deliberately runs.

The versioned package's binary, launcher, manifest and containing directories are
read-only. `src/update.rs` stages alongside `current_exe` then atomically renames
onto it; the unprivileged updater cannot write the package's `libexec` directory.
This also protects the raw executable from an update initiated inside the TUI.
The launcher checks hashes again at every invocation. Version/manifest background
checks are disabled in the fresh config. Do not relax directory permissions, use
sudo to update this package, or point Homebrew/Nix/mise or the upstream installer
at its path. No package-manager-owned directory or global `herdr` link is changed.
Packaging is staging only; any enduring workstation configuration belongs in
Bahamut under rule G5 when the parent prepares approved delivery.

Source contracts: `src/config/io.rs` (`config_dir`, `state_dir`, `config_path`),
`src/session.rs` (`configure_from_args`, `data_dir_for`, `active_api_socket_path`),
`src/server/socket_paths.rs` (explicit-session precedence), `src/server/handoff.rs`
(inherited routing for importer), `src/update.rs` (`current_exe` replacement),
`src/build_info.rs` (supported downstream build identity).

## Parent-owned remote lane

The assigned child is `68a79a8b-13e1-4620-8e10-ab3cea96ade3`,
`herdr-fast-api-20261011`. Do not create or wake another machine. The outer guest
is Ubuntu; the retained Arch root is `/home/boxd/w11y-env/portal`. Its helper
`tools/portal-builder-environment.py` reports recipe digest
`0c08c29cf5b47e3140bb5ef154df888faf26991cf1bf327adec72c529a55b11d`.
All compile/check/rehearsal commands run through that helper's `exec-plan-step`,
inside the pinned Arch container, with reports under `jobs/herdr-fast-api/`.
The child has eight CPUs and 32 GiB RAM. Cargo's total build-job limit is eight;
benchmark work waits until compilation finishes and uses the parent's separate
allocation decisions. The existing copied PostgreSQL service and seed's homes
are not test targets.

`remote_baseline.sh` installs Rust 1.96.1 with clippy/rustfmt in private tool homes,
Zig 0.16.0 verified against the official download index checksum, and private
just/nextest/Bun runners. Both CI and `vendor/libghostty-vt/build.zig.zon` require
Zig 0.16.0. There is no conflicting version at the pinned source. It builds the
pinned source with `just build`, preserves a read-only copy of the installed
published binary, and records same-toolchain baseline, lockfile/tool hashes and
logs. The installed binary is copied as a file only; no local execution, socket
access or update is involved. Installation runner versions are recorded; future
runs should reuse that record rather than silently adopting newer just/nextest.

After setup, use the same helper prefix, with a unique report for each invocation:

```sh
R=/home/boxd/w11y-env/portal
python3 "$R/tools/portal-builder-environment.py" --project portal --variant arch-target \
  --root "$R" --expect-digest 0c08c29cf5b47e3140bb5ef154df888faf26991cf1bf327adec72c529a55b11d \
  exec-plan-step --cwd source/herdr-fast-api-candidate \
  --report "$R/jobs/herdr-fast-api/candidate-check.json" -- \
  bash scripts/fast_api/remote_candidate.sh check
```

The parent uploads the exact integrated source archive to
`source/herdr-fast-api-candidate`, records the source commit in
`hf/artifacts/candidate-source.txt`, and keeps distinct Cargo target directories.
The `check` mode runs repository `just check` and the new packaging regressions.
Windows SDK setup is separate and requires acceptance of Microsoft's license;
checks never download it. If the parent elects Linux-only `just ci` plus
`just docs-contract-test`, record Windows lint as unrun. Do not silently label
that narrower run `just check`. The `build` and `package` modes run `just build`
and stage a versioned read-only package. No upstream release recipe is invoked.

## 174-pane old-to-candidate rehearsal

Run only remotely, after builds finish, using release binaries. The rehearsal
uses `workspace.create`, one pane per tab, `pane.send_input`, `pane.list`, and
`server.live_handoff` with `import_exe`, following `tests/live_handoff.rs`. Every
run needs a new short root. It creates 174 actual PTYs and minimal Python input
consumers with no native identities. Readiness uses filesystem events, not a
sleep or a latency threshold. There is no automatic test timeout; cancellation
belongs to the parent and its own helper container.

```sh
# Inside the same Arch container; R is /home/boxd/w11y-env/portal.
python3 scripts/fast_api/rehearse.py \
  --old "$R/hf/artifacts/published-herdr" \
  --candidate "$R/hf/target-candidate/release/herdr" \
  --root "$R/hf/rh-ok"
# Repeat with fresh rh-protocol, rh-spawn, rh-restored roots and respectively:
# --failure protocol | --failure missing-executable | --failure after-restored
```

Success verifies all 174 canonical pane/workspace/tab/terminal identities, child
PIDs plus process start ticks, no HUP, six ordered input records per child across
the transition, candidate ownership of the public socket, and fresh API connections and generation-1 client handshakes at 483×128. Failure cases verify the source retains ownership and every
child continues consuming ordered input. The existing importer failure injection
`HERDR_TEST_HANDOFF_IMPORT_FAIL=after_restored` exercises rollback after PTY
restoration but before commit. Result JSON, child markers and logs stay in the
owned run root. No successful claim is made until the parent inspects them.

The large Python rehearsal negotiates the generation-1 client endpoint before
and after handoff. It does not drive a full TUI. Run existing `just test-one live_handoff` remotely as the
complementary client bootstrap, keyboard protocol, modified keys, named routing,
unmanaged identity, process death and rollback coverage. Those tests currently
cover client input separately and a 70-pane descriptor-transfer case; they do not
prove full TUI reconnect/input at 174 panes. The parent should report that boundary
and have its harness owner extend end-to-end client input if required.

A fifth fresh-root run with `--failure post-commit` first verifies successful
migration and ordered input, then kills only the socket peer whose executable
matches the assigned candidate and whose start ticks were checked. Its pidfd
wait records candidate exit without a delay. `post-commit.json` reports child
survival/HUP at observation time and whether the source exited. It deliberately
does not invent a recovery guarantee or relaunch native consumers. The result's
"passed" means the destructive scenario executed after migration checks; it does
not mean recovery passed. Binary rollback cannot restore lost PTYs or native
conversations. Supported handoff disconnects clients and may interrupt requests;
post-commit recovery is not guaranteed by the upstream source.

## Updating this downstream series

Keep upstream fetch-only and push disabled; the parent owns fork publishing.
Never run upstream `herdr update`, preview, release or install scripts against
this package. A selected stable upgrade gets a new branch from its exact tag,
replays the small patch series, compares protocol/handoff contracts and immutable
endpoint fixtures, removes upstreamed equivalents, and repeats freshness,
notification, identity, focus, input, recovery and performance checks. Preserve
previous source commits and versioned artifacts with their hashes. Stage a new
package directory and deliberately choose its launcher; never replace the old
versioned directory in place. Native conversation preservation requires supported
consistent provider transcript backups and verified identities under separate
live-change approval; a saved layout is insufficient.

Authored validation commands are not evidence of execution. This worker runs no
local builds, tests or benchmarks. Remote receipts distinguish setup, compilation,
checks, rehearsal and measured performance.
