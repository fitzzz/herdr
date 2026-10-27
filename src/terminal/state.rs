use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

// Effective state arbitration is intentionally centralized here. Full lifecycle
// Herdr hook integrations are hook-authoritative while live; screen recovery
// remains only for session-only/custom hook paths and fallback detection.
// Process-exit updates clear matching hook authority before recomputing state.

use crate::detect::{Agent, AgentState};
use crate::platform::ProcessInstance;
use crate::terminal::TerminalId;

#[path = "metadata.rs"]
mod metadata;
pub use metadata::{AgentMetadata, AgentMetadataReport, EffectivePresentation};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HookAuthority {
    pub source: String,
    pub agent_label: String,
    pub state: AgentState,
    pub message: Option<String>,
    #[serde(skip, default = "Instant::now")]
    pub reported_at: Instant,
    pub session_ref: Option<crate::agent_resume::AgentSessionRef>,
}

/// How the launch record of a pane paired its saved conversation, carried by a
/// live handoff so the next server seeds that launch the same way. Without it
/// no process was bound to the launch yet.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HandoffAgentLaunch {
    /// Confirmed for this process. `process_started` is absent from a
    /// runtime written before it existed, and where the start time is unknown.
    Confirmed {
        process_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_started: Option<u64>,
    },
    /// Assumed for this process, so another reading of it decides again.
    Assumed {
        process_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_started: Option<u64>,
    },
    /// A saved launch that nothing runs.
    Saved,
}

#[cfg(unix)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct HandoffAgentState {
    authority: HookAuthority,
    sequence: Option<u64>,
    acquisition_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SuppressedFullLifecycleHookReport {
    agent_label: String,
    session_ref: Option<crate::agent_resume::AgentSessionRef>,
    observed_at: Instant,
    reason: FullLifecycleHookSuppressionReason,
    replacement_session_ref: Option<crate::agent_resume::AgentSessionRef>,
    pending_replacement_report: Option<PendingFullLifecycleHookReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingFullLifecycleHookReport {
    authority: HookAuthority,
    seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FullLifecycleHookSuppressionReason {
    HookClear,
    ProcessExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FullLifecycleHookReportRoute {
    Accept { reanchor_sequence: bool },
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StaleFullLifecycleHookSession {
    agent_label: String,
    session_ref: crate::agent_resume::AgentSessionRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedAgentPhase {
    Pending {
        ready_after: Option<Instant>,
        deadline: Instant,
        observed_expected: bool,
    },
    Blocked,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ManagedAgent {
    kind: Agent,
    phase: ManagedAgentPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveStateChange {
    pub previous_agent_label: Option<String>,
    pub previous_known_agent: Option<Agent>,
    pub previous_state: AgentState,
    pub previous_presentation: EffectivePresentation,
    pub agent_label: Option<String>,
    pub known_agent: Option<Agent>,
    pub state: AgentState,
    pub presentation: EffectivePresentation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TerminalTitleChange {
    pub(crate) raw_changed: bool,
    pub(crate) stripped_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalStateMutation {
    pub effective_state_change: Option<EffectiveStateChange>,
    pub session_ref_changed: bool,
    pub agent_released: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentNameOwner {
    agent_label: String,
    session_ref: Option<crate::agent_resume::AgentSessionRef>,
}

/// Whether a record seeded by restore or `herdr agent start` keeps its options,
/// and its mark that the agent was resumed without them, for the first process
/// detection reads: when this platform cannot read the command line (WSL), and
/// when a restore fell back to the bare resume command, whose process shows no
/// options because Herdr typed it, shows a command Herdr does not recognize (a
/// wrapper named after the agent), or resumes the seeded conversation with
/// options a wrapper added. Any other seed takes what the process shows, so a
/// fresh agent never inherits options from a resumed one that already exited.
/// A seed bound to a process while it still holds the seed's options is
/// decided again when that process shows a better reading, such as the agent
/// a launcher script execs in its own process. A saved launch that nothing
/// runs (`SeededLaunch::Saved`) is never kept: any process is one the user
/// started.
fn seed_keeps_record(
    seed: &ObservedAgentLaunch,
    process: &ProcessInstance,
    observed: &crate::agent_resume::AgentLaunchOptions,
    resumed_session: Option<&str>,
) -> bool {
    use crate::agent_resume::{AgentLaunchOptions, LaunchOptionsUnavailable};
    match &seed.state {
        LaunchState::Saved(_) => return false,
        LaunchState::Bound {
            process: bound,
            kept_seed,
            ..
        } if !bound.is(process) || kept_seed.is_none() => return false,
        LaunchState::Started(_) | LaunchState::Bound { .. } => {}
    }
    if matches!(
        observed,
        AgentLaunchOptions::Unavailable(LaunchOptionsUnavailable::CommandLineUnreadable)
    ) {
        return true;
    }
    seed.resumed_without_options
        && (match observed {
            AgentLaunchOptions::Known(observed) => observed.is_empty(),
            AgentLaunchOptions::Unavailable(_) => true,
        } || resumes(resumed_session, seed.session()))
}

/// Whether a command line that resumes `resumed_session` resumes `session`.
fn resumes(
    resumed_session: Option<&str>,
    session: Option<&crate::agent_resume::AgentSessionRef>,
) -> bool {
    session.is_some_and(|session| {
        session.kind == crate::agent_resume::AgentSessionRefKind::Id
            && resumed_session == Some(session.value.as_str())
    })
}

/// How much a reading of a command line tells: a later reading of the same
/// process replaces one that told less.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ReadingRank {
    Unreadable,
    Unrecognized,
    UnsupportedOption,
    Parsed,
}

impl ReadingRank {
    fn of(options: &crate::agent_resume::AgentLaunchOptions) -> Self {
        use crate::agent_resume::{AgentLaunchOptions, LaunchOptionsUnavailable};
        match options {
            AgentLaunchOptions::Known(_) => Self::Parsed,
            AgentLaunchOptions::Unavailable(LaunchOptionsUnavailable::CommandLineUnreadable) => {
                Self::Unreadable
            }
            AgentLaunchOptions::Unavailable(LaunchOptionsUnavailable::UnrecognizedCommand) => {
                Self::Unrecognized
            }
            AgentLaunchOptions::Unavailable(LaunchOptionsUnavailable::UnsupportedOption) => {
                Self::UnsupportedOption
            }
            // Only a seed holds these, and a seed bound to its process records
            // the rank of the reading it was kept through.
            AgentLaunchOptions::Unavailable(
                LaunchOptionsUnavailable::NotRecorded
                | LaunchOptionsUnavailable::ExceedsResumeLimits,
            ) => Self::Parsed,
        }
    }
}

/// What a reading of a process's command line says about the conversation
/// that process runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResumeReading<'a> {
    /// The command line was not read or not placed, so it says nothing.
    Unknown,
    /// It starts a new conversation, or forks one into a new conversation.
    Nothing,
    /// It resumes this conversation id.
    Resumes(&'a str),
}

impl<'a> ResumeReading<'a> {
    fn of(
        options: &crate::agent_resume::AgentLaunchOptions,
        resumed_session: Option<&'a str>,
    ) -> Self {
        match (options, resumed_session) {
            (crate::agent_resume::AgentLaunchOptions::Known(_), Some(id)) => Self::Resumes(id),
            (crate::agent_resume::AgentLaunchOptions::Known(_), None) => Self::Nothing,
            (crate::agent_resume::AgentLaunchOptions::Unavailable(_), _) => Self::Unknown,
        }
    }
}

/// Where a launch record stands with its process.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LaunchState {
    /// Herdr started the launch (a restore's resume command or `herdr agent
    /// start`), and detection has not read its process yet.
    Started(Pairing),
    /// A pane restored without its agent keeps its saved launch for the next
    /// save; nothing runs it.
    Saved(crate::agent_resume::AgentSessionRef),
    /// Detection read this process, or a live handoff bound the seed to it.
    Bound {
        process: ProcessInstance,
        pairing: Pairing,
        /// While the record still holds a seed's options for this process,
        /// the rank of the reading they were kept through, so a better
        /// reading of the process decides again.
        kept_seed: Option<ReadingRank>,
    },
}

impl LaunchState {
    fn session(&self) -> Option<&crate::agent_resume::AgentSessionRef> {
        match self {
            Self::Started(pairing) | Self::Bound { pairing, .. } => pairing.session(),
            Self::Saved(session) => Some(session),
        }
    }

    fn earlier(&self) -> Option<&crate::agent_resume::AgentSessionRef> {
        match self {
            Self::Started(pairing) | Self::Bound { pairing, .. } => pairing.earlier(),
            Self::Saved(_) => None,
        }
    }
}

/// The conversation a launch owns. Options are saved only beside it, so a
/// wrapper that starts another process never pairs one process's options
/// with the other's conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pairing {
    /// The process's own command line or the agent's own report showed that
    /// it runs this conversation.
    Confirmed(crate::agent_resume::AgentSessionRef),
    /// Assumed, so another reading of the same process decides it again.
    Assumed {
        session: crate::agent_resume::AgentSessionRef,
        earlier: Option<crate::agent_resume::AgentSessionRef>,
    },
    /// No conversation. `earlier` here and in `Assumed` is the last
    /// conversation an earlier process owned, which a later process is never
    /// paired with.
    Unpaired {
        earlier: Option<crate::agent_resume::AgentSessionRef>,
    },
}

impl Pairing {
    fn session(&self) -> Option<&crate::agent_resume::AgentSessionRef> {
        match self {
            Self::Confirmed(session) | Self::Assumed { session, .. } => Some(session),
            Self::Unpaired { .. } => None,
        }
    }

    fn earlier(&self) -> Option<&crate::agent_resume::AgentSessionRef> {
        match self {
            Self::Confirmed(_) => None,
            Self::Assumed { earlier, .. } | Self::Unpaired { earlier } => earlier.as_ref(),
        }
    }
}

/// The conversation the launch record of `process` owns, given the record
/// it replaces, the pane's current conversation, whether a live agent
/// reported it (rather than a restore leaving it) with no evidence of which
/// process did, the conversations processes were shown to run, including
/// what this process's command line resumes, and what that command line says.
fn session_for_process(
    previous: Option<&ObservedAgentLaunch>,
    agent: Agent,
    current_session: Option<&crate::agent_resume::AgentSessionRef>,
    reported_without_evidence: bool,
    shown: &ProcessSessions,
    process: &ProcessInstance,
    reading: ResumeReading,
) -> Pairing {
    // Another reading of the same process, or the seed of the launch that
    // started it.
    let same_launch = previous.filter(|previous| {
        previous.agent == agent && previous.process().is_none_or(|bound| bound.is(process))
    });
    // The last conversation an earlier process owned stays unpaired until
    // the new process reports its own.
    let earlier = match same_launch {
        Some(launch) => launch.state.earlier(),
        None => previous.and_then(|previous| previous.session().or(previous.state.earlier())),
    };
    // The conversation this process was shown to run is its own, until
    // another process is shown to run it.
    if let Some((session, runs)) = shown.session_of(process) {
        return if runs {
            Pairing::Confirmed(session.clone())
        } else {
            Pairing::Unpaired {
                earlier: earlier.cloned(),
            }
        };
    }
    // Shown to run nothing, this process keeps or assumes a conversation no
    // other process was shown to run.
    let unclaimed =
        |session: &&crate::agent_resume::AgentSessionRef| shown.runner(session).is_none();
    if let Some(
        LaunchState::Started(Pairing::Confirmed(session))
        | LaunchState::Bound {
            pairing: Pairing::Confirmed(session),
            ..
        },
    ) = same_launch.map(|launch| &launch.state)
    {
        if unclaimed(&session) {
            return Pairing::Confirmed(session.clone());
        }
    }
    // The conversation the seed's launch named, or an earlier reading of
    // this process assumed.
    let assumed = same_launch
        .and_then(ObservedAgentLaunch::session)
        .filter(unclaimed);
    // A conversation a live agent reported that no earlier process owned,
    // when no evidence of the report's sender was kept.
    let reported = current_session
        .filter(|session| reported_without_evidence && earlier != Some(*session))
        .filter(unclaimed);
    let session = match reading {
        // A command line that resumes a valid conversation id showed this
        // process running it, above.
        ResumeReading::Unknown | ResumeReading::Resumes(_) => assumed.or(reported),
        // A new conversation cannot be the one the seed named, unless no
        // command line can name that one.
        ResumeReading::Nothing => assumed
            .filter(|session| session.kind != crate::agent_resume::AgentSessionRefKind::Id)
            .or(reported),
    };
    let earlier = earlier.cloned();
    match session.cloned() {
        Some(session) => Pairing::Assumed { session, earlier },
        None => Pairing::Unpaired { earlier },
    }
}

/// What the server knew, as a Claude or Codex conversation report arrived,
/// about which process sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionReportEvidence {
    /// The reporter: the nearest process running the report's agent above
    /// the process that sent the report, inside the pane, with the launchers
    /// directly above it that started it.
    Reporter(Vec<ProcessInstance>),
    /// The server read the process that sent the report but could not place
    /// a process running the agent in the pane: none was found below the
    /// pane's own process, or the walk ended before reaching the pane, such as
    /// when a process exited first, so it cannot tell who reported.
    Unidentified,
    /// The server could not read the process that sent the report, such as
    /// for a report that did not come over the local API socket: the Claude
    /// and Codex processes in the pane's foreground job, when the job could be
    /// read.
    ForegroundAgents(Option<Vec<ProcessInstance>>),
}

impl Default for SessionReportEvidence {
    fn default() -> Self {
        Self::ForegroundAgents(None)
    }
}

impl SessionReportEvidence {
    /// The processes this evidence names as the one that reported, or
    /// `None` when the server cannot tell. A reporter names itself and the
    /// launchers directly above it. A foreground job running exactly one
    /// Claude or Codex process names that process, which a launch not yet
    /// bound to a process is taken to be. An unreadable job, a job running
    /// none or more than one, and a sender the server read but could not
    /// place in the pane name none. Processes match by id and start time
    /// ([`ProcessInstance::is`]). A report records what it names in
    /// [`ProcessSessions`], which both reports and observations decide from,
    /// so the order they arrive in does not change the pairing.
    fn named_processes(&self) -> Option<&[ProcessInstance]> {
        match self {
            Self::Reporter(reporter) => Some(reporter),
            Self::ForegroundAgents(Some(agents)) if agents.len() == 1 => Some(agents),
            Self::Unidentified | Self::ForegroundAgents(_) => None,
        }
    }
}

/// The conversation agent processes were last shown to run, oldest first: a
/// process runs the conversation its own command line resumes, or the one a
/// report named it for. A report is newer than the command line its process
/// started with, so a process a report named keeps that conversation over
/// its command line. The processes last shown to run a conversation run it
/// now; one shown to run it before them no longer does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProcessSessions(Vec<ProcessSession>);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessSession {
    processes: Vec<ProcessInstance>,
    session: crate::agent_resume::AgentSessionRef,
    shown_by: ShownBy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShownBy {
    CommandLine,
    /// A report's sender, or a live handoff that carried a confirmed pairing.
    Reporter,
    /// The lone agent in the pane's foreground job as a report arrived.
    ForegroundJob,
}

impl ProcessSessions {
    /// How many facts are kept: processes that exit without detection
    /// binding them are never forgotten otherwise.
    const LIMIT: usize = 8;

    /// A report, or a live handoff, showed `processes` running `session`.
    fn show(
        &mut self,
        processes: &[ProcessInstance],
        session: &crate::agent_resume::AgentSessionRef,
        shown_by: ShownBy,
    ) {
        for process in processes {
            self.forget(process);
        }
        self.push(ProcessSession {
            processes: processes.to_vec(),
            session: session.clone(),
            shown_by,
        });
    }

    /// Detection read `process`'s command line, which resumes `session`, or
    /// no conversation when `None`. A report that named it stays its latest
    /// fact.
    fn read(
        &mut self,
        process: &ProcessInstance,
        session: Option<crate::agent_resume::AgentSessionRef>,
    ) {
        if self
            .of(process)
            .is_some_and(|fact| fact.shown_by != ShownBy::CommandLine)
        {
            return;
        }
        self.forget(process);
        if let Some(session) = session {
            self.push(ProcessSession {
                processes: vec![*process],
                session,
                shown_by: ShownBy::CommandLine,
            });
        }
    }

    fn push(&mut self, fact: ProcessSession) {
        if self.0.len() == Self::LIMIT {
            self.0.remove(0);
        }
        self.0.push(fact);
    }

    /// The process exited, so it runs nothing.
    fn forget(&mut self, process: &ProcessInstance) {
        self.0.retain_mut(|fact| {
            fact.processes.retain(|shown| !shown.is(process));
            !fact.processes.is_empty()
        });
    }

    /// The latest fact about `process`.
    fn of(&self, process: &ProcessInstance) -> Option<&ProcessSession> {
        self.0
            .iter()
            .rev()
            .find(|fact| fact.processes.iter().any(|shown| shown.is(process)))
    }

    /// The latest fact about `session`: the processes that run it now.
    fn runner(&self, session: &crate::agent_resume::AgentSessionRef) -> Option<&ProcessSession> {
        self.0.iter().rev().find(|fact| &fact.session == session)
    }

    /// The conversation `process` runs now, and whether another process was
    /// shown to run it since, which ends `process`'s claim.
    fn session_of(
        &self,
        process: &ProcessInstance,
    ) -> Option<(&crate::agent_resume::AgentSessionRef, bool)> {
        let fact = self.of(process)?;
        let runs = self
            .runner(&fact.session)
            .is_some_and(|runner| runner.processes.iter().any(|shown| shown.is(process)));
        Some((&fact.session, runs))
    }
}

/// The options a session restore or `herdr agent start` launched an agent
/// with, seeded before detection sees its process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLaunchSeed {
    pub agent: Agent,
    pub options: crate::agent_resume::AgentLaunchOptions,
    /// The restore fell back to the bare resume command.
    pub resumed_without_options: bool,
    pub launch: SeededLaunch,
}

/// What runs the conversation a seed names, and that conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeededLaunch {
    /// Herdr typed it: a restore's resume command or `herdr agent start`. The
    /// process it binds to is paired with the conversation on evidence only.
    /// `session` is the one the restore resumes, or the one `herdr agent
    /// start` arguments name.
    Started {
        session: Option<crate::agent_resume::AgentSessionRef>,
    },
    /// The server a live handoff replaced confirmed `session` for `process`,
    /// which the seed is bound to, so that process keeps the pairing whatever
    /// its command line says.
    Paired {
        process: ProcessInstance,
        session: crate::agent_resume::AgentSessionRef,
    },
    /// The server a live handoff replaced assumed `session` for `process`,
    /// which the seed is bound to; another reading of that process decides
    /// again, and a conversation reported meanwhile is checked as for any
    /// bound process.
    Assumed {
        process: ProcessInstance,
        session: crate::agent_resume::AgentSessionRef,
    },
    /// Nothing runs it: a pane restored without resuming its agent, or whose
    /// directory is missing, keeps the saved launch for its next save. No
    /// process keeps its options or its mark, and a conversation reported
    /// meanwhile belongs to another launch.
    Saved(crate::agent_resume::AgentSessionRef),
}

/// The launch options of the Claude or Codex process running in a pane, kept
/// so restore can repeat them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedAgentLaunch {
    pub agent: Agent,
    pub options: crate::agent_resume::AgentLaunchOptions,
    /// The agent was resumed by a restore that fell back to the bare resume
    /// command; stays set while the record is bound to that process.
    pub resumed_without_options: bool,
    state: LaunchState,
}

impl ObservedAgentLaunch {
    /// The agent process whose command line was read; `None` until detection
    /// binds a record seeded by restore or `herdr agent start` to it.
    pub fn process(&self) -> Option<ProcessInstance> {
        match self.state {
            LaunchState::Bound { process, .. } => Some(process),
            LaunchState::Started(_) | LaunchState::Saved(_) => None,
        }
    }

    /// The conversation known to run in this process, or, before detection
    /// binds a seed, in the launch it seeded.
    fn session(&self) -> Option<&crate::agent_resume::AgentSessionRef> {
        self.state.session()
    }

    fn saved_options(&self) -> Option<(Agent, &[String])> {
        match &self.options {
            crate::agent_resume::AgentLaunchOptions::Known(options) => {
                Some((self.agent, options.as_slice()))
            }
            crate::agent_resume::AgentLaunchOptions::Unavailable(_) => None,
        }
    }

    fn resumed_without_options_agent(&self) -> Option<Agent> {
        self.resumed_without_options.then_some(self.agent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecentAgentProcessExit {
    agent: Agent,
    observed_at: Instant,
}

/// Pure state for a server-owned terminal.
///
/// During the migration this is still one-to-one with a pane-backed PTY, but
/// pane/view state no longer owns terminal identity, cwd, labels, or agent
/// metadata.
#[cfg_attr(test, derive(Clone))]
pub struct TerminalState {
    pub id: TerminalId,
    pub cwd: PathBuf,
    pub detected_agent: Option<Agent>,
    pub fallback_state: AgentState,
    fallback_visible_blocker: bool,
    fallback_observed_at: Option<Instant>,
    pub hook_authority: Option<HookAuthority>,
    pub agent_metadata: HashMap<String, AgentMetadata>,
    pub metadata_tokens: crate::metadata_tokens::MetadataTokens,
    pub persisted_agent_session: Option<crate::agent_resume::PersistedAgentSession>,
    /// A live agent reported `persisted_agent_session`, rather than a restore
    /// leaving it or `herdr agent start` arguments naming it.
    persisted_agent_session_reported: bool,
    reported_resume: Option<crate::agent_resume::ReportedAgentResume>,
    reported_resume_revision: u64,
    pub terminal_title: Option<String>,
    pub manual_label: Option<String>,
    pub agent_name: Option<String>,
    agent_name_owner: Option<AgentNameOwner>,
    managed_agent: Option<ManagedAgent>,
    codex_prompt_ready: bool,
    managed_agent_launch_session: Option<crate::agent_resume::PersistedAgentSession>,
    hook_report_sequences: HashMap<String, u64>,
    suppressed_full_lifecycle_hook_reports: HashMap<String, SuppressedFullLifecycleHookReport>,
    stale_full_lifecycle_hook_sessions: HashMap<String, Vec<StaleFullLifecycleHookSession>>,
    metadata_report_sequences: HashMap<String, u64>,
    metadata_report_agents: HashMap<String, Agent>,
    metadata_token_sequence_sources: std::collections::HashSet<String>,
    pub state: AgentState,
    pub last_agent_state_change_seq: Option<u64>,
    pub last_agent_completion_seq: Option<u64>,
    pub revision: u64,
    pub launch_argv: Option<Vec<String>>,
    pub respawn_shell_on_exit: bool,
    recent_agent_process_exit: Option<RecentAgentProcessExit>,
    agent_process_acquisition_pending: bool,
    pub pending_agent_resume_plan: Option<crate::agent_resume::AgentResumePlan>,
    pub restore_error: Option<String>,
    agent_launch: Option<ObservedAgentLaunch>,
    agent_launch_revision: u64,
    /// The conversation last reported to the launch record with evidence of
    /// its sender, which `process_sessions` holds, so the process detection
    /// binds next is paired with it only on that evidence.
    reported_session: Option<crate::agent_resume::AgentSessionRef>,
    /// The conversations agent processes in this pane were shown to run.
    process_sessions: ProcessSessions,
    /// Whether the restore that started this terminal repeated the agent's
    /// launch options.
    pub agent_launch_restore: Option<crate::agent_resume::LaunchOptionsRestore>,
}

impl TerminalState {
    pub fn new(id: TerminalId, cwd: PathBuf) -> Self {
        Self {
            id,
            cwd,
            detected_agent: None,
            fallback_state: AgentState::Unknown,
            fallback_visible_blocker: false,
            fallback_observed_at: None,
            hook_authority: None,
            agent_metadata: HashMap::new(),
            metadata_tokens: crate::metadata_tokens::MetadataTokens::default(),
            persisted_agent_session: None,
            persisted_agent_session_reported: false,
            reported_resume: None,
            reported_resume_revision: 0,
            terminal_title: None,
            manual_label: None,
            agent_name: None,
            agent_name_owner: None,
            managed_agent: None,
            codex_prompt_ready: false,
            managed_agent_launch_session: None,
            hook_report_sequences: HashMap::new(),
            suppressed_full_lifecycle_hook_reports: HashMap::new(),
            stale_full_lifecycle_hook_sessions: HashMap::new(),
            metadata_report_sequences: HashMap::new(),
            metadata_report_agents: HashMap::new(),
            metadata_token_sequence_sources: std::collections::HashSet::new(),
            state: AgentState::Unknown,
            last_agent_state_change_seq: None,
            last_agent_completion_seq: None,
            revision: 0,
            launch_argv: None,
            respawn_shell_on_exit: false,
            recent_agent_process_exit: None,
            agent_process_acquisition_pending: false,
            pending_agent_resume_plan: None,
            restore_error: None,
            agent_launch: None,
            agent_launch_revision: 0,
            reported_session: None,
            process_sessions: ProcessSessions::default(),
            agent_launch_restore: None,
        }
    }

    #[cfg(test)]
    pub fn agent_launch(&self) -> Option<&ObservedAgentLaunch> {
        self.agent_launch.as_ref()
    }

    /// Changes whenever the launch options a session save would record change.
    pub fn agent_launch_revision(&self) -> u64 {
        self.agent_launch_revision
    }

    fn set_agent_launch(&mut self, launch: Option<ObservedAgentLaunch>) {
        let current = self.agent_launch.as_ref();
        let saved_changed = current.and_then(ObservedAgentLaunch::saved_options)
            != launch.as_ref().and_then(ObservedAgentLaunch::saved_options)
            || current.and_then(ObservedAgentLaunch::resumed_without_options_agent)
                != launch
                    .as_ref()
                    .and_then(ObservedAgentLaunch::resumed_without_options_agent)
            || current.map(ObservedAgentLaunch::session)
                != launch.as_ref().map(ObservedAgentLaunch::session);
        self.agent_launch = launch;
        if saved_changed {
            self.agent_launch_revision += 1;
        }
    }

    /// Records the options a session restore or `herdr agent start` launched
    /// the agent with, before detection sees its process.
    pub fn seed_agent_launch(&mut self, seed: AgentLaunchSeed) {
        let state = match seed.launch {
            SeededLaunch::Started {
                session: Some(session),
            } => LaunchState::Started(Pairing::Assumed {
                session,
                earlier: None,
            }),
            SeededLaunch::Started { session: None } => {
                LaunchState::Started(Pairing::Unpaired { earlier: None })
            }
            SeededLaunch::Paired { process, session } => {
                self.process_sessions
                    .show(&[process], &session, ShownBy::Reporter);
                LaunchState::Bound {
                    process,
                    pairing: Pairing::Confirmed(session),
                    kept_seed: None,
                }
            }
            // The reading the old server assumed it on is not handed off, so
            // any readable command line of the process decides again.
            SeededLaunch::Assumed { process, session } => LaunchState::Bound {
                process,
                pairing: Pairing::Assumed {
                    session,
                    earlier: None,
                },
                kept_seed: Some(ReadingRank::Unreadable),
            },
            SeededLaunch::Saved(session) => LaunchState::Saved(session),
        };
        self.set_agent_launch(Some(ObservedAgentLaunch {
            agent: seed.agent,
            options: seed.options,
            resumed_without_options: seed.resumed_without_options,
            state,
        }));
    }

    /// Detection read the command line of a Claude or Codex process;
    /// `resumed_session` is the conversation id that command line resumes.
    pub fn observe_agent_launch(
        &mut self,
        agent: Agent,
        process: ProcessInstance,
        options: crate::agent_resume::AgentLaunchOptions,
        resumed_session: Option<&str>,
    ) {
        let rank = ReadingRank::of(&options);
        let current = self
            .agent_launch
            .as_ref()
            .filter(|current| current.agent == agent);
        // The same process is observed again when its command line became
        // readable, or when a launcher script exec'd the agent in its own
        // process; that replaces a record that could not read or place it.
        if current.is_some_and(|current| match current.state {
            LaunchState::Bound {
                process: bound,
                kept_seed,
                ..
            } => {
                bound.is(&process)
                    && rank <= kept_seed.unwrap_or_else(|| ReadingRank::of(&current.options))
            }
            LaunchState::Started(_) | LaunchState::Saved(_) => false,
        }) {
            return;
        }
        let reading = ResumeReading::of(&options, resumed_session);
        let (options, resumed_without_options, kept_seed) = match current {
            Some(current) if seed_keeps_record(current, &process, &options, resumed_session) => (
                current.options.clone(),
                current.resumed_without_options,
                Some(rank),
            ),
            _ => (options, false, None),
        };
        self.process_sessions.read(
            &process,
            match reading {
                ResumeReading::Resumes(id) => crate::agent_resume::AgentSessionRef::id(id),
                ResumeReading::Nothing | ResumeReading::Unknown => None,
            },
        );
        let current_session = self.current_session_ref();
        let reported_without_evidence = (self
            .hook_authority
            .as_ref()
            .is_some_and(|authority| authority.session_ref.is_some())
            || self.persisted_agent_session_reported)
            && !self
                .reported_session
                .as_ref()
                .is_some_and(|session| Some(session) == current_session);
        let pairing = session_for_process(
            self.agent_launch.as_ref(),
            agent,
            current_session,
            reported_without_evidence,
            &self.process_sessions,
            &process,
            reading,
        );
        self.set_agent_launch(Some(ObservedAgentLaunch {
            agent,
            options,
            resumed_without_options,
            state: LaunchState::Bound {
                process,
                pairing,
                kept_seed,
            },
        }));
    }

    /// The conversation a session save records for this pane.
    fn current_session_ref(&self) -> Option<&crate::agent_resume::AgentSessionRef> {
        self.hook_authority
            .as_ref()
            .and_then(|authority| authority.session_ref.as_ref())
            .or_else(|| {
                self.persisted_agent_session
                    .as_ref()
                    .map(|session| &session.session_ref)
            })
    }

    /// Records which processes a report of the pane's current conversation
    /// named ([`SessionReportEvidence::named_processes`]) and decides the launch
    /// record's pairing again: a bound process runs the conversation it was
    /// last shown to run until another process is shown to run it
    /// ([`ProcessSessions`]), and an unbound launch is paired when the
    /// evidence names it, such as a conversation change inside it (Claude
    /// `/clear`). When the evidence names no process, the report stays
    /// undecided, so a repeated report decides again. A conversation reported
    /// beside a saved launch that nothing runs belongs to another launch.
    pub fn sync_agent_launch_session(&mut self, evidence: &SessionReportEvidence) {
        let Some(session) = self.current_session_ref().cloned() else {
            return;
        };
        self.reported_session = Some(session.clone());
        if let Some(named) = evidence.named_processes() {
            let shown_by = match evidence {
                SessionReportEvidence::Reporter(_) => ShownBy::Reporter,
                SessionReportEvidence::Unidentified
                | SessionReportEvidence::ForegroundAgents(_) => ShownBy::ForegroundJob,
            };
            self.process_sessions.show(named, &session, shown_by);
        }
        let Some(launch) = self.agent_launch.as_mut() else {
            return;
        };
        let owns_it = launch.session() == Some(&session);
        let (pairing, paired) = match &mut launch.state {
            LaunchState::Saved(_) => return,
            LaunchState::Started(pairing) => {
                // A reporter names processes, never an unbound launch, which
                // it owns once detection binds it to one of them.
                let names_launch = evidence
                    .named_processes()
                    .map(|_| matches!(evidence, SessionReportEvidence::ForegroundAgents(_)));
                let paired = match names_launch {
                    Some(true) => Pairing::Confirmed(session.clone()),
                    Some(false) if owns_it => Pairing::Unpaired {
                        earlier: pairing.earlier().cloned(),
                    },
                    // Only a job's lone agent confirms an unbound launch,
                    // which is taken to be that process, so it no longer runs
                    // the conversation once another fact about it is newer.
                    Some(false) | None
                        if matches!(pairing, Pairing::Confirmed(confirmed)
                        if self.process_sessions.runner(confirmed).is_none_or(|runner| {
                            runner.shown_by != ShownBy::ForegroundJob
                        })) =>
                    {
                        Pairing::Unpaired { earlier: None }
                    }
                    Some(false) | None => return,
                };
                (pairing, paired)
            }
            LaunchState::Bound {
                process, pairing, ..
            } => {
                let paired = match self.process_sessions.session_of(process) {
                    Some((shown, true)) => Pairing::Confirmed(shown.clone()),
                    Some((_, false)) => Pairing::Unpaired {
                        earlier: pairing.earlier().cloned(),
                    },
                    None if owns_it && self.process_sessions.runner(&session).is_some() => {
                        Pairing::Unpaired {
                            earlier: pairing.earlier().cloned(),
                        }
                    }
                    None => return,
                };
                (pairing, paired)
            }
        };
        let changed = pairing.session() != paired.session();
        *pairing = paired;
        if changed {
            self.agent_launch_revision += 1;
        }
    }

    /// The launch record, when its options belong to the conversation a
    /// session save records.
    fn launch_for_current_session(&self) -> Option<&ObservedAgentLaunch> {
        self.agent_launch
            .as_ref()
            .filter(|launch| launch.session() == self.current_session_ref())
    }

    /// Whether the pane's agent was resumed without its launch options.
    pub fn resumed_without_launch_options(&self) -> bool {
        self.agent_launch.as_ref().is_some_and(|launch| {
            !matches!(launch.state, LaunchState::Saved(_)) && launch.resumed_without_options
        })
    }

    /// Whether to save beside a session of `agent_label` that it was resumed
    /// without its launch options.
    pub fn resumed_without_launch_options_for_session(&self, agent_label: &str) -> bool {
        let agent = self
            .launch_for_current_session()
            .and_then(ObservedAgentLaunch::resumed_without_options_agent);
        agent.is_some() && crate::detect::parse_agent_label(agent_label) == agent
    }

    /// The options to save beside a session of `agent_label`, when they are
    /// known for that same agent.
    pub fn launch_options_for_session(&self, agent_label: &str) -> Option<Vec<String>> {
        let (agent, options) = self
            .launch_for_current_session()
            .and_then(ObservedAgentLaunch::saved_options)?;
        (crate::detect::parse_agent_label(agent_label) == Some(agent)).then(|| options.to_vec())
    }

    pub fn set_detected_agent_process_at(
        &mut self,
        agent: Agent,
        now: Instant,
    ) -> TerminalStateMutation {
        let starts_acquisition = !self
            .should_ignore_detected_state_under_full_lifecycle_hook(Some(agent), false)
            && !self.detected_state_observed_before_release_suppression(Some(agent), now);
        let mutation = self.set_detected_state_with_screen_signals_at(
            Some(agent),
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            now,
        );
        if starts_acquisition {
            self.codex_prompt_ready = false;
            self.agent_process_acquisition_pending = true;
        }
        mutation
    }

    #[cfg(unix)]
    pub(crate) fn handoff_agent_state(&self) -> Option<HandoffAgentState> {
        if !self.live_full_lifecycle_hook_authority() {
            return None;
        }
        let authority = self.hook_authority.as_ref()?;
        Some(HandoffAgentState {
            authority: authority.clone(),
            sequence: self.hook_report_sequences.get(&authority.source).copied(),
            acquisition_pending: self.agent_process_acquisition_pending,
        })
    }

    /// How the launch record paired the conversation a session save records,
    /// for a live handoff.
    #[cfg(unix)]
    pub(crate) fn handoff_agent_launch(&self) -> Option<HandoffAgentLaunch> {
        match self.launch_for_current_session()?.state {
            LaunchState::Saved(_) => Some(HandoffAgentLaunch::Saved),
            LaunchState::Bound {
                process,
                pairing: Pairing::Confirmed(_),
                ..
            } => Some(HandoffAgentLaunch::Confirmed {
                process_id: process.pid,
                process_started: process.started,
            }),
            LaunchState::Bound {
                process,
                pairing: Pairing::Assumed { .. },
                ..
            } => Some(HandoffAgentLaunch::Assumed {
                process_id: process.pid,
                process_started: process.started,
            }),
            LaunchState::Started(_) | LaunchState::Bound { .. } => None,
        }
    }

    #[cfg(unix)]
    pub(crate) fn restore_handoff_agent_state(&mut self, snapshot: HandoffAgentState) {
        if let Some(sequence) = snapshot.sequence {
            self.hook_report_sequences
                .insert(snapshot.authority.source.clone(), sequence);
        }
        self.detected_agent = crate::detect::parse_agent_label(&snapshot.authority.agent_label);
        self.state = snapshot.authority.state;
        self.hook_authority = Some(snapshot.authority);
        self.agent_process_acquisition_pending = snapshot.acquisition_pending;
    }

    pub(crate) fn finish_agent_process_acquisition(&mut self) -> bool {
        let reached_idle = self.agent_process_acquisition_pending && self.state == AgentState::Idle;
        let suppress_completion = reached_idle && self.recent_agent_process_exit.is_none();
        if reached_idle {
            self.agent_process_acquisition_pending = false;
        }
        suppress_completion
    }

    pub(crate) fn terminal_title_stripped(&self) -> Option<String> {
        self.terminal_title
            .as_deref()
            .and_then(super::stripped_terminal_title)
    }

    pub(crate) fn set_terminal_title(&mut self, title: Option<String>) -> TerminalTitleChange {
        if self.terminal_title == title {
            return TerminalTitleChange::default();
        }
        let previous_stripped = self.terminal_title_stripped();
        self.terminal_title = title;
        let stripped_changed = previous_stripped != self.terminal_title_stripped();
        if stripped_changed {
            self.revision = self.revision.wrapping_add(1);
        }
        TerminalTitleChange {
            raw_changed: true,
            stripped_changed,
        }
    }

    pub fn with_launch_argv(mut self, argv: Vec<String>) -> Self {
        self.launch_argv = Some(argv);
        self
    }

    pub fn with_respawn_shell_on_exit(mut self) -> Self {
        self.respawn_shell_on_exit = true;
        self
    }

    #[cfg(any(windows, test))]
    pub(crate) fn agent_process_exited_within(&self, now: Instant, max_age: Duration) -> bool {
        self.recent_agent_process_exit
            .is_some_and(|exit| now.saturating_duration_since(exit.observed_at) <= max_age)
    }

    pub fn with_pending_agent_resume_plan(
        mut self,
        plan: crate::agent_resume::AgentResumePlan,
    ) -> Self {
        self.pending_agent_resume_plan = Some(plan);
        self
    }

    #[cfg(test)]
    pub fn set_detected_state(
        &mut self,
        agent: Option<Agent>,
        fallback_state: AgentState,
    ) -> Option<EffectiveStateChange> {
        self.set_detected_state_with_visible_blocker(agent, fallback_state, false, false, false)
    }

    #[cfg(test)]
    pub fn set_detected_state_with_mutation(
        &mut self,
        agent: Option<Agent>,
        fallback_state: AgentState,
    ) -> TerminalStateMutation {
        self.set_detected_state_with_screen_signals_at(
            agent,
            fallback_state,
            false,
            false,
            false,
            false,
            Instant::now(),
        )
    }

    #[cfg(test)]
    pub fn set_detected_state_with_visible_blocker(
        &mut self,
        agent: Option<Agent>,
        fallback_state: AgentState,
        visible_blocker: bool,
        _ignored_screen_idle: bool,
        process_exited: bool,
    ) -> Option<EffectiveStateChange> {
        self.set_detected_state_with_screen_signals_at(
            agent,
            fallback_state,
            visible_blocker,
            false,
            false,
            process_exited,
            Instant::now(),
        )
        .effective_state_change
    }

    pub fn set_detected_state_with_screen_signals_at(
        &mut self,
        agent: Option<Agent>,
        fallback_state: AgentState,
        visible_blocker: bool,
        _visible_idle: bool,
        _visible_working: bool,
        process_exited: bool,
        now: Instant,
    ) -> TerminalStateMutation {
        let previous_agent_label = self.effective_agent_label().map(str::to_string);
        let previous_known_agent = self.effective_known_agent();
        let previous_state = self.state;
        let previous_presentation = self.effective_presentation_for_state_at(previous_state, now);
        let previous_detected_agent = self.detected_agent;
        let previous_session = self.current_session_identity_for_persistence();
        let newer_custom_authority = process_exited
            && self.hook_authority.as_ref().is_some_and(|authority| {
                crate::detect::parse_agent_label(&authority.agent_label) == agent
                    && !crate::agent_resume::is_official_agent_source(
                        &authority.source,
                        &authority.agent_label,
                    )
                    && authority.reported_at > now
            });
        let agent_released = process_exited
            && !newer_custom_authority
            && (previous_agent_label.is_some() || self.agent_name.is_some());
        if self.should_ignore_detected_state_under_full_lifecycle_hook(agent, process_exited) {
            if self
                .hook_authority
                .as_ref()
                .and_then(|authority| crate::detect::parse_agent_label(&authority.agent_label))
                == agent
            {
                self.detected_agent = agent;
            }
            return TerminalStateMutation {
                effective_state_change: self.recompute_effective_state(
                    previous_agent_label,
                    previous_known_agent,
                    previous_state,
                    previous_presentation,
                    now,
                ),
                session_ref_changed: previous_session
                    != self.current_session_identity_for_persistence(),
                agent_released: false,
            };
        }
        let replacement_process_detected = !process_exited
            && agent.is_some()
            && self
                .recent_agent_process_exit
                .is_some_and(|exit| Some(exit.agent) == agent && exit.observed_at < now);
        if !process_exited && self.detected_state_observed_before_release_suppression(agent, now) {
            return TerminalStateMutation {
                effective_state_change: self.recompute_effective_state(
                    previous_agent_label,
                    previous_known_agent,
                    previous_state,
                    previous_presentation,
                    now,
                ),
                session_ref_changed: previous_session
                    != self.current_session_identity_for_persistence(),
                agent_released: false,
            };
        }
        self.detected_agent = agent;
        if agent.is_some()
            && self
                .agent_launch
                .as_ref()
                .is_some_and(|launch| Some(launch.agent) != agent)
        {
            self.set_agent_launch(None);
        }
        if process_exited || agent != Some(Agent::Codex) || fallback_state == AgentState::Blocked {
            self.codex_prompt_ready = false;
        }
        if let Some(agent) = agent {
            let agent_label = crate::detect::agent_label(agent);
            self.reconcile_agent_name_owner(agent_label, None);
        }
        if !process_exited {
            self.clear_full_lifecycle_hook_suppression_for_detected_agent(
                if replacement_process_detected {
                    None
                } else {
                    previous_detected_agent
                },
                agent,
            );
        }
        self.fallback_state = fallback_state;
        self.fallback_visible_blocker = visible_blocker && fallback_state == AgentState::Blocked;
        self.fallback_observed_at = Some(now);
        if process_exited {
            if let Some(agent) = agent {
                self.recent_agent_process_exit = Some(RecentAgentProcessExit {
                    agent,
                    observed_at: now,
                });
            }
        } else if agent.is_some() {
            self.recent_agent_process_exit = None;
        }
        if process_exited {
            let mut reset_sources = Vec::new();
            let mut stale_sessions = Vec::new();
            for (source, suppressed) in &mut self.suppressed_full_lifecycle_hook_reports {
                if crate::detect::parse_agent_label(&suppressed.agent_label) != agent
                    || suppressed.reason == FullLifecycleHookSuppressionReason::HookClear
                {
                    continue;
                }
                let exited_session_ref = suppressed
                    .replacement_session_ref
                    .take()
                    .or_else(|| {
                        suppressed
                            .pending_replacement_report
                            .as_ref()
                            .and_then(|pending| pending.authority.session_ref.clone())
                    })
                    .or_else(|| suppressed.session_ref.clone());
                if let (Some(previous), Some(exited)) =
                    (suppressed.session_ref.as_ref(), exited_session_ref.as_ref())
                {
                    if previous != exited {
                        stale_sessions.push((
                            source.clone(),
                            suppressed.agent_label.clone(),
                            previous.clone(),
                        ));
                    }
                }
                suppressed.session_ref = exited_session_ref;
                suppressed.pending_replacement_report = None;
                suppressed.observed_at = now;
                reset_sources.push(source.clone());
            }
            for (source, agent_label, session_ref) in stale_sessions {
                self.remember_stale_full_lifecycle_hook_session(source, agent_label, session_ref);
            }
            for source in reset_sources {
                self.hook_report_sequences.remove(&source);
            }

            let official_session = self
                .hook_authority
                .as_ref()
                .filter(|authority| {
                    crate::agent_resume::is_official_agent_source(
                        &authority.source,
                        &authority.agent_label,
                    ) && crate::detect::parse_agent_label(&authority.agent_label) == agent
                })
                .map(|authority| {
                    (
                        authority.source.clone(),
                        authority.agent_label.clone(),
                        authority.session_ref.clone(),
                    )
                })
                .or_else(|| {
                    self.persisted_agent_session.as_ref().and_then(|session| {
                        (crate::agent_resume::is_official_agent_source(
                            &session.source,
                            &session.agent,
                        ) && crate::detect::parse_agent_label(&session.agent) == agent)
                            .then(|| {
                                (
                                    session.source.clone(),
                                    session.agent.clone(),
                                    Some(session.session_ref.clone()),
                                )
                            })
                    })
                });
            if let Some((source, agent_label, session_ref)) = official_session {
                self.hook_report_sequences.remove(&source);
                self.suppress_full_lifecycle_hook_report_with_session_ref(
                    source,
                    agent_label,
                    session_ref,
                    FullLifecycleHookSuppressionReason::ProcessExit,
                    now,
                );
            }
            let cleared_hook_source = self.hook_authority.as_ref().and_then(|authority| {
                (crate::detect::parse_agent_label(&authority.agent_label) == agent
                    && !newer_custom_authority)
                    .then(|| authority.source.clone())
            });
            if let Some(source) = cleared_hook_source {
                self.hook_report_sequences.remove(&source);
                self.hook_authority = None;
            }
            if !newer_custom_authority
                && self
                    .persisted_agent_session
                    .as_ref()
                    .is_some_and(|session| {
                        crate::detect::parse_agent_label(&session.agent) == agent
                    })
            {
                self.persisted_agent_session = None;
            }
            if !newer_custom_authority
                && agent.is_some()
                && self
                    .agent_launch
                    .as_ref()
                    .is_some_and(|launch| Some(launch.agent) == agent)
            {
                if let Some(process) = self
                    .agent_launch
                    .as_ref()
                    .and_then(ObservedAgentLaunch::process)
                {
                    self.process_sessions.forget(&process);
                }
                self.set_agent_launch(None);
            }
            if !newer_custom_authority
                && agent.is_some()
                && self
                    .reported_resume
                    .as_ref()
                    .is_some_and(|resume| crate::detect::parse_agent_label(&resume.agent) == agent)
            {
                self.set_reported_resume(None);
            }
            if let Some(agent) = agent {
                let agent_label = crate::detect::agent_label(agent);
                let mut cleared_metadata_sources = Vec::new();
                self.agent_metadata.retain(|source, metadata| {
                    let official_metadata = crate::agent_resume::is_official_agent_source(
                        &metadata.source,
                        agent_label,
                    ) || metadata.applies_to_source.as_deref().is_some_and(
                        |applies_to| {
                            crate::agent_resume::is_official_agent_source(applies_to, agent_label)
                        },
                    );
                    let matches_agent =
                        metadata.agent_label.as_deref() == Some(agent_label) || official_metadata;
                    let clear = matches_agent && (official_metadata || metadata.reported_at <= now);
                    if clear {
                        cleared_metadata_sources.push(source.clone());
                    }
                    !clear
                });
                for source in cleared_metadata_sources {
                    self.metadata_report_sequences.remove(&source);
                    self.metadata_report_agents.remove(&source);
                    self.metadata_token_sequence_sources.remove(&source);
                }
                let mut exited_generation_sources = Vec::new();
                self.metadata_report_agents.retain(|source, owner| {
                    if *owner == agent {
                        exited_generation_sources.push(source.clone());
                        false
                    } else {
                        true
                    }
                });
                for source in exited_generation_sources {
                    self.metadata_report_sequences.remove(&source);
                    self.metadata_token_sequence_sources.remove(&source);
                }
            }
        }
        if self.hook_authority_not_newer_than(now)
            && (self.hook_authority_conflicts_with_detected_agent(agent)
                || (previous_detected_agent.is_some()
                    && agent != previous_detected_agent
                    && self.hook_authority.as_ref().is_some_and(|authority| {
                        crate::detect::parse_agent_label(&authority.agent_label)
                            == previous_detected_agent
                    })))
        {
            let durable_session = self.hook_authority.as_ref().and_then(|authority| {
                authority.session_ref.as_ref().map(|session_ref| {
                    crate::agent_resume::PersistedAgentSession {
                        source: authority.source.clone(),
                        agent: authority.agent_label.clone(),
                        session_ref: session_ref.clone(),
                    }
                })
            });
            self.suppress_current_full_lifecycle_hook_authority(
                FullLifecycleHookSuppressionReason::HookClear,
            );
            self.hook_authority = None;
            self.persisted_agent_session = durable_session;
            self.persisted_agent_session_reported = true;
        }
        // Observing a process exit is not the same as the agent being gone: the
        // observation can be wrong while the agent keeps running, and the name
        // is the only handle its owner has on the pane. Detection uncertainty
        // already keeps the name, so free it at the point the agent actually
        // leaves the pane - a recorded exit with no agent detected any more.
        if agent.is_none() && self.recent_agent_process_exit.is_some() {
            self.clear_agent_name();
        }
        let effective_state_change = self.recompute_effective_state(
            previous_agent_label,
            previous_known_agent,
            previous_state,
            previous_presentation,
            now,
        );
        if fallback_state == AgentState::Working && self.state == AgentState::Working {
            self.agent_process_acquisition_pending = false;
        }
        TerminalStateMutation {
            effective_state_change,
            session_ref_changed: previous_session
                != self.current_session_identity_for_persistence(),
            agent_released,
        }
    }

    #[cfg(test)]
    pub fn set_hook_authority(
        &mut self,
        source: String,
        agent_label: String,
        state: AgentState,
        message: Option<String>,
        seq: Option<u64>,
    ) -> Option<EffectiveStateChange> {
        self.set_hook_authority_at(
            source,
            agent_label,
            state,
            message,
            None,
            seq,
            Instant::now(),
        )
        .and_then(|mutation| mutation.effective_state_change)
    }

    pub fn set_hook_authority_with_session_ref(
        &mut self,
        source: String,
        agent_label: String,
        state: AgentState,
        message: Option<String>,
        session_ref: Option<crate::agent_resume::AgentSessionRef>,
        seq: Option<u64>,
    ) -> Option<TerminalStateMutation> {
        self.set_hook_authority_at(
            source,
            agent_label,
            state,
            message,
            session_ref,
            seq,
            Instant::now(),
        )
    }

    pub fn set_hook_authority_at(
        &mut self,
        source: String,
        agent_label: String,
        state: AgentState,
        message: Option<String>,
        session_ref: Option<crate::agent_resume::AgentSessionRef>,
        seq: Option<u64>,
        now: Instant,
    ) -> Option<TerminalStateMutation> {
        if crate::detect::session_identity_only_integration(&source, &agent_label) {
            return None;
        }
        if !crate::detect::full_lifecycle_hook_authority(&source, &agent_label)
            && self.recent_agent_process_exit.is_some_and(|exit| {
                crate::detect::parse_agent_label(&agent_label) == Some(exit.agent)
            })
        {
            return None;
        }
        let reanchor_sequence = match self.route_full_lifecycle_hook_report(
            &source,
            &agent_label,
            state,
            message.as_deref(),
            &session_ref,
            seq,
            now,
        ) {
            FullLifecycleHookReportRoute::Accept { reanchor_sequence } => reanchor_sequence,
            FullLifecycleHookReportRoute::Ignore => return None,
        };
        if self.known_agent_label_conflicts_with_detected_agent(&agent_label) {
            return None;
        }
        let owner_conflicts = self.current_session_owner_conflicts(&source, &agent_label);
        let foreground_takeover_allowed = owner_conflicts
            && self.foreground_agent_confirms_hook_authority_takeover(
                &source,
                &agent_label,
                &session_ref,
            );
        if owner_conflicts && !foreground_takeover_allowed {
            return None;
        }
        let session_ref = session_ref.map(|session_ref| {
            if self.lifecycle_hook_report_replaces_persisted_session(
                &source,
                &agent_label,
                &session_ref,
            ) {
                session_ref
            } else {
                self.conflicting_same_owner_session_ref(&source, &agent_label, &session_ref, None)
                    .unwrap_or(session_ref)
            }
        });
        if self.live_full_lifecycle_hook_authority_conflicts_with_session(
            &source,
            &agent_label,
            &session_ref,
        ) {
            return None;
        }
        if reanchor_sequence {
            self.hook_report_sequences.remove(&source);
        }
        if !self.accept_hook_report(&source, seq) {
            return None;
        }

        let previous_agent_label = self.effective_agent_label().map(str::to_string);
        let previous_known_agent = self.effective_known_agent();
        let previous_state = self.state;
        let previous_presentation = self.effective_presentation_for_state_at(previous_state, now);
        let previous_session = self.current_session_identity_for_persistence();
        self.reconcile_agent_name_owner(&agent_label, session_ref.as_ref());
        if foreground_takeover_allowed {
            self.suppress_current_full_lifecycle_hook_authority(
                FullLifecycleHookSuppressionReason::HookClear,
            );
        }
        if session_ref.is_some() || reanchor_sequence {
            if let Some(suppressed) = self.suppressed_full_lifecycle_hook_reports.remove(&source) {
                if let Some(suppressed_ref) = suppressed.session_ref {
                    self.remember_stale_full_lifecycle_hook_session(
                        source.clone(),
                        suppressed.agent_label,
                        suppressed_ref,
                    );
                }
            }
        }
        self.persisted_agent_session = None;
        self.hook_authority = Some(HookAuthority {
            source,
            agent_label,
            state,
            message,
            reported_at: now,
            session_ref,
        });
        let current_session = self.current_session_identity_for_persistence();
        let effective_state_change = self.recompute_effective_state(
            previous_agent_label,
            previous_known_agent,
            previous_state,
            previous_presentation,
            now,
        );
        if state == AgentState::Working && self.state == AgentState::Working {
            self.agent_process_acquisition_pending = false;
        }
        Some(TerminalStateMutation {
            effective_state_change,
            session_ref_changed: previous_session != current_session,
            agent_released: false,
        })
    }

    fn hook_authority_not_newer_than(&self, observed_at: Instant) -> bool {
        self.hook_authority
            .as_ref()
            .is_none_or(|authority| authority.reported_at <= observed_at)
    }

    fn fallback_not_older_than_hook(&self) -> bool {
        self.hook_authority.as_ref().is_none_or(|authority| {
            self.fallback_observed_at
                .is_some_and(|observed_at| authority.reported_at <= observed_at)
        })
    }

    fn hook_authority_conflicts_with_detected_agent(&self, detected_agent: Option<Agent>) -> bool {
        let Some(detected_agent) = detected_agent else {
            return false;
        };
        self.hook_authority.as_ref().is_some_and(|authority| {
            crate::detect::parse_agent_label(&authority.agent_label)
                .is_some_and(|hook_agent| hook_agent != detected_agent)
        })
    }

    fn should_ignore_detected_state_under_full_lifecycle_hook(
        &self,
        detected_agent: Option<Agent>,
        process_exited: bool,
    ) -> bool {
        self.live_full_lifecycle_hook_authority()
            && !process_exited
            && !self.hook_authority_conflicts_with_detected_agent(detected_agent)
    }

    fn persisted_agent_session_matches(&self, source: &str, agent: &str) -> bool {
        self.persisted_agent_session
            .as_ref()
            .is_some_and(|session| session.source == source && session.agent == agent)
    }

    fn suppress_current_full_lifecycle_hook_authority(
        &mut self,
        reason: FullLifecycleHookSuppressionReason,
    ) {
        if let Some((source, agent_label, session_ref)) =
            self.hook_authority.as_ref().and_then(|authority| {
                crate::detect::full_lifecycle_hook_authority(
                    &authority.source,
                    &authority.agent_label,
                )
                .then(|| {
                    (
                        authority.source.clone(),
                        authority.agent_label.clone(),
                        authority.session_ref.clone(),
                    )
                })
            })
        {
            self.suppress_full_lifecycle_hook_report_with_session_ref(
                source,
                agent_label,
                session_ref,
                reason,
                Instant::now(),
            );
        }
    }

    fn suppress_full_lifecycle_hook_report(
        &mut self,
        source: &str,
        agent_label: &str,
        reason: FullLifecycleHookSuppressionReason,
    ) {
        if crate::detect::full_lifecycle_hook_authority(source, agent_label) {
            let session_ref = self
                .hook_authority
                .as_ref()
                .and_then(|authority| authority.session_ref.clone());
            self.suppress_full_lifecycle_hook_report_with_session_ref(
                source.to_string(),
                agent_label.to_string(),
                session_ref,
                reason,
                Instant::now(),
            );
        }
    }

    fn suppress_full_lifecycle_hook_report_with_session_ref(
        &mut self,
        source: String,
        agent_label: String,
        session_ref: Option<crate::agent_resume::AgentSessionRef>,
        reason: FullLifecycleHookSuppressionReason,
        observed_at: Instant,
    ) {
        self.suppressed_full_lifecycle_hook_reports.insert(
            source,
            SuppressedFullLifecycleHookReport {
                agent_label,
                session_ref,
                observed_at,
                reason,
                replacement_session_ref: None,
                pending_replacement_report: None,
            },
        );
    }

    fn route_full_lifecycle_hook_report(
        &mut self,
        source: &str,
        agent_label: &str,
        state: AgentState,
        message: Option<&str>,
        session_ref: &Option<crate::agent_resume::AgentSessionRef>,
        seq: Option<u64>,
        reported_at: Instant,
    ) -> FullLifecycleHookReportRoute {
        if !crate::detect::full_lifecycle_hook_authority(source, agent_label) {
            return FullLifecycleHookReportRoute::Accept {
                reanchor_sequence: false,
            };
        }
        if self.full_lifecycle_hook_report_matches_stale_session(source, agent_label, session_ref) {
            return FullLifecycleHookReportRoute::Ignore;
        }

        let known_agent = crate::detect::parse_agent_label(agent_label);
        let process_present = known_agent.is_some()
            && self.detected_agent == known_agent
            && self.recent_agent_process_exit.is_none();
        let anchored_session_ref = self
            .hook_authority
            .as_ref()
            .filter(|authority| authority.source == source && authority.agent_label == agent_label)
            .and_then(|authority| authority.session_ref.as_ref())
            .or_else(|| {
                self.persisted_agent_session
                    .as_ref()
                    .filter(|session| session.source == source && session.agent == agent_label)
                    .map(|session| &session.session_ref)
            });
        let session_anchored = anchored_session_ref.is_some_and(|anchored| {
            session_ref
                .as_ref()
                .is_none_or(|incoming| incoming == anchored)
        });
        let opencode_cross_talk = (source, agent_label) == ("herdr:opencode", "opencode")
            && process_present
            && anchored_session_ref
                .zip(session_ref.as_ref())
                .is_some_and(|(anchored, incoming)| anchored != incoming);
        if opencode_cross_talk {
            return FullLifecycleHookReportRoute::Ignore;
        }
        if let Some(suppressed) = self.suppressed_full_lifecycle_hook_reports.get(source) {
            if suppressed.agent_label != agent_label {
                return FullLifecycleHookReportRoute::Ignore;
            }
            if suppressed.reason == FullLifecycleHookSuppressionReason::HookClear {
                let reanchor_sequence = matches!(
                    (&suppressed.session_ref, session_ref),
                    (Some(previous), Some(incoming)) if previous != incoming
                );
                return if reanchor_sequence {
                    FullLifecycleHookReportRoute::Accept {
                        reanchor_sequence: true,
                    }
                } else {
                    FullLifecycleHookReportRoute::Ignore
                };
            }
        }

        if process_present
            && session_anchored
            && !self
                .suppressed_full_lifecycle_hook_reports
                .contains_key(source)
        {
            return FullLifecycleHookReportRoute::Accept {
                reanchor_sequence: self
                    .full_lifecycle_hook_report_has_fresh_session_after_stale_session(
                        source,
                        agent_label,
                        session_ref,
                    ),
            };
        }

        let Some(session_ref) = session_ref.clone() else {
            return FullLifecycleHookReportRoute::Ignore;
        };
        let Some(seq) = seq else {
            return FullLifecycleHookReportRoute::Ignore;
        };
        if self
            .hook_report_sequences
            .get(source)
            .is_some_and(|previous| seq <= *previous)
        {
            return FullLifecycleHookReportRoute::Ignore;
        }

        let previous_session_ref = self
            .persisted_agent_session
            .as_ref()
            .filter(|session| session.source == source && session.agent == agent_label)
            .map(|session| session.session_ref.clone());
        let suppressed = self
            .suppressed_full_lifecycle_hook_reports
            .entry(source.to_string())
            .or_insert_with(|| SuppressedFullLifecycleHookReport {
                agent_label: agent_label.to_string(),
                session_ref: previous_session_ref,
                observed_at: reported_at,
                reason: FullLifecycleHookSuppressionReason::ProcessExit,
                replacement_session_ref: None,
                pending_replacement_report: None,
            });
        let replace_pending = suppressed
            .pending_replacement_report
            .as_ref()
            .is_none_or(|pending| seq > pending.seq);
        if replace_pending {
            suppressed.pending_replacement_report = Some(PendingFullLifecycleHookReport {
                authority: HookAuthority {
                    source: source.to_string(),
                    agent_label: agent_label.to_string(),
                    state,
                    message: message.map(str::to_string),
                    reported_at,
                    session_ref: Some(session_ref),
                },
                seq,
            });
        }
        FullLifecycleHookReportRoute::Ignore
    }

    fn full_lifecycle_hook_report_matches_stale_session(
        &self,
        source: &str,
        agent_label: &str,
        session_ref: &Option<crate::agent_resume::AgentSessionRef>,
    ) -> bool {
        if !crate::detect::full_lifecycle_hook_authority(source, agent_label) {
            return false;
        }
        self.stale_full_lifecycle_hook_sessions
            .get(source)
            .is_some_and(|stale_sessions| {
                session_ref.as_ref().is_some_and(|incoming_ref| {
                    stale_sessions.iter().any(|stale| {
                        stale.agent_label == agent_label && incoming_ref == &stale.session_ref
                    })
                })
            })
    }

    fn full_lifecycle_hook_report_has_fresh_session_after_stale_session(
        &self,
        source: &str,
        agent_label: &str,
        session_ref: &Option<crate::agent_resume::AgentSessionRef>,
    ) -> bool {
        if !crate::detect::full_lifecycle_hook_authority(source, agent_label) {
            return false;
        }
        self.stale_full_lifecycle_hook_sessions
            .get(source)
            .is_some_and(|stale_sessions| {
                stale_sessions
                    .iter()
                    .any(|stale| stale.agent_label == agent_label)
                    && session_ref.as_ref().is_some_and(|incoming_ref| {
                        stale_sessions.iter().all(|stale| {
                            stale.agent_label != agent_label || incoming_ref != &stale.session_ref
                        })
                    })
            })
    }

    fn live_full_lifecycle_hook_authority_conflicts_with_session(
        &self,
        source: &str,
        agent_label: &str,
        session_ref: &Option<crate::agent_resume::AgentSessionRef>,
    ) -> bool {
        let Some(authority) = self.hook_authority.as_ref() else {
            return false;
        };
        if !crate::detect::full_lifecycle_hook_authority(&authority.source, &authority.agent_label)
        {
            return false;
        }
        if authority.source != source || authority.agent_label != agent_label {
            return false;
        }
        authority
            .session_ref
            .as_ref()
            .zip(session_ref.as_ref())
            .is_some_and(|(current, incoming)| current != incoming)
    }

    fn same_owner_full_lifecycle_hook_authority_session_ref(
        &self,
        source: &str,
        agent_label: &str,
        session_ref: &crate::agent_resume::AgentSessionRef,
    ) -> Option<crate::agent_resume::AgentSessionRef> {
        let authority = self.hook_authority.as_ref()?;
        if !crate::detect::full_lifecycle_hook_authority(&authority.source, &authority.agent_label)
            || authority.source != source
            || authority.agent_label != agent_label
        {
            return None;
        }
        authority
            .session_ref
            .as_ref()
            .filter(|current| *current != session_ref)
            .cloned()
    }

    fn clear_full_lifecycle_hook_suppression_for_detected_agent(
        &mut self,
        previous_detected_agent: Option<Agent>,
        detected_agent: Option<Agent>,
    ) {
        let Some(detected_agent) = detected_agent else {
            return;
        };
        if previous_detected_agent == Some(detected_agent) {
            return;
        }
        let detected_label = crate::detect::agent_label(detected_agent);
        let mut stale_sessions = Vec::new();
        let mut validated_replacement_sessions = Vec::new();
        self.suppressed_full_lifecycle_hook_reports
            .retain(|source, suppressed| {
                let should_clear = crate::detect::parse_agent_label(&suppressed.agent_label)
                    == Some(detected_agent);
                if !should_clear {
                    return true;
                }
                if suppressed.reason == FullLifecycleHookSuppressionReason::ProcessExit {
                    if let Some(session_ref) = suppressed.replacement_session_ref.take() {
                        if let Some(exited_session_ref) = suppressed
                            .session_ref
                            .as_ref()
                            .filter(|exited_session_ref| *exited_session_ref != &session_ref)
                            .cloned()
                        {
                            stale_sessions.push((
                                source.clone(),
                                StaleFullLifecycleHookSession {
                                    agent_label: suppressed.agent_label.clone(),
                                    session_ref: exited_session_ref,
                                },
                            ));
                        }
                        let session_start_seq = self.hook_report_sequences.get(source).copied();
                        let pending =
                            suppressed
                                .pending_replacement_report
                                .take()
                                .filter(|pending| {
                                    pending.authority.session_ref.as_ref() == Some(&session_ref)
                                        && session_start_seq.is_none_or(|seq| pending.seq > seq)
                                });
                        validated_replacement_sessions.push((
                            source.clone(),
                            suppressed.agent_label.clone(),
                            session_ref,
                            pending,
                        ));
                        return false;
                    }
                    return true;
                }
                if let Some(session_ref) = suppressed.session_ref.clone() {
                    stale_sessions.push((
                        source.clone(),
                        StaleFullLifecycleHookSession {
                            agent_label: suppressed.agent_label.clone(),
                            session_ref,
                        },
                    ));
                }
                false
            });
        for (source, stale_session) in stale_sessions {
            self.remember_stale_full_lifecycle_hook_session(
                source,
                stale_session.agent_label,
                stale_session.session_ref,
            );
        }
        self.hook_report_sequences.retain(|source, _| {
            validated_replacement_sessions
                .iter()
                .any(|(validated_source, _, _, _)| validated_source == source)
                || !crate::detect::full_lifecycle_hook_authority(source, detected_label)
        });
        for (source, agent_label, session_ref, pending) in validated_replacement_sessions {
            self.forget_stale_full_lifecycle_hook_session(&source, &agent_label, &session_ref);
            self.reconcile_agent_name_owner(&agent_label, Some(&session_ref));
            self.persisted_agent_session_reported = true;
            self.persisted_agent_session = Some(crate::agent_resume::PersistedAgentSession {
                source: source.clone(),
                agent: agent_label,
                session_ref,
            });
            if let Some(pending) = pending {
                self.hook_report_sequences.insert(source, pending.seq);
                self.hook_authority = Some(pending.authority);
            }
        }
    }

    fn remember_stale_full_lifecycle_hook_session(
        &mut self,
        source: String,
        agent_label: String,
        session_ref: crate::agent_resume::AgentSessionRef,
    ) {
        let stale_session = StaleFullLifecycleHookSession {
            agent_label,
            session_ref,
        };
        let source_stale_sessions = self
            .stale_full_lifecycle_hook_sessions
            .entry(source)
            .or_default();
        if !source_stale_sessions
            .iter()
            .any(|existing| existing == &stale_session)
        {
            source_stale_sessions.push(stale_session);
        }
    }

    fn forget_stale_full_lifecycle_hook_session(
        &mut self,
        source: &str,
        agent_label: &str,
        session_ref: &crate::agent_resume::AgentSessionRef,
    ) {
        let remove_source = self
            .stale_full_lifecycle_hook_sessions
            .get_mut(source)
            .is_some_and(|stale_sessions| {
                stale_sessions.retain(|stale| {
                    stale.agent_label != agent_label || &stale.session_ref != session_ref
                });
                stale_sessions.is_empty()
            });
        if remove_source {
            self.stale_full_lifecycle_hook_sessions.remove(source);
        }
    }

    fn detected_state_observed_before_release_suppression(
        &self,
        detected_agent: Option<Agent>,
        observed_at: Instant,
    ) -> bool {
        let Some(detected_agent) = detected_agent else {
            return false;
        };
        self.suppressed_full_lifecycle_hook_reports
            .values()
            .any(|suppressed| {
                crate::detect::parse_agent_label(&suppressed.agent_label) == Some(detected_agent)
                    && observed_at <= suppressed.observed_at
            })
    }

    fn current_session_identity_for_persistence(
        &self,
    ) -> Option<(
        String,
        String,
        crate::agent_resume::AgentSessionRefKind,
        String,
    )> {
        if let Some(authority) = self.hook_authority.as_ref() {
            if let Some(session_ref) = authority.session_ref.as_ref() {
                return Some((
                    authority.source.clone(),
                    authority.agent_label.clone(),
                    session_ref.kind,
                    session_ref.value.clone(),
                ));
            }
        }
        self.persisted_agent_session.as_ref().map(|session| {
            (
                session.source.clone(),
                session.agent.clone(),
                session.session_ref.kind,
                session.session_ref.value.clone(),
            )
        })
    }

    fn current_session_owner_conflicts(&self, source: &str, agent_label: &str) -> bool {
        self.current_session_identity_for_persistence().is_some_and(
            |(current_source, current_agent, _, _)| {
                current_source != source || current_agent != agent_label
            },
        )
    }

    fn conflicting_same_owner_session_ref(
        &self,
        source: &str,
        agent_label: &str,
        session_ref: &crate::agent_resume::AgentSessionRef,
        session_start_source: Option<&str>,
    ) -> Option<crate::agent_resume::AgentSessionRef> {
        self.current_session_identity_for_persistence().and_then(
            |(current_source, current_agent, current_kind, current_value)| {
                (current_source == source
                    && current_agent == agent_label
                    && current_kind == crate::agent_resume::AgentSessionRefKind::Id
                    && session_ref.kind == crate::agent_resume::AgentSessionRefKind::Id
                    && current_value != session_ref.value
                    && !Self::session_report_allows_session_replacement(
                        source,
                        agent_label,
                        session_start_source,
                    ))
                .then_some(crate::agent_resume::AgentSessionRef {
                    kind: current_kind,
                    value: current_value,
                })
            },
        )
    }

    fn lifecycle_hook_report_replaces_persisted_session(
        &self,
        source: &str,
        agent_label: &str,
        session_ref: &crate::agent_resume::AgentSessionRef,
    ) -> bool {
        self.hook_authority.is_none()
            && (source, agent_label) == ("herdr:mastracode", "mastracode")
            && self
                .persisted_agent_session
                .as_ref()
                .is_some_and(|session| {
                    session.source == source
                        && session.agent == agent_label
                        && session.session_ref.kind == crate::agent_resume::AgentSessionRefKind::Id
                        && session_ref.kind == crate::agent_resume::AgentSessionRefKind::Id
                        && session.session_ref.value != session_ref.value
                })
    }

    fn session_report_allows_session_replacement(
        source: &str,
        agent_label: &str,
        session_start_source: Option<&str>,
    ) -> bool {
        matches!(
            (source, agent_label, session_start_source),
            (
                "herdr:claude",
                "claude",
                Some("clear" | "resume" | "compact")
            ) | (
                "herdr:codex",
                "codex",
                Some("startup" | "clear" | "resume" | "compact")
            ) | ("herdr:mastracode", "mastracode", Some("startup"))
                | ("herdr:hermes", "hermes", Some("startup" | "new" | "resume"))
                | ("herdr:opencode", "opencode", Some("select"))
                | ("herdr:pi", "pi", Some("new" | "resume" | "fork"))
                | ("herdr:grok", "grok", Some("new"))
                | (
                    "herdr:omp",
                    "omp",
                    Some("startup" | "new" | "resume" | "fork")
                )
                | (
                    "herdr:qwen",
                    "qwen",
                    Some("startup" | "clear" | "resume" | "compact" | "branch")
                )
                | ("herdr:antigravity_cli", "agy", None)
        )
    }

    fn session_start_source_is_recognized(session_start_source: Option<&str>) -> bool {
        matches!(
            session_start_source,
            Some("startup" | "clear" | "resume" | "compact" | "new" | "fork" | "select")
        )
    }

    fn is_unsequenced_opencode_selection(
        source: &str,
        agent_label: &str,
        session_start_source: Option<&str>,
        seq: Option<u64>,
    ) -> bool {
        (source, agent_label, session_start_source, seq)
            == ("herdr:opencode", "opencode", Some("select"), None)
    }

    /// Sets the conversation a restore left in this pane.
    pub fn set_persisted_agent_session(
        &mut self,
        session: crate::agent_resume::PersistedAgentSession,
    ) {
        self.persisted_agent_session = Some(session);
        self.persisted_agent_session_reported = false;
    }

    pub fn set_managed_agent_launch_session(
        &mut self,
        session: crate::agent_resume::PersistedAgentSession,
    ) {
        self.persisted_agent_session = Some(session.clone());
        self.persisted_agent_session_reported = false;
        self.managed_agent_launch_session = Some(session);
    }

    pub fn set_agent_session_ref(
        &mut self,
        source: String,
        agent_label: String,
        session_ref: Option<crate::agent_resume::AgentSessionRef>,
        seq: Option<u64>,
    ) -> Option<TerminalStateMutation> {
        self.set_agent_session_ref_for_session_start(source, agent_label, session_ref, seq, None)
    }

    /// Applies a conversation report as the server receives it, with what it
    /// knew about the report's sender. Only a report the pane takes, as
    /// current and not stale or duplicate by its sequence, pairs its
    /// conversation with the launch record; one it rejects says nothing about
    /// which process runs it.
    pub fn apply_agent_session_report(
        &mut self,
        source: String,
        agent_label: String,
        session_ref: Option<crate::agent_resume::AgentSessionRef>,
        seq: Option<u64>,
        session_start_source: Option<String>,
        evidence: &SessionReportEvidence,
    ) -> Option<TerminalStateMutation> {
        let reported = session_ref.clone();
        let mutation = self.set_agent_session_ref_for_session_start(
            source,
            agent_label,
            session_ref,
            seq,
            session_start_source,
        )?;
        if reported.is_some_and(|session| self.session_ref_is_current(&session)) {
            self.sync_agent_launch_session(evidence);
        }
        Some(mutation)
    }

    pub fn set_agent_session_ref_for_session_start(
        &mut self,
        source: String,
        agent_label: String,
        session_ref: Option<crate::agent_resume::AgentSessionRef>,
        seq: Option<u64>,
        session_start_source: Option<String>,
    ) -> Option<TerminalStateMutation> {
        let session_ref = session_ref?;
        let known_agent = crate::detect::parse_agent_label(&agent_label);
        let process_present = known_agent.is_some()
            && self.detected_agent == known_agent
            && self.recent_agent_process_exit.is_none();
        let full_lifecycle_source =
            crate::detect::full_lifecycle_hook_authority(&source, &agent_label);
        let generation_gated = self
            .suppressed_full_lifecycle_hook_reports
            .get(&source)
            .is_some_and(|suppressed| {
                suppressed.agent_label == agent_label
                    && suppressed.reason != FullLifecycleHookSuppressionReason::HookClear
            });
        let session_anchored = self.hook_authority.as_ref().is_some_and(|authority| {
            authority.source == source
                && authority.agent_label == agent_label
                && authority.session_ref.is_some()
        }) || self.persisted_agent_session_matches(&source, &agent_label);
        let unsequenced_selection = Self::is_unsequenced_opencode_selection(
            &source,
            &agent_label,
            session_start_source.as_deref(),
            seq,
        );
        let selection_can_reconcile = unsequenced_selection && process_present;
        if selection_can_reconcile {
            self.suppressed_full_lifecycle_hook_reports.remove(&source);
        } else if full_lifecycle_source && unsequenced_selection {
            let previous_session_ref = self
                .hook_authority
                .as_ref()
                .filter(|authority| {
                    authority.source == source && authority.agent_label == agent_label
                })
                .and_then(|authority| authority.session_ref.clone())
                .or_else(|| {
                    self.persisted_agent_session
                        .as_ref()
                        .filter(|session| session.source == source && session.agent == agent_label)
                        .map(|session| session.session_ref.clone())
                });
            let suppressed = self
                .suppressed_full_lifecycle_hook_reports
                .entry(source)
                .or_insert_with(|| SuppressedFullLifecycleHookReport {
                    agent_label,
                    session_ref: previous_session_ref,
                    observed_at: Instant::now(),
                    reason: FullLifecycleHookSuppressionReason::ProcessExit,
                    replacement_session_ref: None,
                    pending_replacement_report: None,
                });
            suppressed.replacement_session_ref = Some(session_ref);
            suppressed.pending_replacement_report = None;
            return None;
        }
        if full_lifecycle_source
            && !selection_can_reconcile
            && (!process_present || generation_gated || !session_anchored)
        {
            if !Self::session_start_source_is_recognized(session_start_source.as_deref()) {
                return None;
            }
            let seq = seq?;
            if self
                .hook_report_sequences
                .get(&source)
                .is_some_and(|previous| seq <= *previous)
            {
                return None;
            }

            let previous_agent_label = self.effective_agent_label().map(str::to_string);
            let previous_known_agent = self.effective_known_agent();
            let previous_state = self.state;
            let now = Instant::now();
            let previous_presentation =
                self.effective_presentation_for_state_at(previous_state, now);
            let previous_session = self.current_session_identity_for_persistence();
            let suppressed = self
                .suppressed_full_lifecycle_hook_reports
                .entry(source.clone())
                .or_insert_with(|| SuppressedFullLifecycleHookReport {
                    agent_label: agent_label.clone(),
                    session_ref: None,
                    observed_at: now,
                    reason: FullLifecycleHookSuppressionReason::ProcessExit,
                    replacement_session_ref: None,
                    pending_replacement_report: None,
                });
            if suppressed.replacement_session_ref.as_ref() != Some(&session_ref) {
                if suppressed
                    .pending_replacement_report
                    .as_ref()
                    .is_some_and(|pending| {
                        pending.authority.session_ref.as_ref() != Some(&session_ref)
                    })
                {
                    suppressed.pending_replacement_report = None;
                }
                suppressed.replacement_session_ref = Some(session_ref);
            }
            self.hook_report_sequences.insert(source.clone(), seq);

            if process_present {
                self.clear_full_lifecycle_hook_suppression_for_detected_agent(None, known_agent);
                let current_session = self.current_session_identity_for_persistence();
                return Some(TerminalStateMutation {
                    effective_state_change: self.recompute_effective_state(
                        previous_agent_label,
                        previous_known_agent,
                        previous_state,
                        previous_presentation,
                        now,
                    ),
                    session_ref_changed: previous_session != current_session,
                    agent_released: false,
                });
            }
            return None;
        }
        if !unsequenced_selection && !self.accept_hook_report(&source, seq) {
            return None;
        }
        if self.known_agent_label_conflicts_with_detected_agent(&agent_label) {
            return None;
        }
        let session_replacement_allowed = Self::session_report_allows_session_replacement(
            &source,
            &agent_label,
            session_start_source.as_deref(),
        );
        let replacing_identity_only_session =
            crate::detect::session_identity_only_integration(&source, &agent_label)
                && session_replacement_allowed
                && self.current_session_identity_for_persistence().is_some_and(
                    |(current_source, current_agent, current_kind, current_value)| {
                        current_source == source
                            && current_agent == agent_label
                            && current_kind == crate::agent_resume::AgentSessionRefKind::Id
                            && session_ref.kind == crate::agent_resume::AgentSessionRefKind::Id
                            && current_value != session_ref.value
                    },
                );
        if replacing_identity_only_session && !process_present {
            return None;
        }
        let owner_conflicts = self.current_session_owner_conflicts(&source, &agent_label);
        let foreground_takeover_allowed = owner_conflicts
            && self.foreground_agent_confirms_different_owner_takeover(
                &source,
                &agent_label,
                &session_ref,
                session_start_source.as_deref(),
            );
        if owner_conflicts && !foreground_takeover_allowed {
            return None;
        }
        if self
            .conflicting_same_owner_session_ref(
                &source,
                &agent_label,
                &session_ref,
                session_start_source.as_deref(),
            )
            .is_some()
        {
            return None;
        }
        let replaced_hook_session = self.same_owner_full_lifecycle_hook_authority_session_ref(
            &source,
            &agent_label,
            &session_ref,
        );
        if replaced_hook_session.is_some() && !session_replacement_allowed {
            return None;
        }

        let now = Instant::now();
        let previous_agent_label = self.effective_agent_label().map(str::to_string);
        let previous_known_agent = self.effective_known_agent();
        let previous_state = self.state;
        let previous_presentation = self.effective_presentation_for_state_at(previous_state, now);
        let previous_session = self.current_session_identity_for_persistence();
        if session_replacement_allowed || foreground_takeover_allowed {
            self.forget_stale_full_lifecycle_hook_session(&source, &agent_label, &session_ref);
        }
        if let Some(replaced_hook_session) = replaced_hook_session {
            self.remember_stale_full_lifecycle_hook_session(
                source.clone(),
                agent_label.clone(),
                replaced_hook_session,
            );
            self.hook_authority = None;
        } else if foreground_takeover_allowed {
            self.suppress_current_full_lifecycle_hook_authority(
                FullLifecycleHookSuppressionReason::HookClear,
            );
            self.hook_authority = None;
        }
        self.reconcile_agent_name_owner(&agent_label, Some(&session_ref));
        let persisted_session = crate::agent_resume::PersistedAgentSession {
            source,
            agent: agent_label,
            session_ref,
        };
        if self.managed_agent_launch_session.as_ref() == Some(&persisted_session) {
            self.managed_agent_launch_session = None;
        }
        self.persisted_agent_session = Some(persisted_session);
        self.persisted_agent_session_reported = true;
        let current_session = self.current_session_identity_for_persistence();
        if previous_session.is_some() && previous_session != current_session {
            // Rebinding can expose a cached Working screen; only a fresh report ends acquisition.
            self.agent_process_acquisition_pending = true;
        }
        Some(TerminalStateMutation {
            effective_state_change: self.recompute_effective_state(
                previous_agent_label,
                previous_known_agent,
                previous_state,
                previous_presentation,
                now,
            ),
            session_ref_changed: previous_session != current_session,
            agent_released: false,
        })
    }

    fn known_agent_label_conflicts_with_detected_agent(&self, agent_label: &str) -> bool {
        let Some(detected_agent) = self.detected_agent else {
            return false;
        };
        crate::detect::parse_agent_label(agent_label)
            .is_some_and(|hook_agent| hook_agent != detected_agent)
    }

    fn foreground_agent_confirms_different_owner_takeover(
        &self,
        source: &str,
        agent_label: &str,
        session_ref: &crate::agent_resume::AgentSessionRef,
        session_start_source: Option<&str>,
    ) -> bool {
        (source, agent_label) != ("herdr:grok", "grok")
            && Self::session_start_source_is_recognized(session_start_source)
            && self.foreground_agent_confirms_session_owner(source, agent_label, session_ref)
    }

    fn foreground_agent_confirms_hook_authority_takeover(
        &self,
        source: &str,
        agent_label: &str,
        session_ref: &Option<crate::agent_resume::AgentSessionRef>,
    ) -> bool {
        session_ref.as_ref().is_some_and(|session_ref| {
            self.foreground_agent_confirms_session_owner(source, agent_label, session_ref)
        })
    }

    fn foreground_agent_confirms_session_owner(
        &self,
        source: &str,
        agent_label: &str,
        session_ref: &crate::agent_resume::AgentSessionRef,
    ) -> bool {
        let Some(detected_agent) = self.detected_agent else {
            return false;
        };
        crate::detect::parse_agent_label(agent_label) == Some(detected_agent)
            && crate::agent_resume::plan(source, agent_label, session_ref).is_some()
    }

    fn accept_hook_report(&mut self, source: &str, seq: Option<u64>) -> bool {
        if !self.hook_report_is_newer(source, seq) {
            return false;
        }
        if let Some(seq) = seq {
            self.hook_report_sequences.insert(source.to_string(), seq);
        }
        true
    }

    #[cfg(test)]
    pub fn clear_hook_authority(
        &mut self,
        source: Option<&str>,
        seq: Option<u64>,
    ) -> Option<EffectiveStateChange> {
        self.clear_hook_authority_with_mutation(source, seq)
            .and_then(|mutation| mutation.effective_state_change)
    }

    pub fn clear_hook_authority_with_mutation(
        &mut self,
        source: Option<&str>,
        seq: Option<u64>,
    ) -> Option<TerminalStateMutation> {
        let sequence_source = source.map(str::to_string).or_else(|| {
            self.hook_authority
                .as_ref()
                .map(|authority| authority.source.clone())
        });
        let should_clear = self
            .hook_authority
            .as_ref()
            .is_some_and(|authority| source.is_none_or(|source| authority.source == source));
        if !should_clear {
            return None;
        }
        if let Some(source) = sequence_source.as_deref() {
            if !self.accept_hook_report(source, seq) {
                return None;
            }
        }

        let now = Instant::now();
        let previous_agent_label = self.effective_agent_label().map(str::to_string);
        let previous_known_agent = self.effective_known_agent();
        let previous_state = self.state;
        let previous_presentation = self.effective_presentation_for_state_at(previous_state, now);
        let previous_session = self.current_session_identity_for_persistence();
        self.suppress_current_full_lifecycle_hook_authority(
            FullLifecycleHookSuppressionReason::HookClear,
        );
        if let Some(authority) = self.hook_authority.take() {
            self.forget_reported_resume_of(&authority.source, &authority.agent_label);
        }
        self.persisted_agent_session = None;
        Some(TerminalStateMutation {
            effective_state_change: self.recompute_effective_state(
                previous_agent_label,
                previous_known_agent,
                previous_state,
                previous_presentation,
                now,
            ),
            session_ref_changed: previous_session.is_some(),
            agent_released: false,
        })
    }

    pub fn release_agent_with_mutation(
        &mut self,
        source: &str,
        agent_label: &str,
        seq: Option<u64>,
    ) -> Option<TerminalStateMutation> {
        if self.hook_authority.as_ref().is_some_and(|authority| {
            authority.agent_label != agent_label || authority.source != source
        }) {
            return None;
        }

        let matches_current_agent = self.effective_agent_label() == Some(agent_label);
        let matches_persisted_session = self.persisted_agent_session_matches(source, agent_label);
        if !matches_current_agent && !matches_persisted_session {
            return None;
        }
        if !self.accept_hook_report(source, seq) {
            return None;
        }
        let preserve_foreign_persisted_session = self
            .persisted_agent_session
            .as_ref()
            .is_some_and(|session| session.source != source || session.agent != agent_label);
        let process_owns_agent =
            crate::detect::parse_agent_label(agent_label).is_some_and(|agent| {
                self.detected_agent == Some(agent) && self.recent_agent_process_exit.is_none()
            });

        let now = Instant::now();
        let previous_agent_label = self.effective_agent_label().map(str::to_string);
        let previous_known_agent = self.effective_known_agent();
        let previous_state = self.state;
        let previous_presentation = self.effective_presentation_for_state_at(previous_state, now);
        let previous_session = self.current_session_identity_for_persistence();
        self.suppress_full_lifecycle_hook_report(
            source,
            agent_label,
            FullLifecycleHookSuppressionReason::HookClear,
        );
        if !process_owns_agent {
            self.detected_agent = None;
            self.fallback_state = AgentState::Unknown;
            self.fallback_visible_blocker = false;
            self.fallback_observed_at = None;
            self.clear_agent_name();
        }
        self.hook_authority = None;
        self.forget_reported_resume_of(source, agent_label);
        if !preserve_foreign_persisted_session {
            self.persisted_agent_session = None;
        }
        let current_session = self.current_session_identity_for_persistence();
        Some(TerminalStateMutation {
            effective_state_change: self.recompute_effective_state(
                previous_agent_label,
                previous_known_agent,
                previous_state,
                previous_presentation,
                now,
            ),
            session_ref_changed: previous_session != current_session,
            agent_released: !process_owns_agent,
        })
    }

    /// A reporter that Herdr cannot also identify by process holds this pane,
    /// so only the pane returning to its shell can tell Herdr it exited.
    pub fn self_reported_agent_active(&self) -> bool {
        self.hook_authority.as_ref().is_some_and(|authority| {
            crate::detect::parse_agent_label(&authority.agent_label).is_none()
        })
    }

    /// `observed_at` is when the idle shell was seen; a claim made after that
    /// belongs to a newer agent and must survive the delayed signal.
    pub fn clear_self_reported_agent(
        &mut self,
        observed_at: Instant,
    ) -> Option<TerminalStateMutation> {
        if !self.self_reported_agent_active() || !self.hook_authority_not_newer_than(observed_at) {
            return None;
        }
        let now = Instant::now();
        let previous_agent_label = self.effective_agent_label().map(str::to_string);
        let previous_known_agent = self.effective_known_agent();
        let previous_state = self.state;
        let previous_presentation = self.effective_presentation_for_state_at(previous_state, now);
        let previous_session = self.current_session_identity_for_persistence();
        self.hook_authority = None;
        self.set_reported_resume(None);
        self.detected_agent = None;
        self.fallback_state = AgentState::Unknown;
        self.fallback_visible_blocker = false;
        self.fallback_observed_at = None;
        self.clear_agent_name();
        let current_session = self.current_session_identity_for_persistence();
        Some(TerminalStateMutation {
            effective_state_change: self.recompute_effective_state(
                previous_agent_label,
                previous_known_agent,
                previous_state,
                previous_presentation,
                now,
            ),
            session_ref_changed: previous_session != current_session,
            agent_released: true,
        })
    }

    pub fn session_ref_is_current(
        &self,
        session_ref: &crate::agent_resume::AgentSessionRef,
    ) -> bool {
        self.current_session_identity_for_persistence()
            .is_some_and(|(_, _, kind, value)| {
                kind == session_ref.kind && value == session_ref.value
            })
    }

    /// A reporter may own the resume command when it holds the pane, or when it
    /// is an agent Herdr currently sees running there and can therefore see exit.
    pub fn can_record_reported_resume(&self, source: &str, agent_label: &str) -> bool {
        match self.hook_authority.as_ref() {
            Some(authority) => authority.source == source && authority.agent_label == agent_label,
            None => {
                self.recent_agent_process_exit.is_none()
                    && self.detected_agent.is_some()
                    && crate::detect::parse_agent_label(agent_label) == self.detected_agent
            }
        }
    }

    /// Same ordering rule as lifecycle reports, evaluated before the report is
    /// applied so a duplicate cannot pass as the report that was just accepted.
    pub fn hook_report_is_newer(&self, source: &str, seq: Option<u64>) -> bool {
        let last_seq = self.hook_report_sequences.get(source).copied();
        match seq {
            Some(seq) => last_seq.is_none_or(|last| seq > last),
            None => last_seq.is_none(),
        }
    }

    /// Callers must first check `hook_report_is_newer` against the state
    /// before the carrying report was applied.
    pub fn record_reported_resume(
        &mut self,
        source: &str,
        agent_label: &str,
        seq: Option<u64>,
        argv: Vec<String>,
    ) -> bool {
        if !self.can_record_reported_resume(source, agent_label) {
            return false;
        }
        if let Some(seq) = seq {
            let last = self
                .hook_report_sequences
                .entry(source.to_string())
                .or_insert(seq);
            *last = (*last).max(seq);
        }
        let resume = crate::agent_resume::ReportedAgentResume {
            source: source.to_string(),
            agent: agent_label.to_string(),
            argv,
        };
        if self.reported_resume.as_ref() == Some(&resume) {
            return false;
        }
        self.set_reported_resume(Some(resume));
        true
    }

    pub fn reported_resume(&self) -> Option<&crate::agent_resume::ReportedAgentResume> {
        self.reported_resume.as_ref()
    }

    /// Changes whenever the stored resume command changes, so callers can
    /// persist removals as well as new commands.
    pub fn reported_resume_revision(&self) -> u64 {
        self.reported_resume_revision
    }

    /// Drops the command once a different agent holds the pane.
    pub fn reconcile_reported_resume(&mut self) {
        let Some(resume) = self.reported_resume.as_ref() else {
            return;
        };
        let other_authority = self.hook_authority.as_ref().is_some_and(|authority| {
            authority.source != resume.source || authority.agent_label != resume.agent
        });
        let other_process = self
            .detected_agent
            .is_some_and(|agent| crate::detect::agent_label(agent) != resume.agent);
        if other_authority || other_process {
            self.set_reported_resume(None);
        }
    }

    pub fn restore_reported_resume(&mut self, resume: crate::agent_resume::ReportedAgentResume) {
        self.set_reported_resume(Some(resume));
    }

    fn set_reported_resume(&mut self, resume: Option<crate::agent_resume::ReportedAgentResume>) {
        if self.reported_resume != resume {
            self.reported_resume = resume;
            self.reported_resume_revision += 1;
        }
    }

    fn forget_reported_resume_of(&mut self, source: &str, agent_label: &str) {
        if self
            .reported_resume
            .as_ref()
            .is_some_and(|resume| resume.source == source && resume.agent == agent_label)
        {
            self.set_reported_resume(None);
        }
    }

    fn hook_authority_is_effective(&self, authority: &HookAuthority) -> bool {
        !crate::detect::full_lifecycle_hook_authority(&authority.source, &authority.agent_label)
            || crate::detect::parse_agent_label(&authority.agent_label).is_none_or(|agent| {
                self.detected_agent == Some(agent) && self.recent_agent_process_exit.is_none()
            })
    }

    pub fn effective_agent_label(&self) -> Option<&str> {
        self.hook_authority
            .as_ref()
            .filter(|authority| self.hook_authority_is_effective(authority))
            .map(|authority| authority.agent_label.as_str())
            .or_else(|| {
                self.recent_agent_process_exit
                    .is_none()
                    .then(|| self.detected_agent.map(crate::detect::agent_label))
                    .flatten()
            })
    }

    pub fn effective_known_agent(&self) -> Option<Agent> {
        self.effective_agent_label()
            .and_then(crate::detect::parse_agent_label)
    }

    pub(crate) fn unchanged_effective_state_change_at(&self, now: Instant) -> EffectiveStateChange {
        let agent_label = self.effective_agent_label().map(str::to_string);
        let known_agent = self.effective_known_agent();
        let state = self.state;
        let presentation = self.effective_presentation_for_state_at(state, now);
        EffectiveStateChange {
            previous_agent_label: agent_label.clone(),
            previous_known_agent: known_agent,
            previous_state: state,
            previous_presentation: presentation.clone(),
            agent_label,
            known_agent,
            state,
            presentation,
        }
    }

    pub fn full_lifecycle_hook_authority_active(&self) -> bool {
        self.live_full_lifecycle_hook_authority()
    }

    fn visible_blocker_overrides_hook(&self) -> bool {
        if self.live_full_lifecycle_hook_authority() {
            return false;
        }
        self.fallback_visible_blocker
            && self.fallback_not_older_than_hook()
            && self.hook_authority.as_ref().is_some_and(|authority| {
                authority.state != AgentState::Blocked
                    && crate::detect::parse_agent_label(&authority.agent_label)
                        == self.detected_agent
            })
    }

    fn live_full_lifecycle_hook_authority(&self) -> bool {
        self.hook_authority.as_ref().is_some_and(|authority| {
            self.hook_authority_is_effective(authority)
                && crate::detect::full_lifecycle_hook_authority(
                    &authority.source,
                    &authority.agent_label,
                )
        })
    }

    pub fn set_manual_label(&mut self, label: String) {
        let label = label.trim().to_string();
        self.manual_label = (!label.is_empty()).then_some(label);
    }

    pub fn clear_manual_label(&mut self) {
        self.manual_label = None;
    }

    pub fn set_agent_name(&mut self, name: String) {
        self.agent_name = (!name.is_empty()).then_some(name);
        self.agent_name_owner = self.agent_name.as_ref().and_then(|_| {
            self.hook_authority
                .as_ref()
                .map(|authority| AgentNameOwner {
                    agent_label: authority.agent_label.clone(),
                    session_ref: authority.session_ref.clone(),
                })
                .or_else(|| {
                    self.persisted_agent_session
                        .as_ref()
                        .map(|session| AgentNameOwner {
                            agent_label: session.agent.clone(),
                            session_ref: Some(session.session_ref.clone()),
                        })
                })
                .or_else(|| {
                    self.effective_agent_label()
                        .map(|agent_label| AgentNameOwner {
                            agent_label: agent_label.to_string(),
                            session_ref: None,
                        })
                })
        });
    }

    pub fn begin_managed_agent(
        &mut self,
        name: String,
        kind: Agent,
        now: Instant,
        settle_delay: Duration,
        timeout: Duration,
    ) {
        self.codex_prompt_ready = false;
        self.set_agent_name(name);
        self.agent_process_acquisition_pending = true;
        self.agent_name_owner = Some(AgentNameOwner {
            agent_label: crate::detect::agent_label(kind).to_string(),
            session_ref: None,
        });
        self.managed_agent = Some(ManagedAgent {
            kind,
            phase: ManagedAgentPhase::Pending {
                ready_after: Some(now.checked_add(settle_delay).unwrap_or(now)),
                deadline: now.checked_add(timeout).unwrap_or(now),
                observed_expected: false,
            },
        });
    }

    pub fn managed_agent_launch_pending(&self) -> bool {
        self.managed_agent.is_some_and(|managed| {
            matches!(
                managed.phase,
                ManagedAgentPhase::Pending { .. } | ManagedAgentPhase::Blocked
            )
        })
    }

    pub fn managed_agent_interactive_ready(&self) -> bool {
        self.managed_agent
            .is_some_and(|managed| matches!(managed.phase, ManagedAgentPhase::Active))
    }

    pub fn observe_codex_prompt_ready(&mut self, ready: bool) -> Option<TerminalStateMutation> {
        if self.detected_agent != Some(Agent::Codex)
            || self.recent_agent_process_exit.is_some()
            || !self.managed_agent.is_some_and(|managed| {
                managed.kind == Agent::Codex && managed.phase != ManagedAgentPhase::Active
            })
            || self.codex_prompt_ready == ready
        {
            return None;
        }
        self.codex_prompt_ready = ready;
        Some(TerminalStateMutation::default())
    }

    pub fn managed_agent_kind(&self) -> Option<Agent> {
        self.managed_agent.map(|managed| managed.kind)
    }

    pub fn next_managed_agent_deadline(&self) -> Option<Instant> {
        let ManagedAgentPhase::Pending {
            ready_after,
            deadline,
            ..
        } = self.managed_agent?.phase
        else {
            return None;
        };
        Some(ready_after.unwrap_or(deadline).min(deadline))
    }

    pub fn reconcile_managed_agent_at(&mut self, now: Instant, process_exited: bool) -> bool {
        let Some(managed) = self.managed_agent else {
            return false;
        };
        let known_agent = self.effective_known_agent();
        let observed_expected = match managed.phase {
            ManagedAgentPhase::Pending {
                observed_expected, ..
            } => observed_expected || known_agent == Some(managed.kind),
            ManagedAgentPhase::Blocked | ManagedAgentPhase::Active => false,
        };
        let clear = process_exited
            || known_agent.is_some_and(|agent| agent != managed.kind)
            || matches!(managed.phase, ManagedAgentPhase::Pending { .. })
                && observed_expected
                && known_agent.is_none();
        if clear {
            self.clear_agent_name();
            return true;
        }
        if managed.phase == ManagedAgentPhase::Blocked {
            if known_agent == Some(managed.kind)
                && (self.state == AgentState::Idle
                    || managed.kind == Agent::Codex
                        && self.state == AgentState::Unknown
                        && self.codex_prompt_ready)
            {
                self.managed_agent = Some(ManagedAgent {
                    kind: managed.kind,
                    phase: ManagedAgentPhase::Active,
                });
                self.managed_agent_launch_session = None;
                return true;
            }
            return false;
        }
        if let ManagedAgentPhase::Pending {
            ready_after,
            deadline,
            observed_expected: previous_observed_expected,
        } = managed.phase
        {
            if known_agent == Some(managed.kind) && self.state == AgentState::Blocked {
                self.managed_agent = Some(ManagedAgent {
                    kind: managed.kind,
                    phase: ManagedAgentPhase::Blocked,
                });
                return true;
            }
            if now >= deadline {
                self.clear_agent_name();
                return true;
            }
            if ready_after.is_none_or(|ready_after| now >= ready_after) {
                if known_agent == Some(managed.kind)
                    && (self.state == AgentState::Idle
                        || managed.kind == Agent::Codex
                            && self.state == AgentState::Unknown
                            && self.codex_prompt_ready)
                {
                    self.managed_agent = Some(ManagedAgent {
                        kind: managed.kind,
                        phase: ManagedAgentPhase::Active,
                    });
                    self.managed_agent_launch_session = None;
                    return true;
                }
                if ready_after.is_some() {
                    self.managed_agent = Some(ManagedAgent {
                        kind: managed.kind,
                        phase: ManagedAgentPhase::Pending {
                            ready_after: None,
                            deadline,
                            observed_expected,
                        },
                    });
                    return true;
                }
            }
            if observed_expected != previous_observed_expected {
                self.managed_agent = Some(ManagedAgent {
                    kind: managed.kind,
                    phase: ManagedAgentPhase::Pending {
                        ready_after,
                        deadline,
                        observed_expected,
                    },
                });
                return true;
            }
        }
        false
    }

    pub fn restore_managed_agent(&mut self, name: String, kind: Agent) {
        self.set_agent_name(name);
        self.agent_name_owner = Some(AgentNameOwner {
            agent_label: crate::detect::agent_label(kind).to_string(),
            session_ref: None,
        });
        self.managed_agent = Some(ManagedAgent {
            kind,
            phase: ManagedAgentPhase::Active,
        });
    }

    pub fn clear_agent_name(&mut self) {
        self.codex_prompt_ready = false;
        if self
            .managed_agent_launch_session
            .take()
            .as_ref()
            .is_some_and(|session| self.persisted_agent_session.as_ref() == Some(session))
        {
            self.persisted_agent_session = None;
        }
        self.agent_name = None;
        self.agent_name_owner = None;
        self.managed_agent = None;
    }

    pub fn clear_agent_runtime_identity_after_respawn(&mut self) {
        self.detected_agent = None;
        self.fallback_state = AgentState::Unknown;
        self.fallback_visible_blocker = false;
        self.fallback_observed_at = None;
        self.hook_authority = None;
        self.persisted_agent_session = None;
        self.agent_metadata.clear();
        self.metadata_report_agents.clear();
        self.suppressed_full_lifecycle_hook_reports.clear();
        self.stale_full_lifecycle_hook_sessions.clear();
        self.state = AgentState::Unknown;
        self.last_agent_state_change_seq = None;
        self.last_agent_completion_seq = None;
        self.launch_argv = None;
        self.respawn_shell_on_exit = false;
        self.recent_agent_process_exit = None;
        self.agent_process_acquisition_pending = false;
        self.pending_agent_resume_plan = None;
        self.set_agent_launch(None);
        self.reported_session = None;
        self.process_sessions = ProcessSessions::default();
        self.agent_launch_restore = None;
        self.persisted_agent_session_reported = false;
        self.clear_agent_name();
    }

    pub fn is_agent_terminal(&self) -> bool {
        self.agent_name.is_some() || self.effective_agent_label().is_some()
    }

    fn reconcile_agent_name_owner(
        &mut self,
        agent_label: &str,
        session_ref: Option<&crate::agent_resume::AgentSessionRef>,
    ) {
        if self.agent_name.is_none() {
            return;
        }
        if self.managed_agent.is_some_and(|managed| {
            crate::detect::parse_agent_label(agent_label) == Some(managed.kind)
        }) {
            return;
        }
        match self.agent_name_owner.as_mut() {
            Some(owner)
                if owner.agent_label != agent_label
                    || owner
                        .session_ref
                        .as_ref()
                        .zip(session_ref)
                        .is_some_and(|(current, incoming)| current != incoming) =>
            {
                self.agent_name = None;
                self.agent_name_owner = None;
            }
            Some(owner) if owner.session_ref.is_none() && session_ref.is_some() => {
                owner.session_ref = session_ref.cloned();
            }
            None => {
                self.agent_name_owner = Some(AgentNameOwner {
                    agent_label: agent_label.to_string(),
                    session_ref: session_ref.cloned(),
                })
            }
            _ => {}
        }
    }

    pub fn border_label(&self, show_agent_labels: bool) -> Option<String> {
        self.effective_title().or_else(|| {
            self.manual_label.clone().or_else(|| {
                show_agent_labels
                    .then(|| {
                        self.effective_display_agent()
                            .or_else(|| self.effective_agent_label().map(str::to_string))
                    })
                    .flatten()
            })
        })
    }

    fn recompute_effective_state(
        &mut self,
        previous_agent_label: Option<String>,
        previous_known_agent: Option<Agent>,
        previous_state: AgentState,
        previous_presentation: EffectivePresentation,
        now: Instant,
    ) -> Option<EffectiveStateChange> {
        let state = if self.visible_blocker_overrides_hook() {
            AgentState::Blocked
        } else {
            self.hook_authority
                .as_ref()
                .filter(|authority| self.hook_authority_is_effective(authority))
                .map(|authority| authority.state)
                .unwrap_or(self.fallback_state)
        };
        let agent_label = self.effective_agent_label().map(str::to_string);
        let known_agent = self.effective_known_agent();

        let presentation = self.effective_presentation_for_state_at(state, now);
        self.clear_expiry_pending_for_hidden_metadata();

        if previous_agent_label == agent_label
            && previous_state == state
            && previous_presentation == presentation
        {
            return None;
        }

        self.state = state;
        Some(EffectiveStateChange {
            previous_agent_label,
            previous_known_agent,
            previous_state,
            previous_presentation,
            agent_label,
            known_agent,
            state,
            presentation,
        })
    }
}

pub(crate) fn stabilize_agent_detection(detection: crate::detect::AgentDetection) -> AgentState {
    detection.state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::AgentDetection;

    fn test_terminal() -> TerminalState {
        TerminalState::new(TerminalId::alloc(), "/tmp".into())
    }

    fn test_session_path(name: &str) -> String {
        std::env::current_dir()
            .unwrap()
            .join(name)
            .display()
            .to_string()
    }

    fn anchor_full_lifecycle_session(
        terminal: &mut TerminalState,
        agent: Agent,
        source: &str,
        agent_label: &str,
        session_ref: crate::agent_resume::AgentSessionRef,
    ) {
        terminal.set_detected_state(Some(agent), terminal.fallback_state);
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: source.into(),
            agent: agent_label.into(),
            session_ref,
        });
    }

    fn known_options(options: &[&str]) -> crate::agent_resume::AgentLaunchOptions {
        crate::agent_resume::AgentLaunchOptions::Known(
            options.iter().map(|option| option.to_string()).collect(),
        )
    }

    #[test]
    fn agent_launch_observation_replaces_records_for_new_processes() {
        let mut terminal = test_terminal();
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(10),
            known_options(&["--bypass"]),
            None,
        );
        let first_revision = terminal.agent_launch_revision();
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--bypass".to_string()])
        );

        // The same process observed again changes nothing.
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(10),
            known_options(&[]),
            None,
        );
        assert_eq!(terminal.agent_launch_revision(), first_revision);
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--bypass".to_string()])
        );

        // A replacement Claude process in a new process group replaces them.
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(11),
            known_options(&[]),
            None,
        );
        assert_ne!(terminal.agent_launch_revision(), first_revision);
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(Vec::new())
        );

        // A different agent replaces them too, and they never apply to Claude.
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(11),
            known_options(&["--search"]),
            None,
        );
        assert_eq!(terminal.launch_options_for_session("claude"), None);
        assert_eq!(
            terminal.launch_options_for_session("codex"),
            Some(vec!["--search".to_string()])
        );
    }

    #[test]
    fn a_second_agent_process_replaces_the_launch_record() {
        let mut terminal = test_terminal();
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(100),
            known_options(&["--placeholder-a"]),
            None,
        );
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(101),
            known_options(&["--placeholder-b"]),
            None,
        );
        assert_eq!(
            terminal.agent_launch().unwrap().process(),
            Some(ProcessInstance::unknown_start(101))
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-b".to_string()])
        );

        // A fallback seed still binds to the bare resume process it started.
        let mut terminal = test_terminal();
        let not_recorded = crate::agent_resume::AgentLaunchOptions::Unavailable(
            crate::agent_resume::LaunchOptionsUnavailable::NotRecorded,
        );
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: not_recorded.clone(),
            resumed_without_options: true,
            launch: SeededLaunch::Started { session: None },
        });
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(100),
            known_options(&[]),
            None,
        );
        assert_eq!(terminal.agent_launch().unwrap().options, not_recorded);
        assert!(terminal.resumed_without_launch_options());
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(101),
            known_options(&["--placeholder-b"]),
            None,
        );
        assert!(!terminal.resumed_without_launch_options());
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-b".to_string()])
        );
    }

    #[test]
    fn fallback_seed_keeps_its_mark_for_an_unrecognized_bare_process() {
        let mut terminal = test_terminal();
        let not_recorded = crate::agent_resume::AgentLaunchOptions::Unavailable(
            crate::agent_resume::LaunchOptionsUnavailable::NotRecorded,
        );
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: not_recorded.clone(),
            resumed_without_options: true,
            launch: SeededLaunch::Started { session: None },
        });
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(100),
            crate::agent_resume::AgentLaunchOptions::Unavailable(
                crate::agent_resume::LaunchOptionsUnavailable::UnrecognizedCommand,
            ),
            None,
        );
        let record = terminal.agent_launch().unwrap();
        assert_eq!(record.process(), Some(ProcessInstance::unknown_start(100)));
        assert_eq!(record.options, not_recorded);
        assert!(terminal.resumed_without_launch_options());
    }

    #[test]
    fn fallback_seed_keeps_its_mark_for_a_wrapper_resuming_the_seeded_conversation() {
        let not_recorded = crate::agent_resume::AgentLaunchOptions::Unavailable(
            crate::agent_resume::LaunchOptionsUnavailable::NotRecorded,
        );
        let seeded = || {
            let mut terminal = test_terminal();
            terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
                source: "herdr:claude".into(),
                agent: "claude".into(),
                session_ref: crate::agent_resume::AgentSessionRef::id("placeholder-session")
                    .unwrap(),
            });
            terminal.seed_agent_launch(AgentLaunchSeed {
                agent: Agent::Claude,
                options: not_recorded.clone(),
                resumed_without_options: true,
                launch: SeededLaunch::Started {
                    session: crate::agent_resume::AgentSessionRef::id("placeholder-session"),
                },
            });
            terminal
        };

        // A wrapper adds its own option to the restored resume command.
        let mut terminal = seeded();
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(100),
            known_options(&["--placeholder-wrapper"]),
            Some("placeholder-session"),
        );
        let record = terminal.agent_launch().unwrap();
        assert_eq!(record.process(), Some(ProcessInstance::unknown_start(100)));
        assert_eq!(record.options, not_recorded);
        assert!(terminal.resumed_without_launch_options());
        assert_eq!(terminal.launch_options_for_session("claude"), None);

        // A process resuming another conversation, or starting a new one, takes
        // its own options; it does not run the seeded conversation, so they
        // wait for its own.
        for resumed in [Some("placeholder-other"), None] {
            let mut terminal = seeded();
            terminal.observe_agent_launch(
                Agent::Claude,
                ProcessInstance::unknown_start(100),
                known_options(&["--placeholder-wrapper"]),
                resumed,
            );
            assert!(!terminal.resumed_without_launch_options());
            assert_eq!(
                terminal.agent_launch().unwrap().options,
                known_options(&["--placeholder-wrapper"])
            );
            assert_eq!(terminal.launch_options_for_session("claude"), None);
        }
    }

    fn launch_record(
        agent: Agent,
        process_id: Option<u32>,
        resumed_without_options: bool,
        session: Option<&str>,
        earlier_session: Option<&str>,
    ) -> ObservedAgentLaunch {
        let id = |id: &str| crate::agent_resume::AgentSessionRef::id(id).unwrap();
        let earlier = earlier_session.map(id);
        let pairing = match session.map(id) {
            Some(session) => Pairing::Assumed { session, earlier },
            None => Pairing::Unpaired { earlier },
        };
        ObservedAgentLaunch {
            agent,
            options: known_options(&["--placeholder-seeded"]),
            resumed_without_options,
            state: match process_id {
                Some(process_id) => LaunchState::Bound {
                    process: ProcessInstance::unknown_start(process_id),
                    pairing,
                    kept_seed: None,
                },
                None => LaunchState::Started(pairing),
            },
        }
    }

    /// `record` with its assumed conversation confirmed.
    fn confirmed(mut record: ObservedAgentLaunch) -> ObservedAgentLaunch {
        if let LaunchState::Started(pairing) | LaunchState::Bound { pairing, .. } =
            &mut record.state
        {
            if let Pairing::Assumed { session, .. } = pairing {
                *pairing = Pairing::Confirmed(session.clone());
            }
        }
        record
    }

    #[test]
    fn seed_keeps_record_decides_every_case() {
        use crate::agent_resume::{AgentLaunchOptions, LaunchOptionsUnavailable};
        let unavailable = AgentLaunchOptions::Unavailable;
        let unreadable = unavailable(LaunchOptionsUnavailable::CommandLineUnreadable);
        let unrecognized = unavailable(LaunchOptionsUnavailable::UnrecognizedCommand);
        let empty = known_options(&[]);
        let wrapper = known_options(&["--placeholder-wrapper"]);
        let kept = launch_record(Agent::Claude, None, false, Some("seeded"), None);
        let fallback = launch_record(Agent::Claude, None, true, Some("seeded"), None);
        let bound = launch_record(Agent::Claude, Some(10), true, Some("seeded"), None);
        let mut kept_through_launcher = bound.clone();
        if let LaunchState::Bound { kept_seed, .. } = &mut kept_through_launcher.state {
            *kept_seed = Some(ReadingRank::Unrecognized);
        }
        // A saved launch that nothing runs, with the mark.
        let saved = ObservedAgentLaunch {
            state: LaunchState::Saved(crate::agent_resume::AgentSessionRef::id("seeded").unwrap()),
            ..fallback.clone()
        };
        for (case, seed, observed, resumed, keeps) in [
            ("bound record is no seed", &bound, &unreadable, None, false),
            (
                "seed kept through this process, agent's own options",
                &kept_through_launcher,
                &wrapper,
                None,
                false,
            ),
            (
                "seed kept through this process, no options",
                &kept_through_launcher,
                &empty,
                None,
                true,
            ),
            (
                "seed kept through this process, resumes seeded",
                &kept_through_launcher,
                &wrapper,
                Some("seeded"),
                true,
            ),
            ("kept seed, unreadable", &kept, &unreadable, None, true),
            ("kept seed, unrecognized", &kept, &unrecognized, None, false),
            ("kept seed, no options", &kept, &empty, None, false),
            (
                "kept seed, resumes seeded",
                &kept,
                &wrapper,
                Some("seeded"),
                false,
            ),
            (
                "fallback seed, unreadable",
                &fallback,
                &unreadable,
                None,
                true,
            ),
            (
                "fallback seed, unrecognized",
                &fallback,
                &unrecognized,
                None,
                true,
            ),
            ("fallback seed, no options", &fallback, &empty, None, true),
            (
                "fallback seed, resumes seeded",
                &fallback,
                &wrapper,
                Some("seeded"),
                true,
            ),
            (
                "fallback seed, resumes another",
                &fallback,
                &wrapper,
                Some("other"),
                false,
            ),
            ("saved launch, unreadable", &saved, &unreadable, None, false),
            ("saved launch, no options", &saved, &empty, None, false),
            (
                "saved launch, resumes seeded",
                &saved,
                &wrapper,
                Some("seeded"),
                false,
            ),
            (
                "fallback seed, own options",
                &fallback,
                &wrapper,
                None,
                false,
            ),
        ] {
            assert_eq!(
                seed_keeps_record(seed, &ProcessInstance::unknown_start(10), observed, resumed),
                keeps,
                "{case}"
            );
        }
        // A seed kept through one process is bound to it.
        assert!(!seed_keeps_record(
            &kept_through_launcher,
            &ProcessInstance::unknown_start(11),
            &empty,
            None
        ));
    }

    #[test]
    fn session_for_process_decides_every_case() {
        use ResumeReading::{Nothing, Resumes, Unknown};
        let id = |id: &str| crate::agent_resume::AgentSessionRef::id(id).unwrap();
        let current = id("current");
        let seed = launch_record(Agent::Claude, None, false, Some("current"), None);
        let seed_of_other = launch_record(Agent::Claude, None, false, Some("other"), None);
        let seed_of_nothing = launch_record(Agent::Claude, None, false, None, None);
        let reported_seed = confirmed(seed.clone());
        let path = crate::agent_resume::AgentSessionRef::path(test_session_path(
            "placeholder-session.jsonl",
        ))
        .unwrap();
        let path_seed = ObservedAgentLaunch {
            state: LaunchState::Started(Pairing::Assumed {
                session: path.clone(),
                earlier: None,
            }),
            ..seed_of_nothing.clone()
        };
        let owner = confirmed(launch_record(
            Agent::Claude,
            Some(10),
            false,
            Some("current"),
            None,
        ));
        let assumed_owner = launch_record(Agent::Claude, Some(10), false, Some("current"), None);
        let unpaired = launch_record(Agent::Claude, Some(10), false, None, None);
        let unpaired_after_owner =
            launch_record(Agent::Claude, Some(10), false, None, Some("current"));
        let owner_of_other = confirmed(launch_record(
            Agent::Claude,
            Some(10),
            false,
            Some("other"),
            None,
        ));
        let codex_owner = confirmed(launch_record(
            Agent::Codex,
            Some(10),
            false,
            Some("current"),
            None,
        ));
        let other = Resumes("other");
        let resumes_current = Resumes("current");
        // As observation decides: the report's evidence of the current
        // conversation and the process's own reading are what it was shown.
        let decide = |previous: Option<&ObservedAgentLaunch>,
                      reported: bool,
                      report: Option<&SessionReportEvidence>,
                      process_id: u32,
                      reading: ResumeReading| {
            let process = ProcessInstance::unknown_start(process_id);
            let mut shown = ProcessSessions::default();
            // A bound record is confirmed by its process's command line, or
            // by a report, which is newer than any reading of it.
            if let Some(LaunchState::Bound {
                process,
                pairing: Pairing::Confirmed(session),
                ..
            }) = previous.map(|previous| &previous.state)
            {
                shown.show(&[*process], session, ShownBy::Reporter);
            }
            if let Some(named) = report.and_then(SessionReportEvidence::named_processes) {
                shown.show(named, &current, ShownBy::Reporter);
            }
            if let Resumes(resumed) = reading {
                shown.read(&process, Some(id(resumed)));
            }
            session_for_process(
                previous,
                Agent::Claude,
                Some(&current),
                reported && report.is_none(),
                &shown,
                &process,
                reading,
            )
        };
        // case, previous record, current conversation reported, process, reading,
        // owned conversation, confirmed, carried earlier conversation
        for (case, previous, reported, process_id, reading, expected, is_confirmed, carried) in [
            (
                "first process, restored conversation",
                None,
                false,
                10,
                Nothing,
                None,
                false,
                None,
            ),
            (
                "first process, reported conversation",
                None,
                true,
                10,
                Nothing,
                Some("current"),
                false,
                None,
            ),
            (
                "first unreadable process, restored conversation",
                None,
                false,
                10,
                Unknown,
                None,
                false,
                None,
            ),
            (
                "first unreadable process, reported conversation",
                None,
                true,
                10,
                Unknown,
                Some("current"),
                false,
                None,
            ),
            (
                "first process resuming the restored one",
                None,
                false,
                10,
                resumes_current,
                Some("current"),
                true,
                None,
            ),
            (
                "first process resuming another one",
                None,
                true,
                10,
                other,
                Some("other"),
                true,
                None,
            ),
            (
                "seed's process resuming the one it named",
                Some(&seed),
                false,
                10,
                resumes_current,
                Some("current"),
                true,
                None,
            ),
            (
                "seed's unreadable process",
                Some(&seed),
                false,
                10,
                Unknown,
                Some("current"),
                false,
                None,
            ),
            (
                "seed's process starting a new conversation",
                Some(&seed),
                false,
                10,
                Nothing,
                None,
                false,
                None,
            ),
            (
                "seed's process resuming another one",
                Some(&seed),
                false,
                10,
                other,
                Some("other"),
                true,
                None,
            ),
            (
                "seed's process resuming the one it named, not yet current",
                Some(&seed_of_other),
                false,
                10,
                other,
                Some("other"),
                true,
                None,
            ),
            (
                "seed naming nothing, reported conversation",
                Some(&seed_of_nothing),
                true,
                10,
                Nothing,
                Some("current"),
                false,
                None,
            ),
            (
                "seed naming nothing, restored conversation",
                Some(&seed_of_nothing),
                false,
                10,
                Nothing,
                None,
                false,
                None,
            ),
            (
                "seed paired by a report",
                Some(&reported_seed),
                false,
                10,
                Nothing,
                Some("current"),
                true,
                None,
            ),
            (
                "same process keeps a confirmed pairing",
                Some(&owner),
                false,
                10,
                other,
                Some("current"),
                true,
                None,
            ),
            (
                "same process decides an assumed pairing again: resumes another",
                Some(&assumed_owner),
                false,
                10,
                other,
                Some("other"),
                true,
                None,
            ),
            (
                "same process decides an assumed pairing again: resumes it",
                Some(&assumed_owner),
                false,
                10,
                resumes_current,
                Some("current"),
                true,
                None,
            ),
            (
                "same process decides an assumed pairing again: new conversation",
                Some(&assumed_owner),
                false,
                10,
                Nothing,
                None,
                false,
                None,
            ),
            (
                "same process keeps an assumed pairing it cannot read",
                Some(&assumed_owner),
                false,
                10,
                Unknown,
                Some("current"),
                false,
                None,
            ),
            (
                "same unpaired process resuming the current one",
                Some(&unpaired),
                false,
                10,
                resumes_current,
                Some("current"),
                true,
                None,
            ),
            (
                "same unpaired process stays unpaired",
                Some(&unpaired),
                false,
                10,
                Nothing,
                None,
                false,
                None,
            ),
            (
                "same unpaired process carries the earlier one",
                Some(&unpaired_after_owner),
                true,
                10,
                Nothing,
                None,
                false,
                Some("current"),
            ),
            (
                "new process after the owner",
                Some(&owner),
                true,
                11,
                Nothing,
                None,
                false,
                Some("current"),
            ),
            (
                "new process after an unpaired one",
                Some(&unpaired_after_owner),
                true,
                11,
                Nothing,
                None,
                false,
                Some("current"),
            ),
            (
                "new process after an unpaired first one, reported",
                Some(&unpaired),
                true,
                11,
                Unknown,
                Some("current"),
                false,
                None,
            ),
            (
                "new process after another owner, reported",
                Some(&owner_of_other),
                true,
                11,
                Nothing,
                Some("current"),
                false,
                Some("other"),
            ),
            (
                "new process after another owner, restored",
                Some(&owner_of_other),
                false,
                11,
                Nothing,
                None,
                false,
                Some("other"),
            ),
            (
                "new process resuming the current one",
                Some(&owner),
                false,
                11,
                resumes_current,
                Some("current"),
                true,
                Some("current"),
            ),
            (
                "new process resuming another one",
                Some(&owner),
                true,
                11,
                other,
                Some("other"),
                true,
                None,
            ),
            (
                "other agent's record",
                Some(&codex_owner),
                true,
                11,
                Nothing,
                None,
                false,
                Some("current"),
            ),
        ] {
            assert_eq!(
                decide(previous, reported, None, process_id, reading),
                match (expected.map(id), is_confirmed) {
                    (Some(session), true) => Pairing::Confirmed(session),
                    (Some(session), false) => Pairing::Assumed {
                        session,
                        earlier: carried.map(id),
                    },
                    (None, _) => Pairing::Unpaired {
                        earlier: carried.map(id),
                    },
                },
                "{case}"
            );
        }
        assert_eq!(
            session_for_process(
                None,
                Agent::Claude,
                None,
                false,
                &ProcessSessions::default(),
                &ProcessInstance::unknown_start(10),
                Nothing
            ),
            Pairing::Unpaired { earlier: None },
            "no conversation yet"
        );
        // A report whose sender the server read.
        let reported_by = |reporter: &[u32]| {
            SessionReportEvidence::Reporter(
                reporter
                    .iter()
                    .map(|&pid| ProcessInstance::unknown_start(pid))
                    .collect(),
            )
        };
        let beside_another_agent = foreground_agents(&[10, 12]);
        for (case, previous, report, reading, expected) in [
            (
                "reported by this process, record of another",
                Some(&owner_of_other),
                reported_by(&[11, 7]),
                Nothing,
                Pairing::Confirmed(current.clone()),
            ),
            (
                "reported by another process",
                None,
                reported_by(&[12]),
                Nothing,
                Pairing::Unpaired { earlier: None },
            ),
            (
                "seed naming what another process reported",
                Some(&seed),
                reported_by(&[12]),
                Unknown,
                Pairing::Unpaired { earlier: None },
            ),
            (
                "resumes what another process reported",
                Some(&seed),
                reported_by(&[12]),
                resumes_current,
                Pairing::Confirmed(current.clone()),
            ),
            (
                "reported beside another agent",
                None,
                beside_another_agent.clone(),
                Nothing,
                Pairing::Unpaired { earlier: None },
            ),
            (
                "seed naming what was reported beside another agent",
                Some(&seed),
                beside_another_agent,
                Unknown,
                Pairing::Assumed {
                    session: current.clone(),
                    earlier: None,
                },
            ),
            (
                "reported while the job was unreadable",
                None,
                SessionReportEvidence::ForegroundAgents(None),
                Nothing,
                Pairing::Unpaired { earlier: None },
            ),
            (
                "reported while the job ran no agent",
                None,
                foreground_agents(&[]),
                Nothing,
                Pairing::Unpaired { earlier: None },
            ),
            (
                "reported while the job ran one other agent",
                None,
                foreground_agents(&[12]),
                Nothing,
                Pairing::Unpaired { earlier: None },
            ),
            (
                "reported while the job ran only this process",
                None,
                foreground_agents(&[11]),
                Nothing,
                Pairing::Confirmed(current.clone()),
            ),
        ] {
            assert_eq!(
                decide(previous, true, Some(&report), 11, reading),
                expected,
                "{case}"
            );
        }
        // No command line names a conversation kept as a path, so the seed's
        // pairing holds for its process.
        assert_eq!(
            decide(Some(&path_seed), false, None, 10, Nothing).session(),
            Some(&path),
            "seed naming a path"
        );
    }

    /// A launcher script (`/usr/bin/python3 <bin>/claude ...`) that execs the
    /// real Claude in its own process, observed before and after the exec.
    fn observe_launcher_then_agent(terminal: &mut TerminalState, resumed: Option<&str>) {
        let tail = ["--model", "placeholder", "--permission-mode", "plan"];
        let resume = resumed.map(|id| ["--resume", id]);
        let launcher = ["/usr/bin/python3", "/home/placeholder/bin/claude"]
            .into_iter()
            .chain(tail)
            .chain(resume.into_iter().flatten())
            .map(String::from)
            .collect::<Vec<_>>();
        let agent = ["/home/placeholder/.local/share/claude/latest/claude"]
            .into_iter()
            .chain(tail)
            .chain(resume.into_iter().flatten())
            .map(String::from)
            .collect::<Vec<_>>();
        for argv in [&launcher, &launcher, &agent] {
            let options =
                crate::agent_resume::launch_options_from_process_argv(Agent::Claude, Some(argv))
                    .unwrap();
            let resumed =
                crate::agent_resume::resumed_session_from_process_argv(Agent::Claude, argv);
            terminal.observe_agent_launch(
                Agent::Claude,
                ProcessInstance::unknown_start(100),
                options,
                resumed.as_deref(),
            );
        }
    }

    #[test]
    fn launcher_exec_in_the_same_process_records_the_agent_options() {
        let options = || {
            ["--model", "placeholder", "--permission-mode", "plan"]
                .map(String::from)
                .to_vec()
        };
        // Started by hand.
        let mut terminal = test_terminal();
        observe_launcher_then_agent(&mut terminal, None);
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(options())
        );

        // Started by `herdr agent start` with the same options.
        let mut terminal = test_terminal();
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: known_options(&options().iter().map(String::as_str).collect::<Vec<_>>()),
            resumed_without_options: false,
            launch: SeededLaunch::Started { session: None },
        });
        observe_launcher_then_agent(&mut terminal, None);
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(options())
        );

        // A restore that kept the options types them before `--resume <id>`.
        let mut terminal = test_terminal();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("placeholder-session").unwrap(),
        });
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: known_options(&options().iter().map(String::as_str).collect::<Vec<_>>()),
            resumed_without_options: false,
            launch: SeededLaunch::Started {
                session: crate::agent_resume::AgentSessionRef::id("placeholder-session"),
            },
        });
        observe_launcher_then_agent(&mut terminal, Some("placeholder-session"));
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(options())
        );
        assert!(!terminal.resumed_without_launch_options());
    }

    #[test]
    fn a_launcher_bound_to_a_seed_does_not_pair_the_agent_resuming_another_conversation() {
        let mut terminal = test_terminal();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("placeholder-a").unwrap(),
        });
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: known_options(&["--placeholder-seeded"]),
            resumed_without_options: false,
            launch: SeededLaunch::Started {
                session: crate::agent_resume::AgentSessionRef::id("placeholder-a"),
            },
        });
        // The launcher is paired with A by assumption; the agent it execs
        // resumes B, so its options are paired with B, not A.
        observe_launcher_then_agent(&mut terminal, Some("placeholder-b"));
        let record = terminal.agent_launch().unwrap();
        assert_eq!(
            record.options,
            known_options(&["--model", "placeholder", "--permission-mode", "plan"])
        );
        assert_eq!(
            record.session(),
            crate::agent_resume::AgentSessionRef::id("placeholder-b").as_ref()
        );
        assert_eq!(terminal.launch_options_for_session("claude"), None);
    }

    #[test]
    fn an_unsupported_option_reading_takes_a_later_parsed_reading_of_the_same_process() {
        let unsupported = crate::agent_resume::AgentLaunchOptions::Unavailable(
            crate::agent_resume::LaunchOptionsUnavailable::UnsupportedOption,
        );
        let mut terminal = test_terminal();
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(100),
            unsupported.clone(),
            None,
        );
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(100),
            known_options(&["--placeholder"]),
            None,
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder".to_string()])
        );
        // A later unsupported reading tells less and changes nothing.
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(100),
            unsupported,
            None,
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder".to_string()])
        );
    }

    #[test]
    fn launcher_that_never_execs_keeps_its_record() {
        let launcher = [
            "/usr/bin/python3",
            "/home/placeholder/bin/claude",
            "--model",
            "placeholder",
        ]
        .map(String::from);
        let unrecognized = crate::agent_resume::AgentLaunchOptions::Unavailable(
            crate::agent_resume::LaunchOptionsUnavailable::UnrecognizedCommand,
        );
        let mut terminal = test_terminal();
        for _ in 0..2 {
            terminal.observe_agent_launch(
                Agent::Claude,
                ProcessInstance::unknown_start(100),
                crate::agent_resume::launch_options_from_process_argv(
                    Agent::Claude,
                    Some(&launcher),
                )
                .unwrap(),
                None,
            );
        }
        assert_eq!(terminal.agent_launch().unwrap().options, unrecognized);
        let revision = terminal.agent_launch_revision();
        // An unreadable read of it later tells less and changes nothing.
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(100),
            crate::agent_resume::AgentLaunchOptions::Unavailable(
                crate::agent_resume::LaunchOptionsUnavailable::CommandLineUnreadable,
            ),
            None,
        );
        assert_eq!(terminal.agent_launch().unwrap().options, unrecognized);
        assert_eq!(terminal.agent_launch_revision(), revision);

        // A fallback seed keeps its options and mark through a launcher that
        // never execs.
        let not_recorded = crate::agent_resume::AgentLaunchOptions::Unavailable(
            crate::agent_resume::LaunchOptionsUnavailable::NotRecorded,
        );
        let mut terminal = test_terminal();
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: not_recorded.clone(),
            resumed_without_options: true,
            launch: SeededLaunch::Started { session: None },
        });
        for _ in 0..2 {
            terminal.observe_agent_launch(
                Agent::Claude,
                ProcessInstance::unknown_start(100),
                crate::agent_resume::launch_options_from_process_argv(
                    Agent::Claude,
                    Some(&launcher),
                )
                .unwrap(),
                None,
            );
        }
        assert_eq!(terminal.agent_launch().unwrap().options, not_recorded);
        assert!(terminal.resumed_without_launch_options());
    }

    #[test]
    fn a_fork_of_the_seeded_conversation_does_not_keep_the_fallback_seed() {
        let mut terminal = test_terminal();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("placeholder-session").unwrap(),
        });
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: crate::agent_resume::AgentLaunchOptions::Unavailable(
                crate::agent_resume::LaunchOptionsUnavailable::NotRecorded,
            ),
            resumed_without_options: true,
            launch: SeededLaunch::Started {
                session: crate::agent_resume::AgentSessionRef::id("placeholder-session"),
            },
        });
        let argv = [
            "claude",
            "--resume",
            "placeholder-session",
            "--fork-session",
            "--model",
            "placeholder",
        ]
        .map(String::from);
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(100),
            crate::agent_resume::launch_options_from_process_argv(Agent::Claude, Some(&argv))
                .unwrap(),
            crate::agent_resume::resumed_session_from_process_argv(Agent::Claude, &argv).as_deref(),
        );
        assert_eq!(
            terminal.agent_launch().unwrap().options,
            known_options(&["--model", "placeholder"])
        );
        assert!(!terminal.resumed_without_launch_options());
    }

    #[test]
    fn a_fallback_seed_kept_through_a_launcher_decides_again_on_the_agent_it_execs() {
        let not_recorded = crate::agent_resume::AgentLaunchOptions::Unavailable(
            crate::agent_resume::LaunchOptionsUnavailable::NotRecorded,
        );
        let fallback_seed = |terminal: &mut TerminalState| {
            terminal.seed_agent_launch(AgentLaunchSeed {
                agent: Agent::Claude,
                options: not_recorded.clone(),
                resumed_without_options: true,
                launch: SeededLaunch::Started {
                    session: crate::agent_resume::AgentSessionRef::id("placeholder-session"),
                },
            });
        };

        // The agent shows options of its own: the user started it by hand.
        let mut terminal = test_terminal();
        fallback_seed(&mut terminal);
        observe_launcher_then_agent(&mut terminal, None);
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(
                ["--model", "placeholder", "--permission-mode", "plan"]
                    .map(String::from)
                    .to_vec()
            )
        );
        assert!(!terminal.resumed_without_launch_options());

        // The agent resumes the seeded conversation: the restored process.
        let mut terminal = test_terminal();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("placeholder-session").unwrap(),
        });
        fallback_seed(&mut terminal);
        observe_launcher_then_agent(&mut terminal, Some("placeholder-session"));
        assert_eq!(terminal.agent_launch().unwrap().options, not_recorded);
        assert!(terminal.resumed_without_launch_options());
    }

    #[test]
    fn unreadable_launch_record_takes_a_later_readable_observation_of_the_same_process() {
        let unreadable = || {
            crate::agent_resume::AgentLaunchOptions::Unavailable(
                crate::agent_resume::LaunchOptionsUnavailable::CommandLineUnreadable,
            )
        };
        let mut terminal = test_terminal();
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(30),
            unreadable(),
            None,
        );
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(30),
            unreadable(),
            None,
        );
        assert_eq!(terminal.agent_launch().unwrap().options, unreadable());
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(30),
            known_options(&["--placeholder"]),
            None,
        );
        assert_eq!(
            terminal.launch_options_for_session("codex"),
            Some(vec!["--placeholder".to_string()])
        );
        // A readable record is never downgraded by a later unreadable read.
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(30),
            unreadable(),
            None,
        );
        assert_eq!(
            terminal.launch_options_for_session("codex"),
            Some(vec!["--placeholder".to_string()])
        );

        // A fallback seed bound through an unreadable read keeps its mark.
        let mut terminal = test_terminal();
        let not_recorded = crate::agent_resume::AgentLaunchOptions::Unavailable(
            crate::agent_resume::LaunchOptionsUnavailable::NotRecorded,
        );
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: not_recorded.clone(),
            resumed_without_options: true,
            launch: SeededLaunch::Started { session: None },
        });
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(40),
            unreadable(),
            None,
        );
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(40),
            known_options(&[]),
            None,
        );
        assert_eq!(terminal.agent_launch().unwrap().options, not_recorded);
        assert!(terminal.resumed_without_launch_options());
    }

    #[test]
    fn seeded_agent_launch_binds_to_the_first_observed_process() {
        let mut terminal = test_terminal();
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Codex,
            options: known_options(&["--seeded"]),
            resumed_without_options: false,
            launch: SeededLaunch::Started { session: None },
        });
        let seeded_revision = terminal.agent_launch_revision();

        // Where the command line cannot be read, the seeded options stay.
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(20),
            crate::agent_resume::AgentLaunchOptions::Unavailable(
                crate::agent_resume::LaunchOptionsUnavailable::CommandLineUnreadable,
            ),
            None,
        );
        let launch = terminal.agent_launch().unwrap();
        assert_eq!(launch.process(), Some(ProcessInstance::unknown_start(20)));
        assert_eq!(launch.options, known_options(&["--seeded"]));
        assert_eq!(terminal.agent_launch_revision(), seeded_revision);

        // Once bound, a later process does not inherit them.
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(21),
            crate::agent_resume::AgentLaunchOptions::Unavailable(
                crate::agent_resume::LaunchOptionsUnavailable::CommandLineUnreadable,
            ),
            None,
        );
        assert_eq!(terminal.launch_options_for_session("codex"), None);

        // A readable command line replaces seeded options.
        let mut terminal = test_terminal();
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Codex,
            options: known_options(&["--seeded"]),
            resumed_without_options: false,
            launch: SeededLaunch::Started { session: None },
        });
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(30),
            known_options(&["--observed"]),
            None,
        );
        assert_eq!(
            terminal.launch_options_for_session("codex"),
            Some(vec!["--observed".to_string()])
        );
    }

    #[test]
    fn seeded_agent_launch_survives_the_bare_resume_process() {
        // A fallback restore seeds unknown options; the bare resume process
        // Herdr typed must not turn them into "launched with no options".
        let mut terminal = test_terminal();
        let not_recorded = crate::agent_resume::AgentLaunchOptions::Unavailable(
            crate::agent_resume::LaunchOptionsUnavailable::NotRecorded,
        );
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: not_recorded.clone(),
            resumed_without_options: true,
            launch: SeededLaunch::Started { session: None },
        });
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(40),
            known_options(&[]),
            None,
        );
        let launch = terminal.agent_launch().unwrap();
        assert_eq!(launch.process(), Some(ProcessInstance::unknown_start(40)));
        assert_eq!(launch.options, not_recorded);
        assert_eq!(terminal.launch_options_for_session("claude"), None);

        // Saved options that did not fit the restore command stay saved.
        let mut terminal = test_terminal();
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Codex,
            options: known_options(&["--placeholder-a", "--placeholder-b"]),
            resumed_without_options: true,
            launch: SeededLaunch::Started { session: None },
        });
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(41),
            known_options(&[]),
            None,
        );
        assert_eq!(
            terminal.launch_options_for_session("codex"),
            Some(vec![
                "--placeholder-a".to_string(),
                "--placeholder-b".to_string()
            ])
        );

        // Any options the process does show replace the seed.
        let mut terminal = test_terminal();
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: not_recorded,
            resumed_without_options: true,
            launch: SeededLaunch::Started { session: None },
        });
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(42),
            known_options(&["--other"]),
            None,
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--other".to_string()])
        );
    }

    #[test]
    fn kept_or_started_agent_launch_takes_an_empty_observation() {
        // A seed from a restore that kept its options, or from `herdr agent
        // start`, may outlive its process; a bare agent typed afterwards must
        // not inherit its options.
        let mut terminal = test_terminal();
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: known_options(&["--placeholder-a", "--placeholder-b"]),
            resumed_without_options: false,
            launch: SeededLaunch::Started { session: None },
        });
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(50),
            known_options(&[]),
            None,
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(Vec::new())
        );
    }

    #[test]
    fn resumed_without_launch_options_lasts_while_that_agent_runs() {
        let not_recorded = || {
            crate::agent_resume::AgentLaunchOptions::Unavailable(
                crate::agent_resume::LaunchOptionsUnavailable::NotRecorded,
            )
        };
        let fallback = || {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
            terminal.seed_agent_launch(AgentLaunchSeed {
                agent: Agent::Claude,
                options: not_recorded(),
                resumed_without_options: true,
                launch: SeededLaunch::Started { session: None },
            });
            terminal
        };

        // A fallback seed sets the mark, a save records it, and binding to the
        // bare resume process keeps it.
        let mut terminal = test_terminal();
        let revision = terminal.agent_launch_revision();
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: not_recorded(),
            resumed_without_options: true,
            launch: SeededLaunch::Started { session: None },
        });
        assert_ne!(terminal.agent_launch_revision(), revision);
        assert!(terminal.resumed_without_launch_options());
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(40),
            known_options(&[]),
            None,
        );
        assert_eq!(
            terminal.agent_launch().unwrap().process(),
            Some(ProcessInstance::unknown_start(40))
        );
        assert!(terminal.resumed_without_launch_options());
        assert!(terminal.resumed_without_launch_options_for_session("claude"));
        assert!(!terminal.resumed_without_launch_options_for_session("codex"));

        // Another process clears it.
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(41),
            known_options(&[]),
            None,
        );
        assert!(!terminal.resumed_without_launch_options());
        assert!(!terminal.resumed_without_launch_options_for_session("claude"));

        // So does the agent exiting.
        let mut terminal = fallback();
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(40),
            known_options(&[]),
            None,
        );
        terminal.set_detected_state_with_visible_blocker(
            Some(Agent::Claude),
            AgentState::Idle,
            false,
            false,
            true,
        );
        assert!(!terminal.resumed_without_launch_options());

        // And a change of agent.
        let mut terminal = fallback();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        assert!(!terminal.resumed_without_launch_options());
        let mut terminal = fallback();
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(40),
            known_options(&[]),
            None,
        );
        assert!(!terminal.resumed_without_launch_options());

        // A kept restore or `herdr agent start` seed never sets it.
        let mut terminal = test_terminal();
        terminal.seed_agent_launch(AgentLaunchSeed {
            agent: Agent::Claude,
            options: known_options(&["--placeholder"]),
            resumed_without_options: false,
            launch: SeededLaunch::Started { session: None },
        });
        assert!(!terminal.resumed_without_launch_options());
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(40),
            known_options(&[]),
            None,
        );
        assert!(!terminal.resumed_without_launch_options());
        assert!(!terminal.resumed_without_launch_options_for_session("claude"));
    }

    #[test]
    fn agent_launch_is_cleared_when_the_agent_exits_or_changes() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(10),
            known_options(&["--bypass"]),
            None,
        );
        terminal.set_detected_state_with_visible_blocker(
            Some(Agent::Claude),
            AgentState::Idle,
            false,
            false,
            true,
        );
        assert!(terminal.agent_launch().is_none());

        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(10),
            known_options(&["--bypass"]),
            None,
        );
        let revision = terminal.agent_launch_revision();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        assert!(terminal.agent_launch().is_none());
        assert_ne!(terminal.agent_launch_revision(), revision);
    }

    #[test]
    fn managed_agent_readiness_tracks_detection_state() {
        let mut terminal = test_terminal();
        let now = Instant::now();
        terminal.begin_managed_agent(
            "reviewer".into(),
            Agent::Pi,
            now,
            Duration::from_millis(100),
            Duration::from_secs(1),
        );
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Unknown);

        assert!(terminal.managed_agent_launch_pending());
        assert!(!terminal.managed_agent_interactive_ready());
        assert!(terminal.reconcile_managed_agent_at(now + Duration::from_millis(100), false));
        assert!(terminal.managed_agent_launch_pending());

        terminal.set_detected_state(Some(Agent::Pi), AgentState::Working);
        assert!(!terminal.reconcile_managed_agent_at(now + Duration::from_millis(101), false));

        terminal.set_detected_state(Some(Agent::Pi), AgentState::Blocked);
        assert!(terminal.reconcile_managed_agent_at(now + Duration::from_millis(102), false));
        assert!(terminal.managed_agent_launch_pending());
        assert!(!terminal.managed_agent_interactive_ready());
        assert_eq!(terminal.next_managed_agent_deadline(), None);
        assert_eq!(terminal.agent_name.as_deref(), Some("reviewer"));
        assert!(!terminal.reconcile_managed_agent_at(now + Duration::from_secs(2), false));
        assert_eq!(terminal.agent_name.as_deref(), Some("reviewer"));

        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        assert!(terminal.reconcile_managed_agent_at(now + Duration::from_secs(2), false));
        assert!(!terminal.managed_agent_launch_pending());
        assert!(terminal.managed_agent_interactive_ready());

        terminal.set_detected_state(None, AgentState::Unknown);
        assert!(terminal.managed_agent_interactive_ready());
        assert!(!terminal.reconcile_managed_agent_at(now + Duration::from_secs(2), false));
        assert_eq!(terminal.agent_name.as_deref(), Some("reviewer"));
        assert!(terminal.reconcile_managed_agent_at(now + Duration::from_secs(2), true));
        assert_eq!(terminal.agent_name, None);
    }

    #[test]
    fn codex_managed_readiness_requires_current_prompt_without_idle() {
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.begin_managed_agent(
            "reviewer".into(),
            Agent::Codex,
            now,
            Duration::from_millis(100),
            Duration::from_secs(1),
        );
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Unknown);
        terminal.observe_codex_prompt_ready(true);
        terminal.reconcile_managed_agent_at(now, false);
        assert!(!terminal.managed_agent_interactive_ready());
        terminal.observe_codex_prompt_ready(false);
        assert!(terminal.reconcile_managed_agent_at(now + Duration::from_millis(100), false));
        assert!(!terminal.managed_agent_interactive_ready());

        terminal.observe_codex_prompt_ready(true);
        assert!(terminal.reconcile_managed_agent_at(now + Duration::from_millis(101), false));
        assert!(terminal.managed_agent_interactive_ready());
        assert_eq!(terminal.state, AgentState::Unknown);
        terminal.observe_codex_prompt_ready(false);
        assert!(terminal.managed_agent_interactive_ready());

        terminal.begin_managed_agent(
            "next".into(),
            Agent::Codex,
            now,
            Duration::ZERO,
            Duration::from_secs(1),
        );
        terminal.reconcile_managed_agent_at(now, false);
        assert!(!terminal.managed_agent_interactive_ready());

        terminal.set_detected_state(Some(Agent::Codex), AgentState::Blocked);
        assert!(terminal.reconcile_managed_agent_at(now, false));
        terminal.observe_codex_prompt_ready(false);
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Unknown);
        assert!(!terminal.reconcile_managed_agent_at(now, false));
        terminal.observe_codex_prompt_ready(true);
        assert!(terminal.reconcile_managed_agent_at(now, false));
        assert!(terminal.managed_agent_interactive_ready());
    }

    #[test]
    fn managed_agent_mismatch_and_timeout_release_name() {
        let now = Instant::now();
        let mut mismatch = test_terminal();
        mismatch.begin_managed_agent(
            "reviewer".into(),
            Agent::Pi,
            now,
            Duration::ZERO,
            Duration::from_secs(1),
        );
        mismatch.set_detected_state(Some(Agent::Codex), AgentState::Idle);
        assert!(mismatch.reconcile_managed_agent_at(now, false));
        assert_eq!(mismatch.agent_name, None);
        assert_eq!(mismatch.managed_agent_kind(), None);

        let mut timed_out = test_terminal();
        timed_out.begin_managed_agent(
            "reviewer".into(),
            Agent::Pi,
            now,
            Duration::from_millis(10),
            Duration::from_millis(20),
        );
        timed_out.set_managed_agent_launch_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:codex".into(),
            agent: "codex".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("codex-session").unwrap(),
        });
        assert!(timed_out.reconcile_managed_agent_at(now + Duration::from_millis(20), false));
        assert_eq!(timed_out.agent_name, None);
        assert_eq!(timed_out.managed_agent_kind(), None);
        assert!(timed_out.persisted_agent_session.is_none());
    }

    #[test]
    fn stabilization_uses_raw_policy_state() {
        let detection = AgentDetection {
            state: AgentState::Idle,
            skip_state_update: false,
            visible_idle: false,
            visible_blocker: false,
            visible_working: false,
        };

        assert_eq!(stabilize_agent_detection(detection), AgentState::Idle);
    }

    #[test]
    fn hook_authority_overrides_fallback_for_same_agent() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(test_session_path("root.jsonl")).unwrap(),
        );
        terminal.set_hook_authority(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            None,
        );

        assert_eq!(terminal.detected_agent, Some(Agent::Pi));
        assert_eq!(terminal.fallback_state, AgentState::Idle);
        assert_eq!(terminal.effective_agent_label(), Some("pi"));
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn hook_authority_can_override_with_unknown_agent_label() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal.set_hook_authority(
            "herdr:custom".into(),
            "custom-agent".into(),
            AgentState::Working,
            None,
            None,
        );

        assert_eq!(terminal.detected_agent, Some(Agent::Pi));
        assert_eq!(terminal.effective_agent_label(), Some("custom-agent"));
        assert_eq!(terminal.effective_known_agent(), None);
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn omp_hook_authority_overrides_detected_fallback() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Omp), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Omp,
            "herdr:omp",
            "omp",
            crate::agent_resume::AgentSessionRef::id("omp-root").unwrap(),
        );
        terminal.set_hook_authority(
            "herdr:omp".into(),
            "omp".into(),
            AgentState::Working,
            None,
            None,
        );

        assert_eq!(terminal.detected_agent, Some(Agent::Omp));
        assert_eq!(terminal.effective_agent_label(), Some("omp"));
        assert_eq!(terminal.effective_known_agent(), Some(Agent::Omp));
        assert_eq!(terminal.state, AgentState::Working);

        let change = terminal.set_detected_state_with_visible_blocker(
            Some(Agent::Omp),
            AgentState::Blocked,
            true,
            false,
            false,
        );

        assert_eq!(terminal.fallback_state, AgentState::Idle);
        assert_eq!(terminal.state, AgentState::Working);
        assert!(change.is_none());
    }

    #[test]
    fn session_only_report_does_not_create_hook_authority() {
        for (agent, source, label, session_id) in [
            (Agent::Codex, "herdr:codex", "codex", "codex-session"),
            (Agent::Devin, "herdr:devin", "devin", "devin-session"),
        ] {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(agent), AgentState::Idle);

            let mutation = terminal.set_agent_session_ref(
                source.into(),
                label.into(),
                crate::agent_resume::AgentSessionRef::id(session_id),
                Some(1),
            );

            assert!(mutation.is_some());
            assert!(terminal.hook_authority.is_none());
            assert!(!terminal.full_lifecycle_hook_authority_active());
            assert_eq!(terminal.state, AgentState::Idle);

            terminal.set_detected_state_with_screen_signals_at(
                Some(agent),
                AgentState::Working,
                false,
                false,
                false,
                false,
                Instant::now(),
            );

            assert_eq!(terminal.state, AgentState::Working);
        }
    }

    #[test]
    fn startup_session_claim_activates_full_lifecycle_integrations() {
        for (agent, source, label) in [
            (Agent::Kimi, "herdr:kimi", "kimi"),
            (Agent::Kilo, "herdr:kilo", "kilo"),
        ] {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(agent), AgentState::Idle);
            let session_ref = crate::agent_resume::AgentSessionRef::id(format!("{label}-root"));

            let session = terminal.set_agent_session_ref_for_session_start(
                source.into(),
                label.into(),
                session_ref.clone(),
                Some(10),
                Some("startup".into()),
            );
            let working = terminal.set_hook_authority_with_session_ref(
                source.into(),
                label.into(),
                AgentState::Working,
                None,
                session_ref,
                Some(11),
            );

            assert!(
                session.is_some(),
                "{label} should accept its startup session"
            );
            assert!(
                working.is_some(),
                "{label} should accept state after startup"
            );
            assert_eq!(terminal.state, AgentState::Working);
        }
    }

    #[test]
    fn session_identity_claims_leave_state_to_detection() {
        for (source, label, agent, start_source, replacement_source) in [
            (
                "herdr:hermes",
                "hermes",
                Agent::Hermes,
                Some("startup"),
                Some("resume"),
            ),
            (
                "herdr:antigravity_cli",
                "agy",
                Agent::Antigravity,
                None,
                None,
            ),
        ] {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(agent), AgentState::Idle);
            let first_ref =
                crate::agent_resume::AgentSessionRef::id(format!("{label}-root")).unwrap();
            let first = terminal.set_agent_session_ref_for_session_start(
                source.into(),
                label.into(),
                Some(first_ref.clone()),
                Some(10),
                start_source.map(str::to_string),
            );

            assert!(first.is_some(), "{label} should accept its session");
            assert!(terminal.hook_authority.is_none());
            assert_eq!(terminal.state, AgentState::Idle);
            assert_eq!(
                terminal
                    .persisted_agent_session
                    .as_ref()
                    .map(|session| &session.session_ref),
                Some(&first_ref)
            );

            terminal.set_detected_state(Some(agent), AgentState::Working);
            let replacement_ref =
                crate::agent_resume::AgentSessionRef::id(format!("{label}-replacement")).unwrap();
            let replacement = terminal.set_agent_session_ref_for_session_start(
                source.into(),
                label.into(),
                Some(replacement_ref.clone()),
                Some(11),
                start_source.map(str::to_string),
            );

            assert!(
                replacement.is_some_and(|mutation| mutation.session_ref_changed),
                "{label} should replace its detected session"
            );
            assert!(terminal.hook_authority.is_none());
            assert_eq!(terminal.state, AgentState::Working);
            assert_eq!(
                terminal
                    .persisted_agent_session
                    .as_ref()
                    .map(|session| &session.session_ref),
                Some(&replacement_ref)
            );

            let legacy_state = terminal.set_hook_authority_with_session_ref(
                source.into(),
                label.into(),
                AgentState::Blocked,
                None,
                Some(replacement_ref.clone()),
                Some(12),
            );
            assert!(legacy_state.is_none());
            assert!(terminal.hook_authority.is_none());
            assert_eq!(terminal.state, AgentState::Working);

            terminal.set_detected_state(None, AgentState::Unknown);
            let background_ref =
                crate::agent_resume::AgentSessionRef::id(format!("{label}-background")).unwrap();
            let background_replacement = terminal.set_agent_session_ref_for_session_start(
                source.into(),
                label.into(),
                Some(background_ref.clone()),
                Some(13),
                replacement_source.map(str::to_string),
            );
            assert!(
                background_replacement.is_none(),
                "{label} should reject a background replacement"
            );
            assert_eq!(
                terminal
                    .persisted_agent_session
                    .as_ref()
                    .map(|session| &session.session_ref),
                Some(&replacement_ref)
            );

            terminal.set_detected_state(Some(agent), AgentState::Idle);
            let retried_replacement = terminal.set_agent_session_ref_for_session_start(
                source.into(),
                label.into(),
                Some(background_ref.clone()),
                Some(14),
                replacement_source.map(str::to_string),
            );
            assert!(
                retried_replacement.is_some_and(|mutation| mutation.session_ref_changed),
                "{label} should replace the session once detected"
            );
            assert_eq!(
                terminal
                    .persisted_agent_session
                    .as_ref()
                    .map(|session| &session.session_ref),
                Some(&background_ref)
            );
        }
    }

    #[test]
    fn pi_session_replacement_reports_reanchor_full_lifecycle_authority() {
        for reason in ["new", "resume", "fork"] {
            let mut terminal = test_terminal();
            let old_session = test_session_path(&format!("pi-{reason}-old.jsonl"));
            let new_session = test_session_path(&format!("pi-{reason}-new.jsonl"));
            terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
            terminal.set_hook_authority_with_session_ref(
                "herdr:pi".into(),
                "pi".into(),
                AgentState::Idle,
                None,
                crate::agent_resume::AgentSessionRef::path(old_session),
                Some(10),
            );

            let session_report = terminal.set_agent_session_ref_for_session_start(
                "herdr:pi".into(),
                "pi".into(),
                crate::agent_resume::AgentSessionRef::path(new_session.clone()),
                Some(11),
                Some(reason.into()),
            );

            assert!(
                session_report.is_some(),
                "{reason} should replace the previous Pi session"
            );
            assert!(terminal.hook_authority.is_none());

            let working = terminal.set_hook_authority_with_session_ref(
                "herdr:pi".into(),
                "pi".into(),
                AgentState::Working,
                None,
                crate::agent_resume::AgentSessionRef::path(new_session.clone()),
                Some(12),
            );

            assert!(
                working.is_some(),
                "{reason} should accept working for the replacement session"
            );
            assert_eq!(terminal.state, AgentState::Working);
            assert_eq!(
                terminal.hook_authority.as_ref().unwrap().session_ref,
                crate::agent_resume::AgentSessionRef::path(new_session)
            );
        }
    }

    #[test]
    fn pi_resume_reactivates_a_previously_stale_session() {
        let mut terminal = test_terminal();
        let session_a = test_session_path("pi-session-a.jsonl");
        let session_b = test_session_path("pi-session-b.jsonl");
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Idle,
            None,
            crate::agent_resume::AgentSessionRef::path(session_a.clone()),
            Some(10),
        );

        terminal.set_agent_session_ref_for_session_start(
            "herdr:pi".into(),
            "pi".into(),
            crate::agent_resume::AgentSessionRef::path(session_b.clone()),
            Some(11),
            Some("new".into()),
        );
        terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Idle,
            None,
            crate::agent_resume::AgentSessionRef::path(session_b.clone()),
            Some(12),
        );

        let resumed = terminal.set_agent_session_ref_for_session_start(
            "herdr:pi".into(),
            "pi".into(),
            crate::agent_resume::AgentSessionRef::path(session_a.clone()),
            Some(13),
            Some("resume".into()),
        );
        let working = terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(session_a.clone()),
            Some(14),
        );

        assert!(resumed.is_some());
        assert!(working.is_some());
        assert_eq!(terminal.state, AgentState::Working);
        assert_eq!(
            terminal.hook_authority.as_ref().unwrap().session_ref,
            crate::agent_resume::AgentSessionRef::path(session_a)
        );

        let late_session_b = terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Idle,
            None,
            crate::agent_resume::AgentSessionRef::path(session_b),
            Some(15),
        );
        assert!(late_session_b.is_none());
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn pi_startup_adopts_persisted_session_without_live_authority() {
        let mut terminal = test_terminal();
        let old_session = test_session_path("pi-startup-old.jsonl");
        let new_session = test_session_path("pi-startup-new.jsonl");
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            session_ref: crate::agent_resume::AgentSessionRef::path(old_session)
                .expect("test session path should be valid"),
        });

        let startup = terminal.set_agent_session_ref_for_session_start(
            "herdr:pi".into(),
            "pi".into(),
            crate::agent_resume::AgentSessionRef::path(new_session.clone()),
            Some(11),
            Some("startup".into()),
        );

        assert!(startup.is_some());
        assert_eq!(
            terminal.current_session_identity_for_persistence(),
            Some((
                "herdr:pi".into(),
                "pi".into(),
                crate::agent_resume::AgentSessionRefKind::Path,
                new_session,
            ))
        );
    }

    #[test]
    fn pi_non_replacement_reports_preserve_full_lifecycle_authority() {
        for reason in [None, Some("reload"), Some("startup")] {
            let mut terminal = test_terminal();
            let old_session = test_session_path("pi-current.jsonl");
            let new_session = test_session_path("pi-unexpected.jsonl");
            terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
            anchor_full_lifecycle_session(
                &mut terminal,
                Agent::Pi,
                "herdr:pi",
                "pi",
                crate::agent_resume::AgentSessionRef::path(old_session.clone()).unwrap(),
            );
            terminal.set_hook_authority_with_session_ref(
                "herdr:pi".into(),
                "pi".into(),
                AgentState::Idle,
                None,
                crate::agent_resume::AgentSessionRef::path(old_session.clone()),
                Some(10),
            );

            let session_report = terminal.set_agent_session_ref_for_session_start(
                "herdr:pi".into(),
                "pi".into(),
                crate::agent_resume::AgentSessionRef::path(new_session.clone()),
                Some(11),
                reason.map(str::to_string),
            );
            let working = terminal.set_hook_authority_with_session_ref(
                "herdr:pi".into(),
                "pi".into(),
                AgentState::Working,
                None,
                crate::agent_resume::AgentSessionRef::path(new_session),
                Some(12),
            );

            assert!(session_report.is_none());
            assert!(working.is_none());
            assert_eq!(terminal.state, AgentState::Idle);
            assert_eq!(
                terminal.hook_authority.as_ref().unwrap().session_ref,
                crate::agent_resume::AgentSessionRef::path(old_session),
                "{reason:?} must not replace the current Pi session"
            );
        }
    }

    #[test]
    fn omp_resume_session_report_reanchors_full_lifecycle_authority() {
        let mut terminal = test_terminal();
        let old_session = test_session_path("omp-old.jsonl");
        let new_session = test_session_path("omp-new.jsonl");
        terminal.set_detected_state(Some(Agent::Omp), AgentState::Idle);
        terminal.set_hook_authority_with_session_ref(
            "herdr:omp".into(),
            "omp".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(old_session.clone()),
            Some(10),
        );

        let session_report = terminal.set_agent_session_ref_for_session_start(
            "herdr:omp".into(),
            "omp".into(),
            crate::agent_resume::AgentSessionRef::path(new_session.clone()),
            Some(11),
            Some("resume".into()),
        );

        assert!(session_report.is_some());
        assert!(terminal.hook_authority.is_none());
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .unwrap()
                .session_ref,
            crate::agent_resume::AgentSessionRef::path(new_session.clone()).unwrap()
        );

        let blocked = terminal.set_hook_authority_with_session_ref(
            "herdr:omp".into(),
            "omp".into(),
            AgentState::Blocked,
            Some("waiting".into()),
            crate::agent_resume::AgentSessionRef::path(new_session.clone()),
            Some(12),
        );

        assert!(blocked.is_some());
        assert_eq!(terminal.state, AgentState::Blocked);
        assert_eq!(
            terminal.hook_authority.as_ref().unwrap().session_ref,
            crate::agent_resume::AgentSessionRef::path(new_session)
        );

        let stale = terminal.set_hook_authority_with_session_ref(
            "herdr:omp".into(),
            "omp".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(old_session),
            Some(13),
        );

        assert!(stale.is_none());
        assert_eq!(terminal.state, AgentState::Blocked);
    }

    #[test]
    fn late_full_lifecycle_hook_with_same_session_after_process_exit_does_not_reacquire_authority()
    {
        let now = Instant::now();
        let mut terminal = test_terminal();
        let session_path = test_session_path("pi.jsonl");
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Working);
        terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(session_path.clone()),
            Some(20),
        );

        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            true,
            now + Duration::from_millis(1),
        );
        let late = terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(session_path),
            Some(21),
        );

        assert!(late.is_none());
        assert!(terminal.hook_authority.is_none());
        assert_eq!(terminal.state, AgentState::Idle);
    }

    #[test]
    fn live_full_lifecycle_hook_rejects_different_session_ref_for_same_source() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(test_session_path("one.jsonl")).unwrap(),
        );
        terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(test_session_path("one.jsonl")),
            Some(20),
        );

        let mutation = terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Idle,
            None,
            crate::agent_resume::AgentSessionRef::path(test_session_path("two.jsonl")),
            Some(21),
        );

        assert!(mutation.is_none());
        assert_eq!(terminal.state, AgentState::Working);
        assert_eq!(
            terminal
                .hook_authority
                .as_ref()
                .and_then(|authority| authority.session_ref.as_ref())
                .map(|session_ref| session_ref.value.as_str()),
            Some(test_session_path("one.jsonl").as_str())
        );
    }

    #[test]
    fn fresh_detected_process_keeps_old_session_suppressed_after_process_exit() {
        let mut terminal = test_terminal();
        let old_session = test_session_path("old-process-exit.jsonl");
        let new_session = test_session_path("new-process-exit.jsonl");
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(old_session.clone()),
            Some(1000),
        );
        let process_exit_seen_at = Instant::now() + Duration::from_secs(1);
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            true,
            process_exit_seen_at,
        );

        let fresh_process_seen_at = process_exit_seen_at + Duration::from_millis(1);
        terminal.set_detected_state_with_screen_signals_at(
            None,
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            fresh_process_seen_at,
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            fresh_process_seen_at + Duration::from_millis(1),
        );

        let late_old = terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(old_session),
            Some(500),
        );
        let fresh_new = terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(new_session.clone()),
            Some(501),
        );

        assert!(late_old.is_none());
        assert!(fresh_new.is_none());
        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:pi".into(),
                "pi".into(),
                crate::agent_resume::AgentSessionRef::path(new_session),
                Some(400),
                Some("startup".into()),
            )
            .expect("fresh session should activate the buffered report");
        assert!(terminal.hook_authority.is_some());
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn rapid_restart_replays_reports_that_arrive_before_process_evidence() {
        let mut terminal = test_terminal();
        let session_path = test_session_path("reports-before-process-evidence.jsonl");
        let now = Instant::now();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(session_path.clone()),
            Some(1000),
            now,
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            true,
            now + Duration::from_millis(1),
        );

        let lower_sequence = terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Idle,
            None,
            crate::agent_resume::AgentSessionRef::path(session_path.clone()),
            Some(1001),
            now + Duration::from_millis(2),
        );
        let missing_sequence = terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Idle,
            None,
            crate::agent_resume::AgentSessionRef::path(session_path.clone()),
            None,
            now + Duration::from_millis(3),
        );
        let buffered_working = terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(session_path.clone()),
            Some(2001),
            now + Duration::from_millis(4),
        );
        let startup = terminal.set_agent_session_ref_for_session_start(
            "herdr:pi".into(),
            "pi".into(),
            crate::agent_resume::AgentSessionRef::path(session_path),
            Some(2000),
            Some("startup".into()),
        );
        assert!(startup.is_none());
        assert!(lower_sequence.is_none());
        assert!(missing_sequence.is_none());
        assert!(buffered_working.is_none());
        assert!(!terminal.full_lifecycle_hook_authority_active());

        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            false,
            now + Duration::from_millis(5),
        );

        assert!(terminal.full_lifecycle_hook_authority_active());
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn process_exit_discards_unclaimed_buffered_state_from_that_generation() {
        let mut terminal = test_terminal();
        let old_session = test_session_path("buffered-exit-old.jsonl");
        let shared_session = test_session_path("buffered-exit-shared.jsonl");
        let now = Instant::now();
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(old_session.clone()).unwrap(),
        );
        terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(old_session),
            Some(1000),
            now,
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            true,
            now + Duration::from_millis(1),
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            false,
            now + Duration::from_millis(2),
        );
        terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(shared_session.clone()),
            Some(500),
            now + Duration::from_millis(3),
        );

        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            true,
            now + Duration::from_millis(4),
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            false,
            now + Duration::from_millis(5),
        );
        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:pi".into(),
                "pi".into(),
                crate::agent_resume::AgentSessionRef::path(shared_session),
                Some(100),
                Some("startup".into()),
            )
            .expect("new generation session claim");

        assert!(terminal.hook_authority.is_none());
        assert_eq!(terminal.state, AgentState::Idle);
    }

    #[test]
    fn queued_fresh_process_evidence_uses_process_exit_observation_time() {
        let mut terminal = test_terminal();
        let session_path = test_session_path("queued-after-process-exit.jsonl");
        let process_exit_at = Instant::now() - Duration::from_secs(1);
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(session_path.clone()),
            Some(1000),
            process_exit_at - Duration::from_millis(1),
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            true,
            process_exit_at,
        );

        terminal.set_detected_state_with_screen_signals_at(
            None,
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            process_exit_at + Duration::from_millis(1),
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            false,
            process_exit_at + Duration::from_millis(2),
        );
        let startup = terminal.set_agent_session_ref_for_session_start(
            "herdr:pi".into(),
            "pi".into(),
            crate::agent_resume::AgentSessionRef::path(session_path),
            Some(2000),
            Some("startup".into()),
        );

        assert!(startup.is_some());
    }

    #[test]
    fn different_session_after_process_exit_waits_for_fresh_process_evidence() {
        let mut terminal = test_terminal();
        let old_session = test_session_path("old-before-process-exit.jsonl");
        let new_session = test_session_path("new-after-process-exit.jsonl");
        let now = Instant::now();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(old_session),
            Some(1000),
            now,
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            true,
            now + Duration::from_millis(1),
        );

        let early_new = terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(new_session.clone()),
            Some(500),
            now + Duration::from_millis(2),
        );

        assert!(early_new.is_none());
        assert!(terminal.hook_authority.is_none());

        terminal.set_detected_state_with_screen_signals_at(
            None,
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            now + Duration::from_millis(3),
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            now + Duration::from_millis(4),
        );
        let fresh_new = terminal.set_agent_session_ref_for_session_start(
            "herdr:pi".into(),
            "pi".into(),
            crate::agent_resume::AgentSessionRef::path(new_session),
            Some(400),
            Some("startup".into()),
        );

        assert!(fresh_new.is_some());
        assert!(terminal.hook_authority.is_some());
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn missing_session_after_process_exit_waits_for_fresh_process_evidence() {
        let mut terminal = test_terminal();
        let old_session = test_session_path("old-before-nosession-process-exit.jsonl");
        let now = Instant::now();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(old_session),
            Some(1000),
            now,
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            true,
            false,
            true,
            now + Duration::from_millis(1),
        );

        let early_without_session = terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            None,
            Some(500),
            now + Duration::from_millis(2),
        );

        assert!(early_without_session.is_none());
        assert!(terminal.hook_authority.is_none());

        terminal.set_detected_state_with_screen_signals_at(
            None,
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            now + Duration::from_millis(3),
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            now + Duration::from_millis(4),
        );
        let fresh_without_session = terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            None,
            Some(500),
            now + Duration::from_millis(5),
        );
        assert!(fresh_without_session.is_none());
        assert!(terminal.hook_authority.is_none());

        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:pi".into(),
                "pi".into(),
                crate::agent_resume::AgentSessionRef::path(test_session_path(
                    "fresh-after-nosession-process-exit.jsonl",
                )),
                Some(600),
                Some("startup".into()),
            )
            .expect("fresh root session should claim the process generation");
        let child_update = terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            None,
            Some(601),
            now + Duration::from_millis(6),
        );

        assert!(child_update.is_some());
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn mastracode_session_start_replaces_current_root_session() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Mastracode), AgentState::Idle);
        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:mastracode".into(),
                "mastracode".into(),
                crate::agent_resume::AgentSessionRef::id("mastracode-old"),
                Some(20),
                Some("startup".into()),
            )
            .expect("initial root session");

        let replacement = terminal.set_agent_session_ref_for_session_start(
            "herdr:mastracode".into(),
            "mastracode".into(),
            crate::agent_resume::AgentSessionRef::id("mastracode-new"),
            Some(21),
            Some("startup".into()),
        );

        assert!(replacement.is_some_and(|mutation| mutation.session_ref_changed));
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("mastracode-new")
        );
    }

    #[test]
    fn omp_reacquires_full_lifecycle_hook_after_process_exit_with_fresh_process_and_session_ref() {
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Omp), AgentState::Idle);
        terminal.set_hook_authority_at(
            "herdr:omp".into(),
            "omp".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::id("omp-old"),
            Some(1000),
            now,
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Omp),
            AgentState::Idle,
            false,
            true,
            false,
            true,
            now + Duration::from_millis(1),
        );

        let stale = terminal.set_hook_authority_with_session_ref(
            "herdr:omp".into(),
            "omp".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::id("omp-old"),
            Some(500),
        );
        assert!(stale.is_none());
        assert!(terminal.hook_authority.is_none());

        terminal.set_detected_state_with_screen_signals_at(
            None,
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            now + Duration::from_millis(2),
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Omp),
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            now + Duration::from_millis(3),
        );
        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:omp".into(),
                "omp".into(),
                crate::agent_resume::AgentSessionRef::id("omp-new"),
                Some(400),
                Some("startup".into()),
            )
            .expect("fresh process and session should claim the pane");
        let fresh = terminal.set_hook_authority_with_session_ref(
            "herdr:omp".into(),
            "omp".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::id("omp-new"),
            Some(500),
        );

        assert!(fresh.is_some());
        assert!(terminal.hook_authority.is_some());
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn visible_blocker_overrides_non_blocked_hook_for_same_agent() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Idle);
        terminal.set_hook_authority(
            "herdr:codex".into(),
            "codex".into(),
            AgentState::Working,
            None,
            None,
        );

        let change = terminal.set_detected_state_with_visible_blocker(
            Some(Agent::Codex),
            AgentState::Blocked,
            true,
            false,
            false,
        );

        assert_eq!(terminal.fallback_state, AgentState::Blocked);
        assert_eq!(terminal.state, AgentState::Blocked);
        assert_eq!(change.unwrap().previous_state, AgentState::Working);
    }

    #[test]
    fn visible_blocker_does_not_override_full_lifecycle_hook_authority() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(test_session_path("root.jsonl")).unwrap(),
        );
        terminal.set_hook_authority(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            None,
        );

        let change = terminal.set_detected_state_with_visible_blocker(
            Some(Agent::Pi),
            AgentState::Blocked,
            true,
            false,
            false,
        );

        assert_eq!(terminal.fallback_state, AgentState::Idle);
        assert_eq!(terminal.state, AgentState::Working);
        assert!(change.is_none());
    }

    #[test]
    fn weak_blocked_fallback_does_not_override_hook_authority() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Idle);
        terminal.set_hook_authority(
            "herdr:codex".into(),
            "codex".into(),
            AgentState::Working,
            None,
            None,
        );

        let change = terminal.set_detected_state_with_visible_blocker(
            Some(Agent::Codex),
            AgentState::Blocked,
            false,
            false,
            false,
        );

        assert_eq!(terminal.fallback_state, AgentState::Blocked);
        assert_eq!(terminal.state, AgentState::Working);
        assert!(change.is_none());
    }

    #[test]
    fn hook_blocked_wins_over_visible_blocker() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Working);
        terminal.set_hook_authority(
            "herdr:codex".into(),
            "codex".into(),
            AgentState::Blocked,
            None,
            None,
        );

        terminal.set_detected_state_with_visible_blocker(
            Some(Agent::Codex),
            AgentState::Blocked,
            true,
            false,
            false,
        );

        assert_eq!(terminal.state, AgentState::Blocked);
        assert!(terminal.hook_authority.is_some());
    }

    #[test]
    fn visible_blocker_does_not_override_different_agent_hook() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(None, AgentState::Unknown);
        terminal.set_hook_authority(
            "custom:agent".into(),
            "custom-agent".into(),
            AgentState::Working,
            None,
            None,
        );

        terminal.set_detected_state_with_visible_blocker(
            Some(Agent::Codex),
            AgentState::Blocked,
            true,
            false,
            false,
        );

        assert_eq!(terminal.effective_agent_label(), Some("custom-agent"));
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn fallback_idle_does_not_override_hook_working() {
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Working);
        terminal.set_hook_authority_at(
            "herdr:claude".into(),
            "claude".into(),
            AgentState::Working,
            None,
            None,
            None,
            now,
        );

        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Claude),
            AgentState::Idle,
            false,
            true,
            false,
            false,
            now + Duration::from_secs(10),
        );

        assert_eq!(terminal.fallback_state, AgentState::Idle);
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn fallback_idle_does_not_override_full_lifecycle_hook_working() {
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Working);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::OpenCode,
            "herdr:opencode",
            "opencode",
            crate::agent_resume::AgentSessionRef::id("opencode-root").unwrap(),
        );
        terminal.set_hook_authority_at(
            "herdr:opencode".into(),
            "opencode".into(),
            AgentState::Working,
            None,
            None,
            None,
            now,
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::OpenCode),
            AgentState::Idle,
            false,
            true,
            false,
            false,
            now + Duration::from_secs(10),
        );

        assert_eq!(terminal.fallback_state, AgentState::Working);
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn visible_working_does_not_override_hook_idle_for_same_agent() {
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        terminal.set_hook_authority_at(
            "herdr:claude".into(),
            "claude".into(),
            AgentState::Idle,
            None,
            None,
            None,
            now,
        );

        let change = terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Claude),
            AgentState::Working,
            false,
            false,
            true,
            false,
            now + Duration::from_millis(1),
        );

        assert_eq!(terminal.fallback_state, AgentState::Working);
        assert_eq!(terminal.state, AgentState::Idle);
        assert!(change.effective_state_change.is_none());
    }

    #[test]
    fn visible_working_does_not_override_full_lifecycle_hook_idle() {
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Kimi), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Kimi,
            "herdr:kimi",
            "kimi",
            crate::agent_resume::AgentSessionRef::id("kimi-root").unwrap(),
        );
        terminal.set_hook_authority_at(
            "herdr:kimi".into(),
            "kimi".into(),
            AgentState::Idle,
            None,
            None,
            None,
            now,
        );

        let change = terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Kimi),
            AgentState::Working,
            false,
            false,
            true,
            false,
            now + Duration::from_millis(1),
        );

        assert_eq!(terminal.fallback_state, AgentState::Idle);
        assert_eq!(terminal.state, AgentState::Idle);
        assert!(change.effective_state_change.is_none());
    }

    #[test]
    fn detected_working_fallback_is_ignored_under_full_lifecycle_hook_authority() {
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Kilo), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Kilo,
            "herdr:kilo",
            "kilo",
            crate::agent_resume::AgentSessionRef::id("kilo-root").unwrap(),
        );
        terminal.set_hook_authority_at(
            "herdr:kilo".into(),
            "kilo".into(),
            AgentState::Idle,
            None,
            None,
            None,
            now,
        );

        let change = terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Kilo),
            AgentState::Working,
            false,
            false,
            false,
            false,
            now + Duration::from_millis(1),
        );

        assert_eq!(terminal.fallback_state, AgentState::Idle);
        assert_eq!(terminal.state, AgentState::Idle);
        assert!(change.effective_state_change.is_none());
    }

    #[test]
    fn visible_working_does_not_hold_against_newer_claude_hook_idle() {
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Claude),
            AgentState::Working,
            false,
            false,
            true,
            false,
            now,
        );

        let change = terminal.set_hook_authority_at(
            "herdr:claude".into(),
            "claude".into(),
            AgentState::Idle,
            None,
            None,
            None,
            now + Duration::from_millis(100),
        );

        assert_eq!(terminal.state, AgentState::Idle);
        assert_eq!(
            change
                .unwrap()
                .effective_state_change
                .unwrap()
                .previous_state,
            AgentState::Working
        );
    }

    #[test]
    fn refreshed_visible_working_does_not_override_newer_hook_blocked() {
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Codex),
            AgentState::Working,
            false,
            false,
            true,
            false,
            now,
        );
        terminal.set_hook_authority_at(
            "herdr:codex".into(),
            "codex".into(),
            AgentState::Blocked,
            None,
            None,
            None,
            now + Duration::from_millis(1201),
        );

        assert_eq!(terminal.state, AgentState::Blocked);

        let change = terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Codex),
            AgentState::Working,
            false,
            false,
            true,
            false,
            now + Duration::from_millis(2000),
        );

        assert_eq!(terminal.fallback_state, AgentState::Working);
        assert_eq!(terminal.state, AgentState::Blocked);
        assert!(change.effective_state_change.is_none());
    }

    #[test]
    fn fallback_idle_does_not_override_other_agent_hook_working() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Working);
        terminal.set_hook_authority(
            "herdr:codex".into(),
            "codex".into(),
            AgentState::Working,
            None,
            None,
        );

        let change = terminal.set_detected_state_with_visible_blocker(
            Some(Agent::Codex),
            AgentState::Idle,
            false,
            true,
            false,
        );

        assert_eq!(terminal.fallback_state, AgentState::Idle);
        assert_eq!(terminal.state, AgentState::Working);
        assert!(change.is_none());
    }

    #[test]
    fn known_hook_authority_does_not_override_different_detected_agent() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Grok), AgentState::Working);
        let change = terminal.set_hook_authority(
            "herdr:claude".into(),
            "claude".into(),
            AgentState::Blocked,
            None,
            None,
        );

        assert!(change.is_none());
        assert!(terminal.hook_authority.is_none());
        assert_eq!(terminal.detected_agent, Some(Agent::Grok));
        assert_eq!(terminal.effective_agent_label(), Some("grok"));
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn detected_agent_clears_conflicting_known_hook_authority() {
        let mut terminal = test_terminal();
        terminal.set_hook_authority(
            "herdr:claude".into(),
            "claude".into(),
            AgentState::Blocked,
            None,
            None,
        );

        terminal.set_detected_state(Some(Agent::Grok), AgentState::Working);

        assert!(terminal.hook_authority.is_none());
        assert_eq!(terminal.detected_agent, Some(Agent::Grok));
        assert_eq!(terminal.effective_agent_label(), Some("grok"));
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn border_label_prefers_manual_label_over_agent_label() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);

        assert_eq!(terminal.border_label(false), None);
        assert_eq!(terminal.border_label(true).as_deref(), Some("claude"));

        terminal.set_manual_label(" reviewer ".into());
        assert_eq!(terminal.border_label(false).as_deref(), Some("reviewer"));
        assert_eq!(terminal.border_label(true).as_deref(), Some("reviewer"));

        terminal.set_manual_label("   ".into());
        assert_eq!(terminal.border_label(true).as_deref(), Some("claude"));

        terminal.set_manual_label("reviewer".into());
        terminal.clear_manual_label();
        assert_eq!(terminal.border_label(true).as_deref(), Some("claude"));
    }

    #[test]
    fn hook_authority_survives_unrelated_detected_agent_clear() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal.set_hook_authority(
            "herdr:custom".into(),
            "custom-agent".into(),
            AgentState::Working,
            None,
            None,
        );

        terminal.set_detected_state(None, AgentState::Unknown);

        assert!(terminal.hook_authority.is_some());
        assert_eq!(terminal.detected_agent, None);
        assert_eq!(terminal.effective_agent_label(), Some("custom-agent"));
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn full_lifecycle_hook_authority_ignores_detected_agent_clear_without_process_exit() {
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(test_session_path("root.jsonl")).unwrap(),
        );
        terminal.set_hook_authority_at(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            None,
            None,
            now,
        );

        let change = terminal.set_detected_state_with_screen_signals_at(
            None,
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            now + Duration::from_millis(1),
        );

        assert!(terminal.hook_authority.is_some());
        assert_eq!(terminal.detected_agent, Some(Agent::Pi));
        assert_eq!(terminal.fallback_state, AgentState::Idle);
        assert_eq!(terminal.state, AgentState::Working);
        assert!(change.effective_state_change.is_none());
    }

    #[test]
    fn detected_agent_clear_clears_matching_hook_authority() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Cursor), AgentState::Idle);
        terminal.set_hook_authority(
            "herdr:cursor".into(),
            "cursor".into(),
            AgentState::Idle,
            None,
            None,
        );

        terminal.set_detected_state(None, AgentState::Unknown);

        assert!(terminal.hook_authority.is_none());
        assert_eq!(terminal.detected_agent, None);
        assert_eq!(terminal.fallback_state, AgentState::Unknown);
        assert_eq!(terminal.effective_agent_label(), None);
        assert_eq!(terminal.state, AgentState::Unknown);
    }

    #[test]
    fn detected_agent_clear_clears_matching_working_hook_authority() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Working);
        terminal.set_hook_authority(
            "herdr:codex".into(),
            "codex".into(),
            AgentState::Working,
            None,
            None,
        );

        terminal.set_detected_state(None, AgentState::Unknown);

        assert!(terminal.hook_authority.is_none());
        assert_eq!(terminal.detected_agent, None);
        assert_eq!(terminal.effective_agent_label(), None);
        assert_eq!(terminal.state, AgentState::Unknown);
    }

    #[test]
    fn process_exit_clears_matching_hook_authority_before_reporting_idle() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Working);
        terminal.set_hook_authority(
            "herdr:codex".into(),
            "codex".into(),
            AgentState::Working,
            None,
            None,
        );

        terminal.set_detected_state_with_visible_blocker(
            Some(Agent::Codex),
            AgentState::Idle,
            false,
            false,
            true,
        );

        assert!(terminal.hook_authority.is_none());
        assert_eq!(terminal.detected_agent, Some(Agent::Codex));
        assert_eq!(terminal.effective_agent_label(), None);
        assert_eq!(terminal.state, AgentState::Idle);
    }

    #[test]
    fn stale_visible_screen_signal_does_not_override_newer_hook_authority() {
        let mut terminal = test_terminal();
        let observed = Instant::now();
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Claude),
            AgentState::Working,
            false,
            false,
            true,
            false,
            observed,
        );
        terminal.set_hook_authority_at(
            "herdr:claude".into(),
            "claude".into(),
            AgentState::Working,
            None,
            None,
            Some(1),
            observed + Duration::from_secs(1),
        );

        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Claude),
            AgentState::Idle,
            false,
            true,
            false,
            false,
            observed,
        );

        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn stale_process_exit_preserves_newer_custom_authority() {
        let mut terminal = test_terminal();
        let observed = Instant::now();
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            false,
            false,
            false,
            observed,
        );
        terminal.set_hook_authority_at(
            "custom:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            None,
            Some(100),
            observed + Duration::from_secs(1),
        );

        let mutation = terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            false,
            false,
            true,
            observed,
        );

        assert!(!mutation.agent_released);
        assert_eq!(terminal.state, AgentState::Working);
        assert_eq!(
            terminal
                .hook_authority
                .as_ref()
                .map(|hook| hook.source.as_str()),
            Some("custom:pi")
        );
    }

    #[test]
    fn custom_authority_reanchors_sequence_after_process_restart() {
        let mut terminal = test_terminal();
        let observed = Instant::now();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal.set_hook_authority_at(
            "custom:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            None,
            Some(100),
            observed,
        );
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            false,
            false,
            true,
            observed + Duration::from_millis(1),
        );
        terminal.set_detected_state_with_screen_signals_at(
            None,
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            observed + Duration::from_millis(2),
        );

        assert!(terminal
            .release_agent_with_mutation("custom:pi", "pi", Some(200))
            .is_none());
        assert!(terminal
            .clear_hook_authority_with_mutation(Some("custom:pi"), Some(201))
            .is_none());
        assert!(terminal
            .set_hook_authority(
                "custom:pi".into(),
                "pi".into(),
                AgentState::Working,
                None,
                Some(1),
            )
            .is_none());
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            false,
            false,
            false,
            observed + Duration::from_millis(3),
        );
        assert!(terminal
            .set_hook_authority(
                "custom:pi".into(),
                "pi".into(),
                AgentState::Working,
                None,
                Some(1),
            )
            .is_some());
    }

    #[test]
    fn process_exit_clears_newer_same_agent_hook_authority() {
        let mut terminal = test_terminal();
        let observed = Instant::now();
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Codex),
            AgentState::Working,
            false,
            false,
            false,
            false,
            observed,
        );
        terminal.set_hook_authority_at(
            "herdr:codex".into(),
            "codex".into(),
            AgentState::Working,
            None,
            None,
            Some(1),
            observed,
        );
        terminal.set_hook_authority_at(
            "herdr:codex".into(),
            "codex".into(),
            AgentState::Working,
            None,
            None,
            Some(2),
            observed + Duration::from_secs(1),
        );

        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Codex),
            AgentState::Idle,
            false,
            false,
            false,
            true,
            observed,
        );

        assert!(terminal.hook_authority.is_none());
        assert_eq!(terminal.state, AgentState::Idle);
        assert_eq!(terminal.effective_agent_label(), None);
    }

    #[test]
    fn detected_agent_change_clears_previous_matching_hook_authority() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Idle);
        terminal.set_hook_authority(
            "herdr:codex".into(),
            "codex".into(),
            AgentState::Idle,
            None,
            None,
        );

        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Working);

        assert!(terminal.hook_authority.is_none());
        assert_eq!(terminal.detected_agent, Some(Agent::OpenCode));
        assert_eq!(terminal.effective_agent_label(), Some("opencode"));
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn stale_hook_report_sequence_is_ignored_for_same_source() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(test_session_path("root.jsonl")).unwrap(),
        );
        terminal.set_hook_authority(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            Some(20),
        );

        let change = terminal.set_hook_authority(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Idle,
            None,
            Some(19),
        );

        assert!(change.is_none());
        assert_eq!(terminal.state, AgentState::Working);
        assert_eq!(
            terminal.hook_authority.as_ref().unwrap().state,
            AgentState::Working
        );
    }

    #[test]
    fn accepted_hook_report_stores_session_ref() {
        let mut terminal = test_terminal();
        let session_path = test_session_path("pi.jsonl");
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(session_path.clone()).unwrap(),
        );
        let mutation = terminal
            .set_hook_authority_with_session_ref(
                "herdr:pi".into(),
                "pi".into(),
                AgentState::Working,
                None,
                crate::agent_resume::AgentSessionRef::path(session_path.clone()),
                Some(20),
            )
            .expect("accepted report");

        assert!(!mutation.session_ref_changed);
        assert_eq!(
            terminal
                .hook_authority
                .as_ref()
                .and_then(|authority| authority.session_ref.as_ref())
                .map(|session_ref| (&session_ref.kind, session_ref.value.as_str())),
            Some((
                &crate::agent_resume::AgentSessionRefKind::Path,
                session_path.as_str()
            ))
        );
    }

    #[test]
    fn stale_hook_report_cannot_overwrite_session_ref() {
        let mut terminal = test_terminal();
        let session_path = test_session_path("pi.jsonl");
        let new_session_path = test_session_path("new.jsonl");
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(session_path.clone()).unwrap(),
        );
        terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(session_path.clone()),
            Some(20),
        );

        let mutation = terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(new_session_path),
            Some(19),
        );

        assert!(mutation.is_none());
        assert_eq!(
            terminal
                .hook_authority
                .as_ref()
                .and_then(|authority| authority.session_ref.as_ref())
                .map(|session_ref| session_ref.value.as_str()),
            Some(session_path.as_str())
        );
    }

    #[test]
    fn accepted_hook_report_without_session_ref_clears_previous_ref() {
        let mut terminal = test_terminal();
        let session_path = test_session_path("pi.jsonl");
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(session_path.clone()).unwrap(),
        );
        terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(session_path),
            Some(20),
        );

        let mutation = terminal
            .set_hook_authority_with_session_ref(
                "herdr:pi".into(),
                "pi".into(),
                AgentState::Working,
                None,
                None,
                Some(21),
            )
            .expect("accepted report");

        assert!(mutation.session_ref_changed);
        assert!(mutation.effective_state_change.is_none());
        assert!(terminal
            .hook_authority
            .as_ref()
            .unwrap()
            .session_ref
            .is_none());
    }

    #[test]
    fn different_same_agent_session_ref_is_ignored_until_current_session_clears() {
        let mut terminal = test_terminal();
        terminal
            .set_agent_session_ref(
                "herdr:claude".into(),
                "claude".into(),
                crate::agent_resume::AgentSessionRef::id("claude-session"),
                Some(20),
            )
            .expect("initial session should be accepted");

        let mutation = terminal.set_agent_session_ref(
            "herdr:claude".into(),
            "claude".into(),
            crate::agent_resume::AgentSessionRef::id("nested-session"),
            Some(21),
        );

        assert!(mutation.is_none());
        assert_eq!(
            terminal.hook_report_sequences.get("herdr:claude"),
            Some(&21)
        );
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("claude-session")
        );
    }

    #[test]
    fn claude_startup_session_ref_does_not_replace_existing_session_ref() {
        let mut terminal = test_terminal();
        terminal
            .set_agent_session_ref(
                "herdr:claude".into(),
                "claude".into(),
                crate::agent_resume::AgentSessionRef::id("claude-session"),
                Some(20),
            )
            .expect("initial session should be accepted");

        let mutation = terminal.set_agent_session_ref_for_session_start(
            "herdr:claude".into(),
            "claude".into(),
            crate::agent_resume::AgentSessionRef::id("nested-session"),
            Some(21),
            Some("startup".into()),
        );

        assert!(mutation.is_none());
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("claude-session")
        );
    }

    #[test]
    fn claude_lifecycle_session_ref_replaces_existing_session_ref() {
        for session_start_source in ["clear", "resume", "compact"] {
            let mut terminal = test_terminal();
            terminal
                .set_agent_session_ref(
                    "herdr:claude".into(),
                    "claude".into(),
                    crate::agent_resume::AgentSessionRef::id("claude-session"),
                    Some(20),
                )
                .expect("initial session should be accepted");

            let next_session = format!("{session_start_source}-session");
            let mutation = terminal
                .set_agent_session_ref_for_session_start(
                    "herdr:claude".into(),
                    "claude".into(),
                    crate::agent_resume::AgentSessionRef::id(&next_session),
                    Some(21),
                    Some(session_start_source.into()),
                )
                .unwrap_or_else(|| panic!("{session_start_source} should replace the session"));

            assert!(
                mutation.session_ref_changed,
                "{session_start_source} should mark the session changed"
            );
            assert_eq!(
                terminal
                    .persisted_agent_session
                    .as_ref()
                    .map(|session| session.session_ref.value.as_str()),
                Some(next_session.as_str()),
                "{session_start_source} should store the replacement session"
            );
        }
    }

    #[test]
    fn codex_lifecycle_session_ref_replaces_existing_session_ref() {
        for session_start_source in ["startup", "clear", "resume", "compact"] {
            let mut terminal = test_terminal();
            terminal
                .set_agent_session_ref(
                    "herdr:codex".into(),
                    "codex".into(),
                    crate::agent_resume::AgentSessionRef::id("codex-session"),
                    Some(20),
                )
                .expect("initial session should be accepted");

            let next_session = format!("codex-{session_start_source}-session");
            let mutation = terminal
                .set_agent_session_ref_for_session_start(
                    "herdr:codex".into(),
                    "codex".into(),
                    crate::agent_resume::AgentSessionRef::id(&next_session),
                    Some(21),
                    Some(session_start_source.into()),
                )
                .unwrap_or_else(|| panic!("{session_start_source} should replace the session"));

            assert!(mutation.session_ref_changed);
            assert_eq!(
                terminal
                    .persisted_agent_session
                    .as_ref()
                    .map(|session| session.session_ref.value.as_str()),
                Some(next_session.as_str())
            );
        }
    }

    #[test]
    fn qwen_lifecycle_session_ref_replaces_existing_session_ref() {
        for session_start_source in ["startup", "clear", "resume", "compact", "branch"] {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(Agent::Qwen), AgentState::Idle);
            terminal
                .set_agent_session_ref(
                    "herdr:qwen".into(),
                    "qwen".into(),
                    crate::agent_resume::AgentSessionRef::id("qwen-session"),
                    Some(20),
                )
                .expect("initial session should be accepted");

            let next_session = format!("qwen-{session_start_source}-session");
            let mutation = terminal
                .set_agent_session_ref_for_session_start(
                    "herdr:qwen".into(),
                    "qwen".into(),
                    crate::agent_resume::AgentSessionRef::id(&next_session),
                    Some(21),
                    Some(session_start_source.into()),
                )
                .unwrap_or_else(|| panic!("{session_start_source} should replace the session"));

            assert!(mutation.session_ref_changed);
            assert_eq!(
                terminal
                    .persisted_agent_session
                    .as_ref()
                    .map(|session| session.session_ref.value.as_str()),
                Some(next_session.as_str())
            );
        }
    }

    #[test]
    fn qwen_session_ref_does_not_replace_without_foreground_qwen() {
        let mut terminal = test_terminal();
        terminal
            .set_agent_session_ref(
                "herdr:qwen".into(),
                "qwen".into(),
                crate::agent_resume::AgentSessionRef::id("qwen-parent"),
                Some(20),
            )
            .expect("initial session should be accepted");

        let mutation = terminal.set_agent_session_ref_for_session_start(
            "herdr:qwen".into(),
            "qwen".into(),
            crate::agent_resume::AgentSessionRef::id("qwen-branch"),
            Some(21),
            Some("branch".into()),
        );

        assert!(mutation.is_none());
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("qwen-parent")
        );
    }

    #[test]
    fn grok_new_session_ref_replaces_existing_session_ref() {
        let mut terminal = test_terminal();
        terminal
            .set_agent_session_ref(
                "herdr:grok".into(),
                "grok".into(),
                crate::agent_resume::AgentSessionRef::id("grok-old"),
                Some(20),
            )
            .expect("initial session should be accepted");

        let mutation = terminal
            .set_agent_session_ref_for_session_start(
                "herdr:grok".into(),
                "grok".into(),
                crate::agent_resume::AgentSessionRef::id("grok-new"),
                Some(21),
                Some("new".into()),
            )
            .expect("new should replace the grok session");

        assert!(mutation.session_ref_changed);
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("grok-new")
        );
    }

    #[test]
    fn opencode_server_new_does_not_replace_existing_session_ref() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Idle);
        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:opencode".into(),
                "opencode".into(),
                crate::agent_resume::AgentSessionRef::id("opencode-visible"),
                None,
                Some("select".into()),
            )
            .expect("local selection should be accepted");

        let mutation = terminal.set_agent_session_ref_for_session_start(
            "herdr:opencode".into(),
            "opencode".into(),
            crate::agent_resume::AgentSessionRef::id("opencode-attached-client"),
            Some(21),
            Some("new".into()),
        );

        assert!(mutation.is_none());
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("opencode-visible")
        );
    }

    #[test]
    fn opencode_server_resume_does_not_replace_existing_session_ref() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Idle);
        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:opencode".into(),
                "opencode".into(),
                crate::agent_resume::AgentSessionRef::id("opencode-visible"),
                None,
                Some("select".into()),
            )
            .expect("local selection should be accepted");

        let mutation = terminal.set_agent_session_ref_for_session_start(
            "herdr:opencode".into(),
            "opencode".into(),
            crate::agent_resume::AgentSessionRef::id("opencode-attached-client"),
            Some(21),
            Some("resume".into()),
        );

        assert!(mutation.is_none());
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("opencode-visible")
        );
    }

    #[test]
    fn opencode_tui_selection_anchors_after_process_detection() {
        let mut terminal = test_terminal();
        let startup_selection = terminal.set_agent_session_ref_for_session_start(
            "herdr:opencode".into(),
            "opencode".into(),
            crate::agent_resume::AgentSessionRef::id("opencode-startup-selection"),
            None,
            Some("select".into()),
        );
        assert!(startup_selection.is_none());
        assert_eq!(
            terminal
                .suppressed_full_lifecycle_hook_reports
                .get("herdr:opencode")
                .and_then(|suppressed| suppressed.replacement_session_ref.as_ref())
                .map(|session| session.value.as_str()),
            Some("opencode-startup-selection")
        );

        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Idle);
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("opencode-startup-selection")
        );
        assert!(!terminal
            .suppressed_full_lifecycle_hook_reports
            .contains_key("herdr:opencode"));

        terminal.suppressed_full_lifecycle_hook_reports.insert(
            "herdr:opencode".into(),
            SuppressedFullLifecycleHookReport {
                agent_label: "opencode".into(),
                session_ref: None,
                observed_at: Instant::now(),
                reason: FullLifecycleHookSuppressionReason::ProcessExit,
                replacement_session_ref: None,
                pending_replacement_report: None,
            },
        );
        let selected = terminal
            .set_agent_session_ref_for_session_start(
                "herdr:opencode".into(),
                "opencode".into(),
                crate::agent_resume::AgentSessionRef::id("opencode-reselected"),
                None,
                Some("select".into()),
            )
            .expect("local TUI selection should reconcile generation suppression");

        assert!(selected.session_ref_changed);
        assert!(!terminal
            .suppressed_full_lifecycle_hook_reports
            .contains_key("herdr:opencode"));
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("opencode-reselected")
        );
        assert!(!terminal
            .hook_report_sequences
            .contains_key("herdr:opencode"));
    }

    #[test]
    fn opencode_child_prompt_reports_with_root_id_preserve_lifecycle_authority() {
        let mut terminal = test_terminal();
        let root = crate::agent_resume::AgentSessionRef::id("opencode-root").unwrap();
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::OpenCode,
            "herdr:opencode",
            "opencode",
            root.clone(),
        );

        // The plugin projects child permission/question events onto their root.
        for (seq, state) in [
            (20, AgentState::Working),
            (21, AgentState::Blocked),
            (22, AgentState::Working),
            (23, AgentState::Idle),
        ] {
            let mutation = terminal
                .set_hook_authority_with_session_ref(
                    "herdr:opencode".into(),
                    "opencode".into(),
                    state,
                    None,
                    Some(root.clone()),
                    Some(seq),
                )
                .expect("root-scoped lifecycle report should be accepted");
            assert!(!mutation.session_ref_changed);
            assert_eq!(terminal.state, state);
            assert_eq!(
                terminal
                    .hook_authority
                    .as_ref()
                    .unwrap()
                    .session_ref
                    .as_ref(),
                Some(&root)
            );
        }

        let foreign_child_prompt = terminal.set_hook_authority_with_session_ref(
            "herdr:opencode".into(),
            "opencode".into(),
            AgentState::Blocked,
            None,
            crate::agent_resume::AgentSessionRef::id("opencode-other-root"),
            Some(24),
        );
        assert!(foreign_child_prompt.is_none());
        assert_eq!(terminal.state, AgentState::Idle);
    }

    #[test]
    fn opencode_tui_selection_reanchors_full_lifecycle_authority() {
        let mut terminal = test_terminal();
        let old_session = crate::agent_resume::AgentSessionRef::id("opencode-newer").unwrap();
        let selected_session =
            crate::agent_resume::AgentSessionRef::id("opencode-selected-older").unwrap();
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::OpenCode,
            "herdr:opencode",
            "opencode",
            old_session.clone(),
        );
        terminal
            .set_hook_authority_with_session_ref(
                "herdr:opencode".into(),
                "opencode".into(),
                AgentState::Idle,
                None,
                Some(old_session.clone()),
                Some(20),
            )
            .expect("initial session should own lifecycle state");
        let attached_session =
            crate::agent_resume::AgentSessionRef::id("opencode-attached-client").unwrap();
        let attached = terminal.set_hook_authority_with_session_ref(
            "herdr:opencode".into(),
            "opencode".into(),
            AgentState::Working,
            None,
            Some(attached_session.clone()),
            Some(21),
        );
        assert!(attached.is_none());
        assert!(!terminal
            .suppressed_full_lifecycle_hook_reports
            .contains_key("herdr:opencode"));

        let selected = terminal
            .set_agent_session_ref_for_session_start(
                "herdr:opencode".into(),
                "opencode".into(),
                Some(selected_session.clone()),
                None,
                Some("select".into()),
            )
            .expect("selected session should replace the previous session");

        assert!(selected.session_ref_changed);
        assert!(terminal.hook_authority.is_none());
        assert!(!terminal
            .suppressed_full_lifecycle_hook_reports
            .contains_key("herdr:opencode"));
        assert_eq!(
            terminal.hook_report_sequences.get("herdr:opencode"),
            Some(&20)
        );
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| &session.session_ref),
            Some(&selected_session)
        );

        terminal
            .set_hook_authority_with_session_ref(
                "herdr:opencode".into(),
                "opencode".into(),
                AgentState::Working,
                None,
                Some(selected_session.clone()),
                Some(21),
            )
            .expect("selected session should regain lifecycle authority");
        assert_eq!(terminal.state, AgentState::Working);
        assert_eq!(
            terminal
                .hook_authority
                .as_ref()
                .and_then(|authority| authority.session_ref.as_ref()),
            Some(&selected_session)
        );

        let late_old_session = terminal.set_hook_authority_with_session_ref(
            "herdr:opencode".into(),
            "opencode".into(),
            AgentState::Idle,
            None,
            Some(old_session),
            Some(22),
        );
        assert!(late_old_session.is_none());
        assert_eq!(terminal.state, AgentState::Working);
        assert_eq!(
            terminal
                .hook_authority
                .as_ref()
                .and_then(|authority| authority.session_ref.as_ref()),
            Some(&selected_session)
        );

        let late_attached_session = terminal.set_hook_authority_with_session_ref(
            "herdr:opencode".into(),
            "opencode".into(),
            AgentState::Blocked,
            None,
            Some(attached_session),
            Some(23),
        );
        assert!(late_attached_session.is_none());
        assert_eq!(terminal.state, AgentState::Working);

        let final_session =
            crate::agent_resume::AgentSessionRef::id("opencode-final-selection").unwrap();
        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:opencode".into(),
                "opencode".into(),
                Some(final_session.clone()),
                None,
                Some("select".into()),
            )
            .expect("another local selection should remain authoritative");
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| &session.session_ref),
            Some(&final_session)
        );
        assert!(!terminal
            .suppressed_full_lifecycle_hook_reports
            .contains_key("herdr:opencode"));
    }

    #[test]
    fn pi_session_replacement_clears_the_previous_sessions_alias() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:pi".into(),
                "pi".into(),
                crate::agent_resume::AgentSessionRef::id("pi-old"),
                Some(20),
                Some("new".into()),
            )
            .expect("initial session should be accepted");
        terminal.set_agent_name("reviewer".into());

        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:pi".into(),
                "pi".into(),
                crate::agent_resume::AgentSessionRef::id("pi-new"),
                Some(21),
                Some("new".into()),
            )
            .expect("new should replace the session");

        assert!(terminal.agent_name.is_none());
    }

    #[test]
    fn managed_agent_name_survives_native_session_replacement() {
        let mut terminal = test_terminal();
        let now = Instant::now();
        terminal.begin_managed_agent(
            "reviewer".into(),
            Agent::OpenCode,
            now,
            Duration::ZERO,
            Duration::from_secs(1),
        );
        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Idle);
        assert!(terminal.reconcile_managed_agent_at(now, false));

        for session in ["opencode-old", "opencode-new"] {
            terminal
                .set_agent_session_ref_for_session_start(
                    "herdr:opencode".into(),
                    "opencode".into(),
                    crate::agent_resume::AgentSessionRef::id(session),
                    None,
                    Some("select".into()),
                )
                .expect("managed session should be accepted");
        }

        assert_eq!(terminal.agent_name.as_deref(), Some("reviewer"));
        assert!(terminal.managed_agent_interactive_ready());
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("opencode-new")
        );
    }

    #[test]
    fn opencode_session_ref_without_start_source_does_not_replace_existing() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Idle);
        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:opencode".into(),
                "opencode".into(),
                crate::agent_resume::AgentSessionRef::id("opencode-old"),
                None,
                Some("select".into()),
            )
            .expect("local selection should be accepted");

        // session.updated reports carry no session_start_source, so a different
        // id must not displace the established session (cross-talk guard).
        let mutation = terminal.set_agent_session_ref_for_session_start(
            "herdr:opencode".into(),
            "opencode".into(),
            crate::agent_resume::AgentSessionRef::id("opencode-other"),
            Some(21),
            None,
        );

        assert!(mutation.is_none());
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("opencode-old")
        );
    }

    #[test]
    fn different_owner_session_ref_does_not_replace_existing_session_ref() {
        let mut terminal = test_terminal();
        terminal
            .set_agent_session_ref(
                "herdr:droid".into(),
                "droid".into(),
                crate::agent_resume::AgentSessionRef::id("droid-session"),
                Some(20),
            )
            .expect("initial session should be accepted");

        let mutation = terminal.set_agent_session_ref_for_session_start(
            "herdr:claude".into(),
            "claude".into(),
            crate::agent_resume::AgentSessionRef::id("claude-session"),
            Some(21),
            Some("resume".into()),
        );

        assert!(mutation.is_none());
        assert_eq!(
            terminal.persisted_agent_session.as_ref().map(|session| (
                session.source.as_str(),
                session.agent.as_str(),
                session.session_ref.value.as_str()
            )),
            Some(("herdr:droid", "droid", "droid-session"))
        );
    }

    #[test]
    fn grok_new_session_does_not_replace_a_different_owner() {
        let mut terminal = test_terminal();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("claude-session").unwrap(),
        });
        terminal.set_detected_state(Some(Agent::Grok), AgentState::Idle);

        let mutation = terminal.set_agent_session_ref_for_session_start(
            "herdr:grok".into(),
            "grok".into(),
            crate::agent_resume::AgentSessionRef::id("grok-session"),
            Some(21),
            Some("new".into()),
        );

        assert!(mutation.is_none());
        assert_eq!(
            terminal.persisted_agent_session.as_ref().map(|session| (
                session.source.as_str(),
                session.agent.as_str(),
                session.session_ref.value.as_str()
            )),
            Some(("herdr:claude", "claude", "claude-session"))
        );
    }

    #[test]
    fn foreground_agent_session_replaces_stale_different_owner_session_ref() {
        for session_start_source in ["resume", "startup"] {
            let mut terminal = test_terminal();
            terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
                source: "herdr:codex".into(),
                agent: "codex".into(),
                session_ref: crate::agent_resume::AgentSessionRef::id("codex-session").unwrap(),
            });
            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);

            let mutation = terminal
                .set_agent_session_ref_for_session_start(
                    "herdr:claude".into(),
                    "claude".into(),
                    crate::agent_resume::AgentSessionRef::id("claude-session"),
                    Some(21),
                    Some(session_start_source.into()),
                )
                .unwrap_or_else(|| {
                    panic!("{session_start_source} should replace stale codex session")
                });

            assert!(mutation.session_ref_changed);
            assert_eq!(
                terminal.persisted_agent_session.as_ref().map(|session| (
                    session.source.as_str(),
                    session.agent.as_str(),
                    session.session_ref.value.as_str()
                )),
                Some(("herdr:claude", "claude", "claude-session")),
                "{session_start_source} should store claude session"
            );
        }
    }

    #[test]
    fn foreground_agent_session_requires_lifecycle_source_to_replace_different_owner() {
        for session_start_source in [None, Some("other")] {
            let mut terminal = test_terminal();
            terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
                source: "herdr:codex".into(),
                agent: "codex".into(),
                session_ref: crate::agent_resume::AgentSessionRef::id("codex-session").unwrap(),
            });
            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);

            let mutation = terminal.set_agent_session_ref_for_session_start(
                "herdr:claude".into(),
                "claude".into(),
                crate::agent_resume::AgentSessionRef::id("claude-session"),
                Some(21),
                session_start_source.map(str::to_string),
            );

            assert!(
                mutation.is_none(),
                "{session_start_source:?} should not replace"
            );
            assert_eq!(
                terminal.persisted_agent_session.as_ref().map(|session| (
                    session.source.as_str(),
                    session.agent.as_str(),
                    session.session_ref.value.as_str()
                )),
                Some(("herdr:codex", "codex", "codex-session"))
            );
        }
    }

    #[test]
    fn different_owner_session_ref_requires_matching_detected_agent() {
        for session_start_source in ["startup", "resume"] {
            for detected_agent in [None, Some(Agent::Codex)] {
                let mut terminal = test_terminal();
                terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
                    source: "herdr:codex".into(),
                    agent: "codex".into(),
                    session_ref: crate::agent_resume::AgentSessionRef::id("codex-session").unwrap(),
                });
                terminal.set_detected_state(detected_agent, AgentState::Idle);

                let mutation = terminal.set_agent_session_ref_for_session_start(
                    "herdr:claude".into(),
                    "claude".into(),
                    crate::agent_resume::AgentSessionRef::id("claude-session"),
                    Some(21),
                    Some(session_start_source.into()),
                );

                assert!(
                    mutation.is_none(),
                    "{session_start_source} with {detected_agent:?} should not replace"
                );
                assert_eq!(
                    terminal.persisted_agent_session.as_ref().map(|session| (
                        session.source.as_str(),
                        session.agent.as_str(),
                        session.session_ref.value.as_str()
                    )),
                    Some(("herdr:codex", "codex", "codex-session"))
                );
            }
        }
    }

    #[test]
    fn custom_session_report_does_not_replace_different_owner_session_ref() {
        let mut terminal = test_terminal();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:codex".into(),
            agent: "codex".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("codex-session").unwrap(),
        });
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);

        let mutation = terminal.set_agent_session_ref_for_session_start(
            "custom:claude".into(),
            "claude".into(),
            crate::agent_resume::AgentSessionRef::id("claude-session"),
            Some(21),
            Some("resume".into()),
        );

        assert!(mutation.is_none());
        assert_eq!(
            terminal.persisted_agent_session.as_ref().map(|session| (
                session.source.as_str(),
                session.agent.as_str(),
                session.session_ref.value.as_str()
            )),
            Some(("herdr:codex", "codex", "codex-session"))
        );
    }

    #[test]
    fn foreground_agent_session_replaces_stale_different_owner_hook_authority() {
        let mut terminal = test_terminal();
        let now = std::time::Instant::now();
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::OpenCode,
            "herdr:opencode",
            "opencode",
            crate::agent_resume::AgentSessionRef::id("opencode-session").unwrap(),
        );
        terminal
            .set_hook_authority_at(
                "herdr:opencode".into(),
                "opencode".into(),
                AgentState::Working,
                None,
                crate::agent_resume::AgentSessionRef::id("opencode-session"),
                Some(20),
                now + Duration::from_millis(1),
            )
            .expect("initial hook authority should be accepted");
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Codex),
            AgentState::Idle,
            false,
            false,
            false,
            false,
            now,
        );

        let mutation = terminal
            .set_agent_session_ref_for_session_start(
                "herdr:codex".into(),
                "codex".into(),
                crate::agent_resume::AgentSessionRef::id("codex-session"),
                Some(21),
                Some("startup".into()),
            )
            .expect("foreground codex should replace stale hook authority");

        assert!(mutation.session_ref_changed);
        assert!(terminal.hook_authority.is_none());
        assert_eq!(
            terminal.current_session_identity_for_persistence(),
            Some((
                "herdr:codex".into(),
                "codex".into(),
                crate::agent_resume::AgentSessionRefKind::Id,
                "codex-session".into()
            ))
        );
        let late_old_session = terminal.set_hook_authority_with_session_ref(
            "herdr:opencode".into(),
            "opencode".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::id("opencode-session"),
            Some(22),
        );
        assert!(late_old_session.is_none());

        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Idle);
        terminal
            .set_agent_session_ref_for_session_start(
                "herdr:opencode".into(),
                "opencode".into(),
                crate::agent_resume::AgentSessionRef::id("opencode-new-session"),
                None,
                Some("select".into()),
            )
            .expect("fresh local selection");
        let fresh_session = terminal.set_hook_authority_with_session_ref(
            "herdr:opencode".into(),
            "opencode".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::id("opencode-new-session"),
            Some(24),
        );
        assert!(fresh_session.is_some());
    }

    #[test]
    fn different_owner_full_lifecycle_hook_does_not_replace_existing_session_ref() {
        let mut terminal = test_terminal();
        terminal
            .set_agent_session_ref(
                "herdr:droid".into(),
                "droid".into(),
                crate::agent_resume::AgentSessionRef::id("droid-session"),
                Some(20),
            )
            .expect("initial session should be accepted");

        let mutation = terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path("/tmp/pi-session.jsonl"),
            Some(21),
        );

        assert!(mutation.is_none());
        assert!(terminal.hook_authority.is_none());
        assert_eq!(
            terminal.persisted_agent_session.as_ref().map(|session| (
                session.source.as_str(),
                session.agent.as_str(),
                session.session_ref.value.as_str()
            )),
            Some(("herdr:droid", "droid", "droid-session"))
        );
    }

    #[test]
    fn repeated_same_agent_session_ref_is_accepted_without_session_change() {
        let mut terminal = test_terminal();
        terminal
            .set_agent_session_ref(
                "herdr:claude".into(),
                "claude".into(),
                crate::agent_resume::AgentSessionRef::id("claude-session"),
                Some(20),
            )
            .expect("initial session should be accepted");

        let mutation = terminal
            .set_agent_session_ref(
                "herdr:claude".into(),
                "claude".into(),
                crate::agent_resume::AgentSessionRef::id("claude-session"),
                Some(21),
            )
            .expect("same session should be accepted");

        assert!(!mutation.session_ref_changed);
    }

    #[test]
    fn hook_authority_rejects_state_from_a_different_session() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::OpenCode,
            "herdr:opencode",
            "opencode",
            crate::agent_resume::AgentSessionRef::id("opencode-session").unwrap(),
        );
        terminal
            .set_hook_authority_with_session_ref(
                "herdr:opencode".into(),
                "opencode".into(),
                AgentState::Working,
                None,
                crate::agent_resume::AgentSessionRef::id("opencode-session"),
                Some(20),
            )
            .expect("initial session should be accepted");

        let mutation = terminal.set_hook_authority_with_session_ref(
            "herdr:opencode".into(),
            "opencode".into(),
            AgentState::Blocked,
            Some("needs approval".into()),
            crate::agent_resume::AgentSessionRef::id("nested-session"),
            Some(21),
        );

        assert!(mutation.is_none());
        assert_eq!(terminal.state, AgentState::Working);
        assert_eq!(
            terminal
                .hook_authority
                .as_ref()
                .and_then(|authority| authority.session_ref.as_ref())
                .map(|session_ref| session_ref.value.as_str()),
            Some("opencode-session")
        );
    }

    #[test]
    fn detected_agent_clear_does_not_clear_current_session_ref() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Working);
        terminal
            .set_agent_session_ref(
                "herdr:claude".into(),
                "claude".into(),
                crate::agent_resume::AgentSessionRef::id("claude-session"),
                Some(20),
            )
            .expect("initial session should be accepted");

        let clear = terminal.set_detected_state_with_mutation(None, AgentState::Unknown);
        assert!(!clear.session_ref_changed);

        let mutation = terminal.set_agent_session_ref(
            "herdr:claude".into(),
            "claude".into(),
            crate::agent_resume::AgentSessionRef::id("new-session"),
            Some(21),
        );

        assert!(mutation.is_none());
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("claude-session")
        );
    }

    #[test]
    fn clearing_hook_authority_clears_session_ref() {
        let mut terminal = test_terminal();
        let session_path = test_session_path("pi.jsonl");
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(session_path.clone()).unwrap(),
        );
        terminal.set_hook_authority_with_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::path(session_path),
            Some(20),
        );

        let mutation = terminal
            .clear_hook_authority_with_mutation(Some("herdr:pi"), Some(21))
            .expect("accepted clear");

        assert!(mutation.session_ref_changed);
        assert!(terminal.hook_authority.is_none());
    }

    #[test]
    fn a_process_exit_observation_alone_does_not_free_the_name() {
        // The pane keeps reporting the same agent throughout: this models a
        // process-exit observation that is wrong (the agent is still running),
        // which is what `agent_alias_survives_detection_uncertainty_...`
        // already refuses to let destroy the name through the detection path.
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Working);
        terminal.set_agent_name("reviewer".into());

        // Contract today: losing detection does not cost the pane its name.
        terminal.set_detected_state(None, AgentState::Unknown);
        assert_eq!(
            terminal.agent_name.as_deref(),
            Some("reviewer"),
            "detection uncertainty must not release the name"
        );
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Working);

        // A single process-exit observation for that same agent destroys it.
        let exit = terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            false,
            false,
            true,
            now,
        );
        assert!(exit.agent_released);
        assert_eq!(
            terminal.agent_name.as_deref(),
            Some("reviewer"),
            "a process-exit observation for the still-detected agent must not \
             destroy a name that detection uncertainty is allowed to keep"
        );

        // And the agent proving it is alive again must not leave the pane
        // permanently unreachable by the name its owner assigned.
        terminal.set_detected_agent_process_at(Agent::Pi, now + Duration::from_secs(1));
        assert_eq!(
            terminal.agent_name.as_deref(),
            Some("reviewer"),
            "the name must still resolve once the agent is observed alive again"
        );
        assert_eq!(terminal.detected_agent, Some(Agent::Pi));
    }

    #[test]
    fn a_confirmed_agent_exit_still_frees_the_name_for_reuse() {
        // The other side of `a_process_exit_observation_alone_does_not_free_the_name`:
        // once the agent is actually gone from the pane the name must be
        // released, so `agent start` can reuse it.
        let now = Instant::now();
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Working);
        terminal.set_agent_name("reviewer".into());

        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            false,
            false,
            true,
            now,
        );
        assert_eq!(terminal.agent_name.as_deref(), Some("reviewer"));

        // The agent really is gone: the next observation finds no agent while
        // the recorded exit still stands.
        terminal.set_detected_state_with_screen_signals_at(
            None,
            AgentState::Unknown,
            false,
            false,
            false,
            false,
            now + Duration::from_secs(1),
        );
        assert_eq!(
            terminal.agent_name, None,
            "a confirmed exit must release the name"
        );
        assert!(!terminal.is_agent_terminal());
    }

    #[test]
    fn agent_alias_survives_detection_uncertainty_and_reported_release_but_not_replacement() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Working);
        terminal.set_agent_name("reviewer".into());

        terminal.set_detected_state(None, AgentState::Unknown);
        assert_eq!(terminal.agent_name.as_deref(), Some("reviewer"));

        terminal.set_detected_state(Some(Agent::Codex), AgentState::Idle);
        assert!(terminal.agent_name.is_none());

        terminal.set_agent_name("replacement".into());
        let mutation = terminal
            .release_agent_with_mutation("herdr:codex", "codex", None)
            .expect("detected agent release should be accepted");
        assert!(!mutation.agent_released);
        assert_eq!(terminal.agent_name.as_deref(), Some("replacement"));
        assert_eq!(terminal.detected_agent, Some(Agent::Codex));
    }

    #[test]
    fn custom_release_preserves_process_owned_agent_state() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal
            .set_hook_authority(
                "custom:pi".into(),
                "pi".into(),
                AgentState::Working,
                None,
                Some(10),
            )
            .expect("custom state should be accepted");

        let mutation = terminal
            .release_agent_with_mutation("custom:pi", "pi", Some(11))
            .expect("custom release should be accepted");

        assert!(!mutation.agent_released);
        assert!(terminal.hook_authority.is_none());
        assert_eq!(terminal.detected_agent, Some(Agent::Pi));
        assert_eq!(terminal.effective_agent_label(), Some("pi"));
        assert_eq!(terminal.state, AgentState::Idle);
    }

    #[test]
    fn agent_replacement_clears_alias_owned_by_hook_identity() {
        let mut terminal = test_terminal();
        terminal.set_hook_authority(
            "herdr:claude".into(),
            "claude".into(),
            AgentState::Working,
            None,
            Some(20),
        );
        terminal.set_agent_name("reviewer".into());

        terminal.set_detected_state(Some(Agent::Grok), AgentState::Idle);

        assert!(terminal.agent_name.is_none());
        assert_eq!(terminal.effective_known_agent(), Some(Agent::Grok));
    }

    #[test]
    fn accepted_hook_replacement_clears_the_previous_agents_alias() {
        let mut terminal = test_terminal();
        terminal
            .set_hook_authority_at(
                "custom:agent".into(),
                "pi".into(),
                AgentState::Working,
                None,
                None,
                Some(20),
                Instant::now(),
            )
            .expect("initial hook should be accepted");
        terminal.set_agent_name("reviewer".into());

        terminal
            .set_hook_authority_at(
                "custom:agent".into(),
                "claude".into(),
                AgentState::Idle,
                None,
                None,
                Some(21),
                Instant::now(),
            )
            .expect("replacement hook should be accepted");

        assert!(terminal.agent_name.is_none());
        assert_eq!(terminal.effective_known_agent(), Some(Agent::Claude));
    }

    #[test]
    fn accepted_same_kind_hook_owner_replacement_clears_the_alias() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(test_session_path("first.jsonl")).unwrap(),
        );
        terminal
            .set_hook_authority_at(
                "herdr:pi".into(),
                "pi".into(),
                AgentState::Working,
                None,
                crate::agent_resume::AgentSessionRef::path(test_session_path("first.jsonl")),
                Some(20),
                Instant::now(),
            )
            .expect("initial hook should be accepted");
        terminal.set_agent_name("reviewer".into());
        terminal
            .clear_hook_authority_with_mutation(Some("herdr:pi"), Some(21))
            .expect("hook clear should be accepted");
        assert_eq!(terminal.agent_name.as_deref(), Some("reviewer"));

        terminal
            .set_hook_authority_at(
                "herdr:pi".into(),
                "pi".into(),
                AgentState::Idle,
                None,
                crate::agent_resume::AgentSessionRef::path(test_session_path("second.jsonl")),
                Some(22),
                Instant::now(),
            )
            .expect("replacement hook should be accepted");

        assert!(terminal.agent_name.is_none());
        assert_eq!(terminal.effective_known_agent(), Some(Agent::Pi));
    }

    #[test]
    fn launch_command_alone_does_not_make_a_terminal_an_agent() {
        let terminal = test_terminal().with_launch_argv(vec!["just".into(), "dev".into()]);

        assert!(!terminal.is_agent_terminal());
    }

    #[test]
    fn release_agent_clears_matching_restored_session_ref_before_detection() {
        let mut terminal = test_terminal();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:hermes".into(),
            agent: "hermes".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("hermes-session").unwrap(),
        });

        let mutation = terminal
            .release_agent_with_mutation("herdr:hermes", "hermes", Some(21))
            .expect("accepted release");

        assert!(mutation.session_ref_changed);
        assert!(mutation.effective_state_change.is_none());
        assert!(terminal.persisted_agent_session.is_none());
    }

    #[test]
    fn release_agent_preserves_foreign_persisted_session_ref() {
        let mut terminal = test_terminal();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("claude-session").unwrap(),
        });
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);

        let mutation = terminal
            .release_agent_with_mutation("herdr:pi", "pi", Some(21))
            .expect("visible agent release should be accepted");

        assert!(!mutation.session_ref_changed);
        assert_eq!(
            terminal.persisted_agent_session.as_ref().map(|session| (
                session.source.as_str(),
                session.agent.as_str(),
                session.session_ref.value.as_str()
            )),
            Some(("herdr:claude", "claude", "claude-session"))
        );
    }

    #[test]
    fn process_exit_clears_matching_persisted_session_ref() {
        let mut terminal = test_terminal();
        let session_ref =
            crate::agent_resume::AgentSessionRef::path(test_session_path("pi.jsonl")).unwrap();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            session_ref: session_ref.clone(),
        });
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Working);

        let mutation = terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            false,
            false,
            true,
            std::time::Instant::now(),
        );

        assert!(mutation.session_ref_changed);
        assert!(terminal.persisted_agent_session.is_none());

        let delayed = terminal.set_agent_session_ref(
            "herdr:pi".into(),
            "pi".into(),
            Some(session_ref),
            Some(21),
        );
        assert!(delayed.is_none());
        assert!(terminal.persisted_agent_session.is_none());
    }

    #[test]
    fn process_exit_preserves_foreign_persisted_session_ref() {
        let mut terminal = test_terminal();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("claude-session").unwrap(),
        });
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Working);

        let mutation = terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Pi),
            AgentState::Idle,
            false,
            false,
            false,
            true,
            std::time::Instant::now(),
        );

        assert!(!mutation.session_ref_changed);
        assert_eq!(
            terminal
                .persisted_agent_session
                .as_ref()
                .map(|session| session.session_ref.value.as_str()),
            Some("claude-session")
        );
    }

    #[test]
    fn respawn_cleanup_resets_restored_agent_status() {
        let mut terminal = test_terminal();
        terminal.respawn_shell_on_exit = true;
        terminal.set_agent_name("codex".into());
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:codex".into(),
            agent: "codex".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("codex-session").unwrap(),
        });
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Idle);
        terminal.set_detected_agent_process_at(Agent::Codex, Instant::now());
        terminal.set_agent_session_ref_for_session_start(
            "herdr:codex".into(),
            "codex".into(),
            crate::agent_resume::AgentSessionRef::id("codex-session-2"),
            Some(1),
            Some("startup".into()),
        );
        assert!(terminal.persisted_agent_session_reported);
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(10),
            known_options(&["--placeholder"]),
            None,
        );
        terminal.agent_launch_restore = Some(crate::agent_resume::LaunchOptionsRestore::Kept);

        terminal.clear_agent_runtime_identity_after_respawn();

        assert_eq!(terminal.state, AgentState::Unknown);
        assert!(terminal.detected_agent.is_none());
        assert!(terminal.agent_name.is_none());
        assert!(terminal.persisted_agent_session.is_none());
        assert!(!terminal.respawn_shell_on_exit);
        assert!(!terminal.finish_agent_process_acquisition());
        assert!(terminal.agent_launch().is_none());
        assert!(terminal.agent_launch_restore.is_none());
        assert!(!terminal.persisted_agent_session_reported);
    }

    #[test]
    fn agent_process_exit_tracks_recent_respawn_window() {
        let mut terminal = test_terminal();
        let now = std::time::Instant::now();

        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::OpenCode),
            AgentState::Idle,
            false,
            false,
            false,
            true,
            now,
        );

        assert!(terminal.agent_process_exited_within(now, Duration::from_secs(2)));
        assert!(!terminal
            .agent_process_exited_within(now + Duration::from_secs(3), Duration::from_secs(2)));

        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::OpenCode),
            AgentState::Working,
            false,
            false,
            true,
            false,
            now + Duration::from_secs(4),
        );

        assert!(!terminal
            .agent_process_exited_within(now + Duration::from_secs(4), Duration::from_secs(2)));
    }

    #[test]
    fn detected_conflict_clears_live_hook_but_preserves_session_ref() {
        let mut terminal = test_terminal();
        terminal.set_hook_authority_with_session_ref(
            "herdr:claude".into(),
            "claude".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::id("claude-session"),
            Some(20),
        );

        let mutation =
            terminal.set_detected_state_with_mutation(Some(Agent::Grok), AgentState::Idle);

        assert!(!mutation.session_ref_changed);
        assert!(terminal.hook_authority.is_none());
        assert_eq!(
            terminal.persisted_agent_session.as_ref().map(|session| (
                session.source.as_str(),
                session.agent.as_str(),
                session.session_ref.value.as_str()
            )),
            Some(("herdr:claude", "claude", "claude-session"))
        );
    }

    #[test]
    fn detected_agent_disappearance_does_not_clear_full_lifecycle_hook_session_ref() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Kimi), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Kimi,
            "herdr:kimi",
            "kimi",
            crate::agent_resume::AgentSessionRef::id("kimi-session").unwrap(),
        );
        terminal.set_hook_authority_with_session_ref(
            "herdr:kimi".into(),
            "kimi".into(),
            AgentState::Working,
            None,
            crate::agent_resume::AgentSessionRef::id("kimi-session"),
            Some(20),
        );

        let mutation = terminal.set_detected_state_with_mutation(None, AgentState::Unknown);

        assert!(!mutation.session_ref_changed);
        assert!(terminal.hook_authority.is_some());
        assert!(terminal.persisted_agent_session.is_none());
        assert_eq!(terminal.effective_agent_label(), Some("kimi"));
    }

    #[test]
    fn detected_agent_disappearance_preserves_matching_persisted_session_ref() {
        let mut terminal = test_terminal();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:opencode".into(),
            agent: "opencode".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("opencode-session").unwrap(),
        });

        let first =
            terminal.set_detected_state_with_mutation(Some(Agent::OpenCode), AgentState::Idle);
        assert!(!first.session_ref_changed);
        assert!(terminal.persisted_agent_session.is_some());

        let second = terminal.set_detected_state_with_mutation(None, AgentState::Unknown);
        assert!(!second.session_ref_changed);
        assert!(terminal.persisted_agent_session.is_some());
    }

    #[test]
    fn initial_unknown_detection_preserves_restored_session_ref() {
        let mut terminal = test_terminal();
        terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
            source: "herdr:hermes".into(),
            agent: "hermes".into(),
            session_ref: crate::agent_resume::AgentSessionRef::id("hermes-session").unwrap(),
        });

        let mutation = terminal.set_detected_state_with_mutation(None, AgentState::Unknown);
        assert!(!mutation.session_ref_changed);
        assert!(terminal.persisted_agent_session.is_some());
    }

    #[test]
    fn unsequenced_hook_report_is_ignored_after_source_uses_sequence() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(test_session_path("root.jsonl")).unwrap(),
        );
        terminal.set_hook_authority(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            Some(20),
        );

        let change = terminal.set_hook_authority(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Idle,
            None,
            None,
        );

        assert!(change.is_none());
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn stale_clear_all_sequence_is_checked_against_current_authority_source() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        anchor_full_lifecycle_session(
            &mut terminal,
            Agent::Pi,
            "herdr:pi",
            "pi",
            crate::agent_resume::AgentSessionRef::path(test_session_path("root.jsonl")).unwrap(),
        );
        terminal.set_hook_authority(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            Some(20),
        );

        let change = terminal.clear_hook_authority(None, Some(19));

        assert!(change.is_none());
        assert_eq!(terminal.state, AgentState::Working);
        assert!(terminal.hook_authority.is_some());
    }

    #[test]
    fn same_sequence_from_different_sources_is_independent() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        terminal.set_hook_authority(
            "herdr:pi".into(),
            "pi".into(),
            AgentState::Working,
            None,
            Some(20),
        );

        terminal.set_hook_authority(
            "custom:pi".into(),
            "pi".into(),
            AgentState::Idle,
            None,
            Some(19),
        );

        assert_eq!(terminal.state, AgentState::Idle);
        assert_eq!(
            terminal.hook_authority.as_ref().unwrap().source,
            "custom:pi"
        );
    }

    /// The report evidence of a pane whose foreground job runs these Claude or
    /// Codex processes.
    fn foreground_agents(process_ids: &[u32]) -> SessionReportEvidence {
        SessionReportEvidence::ForegroundAgents(Some(
            process_ids
                .iter()
                .map(|&pid| ProcessInstance::unknown_start(pid))
                .collect(),
        ))
    }

    /// The agent's own hook reports conversation `id` at session start.
    fn report_session(terminal: &mut TerminalState, agent: &str, id: &str, seq: u64, start: &str) {
        terminal.set_agent_session_ref_for_session_start(
            format!("herdr:{agent}"),
            agent.into(),
            crate::agent_resume::AgentSessionRef::id(id),
            Some(seq),
            Some(start.into()),
        );
    }

    /// A terminal whose `agent` process 10 runs conversation `<agent>-a`,
    /// launched with `--placeholder-a`, paired by its report.
    fn agent_running_its_reported_conversation(agent: Agent) -> TerminalState {
        let label = crate::detect::agent_label(agent);
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(agent), AgentState::Idle);
        report_session(&mut terminal, label, &format!("{label}-a"), 1, "startup");
        terminal.observe_agent_launch(
            agent,
            ProcessInstance::unknown_start(10),
            known_options(&["--placeholder-a"]),
            None,
        );
        terminal.sync_agent_launch_session(&foreground_agents(&[10]));
        assert_eq!(
            terminal.launch_options_for_session(label),
            Some(vec!["--placeholder-a".into()])
        );
        terminal
    }

    /// Conversation `id` is reported, by agent process `reporter` when Herdr
    /// knows it, while the pane's foreground job runs `job`.
    fn report_from(
        terminal: &mut TerminalState,
        agent: &str,
        id: &str,
        (seq, start): (u64, &str),
        reporter: Option<u32>,
        job: &[(u32, &str)],
    ) {
        let evidence = match reporter {
            Some(reporter) => {
                SessionReportEvidence::Reporter(vec![ProcessInstance::unknown_start(reporter)])
            }
            None => foreground_agents(
                &job.iter()
                    .map(|(process_id, _)| *process_id)
                    .collect::<Vec<_>>(),
            ),
        };
        report_with(terminal, agent, id, (seq, start), &evidence);
    }

    /// Conversation `id` is reported with this evidence of its sender.
    fn report_with(
        terminal: &mut TerminalState,
        agent: &str,
        id: &str,
        (seq, start): (u64, &str),
        evidence: &SessionReportEvidence,
    ) {
        report_session(terminal, agent, id, seq, start);
        terminal.sync_agent_launch_session(evidence);
    }

    #[test]
    fn a_started_seed_pairs_only_beside_one_agent_in_the_job() {
        for (case, job, pairs) in [
            (
                "job unreadable",
                SessionReportEvidence::ForegroundAgents(None),
                false,
            ),
            ("two agents in the job", foreground_agents(&[10, 12]), false),
            ("one agent in the job", foreground_agents(&[10]), true),
        ] {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
            // `herdr agent start` typed the launch; detection has not read
            // its process yet when a conversation is reported over a
            // connection the server cannot read.
            terminal.seed_agent_launch(AgentLaunchSeed {
                agent: Agent::Claude,
                options: known_options(&["--placeholder-a"]),
                resumed_without_options: false,
                launch: SeededLaunch::Started { session: None },
            });
            report_with(&mut terminal, "claude", "claude-b", (1, "startup"), &job);
            assert_eq!(
                terminal.launch_options_for_session("claude"),
                pairs.then(|| vec!["--placeholder-a".to_string()]),
                "{case}"
            );
        }
    }

    #[test]
    fn a_reused_process_id_in_the_job_does_not_confirm_the_bound_record() {
        let bound = ProcessInstance {
            pid: 10,
            started: Some(100),
        };
        for (in_job, pairs) in [(Some(200), false), (Some(100), true)] {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
            terminal.observe_agent_launch(
                Agent::Claude,
                bound,
                known_options(&["--placeholder-a"]),
                None,
            );
            // Over a connection the server cannot read, a conversation is
            // reported while the job runs a process with the bound id; one
            // with another start time is a later process that reused it.
            report_with(
                &mut terminal,
                "claude",
                "claude-b",
                (1, "startup"),
                &SessionReportEvidence::ForegroundAgents(Some(vec![ProcessInstance {
                    pid: 10,
                    started: in_job,
                }])),
            );
            assert_eq!(
                terminal.launch_options_for_session("claude"),
                pairs.then(|| vec!["--placeholder-a".to_string()]),
                "started {in_job:?}"
            );
        }
    }

    #[test]
    fn a_reused_reporter_process_id_is_not_confirmed() {
        let reporter = ProcessInstance {
            pid: 20,
            started: Some(100),
        };
        for (later, confirmed) in [(Some(200), false), (Some(100), true), (None, true)] {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
            report_with(
                &mut terminal,
                "claude",
                "claude-b",
                (1, "startup"),
                &SessionReportEvidence::Reporter(vec![reporter]),
            );
            // Process 20 exits unobserved, and a later Claude that starts a
            // conversation of its own gets the same process id, with another
            // start time; the same start time, or none read, is the reporter.
            terminal.observe_agent_launch(
                Agent::Claude,
                ProcessInstance {
                    pid: 20,
                    started: later,
                },
                known_options(&["--placeholder-x"]),
                None,
            );
            assert_eq!(
                terminal.launch_options_for_session("claude"),
                confirmed.then(|| vec!["--placeholder-x".to_string()]),
                "started {later:?}"
            );
        }
    }

    #[test]
    fn a_bound_process_id_reused_by_a_reporter_is_not_confirmed() {
        let bound = ProcessInstance {
            pid: 10,
            started: Some(100),
        };
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        terminal.observe_agent_launch(
            Agent::Claude,
            bound,
            known_options(&["--placeholder-a"]),
            None,
        );
        report_with(
            &mut terminal,
            "claude",
            "claude-a",
            (1, "startup"),
            &SessionReportEvidence::Reporter(vec![bound]),
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-a".into()])
        );
        // A process with the bound process's id but another start time
        // reports a conversation change.
        report_with(
            &mut terminal,
            "claude",
            "claude-b",
            (2, "clear"),
            &SessionReportEvidence::Reporter(vec![ProcessInstance {
                pid: 10,
                started: Some(200),
            }]),
        );
        assert_eq!(terminal.launch_options_for_session("claude"), None);
    }

    #[test]
    fn a_handed_off_pairing_is_not_kept_by_a_process_that_reused_its_id() {
        let handed_off = ProcessInstance {
            pid: 4242,
            started: Some(100),
        };
        for (observed, kept) in [(Some(200), false), (Some(100), true)] {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
            report_session(&mut terminal, "claude", "claude-a", 1, "startup");
            terminal.seed_agent_launch(AgentLaunchSeed {
                agent: Agent::Claude,
                options: known_options(&["--placeholder-a"]),
                resumed_without_options: false,
                launch: SeededLaunch::Paired {
                    process: handed_off,
                    session: crate::agent_resume::AgentSessionRef::id("claude-a").unwrap(),
                },
            });
            // Detection reads process 4242 after the handoff, started fresh.
            terminal.observe_agent_launch(
                Agent::Claude,
                ProcessInstance {
                    pid: 4242,
                    started: observed,
                },
                known_options(&["--placeholder-x"]),
                None,
            );
            assert_eq!(
                terminal.launch_options_for_session("claude"),
                kept.then(|| vec!["--placeholder-a".to_string()]),
                "started {observed:?}"
            );
        }
    }

    #[test]
    fn a_report_whose_walk_failed_is_not_paired_with_the_job() {
        let mut terminal = agent_running_its_reported_conversation(Agent::Claude);
        // A Claude nested through the parent's Bash tool runs in its own
        // session, so the job shows only the parent, and its hook exited
        // before the server read the chain up to it.
        report_with(
            &mut terminal,
            "claude",
            "claude-c",
            (2, "resume"),
            &SessionReportEvidence::Unidentified,
        );
        assert_eq!(terminal.launch_options_for_session("claude"), None);
        // The report stayed undecided: the parent itself reporting it again
        // pairs it.
        report_from(
            &mut terminal,
            "claude",
            "claude-c",
            (3, "compact"),
            None,
            &[(10, "claude")],
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-a".into()])
        );

        // Reported first, the conversation is not taken by the next process.
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        report_with(
            &mut terminal,
            "claude",
            "claude-c",
            (1, "startup"),
            &SessionReportEvidence::Unidentified,
        );
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(10),
            known_options(&["--placeholder-a"]),
            None,
        );
        assert_eq!(terminal.launch_options_for_session("claude"), None);
    }

    #[test]
    fn a_conversation_reported_from_outside_the_pane_is_not_paired_with_the_job() {
        let mut terminal = agent_running_its_reported_conversation(Agent::Codex);
        // A Codex process outside the pane, or one whose walk was cut before
        // the pane, sent the report while the pane's job runs its one Codex.
        report_with(
            &mut terminal,
            "codex",
            "codex-b",
            (2, "clear"),
            &SessionReportEvidence::Unidentified,
        );
        assert_eq!(terminal.launch_options_for_session("codex"), None);
        // The report stayed undecided: repeated over a connection the server
        // cannot read, the foreground job running its one Codex pairs it.
        report_from(
            &mut terminal,
            "codex",
            "codex-b",
            (3, "compact"),
            None,
            &[(10, "codex")],
        );
        assert_eq!(
            terminal.launch_options_for_session("codex"),
            Some(vec!["--placeholder-a".into()])
        );
    }

    #[test]
    fn a_nested_claude_report_is_not_paired_with_its_parent() {
        let mut terminal = agent_running_its_reported_conversation(Agent::Claude);
        // The parent's Bash tool runs `claude --resume claude-c` in its own
        // process group, so the pane's foreground job shows only the parent.
        report_from(
            &mut terminal,
            "claude",
            "claude-c",
            (2, "resume"),
            Some(20),
            &[(10, "claude")],
        );
        assert_eq!(terminal.launch_options_for_session("claude"), None);
    }

    #[test]
    fn a_report_from_a_child_in_the_job_is_not_paired_with_the_parent_in_any_order() {
        let parent_and_child = [(10, "claude"), (12, "claude")];
        for (reporter, seeded, report_first) in [
            (None, false, true),
            (Some(12), false, true),
            (None, false, false),
            (Some(12), false, false),
            (None, true, true),
            (Some(12), true, true),
        ] {
            let case =
                format!("reporter {reporter:?}, seeded {seeded}, report first {report_first}");
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
            if seeded {
                // `herdr agent start` launched the parent.
                terminal.seed_agent_launch(AgentLaunchSeed {
                    agent: Agent::Claude,
                    options: known_options(&["--placeholder-a"]),
                    resumed_without_options: false,
                    launch: SeededLaunch::Started { session: None },
                });
            }
            let observe_parent = |terminal: &mut TerminalState| {
                terminal.observe_agent_launch(
                    Agent::Claude,
                    ProcessInstance::unknown_start(10),
                    known_options(&["--placeholder-a"]),
                    None,
                )
            };
            if !report_first {
                observe_parent(&mut terminal);
            }
            report_from(
                &mut terminal,
                "claude",
                "claude-c",
                (1, "startup"),
                reporter,
                &parent_and_child,
            );
            if report_first {
                observe_parent(&mut terminal);
            }
            assert_eq!(
                terminal.launch_options_for_session("claude"),
                None,
                "{case}"
            );
        }
    }

    #[test]
    fn a_first_observation_does_not_take_a_conversation_reported_beside_another_agent() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Idle);
        // Two Codex processes share the job when one of them reports.
        report_from(
            &mut terminal,
            "codex",
            "codex-b",
            (1, "startup"),
            None,
            &[(10, "codex"), (11, "codex")],
        );
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(10),
            known_options(&["--placeholder-a"]),
            None,
        );
        assert_eq!(terminal.launch_options_for_session("codex"), None);
    }

    #[test]
    fn a_conversation_whose_reporter_exited_unobserved_is_not_taken_by_the_next_process() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        report_from(
            &mut terminal,
            "claude",
            "claude-b",
            (1, "startup"),
            Some(20),
            &[(20, "claude")],
        );
        // Process 20 exits before detection reads it; the next one starts a
        // new conversation of its own.
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(30),
            known_options(&["--placeholder-x"]),
            None,
        );
        assert_eq!(terminal.launch_options_for_session("claude"), None);
    }

    #[test]
    fn a_report_dropped_beside_another_agent_is_decided_again_when_repeated() {
        let mut terminal = agent_running_its_reported_conversation(Agent::Claude);
        // A second Claude shares the job while the first clears to claude-b,
        // and Herdr cannot tell which one reported it.
        report_from(
            &mut terminal,
            "claude",
            "claude-b",
            (2, "clear"),
            None,
            &[(10, "claude"), (12, "claude")],
        );
        assert_eq!(terminal.launch_options_for_session("claude"), None);
        // The second one exits, and the first reports claude-b again.
        report_from(
            &mut terminal,
            "claude",
            "claude-b",
            (3, "compact"),
            None,
            &[(10, "claude")],
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-a".into()])
        );

        // Observed first, claude-b is reported beside another lone Claude,
        // which exits, and then beside this process alone.
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(10),
            known_options(&["--placeholder-a"]),
            None,
        );
        report_from(
            &mut terminal,
            "claude",
            "claude-b",
            (1, "startup"),
            None,
            &[(12, "claude")],
        );
        assert_eq!(terminal.launch_options_for_session("claude"), None);
        report_from(
            &mut terminal,
            "claude",
            "claude-b",
            (2, "resume"),
            None,
            &[(10, "claude")],
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-a".into()])
        );
    }

    #[test]
    fn a_process_runs_the_conversation_it_was_last_shown_to_run() {
        let observe = |terminal: &mut TerminalState, process_id, option, resumed| {
            terminal.observe_agent_launch(
                Agent::Claude,
                ProcessInstance::unknown_start(process_id),
                known_options(&[option]),
                resumed,
            );
        };
        let setup = || {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
            terminal
        };

        // A report no one can place does not undo the earlier report that
        // named process 10.
        let mut terminal = setup();
        report_from(
            &mut terminal,
            "claude",
            "claude-s",
            (1, "startup"),
            Some(10),
            &[],
        );
        report_from(
            &mut terminal,
            "claude",
            "claude-s",
            (2, "compact"),
            None,
            &[],
        );
        observe(&mut terminal, 10, "--placeholder-a", None);
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-a".into()]),
            "an undecided report"
        );

        // Process 10 was named for claude-s, which is newer than its command
        // line resuming claude-t, so a report of claude-t no one can place
        // is not paired with it.
        let mut terminal = setup();
        report_from(
            &mut terminal,
            "claude",
            "claude-s",
            (1, "startup"),
            None,
            &[(10, "claude")],
        );
        observe(&mut terminal, 10, "--placeholder-a", Some("claude-t"));
        report_from(
            &mut terminal,
            "claude",
            "claude-t",
            (2, "resume"),
            None,
            &[],
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            None,
            "a command line older than the report"
        );

        // Process 12 resumed claude-s after a report named process 10 for
        // it, so 10 no longer runs it.
        let mut terminal = setup();
        report_from(
            &mut terminal,
            "claude",
            "claude-s",
            (1, "startup"),
            None,
            &[(10, "claude")],
        );
        observe(&mut terminal, 12, "--placeholder-b", Some("claude-s"));
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-b".into()])
        );
        observe(&mut terminal, 10, "--placeholder-a", Some("claude-s"));
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            None,
            "a newer process resuming it"
        );
    }

    #[test]
    fn a_report_first_beside_two_agents_is_decided_again_after_observation() {
        let mut terminal = test_terminal();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        // Claude-b is reported beside two Claudes before detection reads
        // process 10, which leaves it undecided.
        report_from(
            &mut terminal,
            "claude",
            "claude-b",
            (1, "startup"),
            None,
            &[(10, "claude"), (12, "claude")],
        );
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(10),
            known_options(&["--placeholder-a"]),
            None,
        );
        assert_eq!(terminal.launch_options_for_session("claude"), None);
        // The second one exits, and claude-b is reported again beside 10
        // alone.
        report_from(
            &mut terminal,
            "claude",
            "claude-b",
            (2, "compact"),
            None,
            &[(10, "claude")],
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-a".into()])
        );
    }

    #[test]
    fn a_report_first_beside_one_agent_is_paired_only_with_that_process() {
        let in_job = ProcessInstance {
            pid: 10,
            started: Some(100),
        };
        for (case, observed, pairs) in [
            ("another process", ProcessInstance::unknown_start(12), false),
            (
                "the job's id reused later",
                ProcessInstance {
                    pid: 10,
                    started: Some(200),
                },
                false,
            ),
            ("the job's process", in_job, true),
        ] {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
            report_with(
                &mut terminal,
                "claude",
                "claude-b",
                (1, "startup"),
                &SessionReportEvidence::ForegroundAgents(Some(vec![in_job])),
            );
            terminal.observe_agent_launch(
                Agent::Claude,
                observed,
                known_options(&["--placeholder-a"]),
                None,
            );
            assert_eq!(
                terminal.launch_options_for_session("claude"),
                pairs.then(|| vec!["--placeholder-a".to_string()]),
                "{case}"
            );
        }
    }

    #[test]
    fn a_seed_paired_beside_one_agent_binds_only_to_that_process() {
        let in_job = ProcessInstance {
            pid: 10,
            started: Some(100),
        };
        for (case, observed, pairs) in [
            ("another process", ProcessInstance::unknown_start(12), false),
            (
                "the job's id reused later",
                ProcessInstance {
                    pid: 10,
                    started: Some(200),
                },
                false,
            ),
            ("the job's process", in_job, true),
        ] {
            let mut terminal = test_terminal();
            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
            // `herdr agent start` typed the launch; its conversation is
            // reported beside one Claude before detection reads a process.
            terminal.seed_agent_launch(AgentLaunchSeed {
                agent: Agent::Claude,
                options: known_options(&["--placeholder-a"]),
                resumed_without_options: false,
                launch: SeededLaunch::Started { session: None },
            });
            report_with(
                &mut terminal,
                "claude",
                "claude-b",
                (1, "startup"),
                &SessionReportEvidence::ForegroundAgents(Some(vec![in_job])),
            );
            terminal.observe_agent_launch(
                Agent::Claude,
                observed,
                known_options(&["--placeholder-x"]),
                None,
            );
            assert_eq!(
                terminal.launch_options_for_session("claude").is_some(),
                pairs,
                "{case}"
            );
        }
    }

    #[test]
    fn a_conversation_change_reported_by_the_bound_process_keeps_its_options() {
        let mut terminal = agent_running_its_reported_conversation(Agent::Claude);
        // `/clear` in the agent, with a nested Claude in the job.
        report_from(
            &mut terminal,
            "claude",
            "claude-b",
            (2, "clear"),
            Some(10),
            &[(10, "claude"), (12, "claude")],
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-a".into()])
        );
    }

    #[test]
    fn a_report_from_a_second_codex_beside_a_suspended_one_is_not_paired_with_it() {
        let mut terminal = agent_running_its_reported_conversation(Agent::Codex);
        // Ctrl-Z stops the first Codex, and a second one starts in the
        // foreground and reports its new conversation.
        report_session(&mut terminal, "codex", "codex-b", 2, "startup");
        terminal.sync_agent_launch_session(&foreground_agents(&[11]));
        assert_eq!(terminal.launch_options_for_session("codex"), None);
        // Detection reads the second Codex, which takes its conversation.
        terminal.observe_agent_launch(
            Agent::Codex,
            ProcessInstance::unknown_start(11),
            known_options(&["--placeholder-b"]),
            None,
        );
        assert_eq!(
            terminal.launch_options_for_session("codex"),
            Some(vec!["--placeholder-b".into()])
        );
    }

    #[test]
    fn a_report_from_a_claude_resumed_beside_a_suspended_one_is_not_paired_with_it() {
        let mut terminal = agent_running_its_reported_conversation(Agent::Claude);
        // `claude --resume claude-c` runs in the foreground while the first
        // Claude is stopped.
        report_session(&mut terminal, "claude", "claude-c", 2, "resume");
        terminal.sync_agent_launch_session(&foreground_agents(&[11]));
        assert_eq!(terminal.launch_options_for_session("claude"), None);
        terminal.observe_agent_launch(
            Agent::Claude,
            ProcessInstance::unknown_start(11),
            known_options(&["--placeholder-b"]),
            Some("claude-c"),
        );
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-b".into()])
        );
    }

    #[test]
    fn a_report_from_a_child_agent_in_the_same_job_is_not_paired_with_its_parent() {
        let mut terminal = agent_running_its_reported_conversation(Agent::Claude);
        // The first Claude runs another Claude in its own process group, and
        // that one resumes another conversation.
        report_session(&mut terminal, "claude", "claude-c", 2, "resume");
        terminal.sync_agent_launch_session(&foreground_agents(&[10, 12]));
        assert_eq!(terminal.launch_options_for_session("claude"), None);
    }

    #[test]
    fn a_conversation_change_inside_the_only_agent_of_the_job_keeps_its_options() {
        let mut terminal = agent_running_its_reported_conversation(Agent::Claude);
        // `/clear`, with a tool the agent runs in the same job, which is no
        // Claude or Codex process.
        report_session(&mut terminal, "claude", "claude-b", 2, "clear");
        terminal.sync_agent_launch_session(&foreground_agents(&[10]));
        assert_eq!(
            terminal.launch_options_for_session("claude"),
            Some(vec!["--placeholder-a".into()])
        );
    }

    /// Every short sequence of launch, report and exit events, checked after
    /// each step against a model of which process runs which conversation.
    mod exhaustive_pairing {
        use super::*;

        const MAX_LENGTH: usize = 4;

        /// P and Q are agent processes; `P2` reuses P's id after P exited,
        /// with another start time.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Process {
            P,
            Q,
            P2,
        }

        const PROCESSES: [Process; 3] = [Process::P, Process::Q, Process::P2];

        impl Process {
            fn index(self) -> usize {
                self as usize
            }

            fn instance(self) -> ProcessInstance {
                match self {
                    Self::P => ProcessInstance {
                        pid: 10,
                        started: Some(1),
                    },
                    Self::Q => ProcessInstance {
                        pid: 11,
                        started: Some(2),
                    },
                    Self::P2 => ProcessInstance {
                        pid: 10,
                        started: Some(3),
                    },
                }
            }

            fn options(self) -> Vec<String> {
                vec![format!("--placeholder-{self:?}").to_lowercase()]
            }
        }

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Conversation {
            S,
            T,
        }

        impl Conversation {
            fn id(self) -> &'static str {
                match self {
                    Self::S => "conversation-s",
                    Self::T => "conversation-t",
                }
            }

            fn session(self) -> crate::agent_resume::AgentSessionRef {
                crate::agent_resume::AgentSessionRef::id(self.id()).unwrap()
            }
        }

        /// What detection reads from a process's command line.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Reading {
            Resumes(Conversation),
            Nothing,
            Unrecognized,
        }

        /// What the server knows about a report's sender.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Sender {
            Reporter(Process),
            Unplaced,
            Job(&'static [Process]),
            JobUnreadable,
        }

        impl Sender {
            fn evidence(self) -> SessionReportEvidence {
                match self {
                    Self::Reporter(process) => {
                        SessionReportEvidence::Reporter(vec![process.instance()])
                    }
                    Self::Unplaced => SessionReportEvidence::Unidentified,
                    Self::Job(processes) => SessionReportEvidence::ForegroundAgents(Some(
                        processes.iter().map(|process| process.instance()).collect(),
                    )),
                    Self::JobUnreadable => SessionReportEvidence::ForegroundAgents(None),
                }
            }

            fn processes(self) -> &'static [Process] {
                match self {
                    Self::Reporter(Process::P) | Self::Job(&[Process::P]) => &[Process::P],
                    Self::Reporter(Process::Q) | Self::Job(&[Process::Q]) => &[Process::Q],
                    Self::Reporter(Process::P2) => &[Process::P2],
                    Self::Job(processes) => processes,
                    Self::Unplaced | Self::JobUnreadable => &[],
                }
            }

            /// The one process this evidence names, by its kind.
            fn named(self) -> Option<(Process, FactSource)> {
                match self {
                    Self::Reporter(process) => Some((process, FactSource::Reporter)),
                    Self::Job(&[process]) => Some((process, FactSource::Job)),
                    _ => None,
                }
            }
        }

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Event {
            /// A restore typed `claude --resume S` with saved options.
            Seed,
            Observe(Process, Reading),
            /// A report of a conversation; `false` when its sequence is stale.
            Report(Conversation, Sender, bool),
            /// The process detection last read exits.
            Exit,
        }

        fn alphabet() -> Vec<Event> {
            let mut events = vec![Event::Seed, Event::Exit];
            for process in PROCESSES {
                for reading in [
                    Reading::Resumes(Conversation::S),
                    Reading::Resumes(Conversation::T),
                    Reading::Nothing,
                    Reading::Unrecognized,
                ] {
                    events.push(Event::Observe(process, reading));
                }
            }
            for conversation in [Conversation::S, Conversation::T] {
                for sender in [
                    Sender::Reporter(Process::P),
                    Sender::Reporter(Process::Q),
                    Sender::Reporter(Process::P2),
                    Sender::Unplaced,
                    Sender::Job(&[Process::P]),
                    Sender::Job(&[Process::Q]),
                    Sender::Job(&[Process::P, Process::Q]),
                    Sender::Job(&[]),
                    Sender::JobUnreadable,
                ] {
                    for accepted in [true, false] {
                        events.push(Event::Report(conversation, sender, accepted));
                    }
                }
            }
            events
        }

        const SEED_OPTIONS: &str = "--placeholder-seed";

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum FactSource {
            CommandLine,
            Reporter,
            Job,
        }

        /// What is true, from the events alone.
        #[derive(Debug, Clone, Default)]
        struct Model {
            time: u32,
            appeared: [bool; 3],
            exited: [bool; 3],
            /// The options of each process's latest reading; `Some(None)`
            /// when its command line was not recognized.
            observed: [Option<Option<Vec<String>>>; 3],
            readings: [Option<Reading>; 3],
            /// The conversation each process runs, when it was learned and
            /// how.
            runs: [Option<(Conversation, u32, FactSource)>; 3],
            bound: Option<Process>,
            seeded: bool,
            observed_since_seed: bool,
            sequence: u64,
            accepted_since_exit: bool,
        }

        impl Model {
            fn alive(&self, process: Process) -> bool {
                !self.exited[process.index()]
            }

            /// Whether `event` can happen next.
            fn allows(&self, event: Event) -> bool {
                let mentioned: &[Process] = match event {
                    Event::Seed => return self.time == 0,
                    Event::Exit => return self.bound.is_some(),
                    Event::Observe(process, reading) => {
                        // A process's command line does not change, except
                        // when a launcher detection could not place execs the
                        // agent in its own process.
                        if self.readings[process.index()]
                            .is_some_and(|read| read != Reading::Unrecognized || reading == read)
                        {
                            return false;
                        }
                        match process {
                            Process::P => &[Process::P],
                            Process::Q => &[Process::Q],
                            Process::P2 => &[Process::P2],
                        }
                    }
                    Event::Report(_, sender, accepted) => {
                        if !accepted && !self.accepted_since_exit {
                            return false;
                        }
                        sender.processes()
                    }
                };
                // P cannot come back once another process took its id.
                mentioned.iter().all(|process| {
                    self.alive(*process)
                        && (*process != Process::P || !self.appeared[Process::P2.index()])
                })
            }

            fn appear(&mut self, process: Process) {
                self.appeared[process.index()] = true;
            }

            fn apply(&mut self, event: Event) {
                self.time += 1;
                match event {
                    Event::Seed => self.seeded = true,
                    Event::Observe(process, reading) => {
                        self.appear(process);
                        self.observed_since_seed = true;
                        self.bound = Some(process);
                        self.readings[process.index()] = Some(reading);
                        self.observed[process.index()] = Some(match reading {
                            Reading::Unrecognized => None,
                            _ => Some(process.options()),
                        });
                        // A command line precedes every report its process
                        // sends, so a report naming it stays the latest fact.
                        let reported = self.runs[process.index()]
                            .is_some_and(|(_, _, source)| source != FactSource::CommandLine);
                        if !reported {
                            self.runs[process.index()] = match reading {
                                Reading::Resumes(conversation) => {
                                    Some((conversation, self.time, FactSource::CommandLine))
                                }
                                Reading::Nothing | Reading::Unrecognized => None,
                            };
                        }
                    }
                    Event::Report(conversation, sender, accepted) => {
                        for process in sender.processes() {
                            self.appear(*process);
                        }
                        if accepted {
                            self.sequence += 1;
                            self.accepted_since_exit = true;
                            if let Some((process, source)) = sender.named() {
                                self.runs[process.index()] =
                                    Some((conversation, self.time, source));
                            }
                        }
                    }
                    Event::Exit => {
                        if let Some(process) = self.bound.take() {
                            self.exited[process.index()] = true;
                            self.runs[process.index()] = None;
                        }
                        self.accepted_since_exit = false;
                    }
                }
            }

            /// The live process that runs `conversation` now: the latest fact
            /// about it wins.
            fn runner(&self, conversation: Conversation) -> Option<(Process, FactSource)> {
                PROCESSES
                    .into_iter()
                    .filter(|process| self.alive(*process))
                    .filter_map(|process| {
                        self.runs[process.index()]
                            .filter(|(runs, _, _)| *runs == conversation)
                            .map(|(_, time, source)| (time, process, source))
                    })
                    .max_by_key(|(time, _, _)| *time)
                    .map(|(_, process, source)| (process, source))
            }
        }

        /// Drives the terminal as the server does for each event.
        fn drive(terminal: &mut TerminalState, model: &Model, event: Event) -> Result<(), String> {
            match event {
                Event::Seed => {
                    terminal.set_persisted_agent_session(
                        crate::agent_resume::PersistedAgentSession {
                            source: "herdr:claude".into(),
                            agent: "claude".into(),
                            session_ref: Conversation::S.session(),
                        },
                    );
                    terminal.seed_agent_launch(AgentLaunchSeed {
                        agent: Agent::Claude,
                        options: known_options(&[SEED_OPTIONS]),
                        resumed_without_options: false,
                        launch: SeededLaunch::Started {
                            session: Some(Conversation::S.session()),
                        },
                    });
                }
                Event::Observe(process, reading) => {
                    terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
                    let (options, resumed) = match reading {
                        Reading::Resumes(conversation) => (
                            crate::agent_resume::AgentLaunchOptions::Known(process.options()),
                            Some(conversation.id()),
                        ),
                        Reading::Nothing => (
                            crate::agent_resume::AgentLaunchOptions::Known(process.options()),
                            None,
                        ),
                        Reading::Unrecognized => (
                            crate::agent_resume::AgentLaunchOptions::Unavailable(
                                crate::agent_resume::LaunchOptionsUnavailable::UnrecognizedCommand,
                            ),
                            None,
                        ),
                    };
                    terminal.observe_agent_launch(
                        Agent::Claude,
                        process.instance(),
                        options,
                        resumed,
                    );
                }
                Event::Report(conversation, sender, accepted) => {
                    terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
                    let sequence = if accepted {
                        model.sequence + 1
                    } else {
                        model.sequence
                    };
                    let took = report_as_the_server_does(
                        terminal,
                        conversation.session(),
                        sequence,
                        &sender.evidence(),
                    );
                    if took != accepted {
                        return Err(format!("the report was accepted: {took}"));
                    }
                }
                Event::Exit => {
                    terminal.set_detected_state_with_visible_blocker(
                        Some(Agent::Claude),
                        AgentState::Idle,
                        false,
                        false,
                        true,
                    );
                }
            }
            Ok(())
        }

        /// A Claude report as the server applies it: whether the pane took it.
        fn report_as_the_server_does(
            terminal: &mut TerminalState,
            session: crate::agent_resume::AgentSessionRef,
            sequence: u64,
            evidence: &SessionReportEvidence,
        ) -> bool {
            terminal
                .apply_agent_session_report(
                    "herdr:claude".into(),
                    "claude".into(),
                    Some(session),
                    Some(sequence),
                    Some("resume".into()),
                    evidence,
                )
                .is_some()
        }

        /// Whether the options a session save writes now agree with the
        /// model: `Err` names the disagreement, and `Ok(true)` a known limit.
        fn check(terminal: &TerminalState, model: &Model) -> Result<bool, String> {
            let Some(current) = terminal.current_session_ref() else {
                return Ok(false);
            };
            let Some(conversation) = [Conversation::S, Conversation::T]
                .into_iter()
                .find(|conversation| conversation.session() == *current)
            else {
                return Ok(false);
            };
            let saved = terminal.launch_options_for_session("claude");
            let runner = model.runner(conversation);
            let runner_options =
                runner.and_then(|(process, _)| model.observed[process.index()].clone().flatten());
            if let Some(saved) = &saved {
                if runner_options.as_ref() == Some(saved) {
                    return Ok(false);
                }
                let seed_options = saved.as_slice() == [SEED_OPTIONS.to_string()];
                if seed_options && model.seeded && !model.observed_since_seed {
                    // The typed launch runs S until detection reads a
                    // process.
                    if conversation == Conversation::S && runner.is_none() {
                        return Ok(false);
                    }
                    // Known limit (PRD Assumptions, no readable sender): the
                    // lone agent in the job is taken to be the seed's launch
                    // before detection reads it.
                    if runner.is_some_and(|(_, source)| source == FactSource::Job) {
                        return Ok(true);
                    }
                }
                return Err(format!(
                    "saved {saved:?} beside {conversation:?}, which {runner:?} runs"
                ));
            }
            // A lone process that resumed the conversation itself, or that
            // its reporter named, keeps its options beside it.
            let lone = model.appeared.iter().filter(|appeared| **appeared).count() == 1;
            if let (
                true,
                Some((_, FactSource::CommandLine | FactSource::Reporter)),
                Some(options),
            ) = (lone, runner, runner_options)
            {
                return Err(format!(
                    "saved no options beside {conversation:?}, which {runner:?} runs with {options:?}"
                ));
            }
            Ok(false)
        }

        #[derive(Default)]
        struct Outcome {
            sequences: u64,
            known_limit: u64,
            failures: Vec<(Vec<Event>, String)>,
        }

        fn explore(
            terminal: &TerminalState,
            model: &Model,
            sequence: &mut Vec<Event>,
            events: &[Event],
            outcome: &mut Outcome,
        ) {
            if sequence.len() == MAX_LENGTH {
                return;
            }
            for &event in events {
                if !model.allows(event) {
                    continue;
                }
                let mut terminal = terminal.clone();
                let mut model = model.clone();
                sequence.push(event);
                outcome.sequences += 1;
                let result = drive(&mut terminal, &model, event).and_then(|()| {
                    model.apply(event);
                    check(&terminal, &model)
                });
                match result {
                    Err(failure) => outcome.failures.push((sequence.clone(), failure)),
                    Ok(known_limit) => {
                        outcome.known_limit += u64::from(known_limit);
                        explore(&terminal, &model, sequence, events, outcome);
                    }
                }
                sequence.pop();
            }
        }

        #[test]
        fn exhaustive_pairing_saves_options_only_beside_their_process_conversation() {
            let events = alphabet();
            let started = Instant::now();
            let outcomes = std::thread::scope(|scope| {
                events
                    .iter()
                    .map(|&first| {
                        let events = &events;
                        scope.spawn(move || {
                            let mut outcome = Outcome::default();
                            let mut terminal = test_terminal();
                            terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
                            let model = Model::default();
                            if model.allows(first) {
                                let mut sequence = vec![first];
                                outcome.sequences += 1;
                                let mut model = model;
                                match drive(&mut terminal, &model, first).and_then(|()| {
                                    model.apply(first);
                                    check(&terminal, &model)
                                }) {
                                    Err(failure) => {
                                        outcome.failures.push((sequence.clone(), failure))
                                    }
                                    Ok(known_limit) => {
                                        outcome.known_limit += u64::from(known_limit);
                                        explore(
                                            &terminal,
                                            &model,
                                            &mut sequence,
                                            events,
                                            &mut outcome,
                                        );
                                    }
                                }
                            }
                            outcome
                        })
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|handle| handle.join().unwrap())
                    .collect::<Vec<_>>()
            });
            let sequences: u64 = outcomes.iter().map(|outcome| outcome.sequences).sum();
            let known_limit: u64 = outcomes.iter().map(|outcome| outcome.known_limit).sum();
            let mut failures = outcomes
                .into_iter()
                .flat_map(|outcome| outcome.failures)
                .collect::<Vec<_>>();
            failures.sort_by_key(|(sequence, _)| sequence.len());
            eprintln!(
                "exhaustive pairing: {sequences} sequences up to length {MAX_LENGTH}, \
                 {known_limit} steps at a known limit, {} failing, in {:?}",
                failures.len(),
                started.elapsed()
            );
            for (sequence, failure) in failures.iter().take(40) {
                eprintln!("failing: {sequence:?}: {failure}");
            }
            assert!(failures.is_empty(), "{} failing sequences", failures.len());
        }
    }
}
