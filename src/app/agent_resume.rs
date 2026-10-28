use std::time::Instant;

use bytes::Bytes;
use ratatui::layout::Rect;

use super::App;

struct PendingAgentResumeCandidate {
    pane_id: crate::layout::PaneId,
    terminal_id: crate::terminal::TerminalId,
    cwd: std::path::PathBuf,
    plan: crate::agent_resume::AgentResumePlan,
    rows: u16,
    cols: u16,
}

impl App {
    pub(crate) fn has_pending_agent_resumes(&self) -> bool {
        self.state
            .terminals
            .values()
            .any(|terminal| terminal.pending_agent_resume_plan.is_some())
    }

    pub(crate) fn sync_pending_agent_resume_deadline(&mut self, now: Instant) {
        if !self.has_pending_agent_resumes() {
            self.pending_agent_resume_deadline = None;
            self.next_agent_resume_at = None;
            return;
        }
        if self.pending_agent_resume_candidates().is_empty() {
            self.pending_agent_resume_deadline = None;
            return;
        }
        if let Some(next) = self.next_agent_resume_at {
            self.pending_agent_resume_deadline = Some(next);
        } else {
            self.pending_agent_resume_deadline
                .get_or_insert(now + super::PENDING_AGENT_RESUME_THEME_WAIT);
        }
    }

    pub(crate) fn pending_agent_resume_due(&self, now: Instant) -> bool {
        self.pending_agent_resume_deadline
            .is_some_and(|deadline| now >= deadline)
    }

    pub(crate) fn start_pending_agent_resumes(
        &mut self,
        now: Instant,
        allow_empty_theme: bool,
    ) -> bool {
        // Geometry/theme events can also enter here; they must not bypass spacing.
        if self.next_agent_resume_at.is_some_and(|next| now < next) {
            return false;
        }
        let pending = self.pending_agent_resume_candidates();
        let mut changed = false;
        for PendingAgentResumeCandidate {
            pane_id,
            terminal_id,
            cwd,
            plan,
            rows,
            cols,
        } in pending
        {
            if self.terminal_runtimes.get(&terminal_id).is_some() {
                continue;
            }
            changed |= self.start_pending_agent_resume(
                pane_id,
                terminal_id,
                cwd,
                plan,
                rows,
                cols,
                allow_empty_theme,
            );
            if changed && !self.startup_per_agent_delay.is_zero() {
                self.next_agent_resume_at = Some(now + self.startup_per_agent_delay);
                self.pending_agent_resume_deadline = self.next_agent_resume_at;
                break;
            }
        }

        if changed {
            self.schedule_session_save();
        }
        if !self.has_pending_agent_resumes() || self.pending_agent_resume_candidates().is_empty() {
            self.pending_agent_resume_deadline = None;
        }
        if !self.has_pending_agent_resumes() {
            self.next_agent_resume_at = None;
        }
        changed
    }

    fn pending_agent_resume_candidates(&self) -> Vec<PendingAgentResumeCandidate> {
        let terminal_area = self.state.view.terminal_area;
        if terminal_area.width == 0 || terminal_area.height == 0 {
            return Vec::new();
        };

        let mut pending = Vec::new();
        for (ws_idx, ws) in self.state.workspaces.iter().enumerate() {
            for (tab_idx, tab) in ws.tabs.iter().enumerate() {
                for info in
                    self.pending_agent_resume_pane_infos(ws_idx, tab_idx, tab, terminal_area)
                {
                    let Some(pane) = tab.panes.get(&info.id) else {
                        continue;
                    };
                    if self
                        .terminal_runtimes
                        .get(&pane.attached_terminal_id)
                        .is_some()
                    {
                        continue;
                    }
                    let Some(terminal) = self.state.terminals.get(&pane.attached_terminal_id)
                    else {
                        continue;
                    };
                    let Some(plan) = terminal.pending_agent_resume_plan.clone() else {
                        continue;
                    };
                    pending.push(PendingAgentResumeCandidate {
                        pane_id: info.id,
                        terminal_id: pane.attached_terminal_id.clone(),
                        cwd: terminal.cwd.clone(),
                        plan,
                        rows: info.inner_rect.height,
                        cols: info.inner_rect.width,
                    });
                }
            }
        }
        pending
    }

    fn pending_agent_resume_pane_infos(
        &self,
        ws_idx: usize,
        tab_idx: usize,
        tab: &crate::workspace::Tab,
        terminal_area: Rect,
    ) -> Vec<crate::layout::PaneInfo> {
        let mut pane_infos =
            derived_pending_agent_resume_pane_infos(&self.state, tab, terminal_area);

        if self.state.active == Some(ws_idx)
            && self
                .state
                .workspaces
                .get(ws_idx)
                .is_some_and(|ws| tab_idx == ws.active_tab_index())
        {
            for visible_info in &self.state.view.pane_infos {
                if let Some(info) = pane_infos
                    .iter_mut()
                    .find(|info| info.id == visible_info.id)
                {
                    *info = visible_info.clone();
                } else {
                    pane_infos.push(visible_info.clone());
                }
            }
        }

        pane_infos
    }

    pub(crate) fn start_pending_agent_resume_for_terminal(
        &mut self,
        terminal_id: &crate::terminal::TerminalId,
        rows: u16,
        cols: u16,
        allow_empty_theme: bool,
    ) -> bool {
        if self.terminal_runtimes.get(terminal_id).is_some() {
            return false;
        }
        let Some((pane_id, cwd, plan)) = self.state.workspaces.iter().find_map(|ws| {
            ws.tabs.iter().find_map(|tab| {
                tab.layout.pane_ids().into_iter().find_map(|pane_id| {
                    let pane = tab.panes.get(&pane_id)?;
                    if &pane.attached_terminal_id != terminal_id {
                        return None;
                    }
                    let terminal = self.state.terminals.get(terminal_id)?;
                    Some((
                        pane_id,
                        terminal.cwd.clone(),
                        terminal.pending_agent_resume_plan.clone()?,
                    ))
                })
            })
        }) else {
            return false;
        };

        let changed = self.start_pending_agent_resume(
            pane_id,
            terminal_id.clone(),
            cwd,
            plan,
            rows,
            cols,
            allow_empty_theme,
        );
        if changed {
            self.schedule_session_save();
        }
        if !self.has_pending_agent_resumes() {
            self.pending_agent_resume_deadline = None;
        }
        changed
    }

    fn start_pending_agent_resume(
        &mut self,
        pane_id: crate::layout::PaneId,
        terminal_id: crate::terminal::TerminalId,
        cwd: std::path::PathBuf,
        plan: crate::agent_resume::AgentResumePlan,
        rows: u16,
        cols: u16,
        allow_empty_theme: bool,
    ) -> bool {
        let host_terminal_theme = self.state.host_terminal_theme;
        if host_terminal_theme.is_empty() && !allow_empty_theme {
            return false;
        }

        // The pane runs the configured shell, so the command is quoted for it.
        let shell = crate::pane::pane_shell(&self.state.default_shell);
        let native_args = plan.argv.get(1..).unwrap_or_default();
        let resume_command = if let Some(binding) = &plan.launcher {
            let session = self
                .state
                .terminals
                .get(&terminal_id)
                .and_then(|terminal| terminal.persisted_agent_session.as_ref());
            let identity_matches = binding.agent == plan.agent
                && binding.session.as_ref().is_none_or(|expected| {
                    session.is_some_and(|session| {
                        session.agent == binding.agent && &session.session_ref == expected
                    })
                });
            // A line longer than the shell takes is cut, and a cut launcher
            // line can run something else, so it is not typed.
            let command = if identity_matches {
                binding
                    .launcher
                    .command(native_args, &shell)
                    .map(crate::platform::shell_restore_line)
                    .and_then(|command| {
                        if crate::platform::interactive_shell_accepts_line(&command, &shell) {
                            Ok(command)
                        } else {
                            Err("Saved launcher command is longer than this shell takes in one line. Shorten the launcher's arguments or environment and restart this session.".into())
                        }
                    })
            } else {
                Err("Saved launcher does not match the native conversation identity".into())
            };
            match command {
                Ok(command) => Some(command),
                Err(error) => {
                    if let Some(terminal) = self.state.terminals.get_mut(&terminal_id) {
                        terminal.pending_agent_resume_plan = None;
                        terminal.restore_error = Some(error);
                        terminal.revision = terminal.revision.saturating_add(1);
                    }
                    return true;
                }
            }
        } else {
            crate::platform::interactive_shell_restore_command(&plan.argv, &shell)
        };
        let Some(resume_command) = resume_command else {
            tracing::warn!(
                pane = pane_id.raw(),
                terminal = %terminal_id,
                agent = %plan.agent,
                "failed to start deferred agent resume with empty argv"
            );
            return false;
        };
        let Some(launch_env) = self
            .find_pane(pane_id)
            .and_then(|(ws_idx, _)| self.pane_launch_env(ws_idx, pane_id, Vec::new()))
        else {
            return false;
        };

        if !cwd.is_dir() {
            if let Some(terminal) = self.state.terminals.get_mut(&terminal_id) {
                terminal.pending_agent_resume_plan = None;
                terminal.restore_error = Some("Saved directory is unavailable. Restore the directory and restart this session.".into());
                terminal.revision = terminal.revision.saturating_add(1);
            }
            return true;
        }

        let launch_options_notice = self
            .state
            .terminals
            .get(&terminal_id)
            .and_then(|terminal| terminal.agent_launch_restore)
            .and_then(|outcome| launch_options_notice(&plan.agent, outcome));
        let runtime = match crate::terminal::TerminalRuntime::spawn_with_initial_history(
            pane_id,
            rows,
            cols,
            cwd,
            self.state.pane_scrollback_limit_bytes,
            host_terminal_theme,
            self.state.host_terminal_appearance,
            crate::pane::PaneShellConfig::new(&self.state.default_shell, self.state.shell_mode),
            &launch_env,
            launch_options_notice.as_deref(),
            self.event_tx.clone(),
            self.render_notify.clone(),
            self.render_dirty.clone(),
        ) {
            Ok(runtime) => runtime,
            Err(err) => {
                tracing::warn!(
                    pane = pane_id.raw(),
                    terminal = %terminal_id,
                    agent = %plan.agent,
                    err = %err,
                    "failed to start shell for deferred agent resume"
                );
                if let Some(terminal) = self.state.terminals.get_mut(&terminal_id) {
                    terminal.pending_agent_resume_plan = None;
                    terminal.restore_error = Some(format!("Could not start the saved shell: {err}. Fix the shell configuration and restart this session."));
                    terminal.revision = terminal.revision.saturating_add(1);
                }
                return true;
            }
        };

        let mut input = resume_command;
        input.push('\r');
        if let Err(err) = runtime.try_send_bytes(Bytes::from(input)) {
            tracing::warn!(
                pane = pane_id.raw(),
                terminal = %terminal_id,
                agent = %plan.agent,
                err = %err,
                "failed to send deferred agent resume command to shell"
            );
            runtime.shutdown();
            return false;
        }

        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        if let Some(terminal) = self.state.terminals.get_mut(&terminal_id) {
            if let Some(mut binding) = plan.launcher {
                binding.observed = false;
                binding.argv = binding.launcher.argv(native_args);
                binding.argv_origin = crate::agent_launcher::LauncherArgvOrigin::CanonicalResume;
                terminal.agent_launcher = Some(binding);
            }
            terminal.pending_agent_resume_plan = None;
            terminal.respawn_shell_on_exit = false;
        }
        true
    }
}

/// The line written into a restored pane whose agent resumed without the
/// options it was launched with, so the user can relaunch it in the right mode.
fn launch_options_notice(
    agent: &str,
    outcome: crate::agent_resume::LaunchOptionsRestore,
) -> Option<String> {
    use crate::agent_resume::{LaunchOptionsRestore, LaunchOptionsUnavailable};

    let LaunchOptionsRestore::NotKept(reason) = outcome else {
        return None;
    };
    let reason = match reason {
        LaunchOptionsUnavailable::ExceedsResumeLimits => "they do not fit in the restore command",
        // Restore falls back only when the options were not saved or do not
        // fit; options Herdr could not read are never saved.
        _ => "Herdr had not recorded them",
    };
    let agent = match crate::detect::parse_agent_label(agent) {
        Some(crate::detect::Agent::Claude) => "Claude",
        Some(crate::detect::Agent::Codex) => "Codex",
        _ => agent,
    };
    Some(format!(
        "\x1b[33mHerdr: this {agent} conversation was resumed without its launch options \
         ({reason}). Relaunch it with the permission mode and options you need.\x1b[0m\r\n"
    ))
}

fn derived_pending_agent_resume_pane_infos(
    state: &crate::app::AppState,
    tab: &crate::workspace::Tab,
    terminal_area: Rect,
) -> Vec<crate::layout::PaneInfo> {
    crate::ui::apply_pane_chrome(
        tab.layout.panes(terminal_area),
        state.pane_borders,
        state.pane_gaps,
        state.pane_outer_borders,
    )
    .into_iter()
    .map(|mut info| {
        let pane_inner =
            crate::ui::pane_content_rect(&state.terminals, tab, info.id, info.rect, info.borders);
        info.inner_rect = stable_terminal_inner_rect(pane_inner);
        info
    })
    .collect()
}

fn stable_terminal_inner_rect(pane_inner: Rect) -> Rect {
    if pane_inner.width <= 4 {
        return pane_inner;
    }

    Rect::new(
        pane_inner.x,
        pane_inner.y,
        pane_inner.width.saturating_sub(1),
        pane_inner.height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn agent_launcher_survives_snapshot_and_actual_deferred_restore() {
        restore_codex_through_launcher(None, serde_json::json!(["resume", "same-uuid"])).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn agent_launcher_restore_carries_saved_launch_options() {
        let options = vec!["-c".to_string(), "model_reasoning_effort='low'".to_string()];
        restore_codex_through_launcher(
            Some(options),
            serde_json::json!(["resume", "same-uuid", "-c", "model_reasoning_effort='low'"]),
        )
        .await;
    }

    /// Restores a launcher-bound Codex pane through a real shell and checks
    /// the arguments the launcher received.
    #[cfg(unix)]
    async fn restore_codex_through_launcher(
        launch_options: Option<Vec<String>>,
        expected_args: serde_json::Value,
    ) {
        let temp = crate::agent_launcher::TestDirectory::new();
        let receipt = temp.path().join("receipt.json");
        let mut original = test_app();
        original.state.workspaces = vec![crate::workspace::Workspace::test_new("launcher-restore")];
        original.state.ensure_test_terminals();
        original.state.active = Some(0);
        let pane = original.state.workspaces[0].tabs[0].root_pane;
        let id = original.state.workspaces[0]
            .terminal_id(pane)
            .unwrap()
            .clone();
        let session = crate::agent_resume::AgentSessionRef::id("same-uuid").unwrap();
        let binding = crate::agent_launcher::BoundAgentLauncher {
                observed: false,
                argv: Vec::new(), argv_origin: crate::agent_launcher::LauncherArgvOrigin::Admission,
            agent: "codex".into(), session: Some(session.clone()),
            launcher: crate::api::schema::AgentLauncher {
                executable: "/usr/bin/python3".into(),
                prefix_args: vec!["-c".into(), "import os,sys,json;open(sys.argv[1],'w').write(json.dumps([sys.argv[2:],os.environ['FIXTURE_ACCOUNT']]))".into(), receipt.to_string_lossy().into()],
                env: [("FIXTURE_ACCOUNT".into(), "assigned".into())].into(),
            },
        };
        let terminal = original.state.terminals.get_mut(&id).unwrap();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:codex".into(),
            agent: "codex".into(),
            session_ref: session,
        });
        terminal.agent_launcher = Some(binding.clone());
        let snapshot = crate::persist::capture(
            &original.state.workspaces,
            &original.state.terminals,
            &original.terminal_runtimes,
            Some(0),
            0,
        );
        let mut snapshot = snapshot;
        for pane in snapshot.workspaces[0].tabs[0].panes.values_mut() {
            if let Some(session) = pane.agent_session.as_mut() {
                session.launch_options = launch_options.clone();
            }
        }
        assert_eq!(snapshot.version, 4);
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let snapshot = serde_json::from_slice(&bytes).unwrap();
        let mut app = test_app();
        let (workspaces, terminals, runtimes) = crate::persist::restore(
            &snapshot,
            None,
            24,
            80,
            0,
            "bash",
            crate::config::ShellModeConfig::NonLogin,
            true,
            app.event_tx.clone(),
            app.render_notify.clone(),
            app.render_dirty.clone(),
        );
        assert!(runtimes.is_empty());
        app.state.workspaces = workspaces;
        app.state.terminals = terminals;
        app.terminal_runtimes = runtimes.into();
        app.state.active = Some(0);
        app.state.default_shell = "bash".into();
        let id = app.state.terminals.keys().next().unwrap().clone();
        assert_eq!(
            app.state.terminals[&id].agent_launcher.as_ref(),
            Some(&binding)
        );
        assert!(app.start_pending_agent_resume_for_terminal(&id, 24, 80, true));
        let applied = app.state.terminals[&id].agent_launcher.as_ref().unwrap();
        assert_eq!(
            applied.argv_origin,
            crate::agent_launcher::LauncherArgvOrigin::CanonicalResume
        );
        assert_eq!(
            applied.argv,
            binding
                .launcher
                .argv(&serde_json::from_value::<Vec<String>>(expected_args.clone()).unwrap())
        );
        assert!(
            !applied.observed,
            "a saved identity alone cannot confirm the new process"
        );
        for _ in 0..100 {
            if receipt.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
        assert_eq!(result, serde_json::json!([expected_args, "assigned"]));
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn agent_launcher_missing_on_restore_refuses_before_native_spawn() {
        let mut app = test_app();
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("launcher-missing")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        let id = app.state.terminals.keys().next().unwrap().clone();
        let terminal = app.state.terminals.get_mut(&id).unwrap();
        terminal.pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
            agent: "codex".into(),
            argv: vec!["codex".into(), "resume".into(), "same-uuid".into()],
            dedupe_key: "fixture".into(),
            launcher: Some(crate::agent_launcher::BoundAgentLauncher {
                observed: false,
                argv: Vec::new(),
                argv_origin: crate::agent_launcher::LauncherArgvOrigin::Admission,
                agent: "codex".into(),
                session: None,
                launcher: crate::api::schema::AgentLauncher {
                    executable: "/__missing_launcher__".into(),
                    prefix_args: vec![],
                    env: Default::default(),
                },
            }),
        });
        assert!(app.start_pending_agent_resume_for_terminal(&id, 24, 80, true));
        assert_eq!(app.terminal_runtimes.len(), 0);
        assert!(app.state.terminals[&id]
            .restore_error
            .as_deref()
            .unwrap()
            .contains("launcher"));
        assert!(app.state.terminals[&id].pending_agent_resume_plan.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_launcher_line_longer_than_the_shell_takes_is_not_typed() {
        let mut app = test_app();
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("launcher-too-long")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        // A shell held to the canonical line limit on every platform.
        app.state.default_shell = "dash".into();
        let id = app.state.terminals.keys().next().unwrap().clone();
        let binding = crate::agent_launcher::BoundAgentLauncher {
            observed: false,
            argv: Vec::new(),
            argv_origin: crate::agent_launcher::LauncherArgvOrigin::Admission,
            agent: "claude".into(),
            session: None,
            launcher: crate::api::schema::AgentLauncher {
                executable: "/bin/sh".into(),
                prefix_args: vec![],
                env: [(
                    "T2_PLACEHOLDER".into(),
                    "a".repeat(crate::platform::CANONICAL_LINE_MAX_BYTES),
                )]
                .into(),
            },
        };
        let terminal = app.state.terminals.get_mut(&id).unwrap();
        terminal.agent_launcher = Some(binding.clone());
        terminal.pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
            agent: "claude".into(),
            argv: vec!["claude".into(), "--resume".into(), "same-uuid".into()],
            dedupe_key: "fixture".into(),
            launcher: Some(binding.clone()),
        });
        assert!(app.start_pending_agent_resume_for_terminal(&id, 24, 80, true));
        assert_eq!(
            app.terminal_runtimes.len(),
            0,
            "no shell was started to type a cut line"
        );
        let terminal = &app.state.terminals[&id];
        assert!(terminal
            .restore_error
            .as_deref()
            .unwrap()
            .contains("longer than this shell takes"));
        assert!(terminal.pending_agent_resume_plan.is_none());
        assert_eq!(terminal.agent_launcher.as_ref(), Some(&binding));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn agent_launcher_restore_mismatch_retains_unconfirmed_binding() {
        let mut app = test_app();
        let cwd = std::env::current_dir().unwrap();
        let snapshot = serde_json::from_value(serde_json::json!({
            "version":4, "active":0, "selected":0, "workspaces":[{"identity_cwd":cwd, "tabs":[{
                "layout":{"Pane":0},"zoomed":false,"panes":{"0":{
                    "cwd":cwd,"agent_session":{"source":"herdr:codex","agent":"codex","kind":"id","value":"saved-native-uuid"},
                    "agent_launcher":{"kind":"codex","session":{"kind":"id","value":"different-bound-uuid"},
                        "launcher":{"executable":"/usr/bin/printf","prefix_args":["no execution"],"env":{}},
                        "argv":["/usr/bin/printf","no execution"],"argv_origin":"admission"}
                }}
            }]}]
        })).unwrap();
        let (workspaces, terminals, runtimes) = crate::persist::restore(
            &snapshot,
            None,
            24,
            80,
            0,
            "bash",
            crate::config::ShellModeConfig::NonLogin,
            true,
            app.event_tx.clone(),
            app.render_notify.clone(),
            app.render_dirty.clone(),
        );
        app.state.workspaces = workspaces;
        app.state.terminals = terminals;
        app.terminal_runtimes = runtimes.into();
        app.state.active = Some(0);
        let id = app.state.terminals.keys().next().unwrap().clone();
        assert!(
            app.state.terminals[&id].agent_launcher.is_some(),
            "a persisted identity mismatch is not a confirmed replacement"
        );
        assert!(app.start_pending_agent_resume_for_terminal(&id, 24, 80, true));
        assert_eq!(app.terminal_runtimes.len(), 0);
        assert!(app.state.terminals[&id].restore_error.is_some());
        let saved_again = crate::persist::capture(
            &app.state.workspaces,
            &app.state.terminals,
            &app.terminal_runtimes,
            Some(0),
            0,
        );
        assert_eq!(
            saved_again.version, 4,
            "refused recovery must not erase required launcher on the next save"
        );
    }

    #[cfg(unix)]
    fn test_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        )
    }

    #[tokio::test]
    async fn pending_agent_resume_spacing_survives_events_and_failed_restores() {
        for delay_ms in [100, 250, 0] {
            let config: crate::config::Config = toml::from_str(&format!(
                "[session]\nstartup_per_agent_delay_ms = {delay_ms}"
            ))
            .unwrap();
            let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
            let mut app = App::new(
                &config,
                crate::app::AppPolicy::TEST,
                None,
                api_rx,
                crate::api::EventHub::default(),
            );
            app.state.workspaces = (0..4)
                .map(|_| crate::workspace::Workspace::test_new("restore"))
                .collect();
            app.state.active = Some(0);
            app.state.view.terminal_area = Rect::new(0, 0, 100, 30);
            app.state.ensure_test_terminals();
            let missing = std::env::current_dir()
                .unwrap()
                .join("__missing_resume_cwd__");
            assert!(!missing.exists());
            for terminal in app.state.terminals.values_mut() {
                terminal.cwd = missing.clone();
                terminal.pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
                    launcher: None,
                    agent: "codex".into(),
                    argv: vec!["codex".into()],
                    dedupe_key: terminal.id.to_string(),
                });
            }
            let now = Instant::now();
            app.sync_pending_agent_resume_deadline(now);
            assert!(!app.start_pending_agent_resumes(now, false));
            assert!(app.start_pending_agent_resumes(now, true));
            if delay_ms != 0 {
                let next = now + std::time::Duration::from_millis(delay_ms);
                assert_eq!(
                    app.state
                        .terminals
                        .values()
                        .filter(|t| t.restore_error.is_some())
                        .count(),
                    1
                );
                // Geometry changes clear the wakeup, but must preserve the launch gap.
                app.pending_agent_resume_deadline = None;
                app.sync_pending_agent_resume_deadline(now);
                assert_eq!(app.pending_agent_resume_deadline, Some(next));
                assert!(!app
                    .start_pending_agent_resumes(next - std::time::Duration::from_millis(1), true));
                // A late wakeup must not release every overdue agent in a burst.
                for processed in 2..=4 {
                    let late = now + std::time::Duration::from_secs(processed * 10);
                    assert!(app.start_pending_agent_resumes(late, true));
                    assert_eq!(
                        app.state
                            .terminals
                            .values()
                            .filter(|t| t.restore_error.is_some())
                            .count(),
                        processed as usize
                    );
                }
            }
            assert!(!app.has_pending_agent_resumes());
            assert!(app.pending_agent_resume_deadline.is_none());
            assert!(app.next_agent_resume_at.is_none());
            assert_eq!(
                app.state
                    .terminals
                    .values()
                    .filter(|t| t.restore_error.is_some())
                    .count(),
                4
            );
        }
    }

    #[cfg(unix)]
    fn long_running_test_argv() -> Vec<String> {
        vec!["/bin/sh".into(), "-c".into(), "sleep 5".into()]
    }

    #[cfg(unix)]
    fn marker_resume_test_argv() -> Vec<String> {
        vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf '%s' 'restored agent: shell quoted | marker'; sleep 5".into(),
        ]
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_deferred_restore_keeps_session_reference_without_retrying_elsewhere() {
        for missing_shell in [false, true] {
            let mut app = test_app();
            let workspace = crate::workspace::Workspace::test_new("unavailable");
            let pane_id = workspace.tabs[0].root_pane;
            let terminal_id = workspace.terminal_id(pane_id).unwrap().clone();
            app.state.workspaces = vec![workspace];
            app.state.active = Some(0);
            app.state.ensure_test_terminals();
            if missing_shell {
                app.state.default_shell = "__herdr_missing_resume_shell__".into();
            }
            let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
            if !missing_shell {
                terminal.cwd = std::env::current_dir()
                    .unwrap()
                    .join("__herdr_missing_resume_cwd__");
                assert!(!terminal.cwd.exists());
            }
            let session = crate::agent_resume::PersistedAgentSession {
                source: "herdr:codex".into(),
                agent: "codex".into(),
                session_ref: crate::agent_resume::AgentSessionRef::id("resume-test").unwrap(),
            };
            terminal.persisted_agent_session = Some(session.clone());
            terminal.pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
                launcher: None,
                agent: "codex".into(),
                argv: long_running_test_argv(),
                dedupe_key: "resume-test".into(),
            });
            app.start_pending_agent_resume_for_terminal(&terminal_id, 24, 80, true);
            assert!(app.terminal_runtimes.get(&terminal_id).is_none());
            let terminal = &app.state.terminals[&terminal_id];
            assert!(terminal.pending_agent_resume_plan.is_none());
            assert_eq!(terminal.persisted_agent_session.as_ref(), Some(&session));
            assert!(terminal.restore_error.is_some());
            assert!(!app.has_pending_agent_resumes());
            assert!(!app.start_pending_agent_resume_for_terminal(&terminal_id, 24, 80, true));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pending_agent_resume_waits_for_host_theme_before_launch() {
        let mut app = test_app();
        let workspace = crate::workspace::Workspace::test_new("restored");
        let pane_id = workspace.tabs[0].root_pane;
        let terminal_id = workspace.terminal_id(pane_id).cloned().unwrap();
        let pane_infos = workspace.tabs[0]
            .layout
            .panes(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
        app.state.view.pane_infos = pane_infos;
        let terminal = app
            .state
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist");
        terminal.pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
            launcher: None,
            agent: "codex".into(),
            argv: marker_resume_test_argv(),
            dedupe_key: "herdr:codex\0codex\0Id\0codex-session".into(),
        });

        assert!(!app.start_pending_agent_resumes(Instant::now(), false));
        assert!(app.terminal_runtimes.get(&terminal_id).is_none());

        app.state.host_terminal_theme = crate::terminal_theme::TerminalTheme {
            foreground: Some(crate::terminal_theme::RgbColor {
                r: 220,
                g: 220,
                b: 220,
            }),
            background: Some(crate::terminal_theme::RgbColor {
                r: 20,
                g: 20,
                b: 20,
            }),
            ..Default::default()
        };

        assert!(app.start_pending_agent_resumes(Instant::now(), false));
        assert!(app.terminal_runtimes.get(&terminal_id).is_some());
        let terminal = app
            .state
            .terminals
            .get(&terminal_id)
            .expect("terminal should survive launch");
        assert!(terminal.pending_agent_resume_plan.is_none());
        assert!(!terminal.respawn_shell_on_exit);

        let runtime = app
            .terminal_runtimes
            .get(&terminal_id)
            .expect("pending resume should leave a shell runtime");
        let marker = "restored agent: shell quoted | marker";
        for _ in 0..20 {
            if runtime
                .snapshot_history()
                .is_some_and(|text| text.contains(marker))
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(
            runtime
                .snapshot_history()
                .expect("runtime should expose terminal history")
                .contains(marker),
            "deferred restore should inject the resume argv into the restored shell"
        );

        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pending_agent_resume_can_launch_after_theme_wait_expires() {
        let mut app = test_app();
        let workspace = crate::workspace::Workspace::test_new("restored");
        let pane_id = workspace.tabs[0].root_pane;
        let terminal_id = workspace.terminal_id(pane_id).cloned().unwrap();
        app.state.view.pane_infos = workspace.tabs[0]
            .layout
            .panes(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
            launcher: None,
            agent: "codex".into(),
            argv: long_running_test_argv(),
            dedupe_key: "herdr:codex\0codex\0Id\0codex-session".into(),
        });

        app.sync_pending_agent_resume_deadline(std::time::Instant::now());
        assert!(!app.start_pending_agent_resumes(Instant::now(), false));
        assert!(app.start_pending_agent_resumes(Instant::now(), true));
        assert!(app.terminal_runtimes.get(&terminal_id).is_some());

        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn pending_agent_resume_launches_hidden_panes_with_current_terminal_area() {
        let mut app = test_app();
        let active_workspace = crate::workspace::Workspace::test_new("active");
        let active_pane = active_workspace.tabs[0].root_pane;
        let active_terminal = active_workspace.terminal_id(active_pane).cloned().unwrap();
        let hidden_workspace = crate::workspace::Workspace::test_new("hidden");
        let hidden_pane = hidden_workspace.tabs[0].root_pane;
        let hidden_terminal = hidden_workspace.terminal_id(hidden_pane).cloned().unwrap();
        app.state.view.pane_infos = active_workspace.tabs[0]
            .layout
            .panes(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
        app.state.workspaces = vec![active_workspace, hidden_workspace];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        app.state.host_terminal_theme = crate::terminal_theme::TerminalTheme {
            foreground: Some(crate::terminal_theme::RgbColor {
                r: 220,
                g: 220,
                b: 220,
            }),
            background: Some(crate::terminal_theme::RgbColor {
                r: 20,
                g: 20,
                b: 20,
            }),
            ..Default::default()
        };
        for terminal_id in [&active_terminal, &hidden_terminal] {
            app.state
                .terminals
                .get_mut(terminal_id)
                .expect("test terminal should exist")
                .pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
                launcher: None,
                agent: "codex".into(),
                argv: long_running_test_argv(),
                dedupe_key: format!("herdr:codex\0codex\0Id\0{terminal_id}"),
            });
        }
        app.pending_agent_resume_deadline =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(1));

        let now = Instant::now();
        assert!(app.start_pending_agent_resumes(now, false));
        assert!(app.terminal_runtimes.get(&active_terminal).is_some());
        assert!(app.terminal_runtimes.get(&hidden_terminal).is_none());
        assert!(!app.start_pending_agent_resumes(now, true));
        assert!(
            app.start_pending_agent_resumes(now + std::time::Duration::from_millis(100), false,)
        );
        assert!(app.terminal_runtimes.get(&hidden_terminal).is_some());
        assert!(
            app.pending_agent_resume_deadline.is_none(),
            "launched pending resumes should clear the wakeup deadline"
        );

        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn pending_agent_resume_launches_inactive_tab_panes_with_current_terminal_area() {
        let mut app = test_app();
        let mut workspace = crate::workspace::Workspace::test_new("tabs");
        let active_pane = workspace.tabs[0].root_pane;
        let inactive_tab = workspace.test_add_tab(Some("agents"));
        let inactive_pane = workspace.tabs[inactive_tab].root_pane;
        let inactive_terminal = workspace.tabs[inactive_tab]
            .terminal_id(inactive_pane)
            .cloned()
            .unwrap();
        app.state.view.pane_infos = workspace.tabs[0]
            .layout
            .panes(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        assert!(app
            .state
            .workspaces
            .first()
            .and_then(|ws| ws.tabs[0].terminal_id(active_pane))
            .is_some());
        app.state.host_terminal_theme = crate::terminal_theme::TerminalTheme {
            foreground: Some(crate::terminal_theme::RgbColor {
                r: 220,
                g: 220,
                b: 220,
            }),
            background: Some(crate::terminal_theme::RgbColor {
                r: 20,
                g: 20,
                b: 20,
            }),
            ..Default::default()
        };
        app.state
            .terminals
            .get_mut(&inactive_terminal)
            .expect("inactive tab terminal should exist")
            .pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
            launcher: None,
            agent: "codex".into(),
            argv: long_running_test_argv(),
            dedupe_key: "herdr:codex\0codex\0Id\0inactive-tab-session".into(),
        });

        assert!(app.start_pending_agent_resumes(Instant::now(), false));
        assert!(app.terminal_runtimes.get(&inactive_terminal).is_some());
        assert!(
            app.state
                .terminals
                .get(&inactive_terminal)
                .expect("inactive tab terminal should still exist")
                .pending_agent_resume_plan
                .is_none(),
            "inactive tab restored panes should not wait for tab focus"
        );

        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn pending_agent_resume_launches_zoom_hidden_active_tab_panes() {
        let mut app = test_app();
        let mut workspace = crate::workspace::Workspace::test_new("zoomed");
        let hidden_pane = workspace.tabs[0].root_pane;
        let visible_pane = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.tabs[0].zoomed = true;
        let hidden_terminal = workspace.terminal_id(hidden_pane).cloned().unwrap();
        app.state.view.pane_infos = vec![crate::layout::PaneInfo {
            id: visible_pane,
            rect: ratatui::layout::Rect::new(0, 0, 100, 30),
            inner_rect: ratatui::layout::Rect::new(1, 1, 98, 28),
            scrollbar_rect: None,
            borders: ratatui::widgets::Borders::ALL,
            is_focused: true,
        }];
        app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        app.state.host_terminal_theme = crate::terminal_theme::TerminalTheme {
            foreground: Some(crate::terminal_theme::RgbColor {
                r: 220,
                g: 220,
                b: 220,
            }),
            background: Some(crate::terminal_theme::RgbColor {
                r: 20,
                g: 20,
                b: 20,
            }),
            ..Default::default()
        };
        app.state
            .terminals
            .get_mut(&hidden_terminal)
            .expect("hidden zoom pane terminal should exist")
            .pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
            launcher: None,
            agent: "codex".into(),
            argv: long_running_test_argv(),
            dedupe_key: "herdr:codex\0codex\0Id\0zoom-hidden-session".into(),
        });

        assert!(app.start_pending_agent_resumes(Instant::now(), false));
        assert!(app.terminal_runtimes.get(&hidden_terminal).is_some());
        assert!(
            app.state
                .terminals
                .get(&hidden_terminal)
                .expect("hidden zoom pane terminal should still exist")
                .pending_agent_resume_plan
                .is_none(),
            "zoom-hidden restored panes should not wait for pane focus"
        );

        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn pending_agent_resume_uses_current_terminal_area_for_background_panes() {
        let mut app = test_app();
        let previous_workspace = crate::workspace::Workspace::test_new("previous");
        let previous_pane = previous_workspace.tabs[0].root_pane;
        let previous_terminal = previous_workspace
            .terminal_id(previous_pane)
            .cloned()
            .unwrap();
        let current_workspace = crate::workspace::Workspace::test_new("current");
        app.state.view.pane_infos = previous_workspace.tabs[0]
            .layout
            .panes(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 80, 24);
        app.state.workspaces = vec![previous_workspace, current_workspace];
        app.state.active = Some(1);
        app.state.ensure_test_terminals();
        app.state.host_terminal_theme = crate::terminal_theme::TerminalTheme {
            foreground: Some(crate::terminal_theme::RgbColor {
                r: 220,
                g: 220,
                b: 220,
            }),
            background: Some(crate::terminal_theme::RgbColor {
                r: 20,
                g: 20,
                b: 20,
            }),
            ..Default::default()
        };
        app.state
            .terminals
            .get_mut(&previous_terminal)
            .expect("test terminal should exist")
            .pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
            launcher: None,
            agent: "codex".into(),
            argv: long_running_test_argv(),
            dedupe_key: "herdr:codex\0codex\0Id\0codex-session".into(),
        });

        app.sync_pending_agent_resume_deadline(std::time::Instant::now());
        assert!(app.pending_agent_resume_deadline.is_some());
        assert!(app.start_pending_agent_resumes(Instant::now(), false));
        assert!(app.terminal_runtimes.get(&previous_terminal).is_some());
        assert!(
            app.state
                .terminals
                .get(&previous_terminal)
                .expect("previous terminal should still exist")
                .pending_agent_resume_plan
                .is_none(),
            "background restored panes should not wait for focus once terminal area is known"
        );

        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pending_agent_resume_launches_with_inner_rect_size() {
        let mut app = test_app();
        let mut workspace = crate::workspace::Workspace::test_new("split");
        let pane_id = workspace.test_split(ratatui::layout::Direction::Horizontal);
        let terminal_id = workspace.terminal_id(pane_id).cloned().unwrap();
        app.state.view.pane_infos = vec![crate::layout::PaneInfo {
            id: pane_id,
            rect: ratatui::layout::Rect::new(0, 0, 100, 30),
            inner_rect: ratatui::layout::Rect::new(1, 1, 98, 28),
            scrollbar_rect: None,
            borders: ratatui::widgets::Borders::ALL,
            is_focused: true,
        }];
        app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        app.state.host_terminal_theme = crate::terminal_theme::TerminalTheme {
            foreground: Some(crate::terminal_theme::RgbColor {
                r: 220,
                g: 220,
                b: 220,
            }),
            background: Some(crate::terminal_theme::RgbColor {
                r: 20,
                g: 20,
                b: 20,
            }),
            ..Default::default()
        };
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
            launcher: None,
            agent: "codex".into(),
            argv: long_running_test_argv(),
            dedupe_key: "herdr:codex\0codex\0Id\0codex-session".into(),
        });

        assert!(app.start_pending_agent_resumes(Instant::now(), false));
        assert_eq!(
            app.terminal_runtimes
                .get(&terminal_id)
                .expect("pending resume should launch")
                .current_size(),
            (28, 98)
        );

        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    #[test]
    fn launch_options_notice_explains_each_fallback_and_skips_kept_options() {
        use crate::agent_resume::{LaunchOptionsRestore, LaunchOptionsUnavailable};

        assert_eq!(
            launch_options_notice("claude", LaunchOptionsRestore::Kept),
            None
        );
        let notice = launch_options_notice(
            "claude",
            LaunchOptionsRestore::NotKept(LaunchOptionsUnavailable::NotRecorded),
        )
        .unwrap();
        assert!(notice.contains("Herdr: this Claude conversation was resumed without its launch options (Herdr had not recorded them)."));
        assert!(notice.ends_with("\r\n"));
        let notice = launch_options_notice(
            "codex",
            LaunchOptionsRestore::NotKept(LaunchOptionsUnavailable::ExceedsResumeLimits),
        )
        .unwrap();
        assert!(notice.contains(
            "this Codex conversation was resumed without its launch options (they do not fit in the restore command)."
        ));
        assert!(notice.contains("Relaunch it with the permission mode"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pending_agent_resume_writes_launch_options_notice_before_resume() {
        for (outcome, expect_notice) in [
            (
                crate::agent_resume::LaunchOptionsRestore::NotKept(
                    crate::agent_resume::LaunchOptionsUnavailable::NotRecorded,
                ),
                true,
            ),
            (crate::agent_resume::LaunchOptionsRestore::Kept, false),
        ] {
            let mut app = test_app();
            let workspace = crate::workspace::Workspace::test_new("restored");
            let pane_id = workspace.tabs[0].root_pane;
            let terminal_id = workspace.terminal_id(pane_id).cloned().unwrap();
            app.state.view.pane_infos = workspace.tabs[0]
                .layout
                .panes(ratatui::layout::Rect::new(0, 0, 200, 30));
            app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 200, 30);
            app.state.workspaces = vec![workspace];
            app.state.active = Some(0);
            app.state.ensure_test_terminals();
            let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
            terminal.pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
                agent: "codex".into(),
                argv: long_running_test_argv(),
                dedupe_key: "herdr:codex\0codex\0Id\0codex-session".into(),
                launcher: None,
            });
            terminal.agent_launch_restore = Some(outcome);

            assert!(app.start_pending_agent_resumes(Instant::now(), true));
            let text = app
                .terminal_runtimes
                .get(&terminal_id)
                .unwrap()
                .visible_text();
            assert_eq!(
                text.contains("resumed without its launch options"),
                expect_notice,
                "{outcome:?}: {text}"
            );

            for (_, runtime) in app.terminal_runtimes.drain() {
                runtime.shutdown();
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn empty_launch_option_value_is_restored_quoted() {
        let argv = [
            "claude",
            "--append-system-prompt",
            "",
            "--permission-mode",
            "plan",
        ]
        .map(String::from);
        let Some(crate::agent_resume::AgentLaunchOptions::Known(options)) =
            crate::agent_resume::launch_options_from_process_argv(
                crate::detect::Agent::Claude,
                Some(&argv),
            )
        else {
            panic!("an empty option value should parse");
        };
        assert_eq!(options, argv[1..].to_vec());
        let session = crate::agent_resume::AgentSessionRef::id("claude-session").unwrap();
        let (plan, outcome) = crate::agent_resume::plan_with_launch_options(
            "herdr:claude",
            "claude",
            &session,
            Some(&options),
        )
        .unwrap();
        assert_eq!(
            outcome,
            Some(crate::agent_resume::LaunchOptionsRestore::Kept)
        );
        assert_eq!(
            crate::platform::interactive_shell_restore_command(&plan.argv, "bash").as_deref(),
            Some(
                " claude --append-system-prompt '' --permission-mode plan --resume claude-session"
            )
        );
    }
}
