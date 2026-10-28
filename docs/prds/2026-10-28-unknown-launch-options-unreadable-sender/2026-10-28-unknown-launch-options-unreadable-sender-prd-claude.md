# PRD: Leave launch options unknown when the report sender is unreadable

Portal task T-4, project herdr-fast-api. Interview run `6e94e556-7a42-49fa-9fec-da899bf52391`, 2026-10-28.
Follow-up to T-2 (keep Claude and Codex launch options across restore), whose PRD is
`docs/prds/2026-10-19-restore-agent-launch-options/2026-10-19-restore-agent-launch-options-prd-claude.md`
(called "the T-2 PRD" below).

## Implementation Log

| Field | Value |
| --- | --- |
| Started | 2026-10-28 07:52:10 EDT |
| Agent | opencode |
| Model | deepseek/deepseek-v4.1-flash-1017 |
| Reasoning Level | unknown |

### Verification Evidence

Paths starting with `w1/`, `w2/` or `checks-` are in
`~/.local/share/agent-work/herdr-fast-api-t4/` (outside the repository).

| ID | Implementation | Automated Tests | Interaction Run |
| --- | --- | --- | --- |
| FR-1 | src/app/api/panes.rs (`session_report_evidence` returns `Unidentified` without a peer), src/terminal/state.rs (`SessionReportEvidence` has no foreground-job variant) | app::actions::tests::a_conversation_reported_with_no_readable_sender_saves_no_options; terminal::state::tests::a_report_with_no_readable_sender_beside_one_agent_saves_no_options; terminal::state::tests::session_for_process_decides_every_case; terminal::state::tests::exhaustive_pairing::exhaustive_pairing_saves_options_only_beside_their_process_conversation | Yes
| FR-2 | src/terminal/state.rs (`sync_agent_launch_session` never confirms `LaunchState::Started`) | terminal::state::tests::an_unbound_seed_is_not_confirmed_by_a_report_with_no_readable_sender (restore seed and `agent start` seed, then a readable report pairs again); terminal::state::tests::a_report_with_no_readable_sender_beside_one_agent_saves_no_options | Yes
| FR-3 | src/terminal/state.rs (`session_for_process` has no `reported` fallback; `reported_session` and `persisted_agent_session_reported` are gone) | terminal::state::tests::a_first_observed_process_does_not_take_a_conversation_a_state_report_set (three orders); terminal::state::tests::exhaustive_pairing::exhaustive_pairing_saves_options_only_beside_their_process_conversation (state reports in the alphabet) | N/A: the difference appears only in the saved session file, and the order of a state report and the first detection read cannot be controlled in a real agent; the FR-4 live case drives the same report path. |
| FR-4 | src/terminal/state.rs, src/app/actions.rs | app::actions::tests::a_conversation_reported_with_no_readable_sender_saves_no_options asserts the saved entry has no options and `plan_with_launch_options` answers `NotKept(NotRecorded)` | Yes
| FR-5 | src/app/api/panes.rs (a readable peer still yields `Reporter`) | app::actions::tests::another_process_reporting_the_paired_conversation_takes_it_until_the_agent_reports_it; app::actions::tests::a_rejected_report_does_not_replace_the_evidence_of_the_accepted_one; terminal::state::tests::a_conversation_change_reported_by_the_bound_process_keeps_its_options; persist::restore::tests::live_handoff_keeps_a_confirmed_pairing_bound_to_its_process; exhaustive pairing test | Yes
| FR-6 | src/terminal/state.rs (command line, `Bound` confirmed carry and reporter arms kept) | terminal::state::tests::launcher_exec_in_the_same_process_records_the_agent_options; terminal::state::tests::seeded_agent_launch_binds_to_the_first_observed_process; terminal::state::tests::a_process_runs_the_conversation_it_was_last_shown_to_run; persist::restore::tests::live_handoff_keeps_a_confirmed_pairing_bound_to_its_process; persist::restore::tests::native_agent_restore_seeds_saved_launch_options_for_the_next_save; exhaustive pairing test | Yes: the command line and the `herdr agent start` seed were exercised live in case K; the live-handoff carry and external-launcher binding are covered by their automated tests only.
| FR-7 | src/terminal/state.rs | terminal::state::tests::an_unbound_seed_is_not_confirmed_by_a_report_with_no_readable_sender; terminal::state::tests::a_first_observed_process_does_not_take_a_conversation_a_state_report_set (both end with a readable report that pairs the process again) | N/A: it needs a later readable report from the same agent, which a live run cannot force reliably; the automated tests cover it. |
| FR-8 | src/terminal/state.rs | terminal::state::tests::a_conversation_reported_from_outside_the_pane_is_not_paired_with_the_job; terminal::state::tests::a_report_whose_walk_failed_is_not_paired_with_the_job; terminal::state::tests::a_report_from_a_child_agent_in_the_same_job_is_not_paired_with_its_parent | N/A: no separate user surface; the saved session file is the surface the FR-4 live case reads. |
| FR-9 | src/terminal/state.rs (`exhaustive_pairing` module) | terminal::state::tests::exhaustive_pairing::exhaustive_pairing_saves_options_only_beside_their_process_conversation: 289,650 sequences up to length 4, 0 failing; the job senders and the known-limit exclusion are gone and `StateReport` events are in the alphabet (`w1/exhaustive-1.log`) | N/A: the exhaustive test is the verification itself. |
| FR-10 | tests only, no production code | At base `34018fa6` with the new tests spliced in, 5 of 5 fail (`w1/base-check.log`), including 1,571 failing sequences from `[StateReport(S), Observe(P, Nothing)]`; at the fix commit (now `32baedef`) the four focused tests and the exhaustive test pass (`w1/focused-head.log`: 606 passed) | N/A: the base check is the verification itself; see `w1/base-check.log`. |
| FR-11 | docs/next/website/src/content/docs/session-state.mdx, ja/session-state.mdx, zh-cn/session-state.mdx: the foreground-job cause is replaced by "the report did not reach Herdr over its local API socket" | just docs-contract-test: 7 pass; maintenance-test includes scripts.test_docs_translation_parity: pass (`checks-ci.log`, `checks-docs.log`) | N/A: documentation only; checked by just docs-contract-test and the translation parity test. |
| FR-12 | no change: the branch touches only src/app/actions.rs, src/app/api/panes.rs, src/persist/restore.rs, src/persist/snapshot.rs and src/terminal/state.rs | N/A: no code can test an absence of edits; the diff shows no hook, integration asset, API schema, codec, fixture or protocol version change | N/A: verified by the diff. |
| FR-13 | docs/prds/2026-10-19-restore-agent-launch-options/2026-10-19-restore-agent-launch-options-prd-claude.md, Change Log entry of 2026-10-28 07:44:29 EDT | N/A: documentation only | N/A: a documentation record, not a user surface. |
| FR-14 | process: the live check ran in a named debug session `t4proof-*` with its own `HERDR_CONFIG_PATH`, socket overrides cleared, and only agents the implementer started | N/A: a process requirement | Yes

### Change Log

- 2026-10-28 07:52:10 EDT - Started implementation of the PRD as written; no change request yet.
- 2026-10-28 10:16:24 EDT - Verification complete for the fix commit (then `b76db804`, now `32baedef`). `just ci` passed (fmt and clippy
  clean, 4,298 tests passed and 15 skipped, maintenance, UI hot-path and integration-asset suites) and
  `just docs-contract-test` passed. `just check`'s `windows-lint` did not run because this machine has
  no Windows SDK, the same limit T-2 recorded; no Windows-only code changed. The five new or extended
  tests fail at base `34018fa6` (`w1/base-check.log`) and pass at the head. The live check ran in
  isolated debug sessions: case K kept an official-hook Claude's 6 options and case C a Codex's 7
  options across two restarts each; case U showed a report with no readable sender saving no options
  beside a running agent, and variant X showed the same conversation restored bare with the
  `launch options not kept` row and reason `NotRecorded`. Every requirement row is now Yes or a
  reasoned N/A. Notes: no automated test drives a real server foreground job, so FR-1 rests on the
  removed path and the state-level tests; the live-handoff carry and external-launcher binding were
  not exercised live, only by their automated tests; the live checks resumed a real Claude
  conversation once without sending it any prompt. No change request was made; the pre-draft decision
  to drive the live case over the client-shell endpoint was the assumption the PRD allowed.
- 2026-10-28 10:20:19 EDT - `origin/main` advanced to `c1dcd3d2` (PR #11: Grok launch options and the
  agent-accounts restore) before the pull request. The branch was rebased onto it; the conflicts were
  in `src/app/api/panes.rs` and `src/terminal/state.rs`, where the Grok generalization of the agent
  label and the seed `account` field met this removal. The resolution keeps the generalized agent
  filter and drops the foreground-job read, and the new tests now set `account: None`. `just ci`
  passes again (4,313 tests passed, 15 skipped); the live check ran on the pre-rebase binary, whose
  code for this path is the same.

## Introduction/Overview

T-2 taught Herdr to save each Claude or Codex agent's launch options (permission mode, model, effort and the rest)
beside the conversation that agent runs, so a restore resumes the conversation with the same options. Saving options
beside the wrong conversation is the dangerous failure: a restore would then resume a conversation with another
agent's permissions. When Herdr does not know an agent's options it falls back to the bare resume command and shows
`launch options not kept` on the pane (T-2 FR-10 and FR-29); losing options is the safe failure.

To know which agent a conversation belongs to, Herdr has to know which process sent the conversation report. Since
T-2 review round 9 (`b0ab1307`) the API server reads the sending process from the local API socket (the kernel's peer
process id on Linux, macOS and Windows), walks its parents to the nearest Claude or Codex process inside the pane, and
pairs the conversation only with that process. When Herdr cannot read the sender at all, T-2 kept two guesses:

1. **The foreground-job guess.** If the pane's foreground job holds exactly one Claude or Codex process, the report is
   taken to come from that process. This also lets an unbound seed (a restore or `herdr agent start` launch not yet
   bound to a process) be confirmed for the reported conversation. Review round 17's P1
   (`r17-seed-job-inference`) showed the consequence: a seed whose launch exited before detection read it can have its
   options saved beside another agent's conversation.
2. **The first-observation guess.** A process detection reads for the first time takes a conversation a live agent
   reported when Herdr kept no evidence of who reported it.

The project supervisor adopted review finding r7-3 on 2026-10-27 (an engineering decision under G61, inferred, logged
in the T-2 PRD change log at 20:17:07 EDT): remove both guesses rather than keep them as a fallback, so such a report
leaves the options unknown and restore falls back with the marker. Fitz's option 1a (the T-2 PRD, 20:34:26 EDT)
capped T-2's review rounds, so the supervisor moved that change to this follow-up task. The request says to start only
after T-2 merges; T-2 merged into `origin/main` as pull request #1 (`a1a911c8`, 2026-10-28 07:20 EDT), followed by
T-3's pull request #9 (`34018fa6`), which is this task's base.

### What exists today and the actual gap

Evidence was checked by reading the code at `34018fa6` and the branch history; nothing was run live in this interview.

| # | Requested outcome | Existing functionality and evidence | Actual gap and effect of the proposed work |
| --- | --- | --- | --- |
| 1 | A report whose sender Herdr cannot read does not pair with the pane's lone foreground Claude or Codex process. | **Missing (the guess is live).** `session_report_evidence` (`src/app/api/panes.rs:1648-1696`) builds `SessionReportEvidence::ForegroundAgents` from the foreground job when `peer_ancestry` is `None`; `named_processes` (`src/terminal/state.rs:445-452`) names a lone job agent with `ShownBy::ForegroundJob`; `sync_agent_launch_session` (`state.rs:967-1022`) records it in `ProcessSessions`. | Remove the guess: such a report names no process. |
| 2 | An unbound seed is not confirmed for a conversation reported with no readable sender (round 17 P1). | **Missing.** The `LaunchState::Started` arm of `sync_agent_launch_session` (`state.rs:976-998`) confirms the seed when the evidence is `ShownBy::ForegroundJob`. | Remove; the seed stays as it is until detection binds it and a readable report or the process's own command line decides. |
| 3 | A first-observed process does not take a conversation reported with no kept evidence of its sender. | **Missing.** `observe_agent_launch` computes `reported_without_evidence` (`state.rs:910-918`) and `session_for_process` assumes that conversation (`state.rs:384-398`). This path is reached by any report that sets the pane's conversation without going through `apply_agent_session_report` with evidence, for example a `pane.report_agent` state report carrying a session (documented in `docs/next/website/src/content/docs/socket-api.mdx:747`); the server reads the peer only for `pane.report_agent_session` (`src/api/server.rs:467-472`). | Remove; the process stays unpaired. |
| 4 | A sender Herdr read but could not place in the pane names no process. | **Already implemented.** `SessionReportEvidence::Unidentified` (`state.rs:419`, `panes.rs:1672`). | No change. |
| 5 | Reports from the official Claude and Codex hooks keep their options. | **Already implemented.** Both hooks send conversations only through `pane.report_agent_session` over the local API socket (`src/integration/assets/claude/herdr-agent-state.sh:82`, `src/integration/assets/codex/herdr-agent-state.sh:80`), so the sender is read on Linux, macOS and Windows. | Must not regress. Inferred from code, not verified live here: on those platforms with the official hooks, users see no change. |
| 6 | Unknown options restore bare with the marker. | **Already implemented** by T-2 FR-10 and FR-29 (`LaunchOptionsUnavailable::NotRecorded`, `src/agent_resume.rs:180`). | Reused unchanged. |
| 7 | Where the sender is unreadable today. | **Already implemented paths:** the client-shell endpoint passes no peer (`src/server/headless/endpoint_requests.rs:129`, used by the TUI client, SSH and `--machine`); platforms without a peer reader (`src/platform/fallback.rs:223`); a failed peer read; and state reports, whose peer is never read. | These are the only places users will see a change: an agent alone in its pane whose conversation arrives this way restores bare with the marker instead of with its options. |
| 8 | A draft of this change. | **Partially implemented, not merged.** W2's branch `portal/t2-4445c337-w2-r7-3`: `789857e0` (removes both guesses, two new tests that fail at their base, five job-fallback tests folded into one, the rest rewritten to reporter evidence; focused tests and clippy passed) and `424ff154` (WIP: adds a state report with no sender to the exhaustive pairing test; it fails because the test's model of when a state report is accepted is wrong, not because of pairing). Based on `0e162227`, before the round 16 and round 17 fixes; `git merge-tree` against `origin/main` reports no conflicts. Full suite, macOS check, UI hot-path, maintenance, integration-asset and docs-contract tests were not run. Status: `~/.local/share/agent-work/herdr-fast-api-t2/w2/r7-3-draft-STATUS.md`. | The implementer can start from it; it still needs the exhaustive test finished, the docs edited, the full checks and a live check. |

## Goals

1. Herdr never saves launch options beside a conversation on the strength of a report whose sender it could not read.
2. In that case the options stay unknown, so the next restore resumes bare with `launch options not kept`, the existing
   safe failure.
3. Agents whose reports carry a readable sender, which is every official Claude and Codex hook report on Linux,
   macOS and Windows, keep their options exactly as after T-2.
4. The exhaustive pairing test can see reports with no readable sender, including state reports, and finds no
   sequence that saves options beside a conversation without evidence.

## User Stories

- As a Herdr user restoring a session, I never get a resumed conversation running with another agent's permission
  options, even when Herdr could not tell which process reported that conversation.
- As a Herdr user on Linux, macOS or Windows with the official Claude or Codex hooks, my agents still come back with
  their launch options after a restart.
- As a Herdr user whose agent's options could not be tied to its conversation, I see `launch options not kept` on the
  pane so I know to relaunch it in the mode I need.

## Functional Requirements

| ID | Requirement | Source | Implemented | Automated Tests | Interaction Testing |
| --- | --- | --- | --- | --- | --- |
| FR-1 | A Claude or Codex conversation report whose sender Herdr cannot read (no peer process, such as the client-shell endpoint, a platform without a peer reader, or a failed peer read) names no process: Herdr does not take the pane's foreground job, even one holding a single Claude or Codex process, as the sender. | stated (request; supervisor's r7-3 decision) | Yes | Yes | Yes
| FR-2 | An unbound seed (a restore or `herdr agent start` launch not yet bound to a process) is never confirmed for a conversation by a report whose sender Herdr cannot read (review round 17 P1, `r17-seed-job-inference`). | stated (request) | Yes | Yes | Yes
| FR-3 | A process detection reads for the first time never takes a conversation a live agent reported when Herdr kept no evidence of the report's sender, including a conversation set by a `pane.report_agent` state report. | stated (supervisor's r7-3 decision) | Yes | Yes | N/A: the difference appears only in the saved session file, and the order of a state report and the first detection read cannot be controlled in a real agent; the FR-4 live case drives the same report path.
| FR-4 | In the cases of FR-1 to FR-3 the session save records no launch options beside that conversation, and the next restore resumes it with the bare command, the `launch options not kept` marker and reason `NotRecorded` (T-2 FR-10 and FR-29). | stated (request: "leave launch options unknown") | Yes | Yes | Yes
| FR-5 | A report whose sender Herdr reads and places in the pane pairs exactly as after T-2: the official Claude and Codex hooks on Linux, macOS and Windows keep their agents' options across a server restart and a live handoff. | inferred | Yes | Yes | Yes
| FR-6 | Evidence that does not depend on a report's sender keeps working as after T-2: a process's own command line resuming a conversation, restore and `herdr agent start` seeds bound by detection, the live-handoff carry of a confirmed pairing, and external-launcher binding. | inferred | Yes | Yes | Yes: the command line and the `herdr agent start` seed were exercised live in case K; the live-handoff carry and external-launcher binding are covered by their automated tests only.
| FR-7 | Losing the pairing is not permanent: when the agent later reports its conversation with a readable sender, Herdr pairs it and saves its options again. | inferred | Yes | Yes | N/A: it needs a later readable report from the same agent, which a live run cannot force reliably; the automated tests cover it.
| FR-8 | A report whose sender Herdr read but could not place in the pane (`Unidentified`) keeps naming no process, as today. | inferred | Yes | Yes | N/A: no separate user surface; the saved session file is the surface the FR-4 live case reads.
| FR-9 | The exhaustive pairing test includes reports with no readable sender, both conversation reports and state reports carrying a conversation, and finds no sequence that saves options beside a conversation their process was not shown to run. | inferred | Yes | Yes | N/A: the exhaustive test is the verification itself.
| FR-10 | New tests for FR-1 to FR-4 fail on the current `main` code and pass with the change (G12). | rule G12 | Yes | Yes | N/A: the base check is the verification itself; see the log named in Verification Evidence.
| FR-11 | The unreleased docs (`docs/next/website/src/content/docs/session-state.mdx` and its `ja` and `zh-cn` versions) no longer give "another Claude Code or Codex process runs in the same foreground job" as the cause; they say the options are not kept when Herdr cannot tell which process reported the agent's conversation, such as when the report did not reach Herdr over its local API socket. | inferred | Yes | Yes | N/A: documentation only; checked by just docs-contract-test and the translation parity test.
| FR-12 | No change to hook scripts, integration-asset versions, API method shapes, endpoint codecs or fixtures, or the wire protocol version. | inferred | Yes | N/A: no code can test an absence of edits; the diff shows only five src files changed and no hook, integration asset, API schema, codec, fixture or protocol version. | N/A: verified by the diff.
| FR-13 | The T-2 PRD gets a dated change-log entry saying this task removed the guesses its known limit (review rounds 8 and 9) describes, with a pointer to this PRD; its earlier text stays as history. | rule G18 | Yes | N/A: documentation only. | N/A: a documentation record, not a user surface.
| FR-14 | Live checks run only in isolated Herdr sessions and with agents the implementer started for this work; no running user agent or the installed Herdr server is restarted or replaced (T-2 FR-20). | inferred | Yes | N/A: a process requirement. | Yes

## Rules Applied

- G2: every requirement and decision is labeled stated or inferred; the supervisor's r7-3 decision is recorded as the
  supervisor's inferred engineering call, not Fitz's.
- G3: this interview implements nothing; the PRD is left in the task checkout.
- G12: FR-10 (tests fail before the fix, pass after).
- G18: FR-13, and the T-2 records carried into this PRD.
- G61: the engineering choices under Assumptions are made here and recorded as inferred instead of being asked.
- Repository `CLAUDE.md`, Stable client endpoint contract and Runtime/client boundary: FR-12.
- Repository `CLAUDE.md`, Docs: user-facing docs go in `docs/next`; `docs/next/CHANGELOG.md`, the root README and
  `skills/herdr/SKILL.md` are not edited.
- Repository `CLAUDE.md`, Testing: `just check` before committing; unit tests next to the code.

## Naming and Domain Boundaries

The change removes concepts rather than adding them. Inferred, following W2's draft: `SessionReportEvidence` loses
`ForegroundAgents`, its `Default` becomes the case that names no process (`Unidentified`), and `ShownBy::ForegroundJob`
goes away; `session_for_process` loses its `reported_without_evidence` input. If the implementer keeps a variant for
"sender not read" separate from "sender read but not placed" for logging, both must name no process. The pairing rules
stay in `src/terminal/state.rs` (pure state, testable without PTYs); sender reading stays in the API layer.

## Design Pattern Decisions

No design pattern proposals for this feature: it deletes two branches of an existing evidence enum and their callers,
so there is no new structure to organize.

## Non-Goals (Out of Scope)

- Having the hook scripts send the agent's own process id (the original r7-3 proposal). Herdr already reads the sender
  from the socket, which is more reliable than a pid written by a script (a hook's `$PPID` can be an intermediate
  shell).
- Reading the sender for `pane.report_agent` state reports. Under this change such a report leaves options unknown; a
  later `pane.report_agent_session` from the official hooks still pairs.
- The other round 16 and round 17 observations the T-2 PRD lists as candidate follow-ups (for example
  `agent.start_with_launcher` refusing an unlisted shell, the missing leading space on its typed line, and the
  unlogged restore error).
- The known limit where another Claude or Codex process reporting the agent's conversation drops its options until the
  agent reports again (T-2 PRD, review round 14). It loses options and never saves wrong ones.
- Windows-only and WSL behavior beyond compiling and the existing tests.

## Success Metrics

- Zero sequences in the exhaustive pairing test, extended per FR-9, that save options beside a conversation without
  evidence.
- The FR-1 to FR-4 tests fail at `34018fa6` and pass with the change.
- `just check` passes (or any failure is a named pre-existing base failure).
- Live: official-hook Claude and Codex keep their options across two restarts; a conversation set without a readable
  sender restores bare with the marker.

## Assumptions

All inferred (G61), each with its reason:

- FR-5 to FR-9, FR-11, FR-12 and FR-14 are inferred requirements: they preserve T-2's behavior and process where this
  change does not touch it, or describe the testing and docs that make the removal verifiable.
- The implementer may start from W2's draft (`789857e0`, then finish `424ff154`) rebased or merged onto `main`; the
  draft predates the round 16 and 17 fixes (`4120e7f5`, `9bd8b5ad`), so every test touching seeds and launchers is
  rechecked. Reason: it already removes both guesses with tests that failed at their base, and merges cleanly.
- The docs wording follows W2's proposal in the draft status file, adjusted if the final behavior differs. Reason: it
  states the remaining cause precisely in all three languages.
- Hook scripts and integration versions stay unchanged (FR-12). Reason: the T-2 agent decision of 2026-10-27, flagged
  to the supervisor and not overruled in any recorded answer found by this interview's knowledge queries; the sender
  is already read from the socket.
- The live check of the unreadable-sender case may use a `pane.report_agent` state report carrying a conversation, or
  a report sent through the client-shell endpoint, whichever the implementer can drive in an isolated session. If
  neither can be driven, the row's Interaction Testing becomes `N/A: <reason>` and the automated tests carry it.
- Users on Linux, macOS and Windows with the official hooks see no change. Inferred from code (outcome 5 above), not
  verified live in this interview.
- Review follows the project's normal settings for a standalone task (K-6 revision 2); this PRD adds no gate (G48).

## Open Questions

None.

## Interview Answers

No questions were asked. The request, the supervisor's recorded r7-3 decision and round 17 disposition in the T-2 PRD,
and the rules above settle the scope; the remaining choices are engineering calls recorded under Assumptions.

## Supporting Context

- T-2 PRD change-log entries: 2026-10-26 22:11:26 EDT (sender read from the socket, `b0ab1307`), 2026-10-27 20:17:07 EDT
  (r7-3 adopted), 20:34:26 EDT (review cap, r7-3 moved to a follow-up), 20:45:19 EDT (round 17 P1
  `r17-seed-job-inference` at `src/terminal/state.rs:957` of `dc4f2dd7`), and the review rounds 8 and 9 known limit
  under its Assumptions.
- W2's draft branch `portal/t2-4445c337-w2-r7-3` (`789857e0`, `424ff154`) and its status file
  `~/.local/share/agent-work/herdr-fast-api-t2/w2/r7-3-draft-STATUS.md`, with scratch logs in
  `~/.local/share/agent-work/herdr-fast-api-t2/w2/r7-3/`. The brief it worked from is named
  `BRIEF-W2-r7-3-no-inference.md` in the T-2 PRD.
- Live-check setup that worked for T-2 (round 17 notes): isolated sessions, a session-only config via
  `HERDR_CONFIG_PATH`, and the agents started from real bash.
- Knowledge queried for this interview (`t4-r7-3-decision`, `t4-followup-scope`): no recorded decision on r7-3 beyond
  the T-2 PRD; K-6 revision 2 (post-implementation reviews) and K-442 revision 1 (durable job files) apply to the
  implementation.

## Appendix A - Original Request (Verbatim)

Title, description and original text as Portal supplied them:

```text
Leave launch options unknown when the report sender is unreadable

Follow-up to herdr-fast-api T-2 for its round 17 P1: drop inferred launch-option pairing when Herdr cannot read the report sender. Start only after T-2 merges.
```
