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
| FR-1 | src/agent_resume.rs, src/persist/restore.rs | agent_resume::tests::claude_bypass_launch_options_are_kept_on_restore; persist::restore::tests::pane_restore_startup_resumes_with_saved_launch_options; terminal::state::tests::launcher_exec_in_the_same_process_records_the_agent_options; app::actions::tests::a_launcher_exec_does_not_pair_a_new_process_with_the_previous_conversation; a_third_process_does_not_pair_with_the_conversation_an_earlier_process_owned; a_new_process_resuming_the_current_conversation_owns_it; terminal::state::tests::session_for_process_decides_every_case; agent_resume::tests::resumed_session_from_process_argv_reads_the_resumed_conversation; app::actions::tests::a_new_process_resuming_the_current_conversation_by_an_attached_id_owns_it; a_forked_process_does_not_own_the_conversation_it_forks | isolated proof `w3/PROOF-AFTER-FIX.md` case A: Claude 2.1.300 launched with `--model claude-haiku-5-5 --effort low --permission-mode bypassPermissions` came back after two server restarts as `claude <same options> --resume <same id>` showing `bypass permissions on`; isolated proof `w11/BYPASS-CODEX-PROOF.md` (build `7449fe52`) case A: Claude started through the python launcher with `--permission-mode bypassPermissions` came back as `claude <same 6 options> --resume <same id>` showing `bypass permissions on` after two restarts; isolated proof `w10b/LAUNCHER-PROOF.md` (build `7449fe52`, sha256 `145693c3...15cf5a`) cases K and H (plan) came back with their options; isolated proofs `w12/ROUND5-PROOF.md` cases S and E and `w12b/CODEX-HANDOFF-PROOF.md` case Y (build `6fe0bf45`): Claude came back as `claude <same options> --resume <same id>` after two restarts; isolated proof `w13/ROUND6-PROOF.md` cases A and Y (build `ff1bc02a`): a Claude resumed with `-r<id>` and one started with `agent start` came back as `claude <same options> --resume <same id>` after two restarts; round 7: With `2fec054b`/`70f63fb7`: `w14/ROUND7-PROOF.md` and `w14b/RESUME-OFF-HANDOFF-PROOF.md`, Claude via `agent start` and typed by hand came back with its options, mode and session id; round 8: `w15` A and G, `w15b` H and P (build `ee3144a8`); round 9: `w16`, `w16b` (build `b0ab1307`) |
| FR-2 | src/agent_resume.rs, src/persist/restore.rs | agent_resume::tests::codex_bypass_launch_options_are_kept_on_restore; persist::restore::tests::pane_restore_startup_resumes_with_saved_launch_options; terminal::state::tests::launcher_exec_in_the_same_process_records_the_agent_options; app::actions::tests::a_launcher_exec_does_not_pair_a_new_process_with_the_previous_conversation; a_third_process_does_not_pair_with_the_conversation_an_earlier_process_owned; a_new_process_resuming_the_current_conversation_owns_it; terminal::state::tests::session_for_process_decides_every_case | isolated proof `w3/PROOF-AFTER-FIX.md` case B: Codex 0.162.1 launched with `--no-daemon -c model_reasoning_effort=low --dangerously-bypass-approvals-and-sandbox` came back after two restarts as `codex resume <same id> <same options>` showing `YOLO mode`; isolated proof `w11/BYPASS-CODEX-PROOF.md` (build `7449fe52`) cases B and D: Codex started through the python launcher came back as `codex resume <same id> <same options>` after two restarts; isolated proof `w12b/CODEX-HANDOFF-PROOF.md` case X (build `6fe0bf45`): Codex 0.163.0 started through the launcher came back as `codex resume <same id> <same options>` in Read Only mode after two restarts and two live handoffs; round 7: With `70f63fb7`: `w14/ROUND7-PROOF.md` case C, a hand-typed Codex kept its 7 options, read-only and its session id across a handoff and a restart; round 8: `w15` X and Z, `w15b` Q; round 9: `w16` X, `w16b` Q; round 10: `w17` (Codex without `--no-daemon` kept its options), `w18`, `w18b` (build `1945cf5b`) |
| FR-3 | src/agent_resume.rs | agent_resume::tests::claude_bypass_launch_options_are_kept_on_restore; agent_resume::tests::codex_bypass_launch_options_are_kept_on_restore; terminal::state::tests::launcher_exec_in_the_same_process_records_the_agent_options | isolated proof `w3/PROOF-AFTER-FIX.md` cases A and B: both restored processes carry their bypass option and the agents show bypass and YOLO mode; isolated proof `w11/BYPASS-CODEX-PROOF.md` (build `7449fe52`): A came back with `bypass permissions on`; B came back with `permissions: YOLO mode` and `/status` `Permissions: Full Access`; round 7: `w14c/BYPASS-PROOF.md` (build `2fec054b`), Claude with `--dangerously-skip-permissions` and Codex with `--dangerously-bypass-approvals-and-sandbox` came back in bypass and YOLO mode with the same session ids; round 8: `w15b` P (Claude bypass) and Q (Codex YOLO) came back that way over a handoff and two restarts; round 9: `w16b` P and Q came back in bypass and YOLO mode over a handoff and two restarts; round 10: `w18b` P and Q; round 11: `w19b` P and Q |
| FR-4 | src/agent_resume.rs | agent_resume::tests::restrictive_launch_options_are_kept_on_restore; terminal::state::tests::launcher_exec_in_the_same_process_records_the_agent_options; app::actions::tests::a_launcher_exec_does_not_pair_a_new_process_with_the_previous_conversation; a_third_process_does_not_pair_with_the_conversation_an_earlier_process_owned; a_new_process_resuming_the_current_conversation_owns_it; terminal::state::tests::session_for_process_decides_every_case; agent_resume::tests::resumed_session_from_process_argv_reads_the_resumed_conversation; app::actions::tests::a_new_process_resuming_the_current_conversation_by_an_attached_id_owns_it; a_forked_process_does_not_own_the_conversation_it_forks | isolated proof `w3/PROOF-AFTER-FIX.md` case C came back with `--permission-mode plan`, showed plan mode and refused a write outside its folder; case D came back with `-s read-only -a on-request` and its outside write needed an approval escalation (approved by the user's own Codex `auto_review` setting, so no refusal was seen); isolated proof `w4/PROOF2-AFTER-FIX.md` case F came back with `-s read-only -a on-request`; isolated proof `w10b/LAUNCHER-PROOF.md` (build `7449fe52`, sha256 `145693c3...15cf5a`) K and H came back in plan mode; isolated proof `w11/BYPASS-CODEX-PROOF.md` (build `7449fe52`) D came back with `-s read-only -a on-request`, `/status` `Permissions: Read Only (Approve for me)`; isolated proof `w12b/CODEX-HANDOFF-PROOF.md` (build `6fe0bf45`): Codex read-only with on-request approvals (X) and Claude plan mode (Y) came back in those modes; isolated proof `w13/ROUND6-PROOF.md` case Y (build `ff1bc02a`): plan mode came back after two restarts |
| FR-5 | src/agent_resume.rs | agent_resume::tests::launch_without_permission_options_restores_without_them; app::actions::tests::a_new_process_seen_before_its_conversation_does_not_lend_its_options; a_conversation_reported_after_its_process_exited_waits_for_the_next_process; app::actions::tests::a_launcher_exec_does_not_pair_a_new_process_with_the_previous_conversation; a_third_process_does_not_pair_with_the_conversation_an_earlier_process_owned; a_new_process_resuming_the_current_conversation_owns_it; terminal::state::tests::session_for_process_decides_every_case; agent_resume::tests::resumed_session_from_process_argv_reads_the_resumed_conversation; app::actions::tests::a_new_process_resuming_the_current_conversation_by_an_attached_id_owns_it; a_forked_process_does_not_own_the_conversation_it_forks | isolated proof `w4/PROOF2-AFTER-FIX.md` case G: a Claude agent launched with model and effort but no permission option came back with only those options and in auto mode; isolated proof `w3/PROOF-AFTER-FIX.md` case E (fallback) came back bare with no permission option; isolated proof `w10/ROUND4-PROOF.md` (build `4d94cb5b`) case R: after the wrapper's second Claude started with `--permission-mode acceptEdits`, the first conversation was saved without options, never with the second process's; isolated proof `w11/BYPASS-CODEX-PROOF.md` (build `7449fe52`): every kept pane came back with only its own options; isolated proof `w12/ROUND5-PROOF.md` case T (build `6fe0bf45`): after the second and third Claude processes started with `acceptEdits`, the first conversation was saved without options, never with theirs, and came back bare with the marker in plan mode; isolated proof `w13/ROUND6-PROOF.md` case F (build `ff1bc02a`): a fork started with `acceptEdits` from a plan-mode conversation left that conversation saved without options, never with the fork's, and it came back bare with the marker in plan mode; round 7: `w14` R and `w14b` B: a hand-started `acceptEdits` or bypass Claude in a pane restored with resume off was never saved beside the old conversation; round 8: `w15` step 3, a suspended Codex's options never appeared beside the second Codex's conversation; round 9: `w16` N, a nested Claude started from a bypass Claude's Bash tool never had its conversation saved beside `--dangerously-skip-permissions`; round 10: `w18` N, the nested Claude's conversation never saved beside `--dangerously-skip-permissions`; round 11: `w19` N (build `8c7f9d97`); round 12: `w20` N (build `e944e58d`) and `w20b`; round 13: `w21` N (build `b38364c6`) and `w21b` |
| FR-6 | src/agent_resume.rs | agent_resume::tests::model_and_effort_launch_options_are_kept_on_restore; app::actions::tests::a_launcher_exec_does_not_pair_a_new_process_with_the_previous_conversation; a_third_process_does_not_pair_with_the_conversation_an_earlier_process_owned; a_new_process_resuming_the_current_conversation_owns_it; terminal::state::tests::session_for_process_decides_every_case; agent_resume::tests::resumed_session_from_process_argv_reads_the_resumed_conversation; app::actions::tests::a_new_process_resuming_the_current_conversation_by_an_attached_id_owns_it; a_forked_process_does_not_own_the_conversation_it_forks | isolated proof `w3/PROOF-AFTER-FIX.md`: A and C show `Haiku 5.5 with low effort` after restore, B and D show effort `low`; isolated proofs `w12/ROUND5-PROOF.md` cases S and E and `w12b/CODEX-HANDOFF-PROOF.md` (build `6fe0bf45`): Claude showed `Haiku 5.5 with low effort` and Codex reasoning low after each restart; isolated proof `w13/ROUND6-PROOF.md` cases A and Y (build `ff1bc02a`): model and effort came back after two restarts |
| FR-7 | src/agent_resume.rs, src/platform/mod.rs, src/app/agent_resume.rs | agent_resume::tests::other_claude_launch_options_are_kept_exactly; agent_resume::tests::other_codex_launch_options_are_kept_exactly; platform::tests::interactive_shell_command_quotes_backslashes_for_fish; platform::tests::interactive_shell_command_escapes_history_for_csh_and_tcsh; platform::tests::interactive_shell_command_keeps_posix_quoting_for_sh_family_and_unknown_shells | isolated proof `w3/PROOF-AFTER-FIX.md` cases B and D: `--no-daemon` and `-c model_reasoning_effort=low` came back on the restored Codex processes; isolated proof `w9/ROUND3-PROOF.md` (build `bb034c3b`, sha256 `c2f3f939...dbb5838`): under bash, `--append-system-prompt 'back\\slash & $HOME !bang "dq" \d end\'` came back byte for byte after two restarts, with plan mode |
| FR-8 | src/agent_resume.rs | agent_resume::tests::claude_conversation_prompt_and_print_parts_are_dropped; claude_prompt_after_variadic_option_is_one_of_its_values; codex_conversation_prompt_and_image_parts_are_dropped; unknown_launch_option_makes_options_unavailable; launch_subcommands_make_options_unavailable | isolated proof `w4/PROOF2-AFTER-FIX.md` case F: an agent started with `... resume <id>` came back as `codex resume <id> <options>` with no second `resume`; dropping one-time prompts and print mode is shown by the automated tests only |
| FR-9 | src/pane.rs, src/detect/mod.rs, src/events.rs, src/app/actions.rs, src/agent_resume.rs, src/terminal/state.rs | pane::tests::process_probe_carries_the_identified_agent_argv; pane::tests::agent_launch_observation_publishes_each_claude_or_codex_process_once; app::actions::tests::agent_launch_observed_event_records_options_from_the_command_line; agent_resume::tests::launch_options_from_process_argv_handles_install_shapes; pane::tests::agent_launch_observation_reports_a_process_again_once_its_command_line_is_read; terminal::state::tests::unreadable_launch_record_takes_a_later_readable_observation_of_the_same_process; pane::tests::agent_launch_observation_reports_a_launcher_that_execs_the_agent_in_its_process; terminal::state::tests::launcher_exec_in_the_same_process_records_the_agent_options; launcher_that_never_execs_keeps_its_record; app::actions::tests::a_launcher_exec_does_not_pair_a_new_process_with_the_previous_conversation; a_third_process_does_not_pair_with_the_conversation_an_earlier_process_owned; a_new_process_resuming_the_current_conversation_owns_it; terminal::state::tests::session_for_process_decides_every_case; agent_resume::tests::resumed_session_from_process_argv_reads_the_resumed_conversation; app::actions::tests::a_new_process_resuming_the_current_conversation_by_an_attached_id_owns_it; a_forked_process_does_not_own_the_conversation_it_forks; terminal::state::tests::a_fallback_seed_kept_through_a_launcher_decides_again_on_the_agent_it_execs; a_fork_of_the_seeded_conversation_does_not_keep_the_fallback_seed; agent_resume::tests::a_program_name_followed_only_by_empty_arguments_is_unreadable; terminal::state::tests::an_unsupported_option_reading_takes_a_later_parsed_reading_of_the_same_process; pane::tests::agent_launch_observation_reports_a_replacement_that_reused_the_process_id | isolated proof `w5/PROOF3-AFTER-FIX.md` (build `6c030b3d`) cases J1 and J2: Claude and Codex typed by hand at the shell (not `agent start`) were recorded with their options and came back with them after two restarts; isolated proof `w9/ROUND3-PROOF.md` (build `bb034c3b`, sha256 `c2f3f939...dbb5838`): a Claude started by hand through a bash wrapper was recognized and, its command line not recognized, saved no options; the transient unreadable read is covered by automated tests only; isolated proof `w10b/LAUNCHER-PROOF.md` (build `7449fe52`, sha256 `145693c3...15cf5a`) case H: Claude typed by hand through the python launcher saved its 6 options and came back with them; isolated proof `w11/BYPASS-CODEX-PROOF.md` (build `7449fe52`) case D: Codex typed by hand saved its 7 options and came back with them; isolated proof `w12/ROUND5-PROOF.md` case S (build `6fe0bf45`): a second Claude started through the launcher with `--resume <the same id>` had its own options read from its command line and saved with that conversation at once; isolated proof `w13/ROUND6-PROOF.md` case A (build `ff1bc02a`): the options of a second Claude started through the launcher with `-r<id>` were read and saved with that conversation at once |
| FR-10 | src/agent_resume.rs, src/app/agent_resume.rs, src/persist/restore.rs, src/terminal/state.rs, src/persist/snapshot.rs, src/ui/panes.rs, src/platform/linux.rs, src/pane.rs | agent_resume::tests::launch_options_exceeding_restore_limits_use_the_bare_command; unreadable_command_line_makes_options_unavailable; app::agent_resume::tests::launch_options_notice_explains_each_fallback_and_skips_kept_options; pending_agent_resume_writes_launch_options_notice_before_resume; terminal::state::tests::seeded_agent_launch_binds_to_the_first_observed_process; terminal::state::tests::seeded_agent_launch_survives_the_bare_resume_process; persist::restore::tests::restored_agent_launch_keeps_unknown_options_unknown_after_fallback; terminal::state::tests::resumed_without_launch_options_lasts_while_that_agent_runs; ui::panes::tests::pane_border_title_marks_agents_resumed_without_launch_options; persist::restore::tests::live_handoff_keeps_the_mark_of_a_resume_without_launch_options; ui::panes::tests::pane_without_top_border_reserves_a_marker_row_until_the_agent_exits; ui::panes::tests::pane_border_title_keeps_the_whole_marker_beside_a_long_label; ui::panes::tests::pane_with_top_border_keeps_the_marker_in_its_title; terminal::state::tests::fallback_seed_keeps_its_mark_for_an_unrecognized_bare_process; terminal::state::tests::fallback_seed_keeps_its_mark_for_a_wrapper_resuming_the_seeded_conversation; persist::restore cmd line fallback via platform::windows::tests::cmd_restore_line_must_fit_the_cmd_line_limit (written, not run: no Windows SDK); platform::tests::canonical_shell_accepts_line_only_within_its_limit; persist::restore::tests::restore_plan_falls_back_when_a_canonical_shell_cannot_take_the_line; terminal::state::tests::a_fallback_seed_kept_through_a_launcher_decides_again_on_the_agent_it_execs; a_fork_of_the_seeded_conversation_does_not_keep_the_fallback_seed; persist::restore::tests::restore_line_counts_its_leading_space_toward_the_canonical_limit; persist::restore::tests::restore_plan_keeps_a_long_line_for_bash_on_linux; restore_plan_falls_back_for_a_long_bash_line_on_macos | isolated proof `w7/MARKER-PROOF.md` (build `098980f5`, sha256 `7180171e...16b6d5`): after a server restart the fallback panes E (apostrophe, ExceedsResumeLimits), H (Claude, NotRecorded) and K (Codex, NotRecorded) came back bare with the top border `┌ launch options not kept ──┐`; the kept panes A and B showed no marker; the marker stayed through a live handoff with the same PIDs and cleared when E's Claude exited; isolated proof `w8/REVIEW-FIXES-PROOF.md` (build `c6b63af2`, sha256 `2e9d2a31...f8e100`): lone panes with default borders (L Claude, C Codex) and a pane with borders off (F) restored bare showed ` launch options not kept` on their first row with a terminal one row shorter (119 of 120); after L's Claude exited the row and marker were gone and the terminal was back to 120; kept panes (P, K2) showed no marker; isolated proof `w9/ROUND3-PROOF.md` (build `bb034c3b`, sha256 `c2f3f939...dbb5838`): the wrapper-launched pane W restored bare with ` launch options not kept` on its first row after two restarts; the restored shell did not have the wrapper on its search path, so the unrecognized-bare-process case itself is covered by its automated test only; isolated proof `w10/ROUND4-PROOF.md` (build `4d94cb5b`) and isolated proof `w10b/LAUNCHER-PROOF.md` (build `7449fe52`, sha256 `145693c3...15cf5a`): the wrapper pane W restored bare with ` launch options not kept` on its first row after two restarts; isolated proof `w11/BYPASS-CODEX-PROOF.md` (build `7449fe52`) case C (Codex, options removed) restored bare with the marker row; the wrapper whose child resumes the seeded conversation and the cmd line limit are covered by automated tests only; isolated proof `w12/ROUND5-PROOF.md` (build `6fe0bf45`): case D (5,124 bytes of options, pane shell not bash, zsh or fish) came back as the bare `claude --resume <id>` with the marker row and reason ExceedsResumeLimits; case E (3,084 bytes) came back with its options; isolated proof `w13/ROUND6-PROOF.md` case F (build `ff1bc02a`): the forked pane came back as the bare `claude --resume <id>` with the marker row, reason NotRecorded |
| FR-11 | src/agent_resume.rs, src/app/agents.rs | agent_resume::tests::codex_launch_with_options_records_its_session; codex_noncanonical_resume_launch_has_no_explicit_session; app::tests::agent_start_records_codex_launch_options | isolated proof `w4/PROOF2-AFTER-FIX.md` case F: `agent start -- --no-daemon -s read-only -a on-request resume <id>` returned the session id at launch, the saved session carried the options, and restore kept both |
| FR-12 | src/persist/snapshot.rs, src/agent_resume.rs, src/platform/mod.rs | persist::snapshot::tests::capture_saves_known_launch_options_for_the_session_agent; agent_session_launch_options_round_trip_and_old_json_loads; agent_resume::tests::other_claude_launch_options_are_kept_exactly; platform::tests::interactive_shell_command_quotes_backslashes_for_fish; platform::windows::tests::cmd_restore_line_must_fit_the_cmd_line_limit (written, not run: no Windows SDK); platform::tests::restore_command_starts_with_a_space_that_keeps_it_out_of_shell_history; platform::tests::restore_command_is_the_shell_command_unchanged_on_windows | isolated proof `w3/PROOF-AFTER-FIX.md`: `-c model_reasoning_effort=low` and the placeholder `--append-system-prompt` text were saved as-is in `session.json`; the `-c` value came back exactly; isolated proof `w9/ROUND3-PROOF.md` (build `bb034c3b`, sha256 `c2f3f939...dbb5838`): the inline value with backslashes, `&`, `$HOME`, `!` and double quotes came back byte for byte under bash; fish, csh, tcsh and Windows cmd are covered by automated tests only (not installed here); isolated proof `w11/BYPASS-CODEX-PROOF.md` (build `7449fe52`): `-c model_reasoning_effort=low` came back exactly for B and D; round 7: `w14` step 4: the typed restore line starts with a space and stayed out of bash history (`HISTCONTROL=ignorespace`) |
| FR-13 | src/terminal/state.rs, src/persist/restore.rs, src/app/actions.rs | persist::restore::tests::native_agent_restore_seeds_saved_launch_options_for_the_next_save; live_handoff_keeps_agent_launch_options; app::actions::tests::launch_option_changes_mark_the_session_dirty; app::actions::tests::a_launcher_exec_does_not_pair_a_new_process_with_the_previous_conversation; a_third_process_does_not_pair_with_the_conversation_an_earlier_process_owned; a_new_process_resuming_the_current_conversation_owns_it; terminal::state::tests::session_for_process_decides_every_case; live_handoff_keeps_the_mark_of_a_resume_without_launch_options; persist::restore::tests::live_handoff_keeps_an_assumed_pairing_bound_to_its_process; terminal::state::tests::a_handed_off_pairing_is_not_kept_by_a_process_that_reused_its_id | isolated proof `w3/PROOF-AFTER-FIX.md` step 3: a live handoff kept every agent's process id and command line and the next save still carried the options; restart 1 and 2 kept them; isolated proof `w12b/CODEX-HANDOFF-PROOF.md` (build `6fe0bf45`): two live handoffs kept every agent's process id and command line; the next saves kept X's and Y's options and Z's fallback mark, and restarts 1 and 2 kept them; round 7: `w14` step 2 and `w14b` steps 2 to 4: a hand-started Claude kept its options across a live handoff (saved by the handoff server); panes restored with resume off kept theirs across a handoff with no agent running; round 8: `w15b` steps 2 and 5, options kept across a handoff with agents and one with none; round 9: `w16`, `w16b` steps 2 and 5; round 10: `w18`, `w18b` handoffs; round 11: `w19`, `w19b` handoffs |
| FR-14 | src/terminal/state.rs, src/persist/snapshot.rs | terminal::state::tests::agent_launch_observation_replaces_records_for_new_processes; agent_launch_is_cleared_when_the_agent_exits_or_changes; persist::snapshot::tests::capture_saves_no_launch_options_without_a_matching_known_record; terminal::state::tests::kept_or_started_agent_launch_takes_an_empty_observation; app::actions::tests::a_new_process_seen_before_its_conversation_does_not_lend_its_options; a_conversation_reported_after_its_process_exited_waits_for_the_next_process; a_conversation_change_inside_one_process_keeps_its_options; app::actions::tests::a_launcher_exec_does_not_pair_a_new_process_with_the_previous_conversation; a_third_process_does_not_pair_with_the_conversation_an_earlier_process_owned; a_new_process_resuming_the_current_conversation_owns_it; terminal::state::tests::session_for_process_decides_every_case; terminal::state::tests::seed_keeps_record_decides_every_case; agent_resume::tests::resumed_session_from_process_argv_reads_the_resumed_conversation; app::actions::tests::a_new_process_resuming_the_current_conversation_by_an_attached_id_owns_it; a_forked_process_does_not_own_the_conversation_it_forks; terminal::state::tests::a_fallback_seed_kept_through_a_launcher_decides_again_on_the_agent_it_execs; a_fork_of_the_seeded_conversation_does_not_keep_the_fallback_seed; a_launcher_bound_to_a_seed_does_not_pair_the_agent_resuming_another_conversation; persist::restore::tests::a_hand_started_agent_does_not_take_the_conversation_a_restore_without_resume_left; a_hand_started_agent_does_not_take_the_conversation_a_failed_resume_left; terminal::state::tests::a_report_from_a_second_codex_beside_a_suspended_one_is_not_paired_with_it; a_report_from_a_claude_resumed_beside_a_suspended_one_is_not_paired_with_it; a_report_from_a_child_agent_in_the_same_job_is_not_paired_with_its_parent; a_conversation_change_inside_the_only_agent_of_the_job_keeps_its_options; respawn_cleanup_resets_restored_agent_status; terminal::state::tests::a_nested_claude_report_is_not_paired_with_its_parent; a_report_from_a_child_in_the_job_is_not_paired_with_the_parent_in_any_order; a_first_observation_does_not_take_a_conversation_reported_beside_another_agent; a_conversation_whose_reporter_exited_unobserved_is_not_taken_by_the_next_process; a_report_dropped_beside_another_agent_is_decided_again_when_repeated; a_conversation_change_reported_by_the_bound_process_keeps_its_options; subscription_socket_tests::a_session_report_carries_the_process_that_sent_it; detect::tests::a_reporter_names_only_an_execed_child_under_a_native_parent_agent; a_hook_run_by_an_agent_outside_the_pane_names_no_reporter; report_sender_names_the_nearest_agent_inside_the_pane; a_walk_that_reaches_no_agent_below_the_pane_names_no_process; process_ancestry_stops_at_its_limit_unreadable_processes_and_reused_parents; terminal::state::tests::a_reused_reporter_process_id_is_not_confirmed; a_bound_process_id_reused_by_a_reporter_is_not_confirmed; a_report_whose_walk_failed_is_not_paired_with_the_job; detect::tests::a_walk_cut_after_a_child_agent_names_no_process; an_interpreter_run_parent_agent_is_not_the_launcher_of_a_nested_agent; terminal::state::tests::a_conversation_reported_from_outside_the_pane_is_not_paired_with_the_job; terminal::state::tests::a_started_seed_pairs_only_beside_one_agent_in_the_job; a_reused_process_id_in_the_job_does_not_confirm_the_bound_record; terminal::state::tests::a_report_first_beside_two_agents_is_decided_again_after_observation; a_report_first_beside_one_agent_is_paired_only_with_that_process; a_seed_paired_beside_one_agent_binds_only_to_that_process; app::actions::tests::a_conversation_reported_beside_another_process_does_not_take_the_observed_options; persist::restore::tests::live_handoff_carries_no_pairing_reported_beside_another_process | isolated proof `w4/PROOF2-AFTER-FIX.md` case G: after `/exit` the pane had no saved session; a new Claude launched without a permission option was saved and restored without one, in auto mode. The record of an agent started with `agent start` is handled the same way in `6c030b3d` (only restore fallbacks keep their record, test `kept_or_started_agent_launch_takes_an_empty_observation`); isolated proof `w10/ROUND4-PROOF.md` (build `4d94cb5b`) case R poll: A's options left the saved session when B started and were never paired with B's; isolated proof `w10b/LAUNCHER-PROOF.md` (build `7449fe52`, sha256 `145693c3...15cf5a`) case K: after `/clear` the new conversation kept the same process's options and came back with them; isolated proof `w12/ROUND5-PROOF.md` (build `6fe0bf45`): case T, a new process never took the conversation an earlier process owned, including after the launcher exec and for a third process; case S, a process resuming the current conversation took it with its own options; isolated proof `w13/ROUND6-PROOF.md` (build `ff1bc02a`): case F, a fork never took the conversation it forked; case A, a process resuming the current conversation with `-r<id>` took it with its own options; round 7: `w14` R and `w14b` B as for FR-5; the agents' own new conversations never reached Herdr (the separate startup-report defect); round 8: `w15` steps 2 and 3 (`/clear` kept options; suspended Codex case); round 9: `w16` N and H (`/clear`); round 10: `w18` N and H (`/clear`); round 11: `w19` N and H; round 12: `w20` N (build `e944e58d`) and `w20b`; round 13: `w21` N (build `b38364c6`) and `w21b` |
| FR-15 | src/persist/snapshot.rs, src/agent_resume.rs, src/persist/restore.rs, src/ui/panes.rs | persist::snapshot::tests::agent_session_launch_options_round_trip_and_old_json_loads; persist::restore::tests::pane_restore_startup_without_saved_launch_options_uses_bare_command; agent_resume::tests::unrecorded_launch_options_use_the_bare_command_with_notice; persist::restore::tests::native_agent_restore_without_saved_options_keeps_them_unknown; persist::snapshot::tests::agent_session_resumed_without_launch_options_round_trips_and_defaults; persist::restore::tests::unavailable_restored_terminal_keeps_saved_launch_options_for_the_next_save | isolated proof `w7/MARKER-PROOF.md`: panes H and K, whose saved `launch_options` were removed as in a session file from an older Herdr, came back as `claude --resume <id>` and `codex resume <id>` with the border marker after two restarts; isolated proof `w8/REVIEW-FIXES-PROOF.md` (build `c6b63af2`, sha256 `2e9d2a31...f8e100`): L, C and F, whose saved `launch_options` were removed as in an older session file, came back with the bare command and the marker row |
| FR-16 | src/agent_resume.rs, src/persist/restore.rs | agent_resume::tests::launch_options_do_not_change_the_dedupe_key; persist::restore::tests::pane_restore_startup_with_launch_options_respects_disabled_resume_and_dedupe | N/A: this change does not alter how duplicate panes are resolved; the deterministic restore tests cover it |
| FR-17 | src/persist/restore.rs | persist::restore::tests::pane_restore_startup_with_launch_options_respects_disabled_resume_and_dedupe | N/A: no new surface; the setting is read where it was before and the deterministic restore test covers it |
| FR-18 | src/agent_resume.rs, src/persist/restore.rs | agent_resume::tests::other_agents_restore_is_unchanged_by_launch_options; persist::restore::tests::pane_restore_startup_reported_resume_ignores_saved_launch_options | isolated proof `w5/PROOF3-AFTER-FIX.md` (build `6c030b3d`) case P: pi came back as `pi --session <same path>` with nothing added (pi had no model login, so its conversation did not complete); third-party reported resume commands are shown by the automated tests only |
| FR-19 | reproduction before the fix (no code) | N/A: a process requirement; the deterministic tests are FR-23 | Baseline before the fix: `w3/REPRO-BASELINE.md` (Claude bypass came back in auto mode, Codex bypass lost YOLO mode); the same two cases after the fix: isolated proof `w3/PROOF-AFTER-FIX.md` cases A and B |
| FR-20 | evidence process (no code) | N/A: a process requirement | isolated proof `w3/PROOF-AFTER-FIX.md` and isolated proof `w4/PROOF2-AFTER-FIX.md` used only their own debug-build sessions under `~/.config/herdr-dev/`, stopped and deleted them, and touched no other session or agent |
| FR-21 | docs/next/website/src/content/docs/session-state.mdx (and ja, zh-cn) | just docs-contract-test and scripts.test_docs_translation_parity passed | N/A: documentation only; checked by the docs contract and translation parity recipes |
| FR-22 | src/agent_resume.rs, src/terminal/state.rs, src/persist/restore.rs, src/platform/linux.rs | agent_resume::tests::restored_command_line_yields_the_original_launch_options; persist::restore::tests::native_agent_restore_seeds_saved_launch_options_for_the_next_save; persist::restore::tests::restored_agent_launch_keeps_unknown_options_unknown_after_fallback; persist::restore::tests::unavailable_restored_terminal_keeps_saved_launch_options_for_the_next_save; platform::linux::tests::cmdline_argv_keeps_empty_arguments; app::agent_resume::tests::empty_launch_option_value_is_restored_quoted; app::actions::tests::a_launcher_exec_does_not_pair_a_new_process_with_the_previous_conversation; a_third_process_does_not_pair_with_the_conversation_an_earlier_process_owned; a_new_process_resuming_the_current_conversation_owns_it; terminal::state::tests::session_for_process_decides_every_case; platform::tests::canonical_shell_accepts_line_only_within_its_limit; persist::restore::tests::restore_plan_falls_back_when_a_canonical_shell_cannot_take_the_line; agent_resume::tests::resumed_session_from_process_argv_reads_the_resumed_conversation; app::actions::tests::a_new_process_resuming_the_current_conversation_by_an_attached_id_owns_it; a_forked_process_does_not_own_the_conversation_it_forks | `w3/PROOF-AFTER-FIX.md` step 6: A to D still carried their options after a second restart; isolated proof `w5/PROOF3-AFTER-FIX.md` (build `6c030b3d`): kept options (A, B, J1, J2) and fallbacks (E, H, K) were both unchanged after a second restart; isolated proof `w8/REVIEW-FIXES-PROOF.md` (build `c6b63af2`, sha256 `2e9d2a31...f8e100`): P, launched with `--append-system-prompt ''`, kept all 8 options including the empty value over two restarts and still showed plan mode; isolated proof `w9/ROUND3-PROOF.md` (build `bb034c3b`, sha256 `c2f3f939...dbb5838`): S kept all 8 options, identical, over two restarts; isolated proofs `w12/ROUND5-PROOF.md` and `w12b/CODEX-HANDOFF-PROOF.md` (build `6fe0bf45`): kept options (S, E, X, Y) and fallbacks (T, D, Z) were unchanged after a second restart; isolated proof `w13/ROUND6-PROOF.md` (build `ff1bc02a`): kept options (A, Y) and the fallback (F) were unchanged after a second restart; round 7: `w14b`: S and H kept their options through a resume-off restart, a handoff and a restart with resume on |
| FR-23 | src/agent_resume.rs | agent_resume::tests (29 tests) cover bypass, restrictive, no-permission, model and effort restore commands | N/A: this row is the deterministic test suite itself |
| FR-24 | src/agent_resume.rs, src/persist/restore.rs | covered by the FR-1 to FR-3 tests | isolated proof `w3/PROOF-AFTER-FIX.md` cases A (Claude) and B (Codex) after restart: same session id, bypass option on the restored process, bypass and YOLO mode shown, and each read `/var/tmp/t2-proof-20261019-143633/outside/fixture.txt` and wrote beside it with no permission prompt |
| FR-25 | branch `portal/t2-4445c337` on `fitzzz/herdr`; the exact source commit and isolated-test build are the latest ones named in the pull request and in `artifacts/CONSUME.md` ("the current candidate"), each build with its sha256 in `artifacts/SHA256SUMS`; after review round 13 that is code `b38364c6` and `artifacts/herdr-t2-debug-b38364c6`, and later commits up to the pull request head change only docs and this PRD. Historical: the first delivery was code `6c030b3d` with `artifacts/herdr-t2-debug-6c030b3d`, which lacks the review fixes | N/A: a delivery requirement | N/A: checked in the pull request and the delivery report; the artifact itself was used by W5 (FR-27) |
| FR-26 | known-limit list in the Change Log entry of 2026-10-19 15:21:50 EDT, repeated in the pull request and the delivery report | N/A: a delivery requirement | N/A: a report, not a user surface |
| FR-27 | `artifacts/CONSUME.md` (the consumption path for the current candidate build); the original proof used `artifacts/herdr-t2-debug-e219b6d6`, and every later live check used the build named in its proof file | N/A: a delivery requirement | isolated proof `w3/PROOF-AFTER-FIX.md` and isolated proof `w4/PROOF2-AFTER-FIX.md` ran the artifact as `artifacts/CONSUME.md` describes, in their own `herdr-dev` sessions; the installed Herdr server and its agents were not touched |
| FR-28 | evidence format (no code) | N/A: a process requirement | isolated proof `w3/PROOF-AFTER-FIX.md` and isolated proof `w4/PROOF2-AFTER-FIX.md` record the process path version, the transcript version and the shell `--version` separately (Claude 2.1.300 and Codex 0.162.1 from all three) |
| FR-29 | src/app/agent_resume.rs, src/persist/restore.rs, src/terminal/state.rs, src/persist/snapshot.rs, src/ui/panes.rs, src/pane.rs | app::agent_resume::tests::launch_options_notice_explains_each_fallback_and_skips_kept_options; persist::restore::tests::pane_restore_startup_resumes_with_saved_launch_options; pane_restore_startup_without_saved_launch_options_uses_bare_command; terminal::state::tests::seeded_agent_launch_survives_the_bare_resume_process; persist::restore::tests::restored_agent_launch_keeps_unknown_options_unknown_after_fallback; persist::snapshot::tests::capture_saves_resumed_without_launch_options_for_the_session_agent; pane::tests::agent_launch_observation_reports_a_second_agent_in_the_same_process_group; terminal::state::tests::a_second_agent_process_replaces_the_launch_record; terminal::state::tests::launcher_exec_in_the_same_process_records_the_agent_options; platform::tests::canonical_shell_accepts_line_only_within_its_limit; persist::restore::tests::restore_plan_falls_back_when_a_canonical_shell_cannot_take_the_line; terminal::state::tests::seed_keeps_record_decides_every_case; terminal::state::tests::a_fallback_seed_kept_through_a_launcher_decides_again_on_the_agent_it_execs; a_fork_of_the_seeded_conversation_does_not_keep_the_fallback_seed; persist::restore::tests::unavailable_restored_terminal_keeps_saved_launch_options_for_the_next_save (no marker); persist::restore::tests::restored_agent_launch_keeps_the_mark_without_a_resume_command | isolated proof `w7/MARKER-PROOF.md`: per pane the border marker, `resumed_without_launch_options: true` in `session.json` and the server log (`agent resume without launch options ... reason=`) mark the fallback panes E, H and K; the kept panes A and B carry `launch_options`, no marker and `agent resume keeps launch options`; isolated proof `w8/REVIEW-FIXES-PROOF.md` (build `c6b63af2`, sha256 `2e9d2a31...f8e100`): the server log said `agent resume without launch options` (NotRecorded) for L, C and F and `agent resume keeps launch options` for P (8) and K2 (6); only the fallback panes carried `resumed_without_launch_options`; isolated proof `w9/ROUND3-PROOF.md` (build `bb034c3b`, sha256 `c2f3f939...dbb5838`): the server log said `agent resume keeps launch options` (8) for S and `agent resume without launch options` (NotRecorded) for W at both restarts; isolated proof `w10b/LAUNCHER-PROOF.md` (build `7449fe52`, sha256 `145693c3...15cf5a`) and isolated proof `w11/BYPASS-CODEX-PROOF.md` (build `7449fe52`): the server log said `agent resume keeps launch options` for K (6), H (6), A (6), B (4) and D (7) and `agent resume without launch options` (NotRecorded) for W and C at each restart; only W and C carried `resumed_without_launch_options`; isolated proofs `w12/ROUND5-PROOF.md` and `w12b/CODEX-HANDOFF-PROOF.md` (build `6fe0bf45`): every fallback (T and Z NotRecorded, D ExceedsResumeLimits) showed the marker row and saved `resumed_without_launch_options: true`; every kept restore logged `agent resume keeps launch options` and showed no marker; isolated proof `w13/ROUND6-PROOF.md` (build `ff1bc02a`): the fallback (F) showed the marker row and saved `resumed_without_launch_options: true` after each restart; the kept restores (A, Y) logged `agent resume keeps launch options` and showed no marker; round 7: `w14` step 7: the fallback panes showed `launch options not kept` and saved the mark; kept panes showed nothing; round 8: `w15b` F showed `launch options not kept` when restored bare and no marker while resume was off; round 9: `w16` N and `w16b` F came back bare with the marker; no marker with resume off; round 10: `w18` N and `w18b` F bare with the marker; round 11: `w19` N and `w19b` F |
| FR-30 | src/persist/restore.rs (logs counts and reasons only), test fixtures | All new tests use placeholder values; restore logs carry option_count or reason, never values; agent_resume::tests::a_rejected_option_is_logged_by_name_without_its_value | N/A: no user surface; checked by review of fixtures, logs and evidence files |

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
- 2026-10-20 12:24:32 EDT - Review rounds 1 and 2 on `034ecb1f` (Codex run 6c6a2d77, Claude run 7a50c860, Claude round 1
  run 7bcf6d46) found, confirmed by reading the code: (P1) the marker is drawn only in a top border, so the default lone
  pane, borders off and an outer top border turned off show nothing; (P2) the Linux command-line reader drops empty
  arguments, so `--append-system-prompt ''` is misread and reported as kept; (P3) a long pane label cuts the marker off;
  (P3) a second agent process in the same process group keeps the first one's options; (P3) a pane whose directory is
  missing at restore loses its saved options at the next save; (P4) three notice reasons cannot be reached; (P4) four
  stale test comments; (P4) Open Questions says None while the known limits referred to a question (since answered by
  K-377 r1). The 15:56 entry left the agents list out as an agent decision, but the option Fitz chose for K-377 r1
  names "the pane's title and its row in the agents list", so that omission did not meet the decision. FR-10, FR-15
  and FR-29 reset to No (Implemented, Automated Tests and Interaction Testing); FR-22 is reopened for the
  missing-directory path. Fixes assigned to worker W2 (`BRIEF-W2-review-fixes.md`). How a lone or borderless pane shows
  the marker (a reserved row at the top of the pane while the flag is set) is an agent decision within K-377 r1,
  inferred, not Fitz's.
- 2026-10-20 12:59:13 EDT - Worker W2 built the fixes on `portal/t2-4445c337-w2-fixes`, fast-forwarded onto this
  branch: `615d91c9` (unreachable notice reasons and stale comments removed), `bbb07b81` (a pane whose directory is
  missing keeps its saved options), `b95b759a` (Linux keeps empty arguments; an empty value is restored as `''`; macOS
  still gives up on a command line with an empty argument, so restore falls back with the notice there), `6b4f1a8d`
  (each Claude or Codex process is observed by its own process id), `c6b63af2` (a flagged pane without a top border
  reserves its first row for the marker during view computation; the label gives way so the marker stays whole). The
  agents-list marker would need a new snapshot codec or a changed field meaning, so W2 did not build it. Fitz chose
  "Pane only, ship now" (2026-10-20, asked in this session): FR-10 changed to drop the agents list; it stays No until
  verified again. Checks at `c6b63af2`: fmt and clippy clean, UI hot-path architecture test OK, focused filter 344 of
  344, full suite 3976 run, 3974 passed, the same 2 base failures (`logs/check-w2-fixes.log`); Windows lint not run
  (no Windows SDK on this machine). Render scale benchmark, background workspaces, 15 panes, combined pipeline median:
  871 and 872 us with the fixes, 935 and 1076 us on the base, 1099 and 938 us at `098980f5`; no measurable change
  (`logs/bench-render-fixes-*.log`). Isolated live check W8 started (`BRIEF-W8-review-fixes-proof.md`).
- 2026-10-20 13:09:19 EDT - Isolated proof W8 (`w8/REVIEW-FIXES-PROOF.md`, build `c6b63af2`) matched every step: the marker row on lone
  default-border panes (Claude and Codex) and with borders off, cleared with the row returned when the agent exited, and
  the empty `--append-system-prompt ''` value kept over two restarts with plan mode. Added `f68bd01e` (docs: the marker
  row on panes without a top border). FR-10, FR-15, FR-22 and FR-29 set to Yes.
- 2026-10-20 14:07:18 EDT - Review round 3 on `52a66011` (Codex run 02822e04, Claude run 03ea4b87) found, confirmed by reading the code:
  (P1) restore quotes options for POSIX shells only, so in fish `\\` becomes `\` (inline JSON breaks) and restore
  still reports Kept; the 2026-10-19 15:21 known-limits entry recorded this as an observation, not an approved
  exception; (P2) the same quoting in a cmd.exe pane lets `&` run a second command and splits spaced values; (P2) a bare
  resume process that parses as an unrecognized command replaces the fallback seed and loses the marker; (P3) one
  unreadable command-line read leaves an agent's options unknown until it exits; (P4) the marker can stay on a pane
  where the resume command started no agent. FR-7, FR-9, FR-10, FR-12, FR-22 and FR-29 reset to No. Fixes assigned to
  W2 (`BRIEF-W2-round3-fixes.md`): restore goes through the shell-aware command builder that agent start uses, with
  fish and csh quoting added (agent decision within FR-7, inferred).
- 2026-10-20 14:17:54 EDT - W2 built the round 3 fixes, fast-forwarded onto this branch: `981bba18` (restore quotes its command for the
  shell Herdr starts in the pane, the configured shell, else `$SHELL`, else `/bin/sh`, through the shared builder that
  agent start uses; fish doubles backslashes, csh and tcsh escape `!` outside the quotes, PowerShell and cmd on Windows
  use the existing agent-start path; unknown shell names keep POSIX quoting; this also fixes agent start in fish and
  csh), `bb034c3b` (a fallback seed keeps its mark for any unavailable observation of the bare process; a process whose
  command line could not be read is reported once more when it becomes readable, and that reading replaces the
  unreadable record). The marker on a pane where the resume command started no agent is not fixed: the pane has no
  signal that tells "the command finished without an agent" from "the command has not started yet", and clearing it on
  a timer would bring back the original bug for slow agent starts (W2's finding; leaving it is an agent decision,
  recorded as a known limit, not Fitz's). Checks at `bb034c3b`: fmt and clippy clean, UI hot-path architecture test
  OK, focused filter 309 of 309, full suite 3981 run, 3979 passed, the same 2 base failures
  (`logs/nextest-w2-round3.log`); maintenance, integration-assets and docs-contract tests pass. Not run: the Windows
  test of cmd quoting and Windows lint (no Windows SDK here), and fish, zsh, csh and tcsh live (not installed; only
  bash is). Isolated live check W9 started (`BRIEF-W9-round3-proof.md`).
- 2026-10-20 14:22:36 EDT - Isolated proof W9 (`w9/ROUND3-PROOF.md`, build `bb034c3b`): S's special-character value came back byte for
  byte over two restarts under bash with plan mode; the wrapper-launched W saved no options and restored bare with the
  marker row. The restored shell did not have the wrapper on its search path, so the restored process was the real
  Claude; the unrecognized-bare-process case and the transient unreadable read are covered by automated tests only.
  FR-7, FR-9, FR-10, FR-12, FR-22 and FR-29 set to Yes.
- 2026-10-21 06:16:48 EDT - Review round 4 on `1fd36e6b` (Claude run d37c5836, Codex run 647b2e06) found, confirmed by reading the code:
  (P1, Codex) when a wrapper starts a second Claude or Codex process before the probe sees the first one exit, the
  launch record takes the new process's options while the saved session is still the first conversation's, so a save
  in that gap pairs conversation A with process B's options (and the reverse when B's session report arrives first);
  `launch_options_for_session` checks only the agent kind; (P3, Claude) on a Windows cmd pane the encoded restore line
  is about five times the options' length, so options above about 1.5 KB exceed cmd.exe's 8191-character line while
  restore reports Kept; (P3, Claude, flagged by the reviewer as a design disagreement) a wrapper named after the agent
  that adds its own option makes the fallback seed yield, clearing the marker on the first restore and saving the
  wrapper's option as the user's; (P4, Claude) a dead clause `(spec.keep && !spec.resume)` in `parse_codex_args`.
  FR-4, FR-5, FR-10, FR-12, FR-14 and FR-29 reset to No. Fixes assigned to W2 (`BRIEF-W2-round4-fixes.md`): options are
  saved beside a session only when the session and the launch record are known to belong to the same process, else the
  session is saved without options (restore falls back with the marker); a restore line longer than the pane shell
  accepts falls back; the fallback seed keeps its mark for a process that resumes the seeded conversation (agent
  decisions within FR-5, FR-10 and FR-14, inferred).
- 2026-10-21 15:40:09 EDT - W2 built the round 4 fixes, fast-forwarded onto this branch: `e97141a4` (the always-set `resume` field and
  its dead clause are removed from the Codex option table), `424fd604` (the launch record stores the conversation it
  belongs to and options are saved only beside that conversation; a seed takes the conversation current when seeded;
  a new process takes the current conversation only when it is not the previous process's; a conversation reported
  while the record's process still runs, such as Claude `/clear`, keeps its options; one reported after that process
  exited waits for the next process. Either arrival order saves the conversation without options until the pairing is
  known, so restore falls back with the marker in that gap. Known limit: an exited process that is not yet reaped still
  counts as running), `bf426961` (on Windows a cmd pane accepts at most 8191 characters per typed line; a kept restore
  whose line is longer falls back to the bare command with the marker, reason ExceedsResumeLimits; Unix shells keep
  today's behavior, bash took a 9000-character line; `agent start` has the same cmd overflow and is not changed, as it
  is outside restore), `d761cbbf` (the Claude parser reads the conversation a command line resumes; a fallback seed
  keeps its options and marker for a process that resumes the seeded conversation, whatever else its command line
  holds), `4d94cb5b` (the conversation sync compares before copying, so it allocates only when the conversation
  changes). Checks at `4d94cb5b`: fmt and clippy clean, UI hot-path architecture test OK, focused filter 366 of 366,
  full suite 3986 run, 3984 passed, the same 2 base failures (`logs/nextest-4d94cb5b.log`); maintenance,
  integration-assets and docs-contract tests pass at `d761cbbf`. Not run: the Windows test
  `cmd_restore_line_must_fit_the_cmd_line_limit` and Windows lint (no Windows SDK here). Isolated live check W10 started
  (`BRIEF-W10-round4-proof.md`, build `4d94cb5b`, sha256 `ec9cc9f0...8ff319`) with an Opus tester, because the Sonnet
  tester hit its weekly limit (Fitz chose "Use Opus now" on 2026-10-21, an exception to G11 for this check).
- 2026-10-21 15:50:24 EDT - Isolated proof W10 (`w10/ROUND4-PROOF.md`, build `4d94cb5b`): K (`agent start` with
  `--permission-mode plan`) and W (hand-launched through a wrapper that adds options) saved no `launch_options`; all
  three panes restored bare with the marker row at both restarts (NotRecorded). R: A saved with its options; after A
  exited and the script started B, A's conversation was saved without options (the round 4 P1 fix), but B's new
  conversation never reached Herdr, so restore resumed A's conversation bare. Cause of K and W, confirmed by reading the
  code: since 2026-10-21 11:35 `~/.local/bin/claude` is a Python launcher that execs the real Claude in the same
  process; Herdr identifies the python process as Claude, reads its command line as unrecognized, and does not read
  that process again after the exec. This predates round 4 and also drops kept options on a restore that goes through
  the launcher. Fix and the investigation of R's missing conversation assigned to W2 (`BRIEF-W2-round4-launcher.md`).
  FR-1, FR-2, FR-3 and FR-9 reset to No with the rows already reset.
- 2026-10-21 16:12:02 EDT - W2 built `7449fe52` (the probe reports a process again when the program its command line runs
  changes, as on an exec; a later reading of the same process replaces the record only when it tells more:
  unreadable, then unrecognized, then parsed; a launcher that never execs keeps its unrecognized record; the probe
  already read each foreground command line every tick, and the only added per-tick work is comparing its first word
  with the stored one, without allocating). Tests: `terminal::state::tests::launcher_exec_in_the_same_process_records_the_agent_options`
  (hand-launched, `agent start` seed and kept restore seed, each confirmed failing without the change),
  `pane::tests::agent_launch_observation_reports_a_launcher_that_execs_the_agent_in_its_process` (confirmed failing
  without the change), `terminal::state::tests::launcher_that_never_execs_keeps_its_record`. Checks at `7449fe52`:
  fmt and clippy clean, UI hot-path architecture OK, focused filter 367 of 367, full suite 3989 run, 3987 passed, the
  same 2 base failures (`logs/nextest-w2-round4-launcher.log`); maintenance, integration-assets and docs-contract tests
  pass. W2's finding on W10 case R: Herdr lets a Claude conversation-start report replace the pane's conversation only
  for the reasons clear, resume or compact (`session_report_allows_session_replacement`), so a second Claude started
  in the same pane after the first one exited (reason startup) is dropped by `conflicting_same_owner_session_ref`; a
  test showed the same at the base `4b974866` and at `1fd36e6b`, and this branch does not touch that code. It is
  outside this PRD; flagged for Fitz as a separate defect (agent observation, not a decision). Isolated proof W10b
  (`w10b/LAUNCHER-PROOF.md`, build `7449fe52`, sha256 `145693c3...15cf5a`): K (`agent start`) and H (typed by hand),
  both through the launcher with `--permission-mode plan`, saved their 6 options, K kept them after `/clear` with the
  new conversation, and both came back as `claude <same options> --resume <same id>` in plan mode after two restarts
  (`agent resume keeps launch options ... option_count=6`); the wrapper pane W saved none (Herdr identifies the bash
  wrapper, whose command line it does not recognize) and restored bare with the marker row. The wrapper whose child
  resumes the seeded conversation (round 4 item 3) is covered by automated test only. Isolated proof W11 started
  (`BRIEF-W11-bypass-codex-proof.md`) for Claude bypass and Codex through the launcher.
- 2026-10-21 16:26:16 EDT - Isolated proof W11 (`w11/BYPASS-CODEX-PROOF.md`, build `7449fe52`): Claude in bypass (A), Codex in
  bypass (B) and Codex read-only typed by hand (D), all through the python launcher, came back with their options and
  modes after two restarts; Codex with its options removed (C) came back bare with the marker row. Side observation,
  not part of this PRD: for Codex 0.163.0 `agent prompt --wait` timed out with status unknown although the reply was on
  screen (flagged). FR-1, FR-2, FR-3, FR-4, FR-5, FR-9, FR-10, FR-12, FR-14 and FR-29 set to Yes. Not run: the Windows
  cmd line test and Windows lint (no Windows SDK).
- 2026-10-21 16:45:13 EDT - Review round 5 on `74c893a9` (Claude run f2bdd7b1, Codex run 6fb57188) found, confirmed by reading the code:
  (P1, Claude r5-1 and Codex) `observe_agent_launch` still pairs one process's options with another's conversation:
  a same-process re-read (a launcher exec'ing the agent) takes the pane's current conversation even when that process
  was left unpaired, and a third process that replaces an unpaired one is compared against no conversation; (P1, Codex,
  and P3, Claude r5-2) a new process whose command line resumes the pane's current conversation is never paired with
  it, because the repeated report of the same conversation does not reach the pairing, so its options are dropped and
  the next restore falls back with the marker; (P3, Claude r5-3) a pane shell without line editing (dash, ash, busybox)
  reads at most 4095 characters per line on Linux, so a kept restore line between 4 KiB and 8 KiB loses `--resume <id>`
  while the log says Kept; (P3) `observe_agent_launch` makes three decisions in one body; (P4) `restored_agent_launch`
  returns a three-part tuple each caller passes on unchanged. FR-1, FR-2, FR-4, FR-5, FR-6, FR-9, FR-10, FR-13, FR-14,
  FR-22 and FR-29 reset to No. Fixes assigned to W2 (`BRIEF-W2-round5-fixes.md`): a process that resumes the current
  conversation owns it; a same-process re-read keeps the pairing it had; a new process is compared against the last
  conversation an earlier process owned; for r5-3, a restore line longer than the pane shell's canonical line limit
  falls back with the marker for shells not known to edit lines (bash, zsh and fish keep FR-10's 8 KiB), the same
  rule the cmd fix applies (agent decision within FR-10, inferred; flagged for Fitz, who can choose a 4 KiB limit for
  every shell instead); the two code-structure findings are refactored.
- 2026-10-21 17:21:29 EDT - W2 built the round 5 fixes, brought onto this branch: `616d95a9` (a process whose command line resumes the
  pane's current conversation owns it at once, in either arrival order and after a launcher exec; a same-process
  re-read keeps the pairing it had; a new process is compared against the last conversation an earlier process
  owned, carried while a record is unpaired; the seed rule and the pairing rule are now `seed_keeps_record` and
  `session_for_process`, each with a table test), `f99f4cc4` (on Linux and macOS a restore line longer than the
  terminal's canonical line limit, 4,095 bytes on Linux, measured by a pty probe in `w2/canonical_line_probe.out`, and
  1,023 on macOS from `MAX_CANON` in `sys/syslimits.h`, not tested on a Mac, falls back with the marker unless the pane
  shell resolves to bash, zsh or fish), `2f912f34` (`restored_agent_launch` returns an `AgentLaunchSeed` that
  `seed_agent_launch` takes), and `6fe0bf45` (the unreleased session-state docs, in English, Japanese and Chinese, list
  the cmd and canonical shell line limits). The new tests for the pairing and shell fixes failed at `74c893a9`. Checks
  on the same source: fmt and clippy clean, UI hot-path architecture OK, focused filter 369 of 369, full suite 3996
  run, 3994 passed, the same 2 base failures (`logs/nextest-w2-round5.log`); maintenance, integration-assets and
  docs-contract tests pass. Not run: Windows tests and lint (no Windows SDK). Known limit: `herdr agent start` does not
  apply the shell line limit, as it is outside restore. Isolated proofs `w12/ROUND5-PROOF.md` and
  `w12b/CODEX-HANDOFF-PROOF.md` (build `6fe0bf45`, sha256 `3e931102...babd`): a second and third Claude in one pane
  never took the first conversation (T); a second Claude resuming the same conversation kept its own options (S);
  long options in a shell other than bash, zsh or fish fell back with ExceedsResumeLimits while shorter ones were
  kept (D, E); Codex and Claude kept their options across two live handoffs and two restarts (X, Y) and a fallback
  kept its marker (Z). A fallback for ExceedsResumeLimits keeps the saved options on record with the mark, as the
  existing 8 KiB fallback does. FR-1, FR-2, FR-4, FR-5, FR-6, FR-9, FR-10, FR-13, FR-14, FR-22 and FR-29 set to Yes.
- 2026-10-21 17:41:05 EDT - Review round 6 on `0a7ad43b` (Claude run a05333de, Codex run 4d9fa388) found, confirmed by reading the code:
  (P1, Codex) `claude --resume A --fork-session` starts a new conversation, but Herdr reads it as resuming A and pairs
  the fork's options with A; (P1, Codex) the one-word short form `-r<id>` is read as naming no conversation, so a
  process resuming the current conversation that way is never paired; (P3, Claude r6-1) a fallback seed kept through a
  launcher's first read holds options that rank as a full reading, so the agent's own command line after the exec is
  discarded; (P3, Claude r6-2) a process whose command line resumes a conversation other than the pane's current one
  is still given the current one when it is the pane's first process or binds a seed; (P4, Claude r6-3, design
  disagreement with the 2026-10-20 12:59 entry) on macOS an empty argument makes the command line unreadable, a
  fallback cause the user docs do not name; (P4) the earlier-conversation rule is written in two places.
  FR-1, FR-4, FR-5, FR-6, FR-9, FR-10, FR-14, FR-22 and FR-29 reset to No. Fixes assigned to W2
  (`BRIEF-W2-round6-fixes.md`): a forked resume owns no conversation until it reports its own; every Claude resume
  form is read; a seeded record is re-decided when the same process shows a fuller reading; a process resuming a
  different conversation is never given the current one; one function decides both conversations. For r6-3 the docs
  name the macOS empty-argument fallback and the macOS reader is left as it is, because it cannot be built or tested
  here (agent decision within FR-10, inferred; flagged for Fitz).
- 2026-10-21 17:59:18 EDT - W2 built the round 6 fixes, fast-forwarded onto this branch: `c8fe8de8` (the shared option parser returns an
  option's value in every form, so `-r<id>` and short-option groups such as `-pr <id>` name the resumed conversation;
  a Claude command line with `--fork-session` names none; Codex 0.163.0 forks with its own `codex fork` subcommand,
  which was already not read as a resume), `a392782e` (a seed kept through a reading that told less is decided again
  when the same process shows a better one, such as the agent a launcher execs; a process whose command line resumes
  a conversation other than the pane's current one owns none, except a re-read of the same process, which keeps its
  pairing), `ff1bc02a` (one function decides the owned and the carried earlier conversation). Two existing tests
  changed because they asserted the behavior round 6 called slips. The new tests failed at `21167dfc`. Checks: fmt and
  clippy clean, UI hot-path OK, focused filter 379 of 379, full suite 4000 run, 3998 passed, the same 2 base failures
  (`logs/nextest-w2-round6.log`); maintenance, integration-assets and docs-contract tests pass. Isolated proof
  `w13/ROUND6-PROOF.md` (build `ff1bc02a`, sha256 `35712e23...96e9dd`): a fork with `acceptEdits` left the plan-mode
  conversation saved without options and it came back bare with the marker (F); a second Claude resuming with
  `-r<id>` kept its own options and mode (A); an `agent start` control kept plan mode (Y). The fork's own new
  conversation never reached Herdr, the same separate defect as a second Claude's startup report (flagged, not
  changed). The r6-1 and r6-2 cases are covered by automated tests only, because they need a restore whose command
  starts no agent. FR-1, FR-4, FR-5, FR-6, FR-9, FR-10, FR-14, FR-22 and FR-29 set to Yes.
- 2026-10-26 16:30:14 EDT - Review round 7 on `e2095da4` (Claude run 853d8575, Codex run 4676e592) found, confirmed by
  reading the code: (P1, Claude r7-1) a seed and a pane's first process take the pane's current conversation, which
  can be a leftover: a pane restored with `resume_agents_on_restore = false`, or whose typed resume command started
  no agent, keeps its saved id, so a Claude started there by hand with bypass is saved beside the old conversation
  with the bypass option; (P1, Codex r7-pairing) a launcher read before its exec is paired with the seeded
  conversation, and when the exec'd agent resumes a different one the pairing is kept while the options become the
  new agent's; (P3, r7-2) a program name followed only by empty arguments reads as an agent launched without options;
  (P3, r7-3, design disagreement) the reviewers propose that the hooks report the agent's process id instead of
  Herdr inferring the pairing; (P4, r7-4) an unsupported-option reading ranks like a parsed one; (P4, r7-5)
  `shell_quote` is duplicated in the Linux and macOS files; (P4, r7-6) the macOS shell exemption from the line limit
  was never tested on a Mac; (P4, r7-7) the rejected option name is not logged; (P4, r7-8, design disagreement with
  FR-10) apostrophes still always fall back, and the docs do not name control characters; (P4, r7-9) the typed
  restore line enters the shell history. FR-1 to FR-7, FR-9, FR-10, FR-12, FR-13, FR-14, FR-21, FR-22, FR-29 and FR-30
  reset to No. Fixes assigned to W2 (`BRIEF-W2-round7-fixes.md`): a seed names the conversation its own launch runs and
  a pairing Herdr assumed is decided again on a better reading of the same process; an all-empty argument list is
  unreadable; the unsupported-option reading gets its own rank; one `shell_quote`; the rejected option name is logged
  at debug level, the name only (FR-30); the typed restore line starts with a space on Unix shells. Agent decisions,
  inferred, within FR-10 and FR-14 (G61): on macOS the 1,023-byte limit applies to every shell, the safe direction
  while it is untested there; hook-reported process ids (r7-3) are not part of this task because they change the
  integration scripts and their version, and the evidence rule above closes the cases found. r7-8: FR-10's apostrophe
  limit stays as approved; whether to lift it for bash, zsh and fish was asked through Portal (report 000047,
  pending); the docs will name control characters.
- 2026-10-26 17:12:02 EDT - W2 built the round 7 fixes (`BRIEF-W2-round7-fixes.md` and `BRIEF-W2-round7-followup.md`),
  cherry-picked onto this branch: `7a7b456b` (an unsupported-option reading ranks below a parsed one), `b1816d0d` (a
  seed names the conversation its own launch runs; a record knows whether its pairing is confirmed, by the process's
  command line or the agent's report, or assumed, and an assumed one is decided again on a better reading; a first
  process takes the pane's current conversation only when a live agent reported it; a restore that typed no resume
  command seeds nothing), `952b27b6` (only empty arguments after the program name read as unreadable, on every
  platform), `1a8aec75` (the rejected option name is logged at debug level, never its value), `15157481` (one
  `shell_quote`), `c3e9ec76` (on macOS the 1,023-byte limit applies to every shell), `ba5ea640` (the typed restore
  line starts with a space on Unix shells, counted in the limit), `70f63fb7` (a live handoff or missing-directory seed
  keeps the pairing the old server made, so a hand-started agent keeps its options across a handoff; the orchestrator
  found this gap in W2's first version, which had changed the handoff test to hide it). The new tests failed at
  `e2095da4` (the handoff tests at W2's `60e1f5ca`). Existing tests that asserted the old pairing were rewritten
  because round 7 called that behavior a slip. Docs `77c885a0` name the new fallback causes (control characters,
  all-empty arguments, a hand-started agent after a restore that did not resume) and the history-skipping space.
  Checks: fmt and clippy clean, UI hot-path OK, full suite 4,008 run, 4,007 passed, 1 known base failure
  (`logs/nextest-w2-round7b.log`); maintenance, integration-assets and docs-contract tests pass. Not checked: macOS
  and Windows compilation (no cross target here).
- 2026-10-26 19:41:36 EDT - Live check `w14/ROUND7-PROOF.md` (build `70f63fb7`) matched for the handoff, restart,
  history and resume-off pairing cases, and showed that the brief's "a restore that typed no resume command seeds
  nothing" rule (an orchestrator instruction, not a reviewer's) dropped every pane's options at the first save after a
  resume-off restart. W2 then built `b65b9bb0` (a pane restored without resuming its agent keeps its saved options and
  mark, named for the saved conversation and assumed, so no process takes them without evidence) and, on W2's own
  flag, `2fec054b` (a live handoff carries, as an optional field in the private same-install handoff state, whether
  the old server confirmed the pairing and for which process; no frozen endpoint codec, fixture or protocol version is
  touched). The new tests failed before each fix. Live check `w14b/RESUME-OFF-HANDOFF-PROOF.md` (build `2fec054b`,
  sha256 `e9f805ab...7b9a`): options kept through a resume-off restart and a handoff with no agent, a hand-started
  bypass Claude never saved beside the old conversation, and the panes came back with their options when resume was
  turned on again. Full suite at `2fec054b`, run twice: 4,014 run, 4,011 and 4,012 passed; the known base failure each
  time, plus one different test each run (`agent_start_rejects_a_shell_replaced_by_a_foreground_program`,
  `server_stop_then_restart_restores_pane_history`, `foreground_job_detects_shell_running_command`), each of which
  passed every time alone and lives in code round 7 did not change (flagged: timing under load). Interaction Run set to
  Yes for FR-1, FR-2, FR-4 to FR-7, FR-9, FR-10, FR-12, FR-13, FR-14, FR-22 and FR-29. Live check `w14c/BYPASS-PROOF.md` (build
  `2fec054b`): a Claude in bypass and a Codex in YOLO mode came back that way with the same session ids; FR-3 set to
  Yes. Every row is now Yes or a reasoned N/A.
- 2026-10-26 20:13:12 EDT - Review round 8 on `872754b3` (Claude run f67ec87a, Codex run 23d79860) found, confirmed by reading the
  code: (P1, Codex) two new tests assert Linux behavior on other platforms: the history-space test runs on Windows,
  where the restore command has no leading space, and a restore test expects bash to keep a 4,800-byte line on macOS,
  where every shell is limited to 1,023 bytes; this repository's pull request runs no CI, so neither was compiled;
  (P2, Claude r8-1) a conversation reported while the recorded process is alive is confirmed on that process even
  when another agent process in the pane sent it (a Codex suspended with Ctrl-Z and a second one started, a Claude
  resuming another conversation beside a suspended one, a child agent in the same process group); (P3, r8-2, design
  disagreement) an agent that resumes without naming its id (`claude --continue`, the `--resume` picker,
  `codex resume --last`) and replaces the pane's agent before Herdr sees the earlier one exit is never paired, which
  fails safe but the docs did not name it; (P3, r8-3, flagged) a pane's first observed process takes a reported
  conversation even when the process that reported it exited unseen; (P3, r8-4, design disagreement) the pairing state
  is a set of flags with implicit invariants; (P4, r8-5) a shell respawn keeps the old launch record; (P4, r8-6) the
  missing-directory seed shows the marker on a pane that runs no agent; (P4, r8-7, design disagreement) the bash, zsh
  and fish exemption from the 4,095-byte limit assumes line editing is on; (P4, r8-8) a doc comment sits on the wrong
  item. FR-1 to FR-7, FR-9, FR-10, FR-12, FR-13, FR-14, FR-21, FR-22, FR-23, FR-29 and FR-30 reset to No. Fixes
  assigned to W2 (`BRIEF-W2-round8-fixes.md`): platform-split test expectations and a check of every test this branch
  added; a report is confirmed on the recorded process only when that process is the only Claude or Codex process in
  the pane's current foreground job, otherwise the record stays unpaired and the next observation decides; the
  pairing state becomes an enum of states with no behavior change (agent decision, inferred, G61: the unit changed in
  seven rounds and the enum makes the invariants impossible to break); the respawn reset; the missing-directory seed
  saves its launch without the marker; the doc comment. Agent decisions, inferred, within FR-10 and FR-14: r8-2 and
  r8-7 are named in the docs (`session-state.mdx` and translations) instead of changed, because both fail safe or
  need an unusual shell setup, and changing them adds pairing rules or makes every long Linux restore fall back;
  r8-3 is a known limit (below), because closing it needs the agent's process id in its report (the r7-3 proposal);
  W2 will also try to compile the tests for macOS and Windows with the private toolchain.
- 2026-10-26 20:51:27 EDT - W2 built the round 8 fixes, cherry-picked onto this branch: `99d8c6d1` (platform-split test expectations;
  no other Linux-only expectation found in the tests this branch added), `aa80245e` (doc comment), `58d3eb09` (a shell
  respawn drops the launch record), `e8d97235` (the missing-directory seed saves its launch without the marker),
  `fa1a4590` (the launch record holds its pairing as a state enum: `Started`, `Saved`, `Bound` with a `Confirmed`,
  `Assumed` or `Unpaired` pairing; no behavior change), `ee3144a8` (a reported conversation is paired with the recorded
  process only when it is the only Claude or Codex process in the pane's foreground job, read when the report arrives;
  on Windows the foreground job is the one agent process Herdr selects, so a child agent under it is not told apart).
  The new tests failed at `872754b3`. Docs `de5cbaa5` name the shared foreground job as a fallback case. Checks: fmt and
  clippy clean, UI hot-path OK, full suite 4,019 run, 4,018 passed, the known base failure only
  (`logs/nextest-w2-round8.log`); maintenance, integration-assets and docs-contract tests pass; `cargo check --tests
  --target aarch64-apple-darwin` passes; the Windows check stops in the vendored libghostty-vt build without the
  Windows SDK, whose license setup was not run because accepting it is not this job's call. Live check
  `w15/ROUND8-PROOF.md` (build `ee3144a8`, sha256 `78262b24...2834`): every agent's foreground job was one process;
  Claude typed by hand and via `agent start` and two hand-typed Codex agents kept their options over a restart; after
  `/clear` the new conversation kept its options; with a Codex suspended by Ctrl-Z and a second Codex started, the
  first one's options never appeared beside the second one's conversation, and each came back with its own.
- 2026-10-26 21:00:27 EDT - Regression check `w15b/REGRESSION-PROOF.md` (build `ee3144a8`), because the state enum touched every
  pairing path: Claude typed by hand in plan mode, Claude in bypass via `agent start` and a hand-typed Codex in YOLO
  mode kept their options and modes across a live handoff and two restarts; a pane whose options were removed came
  back bare with `launch options not kept` and saved the mark; with resume off no pane showed the marker and every pane
  kept its saved launch, also across a handoff with no agent running; with resume on again each came back as before.
  Interaction Run set to Yes for FR-1 to FR-7, FR-9, FR-10, FR-12, FR-13, FR-14, FR-22, FR-23 and FR-29. Every row is
  now Yes or a reasoned N/A.
- 2026-10-26 21:32:33 EDT - Review round 9 on `30ae18ad` (Claude run e1f13d66, Codex run 2d117179) found, confirmed by reading the
  code: (P1, Codex r9-pairing-order) the foreground-job check from round 8 runs only for a bound record, so a child
  agent whose report arrives before the parent is first observed, or while a seed is unbound, has its conversation
  paired with the parent's options; (P2, Claude r9-1) Claude Code runs its tool shells in their own process group, so
  a nested `claude --resume` started by the agent's Bash tool is never in the foreground job, and its report confirms
  the parent's options on the nested conversation (the Claude hook, unlike the Codex hook, does not skip nested
  sessions; that the nested report replaces the pane's conversation at all predates this change); (P3, r9-2) two
  agents in one job inside the first-observation window; (P3, r9-4) a report dropped by the job check is final for
  that process, because `seen_session` is set first; (P4, r9-3, design disagreement) an assumed pairing is handed off
  as an unbound seed; (P4, cs-1) the job reader is threaded through every event arm; (P4, cs-2) the detection loop is
  duplicated, which predates this change; (P4, cs-3) an unreachable seed arm. FR-1 to FR-7, FR-9, FR-10, FR-13,
  FR-14, FR-21, FR-22, FR-23 and FR-29 reset to No. Agent decision, inferred (G61, the next approach when one fails):
  the pairing findings since round 6 share one cause, Herdr not knowing which process sent a report. The hooks send
  their report from a short Python process over the server's Unix socket, so the server reads the sender's process id
  from the socket and walks up to the nearest Claude or Codex process; a report pairs only with that process, and a
  process is paired at first observation only with a conversation it reported. No integration asset, version or API
  method shape changes. When the sender cannot be read the current rules stay, with the job check applied to
  report-first observations and unbound seeds and `seen_session` set only when a pairing is decided. Also assigned:
  an assumed handoff variant, the job reader read only for conversation reports, and a `Saved` seed that carries its
  session. cs-2 is left as a follow-up because it predates this change (`BRIEF-W2-round9-fixes.md`).
- 2026-10-26 22:11:26 EDT - W2 built the round 9 change, cherry-picked onto this branch: `0aec7bb2` (each seed carries its own
  conversation; the unreachable arm is gone), `e6ce5c0f` (an assumed pairing is handed off bound to its process),
  `8a66d479` (the job and reporter are read only for Claude and Codex session reports), `b0ab1307` (the API server
  reads the peer process id of each connection, Linux `SO_PEERCRED`, macOS `LOCAL_PEERPID`, Windows the named pipe
  client pid; for `pane.report_agent_session` it walks up parent processes to the nearest Claude or Codex process, and
  a report pairs only with that process; the client-shell endpoint, used by the TUI, SSH and `--machine`, passes no
  peer and keeps the fallback rules, which now also check the job for report-first observations and unbound seeds and
  set `seen_session` only when a pairing is decided). No fixture, schema, codec, protocol version, integration asset or
  method shape changed. Agent decisions by W2, inferred: Claude or Codex parents directly above the agent count as the
  reporter (npm's Codex launcher); with an unknown reporter and an unreadable or agent-less job, the report stays
  undecided. A live probe walked from a test process to its Claude ancestor four levels up (`w2/round9/`). The new
  tests failed at `30ae18ad`. Checks: fmt and clippy clean, macOS test compile clean, UI hot-path OK, full suite 4,029
  run, 4,028 passed, the known base failure only (`logs/nextest-w2-round9.log`); maintenance, integration-assets and
  docs-contract tests pass. Windows is not compiled here (the Windows SDK is not set up). Docs `89294591` say the
  shared-job fallback applies only when Herdr cannot tell which process reported, and that a conversation reported by
  an agent Herdr does not watch is saved without options. Live check `w16/ROUND9-PROOF.md` (build `b0ab1307`, sha256
  `113510ae...5e7e`): a Claude in bypass ran `claude -p --resume <A>` through its Bash tool; the nested Claude ran in
  its own process group, its report replaced the pane's conversation with A (the existing behavior the reviewer named,
  not changed here; flagged), A was never saved beside `--dangerously-skip-permissions` and came back bare with the
  marker. Claude typed by hand (also after `/clear`), via `agent start`, and a hand-typed Codex kept their options over
  a handoff and a restart.
- 2026-10-26 22:20:02 EDT - Regression check `w16b/REGRESSION-PROOF.md` (build `b0ab1307`): the same steps as `w15b` gave the same
  results: bypass Claude, YOLO Codex and plan-mode Claude kept their options over a handoff and two restarts, a pane
  whose options were removed came back bare with the marker, resume off showed no marker and kept every saved launch,
  also across a handoff with no agent running. Interaction Run set to Yes for FR-1 to FR-7, FR-9, FR-10, FR-13, FR-14,
  FR-22 and FR-29. Every row is now Yes or a reasoned N/A.
- 2026-10-26 22:43:01 EDT - Review round 10 on `edec49b5` (Claude run 4a22ae1b, Codex run 6560e3a5) found, confirmed by reading the
  code: (P1, Codex r10-reporter-parent, design disagreement with W2's inferred launcher-parent rule) a native parent
  Claude directly above a Claude its Bash tool `exec`s also counts as the reporter, so the parent can be confirmed on
  the child's conversation; (P2, Claude r10-1, flagged, not reproduced) Codex without `--no-daemon` runs its core in
  a shared daemon outside the pane, and if its hooks run there the daemon becomes the reporter, so a default Codex
  would never save its options; every Codex live check so far used `--no-daemon`; (P2, r10-2) the parent walk runs
  on the app thread after the hook has exited, and a failed walk falls back to the foreground job, which reopens the
  nested-agent case; (P3, r10-3, flagged) a reporter pid reused by a later process is confirmed. FR-1 to FR-7, FR-9,
  FR-13, FR-14, FR-22 and FR-29 reset to No. Agent decisions, inferred (G61): a reporter counts only when the walk
  also reaches the pane's own shell, otherwise the fallback rules apply; the walk runs in the connection thread, and a
  peer whose walk failed leaves the report undecided; every pid used as identity carries its process start time; a
  parent counts as the reporter only when it launches the agent through an interpreter or wrapper. Assigned to W2
  (`BRIEF-W2-round10-fixes.md`); the daemon case is being reproduced live on `b0ab1307` first
  (`BRIEF-W17-codex-daemon-proof.md`).
- 2026-10-26 23:01:01 EDT - Live check `w17/CODEX-DAEMON-PROOF.md` (build `b0ab1307`): Codex 0.163.0 without `--no-daemon` used no
  daemon; its `SessionStart` hook ran as a direct child of the pane's own Codex process, and its options were saved
  and restored with the same session id, as with `--no-daemon`. So r10-1 does not occur with this Codex version; the
  fix for it stays as a safeguard. W2 built the round 10 fixes, cherry-picked onto this branch as `1945cf5b`: the
  connection thread reads the peer's parent chain at once; the nearest Claude or Codex process is the reporter only if
  the chain also reaches the pane's own process, otherwise the fallback rules apply; a peer whose walk found no agent
  below the pane leaves the report undecided; a start time travels with every pid used as identity (Linux
  `/proc/<pid>/stat` field 22, macOS `proc_pidinfo`, Windows `GetProcessTimes`; an unknown start time matches by pid);
  a parent joins the reporter only when it runs the agent through an interpreter or wrapper. W2's inferred choices:
  unknown start times match by pid (older handoffs, processes Windows cannot open); a `bash -c "claude ..."` wrapper
  counts as a launcher. The optional handoff field `process_started` keeps older handoffs loading. The new tests failed
  at `edec49b5` (`w2/round10/base-failures.log`). Checks: fmt and clippy clean, macOS test compile clean, UI hot-path
  OK, full suite 4,037 run, 4,036 passed, the known base failure only (`logs/nextest-w2-round10.log`); maintenance,
  integration-assets and docs-contract tests pass; Windows not compiled here. Docs `ac753e32` name a hook
  that exits before Herdr reads its process as a fallback case.
- 2026-10-26 23:15:52 EDT - Live checks on build `1945cf5b` (sha256 `e126a829...ef86`): `w18/ROUND10-PROOF.md` repeated the round 9
  steps: a Claude in bypass ran `claude -p --resume <A>` through its Bash tool (this time with no launcher stage, the
  nested Claude a direct child of the tool shell, in its own process group); A was never saved beside
  `--dangerously-skip-permissions` and came back bare with the marker; Claude typed by hand (also after `/clear`), via
  `agent start` and a hand-typed Codex kept their options over a handoff and a restart. `w18b/REGRESSION-PROOF.md`
  repeated the regression steps with the same results as `w16b`. Interaction Run set to Yes for FR-1 to FR-7, FR-9,
  FR-13, FR-14, FR-22 and FR-29. Every row is now Yes or a reasoned N/A.
- 2026-10-26 23:37:15 EDT - Review round 11 on `5062331e` (Claude run 57e19963, Codex run 2752b363) found, confirmed by reading the
  code: (P1, Codex r11-partial-ancestry) a parent chain cut short (an unreadable or reused parent, the 16-entry limit)
  after a nested agent counts as outside the pane and falls back to the foreground job, which confirms the parent's
  options on the nested conversation; (P1, Codex r11-observation-pid-reuse) the detection cache compares pid and
  program but not start time, so a replacement agent with the same pid is not observed; (P2, Claude r11-1, flagged,
  not reproduced live) an agent run through an interpreter (an npm Claude, `node .../cli.js`) counts as the launcher
  of a nested Claude its tool `exec`s; (P3, r11-2, design disagreement with W2's inferred round 10 rule) the
  outside-the-pane fallback has no observed case that needs it and fires for a reparented nested agent; (P4, cs-1) the
  observation step is written in both detection loops; (P4, r11-4) Open Questions said none while the apostrophe
  question was pending. FR-1 to FR-7, FR-9, FR-13, FR-14, FR-22 and FR-29 reset to No. Agent decisions, inferred
  (G61): a sender Herdr cannot place inside the pane leaves the report undecided, like an unreadable one, so the
  foreground-job rule applies only with no peer at all (cost: a future Codex that runs its hooks from a shared daemon
  would restore bare with the marker until a rule for it exists); a parent counts as a launcher only in the child's
  process group; the observation cache compares start times; one function holds the observation step. Assigned to W2
  (`BRIEF-W2-round11-fixes.md`). r11-4 fixed here: the apostrophe question is listed under Open Questions.
- 2026-10-26 23:48:18 EDT - W2 built the round 11 fixes, cherry-picked onto this branch as `8c7f9d97`: a walk that ends before reaching
  the pane leaves the report undecided, so the foreground-job rule applies only when no sending process could be read;
  a parent counts as a launcher only in the agent's process group (W2 measured that a node child spawned the way
  npm's launcher does shares node's group, and that Claude's tool shell has its own group); without process groups
  (Windows, an unreadable group) only npm's Codex launcher shape counts; every foreground process carries its start
  time from data each platform already loads, and the observation cache compares it; one function,
  `publish_agent_launch_observation`, serves both detection loops. The new tests failed at `5062331e`
  (`w2/round11/base-failures.log`). Checks: fmt and clippy clean, macOS test compile clean, UI hot-path OK, full suite
  4,040 run, 4,039 passed, the known base failure only (`logs/nextest-w2-round11.log`); maintenance,
  integration-assets and docs-contract tests pass; Windows not compiled here. flagged by W2, predates this change: on
  this machine node names its process `node-MainThread`, and Herdr does not identify `node .../claude` or
  `node .../codex.js` as an agent at all, so an npm-installed Claude or Codex is not detected; filed as a discovery.
  Docs `b9866151` name a hook Herdr cannot trace into its pane as a fallback case.
- 2026-10-27 00:02:42 EDT - Live checks on build `8c7f9d97` (sha256 `efe2f02e...2af1c`): `w19/ROUND11-PROOF.md` repeated the nested
  Claude steps (the nested conversation was never saved beside `--dangerously-skip-permissions` and came back bare
  with the marker; Claude typed by hand, after `/clear` and via `agent start`, and a hand-typed Codex kept their
  options over a handoff and a restart) and `w19b/REGRESSION-PROOF.md` repeated the regression steps with the same
  results as `w18b`. Interaction Run set to Yes for FR-1 to FR-7, FR-9, FR-13, FR-14, FR-22 and FR-29. Every row is
  now Yes or a reasoned N/A.
- 2026-10-27 00:23:42 EDT - Review round 12 on `1fdbf28c` (Claude run 5336b189, Codex run 1b165ea4) found, confirmed by reading the
  code and the PRD: (P2, Codex eng-delivery-source) the FR-25 and FR-27 evidence still named the first delivery
  (`6c030b3d`, `herdr-t2-debug-6c030b3d`) as the deliverable; (P3, Claude r12-1) on the path with no readable sender,
  an unbound restore seed pairs with any reported conversation whatever the foreground job shows; (P3, cs-1) the
  foreground-job evidence compares pids without start times; (P4, r12-2, design disagreement with the round 11
  launcher rule) an interpreter-run agent that started a nested agent in its own process group would count as its
  launcher; (P4, r12-3) the FR-14 evidence named a renamed test and repeated one test. FR-5 and FR-14 reset to No.
  Fixed here: FR-25 and FR-27 name the current candidate through the pull request and `artifacts/CONSUME.md` and mark
  the first build historical; the evidence cells name only tests that exist, once each (renamed tests from rounds 7
  and 10 replaced as well). Assigned to W2 (`BRIEF-W2-round12-fixes.md`): the unbound seed pairs only when the job holds
  exactly one agent, and the job evidence carries start times. Agent decision, inferred (G61): r12-2 is kept, because
  no install shape that triggers it has been observed and the stricter rule would drop the non-exec Python launcher
  shape the tests cover.
- 2026-10-27 00:27:11 EDT - W2 built the round 12 fixes, cherry-picked onto this branch as `e944e58d`: on the path with no readable
  sender, an unbound seed pairs with a report only when the foreground job holds exactly one Claude or Codex process,
  and the job evidence carries start times compared like the reporter's. The new seed test failed at `1fdbf28c`
  (`w2/round12/base-failures.log`); the reused-pid test uses the new evidence type. Checks: fmt and clippy clean, macOS
  test compile clean, UI hot-path OK, full suite 4,042 run, 4,041 passed, the known base failure only
  (`logs/nextest-w2-round12.log`); maintenance, integration-assets and docs-contract tests pass; Windows not compiled
  here.
- 2026-10-27 00:41:44 EDT - Live checks on build `e944e58d` (sha256 `3b54d1f2...ee4e`): `w20/ROUND12-PROOF.md` (nested Claude: the
  nested conversation was never saved beside `--dangerously-skip-permissions` and came back bare with the marker;
  the other agents kept their options over a handoff and a restart) and `w20b/REGRESSION-PROOF.md` (the regression
  steps, same results as `w19b`). FR-5 and FR-14 set to Yes. Every row is now Yes or a reasoned N/A.
- 2026-10-27 00:56:19 EDT - Review round 13 on `d1f0df1b` (Claude run 7ae82898, Codex run c934f83a) found, confirmed by reading the
  code: (P1, Codex r13-report-first-process-match, the same gap as Claude r13-1 at P3) when a report arrives before
  Herdr first observes the agent's process, the observation path checks only whether the evidence is undecided and
  never compares the lone agent in the job (or the reporter) with the observed process, so a different or reused
  process is paired with the earlier conversation, and one test asserted that; the sync path, fixed in round 12, and
  the observation path define an undecided report differently; (P3, r13-2) an observation that left the pairing
  undecided still marks the conversation as seen, so a later decisive report of it is dropped; (P4, r13-3, design
  disagreement with the round 12 decision that a lone agent in the job is the seed's launch) a hand-typed agent that
  reports first after the seeded launch exited takes the seed's options for one detection read. FR-5 and FR-14 reset to
  No. Assigned to W2 (`BRIEF-W2-round13-fixes.md`): one evidence-to-process match used in both arrival orders, keeping
  the singleton's identity through an unbound seed, and a repeated report decided again after an undecided
  observation. Agent decision, inferred (G61): r13-3 is kept; its suggested hardening (clear a typed seed when the pane
  is seen at its shell) is not taken, because the shell is in the foreground while it starts the typed command, so it
  could drop a restore's options.
- 2026-10-27 01:06:11 EDT - W2 built the round 13 fixes, cherry-picked onto this branch: `9a089671` (an observation records the pane's
  conversation as seen only when its pairing took it, so a later report decides again; W2 chose this over comparing
  the guard with the decided session, which would stop a decisive report from confirming a restore's resumed launch,
  inferred), `b38364c6` (one rule, `SessionReportEvidence::names`, decides for both a report and an observation: an
  unreadable or empty job, several agents or an unplaced sender name none; a one-agent job or a reporter names a
  process only when pid and start time match; a seed paired through a one-agent job binds only to that process). The
  test fixture behind the assertion at `app/actions.rs:3438` reported every conversation from the test's own pid; it
  now takes the real sender, and no expected value changed. All six new tests failed at `d1f0df1b`
  (`w2/round13/base-failures*.log`). The round 7 restore tests still pass. Checks: fmt and clippy clean, macOS test
  compile clean, UI hot-path OK, full suite 4,047 run, 4,046 passed, the known base failure only
  (`logs/nextest-w2-round13.log`); maintenance, integration-assets and docs-contract tests pass; Windows not compiled
  here. No docs change: the existing sentence on reports whose sender Herdr cannot tell covers the new undecided
  cases.
- 2026-10-27 01:20:04 EDT - Live checks on build `b38364c6` (sha256 `19a44e88...123a`): `w21/ROUND13-PROOF.md` (nested Claude: never
  saved beside `--dangerously-skip-permissions`, came back bare with the marker; the other agents kept their options
  over a handoff and a restart) and `w21b/REGRESSION-PROOF.md` (same results as `w20b`). FR-5 and FR-14 set to Yes.
  Every row is now Yes or a reasoned N/A.
- 2026-10-27 01:42:41 EDT - Review round 14 on `2ef5aea8` (Claude run 43b85f2b, Codex run 4aacfa2f) found, confirmed by reading the
  code: (P1, Codex r14-same-id-other-process) when another process reports the conversation the record owns, the old
  process's options stay saveable beside it until that process is observed; (P1, Codex r14-stale-report-evidence) a
  report rejected as stale still overwrites the newer report's evidence; (P3, Claude r14-1) a report naming another
  process marks the conversation as seen, so a later report naming the record's process is dropped in one arrival
  order; (P4, r14-2) the assumed handoff test now produces a confirmed pairing. FR-1, FR-2, FR-4, FR-5, FR-13, FR-14
  and FR-22 reset to No. Assigned to W2 (`BRIEF-W2-round14-fixes.md`): `seen_session` set only when the evidence names
  the record's process; a contradicted pairing left unpaired before any early return; evidence stored and synced only
  for accepted reports; the handoff test renamed and an assumed case added. Agent decision, inferred (G61): since
  round 8 each review has found one more event order in the same pairing function, so W2 also adds an exhaustive test
  that runs every short sequence of observations, reports (each evidence kind, accepted or rejected) and exits against
  a reference model built only from the events, and checks after every step that saved options belong to the process
  the model says runs that conversation, and that plain pairings are kept.

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
| FR-1 | A restored Claude agent resumes the same native session id with its original launch options. | stated (request) | No | No | No |
| FR-2 | A restored Codex agent resumes the same native session id with its original launch options. | stated (request) | No | No | No |
| FR-3 | An agent launched with bypass (Claude `--dangerously-skip-permissions` or `--permission-mode bypassPermissions`, Codex `--dangerously-bypass-approvals-and-sandbox`) comes back in bypass. | stated (request; T-79) | Yes | Yes | Yes |
| FR-4 | An agent launched with a restrictive permission, approval or sandbox option comes back with that same option. | stated (T-79) | No | No | No |
| FR-5 | Restore never adds a bypass or permission option the agent was not launched with; there is no global bypass setting. | stated (T-79) | No | No | No |
| FR-6 | The model and effort options an agent was launched with come back unchanged. | stated (request) | Yes | Yes | Yes |
| FR-7 | Every other launch option comes back too (for example extra directories, allowed and disallowed tools, MCP and settings files, system-prompt additions, Codex `-c` overrides and `--no-daemon`), except the parts named in FR-8. | stated (request; #2, K-367 r1) | Yes | Yes | Yes |
| FR-8 | Parts of the launch command that pick or start a conversation (for example Claude `--resume`, `--continue`, `--session-id`, `--fork-session`; Codex `resume <id>`, `--last`), a one-time prompt, and print-and-exit mode are not repeated; the saved session id takes their place. The implementer keeps a per-agent list of these parts. | stated (#2, K-367 r1) | Yes | Yes | Yes |
| FR-9 | Herdr learns each agent's launch options by reading the running agent's own command line, which covers agents typed by hand, started with `herdr agent start`, and started by Portal. Where Herdr does not read the command line (inside WSL), FR-10 applies. | stated (#1, K-366 r1) | Yes | Yes | Yes |
| FR-10 | When Herdr cannot carry an agent's launch options (agent running before the updated Herdr recorded them, inside WSL where Herdr does not read the command line, or options that break the restore command limits of 64 arguments, 8 KiB and no apostrophes), restore resumes the conversation with today's bare command and shows a notice that the launch options were not kept, so the user can relaunch it in the right mode. The notice is shown on the pane until that agent exits: in its top border title, or, when the pane has no top border, in a row reserved at the top of the pane. The agents list is not marked. | stated (#3, K-368 r1; placement K-377 r1; pane only, Fitz 2026-10-20) | Yes | Yes | Yes |
| FR-11 | A Codex agent started with `herdr agent start --kind codex -- <options> resume <id>` is recorded at launch with both its conversation id and its options, so a restore right after starting keeps them. | stated (#4, K-369 r1) | Yes | Yes | Yes |
| FR-12 | Launch options that carry inline text or config values (for example inline `--settings` or `--mcp-config` JSON, `--append-system-prompt` text, Codex `-c key=value` overrides) are saved as-is in Herdr's session file and repeated exactly on restore; Herdr does not filter or redact them. | stated (#5, K-370 r1) | Yes | Yes | Yes |
| FR-13 | Each agent's options are kept per pane and survive a server restart and a live handoff. | stated (request) | No | No | No |
| FR-14 | Each agent's options always belong to its current conversation: when the conversation in a pane changes or the agent exits, Herdr does not restore an old conversation with a different agent's options or a new one with stale options. | inferred | No | No | No |
| FR-15 | Session files written before this change still restore, using the bare command and notice from FR-10. | stated (#3, K-368 r1) | Yes | Yes | Yes |
| FR-16 | Two panes holding the same conversation still resume it only once, as today. | inferred | Yes | Yes | N/A: this change does not alter how duplicate panes are resolved; the deterministic restore tests cover it |
| FR-17 | `[session] resume_agents_on_restore = false` still turns off agent resume entirely. | inferred | Yes | Yes | N/A: no new surface; the setting is read where it was before and the deterministic restore test covers it |
| FR-18 | Restore for every other agent, and for third-party reported resume commands, is unchanged. | stated (request scope: Claude and Codex) | Yes | Yes | Yes |
| FR-19 | Before changing code, the implementer reproduces the loss in an isolated Herdr session: Claude started with bypass, restored, and observed without it; and the same for Codex. The fix is verified against those same cases. | rule G12 | Yes | N/A: a process requirement; the deterministic tests are FR-23 | Yes |
| FR-20 | Reproduction and verification use only Herdr sessions and agents the implementer started for this work; no running user agent is restarted, resumed or taken over. | stated (T-79); rule G4 | Yes | N/A: a process requirement | Yes |
| FR-21 | The unreleased docs (`docs/next/website/src/content/docs/session-state.mdx`, resume command table) say that Claude and Codex restore keeps launch options. | inferred | Yes | Yes | N/A: documentation only; checked by the docs contract and translation parity recipes |
| FR-22 | Options survive repeated restores: an agent restored with its options, then restored again, still has them. | inferred | No | No | No |
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
- Known limit, inferred (review rounds 8 and 9): when Herdr cannot tell which process sent a conversation report
  (no peer process id, as on the client-shell endpoint, or no Claude or Codex ancestor), pairing falls back to process
  order and the foreground job, so two agent processes inside one process-check interval (about 5 seconds) can still
  be paired wrongly. Hook reports on the host where the agents run carry the sender since round 9 (`b0ab1307`).
- Inferred rows that stay inferred because they would not change what is built: FR-14 (options always belong to the
  pane's current conversation), FR-16 (a shared conversation still resumes once), FR-17 (the resume setting still turns
  restore off), FR-21 (unreleased docs updated), FR-22 (options survive repeated restores).

## Open Questions

1. Should launch options that contain an apostrophe (for example an `--append-system-prompt` text with "don't") be
   carried on restore? FR-10 as approved makes them fall back to the bare resume command with the
   `launch options not kept` notice. Options asked: (a) keep the FR-10 limit (recommended: the approved, safe
   behavior); (b) lift it for bash, zsh and fish, which can quote an apostrophe. Asked through Portal in report 000047
   on 2026-10-26 (review round 7, r7-8); pending. Until answered the FR-10 limit stays.

The notice placement question (`t2-fallback-notice-placement`) referred to in the known limits was answered by K-377 r1.

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
