pub mod client;
mod event_hub;
pub mod schema;
mod server;
mod status;
mod subscriptions;
mod wait;

pub use event_hub::EventHub;
pub use server::ServerHandle;
pub(crate) use server::{api_method_name, start_server_with_stop_control};
pub use status::{read_runtime_status_at, RuntimeStatus};

use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::api::schema::{Method, Request};

pub const SOCKET_PATH_ENV_VAR: &str = "HERDR_SOCKET_PATH";

pub(crate) fn request_changes_ui(request: &Request) -> bool {
    matches!(
        &request.method,
        Method::ServerReloadConfig(_)
            | Method::ServerReloadAgentManifests(_)
            | Method::NotificationShow(_)
            | Method::ProductAnnouncementDismiss(_)
            | Method::ReleaseNotesDismiss(_)
            | Method::CommandInvoke(_)
            | Method::WorkspaceCreate(_)
            | Method::WorkspaceFocus(_)
            | Method::WorkspaceRename(_)
            | Method::WorkspaceMove(_)
            | Method::WorkspaceMoveBlock(_)
            | Method::WorkspaceReportMetadata(_)
            | Method::WorkspaceClose(_)
            | Method::WorktreeCreate(_)
            | Method::WorktreeOpen(_)
            | Method::WorktreeRemove(_)
            | Method::TabCreate(_)
            | Method::TabFocus(_)
            | Method::TabRename(_)
            | Method::TabMove(_)
            | Method::TabClose(_)
            | Method::LayoutApply(_)
            | Method::LayoutSetSplitRatio(_)
            | Method::AgentRename(_)
            | Method::AgentViewSet(_)
            | Method::AgentViewClear(_)
            | Method::AgentFocus(_)
            | Method::AgentStart(_)
            | Method::AgentStartWithLauncher(_)
            | Method::AgentPrompt(_)
            | Method::AgentSendKeys(_)
            | Method::PaneSplit(_)
            | Method::PaneSwap(_)
            | Method::PaneMove(_)
            | Method::PaneZoom(_)
            | Method::PaneFocusDirection(_)
            | Method::PaneResize(_)
            | Method::PaneScroll(_)
            | Method::PaneClear(_)
            | Method::PaneEditScrollback(_)
            | Method::PaneFocus(_)
            | Method::PaneInputSet(_)
            | Method::PaneRename(_)
            | Method::PaneReportAgent(_)
            | Method::PaneReportAgentSession(_)
            | Method::PaneReportMetadata(_)
            | Method::PaneClearAgentAuthority(_)
            | Method::PaneReleaseAgent(_)
            | Method::PaneClose(_)
            | Method::PopupClose(_)
            | Method::PluginUnlink(_)
            | Method::PluginDisable(_)
            | Method::PluginActionInvoke(_)
            | Method::PluginPaneOpen(_)
            | Method::PluginPaneFocus(_)
            | Method::PluginPaneClose(_)
    )
}

pub struct ApiRequestMessage {
    pub request: Request,
    // In-process dispatch only. This is absent from Request/Method and cannot be
    // selected by a JSON client or change an advertised endpoint method.
    pub(crate) projection: Option<ApiRequestProjection>,
    // In-process dispatch only, like `projection`: the process on the other end
    // of the local connection that sent a `pane.report_agent_session` and its
    // parents, read as the request arrived, while the sender still runs.
    pub(crate) peer_ancestry: Option<Vec<crate::platform::AncestorProcess>>,
    pub respond_to: std::sync::mpsc::Sender<String>,
    pub response_write_complete: Option<std::sync::mpsc::Receiver<()>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ApiRequestProjection {
    PaneAgentStatus,
    /// agent.prompt's check for input its idle Codex never read. Valid only
    /// with agent.get, and only for the terminal the prompt was written to.
    /// With `nudge`, it may write one focus-in report (see
    /// `App::handle_agent_input_wakeup`); without it, it only measures.
    AgentInputWakeup {
        terminal_id: String,
        nudge: Option<WakeupNudge>,
    },
}

impl ApiRequestMessage {
    pub(crate) fn validate_projection(&self) -> Result<(), schema::ErrorResponse> {
        match (&self.projection, &self.request.method) {
            (None, _)
            | (Some(ApiRequestProjection::PaneAgentStatus), Method::PaneGet(_))
            | (Some(ApiRequestProjection::AgentInputWakeup { .. }), Method::AgentGet(_)) => Ok(()),
            _ => Err(schema::ErrorResponse {
                id: self.request.id.clone(),
                error: schema::ErrorBody {
                    code: "internal_error".into(),
                    message: "invalid internal API projection request".into(),
                },
            }),
        }
    }
}

// Reuse the event's exact fields without adding a public ResponseResult variant.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct PaneAgentStatusProjectionResponse {
    pub(crate) id: String,
    pub(crate) result: schema::PaneAgentStatusChangedEvent,
}

/// Permission for one wake-up write: it lapses at `not_after`, and the waiting
/// prompt can withdraw it through `claim`. The app checks both immediately
/// before it writes, after measuring, so a late request writes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WakeupNudge {
    pub(crate) not_after: std::time::Instant,
    pub(crate) claim: WakeupClaim,
}

impl WakeupNudge {
    /// Whether the app may write now. Claiming is final: once this returns
    /// true the prompt can no longer cancel, and it never returns true after
    /// the prompt cancelled or the permission lapsed.
    pub(crate) fn claim_at(&self, now: std::time::Instant) -> bool {
        if now > self.not_after {
            self.claim.cancel();
            return false;
        }
        self.claim.claim()
    }
}

/// One wake-up's single write right, shared by the waiting prompt and the app.
/// Whichever side moves first wins: the app claims it immediately before it
/// writes, and the prompt cancels it when it stops waiting for the answer or
/// returns, so a request handled after that writes nothing, and a prompt that
/// gave up knows whether the write happened.
#[derive(Debug, Clone, Default)]
pub(crate) struct WakeupClaim(std::sync::Arc<std::sync::atomic::AtomicU8>);

impl WakeupClaim {
    const PENDING: u8 = 0;
    const CLAIMED: u8 = 1;
    const CANCELLED: u8 = 2;

    fn claim(&self) -> bool {
        self.0
            .compare_exchange(
                Self::PENDING,
                Self::CLAIMED,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
    }

    /// Withdraw the write right. True when no write happened or can happen;
    /// false when the app already claimed it.
    pub(crate) fn cancel(&self) -> bool {
        match self.0.compare_exchange(
            Self::PENDING,
            Self::CANCELLED,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        ) {
            Ok(_) => true,
            Err(state) => state == Self::CANCELLED,
        }
    }
}

impl PartialEq for WakeupClaim {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for WakeupClaim {}

/// What the lost-wakeup check measured, and whether it wrote the one nudge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AgentInputWakeup {
    /// `None` when the platform cannot measure unread PTY input.
    pub(crate) unread_input_bytes: Option<usize>,
    pub(crate) focus_reporting: bool,
    pub(crate) nudged: bool,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct AgentInputWakeupProjectionResponse {
    pub(crate) id: String,
    pub(crate) result: AgentInputWakeup,
}

pub type ApiRequestSender = mpsc::UnboundedSender<ApiRequestMessage>;

pub fn socket_path() -> PathBuf {
    crate::session::active_api_socket_path()
}
