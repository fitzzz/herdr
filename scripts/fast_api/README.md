# Isolated downstream fast API candidate

This Linux-only package supplements upstream v0.9.3 at
`7b116c05bfda646af39d2524c54e70c751f57ee8`. It does not install over `herdr`,
attach to the user's server, import existing sessions, or migrate conversations.
The alternate candidate is ready for isolated validation. It is **not safe for
live in-place migration**. Replacing or restarting the live server remains
unauthorized and needs a separate decision from Fitz.

The compiled release selected for final staging is
`12cc5dd1d6775cd202b0efaf9abe276022522521`, version
`herdr 0.9.3-fast-api.12cc5dd1d677-readiness`, SHA256
`b26a94a15b9a109a0a919848f4cef44088edfe81a2f7d25358b0eb743fe92640`.
Stage this existing artifact; do not rebuild it or substitute a moving target
directory. Package/launcher checks for this final artifact remain pending until
the parent transfers the remote lane. Previous checks used product `512e8e29` and do not
establish a result for this package.

The package manifest's `source_commit` identifies the compiled product. Record
the separately reviewed packaging source, source-archive hash, launcher hash
and retained tool provenance beside the package. A documentation or launcher
revision must not be represented as a rebuilt product.

## Namespace and update protection

The packaged `bin/herdr-fast-api` launcher verifies the artifact hash, removes
inherited Herdr routing and pane identities, removes all provider-home selectors
recognized by `src/integration/env.rs` and platform home fallbacks, clears
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
Use the private NSS helper recorded by the current lane handback, with its
verified private passwd mapping; the original helper alone is unsuitable. The
retained helper is
`hf/diagnostics/causal-r4-input/private-nss-r4.py`, SHA256
`41ff0646037a2146dc465b861d6744fb168c57db4edb36e63649466702a71ab5`.
All compile/check/rehearsal commands run through its `exec-plan-step`,
inside the pinned Arch container, with reports under `jobs/herdr-fast-api/`.
The child has eight CPUs and 32 GiB RAM. Cargo's total build-job limit is eight;
benchmark work waits until compilation finishes and uses the parent's separate
allocation decisions. The existing copied PostgreSQL service and seed's homes
are not test targets.

`remote_baseline.sh` installs Rust 1.96.1 with clippy/rustfmt in private tool homes,
Zig 0.16.0 verified against the official download index checksum, and private
just 1.58.0, cargo-nextest 0.9.148 and Bun 1.3.14 runners, matching the retained
lane receipts. Both CI and `vendor/libghostty-vt/build.zig.zon` require
Zig 0.16.0. There is no conflicting version at the pinned source. It builds the
pinned source with `just build`, preserves a read-only copy of the installed
published binary, and records same-toolchain baseline, lockfile/tool hashes and
logs. The installed binary is copied as a file only; no local execution, socket
access or update is involved. Exact version checks apply even to preexisting
tools. Cargo runner installations use pinned `--version` and `--locked`; rustup
1.29.1 installs the pinned toolchain with self-update disabled. The versioned
rustup installer replaces the mutable shell bootstrap and is checked against
its [official SHA256](https://static.rust-lang.org/rustup/archive/1.29.1/x86_64-unknown-linux-gnu/rustup-init.sha256).
Bun's archive is checked against the digest of the
[official release asset](https://api.github.com/repos/oven-sh/bun/releases/assets/418773972),
including when a retained archive is reused. Installed tool hashes are recorded.
These checks do not retroactively establish the integrity of the original
unchecked installer download; its recorded tool versions remain separate evidence.

The pinned Arch image lacks Node and OpenSSH. `remote_dependencies.sh` adds real
Node 22.13.1 from its official checksum-verified archive and signed Arch OpenSSH
in a disposable root container of the same image. Bind only `hf`, use image ID
`sha256:34aa4196ec9557924a70bd3b0fbf72700f868d08609aa83d335066c070ab63ff`,
and invoke the uploaded script with Bash. Installation changes only that
container and private `hf` tools; it does not retag or modify the helper image.
OpenSSH's actual executable, loader and linked libraries are copied together;
its private wrapper selects those libraries without changing Herdr's loader
environment. Candidate PATH includes both private tools. The setup preserves
download checksums, package versions, linked libraries and file hashes in
`hf/logs/dependencies/`; version probes are its only executable checks. It never
runs tests or compilation. Node is a separate runtime, not a Bun alias.

Setup/build commands below describe the existing lane. Final candidate staging
reuses its verified tools and prebuilt release, with no reinstall or compilation.
The parent must transfer exclusive ownership before any remote command. Use the
same private helper prefix, with a unique report for each invocation:

```sh
R=/home/boxd/w11y-env/portal
python3 "$R/hf/diagnostics/causal-r4-input/private-nss-r4.py" --project portal --variant arch-target \
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
checks never download it. The `linux-check` mode runs the same Linux CI suite
with nextest `--no-fail-fast --test-threads 8`. It renders the repository's lint,
maintenance, hotpath, integration-assets and docs recipes, runs each command
even after failures, and runs the packaging regressions. It returns failure if
any command fails. Record Windows lint as unrun; do not label this narrower run
`just check`. Cargo compilation also uses eight jobs. The `build` and `package`
modes run `just build`
and stage a versioned read-only package. No upstream release recipe is invoked.

Focused remote packaging proof (after the parent hands over the exclusive lane):

```sh
python3 -m unittest scripts.fast_api.test_packaging scripts.fast_api.test_rehearsal
HERDR_FAST_API_TEST_BINARY="$R/hf/artifacts/herdr-fast-api-readiness-12cc5dd1d677/herdr" \
  python3 -m unittest scripts.fast_api.test_launcher_integration
```

The second command requires an explicit prebuilt downstream artifact; missing
input is an error. It stages a disposable package, seeds synthetic external
provider directories, and installs Pi's integration through the private launcher.
It verifies the private extension was written and every external directory and
sentinel remained unchanged. No real provider is launched or configured. The
receipt contract synchronizes an append after the waiter starts, flushes while
the writer remains open, and checks the queued filesystem event without a delay.

## Handoff limitations and retained rehearsal

The 174-pane synthetic handoff failed the strict terminal-ID assertion after
handoff. A two-pane diagnostic confirmed that pane/tab/workspace addresses and
child processes continued, while terminal IDs changed. The assertion and full
identity/output diagnostics remain in `rehearse.py`; nothing is relaxed to make
that result pass. The compiled `12cc5dd1` readiness change does not repair this seam.

Portal can use a terminal ID to recognize a running child when it has no native
session identity. A changed ID can make that child appear gone even though its
process is still running. Handoff also resets the state/completion counters that
Portal uses to decide whether a turn finished. Preserving an ID alone would not
preserve those decisions. The old export does not include the needed counters,
and a read taken before handoff cannot prove they stayed unchanged through the
freeze. No migration source is being implemented to claim otherwise.

See [HANDOFF.md](HANDOFF.md) for retained evidence, unchanged diagnostic commands
and the limits of rollback. Those commands are diagnostic contracts, not a live
transition procedure. Final isolated package staging does not rerun handoff,
the 100-pair startup measurement or the 56-run benchmark matrix. Native-chat
continuity and the installed Portal's reconnection behavior remain unqualified.

## Updating this downstream series

Keep upstream fetch-only and push disabled; the parent owns fork publishing.
Never run upstream `herdr update`, preview, release or install scripts against
this package. A selected stable upgrade gets a new branch from its exact tag,
replays the small patch series, compares protocol/handoff contracts and immutable
endpoint fixtures, removes upstreamed equivalents, and repeats freshness,
notification, identity, focus, input, recovery and performance checks. Preserve
previous source commits and versioned artifacts with their hashes. Stage a new
package directory and deliberately choose its launcher; never replace the old
versioned directory in place. Backups or saved layouts do not establish safe
native-conversation migration. Any future live transition needs a separate
user decision and an approved procedure addressing terminal identity, completion
state, input delivery and consumer reconnection.

Authored validation commands are not evidence of execution. This worker runs no
local builds, tests or benchmarks. Remote receipts distinguish setup, compilation,
checks, rehearsal and measured performance.
