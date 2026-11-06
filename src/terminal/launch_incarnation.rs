//! Which admitted `herdr agent start` launch a terminal's agent process is.
//!
//! Attribution rule. Herdr types the launch into the pane's shell as
//! `<herdr> __attest-launch <nonce> -- <argv...>`. That helper is the process
//! the shell forks for the admitted line. Before it runs anything it sends the
//! one-use nonce to the server, which reads the sender of that connection
//! from the socket (`SO_PEERCRED` on Linux, `LOCAL_PEERPID` on macOS) with its
//! start time and parent. The server binds the launch to that process only
//! when the nonce is the terminal's pending one, the sender's start time is
//! known, and its parent is the pane's shell. The helper then `exec`s the
//! agent (or its launcher), which keeps the process id and start time, so
//! the bound process is the actual process of that admission, identified by
//! its birth identity. The nonce is spent by its first use, whether that use
//! binds or not, so a line repeated from shell history cannot claim it.
//!
//! For `agent start` the typed line lets the pane's shell resolve the
//! command first and runs the helper only for an executable file; an alias,
//! a function or a builtin makes the line end the admission instead, and the
//! shell runs the command unattested. The shell itself is never bound.
//!
//! The launch is attested once the server sees that same process (same id
//! and same known start time) running the agent: detection identifies it as
//! the terminal's agent process, or an official native report the server
//! placed in the pane names it as the reporting agent process. It stays
//! attested while no other process is seen there. Detection can lag a
//! replacement, so the token is reported, and keys guarded by it are queued,
//! only while the process table read at that moment shows the bound process
//! leading the terminal's foreground process group ([`leads_foreground`]),
//! and the terminal writer checks that again before each write it makes.
//! An observation of any other process, an observed exit, a respawn, a new
//! admission, a failed managed start or an agent of another kind ends it for
//! good: a token is never attested again once it was revoked, and a restore
//! never revives one. A live handoff carries a bound launch, which the next
//! server attests again only when it sees the same process itself.

use crate::detect::Agent;
use crate::platform::ProcessInstance;

/// The argument that makes the herdr binary run the attesting launch helper.
pub(crate) const ATTEST_LAUNCH_COMMAND: &str = "__attest-launch";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// Admitted; the helper has not claimed the launch yet.
    Admitted { nonce: String },
    /// Bound to the process that claimed the admission, or carried by a live
    /// handoff, but not yet identified by detection as the agent process.
    Claimed(ProcessInstance),
    /// Detection identified the bound process as the terminal's agent process.
    Attested(ProcessInstance),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchIncarnation {
    token: String,
    agent: Agent,
    phase: Phase,
}

/// Why a claim did not bind the launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaimRefusal {
    /// The sender's start time could not be read.
    UnknownBirth,
    /// The sender is not a direct child of the pane's shell.
    NotTheShellsChild,
}

impl LaunchIncarnation {
    /// A new admission of `agent`, with its public token and the one-use nonce
    /// the typed helper claims it with.
    pub(crate) fn admit(agent: Agent) -> (Self, String) {
        let nonce = mint("nonce");
        (
            Self {
                token: mint("launch"),
                agent,
                phase: Phase::Admitted {
                    nonce: nonce.clone(),
                },
            },
            nonce,
        )
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    pub(crate) fn agent(&self) -> Agent {
        self.agent
    }

    /// The token, only while the bound process is the terminal's agent process.
    pub(crate) fn attested_token(&self) -> Option<&str> {
        self.attested().map(|(token, _)| token)
    }

    /// The token with the process it is bound to, once attested. Whoever
    /// reports or acts on the token proves first that the process still
    /// leads the terminal's foreground, with [`leads_foreground`].
    pub(crate) fn attested(&self) -> Option<(&str, ProcessInstance)> {
        match self.phase {
            Phase::Attested(process) => Some((self.token.as_str(), process)),
            Phase::Admitted { .. } | Phase::Claimed(_) => None,
        }
    }

    /// Whether this admission waits for `nonce`.
    pub(crate) fn awaits(&self, nonce: &str) -> bool {
        matches!(&self.phase, Phase::Admitted { nonce: pending } if pending == nonce)
    }

    /// The process the launch is bound to, once claimed.
    pub(crate) fn process(&self) -> Option<ProcessInstance> {
        match self.phase {
            Phase::Admitted { .. } => None,
            Phase::Claimed(process) | Phase::Attested(process) => Some(process),
        }
    }

    /// The helper `process`, whose parent is `parent`, claimed the admission
    /// in the pane whose shell is `shell`. The caller has already matched the
    /// nonce with [`Self::awaits`] and drops the launch on a refusal.
    pub(crate) fn claim(
        &mut self,
        process: ProcessInstance,
        parent: u32,
        shell: Option<u32>,
    ) -> Result<(), ClaimRefusal> {
        if process.started.is_none() {
            return Err(ClaimRefusal::UnknownBirth);
        }
        if shell != Some(parent) {
            return Err(ClaimRefusal::NotTheShellsChild);
        }
        self.phase = Phase::Claimed(process);
        Ok(())
    }

    /// Detection identified `process`, running `agent`, as the terminal's
    /// agent process. Returns whether the launch still holds the terminal.
    #[must_use]
    pub(crate) fn observe(&mut self, agent: Agent, process: ProcessInstance) -> bool {
        if agent != self.agent {
            return false;
        }
        match self.phase {
            // The helper claims before it execs, so the admission's own
            // process cannot be the agent before the claim.
            Phase::Admitted { .. } => false,
            Phase::Claimed(bound) | Phase::Attested(bound) => {
                if same_process(&bound, &process) {
                    self.phase = Phase::Attested(bound);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Whether a report sent by `processes` comes from the bound process.
    pub(crate) fn reported_by(&self, processes: &[ProcessInstance]) -> bool {
        self.process()
            .is_some_and(|bound| processes.iter().any(|shown| same_process(&bound, shown)))
    }

    /// What a live handoff carries: only a launch bound to a process, which
    /// the next server attests again when its detection identifies the same
    /// process.
    #[cfg(unix)]
    pub(crate) fn handoff(&self) -> Option<HandoffLaunchIncarnation> {
        let process = self.process()?;
        Some(HandoffLaunchIncarnation {
            token: self.token.clone(),
            agent: crate::detect::agent_label(self.agent).to_string(),
            process_id: process.pid,
            process_started: process.started?,
        })
    }

    /// The launch a live handoff carried, still to be attested by the next
    /// detection of its process; `None` when the record cannot name one.
    #[cfg(unix)]
    pub(crate) fn from_handoff(carried: HandoffLaunchIncarnation) -> Option<Self> {
        if !is_canonical_token(&carried.token) {
            return None;
        }
        Some(Self {
            token: carried.token,
            agent: crate::detect::parse_agent_label(&carried.agent)?,
            phase: Phase::Claimed(ProcessInstance {
                pid: carried.process_id,
                started: Some(carried.process_started),
            }),
        })
    }
}

/// A launch carried by a live handoff: its token and the process it is bound
/// to, by id and start time.
#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct HandoffLaunchIncarnation {
    pub token: String,
    pub agent: String,
    pub process_id: u32,
    pub process_started: u64,
}

/// The same process: the same id and the same start time, both known. A
/// process id alone can be reused by a later process.
fn same_process(bound: &ProcessInstance, other: &ProcessInstance) -> bool {
    bound.pid == other.pid && bound.started.is_some() && bound.started == other.started
}

/// The process leading the foreground process group `pgid` of a terminal,
/// with its start time read now. That group is the one a key written to the
/// terminal reaches; a replacement the shell started leads another group,
/// and a reused process id has another start time.
pub(crate) fn group_leader(pgid: u32) -> Option<ProcessInstance> {
    crate::platform::process_and_parent(pgid).map(|now| ProcessInstance {
        pid: now.process.pid,
        started: now.process.started,
    })
}

/// Whether the foreground `leader`, read now, is provably the `bound` launch
/// process: the same id and the same known start time. An unreadable leader
/// proves nothing.
pub(crate) fn leads_foreground(bound: ProcessInstance, leader: Option<ProcessInstance>) -> bool {
    leader.is_some_and(|leader| same_process(&bound, &leader))
}

const TOKEN_HEX_LEN: usize = 32;

/// Whether `token` has the form [`mint`] gives a launch token.
#[cfg(any(unix, test))]
fn is_canonical_token(token: &str) -> bool {
    token.strip_prefix("launch_").is_some_and(|hex| {
        hex.len() == TOKEN_HEX_LEN
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// A value no other launch gets: 128 bits of a SHA-256 over this process's
/// id, the time, a process-wide counter and the per-process random keys of
/// the standard library's hasher. It is an identity, not a secret.
fn mint(prefix: &str) -> String {
    use sha2::Digest as _;
    use std::hash::BuildHasher as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let counter = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let random = std::collections::hash_map::RandomState::new().hash_one(counter);
    let mut digest = sha2::Sha256::new();
    digest.update(std::process::id().to_le_bytes());
    digest.update(nanos.to_le_bytes());
    digest.update(counter.to_le_bytes());
    digest.update(random.to_le_bytes());
    let hex: String = digest
        .finalize()
        .iter()
        .take(TOKEN_HEX_LEN / 2)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{prefix}_{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL: u32 = 10;

    fn process(pid: u32, started: u64) -> ProcessInstance {
        ProcessInstance {
            pid,
            started: Some(started),
        }
    }

    fn claimed(agent: Agent, bound: ProcessInstance) -> LaunchIncarnation {
        let (mut launch, nonce) = LaunchIncarnation::admit(agent);
        assert!(launch.awaits(&nonce));
        launch.claim(bound, SHELL, Some(SHELL)).unwrap();
        launch
    }

    #[test]
    fn minted_tokens_are_canonical_and_never_repeat() {
        let tokens: std::collections::HashSet<String> = (0..1000).map(|_| mint("launch")).collect();
        assert_eq!(tokens.len(), 1000);
        assert!(tokens.iter().all(|token| is_canonical_token(token)));
        let (launch, nonce) = LaunchIncarnation::admit(Agent::Codex);
        assert_ne!(launch.token(), nonce, "the nonce is not the public token");
        assert!(nonce.starts_with("nonce_"));
    }

    #[test]
    fn an_admission_is_not_attested_until_its_claimed_process_is_observed() {
        let (mut launch, nonce) = LaunchIncarnation::admit(Agent::Codex);
        assert_eq!(launch.attested_token(), None);
        assert!(!launch.awaits("nonce_other"));
        launch.claim(process(42, 7), SHELL, Some(SHELL)).unwrap();
        assert!(!launch.awaits(&nonce), "the nonce is spent");
        assert_eq!(launch.attested_token(), None);
        assert!(launch.observe(Agent::Codex, process(42, 7)));
        assert_eq!(launch.attested_token(), Some(launch.token()));
        // The same process observed again keeps it.
        assert!(launch.observe(Agent::Codex, process(42, 7)));
    }

    #[test]
    fn a_process_observed_before_the_claim_is_not_the_admissions_process() {
        let (mut launch, _) = LaunchIncarnation::admit(Agent::Codex);
        assert!(!launch.observe(Agent::Codex, process(42, 7)));
    }

    #[test]
    fn another_process_or_kind_ends_the_launch() {
        // A reused process id with a later start time.
        assert!(!claimed(Agent::Codex, process(42, 7)).observe(Agent::Codex, process(42, 8)));
        // A same-kind replacement.
        assert!(!claimed(Agent::Codex, process(42, 7)).observe(Agent::Codex, process(43, 7)));
        // The bound process seen running another agent.
        assert!(!claimed(Agent::Codex, process(42, 7)).observe(Agent::Claude, process(42, 7)));
        // An observation without a start time proves nothing.
        assert!(!claimed(Agent::Codex, process(42, 7)).observe(
            Agent::Codex,
            ProcessInstance {
                pid: 42,
                started: None
            }
        ));
    }

    #[test]
    fn a_claim_needs_the_shells_child_with_a_known_birth() {
        let (mut launch, _) = LaunchIncarnation::admit(Agent::Codex);
        assert_eq!(
            launch.claim(
                ProcessInstance {
                    pid: 42,
                    started: None
                },
                SHELL,
                Some(SHELL)
            ),
            Err(ClaimRefusal::UnknownBirth)
        );
        assert_eq!(
            launch.claim(process(42, 7), 99, Some(SHELL)),
            Err(ClaimRefusal::NotTheShellsChild)
        );
        assert_eq!(
            launch.claim(process(42, 7), SHELL, None),
            Err(ClaimRefusal::NotTheShellsChild)
        );
    }

    #[test]
    fn only_the_bound_process_reports_for_the_launch() {
        let launch = claimed(Agent::Codex, process(42, 7));
        assert!(launch.reported_by(&[process(41, 1), process(42, 7)]));
        assert!(!launch.reported_by(&[process(42, 8)]));
        assert!(!launch.reported_by(&[ProcessInstance {
            pid: 42,
            started: None
        }]));
        let (unclaimed, _) = LaunchIncarnation::admit(Agent::Codex);
        assert!(!unclaimed.reported_by(&[process(42, 7)]));
    }

    #[cfg(unix)]
    #[test]
    fn a_handoff_carries_a_bound_launch_for_the_next_server_to_attest_again() {
        let (admitted, _) = LaunchIncarnation::admit(Agent::Codex);
        assert_eq!(
            admitted.handoff(),
            None,
            "an unclaimed admission is not carried"
        );

        let mut attested = claimed(Agent::Codex, process(42, 7));
        assert!(attested.observe(Agent::Codex, process(42, 7)));
        let carried = attested.handoff().unwrap();
        let json = serde_json::to_value(&carried).unwrap();
        let mut next = LaunchIncarnation::from_handoff(serde_json::from_value(json).unwrap())
            .expect("a carried launch loads");
        assert_eq!(next.token(), attested.token());
        assert_eq!(
            next.attested_token(),
            None,
            "the next server attests only after it sees the process itself"
        );
        assert!(next.observe(Agent::Codex, process(42, 7)));
        assert_eq!(next.attested_token(), Some(attested.token()));

        let mut replaced = LaunchIncarnation::from_handoff(carried.clone()).unwrap();
        assert!(!replaced.observe(Agent::Codex, process(42, 9)));

        for broken in [
            HandoffLaunchIncarnation {
                token: String::new(),
                ..carried.clone()
            },
            HandoffLaunchIncarnation {
                token: carried.token.to_uppercase(),
                ..carried.clone()
            },
            HandoffLaunchIncarnation {
                agent: "not-an-agent".into(),
                ..carried.clone()
            },
        ] {
            assert_eq!(LaunchIncarnation::from_handoff(broken), None);
        }
        // A record without the start time is not a process identity.
        let without_start = serde_json::json!({
            "token": carried.token, "agent": "codex", "process_id": 42
        });
        assert!(serde_json::from_value::<HandoffLaunchIncarnation>(without_start).is_err());
    }
}
