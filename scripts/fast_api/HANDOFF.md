# Handoff status and diagnostic contract

The separate herdr-fast-api candidate is intended for isolated validation.
It is not qualified for live in-place migration. No live replacement, restart,
session import or native-chat consumer is authorized by these scripts or docs.
The parent has declined migration source implementation: the unchanged old
exporter cannot prove that observations captured before handoff remained current
until its state froze. A future live transition procedure needs a separate user
decision.

## What remains failed or unqualified

The published v0.9.3 -> product `512e8e29` 174-pane rehearsal failed the strict
`(pane_id, tab_id, workspace_id, terminal_id)` comparison after a successful
handoff. It did not pass recovery acceptance. A later two-pane diagnostic found
only terminal IDs changed; public addresses and child PID/start ticks remained
the same, and all six ordered input records per child and their unique terminal
output markers were observed. That smaller result is not a successful 174-pane result
and does not represent real native chats. Diagnostics commit
`f804ba1f016780cadecca93dd08dbe1f1eaae399` keeps complete before/after maps and
partial output evidence without weakening the strict assertion.

Terminal IDs are the keys consumers can keep for a running terminal. Portal's
repository child reconciliation uses that key when no native session identity
is known, so retaining just a pane address can still make an existing child
appear ended. Preserving exact terminal IDs alone would not solve completion:
handoff resets the state/completion counters used to compare a new completion
with a previously submitted turn. An outstanding hold can remain unresolved,
and copying a stale pre-handoff observation can make an old check appear current
again. The old frozen export carries neither those public counters nor their
generator. Input inactivity or a fixed delay does not prove no state transition
occurred between capture and freeze.

The prebuilt readiness release is source
`12cc5dd1d6775cd202b0efaf9abe276022522521`, version
`herdr 0.9.3-fast-api.12cc5dd1d677-readiness`, SHA256
`b26a94a15b9a109a0a919848f4cef44088edfe81a2f7d25358b0eb743fe92640`.
Its readiness correction does not preserve handoff terminal IDs or completion
watermarks. It has not been packaged or rehearsed as the final isolated package
at this documentation boundary. Record its compiled source separately from the
final packaging/launcher source and retain earlier immutable packages.

Evidence root:
`/home/programmer/.local/share/agent-work/herdr-fast-api-20261011/`.

| Retained evidence | Result / boundary |
| --- | --- |
| `packaging-proof-94a0fed62c5e/` | Product `512e8e29`: 13 focused contracts and synthetic real-binary Pi install proof passed; package and 100-pair startup evidence retained; strict 174-pane identity failed. |
| `packaging-identity-94a0-two/` and `PACKAGING-HANDOFF-IDENTITY-DIAGNOSIS.md` | Two synthetic panes: terminal IDs changed; durable public/process identity and input/output observations retained; strict identity still failed. |
| `PORTAL-HANDOFF-CONSUMER-IMPLICATIONS.md` | Portal repository `0ddded7` source analysis, not verification of the installed/running Portal artifact. |
| `wrapper-01a12d3c-COMPLETION-CONTINUITY-SEAM.md` | Source-only completion analysis; capture cannot establish capture-to-freeze continuity with the old export. |
| `WRAPPER-READINESS-LANE-HANDBACK.json` | Exact compiled `12cc5dd1` binary paths/hash and focused readiness results; no final package, 174-pane or live replacement result. |

Full TUI reconnect/input at 174 panes, installed Portal behavior and real native-chat
continuity remain unqualified. The old public export does not provide the
missing guarantees. No receipt-based preserving migration, client identity
relaxation, manual agent cleanup or claimed automatic recovery is offered here.

## Retained synthetic commands, not final package staging

Only run these on an exclusively assigned remote lab after separate parent
instruction. They do not run locally or against a live/user session. Use the
private NSS Arch helper from the lane receipt, fresh short roots and immutable
explicit binary paths. These examples require the caller to set `OLD`,
`CANDIDATE` and `RUN_ROOT` to approved lab artifacts and a never-used root;
they do not choose a server or artifact from inherited routing.

```sh
python3 scripts/fast_api/rehearse.py \
  --old "$OLD" --candidate "$CANDIDATE" --root "$RUN_ROOT"
# Separate new roots for: --failure protocol | missing-executable | after-restored
```

Acceptance keeps all 174 public and terminal identities, child PID/start ticks,
no HUP, exact six input records per child, unique `got:` terminal output for
each pane, candidate socket ownership, and fresh API/generation1 connections
at 483x128. Missing output is checked through supported pane.wait_for_output and
pane.read, never inferred from receipt input. Current products fail terminal
identity and are not expected to pass this whole acceptance. No assertion is
dropped or reclassified as passing.

Each expected output marker must occur exactly once and in order in its own
pane; input echo or another pane cannot satisfy it. The harness retains partial
terminal-output.json and complete before/after identity maps. Readiness follows
filesystem events, not a delay or wall-clock threshold; there is no automatic
test deadline, and parent cancellation targets only the owned test. Precommit
failure controls require source socket ownership and continuing child input.
`HERDR_TEST_HANDOFF_IMPORT_FAIL=after_restored` specifically exercises rejection
after PTY restoration and before commit; its result does not prove postcommit
recovery.

The retained missing-output negative uses another fresh root:

```sh
python3 scripts/fast_api/rehearse.py \
  --old "$OLD" --candidate "$CANDIDATE" --root "$RUN_ROOT" \
  --suppress-output-pane 0
```

The specific negative is `missing or duplicated output 'got:after-0-0'` after
the input ledger succeeds. The synthetic child emits the final output barrier
but omits that marker. An earlier strict identity failure is not a passing
missing-output negative. The focused Python contract separately covers the
synthetic child's complete receipts and incomplete output. Do not restart a
large campaign to rediscover the already established handoff limitation.

Existing `just test-one live_handoff` covers bootstrap/input, keyboard protocol,
named routing, process death and rollback separately, including a 70-pane
descriptor case. Those contracts do not prove full TUI input/reconnect at 174 panes
or native conversations. They are not part of the final packaging-only lane.

## Failure and recovery limits

The old server disconnects clients before importer validation. A precommit
rejection can preserve its processes/readers/socket ownership while their
attached windows have already disconnected and queued input has been discarded.
After `restored`, public socket paths may have been removed; rebinding can fail.
After ownership commits, a killed candidate has no general rollback guarantee.
An API timeout or lost response can also leave a write's outcome unknown;
repeating it can deliver input twice.

The harness retains a separate `--failure post-commit` diagnostic that kills
only the checked candidate and observes child survival/HUP. It is not approved
for the final packaging lane or a live session. Its "passed" field describes
execution of that diagnostic after migration checks, not recovered PTYs or
conversations. Binary rollback and transcript backups cannot by themselves
restore lost PTYs, preserve delivery or reconnect every consumer safely.

Any future live transition needs explicit approval and a separately qualified
procedure for identities, completion holds, in-flight input/nonidempotent
operations, notifications and reattachment. The isolated launcher continues to
use private HOME/XDG/session/socket/provider paths and never attaches to the
existing server automatically.
