# PRD: Truthful agent.prompt outcome when the agent's composer drops or rejects the text

## Implementation Log

| Field | Value |
| --- | --- |
| Started | 2026-10-27 18:24:29 EDT |
| Agent | opencode |
| Model | unknown |
| Reasoning Level | unknown |

### Verification Evidence

| ID | Implementation | Automated Tests | Interaction Run |
| --- | --- | --- | --- |
| FR-1 | `src/app/api/agents.rs` composer check; `codex.toml` selection-list rule | `app::api::agents::tests::agent_prompt_refuses_a_codex_selection_list_without_writing` | Live S6/S8: `agent_not_ready`, seq 3 -> 3, no native turn |
| FR-2 | `src/app/api/agents.rs`; `codex.toml` partial-command rule | `app::api::agents::tests::agent_prompt_refuses_a_partial_slash_command_without_writing` | Live S10/S10b: `agent_not_ready`, seq 3 -> 3, no native turn |
| FR-3 | `src/cli.rs` shared response printer; `src/cli/spec.rs` | `tests/cli/agents.rs` error-exit tests (115 pass) | Refusals and failures exit 1 with the same JSON code/message |
| FR-4 | `src/app/api/agents.rs` names the composer state | App tests assert "selection list" and "slash command" | Live messages quoted in `live3/evidence/RESULTS.md` |
| FR-5 | `src/api/wait.rs` acceptance gate, `agent_prompt_stalled`, `agent_prompt_not_submitted` | `api::wait::tests::stalled_timeout_says_input_was_written`, `not_submitted_evidence_names_the_composer_or_a_rejection` | Live: `/mo` opened the picker -> `agent_prompt_stalled`; `/moXYZ-not-a-command` -> `agent_prompt_not_submitted` |
| FR-6 | `src/api/wait.rs` returns at first evidence; hard cap 5000 ms | `tests/cli/agents.rs` wait tests | S1 returned quickly; S9 settled as requested |
| FR-7 | Removed | Removed | Removed |
| FR-8 | `src/api/wait.rs` queued-evidence branch; `codex.toml` queued rule | `detect::manifest::tests::queued_evidence_skipped_by_composer_match_and_matches_needle` | Working prompt showed the queued message and answered success |
| FR-9 | `src/api/wait.rs` shared refusal and evidence path | `tests/cli/agents.rs` wait tests | S8 refused; S9 succeeded and settled |
| FR-10 | Delivery paths unchanged | `tests/cli/agents.rs` (115 pass) | S1, S2, S3, S4, M7 had native turns; S5 was refused then delivered after clearing |
| FR-11 | `src/app/api/agents.rs` draft refusal; `codex.toml` draft rule | `app::api::agents::tests::agent_prompt_refuses_unsent_typed_text_without_writing` | S5 refused with "unsent typed text"; composer still showed `t`; `T3-S5-REFUSED` count 0 |
| FR-12 | Composer check after existing recipient checks | Existing `agent_prompt_rejects_blocked_agent_without_writing`, `agent_prompt_rejects_managed_agent_while_startup_is_pending`, `tests/cli/agents.rs` | Unchanged refusals pass |
| FR-13 | No clear/edit/retry code added | Refusal tests assert no input bytes | Refusals left the composer untouched |
| FR-14 | `src/api/schema/server.rs` `agent_prompt_outcome`; `src/api/server.rs` default | `api::server::tests::ping_advertises_truthful_agent_prompt_outcome` | N/A: API capability |
| FR-15 | Additive capability only; success shape unchanged | `api::schema::tests::generated_protocol_schema_artifact_is_current` | N/A: API shape |
| FR-16 | `codex.toml` composer rules from captured 0.163.0 screens | Synthetic engine tests only (repository rule) | Live captures and cases in `live3/evidence/` |
| FR-17 | Override checked before use; temporary override removed | N/A: live process rule | No pre-existing override; temporary override removed before the final run |
| FR-18 | Only `codex.toml` gained composer rules | `detect::manifest::tests::all_bundled_manifests_parse_and_validate`; CLI tests | Non-Codex working prompt now returns the unknown outcome; no screen rules changed |
| FR-19 | Engine, app, schema and wait tests | App 7, wait 4, schema 45, detect 40 pass | N/A: tests are the artifact |
| FR-20 | Normal tests use synthetic inputs; live run separate | Targeted suites pass | Live run under FR-21 |
| FR-21 | Live replay in an isolated branch server | N/A: live replay is the evidence | `live3/evidence/RESULTS.md` and raw answers |
| FR-22 | Not run by this task | N/A: later Portal task | N/A: later Portal task |
| FR-23 | Isolated socket/config/state/workspace | N/A: test condition | No live Herdr server used |
| FR-24 | Evidence in `~/.local/share/agent-work/herdr-fast-api-t3/` | N/A: process rule | `live3/evidence/RESULTS.md`, `rollout-path.txt` |
| FR-25 | One composer read per prompt; no render/detection loop work | N/A: inspection-level check | N/A: not a user surface |
| FR-26 | `docs/next` agent automation, CLI reference, socket API; generated schema | `api::schema::tests::generated_protocol_schema_artifact_is_current` | N/A: docs are the artifact |
| FR-27 | Windows paste path unchanged; composer check platform-neutral | N/A: Windows cross toolchain not configured | N/A: no Windows host |
| FR-28 | Branch on main; PR targets main | N/A: metadata | N/A: metadata |
| FR-29 | Commit/version recorded in report and evidence | N/A: metadata | N/A: metadata |
| FR-30 | Capability and error names in `docs/next` and CLI help | Schema/capability test; docs review | Names `agent_prompt_outcome`, `agent_prompt_stalled`, `agent_prompt_not_submitted` are stable |
| FR-31 | Success only after acceptance evidence | App/wait tests; live unknown/not-submitted cases are errors | Every live success had a native user turn; unknown/not-submitted returned errors |

### Change Log

- 2026-10-27 20:00:00 EDT - Implemented FR-1..FR-31 against the revised PRD: composer refusal for selection lists, partial commands and real drafts; plain-call acceptance gate; `agent_prompt_stalled` for unknown; `agent_prompt_not_submitted` for text that stayed or was rejected; success only on acceptance. Source: herdr-fast-api project supervisor decisions under G61 (questions 1, 3, 4 and 5).
- 2026-10-27 20:00:00 EDT - Removed the earlier draft marker and accepted/unverified success fields after the supervisor's question-5 decision replaced the question-2 answer. FR-7 is marked Removed; FR-10 and FR-11 now record the refusal and later delivery. Source: supervisor decision under G61, 2026-10-27.
- 2026-10-27 20:30:00 EDT - f52a9cdc asked for stable names for an accepted-or-unverified success field and for the capability. The supervisor's later FR-7 removal means no success field exists to name; the stable contract is the capability `agent_prompt_outcome` and the error codes `agent_prompt_stalled` and `agent_prompt_not_submitted`. Source: requester exchange f52a9cdc, superseded on the field by FR-7.
- 2026-10-27 20:20:00 EDT - Recorded the requester's superseding note f52a9cdc: the earlier request for two new error codes is withdrawn; refusals reuse `agent_not_ready`, unknown reuses `agent_prompt_stalled`, and only `agent_prompt_not_submitted` is new. Source: requester exchange f52a9cdc, acknowledged in follow-up b0f1af4b.
- 2026-10-27 20:10:00 EDT - `flagged:` live validation did not reproduce a natural-language predicted reply in Codex's composer. Command suggestions appeared above the composer line; the draft rule inspects only the last prompt line, so a future natural-language suggestion placed in the composer would be treated as typed text. Recommendation: re-capture that surface when Codex changes it. Source: FR-11 live-smoke clause.
- 2026-10-27 20:00:00 EDT - Raised the Codex manifest to `2026.10.27.1` (engine 4) and merged the current published Codex rules with the composer rules, so the bundled manifest does not shadow newer remote detection. Source: rule (repository: agent detection updates).

Status: ready for implementation (Portal task T-3, interview run 14a5ca2f, 2026-10-27). Questions 1 to 4 were
decided by the herdr-fast-api project supervisor under G61; its later decision on question 5 replaced its answer on
question 2 (unsent drafts are refused). Every decision is inferred, not Fitz's own, and open to his review.

## Introduction/Overview

`agent.prompt` (socket API) and `herdr agent prompt` (CLI) type a message into a named agent's terminal and press
Enter. Without the optional `wait` parameter, the call answers success as soon as the bytes are queued to the PTY.
Portal task T-100 reproduced, in an isolated Herdr server with a real Codex agent (codex-cli 0.163.0, Herdr
0.9.3-fast-api.6e51c8cb-gpu-bulk-direct, private protocol 22), two states where Codex never receives the message but
the call still answers success:

- **Picker open (T-100 M6, probe S6):** Codex's `/model` picker was open; the pasted text was discarded and the Enter
  selected the highlighted model, advancing the picker to "Select Reasoning Level". The Enter therefore also changed
  the person's model selection flow, a side effect beyond the lost message.
- **Partial slash command (T-100 M8, probe S10):** `/mo` was already in the composer; the merged text became
  `/moPortal…`, Codex answered "Unrecognized command '/moPortal'", and the text stayed unsent in the composer.

In both, agent status stayed idle, `state_change_seq` did not change, and the native Codex session recorded no user
turn. A milder case (T-100 M5, probe S5): an unsent draft `t` merged with the message, which arrived as `t` followed by
the prompt.

The goal is that the call itself tells a caller whether a message was refused before anything was written, written but
not accepted, or accepted, so a caller can tell a written message from an accepted one. The mechanism is demonstrated;
which UI action the person performed in Portal's original incident is unproven, because that pane's screen was not
available.

### Existing functionality and the actual gap

Evidence checked in this checkout (`portal/t3-14a5ca2f`, based on `main` at `ebff37bc`), in T-100's evidence folder,
and in Portal's `origin/main` at `279d78a71` (read-only). Nothing was run live in this interview.

| # | Requested outcome | Existing functionality and evidence | Actual gap and effect of the proposed work |
| --- | --- | --- | --- |
| 1 | Plain `agent.prompt` (no `wait`) reports a truthful outcome for the picker and partial-command cases. | **Missing.** `App::handle_deferred_agent_api_request` and `queue_agent_prompt` (`src/app/api/agents.rs:155-284`) answer `AgentPrompted` once `queue_user_input_submission` completes, which means the bytes were written. T-100 probes S6 and S10 show `{"type":"agent_prompted"}` with seq unchanged. | New behaviour: the call refuses before writing or reports not accepted in these cases. Callers see an error where they used to see success, only in these cases. |
| 2 | Same for `agent.prompt` with `wait`. | **Partially implemented.** `prompt_agent` in `src/api/wait.rs:177-357` waits up to `AGENT_PROMPT_EFFECT_TIMEOUT_MS` (5000 ms, `src/api/wait.rs:20`) for working or blocked after the submission's `state_change_seq` and otherwise answers `agent_prompt_stalled`. T-100 probe S8 shows this catches the picker case, but only after the Enter has already acted on the picker. The partial-command case with `wait` was not probed (not verified). | Keep it; add the refusal before writing so the Enter never reaches a picker. |
| 3 | Truthful outcome when the agent was already working before the prompt. | **Missing (read from source, not run live; T-100 named it unverified, not a proven defect).** `prompt_started_working` (`src/api/wait.rs:206-207, 240-245`) skips the effect check entirely when the agent was Working before the prompt. Not run live by T-100 or here. Codex accepts input during a turn as a queued message (the Codex manifest's working rule names "Queued follow-up inputs", "Messages to be submitted after next tool call" and "Messages to be submitted at end of turn"), so a status change is not expected evidence there. | New behaviour: a working agent's submission is reported as accepted only on positive evidence, otherwise as unverified, never as accepted by skip. |
| 4 | CLI `herdr agent prompt` reports the same outcome. | **Partially implemented.** `agent_prompt` in `src/cli/agent.rs` (around line 838) sends the same request and prints the response through `print_response`, so it inherits the API outcome. Help text in `src/cli/spec.rs:340-363` documents today's behaviour. | Follows from the API change; help text and CLI reference need updating. |
| 5 | Keep recipient protections. | **Already implemented.** `queue_agent_prompt` refuses an empty prompt (`empty_agent_prompt`), unknown target (`agent_not_found`), a blocked agent (`agent_blocked`), an unknown agent or pending managed launch (`agent_not_ready`), and a pane whose foreground process is no longer the agent (`runtime_hosts_agent`, `agent_not_ready`). Target resolution can answer `agent_target_ambiguous` (`src/app/agents.rs:339`). The `wait` path pins terminal id, name and agent (`agent_wait_identity_matches`). | No change; regression tests must keep them. |
| 6 | Keep authority protections. | **Not in Herdr.** Herdr's socket API has no per-caller authority check on `agent.prompt`; authority (permits, current authorization, closed or cancelled runs) is enforced in Portal (`HerdrExecutionBackend::promptAgent` requires a `NativeWritePermit`). | No change in Herdr. |
| 7 | Keep draft protections. | **Not in Herdr.** No code path in `src/` inspects or protects unsent composer text before `agent.prompt` writes. Portal's protocol-update route relies on submitting together with unsent human text (Portal source comment citing K-322 in `deliverProtocolUpdate`; K-322 revision 1 as quoted in T-100's evidence). | Herdr must not start clearing, editing or escaping a person's draft. New: where Herdr can tell real typed text from suggestions and placeholders, it refuses rather than submitting the person's text (FR-11). |
| 8 | Recognise a busy composer from the screen. | **Partially implemented.** `codex_prompt_ready` (`src/pane/agent_detection.rs:321-336`) detects Codex's empty composer placeholder ("› Ask Codex to do anything"), but only as startup evidence. `src/detect/manifests/codex.toml` (version 2026.09.23.1) has no rule for the `/model` picker or a partial slash command; T-100 saw status idle in both. | New Codex screen evidence for the picker and partial-command states, built and validated per the repository's evidence-based detection rules. |
| 9 | Advertise the new behaviour. | **Mechanism exists.** `ServerCapabilities` (`src/api/schema/server.rs:23-53`) carries an advertised `methods` list and boolean feature flags (for example `terminal_input_ack`); `default_capabilities()` (`src/api/server.rs:71-87`) already advertises fork additions such as `agent.start_with_launcher`. `agent.prompt` is a JSON socket API method and does not appear in the frozen generation-1 endpoint fixtures (`tests/fixtures/endpoint-method-shapes-v1.json`). | Add one advertised capability so callers can tell whether a success from a given server distinguishes accepted from unverified. |
| 10 | Portal's side. | **Already done in Portal.** Per the T-100 requester, Portal PR fitzzz/portal#213 (merged 2026-10-21, merge commit `1c3cdb95`) makes ordinary prompts pass `wait {until:[working,blocked]}` and record `agent_prompt_stalled` as unknown. Confirmed by reading Portal `origin/main` `279d78a71`: `promptAgent` sends that wait; `preEffectRefusal` treats only `empty_agent_prompt`, `agent_blocked`, `agent_not_ready` and `agent_target_ambiguous` as proof that nothing was written; every other error after a possible write is Unknown and never resent automatically (`docs/native-submission-outcomes.md`). Portal's protocol-update route (`deliverProtocolUpdate`) deliberately omits `wait` and submits while the recipient is working; Portal's prompt timeout is 20000 ms (`kPromptTimeoutMs`). | No Portal work in this task. The Herdr change must keep these Portal mappings correct without a Portal change (see FR-4, FR-7, FR-22). |

## Goals

1. A plain `agent.prompt` or `herdr agent prompt` never answers success for T-100's picker case or partial slash
   command case.
2. In those two cases, nothing is typed into the agent, so the Enter cannot act on a picker or a half-typed command.
3. A caller can distinguish "refused, nothing written" from "written, not submitted" from "written, outcome unknown"
   from "accepted"; a success always means acceptance was observed.
4. Five of the six T-100 conditions that already deliver (M1, M2, M3, M4, M7) still answer success and still reach
   the native session; the sixth, M5 (draft present), is refused with nothing typed and delivered once the text is
   cleared.
5. Existing recipient protections, Portal's current outcome mapping and every unrelated caller's contract stay correct
   without changes on their side.
6. Servers advertise the behaviour, so a caller knows whether a success from a given server is confirmed.

## User Stories

- As a supervisor-delivery system such as Portal, I want the prompt call to tell me when the agent did not accept my
  message, so I never record a lost message as sent.
- As a script author or agent using `herdr agent prompt`, I want a non-zero exit when the agent dropped or rejected my
  text, and a message saying whether anything was typed, so I do not resend a message that may have arrived.
- As a person with an agent pane open, I want an automated prompt never to select something in a picker I opened, and
  never to clear or rewrite what I am typing.
- As a client of an older Herdr server, I want to see from the server's capabilities whether its success answers are
  confirmed, so I do not trust a write-only success.

## Functional Requirements

| ID | Requirement | Source | Implemented | Automated Tests | Interaction Testing |
| --- | --- | --- | --- | --- | --- |
| FR-1 | With Codex's `/model` picker (or another Codex selection list recognised by the same evidence) open, `agent.prompt` with or without `wait` refuses before writing anything and does not answer success. | stated (original request); refuse-before-write choice herdr-fast-api project supervisor decision under G61, 2026-10-27 (#1) | Yes | Yes | Yes |
| FR-2 | With a partial slash command such as `/mo` in Codex's composer, `agent.prompt` with or without `wait` refuses before writing anything and does not answer success. | stated (original request); refuse-before-write choice herdr-fast-api project supervisor decision under G61, 2026-10-27 (#1) | Yes | Yes | Yes |
| FR-3 | `herdr agent prompt` reports every new outcome with the same code and message as the API; refusals and not-accepted outcomes exit non-zero. | stated (original request) | Yes | Yes | Yes |
| FR-4 | The refusal before writing uses the existing `agent_not_ready` code, with a message naming the composer state (for example "agent X is not at an empty prompt: a selection list is open"). | inferred (#1: "not-ready style answer" in the request; Portal already classifies `agent_not_ready` as nothing written) | Yes | Yes | Yes |
| FR-5 | After writing, the call answers success only on evidence of acceptance: the composer cleared, a queued-message indicator showing the message, a working or blocked state after the submission's `state_change_seq`, or a new turn. Without such evidence it answers an explicit unknown outcome as an error. It answers an explicit not-submitted outcome as an error only when Herdr has positive evidence that the text stayed in the composer or was rejected (for example Codex's "Unrecognized command" line). Both errors say that input was written and must not be resent without checking. The unknown outcome uses the existing `agent_prompt_stalled` code (with a message fitting the case, including a working start), because every Portal generation treats it as unknown and never resends it: builds from Portal commit `b4d7ff99a` (2026-10-26) on through the generic classifier, and older builds such as T-100's merged head `9ec2ac19` through an explicit promotion of that one code. The not-submitted outcome uses one new, distinct code that is not one Portal treats as nothing written (`empty_agent_prompt`, `agent_blocked`, `agent_not_ready`, `agent_target_ambiguous`); its exact name is the implementer's call. Portal builds from `b4d7ff99a` on record it as unknown; older Portal builds would requeue and resend it, which the pull request states as a compatibility note. | herdr-fast-api project supervisor decision under G61, 2026-10-27 (#1) | Yes | Yes | Yes |
| FR-6 | The plain call returns as soon as acceptance evidence appears, targeting well under one second, with a hard cap of the existing 5000 ms effect window; it never waits for the turn to finish. | herdr-fast-api project supervisor decision under G61, 2026-10-27 (#1) | Yes | Yes | Yes |
| FR-7 | Removed (2026-10-27, superseded by FR-31): a success answer with an optional accepted-or-unverified field. The supervisor's decision rules out success without evidence. | Removed | N/A: removed | N/A: removed | N/A: removed |
| FR-8 | When the agent was already working before the prompt, the call answers success only on positive evidence. For Codex, the evidence is the composer clearing with the message shown as a queued message under the working status, or a new turn. Without evidence within the window, and for agents with no working-state evidence rules, the call answers the unknown outcome (FR-5). | herdr-fast-api project supervisor decision under G61, 2026-10-27 (#1); requester asked T-3 to judge `prompt_started_working` | Yes | Yes | Yes |
| FR-9 | `agent.prompt` with `wait` uses the same refusal (FR-1, FR-2, FR-4) and the same working-state evidence (FR-8) instead of skipping the effect check, then continues to the requested settled state as today. | inferred (#1) | Yes | Yes | Yes |
| FR-10 | Five of the six T-100 conditions that already deliver still answer success and reach the native session: first message after start (M1), idle (M2), first of two back to back (M3), second sent close behind (M4), and the next message after the picker was closed (M7). The sixth, unsent draft present (M5), changes: refused with nothing typed, then delivered once the text is cleared (FR-11). | stated (original request); M5 change: herdr-fast-api project supervisor decision under G61, 2026-10-27 (#5, replacing its #2 answer) | Yes | Yes | Yes |
| FR-11 | When a person's real typed text is in the agent's input box, nothing is typed and the call answers `agent_not_ready` with a message saying unsent typed text is present in the agent's input box (the message does not echo the text itself). Suggestions, placeholder text and predicted replies never count as a draft. Where Herdr has no evidence-based rule to tell real typed text apart (any agent without such a rule), it does not refuse and the after-typing acceptance check applies. No draft marker is added. The live smoke test (FR-16, FR-21) covers a real draft (refused), an empty composer showing placeholder text and one showing a suggestion (both submitted). If the live capture shows real typed text cannot be told apart from placeholder or suggestion text for Codex, the pull request says so as `flagged: <what was seen>` with a recommendation, and Codex then falls under the no-rule path. | herdr-fast-api project supervisor decision under G61, 2026-10-27 (#5, replacing its #2 answer) | Yes | Yes | Yes |
| FR-12 | Existing refusals keep their codes, meanings and order: `empty_agent_prompt`, `agent_not_found`, `agent_target_ambiguous`, `agent_blocked`, `agent_not_ready` (unknown agent, pending managed launch, foreground process changed), and `agent_not_running` on identity change during `wait`. The new composer refusal runs after them. | stated (original request: keep recipient protections) | Yes | Yes | Yes |
| FR-13 | Herdr never clears, edits or sends Esc or other keys to the composer to make a submission succeed, and never retries a submission on its own. | stated (original request: keep draft protections); rule K-19 revision 2 | Yes | Yes | Yes |
| FR-14 | The server advertises the truthful-outcome behaviour as one new capability in `ServerCapabilities`, which `ping` returns (additive, optional, absent or false on older servers), following the repository's stable endpoint contract. The existing `wait` mode keeps working. | stated (original request); supervisor decision (#1) | Yes | Yes | N/A: API field, checked by a schema test. |
| FR-15 | No new socket method, no provider-native transport, no new required request field, and no change to the request or response shape of other methods. | stated (original request) | Yes | Yes | N/A: additive schema, checked by a schema test. |
| FR-16 | Codex screen evidence for the selection-list, partial-command, queued-message and real-draft states is captured from a real Codex with `herdr agent read <pane> --source detection --format text` (and `--format ansi` where styling such as dim placeholder text matters), encoded as explicit AND/OR gates in `src/detect/manifests/codex.toml` without whole-pane incidental text, and validated with live smoke tests recording the Codex version, signals and outcomes. If the manifest schema cannot express a composer state, the engine gains a neutral, agent-independent way to express it. | rule (repository: screen detection is evidence-based; agent detection updates) | Yes | N/A: repository rule forbids unit tests that classify captured agent screens; synthetic engine tests cover the schema. | Yes |
| FR-17 | Before writing a local agent-detection override for live testing, check whether one exists; never overwrite or remove a pre-existing override, and remove or restore the temporary override when done. | rule (repository: agent detection updates) | Yes | N/A: process rule for the live test; recorded in the evidence. | Yes |
| FR-18 | Agents other than Codex get only the generic checks (FR-5, FR-8 unverified path); no other agent's screen rules change in this task. Other agents, Claude Code included, get up-front rules in a later task only once a dropped or rejected prompt is reproduced for them. | herdr-fast-api project supervisor decision under G61, 2026-10-27 (#3) | Yes | Yes | N/A: no other agent screen rules changed. |
| FR-19 | Unit tests cover Herdr's engine behaviour (refusal before write, including the real-draft refusal and the no-rule path, not-submitted and unknown after write, success only on evidence, working-case handling, capability advertisement, unchanged existing refusals) with synthetic manifests and minimal inputs, built with `AppState::test_new()` or `Workspace::test_new()` where possible; no test classifies captured or invented Codex screens against bundled rules. | rule (repository: testing; detection testing) | Yes | Yes | N/A: automated tests are the artifact of this row. |
| FR-20 | The normal test suite stays deterministic and needs no real agent; the real-Codex checks run as a separate, intentional live smoke test. | rule K-14 revision 2 | Yes | Yes | N/A: the live run is separate by design and recorded under FR-21. |
| FR-21 | Before merge, the implementer replays T-100's raw probe cases (S1 idle, S2 long mixed-script, S3 back to back, S4 close behind, S5 draft, S6 picker, S8 picker with wait, S9 idle with wait, S10 partial command) plus a working-agent case against an isolated Herdr server built from the branch with a real Codex, and records for each the answer, status, `state_change_seq` and whether the native rollout has a user turn. Expected: S5 (real draft), S6, S8 and S10 refused with nothing typed; S5 then delivered with a native user turn once the test clears the text in its own isolated pane; the others succeed with a native user turn; the working case succeeds on queued-message evidence. The results table goes in the pull request description as the pre-merge evidence; the raw evidence files stay in the agent-work folder (FR-24). | herdr-fast-api project supervisor decision under G61, 2026-10-27 (#4) | Yes | N/A: the live replay is the verification, not an automated test. | Yes |
| FR-22 | Portal's `execution.live_supervisor_delivery` check is run by a later Portal task against the merged Herdr commit and version, in an isolated session; it is not a merge condition for this task. After merge, the herdr-fast-api project supervisor sends Portal a project request naming that commit and version, and the result is recorded against this task when it arrives. | stated (requester note, 2026-10-27); herdr-fast-api project supervisor decision under G61, 2026-10-27 (#4) | N/A: later Portal task | N/A: later Portal task | N/A: later Portal task |
| FR-23 | Live validation runs only in isolated Herdr servers created for the test, never against Fitz's running Herdr. | stated (original request) | Yes | N/A: isolation is a test condition. | Yes |
| FR-24 | Live-test evidence (pane captures, probe logs, server logs, native rollout excerpts) is kept in `~/.local/share/agent-work/herdr-fast-api-t3/` and named in the report, never committed to the repository. | rule K-442 revision 1 | Yes | N/A: evidence retention is a process rule. | Yes |
| FR-25 | The composer check runs once per prompt request, outside render, layout, PTY parsing and detection loops, and keeps terminal-core lock time minimal. | rule (repository: multiplicative performance paths) | Yes | N/A: once-per-prompt check, reviewed by inspection; no benchmark for this path. | N/A: not a user surface. |
| FR-26 | Socket API docs, CLI reference, `herdr agent prompt` help text, agent-automation docs and the API schema in `docs/next` describe the new outcomes, the optional success fields and the capability; `docs/next/CHANGELOG.md`, root README, published docs and `skills/herdr/SKILL.md` are not edited. | rule (repository: docs) | Yes | Yes | N/A: documentation is the artifact. |
| FR-27 | Windows builds keep working, including the Codex paste-boundary path in `queue_agent_prompt`; platform-specific code stays compile-gated. | rule (repository: platform code) | Yes | N/A: Windows cross toolchain not configured in this run; the composer check is platform-neutral and the Windows paste path is unchanged. | N/A: no Windows host. |
| FR-28 | Work is based on `main` and the pull request targets `main`. | stated (original request) | Yes | N/A: branch and PR metadata. | N/A: same. |
| FR-29 | The implementation report and the project-request result name the commit and the Herdr version string that carry the change, kept separate from Portal's pinned upstream contract (herdrdev/herdr v0.9.3, `7b116c05`) and from the installed build (`0.9.3-fast-api.6e51c8cb-gpu-bulk-direct`). Switching the running Herdr onto the change is not part of this task. | stated (requester note, 2026-10-27) | Yes | N/A: report metadata. | N/A: same. |
| FR-30 | The released documentation names the capability and every new error code and refusal message form, and these names and values are not renamed or reinterpreted once a version carries them; a later change uses a new name. | rule (repository: stable client endpoint contract); requester exchange f52a9cdc, 2026-10-27 | Yes | Yes | N/A: documentation is the artifact. |
| FR-31 | On a server advertising the FR-14 capability, a success answer from `agent.prompt` always means acceptance was observed; the call never returns plain success without evidence. | herdr-fast-api project supervisor decision under G61, 2026-10-27 (#1) | Yes | Yes | N/A: covered by the success-only-on-evidence cases in FR-5 and FR-21. |
## Rules Applied

- G2 / K-1 revision 2: every requirement and decision is labelled stated, rule or inferred; the withdrawn questions'
  outcomes are recorded as inferred, not as Fitz's decisions.
- G3: this interview authorizes no implementation, merge or deployment.
- G12: the requirements rest on T-100's reproduction; FR-21 requires reproducing each case before and after the fix.
- G48 / K-20 revision 2: no new review or sign-off gate; Portal's live check is not made a merge condition beyond what
  the requester described (FR-22).
- G61: API shape, error-code reuse, the effect window and test placement were decided here as engineering calls
  (questions 1 to 4 withdrawn; questions 1, 3 and 4 then decided by the project supervisor, see Interview Answers).
- K-14 revision 2 (verification evidence): FR-20.
- K-19 revision 2 (submitted instructions, G45): FR-13 and FR-11; unsubmitted input-line text is never an
  instruction, which the supervisor cited for refusing real drafts.
- K-442 revision 1 (durable job files): FR-24.
- Portal's K-322 revision 1 was not returned by this project's knowledge lookup (it is recorded for Portal's project);
  this PRD relies on it only as quoted in T-100's evidence and in Portal's source comments.
- Repository AGENTS.md: stable client endpoint contract (FR-7, FR-14, FR-15), evidence-based screen detection and
  agent detection updates (FR-16, FR-17), testing (FR-19), multiplicative performance paths (FR-25), docs (FR-26),
  platform isolation (FR-27), runtime/client boundary (neutral server names below).

## Naming and Domain Boundaries

- **Submission outcome** is a runtime fact owned by the server and exposed through the JSON API, per the repository's
  runtime/client boundary. Names stay neutral (no TUI terms such as widget, row or card).
- **Composer** in this PRD means the agent's own input box. Herdr's code calls the related startup evidence "prompt
  ready" (`codex_prompt_ready`); the implementer should reuse that vocabulary where it fits rather than invent a
  parallel term, and should decide whether the startup check and the new per-prompt check share one implementation.
- **Accepted** means Herdr observed the agent take the message (composer cleared, queued indicator, state change or
  turn). It is not model consumption; Portal's vocabulary for that later step is model receipt.
- **Not submitted** and **unknown** are the two written-but-not-accepted outcomes (FR-5): positive evidence the text
  stayed or was rejected, versus no evidence either way.
- Error codes: reuse `agent_not_ready` (refused before writing); the unknown outcome may reuse `agent_prompt_stalled`;
  new codes must stay outside Portal's nothing-written list. Code and capability names are the implementer's call
  (inferred) and become contract once released (FR-30).

## Design Pattern Decisions

No design pattern proposals for this feature: the repository already has the two structures the change needs, a
declarative per-agent screen-rule engine (`src/detect/manifests/`) for recognising composer states and ordered guard
clauses in `queue_agent_prompt` for refusals, and the request adds no new variation point that a pattern would
organise.

Considered and rejected:

- State pattern for composer states: a small enum read from the detection result is enough; no per-state behaviour
  object is needed.
- Chain of responsibility for submission checks: the existing ordered guard clauses already express this without
  indirection.
- Per-agent strategy objects in Rust: per-agent knowledge belongs in manifests, which the repository keeps as the
  source of truth.

## Response/Execution Order

Order of one `agent.prompt` call (herdr-fast-api project supervisor decision under G61, #1):

1. Existing recipient checks, unchanged, in today's order (FR-12).
2. Composer check from the current detection snapshot: a recognised Codex selection list, partial slash command or
   real typed draft refuses with `agent_not_ready`; nothing is written (FR-1, FR-2, FR-4, FR-11).
3. Write the text and Enter, exactly as today (including Copilot focus and the Windows Codex paste boundary).
4. Acceptance check, targeting well under one second, capped at the existing 5000 ms effect window, returning at the
   first evidence (FR-5, FR-6, FR-8):
   - evidence of acceptance (composer cleared, queued indicator, working or blocked after the submission's seq, new
     turn): success;
   - positive evidence the text stayed or was rejected: not-submitted error;
   - no evidence, including a working agent with no evidence rules: unknown error.
5. Success always means accepted (FR-31).
6. With `wait`, continue to the requested settled state as today (FR-9).

## Non-Goals (Out of Scope)

- A new socket method, a provider-native message transport, or reviving the cancelled native-API successor (0da6).
- Changes to Portal; Portal already records `agent_prompt_stalled` as unknown and needs no change for this PRD's codes.
- Clearing or editing a person's unsent draft; draft refusal for agents without an evidence-based draft rule.
- Composer rules for agents other than Codex.
- Automatic retries of an unaccepted or unverified submission.
- Switching Fitz's running Herdr onto the change, or proving which UI action a person took in the original incident.
- Changing `agent.send_keys`, `agent.wait` or any other method's contract.

## Success Metrics

- In an isolated server with a real Codex, replaying T-100's cases (FR-21): picker open and partial slash command are
  refused with nothing typed on every run, and so is a real draft (S5), delivered once cleared; S1 to S4 and S9 answer
  success with a native user turn on every run; the
  working Codex case answers success on queued-message evidence, never a false refusal.
- After merge, Portal's `execution.live_supervisor_delivery` check (run by a Portal task) shows the picker and
  partial-command cases as not submitted or unknown and the six delivering cases as success with a native user turn.
- `just check` passes.

## Assumptions

- Inferred rows, all open to Fitz's review: FR-4 and FR-9. FR-1, FR-2, FR-5, FR-6, FR-8 and FR-31 (default
  truthful outcome), FR-11 (unsent drafts), FR-18 (agent coverage) and FR-21 and FR-22 (validation split) are the project supervisor's
  decisions under G61, also inferred rather than Fitz's.
- Confirmed by the requester (exchange f52a9cdc, 2026-10-27, against Portal `origin/main` `279d78a71`): `agent_not_ready`
  for a recognised busy composer arrives in Portal as known unsent; the follow-up returns to Queued with its reason
  visible and is sent again when the agent next waits for input, with no duplicate. A picker a person leaves open makes
  Portal keep requeueing with a visible reason, which the requester accepts. `agent_prompt_stalled` on a plain call
  becomes DeliveryUnknown and is never resent automatically. For callers other than Portal, reusing `agent_not_ready`
  remains inferred.
- Confirmed by the requester: Portal today ignores optional success fields; with FR-7 removed and no draft marker,
  nothing new on a success needs reading.
- Confirmed by the requester (exchange f52a9cdc): a person who never clears real typed text keeps a Portal message
  requeued with a visible reason; T-100 prefers that to submitting the text. T-100 (exchange d699686d) also asked
  that the refusal's message say a draft is present so Portal can show it, and that any merge option for other
  callers be opt-in and off by default; this PRD adds no merge option.
- Verified in Portal's repository (read-only): the generic outcome classifier landed in Portal commit `b4d7ff99a`
  (2026-10-26) and is contained in `92dd41cb2`, the commit the installed Portal release directory
  `20261027T194737Z-92dd41cb` is named after (that the release was built from it is inferred from the name). T-100's
  answer d699686d described the older behaviour at `9ec2ac19`, where an unrecognised error code is requeued and
  resent; T-100 itself withdrew that reading in exchange f52a9cdc.
- Inferred consequence of the supervisor's decision: Portal's protocol-update route (no `wait`, recipient often
  working) gets success for a working Codex on queued-message evidence, but an unknown outcome for a working agent of
  any other kind, which Portal records as DeliveryUnknown and does not resend. Told to the requester in exchange
  `t3-supervisor-decision-note-1`.
- Inferred: the added delay (target under one second, cap 5000 ms) on plain calls is acceptable; Portal's prompt timeout is 20000 ms, and T-100
  probe S9 saw Codex reach working about 0.7 s after submission.
- Inferred: the partial slash command case with `wait` would already answer `agent_prompt_stalled` (no state change);
  not probed.
- Inferred: Codex input typed during a turn is shown as a queued message under the working status, based on the Codex
  manifest's working rule; not verified live for this task.
- Inferred: Codex's placeholder and suggestion text in an empty composer can be told apart from real draft text by
  styling; FR-11 says what happens if live capture shows it cannot.
- Inferred: the test harness clearing its own isolated pane's composer to complete S5 is a test step standing in for a
  person, not Herdr behaviour (FR-13 still holds).

## Open Questions

- Resolved: question 5, unsent drafts versus the M5 required check. The supervisor decided refusal (follow-up
  5bdc9215), replacing its answer on question 2; M5's required check changes, and the supervisor tells T-100.
- Non-blocking, asked of T-100 (exchange `t3-supervisor-decision-note-1`): whether Portal accepts an unknown outcome
  for working recipients of agent kinds other than Codex on its protocol-update route. It does not change what gets
  built.

- Resolved: T-100's preference on drafts. The requester (exchange f52a9cdc) preferred refusal and left the choice to
  this task; the supervisor's final decision matches that preference.

## Interview Answers

Questions 1 to 4 were sent through Portal on 2026-10-27 and routed to Fitz. All four were withdrawn the same day,
before any answer, after the Portal introduction for this request delegated "technical questions, implementation
details, the exact shape of the delivery within the requested result, and how to test it" to the two task sides, the
requester's note stated how Portal's check would be run, and Portal's source showed how it classifies codes. The
herdr-fast-api project supervisor then withdrew questions 1, 3 and 4 on its side as engineering choices and recorded
its own decisions under G61, shown below. Question 5 was raised afterwards. None of the answers below is Fitz's
decision; each is inferred and open to his review.

### 1. When Herdr cannot confirm that an agent accepted a prompt, should every agent.prompt call report that truthfully by default, or only callers that ask for it?

Options: (a) truthful by default for every call; (b) only when the caller opts in; (c) refuse up front only, no
after-typing check.

**Answer:** Option (a), with refinements, decided by the herdr-fast-api project supervisor on 2026-10-27 (Portal
follow-up 93fe36ee), inferred under G61 and Fitz's 2026-10-27 request that engineering calls not wait for him; not
Fitz's own decision. Refinements as recorded: (1) before typing, if Herdr recognises a busy composer (picker open,
partial slash command, unsent draft), refuse with a not-ready error and type nothing; (2) after typing, wait a bounded
window (target well under a second, hard cap a few seconds) for evidence of acceptance: composer cleared or queued
indicator, status change, or a native turn; (3) without that evidence, return an explicit outcome_unknown, and say
not_submitted only when Herdr has positive evidence the text stayed or was rejected; never return plain success
without evidence; (4) advertise the behaviour as a capability in ping, and keep the existing wait mode working; (5)
the required checks stay: the six conditions that already deliver, including prompting a working agent, must still
report success. Supervisor's reason: the false success is the defect; an opt-in leaves every existing caller on it,
and refuse-only misses unrecognised states. Refinement (1)'s "unsent draft" conflicts with (5) for M5 and is
question 5; the supervisor corrected it in its decision on question 2: an unsent draft does not refuse.

*Earlier record, superseded:* withdrawn by this interview, inferred (a), with an accepted-or-unverified success field.

### 2. When someone has typed unsent text in the agent's input box and a prompt arrives, should Herdr refuse, submit and say a draft was merged, or keep today's behaviour?

Options: (a) submit, report success, and flag the merged draft; (b) refuse as not ready; (c) keep today's behaviour.

**Answer:** Option (b), refuse as not ready, decided by the herdr-fast-api project supervisor on 2026-10-27 in its
decision on question 5 (Portal follow-up 5bdc9215), inferred under G61; not Fitz's own decision. When a person's real
typed text is in the box, nothing is typed and the call answers `agent_not_ready` naming the unsent text; suggestions,
placeholder text and predicted replies never count, and an agent without an evidence-based rule is not refused. The
supervisor stated this replaces its earlier answer below.

*Superseded, supervisor's earlier answer (follow-up 42545d7d):* Option (a), decided by the herdr-fast-api project supervisor on 2026-10-27 (Portal follow-up 42545d7d),
inferred under G61; not Fitz's own decision. Submit, report success, and add an optional marker in the success answer
saying unsent text was in the box and was submitted with the message; older callers ignore it. An unsent draft does
not refuse; only states where the text would be dropped or rejected (open picker, partial slash command) refuse before
typing. This corrects the supervisor's earlier listing of "unsent draft" among refusing states in its decision on
question 1. Supervisor's reason: the agent does receive the message, K-322 revision 1 allows Portal's messages to be
submitted with unsent human text, refusing would make every automated sender fail whenever a person has text in the
box, and the marker makes the merge visible to callers that care.

*Earlier record, superseded:* withdrawn by this interview, Inferred: (a), with the marker set only when Herdr can tell real
draft text from placeholder text. Reason: the request asks to keep existing draft protections (Herdr has none) and to
report the milder case truthfully; refusing would break Portal's protocol-update route, which K-322 revision 1 lets
submit with unsent text.

*Cost recorded at the requester's request (exchange f52a9cdc, 2026-10-27), not reopening the choice:* when a person
has text typed and it is submitted with a Portal message, the agent reads the person's unfinished words joined to
Portal's text, Portal records a successful submission of text it cannot reproduce, and the person's draft becomes a
sentence they did not send. Agent input lines can also hold predicted replies, so some submitted text may never have
been anyone's words. The requester stated a preference for refusal but left the decision to this task.

### 3. Should the input-box checks cover only Codex in this task, or also other agents that have pickers and slash commands, such as Claude Code?

Options: (a) Codex rules now, other agents use the generic check only; (b) Codex and Claude Code now; (c) every agent
Herdr supports.

**Answer:** Option (a), decided by the herdr-fast-api project supervisor on 2026-10-27 (Portal follow-up c199eb74),
inferred under G61; not Fitz's own decision. Codex's busy-input-box rules are added now, built from captured screens
and live-tested as the repository's evidence-based detection rule requires. Every agent gets the after-typing
acceptance check. Other agents, Claude Code included, get their own up-front rules in a later task only once a dropped
or rejected prompt is reproduced for them. Supervisor's reason: only the Codex case is demonstrated, and the generic
acceptance check already gives every agent a truthful unknown instead of a false success.

*Earlier record, superseded:* withdrawn by this interview, inferred (a). Reason: the request is bounded to the
demonstrated Codex cases, and repository rules require live evidence for each agent's screen rules.

### 4. Who runs Portal's live supervisor delivery check that this task requires, and must it pass before the Herdr change merges?

Options: (a) Herdr implementer replays the raw probes before merge, and a Portal task runs Portal's check after merge;
(b) Herdr implementer runs Portal's check before merge; (c) a Portal task runs Portal's check before merge.

*Correction from the requester (exchange f52a9cdc):* a Portal task could run the check before merge against a
server built from the branch, but it would need its own Portal task; both sides prefer the post-merge plan below, so
the sequence is unchanged.

**Answer:** Option (a), decided by the herdr-fast-api project supervisor on 2026-10-27 (Portal follow-up d03be8ed),
inferred under G61 and Fitz's 2026-10-27 rule that requested work merges into main and is then coordinated with the
requester; not Fitz's own decision. The Herdr implementer replays T-100's raw agent.prompt probes, including the
picker, partial slash command, draft merge, idle, working and back-to-back cases, in an isolated Herdr server built
from the branch with a real Codex agent, never the live Herdr, and puts the results in the pull request as the
pre-merge evidence. After merge, the supervisor sends Portal a project request naming the Herdr commit and version so
a Portal task runs `execution.live_supervisor_delivery`; the result is recorded against this task when it arrives.
Supervisor's reason: Portal's check needs a Portal build, a test database and Codex credentials that a Herdr run should
not carry, and waiting on another project before merge would stall this task.

*Earlier record, superseded:* withdrawn by this interview, inferred (a). Reason: the requester wrote on 2026-10-27 that the
check needs Portal's own environment and that "whoever runs it will be a later Portal task", verifying "when you have
a released change"; the Portal introduction delegates how to test to the task sides.

### 5. The supervisor's outcome decision refuses when unsent text is in the agent's input box, but the required checks say the "draft present" case (M5) must still succeed. Which wins?

Options: (a) refuse on real unsent text, and change M5's expected result to "refused, nothing typed, delivered once
the text is cleared"; (b) submit with the unsent text and mark it (M5 stays success); (c) refuse only when the
unsent text would change the message's meaning, such as a slash command. Recommendation: (a). Sent through Portal as
`t3-draft-refusal-vs-m5-check`.

**Answer:** Option (a), decided by the herdr-fast-api project supervisor on 2026-10-27 (Portal follow-up 5bdc9215),
inferred under G61; not Fitz's own decision. When a person's real typed text is in the agent's input box, nothing is
typed and the call answers `agent_not_ready` with a message saying unsent typed text is present in the agent's input box (the message does not echo the text itself). Suggestions, placeholder text and
predicted replies never count as a draft; where Herdr has no evidence-based rule to tell real typed text apart (any
agent without such a rule), it does not refuse and the after-typing acceptance check applies. M5's required check
becomes: refused with nothing typed, then delivered once the text is cleared. Supervisor's reasons: T-100 prefers
refusal (exchange f52a9cdc); Portal requeues an `agent_not_ready` message with a visible reason and sends it once the
agent waits for input, with no duplicate; K-322 permits but does not require submitting with unsent text; and joining
a person's unfinished words to an automated message sends words the person never submitted, which Fitz's rules treat
as never an instruction. The supervisor stated this replaces its answer on question 2 and that it tells T-100 about
the changed M5 check.

*Superseded:* this interview withdrew the question as settled by the supervisor's earlier answer on question 2
(follow-up 42545d7d: submit with a marker; M5 stays a success).

## Appendix A - Original Request (Verbatim)

Portal task T-3 original request:

```text
Truthful agent.prompt outcome when the agent's composer drops or rejects the text

agent.prompt and the CLI submission (herdr agent prompt) answer ok:true once the text is written to the terminal, even when the agent never receives it. Portal T-100 reproduced two cases with a real Codex agent in an isolated server: (a) Codex's /model picker is open, so the pasted text is dropped and Enter selects a model; (b) a partial slash command such as /mo is already in the composer, so the merged text becomes an unrecognized command and stays unsent. In both, agent status stays idle, state_change_seq does not change and the native session records no user turn. A milder case: unsent draft text merges with the message. Make the call itself report a truthful outcome for these cases: refuse with a not-ready style answer, or return an explicit not-submitted or unknown outcome, so a caller can tell a written message from an accepted one. Keep the existing recipient, authority and draft protections. No new API, no provider-native transport, no change to unrelated callers' contracts. Follow the repository AGENTS.md endpoint contract (advertise new behaviour; do not change the meaning of an advertised method without a capability). Base on main and open the PR against main. Required checks: Portal's execution.live_supervisor_delivery check, in an isolated session, shows the /model picker case and the partial slash command case reported as not submitted or unknown; the six conditions that already deliver still report success and reach the native session. Never test against Fitz's live Herdr. The person's exact action in the original incident is unproven; the mechanism is demonstrated. Source request: Portal T-100 (project 4fd11914-50e8-4f5c-97a2-51506fadfae3, task 6ad7ef82-dd9e-4a48-b43f-ee83b8225164).
```

Referenced material (outside the quote): Portal T-100 evidence folder
`/home/programmer/.local/state/portal/runs/c17abbf9-6954-4833-bca7-e8d242917219/evidence/`, in particular
`t100-delivery-report.md`, `t100-agent-prompt-probes.jsonl`, `t100-live-delivery-result.json`,
`t100-native-rollout.jsonl` and `t100-supervisor-pane-screen.txt`; the requester's project request
`t100-herdr-agent-prompt-truthful-outcome`, its Portal introduction (follow-up 6f3bdc80) and its 2026-10-27 note
(exchange 0e91496f, follow-up e5fd3d64) in this run's `exchanges.json`; Portal `origin/main` `279d78a71`
`src/herdr/HerdrExecutionBackend.cpp` and `docs/native-submission-outcomes.md`.
