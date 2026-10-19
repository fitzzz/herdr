# Restored agent launch options for Claude and Codex

Status: interview complete on 2026-10-19; all five questions answered by a person (K-366, K-367, K-368, K-369, K-370).

## Implementation Log

| Field | Value |
| --- | --- |
| Started | 2026-10-19 14:00:15 EDT |
| Agent | claude |
| Model | claude-opus-5-5 |
| Reasoning Level | unknown |

### Verification Evidence

Evidence paths starting with `w3/`, `w4/`, `w5/`, `logs/` or `artifacts/` are in
`~/.local/share/agent-work/herdr-fast-api-t2/` (outside the repository).

| ID | Implementation | Automated Tests | Interaction Run |
| --- | --- | --- | --- |
| FR-1 | src/agent_resume.rs, src/persist/restore.rs | agent_resume::tests::claude_bypass_launch_options_are_kept_on_restore; persist::restore::tests::pane_restore_startup_resumes_with_saved_launch_options | isolated proof `w3/PROOF-AFTER-FIX.md` case A: Claude 2.1.300 launched with `--model claude-haiku-5-5 --effort low --permission-mode bypassPermissions` came back after two server restarts as `claude <same options> --resume <same id>` showing `bypass permissions on` |
| FR-2 | src/agent_resume.rs, src/persist/restore.rs | agent_resume::tests::codex_bypass_launch_options_are_kept_on_restore; persist::restore::tests::pane_restore_startup_resumes_with_saved_launch_options | isolated proof `w3/PROOF-AFTER-FIX.md` case B: Codex 0.162.1 launched with `--no-daemon -c model_reasoning_effort=low --dangerously-bypass-approvals-and-sandbox` came back after two restarts as `codex resume <same id> <same options>` showing `YOLO mode` |
| FR-3 | src/agent_resume.rs | agent_resume::tests::claude_bypass_launch_options_are_kept_on_restore; agent_resume::tests::codex_bypass_launch_options_are_kept_on_restore | isolated proof `w3/PROOF-AFTER-FIX.md` cases A and B: both restored processes carry their bypass option and the agents show bypass and YOLO mode |
| FR-4 | src/agent_resume.rs | agent_resume::tests::restrictive_launch_options_are_kept_on_restore | isolated proof `w3/PROOF-AFTER-FIX.md` case C came back with `--permission-mode plan`, showed plan mode and refused a write outside its folder; case D came back with `-s read-only -a on-request` and its outside write needed an approval escalation (approved by the user's own Codex `auto_review` setting, so no refusal was seen); isolated proof `w4/PROOF2-AFTER-FIX.md` case F came back with `-s read-only -a on-request` |
| FR-5 | src/agent_resume.rs | agent_resume::tests::launch_without_permission_options_restores_without_them | isolated proof `w4/PROOF2-AFTER-FIX.md` case G: a Claude agent launched with model and effort but no permission option came back with only those options and in auto mode; isolated proof `w3/PROOF-AFTER-FIX.md` case E (fallback) came back bare with no permission option |
| FR-6 | src/agent_resume.rs | agent_resume::tests::model_and_effort_launch_options_are_kept_on_restore | isolated proof `w3/PROOF-AFTER-FIX.md`: A and C show `Haiku 5.5 with low effort` after restore, B and D show effort `low` |
| FR-7 | src/agent_resume.rs | agent_resume::tests::other_claude_launch_options_are_kept_exactly; agent_resume::tests::other_codex_launch_options_are_kept_exactly | isolated proof `w3/PROOF-AFTER-FIX.md` cases B and D: `--no-daemon` and `-c model_reasoning_effort=low` came back on the restored Codex processes |
| FR-8 | src/agent_resume.rs | agent_resume::tests::claude_conversation_prompt_and_print_parts_are_dropped; claude_prompt_after_variadic_option_is_one_of_its_values; codex_conversation_prompt_and_image_parts_are_dropped; unknown_launch_option_makes_options_unavailable; launch_subcommands_make_options_unavailable | isolated proof `w4/PROOF2-AFTER-FIX.md` case F: an agent started with `... resume <id>` came back as `codex resume <id> <options>` with no second `resume`; dropping one-time prompts and print mode is shown by the automated tests only |
| FR-9 | src/pane.rs, src/detect/mod.rs, src/events.rs, src/app/actions.rs, src/agent_resume.rs | pane::tests::process_probe_carries_the_identified_agent_argv; pane::tests::agent_launch_observation_publishes_each_claude_or_codex_process_once; app::actions::tests::agent_launch_observed_event_records_options_from_the_command_line; agent_resume::tests::launch_options_from_process_argv_handles_install_shapes | isolated proof `w5/PROOF3-AFTER-FIX.md` (build `6c030b3d`) cases J1 and J2: Claude and Codex typed by hand at the shell (not `agent start`) were recorded with their options and came back with them after two restarts |
| FR-10 | src/agent_resume.rs, src/app/agent_resume.rs, src/persist/restore.rs, src/terminal/state.rs, src/persist/snapshot.rs, src/ui/panes.rs | agent_resume::tests::launch_options_exceeding_restore_limits_use_the_bare_command; unreadable_command_line_makes_options_unavailable; app::agent_resume::tests::launch_options_notice_explains_each_fallback_and_skips_kept_options; pending_agent_resume_writes_launch_options_notice_before_resume; terminal::state::tests::seeded_agent_launch_binds_to_the_first_observed_process; terminal::state::tests::seeded_agent_launch_survives_the_bare_resume_process; persist::restore::tests::restored_agent_launch_keeps_unknown_options_unknown_after_fallback; terminal::state::tests::resumed_without_launch_options_lasts_while_that_agent_runs; ui::panes::tests::pane_border_title_marks_agents_resumed_without_launch_options; persist::restore::tests::live_handoff_keeps_the_mark_of_a_resume_without_launch_options | isolated proof `w7/MARKER-PROOF.md` (build `098980f5`, sha256 `7180171e...16b6d5`): after a server restart the fallback panes E (apostrophe, ExceedsResumeLimits), H (Claude, NotRecorded) and K (Codex, NotRecorded) came back bare with the top border `┌ launch options not kept ──┐`; the kept panes A and B showed no marker; the marker stayed through a live handoff with the same PIDs and cleared when E's Claude exited |
| FR-11 | src/agent_resume.rs, src/app/agents.rs | agent_resume::tests::codex_launch_with_options_records_its_session; codex_noncanonical_resume_launch_has_no_explicit_session; app::tests::agent_start_records_codex_launch_options | isolated proof `w4/PROOF2-AFTER-FIX.md` case F: `agent start -- --no-daemon -s read-only -a on-request resume <id>` returned the session id at launch, the saved session carried the options, and restore kept both |
| FR-12 | src/persist/snapshot.rs, src/agent_resume.rs | persist::snapshot::tests::capture_saves_known_launch_options_for_the_session_agent; agent_session_launch_options_round_trip_and_old_json_loads; agent_resume::tests::other_claude_launch_options_are_kept_exactly | isolated proof `w3/PROOF-AFTER-FIX.md`: `-c model_reasoning_effort=low` and the placeholder `--append-system-prompt` text were saved as-is in `session.json`; the `-c` value came back exactly |
| FR-13 | src/terminal/state.rs, src/persist/restore.rs, src/app/actions.rs | persist::restore::tests::native_agent_restore_seeds_saved_launch_options_for_the_next_save; live_handoff_keeps_agent_launch_options; app::actions::tests::launch_option_changes_mark_the_session_dirty | isolated proof `w3/PROOF-AFTER-FIX.md` step 3: a live handoff kept every agent's process id and command line and the next save still carried the options; restart 1 and 2 kept them |
| FR-14 | src/terminal/state.rs, src/persist/snapshot.rs | terminal::state::tests::agent_launch_observation_replaces_records_for_new_processes; agent_launch_is_cleared_when_the_agent_exits_or_changes; persist::snapshot::tests::capture_saves_no_launch_options_without_a_matching_known_record; terminal::state::tests::kept_or_started_agent_launch_takes_an_empty_observation | isolated proof `w4/PROOF2-AFTER-FIX.md` case G: after `/exit` the pane had no saved session; a new Claude launched without a permission option was saved and restored without one, in auto mode. The record of an agent started with `agent start` is handled the same way in `6c030b3d` (only restore fallbacks keep their record, test `kept_or_started_agent_launch_takes_an_empty_observation`) |
| FR-15 | src/persist/snapshot.rs, src/agent_resume.rs, src/persist/restore.rs, src/ui/panes.rs | persist::snapshot::tests::agent_session_launch_options_round_trip_and_old_json_loads; persist::restore::tests::pane_restore_startup_without_saved_launch_options_uses_bare_command; agent_resume::tests::unrecorded_launch_options_use_the_bare_command_with_notice; persist::restore::tests::native_agent_restore_without_saved_options_keeps_them_unknown; persist::snapshot::tests::agent_session_resumed_without_launch_options_round_trips_and_defaults | isolated proof `w7/MARKER-PROOF.md`: panes H and K, whose saved `launch_options` were removed as in a session file from an older Herdr, came back as `claude --resume <id>` and `codex resume <id>` with the border marker after two restarts |
| FR-16 | src/agent_resume.rs, src/persist/restore.rs | agent_resume::tests::launch_options_do_not_change_the_dedupe_key; persist::restore::tests::pane_restore_startup_with_launch_options_respects_disabled_resume_and_dedupe | N/A: this change does not alter how duplicate panes are resolved; the deterministic restore tests cover it |
| FR-17 | src/persist/restore.rs | persist::restore::tests::pane_restore_startup_with_launch_options_respects_disabled_resume_and_dedupe | N/A: no new surface; the setting is read where it was before and the deterministic restore test covers it |
| FR-18 | src/agent_resume.rs, src/persist/restore.rs | agent_resume::tests::other_agents_restore_is_unchanged_by_launch_options; persist::restore::tests::pane_restore_startup_reported_resume_ignores_saved_launch_options | isolated proof `w5/PROOF3-AFTER-FIX.md` (build `6c030b3d`) case P: pi came back as `pi --session <same path>` with nothing added (pi had no model login, so its conversation did not complete); third-party reported resume commands are shown by the automated tests only |
| FR-19 | reproduction before the fix (no code) | N/A: a process requirement; the deterministic tests are FR-23 | Baseline before the fix: `w3/REPRO-BASELINE.md` (Claude bypass came back in auto mode, Codex bypass lost YOLO mode); the same two cases after the fix: isolated proof `w3/PROOF-AFTER-FIX.md` cases A and B |
| FR-20 | evidence process (no code) | N/A: a process requirement | isolated proof `w3/PROOF-AFTER-FIX.md` and isolated proof `w4/PROOF2-AFTER-FIX.md` used only their own debug-build sessions under `~/.config/herdr-dev/`, stopped and deleted them, and touched no other session or agent |
| FR-21 | docs/next/website/src/content/docs/session-state.mdx (and ja, zh-cn) | just docs-contract-test and scripts.test_docs_translation_parity passed | N/A: documentation only; checked by the docs contract and translation parity recipes |
| FR-22 | src/agent_resume.rs, src/terminal/state.rs | agent_resume::tests::restored_command_line_yields_the_original_launch_options; persist::restore::tests::native_agent_restore_seeds_saved_launch_options_for_the_next_save; persist::restore::tests::restored_agent_launch_keeps_unknown_options_unknown_after_fallback | `w3/PROOF-AFTER-FIX.md` step 6: A to D still carried their options after a second restart; isolated proof `w5/PROOF3-AFTER-FIX.md` (build `6c030b3d`): kept options (A, B, J1, J2) and fallbacks (E, H, K) were both unchanged after a second restart |
| FR-23 | src/agent_resume.rs | agent_resume::tests (29 tests) cover bypass, restrictive, no-permission, model and effort restore commands | N/A: this row is the deterministic test suite itself |
| FR-24 | src/agent_resume.rs, src/persist/restore.rs | covered by the FR-1 to FR-3 tests | isolated proof `w3/PROOF-AFTER-FIX.md` cases A (Claude) and B (Codex) after restart: same session id, bypass option on the restored process, bypass and YOLO mode shown, and each read `/var/tmp/t2-proof-20261019-143633/outside/fixture.txt` and wrote beside it with no permission prompt |
| FR-25 | branch `portal/t2-4445c337` on `fitzzz/herdr`, code at `6c030b3d` (later commits change only this PRD); `artifacts/herdr-t2-debug-6c030b3d` (sha256 in `artifacts/SHA256SUMS`) and `artifacts/CONSUME.md` | N/A: a delivery requirement | N/A: checked in the pull request and the delivery report; the artifact itself was used by W5 (FR-27) |
| FR-26 | known-limit list in the Change Log entry of 2026-10-19 15:21:50 EDT, repeated in the pull request and the delivery report | N/A: a delivery requirement | N/A: a report, not a user surface |
| FR-27 | `artifacts/herdr-t2-debug-e219b6d6`, `artifacts/CONSUME.md` | N/A: a delivery requirement | isolated proof `w3/PROOF-AFTER-FIX.md` and isolated proof `w4/PROOF2-AFTER-FIX.md` ran the artifact as `artifacts/CONSUME.md` describes, in their own `herdr-dev` sessions; the installed Herdr server and its agents were not touched |
| FR-28 | evidence format (no code) | N/A: a process requirement | isolated proof `w3/PROOF-AFTER-FIX.md` and isolated proof `w4/PROOF2-AFTER-FIX.md` record the process path version, the transcript version and the shell `--version` separately (Claude 2.1.300 and Codex 0.162.1 from all three) |
| FR-29 | src/app/agent_resume.rs, src/persist/restore.rs, src/terminal/state.rs, src/persist/snapshot.rs, src/ui/panes.rs | app::agent_resume::tests::launch_options_notice_explains_each_fallback_and_skips_kept_options; persist::restore::tests::pane_restore_startup_resumes_with_saved_launch_options; pane_restore_startup_without_saved_launch_options_uses_bare_command; terminal::state::tests::seeded_agent_launch_survives_the_bare_resume_process; persist::restore::tests::restored_agent_launch_keeps_unknown_options_unknown_after_fallback; persist::snapshot::tests::capture_saves_resumed_without_launch_options_for_the_session_agent; persist::restore::tests::restored_agent_launch_keeps_the_mark_on_a_handoff_only | isolated proof `w7/MARKER-PROOF.md`: per pane the border marker, `resumed_without_launch_options: true` in `session.json` and the server log (`agent resume without launch options ... reason=`) mark the fallback panes E, H and K; the kept panes A and B carry `launch_options`, no marker and `agent resume keeps launch options` |
| FR-30 | src/persist/restore.rs (logs counts and reasons only), test fixtures | All new tests use placeholder values; restore logs carry option_count or reason, never values | N/A: no user surface; checked by review of fixtures, logs and evidence files |

### Change Log

- 2026-10-19, between 14:00 and 14:13 EDT (exact time not taken from the system clock) - Implementation design (agent proposal, within K-366 and K-367; inferred, not a person's
  decision): launch options come from the agent process's command line, read by the existing background detection
  probe when it identifies a Claude or Codex process, and are kept per pane beside the conversation id in the saved
  session. An option that Herdr cannot classify makes the options "cannot carry" (FR-10 fallback with notice), so a
  one-time prompt is never replayed by mistake. Codex `-i/--image` is treated as part of the one-time prompt (FR-8).
- 2026-10-19, between 14:00 and 14:13 EDT (exact time not taken from the system clock) - Blocker: this machine has no Zig 0.16.0, `just`, `cargo-nextest` or rustup, so Herdr
  cannot be compiled or tested here; Portal Builders are disabled for this project. Asked Portal as question
  `t2-local-build-toolchain`.
- 2026-10-19 14:13:08 EDT - FR-19 baseline reproduced before the fix, in the owned disposable Herdr session
  `repro-t2-20261019-140913` with the installed Herdr `0.9.3-fast-api.6e51c8cb-gpu-bulk-direct`. Claude 2.1.300 (process
  path and transcript) launched with `--model claude-haiku-5-5 --effort low --permission-mode bypassPermissions` came
  back as `claude --resume <same id>` and showed auto mode. Codex 0.162.1 (process path and transcript) launched with
  `--no-daemon -c model_reasoning_effort=low --dangerously-bypass-approvals-and-sandbox` came back as
  `codex resume <same id>`, lost YOLO mode, and its sandbox changed from full access to workspace-write. Observation:
  reading a file outside the working directory did not prompt in either restored case, so an outside read alone does not
  tell bypass from these defaults; the shown mode and the transcript permission values do. Evidence:
  `~/.local/share/agent-work/herdr-fast-api-t2/w3/REPRO-BASELINE.md`.
- 2026-10-19 14:19:53 EDT - Question `t2-local-build-toolchain` (Q-375) was withdrawn by the herdr-fast-api supervisor as
  settled by existing delegated authority (agreement 949c38db; rules G3, G24, G43), not answered by a person. Private
  tools were installed only in `~/.local/share/agent-work/herdr-fast-api-t2/tools`, with the pins of
  `scripts/fast_api/remote_baseline.sh`: rustup 1.29.1 and Rust 1.96.1, Zig 0.16.0, just 1.58.0, cargo-nextest
  0.9.148. Sources, versions and checksums: `tools/TOOLCHAIN.txt` in that folder.
- 2026-10-19 14:37:35 EDT - Combined branch at `e219b6d6` (parser `fc0c319f`, wiring `0ac0feeb`, docs `3f449c73` and
  `e219b6d6`) with Rust 1.96.1: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` pass; the 52 new
  and related tests pass; full `cargo nextest run --no-fail-fast` ran 3958 tests, 3956 passed and 2 failed; the
  maintenance, UI hot-path architecture, integration asset and docs contract recipes pass. The two failures are
  existing ones, not caused by this change: the same two tests fail in two full runs of the base `4b974866` (3917 of 3919
  passed). `machine_api_recovers_a_stale_path_before_sending_a_mutation` fails every time on both;
  `observational_reads_preserve_queued_notifications_and_event_sequence` passes 5 of 5 alone and fails under the full
  parallel run on both. Logs: `~/.local/share/agent-work/herdr-fast-api-t2/logs/`.
- 2026-10-19 15:01:57 EDT - Defect found by isolated proof W4 (`w4/PROOF2-AFTER-FIX.md`, case H) on `e219b6d6`: after
  a restore that fell back to the bare command, detection read Herdr's own bare resume process and the next save wrote
  `"launch_options": []`, so a second restore would count as keeping the options (no notice), and options saved but
  too long for the restore command were lost after one restore. This breaks FR-10, FR-15, FR-22 and FR-29 (T-79
  exchange t79-w5-mode-proof-ack-20261019: unknown must stay distinct from a known empty list across later restores).
  Fixed in `6c030b3d`: a fallback restore seeds the pane's record with the options as unknown (or the saved options
  that did not fit) and keeps them when the bound process shows no options; a seed from a restore that kept its options
  or from `herdr agent start` is still replaced, so a bare agent typed later never inherits old options (FR-5, FR-14).
  Interaction runs for FR-10, FR-14, FR-15 and FR-29 wait on isolated proof W5 with the rebuilt build.
- 2026-10-19 15:01:57 EDT - Finding from isolated proof W3 (`w3/PROOF-AFTER-FIX.md`, case E): Claude clears the screen
  and the scrollback when it starts, so the yellow `Herdr:` notice written before the resume command is gone once
  Claude has drawn (the Claude 2.1.300 binary contains the clear-screen-and-scrollback sequence). The fallback is still
  in the server log (`agent resume without launch options`, with the reason) and the saved session. Where the notice
  should appear is asked in Portal as question `t2-fallback-notice-placement` (agent proposal: show it beside the pane
  until the agent exits; pending, not decided).
- 2026-10-19 15:21:50 EDT - Isolated proof W5 (`w5/PROOF3-AFTER-FIX.md`, build `6c030b3d`, sha256 `e3cb5f67...a2bb7`)
  verified the fallback fix over two restarts, agents typed by hand (FR-9) and pi restore (FR-18). `6c030b3d` with
  Rust 1.96.1: fmt and clippy clean; full suite 3962 tests, 3960 passed, and the same 2 failures that fail on the base
  (`logs/check-6c030b3d.log`). The `Herdr:` notice was not visible for the Codex fallback (K) either, so FR-10, FR-15
  and FR-29 keep Interaction Testing at No until question `t2-fallback-notice-placement` is decided and built.
- 2026-10-19 15:21:50 EDT - Known limits for the delivery report (FR-26), flagged as observations, not decisions:
  (1) the fallback notice is not visible once Claude or Codex has drawn (open question above); (2) WSL is not tested
  live, only by automated tests (Herdr does not read the command line there, so it falls back by design); (3) the
  restore command uses POSIX single quotes, so under fish a `\\` inside an option value would become `\` and under
  csh or tcsh a `!` could trigger history expansion (not tested live); (4) only known options are saved, so a pane
  whose options were unavailable for another reason is reported as `NotRecorded` after a restart; (5) observed with
  Codex 0.162.1: `codex resume <id>` with launch options came back on the user's config model (`gpt-6-sol`) instead of
  the conversation's model (`gpt-6.1-sol`), while a bare resume kept it; Herdr adds no model option and the cause in
  Codex was not established; (6) with the user's Codex `approvals_reviewer = "auto_review"`, a restored read-only Codex
  wrote outside its folder after an escalation was approved automatically: read-only was kept (transcript
  `sandbox_policy` read-only) but no refusal was seen; (7) dropping one-time prompts and print mode, and third-party
  reported resume commands, are shown by automated tests only; (8) only Claude 2.1.300 and Codex 0.162.1 were tested
  live; (9) two existing test failures on the base remain.
- 2026-10-19 15:36:02 EDT - Changed FR-10 (decision K-377 r1, a person's decision answering Q-378
  `t2-fallback-notice-placement`): the not-kept notice is shown beside the pane, on its title and its row in the agents
  list, until that agent exits. Reset FR-10 to No in all three columns, and FR-15 and FR-29 (which rely on that notice)
  to No in all three. Design (agent proposal within K-377): one flag on the pane's launch record set by a fallback
  restore, kept while that agent runs and saved in the session file so a live handoff keeps both the marker and the
  unknown-options rule; the yellow `Herdr:` line stays. Assigned to W2 (`BRIEF-W2-marker.md`).
- 2026-10-19 15:39:07 EDT - Codex model change (known limit 5) diagnosed with two bounded tests on Codex 0.162.1 outside Herdr
  (`w6/CODEX-MODEL.md`, `w6/CODEX-MODEL-2.md`). Conversations started with `-m gpt-6.1-sol` while the user's config
  says `gpt-6-sol` kept `gpt-6.1-sol` on `codex resume <id>`, `codex resume <id> --no-daemon` and
  `codex resume <id> -s read-only`, but came back on the config model with `codex resume <id> -c
  model_reasoning_effort=low`. So Codex itself takes the model from its config, not from the conversation, when a
  resume carries a `-c` override; Herdr repeats the launch options exactly and adds no model option. FR-6 (the model
  and effort options an agent was launched with) is met: an agent launched with `-m` gets it back. Not established: why
  the W5 conversations, started without `-m`, ran on `gpt-6.1-sol` while later ones ran on `gpt-6-sol`. No code change
  made; adding the conversation's own model to the restore command would add an option the agent was not launched
  with, so it is offered as an agent proposal, pending, not built.
- 2026-10-19 15:51:35 EDT - T-79 (exchange t79-model-reconciled-notice-delivery-pending-20261019, the requesting agent's position, not
  a person's decision) does not authorize the proposal to add the conversation's own model to a Codex restore command;
  it stays unbuilt and is not a condition of this work. The Codex `-c` model behavior and the unexplained initial
  `gpt-6.1-sol` selection in W5 are recorded as known limits beside T-79's passing explicit-option check of `6c030b3d`.
  That check is pinned to the `6c030b3d` artifact and does not carry over to `098980f5`.
- 2026-10-19 15:56:36 EDT - Built and verified K-377 r1 (commit `098980f5`, worker W2). A pane resumed with the bare command shows
  `launch options not kept` in its top border title (after its label when it has one) until that agent exits or another
  process replaces it; the flag is saved as `resumed_without_launch_options` beside the session so a live handoff keeps
  the marker and the unknown-options rule. The agents list was left out: the TUI client draws it from the frozen
  endpoint snapshot codec, which this repository forbids changing, and K-377 asks only for the notice beside the pane,
  which the server-drawn border title meets (agent decision within K-377). Checks at `098980f5`: fmt and clippy clean,
  UI hot-path architecture test OK, full suite 3968 tests with 3966 passed and the same 2 base failures
  (`logs/check-098980f5.log`); the 11 marker and fallback tests pass (`logs/marker-tests-098980f5.log`). Render scale
  benchmark, 15 panes, full redraw median: 502 and 583 us with the marker, 576 and 581 us without, no measurable change
  (`logs/bench-render-*.log`). Isolated proof `w7/MARKER-PROOF.md` matched every step. FR-10, FR-15 and FR-29 set to Yes.

## Introduction/Overview

When the Herdr server restarts, it reopens each Claude Code and Codex conversation by typing a resume command into the
pane. That command is fixed: `claude --resume <id>` or `codex resume <id>`. The options the agent was launched with
(permission mode, model, effort, extra directories, allowed tools and so on) are not kept, so the conversation comes
back with the same native session id but under the user's default settings. Portal's T-79 request reproduced this: a
Claude 2.1.300 agent started with explicit bypass came back without any permission option and entered auto mode.

The goal is that a restored Claude or Codex agent comes back with the same native session id and the same launch
options it was started with, per agent: a bypass launch stays bypass, a restrictive launch stays restrictive, and no
agent gains bypass it did not start with.

Work happens on the local fast-api line at commit `4b974866829f7c60cde4438e5f3ca634ae4bf629` (local branch `fast-api`
in `/home/programmer/Work/Repos/herdr-fast-api`, whose `origin` is `fitzzz/herdr`; this worktree's branch
`portal/t2-4445c337` starts there). As of 2026-10-19 the `fast-api` branch is not on `origin` (`git ls-remote origin
fast-api` returns nothing). The installed Herdr, `0.9.3-fast-api.6e51c8cb-gpu-bulk-direct`, is built from `6e51c8cb`,
which contains `4b974866` plus 55 later commits (`git merge-base --is-ancestor 4b974866 6e51c8cb` succeeds; `4b974866` has no commits that `6e51c8cb` lacks).

### Who asked and what they will check

Portal task T-79 asked for this through project request `38e16beb-3233-55d9-bef3-339222dd8736` (Portal decision K-288
revision 2 routes it to herdr-fast-api, fork `fitzzz/herdr`, not herdr-annotations). T-79's validation stage waits on
this code. Its stated expectations, kept in full:

- Return the responsible work with its exact selected branch, remote and source commit in `fitzzz/herdr`.
- Deliver reviewed repair code and a usable isolated-test artifact or a documented supported way to consume it.
- Include deterministic regression evidence, and isolated Claude and Codex restoration proof of original permissions,
  native identity, effective mode, and reading outside directories without answering a tool permission prompt.
- State any unavailable capability explicitly.
- Preserve the original policy per agent, including restrictive launches; do not enable bypass globally for unrelated
  agents. Preserve model, effort, identity and unrelated options.
- Do not restart or take control of active user agents. Reproduce and verify repairs only in owned isolated sessions.
- No installed `resume_argv` capability or existing-agent lifecycle authority is assumed.

T-79 will verify the branch and commit, run its own isolated Claude and Codex restore checks, and repeat its isolated
restore acceptance from its Portal run after delivery.

### Evidence from T-79

- Isolated reproduction (Herdr `6e51c8cb`, Claude 2.1.300, session `t79r-5a8561ba`): the fresh process had
  `--permission-mode bypassPermissions` and showed bypass; Herdr restored it as `claude --resume
  fe4ef4ab-39ae-4163-849a-6e051825d9c8` with no permission option, and it showed and recorded `auto` mode. The native
  id was the same before and after. Claude's `SessionStart` hook data had no `permission_mode` value on either start,
  so Claude's hook data cannot supply the permission mode in this version.
- Original incident (omarchy-xui supervisor, pane `w5:p1`): the running process was `claude --resume
  7ab90dde-076b-4b21-9873-bbf0ed5be9da --model claude-opus-5-5 --effort high`, with no permission option; its transcript
  moved from `bypassPermissions` to `auto`, and the auto mode classifier then denied actions. Herdr's fixed Claude
  restore command never includes `--model` or `--effort` (checked in both `4b974866` and `6e51c8cb`), so that relaunch
  was not typed by Herdr's fixed restore command. Who typed it is not established, and T-79 confirmed on 2026-10-19
  (exchange `t79-x2-incident-caller-unverified`) that no record identifies the caller. The isolated reproduction above
  is an independent Herdr restore defect; fixing it does not prove what caused the incident or that the incident is
  resolved.
- Portal's own reopen is a separate path that already adds the permission option itself (T-79: `runAgentArgs` calls
  `fullAccessAgentArgs` before adding resume options). T-79's Portal acceptance recorded the actual commands, which also
  show that both CLIs accept launch options together with a resume:
  - Claude 2.1.300 fresh: `claude --model claude-opus-5-5 --effort high --permission-mode bypassPermissions --add-dir
    <dir> --allowedTools <rule>`; reopened: the same options plus `--resume 6f45498e-7085-42ac-888a-4a698dbfb087`.
    Native metadata confirmed `bypassPermissions`.
  - Codex 0.162.1 fresh: `codex -m gpt-6.1-sol -c model_reasoning_effort=medium
    --dangerously-bypass-approvals-and-sandbox --no-daemon --add-dir <dir>`; reopened: `codex resume <the same options>
    --cd <dir> 01a14f9f-55df-78c1-b0fd-9550948a89be`, keeping the `never` approval and `danger-full-access` policies.
  - Both read outside fixtures after reopen without a tool permission prompt being answered.
  - In both, the first word of the actual command is a full install path (for example
    `/home/programmer/.local/share/mise/installs/claude/2.1.300/claude`). Herdr's restore runs only a plain command
    name (`validate_resume_argv`), as it does today.
- Evidence files: `/home/programmer/.local/state/portal/runs/63b645bb-c20e-4b77-9f37-c9ebfdde10aa/` holds
  `t79-restoration-result.json`, `incident-process.json`, `incident-mode-evidence.json` and
  `t79-portal-supervisor-acceptance.json`. Published write-ups:
  https://github.com/fitzzz/portal/blob/0c8bbdb09918e67886beb795e867bb32f046486d/docs/prds/2026-10-18-agent-bypass-permissions/isolated-restoration-diagnosis.md
  and
  https://github.com/fitzzz/portal/blob/4bea89e15bb1e31fb5f1042983a0a13c9eaf8715/docs/prds/2026-10-18-agent-bypass-permissions/portal-supervisor-acceptance.md.

### What exists today and the actual gap

| # | Requested outcome | Existing functionality and evidence | Actual gap |
| --- | --- | --- | --- |
| 1 | A restored Claude conversation keeps its native session id. | Already implemented. Herdr's Claude hook script reports the session id at session start (`src/integration/assets/claude/herdr-agent-state.sh`), and restore runs `claude --resume <id>` (`src/agent_resume.rs`, `plan`). Covered by tests in `src/agent_resume.rs` (around line 465). | None for the id itself. |
| 2 | A restored Codex conversation keeps its native session id. | Already implemented the same way: the Codex hook reports the id and restore runs `codex resume <id>` (`src/agent_resume.rs`, `plan`; test around line 475). | None for the id itself. |
| 3 | A restored Claude or Codex agent keeps its permission mode, model, effort and other launch options. | Missing. `plan` builds the command from the session id alone. Nothing stores the launch options: `herdr agent start` types the arguments into the shell but keeps only a Codex `resume <id>` pair (`src/app/agents.rs`, `start_agent`; `persisted_session_from_launch_args`), and the saved session file has no field for them (`src/persist/snapshot.rs`, `PaneSnapshot`). | This is the work: record each agent's launch options and repeat them in its restore command. |
| 4 | A way for an agent to hand Herdr a full resume command with options. | Partly exists. Any agent integration can report its own resume command after `--` (`pane report-agent-session ... -- <command>`), and restore prefers it over the fixed command (`src/persist/restore.rs`, around line 820). The public docs already tell third-party agents to include model and permission mode there (`docs/next/website/src/content/docs/add-herdr-support.mdx`). Herdr's own Claude and Codex hook scripts never use it. | Possible reuse by the implementer. Herdr's own scripts do not know the launch options, so the options come from the command line (FR-9). |
| 5 | Herdr can see the options an agent was launched with. | Partly exists. Herdr reads a running process's full command line on Linux (`src/platform/linux.rs`, `process_argv`) and Windows, to recognize agents. Inside WSL it skips that read for recognized agents because it can hang. Nothing stores or uses those arguments for restore. | Chosen source of the options (FR-9, K-366). Needs to be stored per pane and used by restore. |
| 6 | Codex agents started with `herdr agent start --kind codex -- resume <id>` are recorded at launch. | Partly implemented: recorded only when the arguments are exactly `resume <id>`; any extra option means nothing is recorded until the hook reports. | In scope (FR-11, K-369). |

Not verified: whether the Claude and Codex versions Herdr actually launches include launch options in the data they give
hook scripts, and how each behaves when its resume command repeats every launch option. The implementer verifies both
in an isolated session. The interview shell's `claude --version` and `codex --version` report 2.1.302 and 0.163.0, but
T-79 found that the shell's version can differ from the runtime a pane actually runs, so those numbers are not proof of
the pane runtime (see FR-28).

## Goals

1. A Claude or Codex agent launched with a permission mode (bypass or restrictive) comes back from restore in that same
   mode, with the same native session id.
2. Model, effort and every other launch option come back unchanged, except the parts that pick a conversation or send
   a one-time prompt.
3. No restored agent gains a bypass or a looser mode than it was launched with.
4. Agents other than Claude and Codex, and third-party reported resume commands, behave exactly as today.

## User Stories

1. As someone who started a Claude agent with bypass, I want it to come back in bypass after Herdr restarts, so an
   unattended job does not stop at permission prompts or switch to auto mode.
2. As someone who started an agent in a restrictive mode, I want it to come back restrictive, so a restart never
   loosens what it may do.
3. As Portal, which launches Claude and Codex workers with model, effort, permission and directory options, I want each
   worker to come back with exactly its own options and the same conversation.
4. As a user with several agents in different modes, I want each one restored with its own options, not one policy
   applied to all.

## Functional Requirements

| ID | Requirement | Source | Implemented | Automated Tests | Interaction Testing |
| --- | --- | --- | --- | --- | --- |
| FR-1 | A restored Claude agent resumes the same native session id with its original launch options. | stated (request) | Yes | Yes | Yes |
| FR-2 | A restored Codex agent resumes the same native session id with its original launch options. | stated (request) | Yes | Yes | Yes |
| FR-3 | An agent launched with bypass (Claude `--dangerously-skip-permissions` or `--permission-mode bypassPermissions`, Codex `--dangerously-bypass-approvals-and-sandbox`) comes back in bypass. | stated (request; T-79) | Yes | Yes | Yes |
| FR-4 | An agent launched with a restrictive permission, approval or sandbox option comes back with that same option. | stated (T-79) | Yes | Yes | Yes |
| FR-5 | Restore never adds a bypass or permission option the agent was not launched with; there is no global bypass setting. | stated (T-79) | Yes | Yes | Yes |
| FR-6 | The model and effort options an agent was launched with come back unchanged. | stated (request) | Yes | Yes | Yes |
| FR-7 | Every other launch option comes back too (for example extra directories, allowed and disallowed tools, MCP and settings files, system-prompt additions, Codex `-c` overrides and `--no-daemon`), except the parts named in FR-8. | stated (request; #2, K-367 r1) | Yes | Yes | Yes |
| FR-8 | Parts of the launch command that pick or start a conversation (for example Claude `--resume`, `--continue`, `--session-id`, `--fork-session`; Codex `resume <id>`, `--last`), a one-time prompt, and print-and-exit mode are not repeated; the saved session id takes their place. The implementer keeps a per-agent list of these parts. | stated (#2, K-367 r1) | Yes | Yes | Yes |
| FR-9 | Herdr learns each agent's launch options by reading the running agent's own command line, which covers agents typed by hand, started with `herdr agent start`, and started by Portal. Where Herdr does not read the command line (inside WSL), FR-10 applies. | stated (#1, K-366 r1) | Yes | Yes | Yes |
| FR-10 | When Herdr cannot carry an agent's launch options (agent running before the updated Herdr recorded them, inside WSL where Herdr does not read the command line, or options that break the restore command limits of 64 arguments, 8 KiB and no apostrophes), restore resumes the conversation with today's bare command and shows a notice that the launch options were not kept, so the user can relaunch it in the right mode. The notice is shown beside the pane, on its title and its row in the agents list, until that agent exits. | stated (#3, K-368 r1; placement K-377 r1) | Yes | Yes | Yes |
| FR-11 | A Codex agent started with `herdr agent start --kind codex -- <options> resume <id>` is recorded at launch with both its conversation id and its options, so a restore right after starting keeps them. | stated (#4, K-369 r1) | Yes | Yes | Yes |
| FR-12 | Launch options that carry inline text or config values (for example inline `--settings` or `--mcp-config` JSON, `--append-system-prompt` text, Codex `-c key=value` overrides) are saved as-is in Herdr's session file and repeated exactly on restore; Herdr does not filter or redact them. | stated (#5, K-370 r1) | Yes | Yes | Yes |
| FR-13 | Each agent's options are kept per pane and survive a server restart and a live handoff. | stated (request) | Yes | Yes | Yes |
| FR-14 | Each agent's options always belong to its current conversation: when the conversation in a pane changes or the agent exits, Herdr does not restore an old conversation with a different agent's options or a new one with stale options. | inferred | Yes | Yes | Yes |
| FR-15 | Session files written before this change still restore, using the bare command and notice from FR-10. | stated (#3, K-368 r1) | Yes | Yes | Yes |
| FR-16 | Two panes holding the same conversation still resume it only once, as today. | inferred | Yes | Yes | N/A: this change does not alter how duplicate panes are resolved; the deterministic restore tests cover it |
| FR-17 | `[session] resume_agents_on_restore = false` still turns off agent resume entirely. | inferred | Yes | Yes | N/A: no new surface; the setting is read where it was before and the deterministic restore test covers it |
| FR-18 | Restore for every other agent, and for third-party reported resume commands, is unchanged. | stated (request scope: Claude and Codex) | Yes | Yes | Yes |
| FR-19 | Before changing code, the implementer reproduces the loss in an isolated Herdr session: Claude started with bypass, restored, and observed without it; and the same for Codex. The fix is verified against those same cases. | rule G12 | Yes | N/A: a process requirement; the deterministic tests are FR-23 | Yes |
| FR-20 | Reproduction and verification use only Herdr sessions and agents the implementer started for this work; no running user agent is restarted, resumed or taken over. | stated (T-79); rule G4 | Yes | N/A: a process requirement | Yes |
| FR-21 | The unreleased docs (`docs/next/website/src/content/docs/session-state.mdx`, resume command table) say that Claude and Codex restore keeps launch options. | inferred | Yes | Yes | N/A: documentation only; checked by the docs contract and translation parity recipes |
| FR-22 | Options survive repeated restores: an agent restored with its options, then restored again, still has them. | inferred | Yes | Yes | Yes |
| FR-23 | Deterministic regression tests cover the restore command Herdr builds for Claude and Codex with bypass, restrictive, no-permission, model and effort launches. | stated (T-79) | Yes | Yes | N/A: this row is the deterministic test suite itself |
| FR-24 | Isolated live proof for Claude and for Codex shows, after a Herdr server restart: the same native session id, the original permission options on the restored process, the effective mode shown by the agent, and that the restored agent reads a directory outside its working directory without a tool permission prompt being answered. | stated (T-79) | Yes | Yes | Yes |
| FR-25 | The delivery names its branch, remote (`fitzzz/herdr`) and exact source commit, and provides an isolated-test build or a documented way for T-79 to build and run it. | stated (T-79) | Yes | N/A: a delivery requirement | N/A: checked in the pull request and the delivery report |
| FR-26 | The delivery report lists any capability that could not be delivered or verified (for example WSL, or a Codex behavior that could not be shown). | stated (T-79) | Yes | N/A: a delivery requirement | N/A: a report, not a user surface |
| FR-27 | The isolated-test build or consumption path lets T-79 run its checks without stopping, restarting or replacing the installed user Herdr server and without touching any active agent (its own server, socket and session directories). | stated (T-79 exchange t79-t2-assignment-source-20261019) | Yes | N/A: a delivery requirement | Yes |
| FR-28 | Every verification record names the agent version from the actual restored process (its executable path and arguments) and from its native transcript, and keeps the shell's `--version` output beside them as a separate source; the shell's output alone never counts as the pane runtime's version. | stated (T-79 exchange t79-x4-producer-scope-consumer-obligations) | Yes | N/A: a process requirement | Yes |
| FR-29 | A restore that fell back to the bare command because the options were unknown (FR-10) is always distinguishable from a restore that used the agent's known launch options: Herdr's notice and state say which happened for each pane, and verification evidence and the delivery report never count a fallback restore as proof that original permissions were kept (FR-3, FR-4). | stated (T-79 exchange t79-x5-current-question-readback) | Yes | Yes | Yes |
| FR-30 | Actual secret values from launch options (tokens, keys, inline credentials) never appear in verification evidence, test fixtures, logs written for review, delivery reports or Portal coordination messages; tests and evidence use placeholder values. This does not change FR-12: the session file still stores options as-is. | stated (T-79 exchange t79-supervisor-status-corrected-all-answered-20261019) | Yes | Yes | N/A: no user surface; checked by review of fixtures, logs and evidence files |

## Rules Applied

- Rule G2: every requirement is labelled stated or inferred; unverified agent behavior is marked not verified.
- Rule G3: this interview produces the PRD only; it does not implement.
- Rule G4: verification may not touch other agents' panes or sessions (FR-20).
- Rule G12: reproduce the failing case and its root cause before changing code (FR-19).

## Naming and Domain Boundaries

- Herdr already uses "resume command" (`resume_argv`, `ReportedAgentResume`) for a full command an agent reports, and
  "resume plan" (`AgentResumePlan`) for what restore runs. "Launch options" is used here for the options an agent was
  started with. Keep these existing names; do not introduce a second word for the resume command.
- Launch options are a shared runtime fact about the pane's agent, so they belong in server state and the saved session,
  not in TUI-only state (repository rule: runtime/client boundary).

## Design Pattern Decisions

No design pattern proposals. The change extends the existing per-agent match in `agent_resume::plan` and the existing
saved-session fields; there is no new family of behaviors for a pattern to organize.

## Response/Execution Order

On restore, for each Claude or Codex pane: resume the saved native session id with the agent's recorded launch options,
in the same pane and directory, using Herdr's existing restore order and spacing between agents.

## Non-Goals (Out of Scope)

- Agents other than Claude Code and Codex.
- Tracking changes made inside a running session (for example switching permission mode with Shift+Tab or `/model`);
  the request is to keep the original launch options.
- A global setting that applies bypass or any permission mode to all restored agents.
- Changing third-party reported resume commands.
- Identifying who relaunched the original omarchy-xui supervisor, or proving that incident resolved.
- Portal's own reopen path, which already adds permission options.
- Changing or restarting the original omarchy-xui supervisor or any other running agent (not authorized).

## Success Metrics

- In an isolated session, a Claude agent started with bypass and a Codex agent started with bypass both come back in
  bypass with the same session id after a Herdr server restart.
- A Claude agent started restrictive, and one started with no permission option, come back without bypass.
- Model and effort shown by the restored agent match the launch.
- The restored bypass agent reads a directory outside its working directory with no permission prompt.
- T-79 repeats its isolated restore acceptance against the delivered build and it passes.

## Assumptions

- Claude 2.1.300 and Codex 0.162.1 accept launch options together with their resume command (`claude <options>
  --resume <id> <options>`, `codex resume <options> <id>`), shown by T-79's Portal acceptance evidence above (versions
  taken from the actual process paths). Newer versions are planned checks, not passing evidence; the interview shell's
  `codex resume --help` (reporting 0.163.0) lists the
  model, config, profile, sandbox, approval, bypass, extra-directory and `--no-daemon` options.
- Inferred: launch options are not exposed through the JSON API or events in this work; they are needed only by restore.
- Inferred, not verified: on some installs the agent's process command line may begin with an interpreter and script
  (for example `node <path>/cli.js`) rather than the agent's own executable; the implementer finds where the agent's
  options start for each supported install shape, and treats an unrecognized shape as "cannot carry" (FR-10).
- Inferred rows that stay inferred because they would not change what is built: FR-14 (options always belong to the
  pane's current conversation), FR-16 (a shared conversation still resumes once), FR-17 (the resume setting still turns
  restore off), FR-21 (unreleased docs updated), FR-22 (options survive repeated restores).

## Open Questions

None.

## Interview Answers

### 1. Where should Herdr learn which options a Claude or Codex agent was launched with, so it can repeat them when it restores the conversation?

Options offered: (a) read the running agent's command line; (b) only agents started with `herdr agent start`; (c) have
Herdr's Claude and Codex hook scripts report the options.

**Answer:** Read the running agent's command line (option a). Recorded in Portal as Q-332, person decision
K-366 revision 1 (decision record `627cf820-565b-4373-b124-e016716d1cc3`).

### 2. Which of the original launch options should go into the restore command?

Options offered: (a) everything except the parts that pick or start a conversation and one-time prompts; (b) a fixed
list only (permission, approval and sandbox mode, model, effort, extra directories, allowed or disallowed tools).

**Answer:** Everything except conversation-picking parts and one-time prompts (option a). Recorded in Portal as Q-333,
person decision K-367 revision 1 (decision record `d8ff8bb2-6ff7-4b3d-b960-012bd1a6c2c1`).

### 3. When Herdr cannot carry an agent's options, what should restore do?

Options offered: (a) resume with the bare command and show a notice; (b) do not resume, leave the pane at its shell with
a notice; (c) resume with the bare command silently, as today.

**Answer:** Resume with the bare command and show a notice (option a). Recorded in Portal as Q-334, person decision
K-368 revision 1 (decision record `cf076f9e-056d-4e23-937c-22c0d2419e84`).

### 4. Should this work also fix Codex agents started with 'herdr agent start' to resume a conversation with extra options?

Options offered: (a) include it; (b) leave it out.

**Answer:** Include it (option a). Recorded in Portal as Q-335, person decision
K-369 revision 1 (decision record `1ffafaac-913b-43c6-b0e0-2be49e51e6ff`).

### 5. Some launch options can carry long text or secrets. Should Herdr save them as-is in its session file so restore can repeat them?

Options offered: (a) save them as-is; (b) do not save inline text or config values (keep file paths and flags, drop
inline JSON, prompt text and `-c` values, and show the notice for options that could not be kept).

**Answer:** Save them as-is (option a). Recorded in Portal as Q-336, person decision K-370 revision 1 (decision record `56213317-898f-4f7d-8a0f-52a72fdd35c5`).

## Appendix A - Original Request (Verbatim)

```text
Restored agent permission options: keep each Claude and Codex agent's launch options on restore

When Herdr restores a Claude Code or Codex conversation, keep that agent's original launch options: permission mode (bypass or restrictive), model, effort and other options, with the same native session id. Today fast-api restores only 'claude --resume <id>' and 'codex resume <id>'. Work on local fast-api (4b974866). Requested by portal T-79 (request 38e16beb-3233-55d9-bef3-339222dd8736).
```
