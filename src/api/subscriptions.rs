use regex::Regex;

use crate::api::event_hub::EventHistoryError;
use crate::api::schema::{
    ErrorBody, ErrorResponse, EventKind, Method, PaneAgentStatusChangedEvent,
    PaneOutputMatchedEvent, PaneScrollChangedEvent, PaneScrollInfo, Request, Subscription,
    SubscriptionEventData, SubscriptionEventEnvelope, SubscriptionEventKind,
};
use crate::api::server::{
    dispatch_projection_to_app_with_timeout, dispatch_to_app_with_timeout, APP_RESPONSE_TIMEOUT,
};
use crate::api::{ApiRequestSender, EventHub};

pub(super) fn output_match_read_source(
    source: &crate::api::schema::ReadSource,
) -> crate::api::schema::ReadSource {
    match source {
        crate::api::schema::ReadSource::Recent => crate::api::schema::ReadSource::RecentUnwrapped,
        other => *other,
    }
}

pub(super) fn match_output(
    text: &str,
    matcher: &crate::api::schema::OutputMatch,
    regex: Option<&Regex>,
) -> Option<String> {
    match matcher {
        crate::api::schema::OutputMatch::Substring { value } => text
            .lines()
            .find(|line| line.contains(value))
            .map(|line| line.to_string()),
        crate::api::schema::OutputMatch::Regex { .. } => regex.and_then(|re| {
            text.lines()
                .find(|line| re.is_match(line))
                .map(|line| line.to_string())
        }),
    }
}

pub(super) struct ActiveOutputMatchedSubscription {
    pane_id: String,
    source: crate::api::schema::ReadSource,
    lines: Option<u32>,
    matcher: crate::api::schema::OutputMatch,
    regex: Option<Regex>,
    strip_ansi: bool,
    currently_matching: bool,
    request_prefix: String,
}

pub(super) struct ActiveAgentStatusChangedSubscription {
    pane_id: String,
    status_filter: Option<crate::api::schema::AgentStatus>,
    last_status: Option<crate::api::schema::AgentStatus>,
    last_presentation: Option<PanePresentationSnapshot>,
    last_sequence: u64,
    initial_event: Option<PaneAgentStatusChangedEvent>,
    request_prefix: String,
}

pub(super) struct ActiveScrollChangedSubscription {
    pane_id: String,
    last_scroll: Option<PaneScrollInfo>,
    request_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PanePresentationSnapshot {
    title: Option<String>,
    display_agent: Option<String>,
    state_labels: std::collections::HashMap<String, String>,
}

impl PanePresentationSnapshot {
    fn from(pane: &PaneAgentStatusChangedEvent) -> Self {
        Self {
            title: pane.title.clone(),
            display_agent: pane.display_agent.clone(),
            state_labels: pane.state_labels.clone(),
        }
    }

    fn from_event(
        title: &Option<String>,
        display_agent: &Option<String>,
        state_labels: &std::collections::HashMap<String, String>,
    ) -> Self {
        Self {
            title: title.clone(),
            display_agent: display_agent.clone(),
            state_labels: state_labels.clone(),
        }
    }
}

pub(super) struct ActiveEventSubscription {
    event_kind: crate::api::schema::EventKind,
    last_sequence: u64,
}

pub(super) enum ActiveSubscription {
    Event(ActiveEventSubscription),
    OutputMatched(ActiveOutputMatchedSubscription),
    AgentStatusChanged(Box<ActiveAgentStatusChangedSubscription>),
    ScrollChanged(ActiveScrollChangedSubscription),
}

impl ActiveSubscription {
    pub(super) fn new(
        subscription: Subscription,
        request_id: &str,
        index: usize,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
        event_start_sequence: u64,
    ) -> Result<Self, ErrorResponse> {
        let event_subscription = |event_kind| {
            Self::Event(ActiveEventSubscription {
                event_kind,
                last_sequence: event_start_sequence,
            })
        };

        match subscription {
            Subscription::WorkspaceCreated {} => {
                Ok(event_subscription(EventKind::WorkspaceCreated))
            }
            Subscription::WorkspaceUpdated {} => {
                Ok(event_subscription(EventKind::WorkspaceUpdated))
            }
            Subscription::WorkspaceMetadataUpdated {} => {
                Ok(event_subscription(EventKind::WorkspaceMetadataUpdated))
            }
            Subscription::WorkspaceRenamed {} => {
                Ok(event_subscription(EventKind::WorkspaceRenamed))
            }
            Subscription::WorkspaceMoved {} => Ok(event_subscription(EventKind::WorkspaceMoved)),
            Subscription::WorkspaceReordered {} => {
                Ok(event_subscription(EventKind::WorkspaceReordered))
            }
            Subscription::WorkspaceClosed {} => Ok(event_subscription(EventKind::WorkspaceClosed)),
            Subscription::WorkspaceFocused {} => {
                Ok(event_subscription(EventKind::WorkspaceFocused))
            }
            Subscription::WorktreeCreated {} => Ok(event_subscription(EventKind::WorktreeCreated)),
            Subscription::WorktreeOpened {} => Ok(event_subscription(EventKind::WorktreeOpened)),
            Subscription::WorktreeRemoved {} => Ok(event_subscription(EventKind::WorktreeRemoved)),
            Subscription::TabCreated {} => Ok(event_subscription(EventKind::TabCreated)),
            Subscription::TabClosed {} => Ok(event_subscription(EventKind::TabClosed)),
            Subscription::TabFocused {} => Ok(event_subscription(EventKind::TabFocused)),
            Subscription::TabRenamed {} => Ok(event_subscription(EventKind::TabRenamed)),
            Subscription::TabMoved {} => Ok(event_subscription(EventKind::TabMoved)),
            Subscription::PaneCreated {} => Ok(event_subscription(EventKind::PaneCreated)),
            Subscription::PaneClosed {} => Ok(event_subscription(EventKind::PaneClosed)),
            Subscription::PaneUpdated {} => Ok(event_subscription(EventKind::PaneUpdated)),
            Subscription::PaneFocused {} => Ok(event_subscription(EventKind::PaneFocused)),
            Subscription::PaneMoved {} => Ok(event_subscription(EventKind::PaneMoved)),
            Subscription::PaneExited {} => Ok(event_subscription(EventKind::PaneExited)),
            Subscription::PaneAgentDetected {} => {
                Ok(event_subscription(EventKind::PaneAgentDetected))
            }
            Subscription::LayoutUpdated {} => Ok(event_subscription(EventKind::LayoutUpdated)),
            Subscription::PaneOutputMatched {
                pane_id,
                source,
                lines,
                r#match,
                strip_ansi,
            } => {
                let regex = match &r#match {
                    crate::api::schema::OutputMatch::Regex { value } => match Regex::new(value) {
                        Ok(regex) => Some(regex),
                        Err(err) => {
                            return Err(ErrorResponse {
                                id: request_id.to_string(),
                                error: ErrorBody {
                                    code: "invalid_regex".into(),
                                    message: err.to_string(),
                                },
                            });
                        }
                    },
                    crate::api::schema::OutputMatch::Substring { .. } => None,
                };

                let probe = pane_read(
                    format!("{request_id}:sub:{index}:probe"),
                    &pane_id,
                    source,
                    lines,
                    strip_ansi,
                    api_tx,
                );
                probe?;

                Ok(Self::OutputMatched(ActiveOutputMatchedSubscription {
                    pane_id,
                    source,
                    lines,
                    matcher: r#match,
                    regex,
                    strip_ansi,
                    currently_matching: false,
                    request_prefix: format!("{request_id}:sub:{index}"),
                }))
            }
            Subscription::PaneAgentStatusChanged {
                pane_id,
                agent_status,
            } => {
                let last_sequence = event_hub.current_sequence();
                let probe =
                    pane_agent_status(format!("{request_id}:sub:{index}:probe"), &pane_id, api_tx)?;
                let last_status = probe.agent_status;
                let last_presentation = PanePresentationSnapshot::from(&probe);
                let pane_id = probe.pane_id.clone();
                let initial_event = agent_status
                    .is_some_and(|wanted| wanted == probe.agent_status)
                    .then_some(probe);

                Ok(Self::AgentStatusChanged(Box::new(
                    ActiveAgentStatusChangedSubscription {
                        pane_id,
                        status_filter: agent_status,
                        last_status: Some(last_status),
                        last_presentation: Some(last_presentation),
                        last_sequence,
                        initial_event,
                        request_prefix: format!("{request_id}:sub:{index}"),
                    },
                )))
            }
            Subscription::PaneScrollChanged { pane_id } => {
                let probe = pane_get(format!("{request_id}:sub:{index}:probe"), &pane_id, api_tx)?;

                Ok(Self::ScrollChanged(ActiveScrollChangedSubscription {
                    pane_id: probe.pane_id,
                    last_scroll: probe.scroll,
                    request_prefix: format!("{request_id}:sub:{index}"),
                }))
            }
        }
    }

    pub(super) fn poll(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Option<serde_json::Value> {
        match self {
            Self::Event(subscription) => subscription.poll(event_hub),
            Self::OutputMatched(subscription) => {
                serde_json::to_value(subscription.poll(api_tx)?).ok()
            }
            Self::AgentStatusChanged(subscription) => {
                serde_json::to_value(subscription.poll(api_tx, event_hub)?).ok()
            }
            Self::ScrollChanged(subscription) => {
                serde_json::to_value(subscription.poll(api_tx)?).ok()
            }
        }
    }

    pub(super) fn poll_for_wait(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Option<serde_json::Value>, ErrorResponse> {
        match self {
            Self::AgentStatusChanged(subscription) => Ok(subscription
                .poll_result(api_tx, event_hub)?
                .and_then(|event| serde_json::to_value(event).ok())),
            _ => Ok(self.poll(api_tx, event_hub)),
        }
    }

    pub(super) fn poll_batch(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Vec<serde_json::Value>, ErrorBody> {
        match self {
            Self::Event(subscription) => {
                let events = subscription_events_after(event_hub, subscription.last_sequence)?;
                let mut matching = Vec::new();
                for (sequence, event) in events {
                    subscription.last_sequence = sequence;
                    if event.event == subscription.event_kind {
                        matching.push(serde_json::to_value(event).map_err(event_encoding_error)?);
                    }
                }
                Ok(matching)
            }
            Self::AgentStatusChanged(subscription) => {
                let events = subscription_events_after(event_hub, subscription.last_sequence)?;
                let mut matching = Vec::new();
                for (sequence, event) in events {
                    subscription.last_sequence = sequence;
                    if let Some(event) = subscription.event_from_history(event) {
                        matching.push(serde_json::to_value(event).map_err(event_encoding_error)?);
                    }
                }
                if matching.is_empty() {
                    if let Some(event) =
                        subscription.poll_snapshot(api_tx, event_hub).ok().flatten()
                    {
                        matching.push(serde_json::to_value(event).map_err(event_encoding_error)?);
                    }
                }
                Ok(matching)
            }
            // These subscriptions sample current state, not retained event history.
            // Keep their existing cadence even when a lifecycle batch was nonempty.
            Self::OutputMatched(_) | Self::ScrollChanged(_) => {
                Ok(self.poll(api_tx, event_hub).into_iter().collect())
            }
        }
    }
}

fn subscription_events_after(
    event_hub: &EventHub,
    sequence: u64,
) -> Result<Vec<(u64, crate::api::schema::EventEnvelope)>, ErrorBody> {
    event_hub.events_after_checked(sequence).map_err(|error| match error {
        EventHistoryError::Lost => ErrorBody {
            code: "events_lost".into(),
            message: "event subscription fell behind retained history; resubscribe and resync with session.snapshot".into(),
        },
        EventHistoryError::Unavailable => ErrorBody {
            code: "server_unavailable".into(),
            message: "event history is unavailable".into(),
        },
    })
}

fn event_encoding_error(error: serde_json::Error) -> ErrorBody {
    ErrorBody {
        code: "internal_error".into(),
        message: format!("failed to encode subscription event: {error}"),
    }
}

impl ActiveEventSubscription {
    fn poll(&mut self, event_hub: &EventHub) -> Option<serde_json::Value> {
        for (sequence, event) in event_hub.events_after(self.last_sequence) {
            self.last_sequence = sequence;
            if event.event == self.event_kind {
                return serde_json::to_value(event).ok();
            }
        }
        None
    }
}

impl ActiveOutputMatchedSubscription {
    fn poll(&mut self, api_tx: &ApiRequestSender) -> Option<SubscriptionEventEnvelope> {
        let read = pane_read(
            format!("{}:read", self.request_prefix),
            &self.pane_id,
            output_match_read_source(&self.source),
            self.lines,
            self.strip_ansi,
            api_tx,
        )
        .ok()?;

        let matched_line = match_output(&read.text, &self.matcher, self.regex.as_ref());
        match matched_line {
            Some(matched_line) => {
                if self.currently_matching {
                    return None;
                }
                self.currently_matching = true;
                Some(SubscriptionEventEnvelope {
                    event: SubscriptionEventKind::PaneOutputMatched,
                    data: SubscriptionEventData::PaneOutputMatched(PaneOutputMatchedEvent {
                        pane_id: read.pane_id.clone(),
                        matched_line,
                        read,
                    }),
                })
            }
            None => {
                self.currently_matching = false;
                None
            }
        }
    }
}

impl ActiveAgentStatusChangedSubscription {
    fn poll(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Option<SubscriptionEventEnvelope> {
        self.poll_result(api_tx, event_hub).ok().flatten()
    }

    fn poll_result(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Option<SubscriptionEventEnvelope>, ErrorResponse> {
        for (sequence, event) in event_hub.events_after(self.last_sequence) {
            self.last_sequence = sequence;
            if let Some(event) = self.event_from_history(event) {
                return Ok(Some(event));
            }
        }

        self.poll_snapshot(api_tx, event_hub)
    }

    fn event_from_history(
        &mut self,
        event: crate::api::schema::EventEnvelope,
    ) -> Option<SubscriptionEventEnvelope> {
        if event.event != EventKind::PaneAgentStatusChanged {
            return None;
        }
        let crate::api::schema::EventData::PaneAgentStatusChanged {
            pane_id,
            workspace_id,
            agent_status,
            agent,
            title,
            display_agent,
            state_labels,
        } = event.data
        else {
            return None;
        };
        if pane_id != self.pane_id {
            return None;
        }
        self.last_status = Some(agent_status);
        self.last_presentation = Some(PanePresentationSnapshot::from_event(
            &title,
            &display_agent,
            &state_labels,
        ));
        self.initial_event = None;
        if self
            .status_filter
            .is_some_and(|wanted| wanted != agent_status)
        {
            return None;
        }

        Some(SubscriptionEventEnvelope {
            event: SubscriptionEventKind::PaneAgentStatusChanged,
            data: SubscriptionEventData::PaneAgentStatusChanged(PaneAgentStatusChangedEvent {
                pane_id,
                workspace_id,
                agent_status,
                agent,
                title,
                display_agent,
                state_labels,
            }),
        })
    }

    fn poll_snapshot(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Option<SubscriptionEventEnvelope>, ErrorResponse> {
        if event_hub.current_sequence() != self.last_sequence {
            return Ok(None);
        } else if let Some(event) = self.initial_event.take() {
            return Ok(Some(SubscriptionEventEnvelope {
                event: SubscriptionEventKind::PaneAgentStatusChanged,
                data: SubscriptionEventData::PaneAgentStatusChanged(event),
            }));
        }

        let before_snapshot_sequence = self.last_sequence;
        let pane = pane_agent_status(
            format!("{}:pane", self.request_prefix),
            &self.pane_id,
            api_tx,
        );
        let after_snapshot_sequence = event_hub.current_sequence();
        if after_snapshot_sequence != before_snapshot_sequence {
            return Ok(None);
        }
        let pane = pane?;

        let event = self.event_from_snapshot(pane);
        if event.is_some() {
            self.last_sequence = after_snapshot_sequence;
        }
        Ok(event)
    }

    fn event_from_snapshot(
        &mut self,
        pane: PaneAgentStatusChangedEvent,
    ) -> Option<SubscriptionEventEnvelope> {
        let current_status = pane.agent_status;
        let current_presentation = PanePresentationSnapshot::from(&pane);
        let previous_status = self.last_status.replace(current_status);
        let previous_presentation = self.last_presentation.replace(current_presentation.clone());
        let presentation_changed = previous_presentation
            .as_ref()
            .is_some_and(|previous| previous != &current_presentation);
        let status_changed = previous_status.is_some_and(|previous| previous != current_status);
        if !(status_changed || presentation_changed) {
            return None;
        }
        if self
            .status_filter
            .is_some_and(|wanted| wanted != current_status)
        {
            return None;
        }

        Some(SubscriptionEventEnvelope {
            event: SubscriptionEventKind::PaneAgentStatusChanged,
            data: SubscriptionEventData::PaneAgentStatusChanged(pane),
        })
    }
}

impl ActiveScrollChangedSubscription {
    fn poll(&mut self, api_tx: &ApiRequestSender) -> Option<SubscriptionEventEnvelope> {
        let pane = pane_get(
            format!("{}:pane", self.request_prefix),
            &self.pane_id,
            api_tx,
        )
        .ok()?;
        self.event_from_snapshot(pane)
    }

    fn event_from_snapshot(
        &mut self,
        pane: crate::api::schema::PaneInfo,
    ) -> Option<SubscriptionEventEnvelope> {
        let scroll = pane.scroll;
        if self.last_scroll == scroll {
            return None;
        }
        self.last_scroll = scroll;
        let scroll = scroll?;

        Some(SubscriptionEventEnvelope {
            event: SubscriptionEventKind::ScrollChanged,
            data: SubscriptionEventData::ScrollChanged(PaneScrollChangedEvent {
                pane_id: pane.pane_id,
                workspace_id: pane.workspace_id,
                scroll,
            }),
        })
    }
}

fn pane_read(
    request_id: String,
    pane_id: &str,
    source: crate::api::schema::ReadSource,
    lines: Option<u32>,
    strip_ansi: bool,
    api_tx: &ApiRequestSender,
) -> Result<crate::api::schema::PaneReadResult, ErrorResponse> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: request_id.clone(),
            method: Method::PaneRead(crate::api::schema::PaneReadParams {
                pane_id: pane_id.to_string(),
                source,
                lines,
                format: crate::api::schema::ReadFormat::Text,
                strip_ansi,
                intent: crate::api::schema::ReadIntent::Passive,
            }),
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).map_err(|_| ErrorResponse {
        id: request_id.clone(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane read response".into(),
        },
    })?;
    if value.get("error").is_some() {
        return serde_json::from_value(value).map_err(|_| ErrorResponse {
            id: request_id,
            error: ErrorBody {
                code: "internal_error".into(),
                message: "failed to decode pane read error".into(),
            },
        });
    }
    serde_json::from_value(value["result"]["read"].clone()).map_err(|_| ErrorResponse {
        id: request_id,
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane read result".into(),
        },
    })
}

fn pane_agent_status(
    request_id: String,
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<PaneAgentStatusChangedEvent, ErrorResponse> {
    let response = dispatch_projection_to_app_with_timeout(
        Request {
            id: request_id.clone(),
            method: Method::PaneGet(crate::api::schema::PaneTarget {
                pane_id: pane_id.to_string(),
            }),
        },
        crate::api::ApiRequestProjection::PaneAgentStatus,
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).map_err(|_| ErrorResponse {
        id: request_id.clone(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane status response".into(),
        },
    })?;
    if value.get("error").is_some() {
        let response =
            serde_json::from_value::<ErrorResponse>(value).map_err(|_| ErrorResponse {
                id: request_id.clone(),
                error: ErrorBody {
                    code: "internal_error".into(),
                    message: "failed to decode pane status error".into(),
                },
            })?;
        return Err(response);
    }
    serde_json::from_value::<crate::api::PaneAgentStatusProjectionResponse>(value)
        .map(|response| response.result)
        .map_err(|_| ErrorResponse {
            id: request_id,
            error: ErrorBody {
                code: "internal_error".into(),
                message: "failed to decode pane status result".into(),
            },
        })
}

fn pane_get(
    request_id: String,
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<crate::api::schema::PaneInfo, ErrorResponse> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: request_id.clone(),
            method: Method::PaneGet(crate::api::schema::PaneTarget {
                pane_id: pane_id.to_string(),
            }),
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).map_err(|_| ErrorResponse {
        id: request_id.clone(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane get response".into(),
        },
    })?;
    if value.get("error").is_some() {
        let response =
            serde_json::from_value::<ErrorResponse>(value).map_err(|_| ErrorResponse {
                id: request_id,
                error: ErrorBody {
                    code: "internal_error".into(),
                    message: "failed to decode pane get error".into(),
                },
            })?;
        return Err(response);
    }
    serde_json::from_value(value["result"]["pane"].clone()).map_err(|_| ErrorResponse {
        id: request_id,
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane get result".into(),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::api::schema::{AgentStatus, EventData, EventEnvelope, EventKind, PaneInfo};

    fn presentation_event(title: Option<&str>) -> EventEnvelope {
        EventEnvelope {
            event: EventKind::PaneAgentStatusChanged,
            data: EventData::PaneAgentStatusChanged {
                pane_id: "pane_1".into(),
                workspace_id: "workspace_1".into(),
                agent_status: AgentStatus::Working,
                agent: Some("pi".into()),
                title: title.map(str::to_string),
                display_agent: None,
                state_labels: HashMap::new(),
            },
        }
    }

    fn workspace_focused_event(workspace_id: &str) -> EventEnvelope {
        EventEnvelope {
            event: EventKind::WorkspaceFocused,
            data: EventData::WorkspaceFocused {
                workspace_id: workspace_id.into(),
            },
        }
    }

    fn pane_info_with_scroll(scroll: Option<PaneScrollInfo>) -> PaneInfo {
        PaneInfo {
            pane_id: "pane_1".into(),
            terminal_id: "terminal_1".into(),
            workspace_id: "workspace_1".into(),
            tab_id: "tab_1".into(),
            focused: true,
            cwd: None,
            foreground_cwd: None,
            restore_error: None,
            label: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            display_agent: None,
            agent_status: AgentStatus::Unknown,
            state_labels: HashMap::new(),
            tokens: HashMap::new(),
            agent_session: None,
            scroll,
            revision: 0,
        }
    }

    fn status_snapshot(status: AgentStatus, title: Option<&str>) -> PaneAgentStatusChangedEvent {
        PaneAgentStatusChangedEvent {
            pane_id: "pane_1".into(),
            workspace_id: "workspace_1".into(),
            agent_status: status,
            agent: Some("pi".into()),
            title: title.map(str::to_owned),
            display_agent: Some("Reviewer".into()),
            state_labels: HashMap::from([("working".into(), "Reviewing".into())]),
        }
    }

    fn status_subscription(
        pane: &PaneAgentStatusChangedEvent,
        status_filter: Option<AgentStatus>,
        event_hub: &EventHub,
    ) -> ActiveAgentStatusChangedSubscription {
        ActiveAgentStatusChangedSubscription {
            pane_id: pane.pane_id.clone(),
            status_filter,
            last_status: Some(pane.agent_status),
            last_presentation: Some(PanePresentationSnapshot::from(pane)),
            last_sequence: event_hub.current_sequence(),
            initial_event: None,
            request_prefix: "test".into(),
        }
    }

    fn status_responder(
        mut respond: impl FnMut(&crate::api::ApiRequestMessage) -> String + Send + 'static,
    ) -> (ApiRequestSender, std::thread::JoinHandle<()>) {
        let (api_tx, mut api_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::api::ApiRequestMessage>();
        let responder = std::thread::spawn(move || {
            while let Some(request) = api_rx.blocking_recv() {
                assert!(matches!(request.request.method, Method::PaneGet(_)));
                assert_eq!(
                    request.projection,
                    Some(crate::api::ApiRequestProjection::PaneAgentStatus)
                );
                request.respond_to.send(respond(&request)).unwrap();
            }
        });
        (api_tx, responder)
    }

    fn status_response(id: &str, pane: PaneAgentStatusChangedEvent) -> String {
        serde_json::to_string(&crate::api::PaneAgentStatusProjectionResponse {
            id: id.into(),
            result: pane,
        })
        .unwrap()
    }

    #[test]
    fn agent_status_setup_and_quiet_polls_preserve_initial_filter_and_read_cadence() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        for filter in [None, Some(AgentStatus::Working), Some(AgentStatus::Idle)] {
            let reads = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&reads);
            let snapshot = status_snapshot(AgentStatus::Working, Some("review"));
            let (api_tx, responder) = status_responder(move |request| {
                counted.fetch_add(1, Ordering::Relaxed);
                status_response(&request.request.id, snapshot.clone())
            });
            let hub = EventHub::default();
            let mut subscription = ActiveSubscription::new(
                Subscription::PaneAgentStatusChanged {
                    pane_id: "pane_1".into(),
                    agent_status: filter,
                },
                "setup",
                0,
                &api_tx,
                &hub,
                hub.current_sequence(),
            )
            .unwrap();
            assert_eq!(reads.load(Ordering::Relaxed), 1);
            let first = subscription.poll_for_wait(&api_tx, &hub).unwrap();
            let initial_matches = filter == Some(AgentStatus::Working);
            assert_eq!(first.is_some(), initial_matches);
            if let Some(event) = first {
                assert_eq!(event["data"]["agent"], "pi");
                assert_eq!(event["data"]["title"], "review");
                assert_eq!(event["data"]["display_agent"], "Reviewer");
                assert_eq!(event["data"]["state_labels"]["working"], "Reviewing");
            }
            let expected_reads = if initial_matches { 1 } else { 2 };
            assert_eq!(reads.load(Ordering::Relaxed), expected_reads);
            for poll in 1..=3 {
                assert!(subscription.poll_for_wait(&api_tx, &hub).unwrap().is_none());
                assert_eq!(reads.load(Ordering::Relaxed), expected_reads + poll);
            }
            drop(api_tx);
            responder.join().unwrap();
        }
    }

    #[test]
    fn agent_status_fallback_preserves_all_event_fields_and_presentation_changes() {
        let hub = EventHub::default();
        let original = status_snapshot(AgentStatus::Working, None);
        let mut subscription = status_subscription(&original, None, &hub);
        let mut changed = original.clone();
        changed.title = Some("renamed".into());
        changed.display_agent = Some("Auditor".into());
        changed
            .state_labels
            .insert("working".into(), "Auditing".into());
        let event = subscription.event_from_snapshot(changed.clone()).unwrap();
        let SubscriptionEventData::PaneAgentStatusChanged(data) = event.data else {
            panic!("expected status event");
        };
        assert_eq!(data.pane_id, changed.pane_id);
        assert_eq!(data.workspace_id, changed.workspace_id);
        assert_eq!(data.agent_status, changed.agent_status);
        assert_eq!(data.agent, changed.agent);
        assert_eq!(data.title, changed.title);
        assert_eq!(data.display_agent, changed.display_agent);
        assert_eq!(data.state_labels, changed.state_labels);
        assert!(subscription.event_from_snapshot(changed).is_none());

        // These fields are delivered on an event but have never triggered a
        // fallback event on their own. Preserve that distinction.
        let mut identity_only = original.clone();
        identity_only.agent = Some("codex".into());
        identity_only.workspace_id = "workspace_2".into();
        let mut subscription = status_subscription(&original, None, &hub);
        assert!(subscription.event_from_snapshot(identity_only).is_none());
    }

    #[test]
    fn agent_status_fallback_tracks_filtered_out_changes_before_the_next_match() {
        let hub = EventHub::default();
        let working = status_snapshot(AgentStatus::Working, None);
        let blocked = status_snapshot(AgentStatus::Blocked, Some("approval"));
        let mut subscription = status_subscription(&working, Some(AgentStatus::Working), &hub);
        assert!(subscription.event_from_snapshot(blocked).is_none());
        assert_eq!(subscription.last_status, Some(AgentStatus::Blocked));
        assert!(subscription.event_from_snapshot(working.clone()).is_some());
        assert!(subscription.event_from_snapshot(working).is_none());
    }

    #[test]
    fn agent_status_snapshot_race_defers_to_history_without_loss_or_duplicates() {
        for error_during_snapshot in [false, true] {
            let hub = EventHub::default();
            let working = status_snapshot(AgentStatus::Working, None);
            let mut subscription = status_subscription(&working, None, &hub);
            let responder_hub = hub.clone();
            let mut reads = 0;
            let (api_tx, responder) = status_responder(move |request| {
                reads += 1;
                if reads == 1 {
                    let mut event = presentation_event(Some("approval"));
                    let EventData::PaneAgentStatusChanged { agent_status, .. } = &mut event.data
                    else {
                        panic!("expected status event");
                    };
                    *agent_status = AgentStatus::Blocked;
                    responder_hub.push(event);
                    if error_during_snapshot {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.request.id.clone(),
                            error: ErrorBody {
                                code: "pane_not_found".into(),
                                message: "pane not found".into(),
                            },
                        })
                        .unwrap();
                    }
                }
                let mut pane = status_snapshot(AgentStatus::Blocked, Some("approval"));
                pane.display_agent = None;
                pane.state_labels.clear();
                status_response(&request.request.id, pane)
            });
            assert!(subscription.poll_result(&api_tx, &hub).unwrap().is_none());
            assert_eq!(subscription.last_status, Some(AgentStatus::Working));
            assert_eq!(subscription.last_sequence, 0);
            let event = subscription.poll_result(&api_tx, &hub).unwrap().unwrap();
            let SubscriptionEventData::PaneAgentStatusChanged(data) = event.data else {
                panic!("expected status event");
            };
            assert_eq!(data.agent_status, AgentStatus::Blocked);
            assert_eq!(data.title.as_deref(), Some("approval"));
            assert_eq!(subscription.last_sequence, hub.current_sequence());
            assert!(subscription.poll_result(&api_tx, &hub).unwrap().is_none());
            drop(api_tx);
            responder.join().unwrap();
        }
    }

    #[test]
    fn agent_status_missing_pane_errors_propagate_to_wait_but_stream_poll_suppresses_them() {
        let hub = EventHub::default();
        let pane = status_snapshot(AgentStatus::Working, None);
        let (api_tx, responder) = status_responder(|request| {
            serde_json::to_string(&ErrorResponse {
                id: request.request.id.clone(),
                error: ErrorBody {
                    code: "pane_not_found".into(),
                    message: "pane pane_1 not found".into(),
                },
            })
            .unwrap()
        });
        let mut subscription = status_subscription(&pane, None, &hub);
        let error = subscription.poll_result(&api_tx, &hub).unwrap_err();
        assert_eq!(error.id, "test:pane");
        assert_eq!(error.error.code, "pane_not_found");
        assert!(subscription.poll(&api_tx, &hub).is_none());
        let mut stream = ActiveSubscription::AgentStatusChanged(Box::new(subscription));
        assert!(stream.poll_batch(&api_tx, &hub).unwrap().is_empty());
        drop(api_tx);
        responder.join().unwrap();

        let (closed_tx, closed_rx) = tokio::sync::mpsc::unbounded_channel();
        drop(closed_rx);
        let mut subscription = status_subscription(&pane, None, &hub);
        let error = subscription.poll_result(&closed_tx, &hub).unwrap_err();
        assert_eq!(error.error.code, "server_unavailable");
    }

    #[test]
    fn agent_status_setup_rejects_missing_panes_and_disconnected_application_queue() {
        let hub = EventHub::default();
        let (api_tx, responder) = status_responder(|request| {
            serde_json::to_string(&ErrorResponse {
                id: request.request.id.clone(),
                error: ErrorBody {
                    code: "pane_not_found".into(),
                    message: "pane missing not found".into(),
                },
            })
            .unwrap()
        });
        let setup = |sender: &ApiRequestSender| {
            ActiveSubscription::new(
                Subscription::PaneAgentStatusChanged {
                    pane_id: "missing".into(),
                    agent_status: Some(AgentStatus::Working),
                },
                "setup-error",
                4,
                sender,
                &hub,
                hub.current_sequence(),
            )
        };
        let Err(error) = setup(&api_tx) else {
            panic!("expected missing pane error")
        };
        assert_eq!(error.id, "setup-error:sub:4:probe");
        assert_eq!(error.error.code, "pane_not_found");
        drop(api_tx);
        responder.join().unwrap();
        let (closed_tx, closed_rx) = tokio::sync::mpsc::unbounded_channel();
        drop(closed_rx);
        let Err(error) = setup(&closed_tx) else {
            panic!("expected disconnected queue error")
        };
        assert_eq!(error.id, "setup-error:sub:4:probe");
        assert_eq!(error.error.code, "server_unavailable");
    }

    #[test]
    fn many_quiet_agent_status_subscriptions_read_once_per_poll() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let reads = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&reads);
        let (api_tx, responder) = status_responder(move |request| {
            counted.fetch_add(1, Ordering::Relaxed);
            let Method::PaneGet(target) = &request.request.method else {
                unreachable!()
            };
            let mut pane = status_snapshot(AgentStatus::Working, None);
            pane.pane_id = target.pane_id.clone();
            status_response(&request.request.id, pane)
        });
        let hub = EventHub::default();
        let mut subscriptions = (0..121)
            .map(|index| {
                ActiveSubscription::new(
                    Subscription::PaneAgentStatusChanged {
                        pane_id: format!("pane_{index}"),
                        agent_status: None,
                    },
                    "many",
                    index,
                    &api_tx,
                    &hub,
                    hub.current_sequence(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(reads.load(Ordering::Relaxed), subscriptions.len());
        for poll in 1..=3 {
            for subscription in &mut subscriptions {
                assert!(subscription.poll_batch(&api_tx, &hub).unwrap().is_empty());
            }
            assert_eq!(
                reads.load(Ordering::Relaxed),
                subscriptions.len() * (poll + 1)
            );
        }
        drop(api_tx);
        responder.join().unwrap();
    }

    #[test]
    fn lifecycle_subscription_skips_history_but_keeps_setup_window_events() {
        let event_hub = EventHub::default();
        event_hub.push(workspace_focused_event("before_subscription"));
        let event_start_sequence = event_hub.current_sequence();
        event_hub.push(workspace_focused_event("during_setup"));

        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut subscription = ActiveSubscription::new(
            Subscription::WorkspaceFocused {},
            "test",
            0,
            &api_tx,
            &event_hub,
            event_start_sequence,
        )
        .expect("workspace focus subscription");

        let setup_event = subscription
            .poll(&api_tx, &event_hub)
            .expect("setup-window event");
        assert_eq!(setup_event["data"]["workspace_id"], "during_setup");
        assert!(subscription.poll(&api_tx, &event_hub).is_none());

        event_hub.push(workspace_focused_event("after_setup"));
        let live_event = subscription.poll(&api_tx, &event_hub).expect("live event");
        assert_eq!(live_event["data"]["workspace_id"], "after_setup");
    }

    #[test]
    fn workspace_metadata_subscription_uses_dedicated_event_kind() {
        let event_hub = EventHub::default();
        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let subscription = ActiveSubscription::new(
            Subscription::WorkspaceMetadataUpdated {},
            "test",
            0,
            &api_tx,
            &event_hub,
            event_hub.current_sequence(),
        )
        .expect("workspace metadata subscription");

        assert!(matches!(
            subscription,
            ActiveSubscription::Event(ActiveEventSubscription {
                event_kind: EventKind::WorkspaceMetadataUpdated,
                ..
            })
        ));
    }

    #[test]
    fn lifecycle_batch_drains_in_order_and_advances_past_unmatched_events() {
        let event_hub = EventHub::default();
        event_hub.push(workspace_focused_event("old"));
        let start = event_hub.current_sequence();
        event_hub.push(workspace_focused_event("setup"));
        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut subscription = ActiveSubscription::new(
            Subscription::WorkspaceFocused {},
            "batch",
            0,
            &api_tx,
            &event_hub,
            start,
        )
        .unwrap();
        event_hub.push(presentation_event(None));
        event_hub.push(workspace_focused_event("live"));
        let events = subscription.poll_batch(&api_tx, &event_hub).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["data"]["workspace_id"], "setup");
        assert_eq!(events[1]["data"]["workspace_id"], "live");
        assert!(subscription
            .poll_batch(&api_tx, &event_hub)
            .unwrap()
            .is_empty());
        let ActiveSubscription::Event(subscription) = subscription else {
            panic!("expected lifecycle subscription");
        };
        assert_eq!(subscription.last_sequence, event_hub.current_sequence());
    }

    #[test]
    fn agent_status_batch_preserves_transitions_filters_and_initial_state_ordering() {
        for filtered in [false, true] {
            let event_hub = EventHub::default();
            let mut subscription = ActiveSubscription::AgentStatusChanged(Box::new(
                ActiveAgentStatusChangedSubscription {
                    pane_id: "pane_1".into(),
                    status_filter: filtered.then_some(AgentStatus::Working),
                    last_status: Some(AgentStatus::Working),
                    last_presentation: None,
                    last_sequence: event_hub.current_sequence(),
                    initial_event: Some(PaneAgentStatusChangedEvent {
                        pane_id: "pane_1".into(),
                        workspace_id: "workspace_1".into(),
                        agent_status: AgentStatus::Working,
                        agent: Some("pi".into()),
                        title: Some("stale initial snapshot".into()),
                        display_agent: None,
                        state_labels: HashMap::new(),
                    }),
                    request_prefix: "batch".into(),
                },
            ));
            for (status, title) in [
                (AgentStatus::Working, "started"),
                (AgentStatus::Blocked, "approval"),
                (AgentStatus::Idle, "finished"),
                (AgentStatus::Working, "restarted"),
            ] {
                let mut event = presentation_event(Some(title));
                let EventData::PaneAgentStatusChanged { agent_status, .. } = &mut event.data else {
                    panic!("expected status data");
                };
                *agent_status = status;
                event_hub.push(event);
            }
            let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
            let events = subscription.poll_batch(&api_tx, &event_hub).unwrap();
            let titles = events
                .iter()
                .map(|event| event["data"]["title"].as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                titles,
                if filtered {
                    vec!["started", "restarted"]
                } else {
                    vec!["started", "approval", "finished", "restarted"]
                }
            );
            let ActiveSubscription::AgentStatusChanged(subscription) = subscription else {
                panic!("expected agent subscription");
            };
            assert_eq!(subscription.last_sequence, event_hub.current_sequence());
            assert!(subscription.initial_event.is_none());
        }
    }

    #[test]
    fn scroll_subscription_emits_when_scroll_snapshot_changes() {
        let at_bottom = PaneScrollInfo {
            offset_from_bottom: 0,
            max_offset_from_bottom: 40,
            viewport_rows: 20,
        };
        let scrolled_back = PaneScrollInfo {
            offset_from_bottom: 8,
            max_offset_from_bottom: 40,
            viewport_rows: 20,
        };
        let mut subscription = ActiveScrollChangedSubscription {
            pane_id: "pane_1".into(),
            last_scroll: Some(at_bottom),
            request_prefix: "test".into(),
        };

        assert!(subscription
            .event_from_snapshot(pane_info_with_scroll(Some(at_bottom)))
            .is_none());

        let event = subscription
            .event_from_snapshot(pane_info_with_scroll(Some(scrolled_back)))
            .expect("scroll event");
        assert_eq!(event.event, SubscriptionEventKind::ScrollChanged);
        let SubscriptionEventData::ScrollChanged(data) = event.data else {
            panic!("wrong event data");
        };
        assert_eq!(data.pane_id, "pane_1");
        assert_eq!(data.workspace_id, "workspace_1");
        assert_eq!(data.scroll, scrolled_back);
    }

    #[test]
    fn agent_status_subscription_replays_queued_metadata_set_and_expiry_events() {
        let event_hub = EventHub::default();
        let mut subscription = ActiveAgentStatusChangedSubscription {
            pane_id: "pane_1".into(),
            status_filter: None,
            last_status: Some(AgentStatus::Working),
            last_presentation: Some(PanePresentationSnapshot {
                title: None,
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            last_sequence: event_hub.current_sequence(),
            initial_event: None,
            request_prefix: "test".into(),
        };

        event_hub.push(presentation_event(Some("short lived")));
        event_hub.push(presentation_event(None));

        let set_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("set event");
        let SubscriptionEventData::PaneAgentStatusChanged(set_data) = set_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(set_data.title.as_deref(), Some("short lived"));

        let expiry_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("expiry event");
        let SubscriptionEventData::PaneAgentStatusChanged(expiry_data) = expiry_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(expiry_data.title, None);
    }

    #[test]
    fn agent_status_subscription_prefers_setup_window_events_over_initial_snapshot() {
        let event_hub = EventHub::default();
        let mut subscription = ActiveAgentStatusChangedSubscription {
            pane_id: "pane_1".into(),
            status_filter: Some(AgentStatus::Working),
            last_status: Some(AgentStatus::Working),
            last_presentation: Some(PanePresentationSnapshot {
                title: None,
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            last_sequence: event_hub.current_sequence(),
            initial_event: Some(PaneAgentStatusChangedEvent {
                pane_id: "pane_1".into(),
                workspace_id: "workspace_1".into(),
                agent_status: AgentStatus::Working,
                agent: Some("pi".into()),
                title: None,
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            request_prefix: "test".into(),
        };

        event_hub.push(presentation_event(Some("short lived")));
        event_hub.push(presentation_event(None));

        let set_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("set event");
        let SubscriptionEventData::PaneAgentStatusChanged(set_data) = set_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(set_data.title.as_deref(), Some("short lived"));

        let expiry_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("expiry event");
        let SubscriptionEventData::PaneAgentStatusChanged(expiry_data) = expiry_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(expiry_data.title, None);
    }

    #[test]
    fn agent_status_subscription_emits_setup_window_event_already_reflected_by_probe() {
        let event_hub = EventHub::default();
        let mut subscription = ActiveAgentStatusChangedSubscription {
            pane_id: "pane_1".into(),
            status_filter: Some(AgentStatus::Working),
            last_status: Some(AgentStatus::Working),
            last_presentation: Some(PanePresentationSnapshot {
                title: Some("short lived".into()),
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            last_sequence: event_hub.current_sequence(),
            initial_event: Some(PaneAgentStatusChangedEvent {
                pane_id: "pane_1".into(),
                workspace_id: "workspace_1".into(),
                agent_status: AgentStatus::Working,
                agent: Some("pi".into()),
                title: Some("short lived".into()),
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            request_prefix: "test".into(),
        };

        event_hub.push(presentation_event(Some("short lived")));

        let event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("setup-window event");
        let SubscriptionEventData::PaneAgentStatusChanged(data) = event.data else {
            panic!("wrong event data");
        };
        assert_eq!(data.title.as_deref(), Some("short lived"));
        assert!(subscription.initial_event.is_none());
    }
}
