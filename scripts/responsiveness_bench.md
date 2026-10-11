# Isolated responsiveness benchmark

This is an opt-in **remote-only** harness for prebuilt Linux release artifacts.
It never builds, downloads, looks up an installed Herdr executable, connects to
an existing session, launches a provider CLI, or installs dependencies. Run it
only on the parent-assigned isolated allocation. Do not run it on the workstation
or the machine hosting the user's live Herdr session.

The source reference is upstream v0.9.3,
`7b116c05bfda646af39d2524c54e70c751f57ee8`. The input source revision is recorded
as caller-supplied provenance, alongside the actual binary SHA-256 and version;
the revision label alone does not prove which source produced the artifact.

## Source-backed purpose

- `src/api/subscriptions.rs::ActiveAgentStatusChangedSubscription::poll_snapshot`
  uses public `pane.get` when no matching history event is available. Quiet
  subscribers therefore need separate variation from total retained panes.
- `src/api/server.rs::stream_subscriptions` polls subscriptions serially and
  retains the existing 100 ms connection polling cadence. The harness does not
  change it or pretend that subscriber count implies a fixed request rate.
- `src/server/headless.rs::handle_api_request_with_shutdown_check_inner` scans
  retained pane state and synchronizes the foreground client around ordinary
  requests. Its cost depends on total panes and competing client work.
- `src/client/frame_output.rs` preserves synchronized host-output markers. The
  cell observer honors private terminal mode 2026 and publishes the cells only
  after the matching end marker, including when that marker spans reads.
- The existing `just bench-release-smoke` compares CPU under `hidden50` and
  `visible30`. `just bench-render-scale` exercises fixed geometry at several
  render cardinalities. These remain useful supporting profiles. Neither
  measures input-to-echo. This harness reuses their named-session isolation,
  populated background tabs, and monotonic paced-output approach.

No product source, public methods, codecs, freshness rules, notification logic,
input routing, identity handling, or native conversation behavior is changed.

## What is measured

The stand-in is a Python process running in each real Herdr PTY. It disables
kernel terminal echo, accepts numbered input packets, and emits a different
numbered echo marker only after reading the packet. An API success response is
not an echo measurement. Input timestamps are taken before API dispatch or the
write to the TUI's outer PTY. Each packet's send completion is also retained.

- `direct` sends through `pane.send_input` and observes `pane.read` from the
  visible viewport. Its latency includes the read's cost and polling interval.
- `tui` writes actual terminal input to a real Herdr TUI in an outer PTY. An
  independent reader applies TUI output to a `pyte` terminal screen and observes
  echoed cells after synchronized updates commit. There is no steady-state
  direct read observer in this mode.
- `mixed` alternates the two input paths to the same focused pane and observes
  both the TUI cells and direct reads. Cross-client input order is reported as
  evidence, without assuming an atomic ordering guarantee across connections.

The input sample is followed by a scroll phase. A fresh numbered history footer
proves that scroll preparation reached the stand-in and the observer. Direct
scrolls use `pane.scroll`; TUI scrolls use SGR mouse-wheel input at coordinates
derived from visible history cells in that pane. A result requires numbered
history to move in the requested direction, not merely a repaint. Foreground
output pauses during this phase; hidden producers, status subscribers, API
bursts, and additional clients continue. Each scroll has the same explicitly
configured observation window as `--seconds`; an unobserved scroll is retained
and ends the scroll phase, rather than being assigned a made-up latency.

Each case reports p50/p95/p99/max and observation counts, input scheduling
lateness, unobserved samples, child consumption order/duplicates/malformed
packets, subscriber events/errors, achieved API burst counts, CPU/RSS/runqueue
wait for server/TUI/harness separately, and TUI host output bytes/read chunks/
cell changes. Child ledger entries survive datagram receipt loss. A missing
ledger entry can still represent in-flight input at cleanup, so it is called
`not_consumed_by_cleanup`, not automatically called dropped input.

TUI cell observation includes the real server-to-TUI-to-host-output path and the
simulated outer terminal parser. It does **not** measure physical terminal/GPU
presentation. Host output chunks and cell changes are **not** server protocol
frame counts. `/proc` runqueue wait is **not** application queue age or an
attribution of blocked PTY time. This patch does not add product instrumentation
for those internal seams. No fixed latency acceptance threshold is imposed.

## Safety and cleanup

Every case creates a unique, private `/var/tmp/hdl-*` state root and a generated
`latency-*` session name. It supplies an allowlisted environment with fresh HOME,
XDG config/data/state/cache/runtime directories and a non-login stand-in shell.
No provider credentials, SSH agent socket, live Herdr variables, user config,
conversation references, or user shell initialization is inherited. Update and
manifest checks and external toast delivery are disabled in this private config.

The harness starts the specified server in the foreground, creates workspace/
tab/pane IDs only through responses, checks cardinality, and verifies every JSON
socket's Linux peer PID and UID against that child. It does not start the TUI
until the owned server and stand-ins are ready. At 174 panes, 121 stand-ins use
`pane.report_agent` with an isolated `bench` source and a recognized `codex`
label. These are synthetic agent equivalents; no Codex CLI or native session
ID is created. At 1/15 panes all stand-ins are reported. Subscriptions above
pane count deliberately repeat pane IDs; this independently exercises the
subscription cardinality and is identified by the case parameters.

SIGINT/SIGTERM cancels the harness and closes only its recorded connections and
TUI children. Cleanup signals only the foreground server PID it launched,
waits for it, copies child ledgers, checks exact ownership marker/inode and any
remaining process with the exact private root environment marker, then removes
only that root. Remaining owned processes cause state to be retained and are
listed in the report. There is no `pkill`, session discovery/stop/delete,
live-server restart, or arbitrary process matching for termination.

`isolation.json` is written before workload setup and records the owned PID,
start tick, session, socket, state root and environment. `server.log` and all
measurement output survive cleanup. A reused output directory is refused so
previous evidence cannot be overwritten. Readiness has no arbitrary job timeout;
cancel the owned job if source inspection shows it cannot become ready. The
parent should use `codex-job cancel` for an authorized cancellation. Never rerun
an ambiguously submitted load; inspect its job receipt first.

## Parent procedure on the authorized remote lane

The parent supplies the allocation hostname, CPU set, existing checkout,
immutable release artifacts, source revisions, Python environment and durable
remote result root. Python 3.10+ is required. TUI cell observation additionally
requires `pyte==0.8.2` in that isolated environment; the script never installs
it. `--raw-drain` avoids that dependency only for an observer-overhead control,
and intentionally produces no TUI cell-latency claim. The deterministic suite
skips the cell-observer test when pyte is unavailable; run it with pyte installed
for full coverage. Release builds are required because the pinned source uses
the `herdr` config directory for release and `herdr-dev` for debug binaries.
The harness contracts are also included in `just maintenance-test`; Linux-only
contracts skip on other platforms.

The following shell is for the **remote lane only**. Values come from the parent;
unset values fail before load. Keep compilation and other benchmarks off these
CPUs during measurement. Do not reserve new infrastructure from this procedure.

```bash
test "$(hostname)" = "${AUTHORIZED_BENCH_HOST:?parent supplies assigned hostname}"
cd "${REMOTE_CHECKOUT:?parent supplies isolated checkout}"
export PATH="${BENCH_PYTHON_BIN:?parent supplies prepared Python bin directory}:$PATH"
test -x "${BASELINE_BIN:?published v0.9.3 artifact}"
test -x "${SAME_TOOLCHAIN_BASELINE_BIN:?same-toolchain build of the pinned source}"
test -x "${CANDIDATE_BIN:?parent supplies herdr-fast-api release artifact}"
test -d "${REMOTE_RESULTS:?parent supplies durable remote evidence directory}"
test -n "${BENCH_CPUS:?parent supplies isolated CPU set}"
test -n "${CANDIDATE_REV:?parent supplies integrated candidate source revision}"

just test-responsiveness-harness
just bench-responsiveness matrix > "$REMOTE_RESULTS/matrix.jsonl"
sha256sum "$BASELINE_BIN" "$SAME_TOOLCHAIN_BASELINE_BIN" "$CANDIDATE_BIN" Cargo.lock \
  > "$REMOTE_RESULTS/artifact-hashes.txt"
rustc -Vv > "$REMOTE_RESULTS/rustc.txt"
cargo -V > "$REMOTE_RESULTS/cargo.txt"
git rev-parse HEAD > "$REMOTE_RESULTS/harness-source.txt"
```

First qualify setup, identity, delivery and cleanup on a reviewed small case;
this is a functional qualification, not performance evidence:

```bash
taskset -c "$BENCH_CPUS" just bench-responsiveness run --remote-isolated \
  --binary "$BASELINE_BIN" --variant published-baseline --source-revision \
  7b116c05bfda646af39d2524c54e70c751f57ee8 --round 1 \
  --output "$REMOTE_RESULTS/qualification" --panes 1 --subscriptions 1 \
  --connections 1 --mode mixed --cols 120 --rows 40 --workload idle \
  --seconds 5 --input-hz 5 --read-hz 20 --output-hz 30 --scroll-samples 2
```

Review `report.json.errors`, requested/recognized pane counts, child packet
consumption, visible echo observations, directional scroll observations and
`cleanup.owned_survivors`. Preserve all outcomes; qualify any missing evidence
instead of turning it into a performance success. No workload has been executed
by the author of this patch.

Then compare equal scenarios in crossed order with 60-second input samples.
The example rates are declared synthetic workload rates, not measured Portal
traffic or product tuning. Replay observed caller mixes separately once known.

```bash
for round in 1 2; do
  if test "$round" = 1; then
    variants='same-toolchain-baseline candidate'
  else
    variants='candidate same-toolchain-baseline'
  fi
  for variant in $variants; do
    if test "$variant" = candidate; then
      artifact="$CANDIDATE_BIN"
      revision="$CANDIDATE_REV"
    else
      artifact="$SAME_TOOLCHAIN_BASELINE_BIN"
      revision=7b116c05bfda646af39d2524c54e70c751f57ee8
    fi
    for panes in 1 15 174; do
      for subscriptions in 0 15 121; do
        taskset -c "$BENCH_CPUS" just bench-responsiveness run --remote-isolated \
          --binary "$artifact" --variant "$variant" --source-revision "$revision" \
          --round "$round" --output "$REMOTE_RESULTS/$variant-r$round-p$panes-s$subscriptions-mixed-idle" \
          --panes "$panes" --subscriptions "$subscriptions" --connections 1 \
          --mode mixed --cols 120 --rows 40 --workload idle --seconds 60 \
          --input-hz 5 --read-hz 20 --output-hz 30 --scroll-samples 20
      done
    done
  done
done
```

Run published baseline in the same cells to distinguish rebuild/toolchain effects
from patch effects. Extend the reviewed cells using the 162-case `matrix` output:
direct/TUI/mixed independently; 120x40 and 483x128; idle/hidden/visible output.
`hidden` has `panes - 1` producers, so the one-pane cell intentionally has no
hidden producer. Add the following axes **separately** to equivalent cells:

- `--connections 3` distributes the same total subscriber count over three
  independent sockets instead of one.
- `--api-burst-clients 4` adds four independent FIFO connections continuously
  cycling `pane.get`, `agent.list`, and `workspace.list`. Report achieved counts;
  this does not prescribe or assume a requests-per-second rate.
- `--slow-reader-hz 1` attaches one additional raw-drained TUI that reads one
  chunk per second. This is a controlled slow-reader workload, not a new server
  cadence. Adding it to `direct` creates a case with an extra presentation client.

Keep stable ordering and repeat crossed placement on equivalent allocations
when available. Do not run competing variants simultaneously on the same CPUs.
The parent can independently parallelize deterministic checks within its assigned
resource budget. It owns remote dispatch and collection; this worker launches
no jobs or additional agents.

## Reuse existing supporting profiles

On the separately allocated remote verification resources:

```bash
just bench-render-scale > "$REMOTE_RESULTS/render-scale.txt" 2>&1
just bench-api-fairness > "$REMOTE_RESULTS/api-fairness.txt" 2>&1
```

These recipes compile tests, so finish them before reserving CPUs for latency
measurement. For the existing release CPU smoke, pass the explicit baseline to
avoid its stable-download fallback. Run its script with the already-built artifact
to avoid rebuilding during measurement:

```bash
HERDR_PERF_BASELINE_BIN="$BASELINE_BIN" HERDR_PERF_SAMPLE_SECONDS=60 \
  scripts/release_perf_smoke.sh "$CANDIDATE_BIN" \
  > "$REMOTE_RESULTS/release-cpu-smoke.txt" 2>&1
```

The existing CPU smoke deletes its temporary raw logs. Its retained console
comparison is supporting evidence only; the responsiveness harness preserves
its own raw measurements. The existing CPU smoke's CPU regression threshold is
unchanged and must not be presented as an input-latency acceptance threshold.

## Observer/instrumentation overhead

No internal product counters were added. Measure harness overhead on identical
binary/workload/geometry/CPU placement with crossed repetitions:

1. Normal run versus `--no-audit`, with `--scroll-samples 0`, measures the child
   JSON ledger and datagram receipt cost. The no-audit result has no child
   consumption claim; public echo observations remain enabled.
2. Normal TUI run versus `--raw-drain --scroll-samples 0` measures terminal-cell
   emulation/scan overhead. Both drain host output; the raw run has no TUI
   cell-latency result. Compare server/TUI/harness CPU separately and byte volume,
   not a fake raw-drain visibility percentile.
3. Repeat with `--no-resource-sampling` to measure the `/proc` sampler's cost.
4. For direct/mixed, vary `--read-hz 10`, `20`, and `40` independently. Report
   observer API durations, achieved intervals, CPU and missing samples. The
   configured periods alone do not describe the interval under contention.

Input/scroll/read rates are explicit workload parameters. No new product delays,
allocator, queue budgets, or scheduling changes are proposed from these controls.
If the parent adds internal timing counters later, repeat equivalent builds with
those counters disabled/enabled and retain their separate measured overhead.

## Results and review

Each case preserves `report.json`, `isolation.json`, `server.log`, `input.jsonl`,
`child-input.jsonl`, `scroll.jsonl`, `resources.jsonl`, and `observer-api.jsonl`.
`compare` groups only equal scenario parameters and keeps each run's distribution,
hash, errors and cleanup outcome; it does not average percentiles or discard
censored samples:

```bash
just bench-responsiveness compare "$REMOTE_RESULTS"/*/report.json \
  > "$REMOTE_RESULTS/comparison.jsonl"
```

Report the paired distributions, actual sample/consumption counts, application
and observer CPU separately, memory and delivery volume, input schedule lateness,
ordering/duplicate/malformed evidence, subscriber errors and cleanup. For internal
queue age, PTY blocking, request-origin mix, true protocol-frame counts and native
conversation preservation, say unmeasured unless the parent supplies independent
evidence. A measured optimization, incident reproduction, remote test pass, and
installed user improvement remain separate claims.

Collect the remote evidence under the parent's owned local directory
`/home/programmer/.local/share/agent-work/herdr-fast-api-20261011` without committing
logs or results. The parent owns the full applicable remote `just check`, product
regression filters, artifact provenance, integration and final isolated executable.
