//! App-loop ownership of a strict launch. The journal worker cannot queue input or
//! authorize exec. Completions return to the app loop for revalidation first.

use std::collections::HashMap;
use std::io;
use std::sync::{mpsc::Sender, Arc};
use std::time::{Duration, Instant};

use crate::api::schema::{
    AgentClaimTrackedStartParams, AgentStartParams, AgentStartState, AgentStartWithLauncherParams,
    AgentStartWithLauncherTrackedParams, Method, ResponseResult, StartDiscardEvidence,
};
use crate::api::ApiRequestMessage;
use crate::events::AppEvent;
use crate::persist::start_records::{
    canonical_request_id, StartDisposition, StartRecord, StartRecords, StartShell,
};
use crate::platform::{self, ProcessIdentity, ProcessInstance};
use crate::terminal::TerminalId;

use super::agents::{available_shell_name, AgentLaunchMetadata, PreparedAgentStart};
use super::api::responses::{encode_error, encode_success};
use super::App;

// The API can read the journal independently of launch; keep a bound on the
// work queued behind a slow disk and leave the app's other events serviceable.
const MAX_IO: usize = 64;

#[derive(Default)]
pub(crate) struct TrackedStarts {
    pub(crate) records: Option<Arc<StartRecords>>,
    io_in_progress: usize,
    admissions: HashMap<String, Option<Reservation>>,
    pending: HashMap<String, PendingStart>,
}

struct Reservation {
    prepared: PreparedAgentStart,
    shell: StartShell,
}

impl StartShell {
    fn read(pid: u32) -> Option<Self> {
        let ProcessIdentity::Running { started } = platform::process_identity(pid) else {
            return None;
        };
        Some(Self { pid, started })
    }

    fn matches(&self, pid: u32) -> bool {
        pid == self.pid
            && platform::process_identity(pid)
                == ProcessIdentity::Running {
                    started: self.started,
                }
    }
}

struct PendingStart {
    runtime_id: TerminalId,
    admission: HandoffStart,
    deadline: Instant,
    authorizing: bool,
}

/// Only the private authenticated handoff carries the material needed to
/// finish an already-dispatched helper. Layout and the journal omit it.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct HandoffStart {
    start_request_id: String,
    original_terminal: String,
    shell: StartShell,
    nonce: String,
    launch: String,
    start: AgentStartWithLauncherParams,
    remaining_ms: u64,
}

impl HandoffStart {
    pub(crate) fn is_pending_in(&self, records: &StartRecords) -> io::Result<bool> {
        if self.remaining_ms == 0
            || self.remaining_ms > super::MAX_AGENT_START_TIMEOUT.as_millis() as u64
            || !crate::terminal::launch_incarnation::is_canonical_token(&self.launch)
        {
            return Ok(false);
        }
        records.pending_claim_matches(
            &self.start_request_id,
            &self.original_terminal,
            &self.nonce,
            self.shell,
        )
    }
}

impl TrackedStarts {
    pub(super) fn reserved(&self, terminal: &TerminalId, name: &str) -> bool {
        self.admissions.values().flatten().any(|reservation| {
            &reservation.prepared.terminal_id == terminal || reservation.prepared.name == name
        })
    }

    pub(crate) fn io_pending(&self) -> bool {
        self.io_in_progress != 0
    }
}

#[derive(Debug)]
pub struct Reply {
    id: String,
    tx: Sender<String>,
}

impl Reply {
    fn success(self, result: ResponseResult) {
        let _ = self.tx.send(encode_success(self.id, result));
    }

    fn refuse(self, code: &str, reason: impl Into<String>) {
        let _ = self.tx.send(encode_error(self.id, code, reason));
    }

    fn recorded(self) {
        self.refuse(
            "start_request_already_recorded",
            "tracked start ID is already reserved",
        );
    }
}

#[derive(Debug)]
pub enum Completion {
    LookedUp(Reply, AgentStartWithLauncherTrackedParams, io::Result<bool>),
    Reserved(Reply, AgentStartWithLauncherTrackedParams, io::Result<()>),
    Authorized(
        Reply,
        AgentClaimTrackedStartParams,
        ProcessInstance,
        io::Result<()>,
    ),
    Status(Reply, String, io::Result<Option<StartRecord>>),
}

impl App {
    pub(crate) fn handle_tracked_start_request(&mut self, msg: ApiRequestMessage) {
        let reply = Reply {
            id: msg.request.id,
            tx: msg.respond_to,
        };
        let request_id = match &msg.request.method {
            Method::AgentStartWithLauncherTracked(params) => &params.start_request_id,
            Method::AgentClaimTrackedStart(params) => &params.start_request_id,
            Method::AgentStartStatus(params) => &params.start_request_id,
            _ => unreachable!("only tracked start methods have this consumer"),
        };
        if !canonical_request_id(request_id) {
            reply.refuse(
                "invalid_start_request_id",
                "expected a canonical lowercase UUID",
            );
            return;
        }
        // Existing IDs win over ordinary target/name validation.
        if matches!(msg.request.method, Method::AgentStartWithLauncherTracked(_))
            && self.tracked_starts.admissions.contains_key(request_id)
        {
            reply.recorded();
            return;
        }
        let Some(records) = self.tracked_starts.records.clone() else {
            reply.refuse(
                "tracked_start_unavailable",
                "this server has no tracked-start writer",
            );
            return;
        };
        if self.tracked_starts.io_in_progress >= MAX_IO {
            reply.refuse("tracked_start_unavailable", "tracked-start journal is busy");
            return;
        }
        match msg.request.method {
            Method::AgentStartWithLauncherTracked(params) => {
                self.tracked_starts
                    .admissions
                    .insert(params.start_request_id.clone(), None);
                self.tracked_io(move || {
                    let result = records.reserved_id(&params.start_request_id);
                    Completion::LookedUp(reply, params, result)
                });
            }
            Method::AgentStartStatus(params) => self.tracked_io(move || {
                let result = records.get(&params.start_request_id);
                Completion::Status(reply, params.start_request_id, result)
            }),
            Method::AgentClaimTrackedStart(params) => {
                let sender = msg.peer_ancestry.as_deref().and_then(<[_]>::first);
                let Some(sender) = sender else {
                    reply.refuse(
                        "launch_attestation_refused",
                        "missing helper process identity",
                    );
                    return;
                };
                let Some(pending) = self.tracked_starts.pending.get(&params.start_request_id)
                else {
                    reply.refuse("launch_attestation_refused", "no matching strict start");
                    return;
                };
                if pending.authorizing
                    || !self.pending_start_matches(pending, &params)
                    || sender.parent != pending.admission.shell.pid
                    || sender.process.started.is_none()
                    || !process_is_current(ProcessInstance {
                        pid: sender.process.pid,
                        started: sender.process.started,
                    })
                {
                    reply.refuse(
                        "launch_attestation_refused",
                        "helper is not the admitted shell's child",
                    );
                    return;
                }
                let process = ProcessInstance {
                    pid: sender.process.pid,
                    started: sender.process.started,
                };
                let terminal = pending.admission.original_terminal.clone();
                let shell = pending.admission.shell;
                if let Some(pending) = self
                    .tracked_starts
                    .pending
                    .get_mut(&params.start_request_id)
                {
                    pending.authorizing = true;
                }
                self.tracked_io(move || {
                    let result = records.authorize(
                        &params.start_request_id,
                        &terminal,
                        &params.nonce,
                        shell,
                        &params.launch_incarnation,
                    );
                    Completion::Authorized(reply, params, process, result)
                });
            }
            _ => unreachable!("only tracked start methods have this consumer"),
        }
    }

    fn tracked_io(&mut self, operation: impl FnOnce() -> Completion + Send + 'static) {
        let events = self.event_tx.clone();
        self.tracked_starts.io_in_progress += 1;
        tokio::task::spawn_blocking(move || {
            let finished = operation();
            let _ = events.blocking_send(AppEvent::TrackedStartFinished(Box::new(finished)));
        });
    }

    pub(super) fn finish_tracked_start_io(&mut self, completion: Completion) {
        self.tracked_starts.io_in_progress -= 1;
        match completion {
            Completion::LookedUp(reply, params, result) => {
                self.reserve_tracked_start(reply, params, result)
            }
            Completion::Reserved(reply, params, result) => {
                self.queue_tracked_start(reply, params, result)
            }
            Completion::Authorized(reply, params, process, result) => {
                self.finish_tracked_claim(reply, params, process, result)
            }
            Completion::Status(reply, start_request_id, result) => {
                let state = result
                    .ok()
                    .flatten()
                    .map_or(AgentStartState::Unknown, |record| self.start_status(record));
                reply.success(ResponseResult::AgentStartStatus {
                    start_request_id,
                    state,
                });
            }
        }
    }

    fn reserve_tracked_start(
        &mut self,
        reply: Reply,
        params: AgentStartWithLauncherTrackedParams,
        lookup: io::Result<bool>,
    ) {
        self.tracked_starts
            .admissions
            .remove(&params.start_request_id);
        let now = Instant::now();
        self.tracked_starts
            .pending
            .retain(|_, pending| pending.deadline > now);
        match lookup {
            Ok(false) => {}
            Ok(true) => return reply.recorded(),
            Err(err) => return reply.refuse("tracked_start_unavailable", err.to_string()),
        }
        let Some(records) = self.tracked_starts.records.clone() else {
            return reply.refuse(
                "tracked_start_unavailable",
                "tracked-start writer went away",
            );
        };
        let start = &params.start;
        let native = AgentStartParams {
            name: start.name.clone(),
            kind: start.kind.clone(),
            pane_id: start.pane_id.clone(),
            args: start.args.clone(),
            timeout_ms: start.timeout_ms,
        };
        let prepared = match self.prepare_agent_start(
            native,
            Some(start.launcher.clone()),
            Some((&params.start_request_id, &params.expected_terminal_id)),
        ) {
            Ok(prepared) => prepared,
            Err(err) => {
                let error = self.agent_start_error_body(err);
                return reply.refuse(&error.code, error.message);
            }
        };
        let shell = self
            .terminal_runtimes
            .get(&prepared.terminal_id)
            .and_then(|runtime| runtime.child_pid())
            .and_then(StartShell::read);
        let Some(shell) = shell else {
            return reply.refuse(
                "tracked_start_unavailable",
                "the pane shell identity cannot be established",
            );
        };
        let Some((_, nonce)) = prepared.attestation.as_ref() else {
            return reply.refuse(
                "tracked_start_unavailable",
                "the pane cannot host a strict launcher",
            );
        };
        let nonce = nonce.clone();
        self.tracked_starts.admissions.insert(
            params.start_request_id.clone(),
            Some(Reservation { prepared, shell }),
        );
        self.tracked_io(move || {
            let result = records.reserve(
                &params.start_request_id,
                &params.expected_terminal_id,
                &nonce,
                shell,
            );
            Completion::Reserved(reply, params, result)
        });
    }

    fn queue_tracked_start(
        &mut self,
        reply: Reply,
        params: AgentStartWithLauncherTrackedParams,
        result: io::Result<()>,
    ) {
        let Some(Some(reservation)) = self
            .tracked_starts
            .admissions
            .remove(&params.start_request_id)
        else {
            return reply.refuse(
                "tracked_start_unresolved",
                "tracked start reservation was lost",
            );
        };
        if let Err(err) = result {
            if err.kind() == io::ErrorKind::AlreadyExists {
                return reply.recorded();
            }
            return reply.refuse("tracked_start_unresolved", err.to_string());
        }
        let prepared = reservation.prepared;
        let terminal_id = prepared.terminal_id.clone();
        let valid_shell = self
            .terminal_runtimes
            .get(&terminal_id)
            .is_some_and(|runtime| {
                runtime
                    .child_pid()
                    .is_some_and(|pid| reservation.shell.matches(pid))
                    && available_shell_name(runtime).is_some()
            });
        let still_free = self
            .state
            .terminals
            .get(&terminal_id)
            .is_some_and(|terminal| {
                !terminal.is_agent_terminal() && terminal.managed_agent_kind().is_none()
            });
        if !valid_shell || !still_free || !self.agent_name_conflicts(&prepared.name, "").is_empty()
        {
            return reply.refuse(
                "tracked_start_unresolved",
                "the reserved shell or name changed before dispatch",
            );
        }
        let Some((launch, nonce)) = prepared.attestation.as_ref() else {
            return reply.refuse(
                "tracked_start_unresolved",
                "the prepared strict launch is missing",
            );
        };
        let deadline = Instant::now() + prepared.timeout;
        let pending = PendingStart {
            runtime_id: terminal_id,
            admission: HandoffStart {
                start_request_id: params.start_request_id.clone(),
                original_terminal: params.expected_terminal_id,
                shell: reservation.shell,
                nonce: nonce.clone(),
                launch: launch.token().to_string(),
                start: params.start,
                remaining_ms: 0, // Filled only when exporting the remaining live budget.
            },
            deadline,
            authorizing: false,
        };
        let started = match self.dispatch_agent_start(prepared) {
            Ok(started) => started,
            Err(_) => {
                return reply.refuse(
                    "tracked_start_unresolved",
                    "could not dispatch the recorded start",
                )
            }
        };
        let launcher = pending.admission.start.launcher.clone();
        let terminal_id = pending.admission.original_terminal.clone();
        self.tracked_starts
            .pending
            .insert(params.start_request_id.clone(), pending);
        reply.success(ResponseResult::AgentStartedWithLauncher {
            agent: started.agent,
            argv: started.argv,
            launcher,
            launch_incarnation: started.launch_incarnation,
            start_request_id: Some(params.start_request_id),
            terminal_id: Some(terminal_id),
        });
    }

    fn pending_start_matches(
        &self,
        pending: &PendingStart,
        params: &AgentClaimTrackedStartParams,
    ) -> bool {
        pending.admission.nonce == params.nonce
            && pending.admission.launch == params.launch_incarnation
            && Instant::now() < pending.deadline
            && self
                .terminal_runtimes
                .get(&pending.runtime_id)
                .and_then(|runtime| runtime.child_pid())
                .is_some_and(|pid| pending.admission.shell.matches(pid))
            && self
                .state
                .terminals
                .get(&pending.runtime_id)
                .is_some_and(|terminal| {
                    terminal.awaits_tracked_start(
                        &params.start_request_id,
                        &params.nonce,
                        &params.launch_incarnation,
                    )
                })
    }

    fn finish_tracked_claim(
        &mut self,
        reply: Reply,
        params: AgentClaimTrackedStartParams,
        process: ProcessInstance,
        result: io::Result<()>,
    ) {
        let pending = self.tracked_starts.pending.remove(&params.start_request_id);
        if let Err(err) = result {
            return reply.refuse("launch_attestation_refused", err.to_string());
        }
        let Some(pending) = pending else {
            return reply.refuse(
                "launch_attestation_refused",
                "the authorized admission is no longer pending",
            );
        };
        if !self.pending_start_matches(&pending, &params)
            || !platform::process_and_parent(process.pid).is_some_and(|sender| {
                sender.process.started == process.started
                    && sender.parent == pending.admission.shell.pid
            })
        {
            return reply.refuse(
                "launch_attestation_refused",
                "live helper binding changed before permission",
            );
        }
        let Some(terminal) = self.state.terminals.get_mut(&pending.runtime_id) else {
            return reply.refuse(
                "launch_attestation_refused",
                "the authorized terminal is gone",
            );
        };
        let Some(kind) = crate::detect::parse_agent_label(&pending.admission.start.kind) else {
            return reply.refuse(
                "launch_attestation_refused",
                "the authorized agent kind is invalid",
            );
        };
        let Ok(launch_incarnation) = terminal.claim_launch_incarnation(
            process,
            pending.admission.shell.pid,
            Some(pending.admission.shell.pid),
        ) else {
            return reply.refuse(
                "launch_attestation_refused",
                "could not bind the authorized helper",
            );
        };
        let argv = pending
            .admission
            .start
            .launcher
            .argv(&pending.admission.start.args);
        AgentLaunchMetadata::from_start(
            kind,
            &pending.admission.start.args,
            Some(pending.admission.start.launcher),
            &argv,
        )
        .apply(terminal);
        self.state.mark_session_dirty();
        self.schedule_session_save();
        reply.success(ResponseResult::AgentTrackedStartAuthorized {
            start_request_id: params.start_request_id,
            nonce: params.nonce,
            launch_incarnation,
        });
    }

    pub(crate) fn pending_starts_for_handoff(&self) -> Vec<HandoffStart> {
        let now = Instant::now();
        self.tracked_starts
            .pending
            .iter()
            .filter_map(|(id, pending)| {
                let mut carried = pending.admission.clone();
                carried.remaining_ms =
                    pending.deadline.saturating_duration_since(now).as_millis() as u64;
                let valid = !pending.authorizing
                    && carried.remaining_ms != 0
                    && self
                        .state
                        .terminals
                        .get(&pending.runtime_id)
                        .is_some_and(|terminal| {
                            terminal.awaits_tracked_start(id, &carried.nonce, &carried.launch)
                        })
                    && self
                        .terminal_runtimes
                        .get(&pending.runtime_id)
                        .and_then(|runtime| runtime.child_pid())
                        .is_some_and(|pid| carried.shell.matches(pid));
                valid.then_some(carried)
            })
            .collect()
    }

    pub(crate) fn import_pending_starts(&mut self, starts: Vec<HandoffStart>) -> io::Result<()> {
        let now = Instant::now();
        let mut used_terminals = std::collections::HashSet::new();
        for admission in starts {
            let mut matching = self
                .terminal_runtimes
                .iter()
                .filter(|(_, runtime)| runtime.child_pid() == Some(admission.shell.pid));
            let (id, _) = matching
                .next()
                .ok_or_else(|| io::Error::other("tracked start lost its original shell"))?;
            if matching.next().is_some()
                || !admission.shell.matches(admission.shell.pid)
                || !used_terminals.insert(id.clone())
                || self
                    .tracked_starts
                    .pending
                    .contains_key(&admission.start_request_id)
            {
                return Err(io::Error::other(
                    "tracked start shell binding is inconsistent",
                ));
            }
            let kind = crate::detect::parse_agent_label(&admission.start.kind)
                .ok_or_else(|| io::Error::other("tracked start has an invalid agent kind"))?;
            let launch = crate::terminal::launch_incarnation::LaunchIncarnation::carried_tracked(
                kind,
                &admission.start_request_id,
                &admission.nonce,
                &admission.launch,
            );
            let terminal = self
                .state
                .terminals
                .get_mut(id)
                .ok_or_else(|| io::Error::other("tracked start terminal was not restored"))?;
            let remaining = Duration::from_millis(admission.remaining_ms);
            terminal.begin_managed_agent(
                admission.start.name.clone(),
                kind,
                now,
                super::AGENT_START_SETTLE_DELAY,
                remaining,
            );
            terminal.admit_launch_incarnation(Some(launch));
            self.tracked_starts.pending.insert(
                admission.start_request_id.clone(),
                PendingStart {
                    runtime_id: id.clone(),
                    admission,
                    deadline: now + remaining,
                    authorizing: false,
                },
            );
        }
        Ok(())
    }

    fn start_status(&self, record: StartRecord) -> AgentStartState {
        let terminal_id = record.terminal_id;
        match record.disposition {
            StartDisposition::DiscardedAtRestore { restored_at } => {
                AgentStartState::DiscardedAtRestore {
                    terminal_id,
                    restored_at,
                    evidence: StartDiscardEvidence::UnclaimedAtColdRestore,
                }
            }
            StartDisposition::Authorized { launch_incarnation } => {
                let agent = self
                    .terminal_targets()
                    .into_iter()
                    .find(|target| target.terminal_id == terminal_id)
                    .and_then(|target| self.agent_info(target.ws_idx, target.pane_id));
                // AgentInfo includes the attestation from that same observation. A
                // holder of the name or pane with another token is inconclusive.
                let Some(agent) = agent.filter(|agent| {
                    agent.terminal_id == terminal_id
                        && !agent.pane_id.is_empty()
                        && agent
                            .state_change_epoch
                            .as_ref()
                            .is_none_or(|epoch| !epoch.is_empty())
                        && agent.state_change_seq <= (1u64 << 53) - 1
                        && agent.name.as_ref().is_some_and(|name| !name.is_empty())
                        && agent.agent.as_ref().is_some_and(|kind| !kind.is_empty())
                        && agent.launch_incarnation.as_deref() == Some(launch_incarnation.as_str())
                }) else {
                    return AgentStartState::Unknown;
                };
                AgentStartState::AuthorizedAttested {
                    terminal_id,
                    launch_incarnation,
                    agent: Box::new(agent),
                }
            }
            StartDisposition::Pending => AgentStartState::Unknown,
        }
    }
}

fn process_is_current(process: ProcessInstance) -> bool {
    process.started.is_some_and(|started| {
        platform::process_identity(process.pid) == ProcessIdentity::Running { started }
    })
}
