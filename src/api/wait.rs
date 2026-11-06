use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use regex::Regex;

use crate::api::schema::{
    ErrorBody, ErrorResponse, EventData, EventEnvelope, EventKind, EventMatch, EventsWaitParams,
    Method, Request, ResponseResult, Subscription, SubscriptionEventData,
    SubscriptionEventEnvelope, SuccessResponse,
};
use crate::api::server::{
    dispatch_to_app_with_caller_timeout, dispatch_to_app_with_timeout, should_stop_connection,
    APP_RESPONSE_TIMEOUT, CONNECTION_POLL_INTERVAL,
};
use crate::api::subscriptions::ActiveSubscription;
use crate::api::subscriptions::{match_output, output_match_read_source};
use crate::api::{ApiRequestSender, EventHub};
use crate::ipc::LocalStream;

const AGENT_PROMPT_EFFECT_TIMEOUT_MS: u64 = 5_000;
const NOT_SUBMITTED_SETTLE_MS: u64 = 500;
const NOT_SUBMITTED_STREAK: u32 = 2;

pub(super) fn wait_for_output(
    request_id: String,
    params: crate::api::schema::PaneWaitForOutputParams,
    stream: &mut LocalStream,
    api_tx: &ApiRequestSender,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<String>> {
    crate::logging::api_wait_started(&request_id, &params.pane_id, params.timeout_ms);
    let deadline = params
        .timeout_ms
        .map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));

    let regex = match &params.r#match {
        crate::api::schema::OutputMatch::Regex { value } => match Regex::new(value) {
            Ok(regex) => Some(regex),
            Err(err) => {
                return Ok(Some(
                    serde_json::to_string(&ErrorResponse {
                        id: request_id,
                        error: ErrorBody {
                            code: "invalid_regex".into(),
                            message: err.to_string(),
                        },
                    })
                    .unwrap(),
                ));
            }
        },
        crate::api::schema::OutputMatch::Substring { .. } => None,
    };

    loop {
        if should_stop_connection(stream, running)? {
            crate::logging::api_wait_completed(&request_id, &params.pane_id, "client_disconnected");
            return Ok(None);
        }

        let read_request = Request {
            id: format!("{request_id}:read"),
            method: Method::PaneRead(crate::api::schema::PaneReadParams {
                pane_id: params.pane_id.clone(),
                source: output_match_read_source(&params.source),
                lines: params.lines,
                format: crate::api::schema::ReadFormat::Text,
                strip_ansi: params.strip_ansi,
                intent: crate::api::schema::ReadIntent::Passive,
            }),
        };
        let response =
            dispatch_to_app_with_timeout(read_request, api_tx, Some(APP_RESPONSE_TIMEOUT));
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&response) else {
            return Ok(Some(response));
        };
        if value.get("error").is_some() {
            let mut value = value;
            value["id"] = serde_json::Value::String(request_id.clone());
            return Ok(Some(serde_json::to_string(&value).unwrap()));
        }

        let read_value = value["result"]["read"].clone();
        let Ok(read) = serde_json::from_value::<crate::api::schema::PaneReadResult>(read_value)
        else {
            return Ok(Some(
                serde_json::to_string(&ErrorResponse {
                    id: request_id,
                    error: ErrorBody {
                        code: "internal_error".into(),
                        message: "failed to decode pane read result".into(),
                    },
                })
                .unwrap(),
            ));
        };

        let matched_line = match_output(&read.text, &params.r#match, regex.as_ref());
        if matched_line.is_some() {
            let revision = read.revision;
            crate::logging::api_wait_completed(&request_id, &params.pane_id, "matched");
            return Ok(Some(
                serde_json::to_string(&SuccessResponse {
                    id: request_id,
                    result: ResponseResult::OutputMatched {
                        pane_id: read.pane_id.clone(),
                        revision,
                        matched_line,
                        read,
                    },
                })
                .unwrap(),
            ));
        }

        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            crate::logging::api_wait_timed_out(&request_id, &params.pane_id);
            return Ok(Some(
                serde_json::to_string(&ErrorResponse {
                    id: request_id,
                    error: ErrorBody {
                        code: "timeout".into(),
                        message: "timed out waiting for output match".into(),
                    },
                })
                .unwrap(),
            ));
        }

        std::thread::sleep(CONNECTION_POLL_INTERVAL);
    }
}

pub(super) fn wait_for_agent(
    request_id: String,
    params: crate::api::schema::AgentWaitParams,
    stream: &mut LocalStream,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<String>> {
    let last_event_sequence = event_hub.current_sequence();
    let initial = match agent_get(&request_id, &params.target, api_tx) {
        Ok(agent) => agent,
        Err(response) => {
            return serde_json::to_string(&response)
                .map(Some)
                .map_err(std::io::Error::other);
        }
    };
    let until = agent_wait_statuses(params.until);
    if agent_wait_matches(&initial, &until, None) {
        return agent_wait_success(request_id, initial).map(Some);
    }

    match wait_for_resolved_agent(
        request_id.clone(),
        ResolvedAgentWait {
            target: params.target,
            until,
            timeout_ms: params.timeout_ms,
            initial,
            last_event_sequence,
            after_state_change_seq: None,
            accept_transient_status: true,
            timeout_kind: AgentWaitTimeoutKind::Status,
            rejection_probe: None,
            acceptance_probe: None,
            match_status: true,
        },
        stream,
        api_tx,
        event_hub,
        running,
    )? {
        Some(AgentWaitOutcome::Matched(agent)) => agent_wait_success(request_id, *agent).map(Some),
        Some(AgentWaitOutcome::Response(response)) => Ok(Some(response)),
        None => Ok(None),
    }
}

pub(super) fn prompt_agent(
    request_id: String,
    mut params: crate::api::schema::AgentPromptParams,
    stream: &mut LocalStream,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<String>> {
    let wait = params.wait.clone();
    let total_timeout_ms = wait.as_ref().and_then(|wait| wait.timeout_ms);
    let wait_started = std::time::Instant::now();

    let before_prompt = match agent_get_for_prompt(
        &request_id,
        &params.target,
        api_tx,
        total_timeout_ms,
        wait_started,
    ) {
        Ok(agent) => agent,
        Err(response) => {
            return serde_json::to_string(&response)
                .map(Some)
                .map_err(std::io::Error::other);
        }
    };
    let expected_agent = before_prompt
        .agent
        .as_deref()
        .and_then(crate::detect::parse_agent_label);
    let working_evidence_supported = crate::detect::queued_evidence_supported(expected_agent)
        || crate::detect::submitted_evidence_supported(expected_agent);
    let prompt_needle = prompt_evidence_needle(&params.text);
    let target = params.target.clone();

    // Capture the screen before writing so evidence of new text on screen can be
    // distinguished from text that was already there.
    let before_screen = read_agent_screen(&request_id, &target, api_tx).ok();
    let baseline = before_screen
        .as_deref()
        .map(|screen| crate::detect::manifest::composer_text_occurrences(screen, &prompt_needle))
        .unwrap_or(0);
    tracing::debug!(
        request_id = %request_id,
        baseline_read = before_screen.is_some(),
        text_baseline = baseline,
        "agent prompt evidence baseline"
    );

    if let Some(prompt_wait) = params.wait.as_mut() {
        prompt_wait.submission_deadline = total_timeout_ms
            .map(|timeout_ms| wait_started + std::time::Duration::from_millis(timeout_ms));
    }
    let last_event_sequence = event_hub.current_sequence();
    let prompt_request = Request {
        id: request_id.clone(),
        method: Method::AgentPrompt(params),
    };
    #[cfg(windows)]
    let prompt_response = dispatch_to_app_with_caller_timeout(
        prompt_request,
        api_tx,
        remaining_timeout_ms(total_timeout_ms, wait_started).map(std::time::Duration::from_millis),
    );
    #[cfg(not(windows))]
    let prompt_response = dispatch_to_app_with_timeout(prompt_request, api_tx, None);
    let Ok(prompted) = agent_from_response(&request_id, &prompt_response) else {
        return Ok(Some(prompt_response));
    };
    if !agent_wait_identity_matches(
        &prompted,
        &before_prompt.terminal_id,
        before_prompt.name.as_deref().filter(|name| *name == target),
        before_prompt.agent.as_deref(),
    ) {
        return agent_wait_not_running(request_id).map(Some);
    }

    let prompt_state_change_seq = prompted.state_change_seq;
    let mut initial = prompted;
    let prompt_started_working = initial.agent_status == crate::api::schema::AgentStatus::Working;

    if prompt_started_working && !working_evidence_supported {
        return agent_prompt_stalled(
            &request_id,
            &initial,
            "no acceptance evidence rule is available for this agent kind",
        )
        .map(Some);
    }

    let remaining = remaining_timeout_ms(total_timeout_ms, wait_started);
    let (effect_timeout_ms, timeout_kind) =
        prompt_effect_timeout(remaining, prompt_started_working);
    let earliest =
        std::time::Instant::now() + std::time::Duration::from_millis(NOT_SUBMITTED_SETTLE_MS);
    let rejection_probe = PromptRejectionProbe {
        needle: prompt_needle.clone(),
        baseline,
        earliest,
        expected_agent,
    };
    let acceptance_mode = if prompt_started_working {
        Some(PromptAcceptanceMode::Working {
            baseline_screen: before_screen,
        })
    } else if crate::detect::composer_evidence_supported(expected_agent) {
        Some(PromptAcceptanceMode::ComposerCleared)
    } else {
        None
    };
    let acceptance_probe = acceptance_mode.map(|mode| PromptAcceptanceProbe {
        mode,
        needle: prompt_needle.clone(),
        baseline,
        earliest,
        expected_agent,
    });
    let Some(outcome) = wait_for_resolved_agent(
        request_id.clone(),
        ResolvedAgentWait {
            target: target.clone(),
            until: prompt_activity_statuses(),
            timeout_ms: Some(effect_timeout_ms),
            initial,
            last_event_sequence,
            after_state_change_seq: Some(prompt_state_change_seq),
            accept_transient_status: true,
            timeout_kind,
            rejection_probe: Some(rejection_probe),
            acceptance_probe,
            match_status: !prompt_started_working,
        },
        stream,
        api_tx,
        event_hub,
        running,
    )?
    else {
        return Ok(None);
    };
    initial = match outcome {
        AgentWaitOutcome::Matched(agent) => *agent,
        AgentWaitOutcome::Response(response) => return Ok(Some(response)),
    };

    let Some(wait) = wait else {
        return agent_prompt_success(request_id, initial).map(Some);
    };
    let until = agent_wait_statuses(wait.until);
    if agent_wait_matches(&initial, &until, None) {
        return agent_prompt_success(request_id, initial).map(Some);
    }

    let Some(outcome) = wait_for_resolved_agent(
        request_id.clone(),
        ResolvedAgentWait {
            target,
            until,
            timeout_ms: remaining_timeout_ms(total_timeout_ms, wait_started),
            initial,
            // Replay from before submission so terminal lifecycle events consumed by
            // the activity gate still terminate this settled-state wait.
            last_event_sequence,
            after_state_change_seq: None,
            accept_transient_status: false,
            timeout_kind: AgentWaitTimeoutKind::Status,
            rejection_probe: None,
            acceptance_probe: None,
            match_status: true,
        },
        stream,
        api_tx,
        event_hub,
        running,
    )?
    else {
        return Ok(None);
    };
    let agent = match outcome {
        AgentWaitOutcome::Matched(agent) => *agent,
        AgentWaitOutcome::Response(response) => return Ok(Some(response)),
    };
    agent_prompt_success(request_id, agent).map(Some)
}

/// First non-empty line of the submitted text, bounded so a long paste does not
/// make screen evidence expensive to search for.
fn prompt_evidence_needle(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .chars()
        .take(80)
        .collect()
}

fn remaining_timeout_ms(total_ms: Option<u64>, started: std::time::Instant) -> Option<u64> {
    total_ms.map(|total_ms| {
        let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        total_ms.saturating_sub(elapsed_ms)
    })
}

fn prompt_effect_timeout(
    remaining_ms: Option<u64>,
    working_start: bool,
) -> (u64, AgentWaitTimeoutKind) {
    match remaining_ms {
        Some(timeout_ms) if timeout_ms <= AGENT_PROMPT_EFFECT_TIMEOUT_MS => {
            (timeout_ms, AgentWaitTimeoutKind::Status)
        }
        _ if working_start => (
            AGENT_PROMPT_EFFECT_TIMEOUT_MS,
            AgentWaitTimeoutKind::PromptAcceptance {
                timeout_ms: AGENT_PROMPT_EFFECT_TIMEOUT_MS,
            },
        ),
        _ => (
            AGENT_PROMPT_EFFECT_TIMEOUT_MS,
            AgentWaitTimeoutKind::PromptStalled {
                timeout_ms: AGENT_PROMPT_EFFECT_TIMEOUT_MS,
            },
        ),
    }
}

fn agent_prompt_success(
    request_id: String,
    agent: crate::api::schema::AgentInfo,
) -> std::io::Result<String> {
    serde_json::to_string(&SuccessResponse {
        id: request_id,
        result: ResponseResult::AgentPrompted { agent },
    })
    .map_err(std::io::Error::other)
}

/// The written-but-not-accepted outcome. Positive evidence says the text stayed
/// in the composer or the agent rejected it.
fn agent_prompt_not_submitted(
    request_id: &str,
    agent: &crate::api::schema::AgentInfo,
    evidence: &str,
) -> std::io::Result<String> {
    let status = format!("{:?}", agent.agent_status).to_ascii_lowercase();
    serde_json::to_string(&ErrorResponse {
        id: request_id.into(),
        error: ErrorBody {
            code: "agent_prompt_not_submitted".into(),
            message: format!(
                "agent prompt was written but not submitted: {evidence}; the input must not be resent without checking the agent (current status is {status})"
            ),
        },
    })
    .map_err(std::io::Error::other)
}

/// The written-but-not-confirmed outcome. `reason` distinguishes a missing
/// agent-kind rule from evidence that was expected but not observed.
fn agent_prompt_stalled(
    request_id: &str,
    agent: &crate::api::schema::AgentInfo,
    reason: &str,
) -> std::io::Result<String> {
    let status = format!("{:?}", agent.agent_status).to_ascii_lowercase();
    serde_json::to_string(&ErrorResponse {
        id: request_id.into(),
        error: ErrorBody {
            code: "agent_prompt_stalled".into(),
            message: format!(
                "agent prompt was written but {reason}; the input may still be in the agent's input box and must not be resent without checking the agent (current status is {status})"
            ),
        },
    })
    .map_err(std::io::Error::other)
}

fn prompt_acceptance_evidence(screen: &str, probe: &PromptAcceptanceProbe) -> bool {
    let text_occurrences =
        crate::detect::manifest::composer_text_occurrences(screen, &probe.needle);
    let new_text = text_occurrences > probe.baseline;
    match &probe.mode {
        PromptAcceptanceMode::Working { baseline_screen } => {
            let queued = new_text
                && crate::detect::composer_queued_evidence(
                    probe.expected_agent,
                    screen,
                    "",
                    "",
                    &probe.needle,
                );
            // A steered prompt is a submitted entry after all the history this
            // sample shares with the pre-submission screen, above an input box
            // that no longer holds a draft, a command or a selection list. A
            // window that grows reveals old entries above that history, which
            // never count. Without a baseline, or when the two screens share no
            // history, there is no evidence either way.
            let new_entries = baseline_screen.as_deref().and_then(|baseline| {
                crate::detect::composer_submitted_evidence_since(
                    probe.expected_agent,
                    baseline,
                    screen,
                    &probe.needle,
                )
            });
            let holds_input = composer_holds_input(probe.expected_agent, screen);
            let steered = new_entries.is_some_and(|new_entries| new_entries > 0) && !holds_input;
            tracing::debug!(
                text_occurrences,
                text_baseline = probe.baseline,
                new_entries = ?new_entries,
                holds_input,
                queued,
                steered,
                "agent prompt working evidence sample"
            );
            queued || steered
        }
        PromptAcceptanceMode::ComposerCleared => {
            new_text && !composer_holds_input(probe.expected_agent, screen)
        }
    }
}

fn composer_holds_input(agent: Option<crate::detect::Agent>, screen: &str) -> bool {
    crate::detect::composer_match(agent, screen, "", "").is_some_and(|composer| {
        matches!(
            composer.state,
            crate::detect::ComposerState::SelectionList
                | crate::detect::ComposerState::PartialCommand
                | crate::detect::ComposerState::Draft
        )
    })
}

fn prompt_not_submitted_evidence(
    screen: &str,
    needle: &str,
    baseline: usize,
    agent: Option<crate::detect::Agent>,
) -> Option<&'static str> {
    if needle.is_empty()
        || crate::detect::manifest::composer_text_occurrences(screen, needle) <= baseline
    {
        return None;
    }
    if crate::detect::composer_match(agent, screen, "", "").is_some_and(|composer| {
        matches!(
            composer.state,
            crate::detect::ComposerState::Draft | crate::detect::ComposerState::PartialCommand
        ) && crate::detect::manifest::composer_text_occurrences(&composer.region_text, needle) > 0
    }) {
        return Some("the text is still in the agent's input box");
    }
    None
}

fn read_agent_screen(
    request_id: &str,
    target: &str,
    api_tx: &ApiRequestSender,
) -> Result<String, ErrorResponse> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: format!("{request_id}:read"),
            method: Method::AgentRead(crate::api::schema::AgentReadParams {
                target: target.to_string(),
                source: crate::api::schema::ReadSource::Detection,
                lines: Some(80),
                format: crate::api::schema::ReadFormat::Text,
                strip_ansi: true,
            }),
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).map_err(|_| ErrorResponse {
        id: request_id.into(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode agent read response".into(),
        },
    })?;
    if let Some(error) = response_error(request_id, &value) {
        return Err(error);
    }
    Ok(value["result"]["read"]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string())
}

fn response_error(request_id: &str, value: &serde_json::Value) -> Option<ErrorResponse> {
    value.get("error")?;
    let error = serde_json::from_value(value["error"].clone()).ok()?;
    Some(ErrorResponse {
        id: request_id.into(),
        error,
    })
}

struct ResolvedAgentWait {
    target: String,
    until: Vec<crate::api::schema::AgentStatus>,
    timeout_ms: Option<u64>,
    initial: crate::api::schema::AgentInfo,
    last_event_sequence: u64,
    after_state_change_seq: Option<u64>,
    accept_transient_status: bool,
    timeout_kind: AgentWaitTimeoutKind,
    rejection_probe: Option<PromptRejectionProbe>,
    acceptance_probe: Option<PromptAcceptanceProbe>,
    match_status: bool,
}

struct PromptRejectionProbe {
    needle: String,
    baseline: usize,
    earliest: std::time::Instant,
    expected_agent: Option<crate::detect::Agent>,
}

enum PromptAcceptanceMode {
    /// The agent was already working: the prompt must appear as a queued
    /// message or, when the pre-submission screen was read, as a steered
    /// entry after the history both screens share.
    Working {
        baseline_screen: Option<String>,
    },
    ComposerCleared,
}

struct PromptAcceptanceProbe {
    mode: PromptAcceptanceMode,
    needle: String,
    baseline: usize,
    earliest: std::time::Instant,
    expected_agent: Option<crate::detect::Agent>,
}

impl PromptAcceptanceProbe {
    fn required_streak(&self) -> u32 {
        match self.mode {
            PromptAcceptanceMode::Working { .. } => 1,
            PromptAcceptanceMode::ComposerCleared => NOT_SUBMITTED_STREAK,
        }
    }
}

#[derive(Clone, Copy)]
enum AgentWaitTimeoutKind {
    Status,
    PromptStalled { timeout_ms: u64 },
    PromptAcceptance { timeout_ms: u64 },
}

enum AgentWaitOutcome {
    Matched(Box<crate::api::schema::AgentInfo>),
    Response(String),
}

fn wait_for_resolved_agent(
    request_id: String,
    wait: ResolvedAgentWait,
    stream: &mut LocalStream,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<AgentWaitOutcome>> {
    let deadline = wait
        .timeout_ms
        .map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));
    let expected_terminal_id = wait.initial.terminal_id.clone();
    let expected_name = wait
        .initial
        .name
        .as_ref()
        .filter(|name| name.as_str() == wait.target)
        .cloned();
    let expected_agent = wait.initial.agent.clone();
    let pane_id = wait.initial.pane_id.clone();
    let mut last_event_sequence = wait.last_event_sequence;
    let mut not_submitted_streak = 0;
    let mut acceptance_streak = 0;

    loop {
        if should_stop_connection(stream, running)? {
            return Ok(None);
        }

        let mut should_probe = false;
        let mut matched_event_status = None;
        for (sequence, event) in event_hub.events_after(last_event_sequence) {
            last_event_sequence = sequence;
            match event.data {
                EventData::PaneAgentDetected {
                    pane_id: event_pane,
                    agent,
                    released,
                    final_status,
                    ..
                } if event_pane == pane_id => {
                    if released {
                        if wait.match_status {
                            if let Some(status) = final_status
                                .filter(|status| wait.until.contains(status))
                                .or(matched_event_status)
                            {
                                let mut matched = wait.initial.clone();
                                matched.agent_status = status;
                                return Ok(Some(AgentWaitOutcome::Matched(Box::new(matched))));
                            }
                        }
                        return agent_wait_not_running(request_id)
                            .map(AgentWaitOutcome::Response)
                            .map(Some);
                    }
                    if agent.is_some() && expected_agent.is_some() && agent != expected_agent {
                        return agent_wait_not_running(request_id)
                            .map(AgentWaitOutcome::Response)
                            .map(Some);
                    }
                    should_probe = true;
                }
                EventData::PaneAgentStatusChanged {
                    pane_id: event_pane,
                    agent_status,
                    ..
                } if event_pane == pane_id => {
                    if wait.match_status
                        && wait.accept_transient_status
                        && wait.until.contains(&agent_status)
                    {
                        matched_event_status = Some(agent_status);
                    }
                    should_probe = true;
                }
                EventData::PaneUpdated { pane } if pane.pane_id == pane_id => should_probe = true,
                EventData::PaneMoved {
                    previous_pane_id, ..
                } if previous_pane_id == pane_id => {
                    return agent_wait_not_running(request_id)
                        .map(AgentWaitOutcome::Response)
                        .map(Some);
                }
                EventData::PaneClosed {
                    pane_id: event_pane,
                    ..
                }
                | EventData::PaneExited {
                    pane_id: event_pane,
                    ..
                } if event_pane == pane_id => {
                    return agent_wait_not_running(request_id)
                        .map(AgentWaitOutcome::Response)
                        .map(Some);
                }
                _ => {}
            }
        }

        if should_probe {
            let current = match agent_get(&request_id, &wait.target, api_tx) {
                Ok(agent) => agent,
                Err(response) => {
                    return agent_wait_probe_error(response)
                        .map(AgentWaitOutcome::Response)
                        .map(Some);
                }
            };
            if !agent_wait_identity_matches(
                &current,
                &expected_terminal_id,
                expected_name.as_deref(),
                expected_agent.as_deref(),
            ) {
                return agent_wait_not_running(request_id)
                    .map(AgentWaitOutcome::Response)
                    .map(Some);
            }
            if wait.match_status {
                if let Some(status) = matched_event_status {
                    let mut matched = current;
                    matched.agent_status = status;
                    return Ok(Some(AgentWaitOutcome::Matched(Box::new(matched))));
                }
                if agent_wait_matches(&current, &wait.until, wait.after_state_change_seq) {
                    return Ok(Some(AgentWaitOutcome::Matched(Box::new(current))));
                }
            }
        }

        if wait.rejection_probe.is_some() || wait.acceptance_probe.is_some() {
            let screen = read_agent_screen(&request_id, &wait.target, api_tx);
            if let Err(error) = &screen {
                tracing::debug!(
                    request_id = %request_id,
                    code = %error.error.code,
                    "agent prompt evidence screen read failed"
                );
            }
            if let Ok(screen) = screen {
                if let Some(probe) = &wait.rejection_probe {
                    if std::time::Instant::now() >= probe.earliest {
                        if let Some(evidence) = prompt_not_submitted_evidence(
                            &screen,
                            &probe.needle,
                            probe.baseline,
                            probe.expected_agent,
                        ) {
                            not_submitted_streak += 1;
                            if not_submitted_streak >= NOT_SUBMITTED_STREAK {
                                let current = agent_get(&request_id, &wait.target, api_tx)
                                    .unwrap_or_else(|_| wait.initial.clone());
                                return agent_prompt_not_submitted(&request_id, &current, evidence)
                                    .map(AgentWaitOutcome::Response)
                                    .map(Some);
                            }
                        } else {
                            not_submitted_streak = 0;
                        }
                    }
                }

                if let Some(probe) = &wait.acceptance_probe {
                    if std::time::Instant::now() >= probe.earliest {
                        if prompt_acceptance_evidence(&screen, probe) {
                            acceptance_streak += 1;
                            if acceptance_streak >= probe.required_streak() {
                                let current = agent_get(&request_id, &wait.target, api_tx)
                                    .unwrap_or_else(|_| wait.initial.clone());
                                return Ok(Some(AgentWaitOutcome::Matched(Box::new(current))));
                            }
                        } else {
                            acceptance_streak = 0;
                        }
                    }
                }
            }
        }

        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            let current = match agent_get(&request_id, &wait.target, api_tx) {
                Ok(agent) => agent,
                Err(response) => {
                    return agent_wait_probe_error(response)
                        .map(AgentWaitOutcome::Response)
                        .map(Some);
                }
            };
            if !agent_wait_identity_matches(
                &current,
                &expected_terminal_id,
                expected_name.as_deref(),
                expected_agent.as_deref(),
            ) {
                return agent_wait_not_running(request_id)
                    .map(AgentWaitOutcome::Response)
                    .map(Some);
            }
            if wait.match_status
                && agent_wait_matches(&current, &wait.until, wait.after_state_change_seq)
            {
                return Ok(Some(AgentWaitOutcome::Matched(Box::new(current))));
            }
            return agent_wait_timeout(request_id, wait.timeout_kind, &current)
                .map(AgentWaitOutcome::Response)
                .map(Some);
        }
        std::thread::sleep(CONNECTION_POLL_INTERVAL);
    }
}

fn prompt_activity_statuses() -> Vec<crate::api::schema::AgentStatus> {
    vec![
        crate::api::schema::AgentStatus::Working,
        crate::api::schema::AgentStatus::Blocked,
    ]
}

fn agent_wait_statuses(
    until: Vec<crate::api::schema::AgentStatus>,
) -> Vec<crate::api::schema::AgentStatus> {
    if until.is_empty() {
        vec![
            crate::api::schema::AgentStatus::Idle,
            crate::api::schema::AgentStatus::Done,
            crate::api::schema::AgentStatus::Blocked,
        ]
    } else {
        until
    }
}

fn agent_wait_identity_matches(
    agent: &crate::api::schema::AgentInfo,
    expected_terminal_id: &str,
    expected_name: Option<&str>,
    expected_agent: Option<&str>,
) -> bool {
    agent.terminal_id == expected_terminal_id
        && expected_name.is_none_or(|name| agent.name.as_deref() == Some(name))
        && match (expected_agent, agent.agent.as_deref()) {
            (Some(expected), Some(current)) => expected == current,
            (Some(_), None) => agent.name.is_some(),
            (None, _) => true,
        }
}

fn agent_wait_matches(
    agent: &crate::api::schema::AgentInfo,
    until: &[crate::api::schema::AgentStatus],
    after_state_change_seq: Option<u64>,
) -> bool {
    until.contains(&agent.agent_status)
        && after_state_change_seq.is_none_or(|baseline| agent.state_change_seq > baseline)
}

fn agent_get(
    request_id: &str,
    target: &str,
    api_tx: &ApiRequestSender,
) -> Result<crate::api::schema::AgentInfo, ErrorResponse> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: format!("{request_id}:agent"),
            method: Method::AgentGet(crate::api::schema::AgentTarget {
                target: target.to_string(),
            }),
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    agent_from_response(request_id, &response)
}

fn agent_get_for_prompt(
    request_id: &str,
    target: &str,
    api_tx: &ApiRequestSender,
    total_timeout_ms: Option<u64>,
    started: std::time::Instant,
) -> Result<crate::api::schema::AgentInfo, ErrorResponse> {
    let request = Request {
        id: format!("{request_id}:agent"),
        method: Method::AgentGet(crate::api::schema::AgentTarget {
            target: target.to_string(),
        }),
    };
    let remaining_ms = remaining_timeout_ms(total_timeout_ms, started);
    let response = match remaining_ms {
        Some(timeout_ms) if timeout_ms <= APP_RESPONSE_TIMEOUT.as_millis() as u64 => {
            dispatch_to_app_with_caller_timeout(
                request,
                api_tx,
                Some(std::time::Duration::from_millis(timeout_ms)),
            )
        }
        _ => dispatch_to_app_with_timeout(request, api_tx, Some(APP_RESPONSE_TIMEOUT)),
    };
    agent_from_response(request_id, &response)
}

fn agent_from_response(
    request_id: &str,
    response: &str,
) -> Result<crate::api::schema::AgentInfo, ErrorResponse> {
    let value: serde_json::Value = serde_json::from_str(response).map_err(|_| ErrorResponse {
        id: request_id.into(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode agent response".into(),
        },
    })?;
    if value.get("error").is_some() {
        let error = serde_json::from_value(value["error"].clone()).map_err(|_| ErrorResponse {
            id: request_id.into(),
            error: ErrorBody {
                code: "internal_error".into(),
                message: "failed to decode agent error".into(),
            },
        })?;
        return Err(ErrorResponse {
            id: request_id.into(),
            error,
        });
    }
    serde_json::from_value(value["result"]["agent"].clone()).map_err(|_| ErrorResponse {
        id: request_id.into(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode agent result".into(),
        },
    })
}

fn agent_wait_success(
    request_id: String,
    agent: crate::api::schema::AgentInfo,
) -> std::io::Result<String> {
    serde_json::to_string(&SuccessResponse {
        id: request_id,
        result: ResponseResult::AgentInfo { agent },
    })
    .map_err(std::io::Error::other)
}

fn agent_wait_timeout(
    request_id: String,
    kind: AgentWaitTimeoutKind,
    current: &crate::api::schema::AgentInfo,
) -> std::io::Result<String> {
    let (code, message) = match kind {
        AgentWaitTimeoutKind::Status => {
            ("timeout", "timed out waiting for agent status".to_string())
        }
        AgentWaitTimeoutKind::PromptStalled { timeout_ms } => {
            let status = format!("{:?}", current.agent_status).to_ascii_lowercase();
            (
                "agent_prompt_stalled",
                format!(
                    "agent prompt produced no observed working or blocked state within {timeout_ms} ms; input was written and may still be in the agent's input box, so check the agent before resending (current status is {status})"
                ),
            )
        }
        AgentWaitTimeoutKind::PromptAcceptance { timeout_ms } => {
            let status = format!("{:?}", current.agent_status).to_ascii_lowercase();
            (
                "agent_prompt_stalled",
                format!(
                    "agent prompt was written but no acceptance evidence was observed within {timeout_ms} ms; the input may still be in the agent's input box and must not be resent without checking the agent (current status is {status})"
                ),
            )
        }
    };
    serde_json::to_string(&ErrorResponse {
        id: request_id,
        error: ErrorBody {
            code: code.into(),
            message,
        },
    })
    .map_err(std::io::Error::other)
}

fn agent_wait_not_running(request_id: String) -> std::io::Result<String> {
    serde_json::to_string(&ErrorResponse {
        id: request_id,
        error: ErrorBody {
            code: "agent_not_running".into(),
            message: "agent is no longer running in the target pane".into(),
        },
    })
    .map_err(std::io::Error::other)
}

fn agent_wait_probe_error(response: ErrorResponse) -> std::io::Result<String> {
    if response.error.code == "agent_not_found" {
        return agent_wait_not_running(response.id);
    }
    serde_json::to_string(&response).map_err(std::io::Error::other)
}

pub(super) fn wait_for_event(
    request_id: String,
    params: EventsWaitParams,
    stream: &mut LocalStream,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<String>> {
    let deadline = params
        .timeout_ms
        .map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));

    let subscription = match event_match_subscription(&request_id, params.match_event) {
        Ok(subscription) => subscription,
        Err(response) => return Ok(Some(serde_json::to_string(&response).unwrap())),
    };
    let mut active = match ActiveSubscription::new(
        subscription,
        &request_id,
        0,
        api_tx,
        event_hub,
        event_hub.current_sequence(),
    ) {
        Ok(active) => active,
        Err(response) => return Ok(Some(serde_json::to_string(&response).unwrap())),
    };

    loop {
        if should_stop_connection(stream, running)? {
            return Ok(None);
        }

        match active.poll_for_wait(api_tx, event_hub) {
            Ok(Some(event)) => return Ok(Some(wait_matched_response(&request_id, event))),
            Ok(None) => {}
            Err(mut response) if response.error.code == "pane_not_found" => {
                response.id = request_id;
                return serde_json::to_string(&response)
                    .map(Some)
                    .map_err(std::io::Error::other);
            }
            Err(_) => {}
        }

        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            return Ok(Some(
                serde_json::to_string(&ErrorResponse {
                    id: request_id,
                    error: ErrorBody {
                        code: "timeout".into(),
                        message: "timed out waiting for event match".into(),
                    },
                })
                .unwrap(),
            ));
        }

        std::thread::sleep(CONNECTION_POLL_INTERVAL);
    }
}

fn event_match_subscription(
    request_id: &str,
    match_event: EventMatch,
) -> Result<Subscription, ErrorResponse> {
    match match_event {
        EventMatch::PaneAgentStatusChanged {
            pane_id,
            agent_status,
        } => Ok(Subscription::PaneAgentStatusChanged {
            pane_id,
            agent_status: Some(agent_status),
        }),
        _ => Err(ErrorResponse {
            id: request_id.into(),
            error: ErrorBody {
                code: "unsupported_event_wait_match".into(),
                message: "events.wait currently supports pane agent status matches".into(),
            },
        }),
    }
}

fn wait_matched_response(request_id: &str, event: serde_json::Value) -> String {
    let Ok(event) = serde_json::from_value::<SubscriptionEventEnvelope>(event) else {
        return serde_json::to_string(&ErrorResponse {
            id: request_id.into(),
            error: ErrorBody {
                code: "internal_error".into(),
                message: "failed to decode matched event".into(),
            },
        })
        .unwrap();
    };

    let SubscriptionEventData::PaneAgentStatusChanged(data) = event.data else {
        return serde_json::to_string(&ErrorResponse {
            id: request_id.into(),
            error: ErrorBody {
                code: "unsupported_event_wait_match".into(),
                message: "events.wait currently supports pane agent status matches".into(),
            },
        })
        .unwrap();
    };

    serde_json::to_string(&SuccessResponse {
        id: request_id.into(),
        result: ResponseResult::WaitMatched {
            event: EventEnvelope {
                event: EventKind::PaneAgentStatusChanged,
                data: EventData::PaneAgentStatusChanged {
                    pane_id: data.pane_id,
                    workspace_id: data.workspace_id,
                    agent_status: data.agent_status,
                    agent: data.agent,
                    title: data.title,
                    display_agent: data.display_agent,
                    state_labels: data.state_labels,
                },
            },
        },
    })
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_wait_probe_only_translates_agent_disappearance() {
        let disappeared = agent_wait_probe_error(ErrorResponse {
            id: "wait".into(),
            error: ErrorBody {
                code: "agent_not_found".into(),
                message: "missing".into(),
            },
        })
        .unwrap();
        let disappeared: ErrorResponse = serde_json::from_str(&disappeared).unwrap();
        assert_eq!(disappeared.id, "wait");
        assert_eq!(disappeared.error.code, "agent_not_running");

        let unavailable = agent_wait_probe_error(ErrorResponse {
            id: "wait".into(),
            error: ErrorBody {
                code: "server_unavailable".into(),
                message: "timed out waiting for app response".into(),
            },
        })
        .unwrap();
        let unavailable: ErrorResponse = serde_json::from_str(&unavailable).unwrap();
        assert_eq!(unavailable.id, "wait");
        assert_eq!(unavailable.error.code, "server_unavailable");
    }

    fn test_agent() -> crate::api::schema::AgentInfo {
        serde_json::from_value(serde_json::json!({
            "terminal_id": "term_1",
            "agent_status": "idle",
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "pane_id": "w1:p1",
            "focused": true,
            "revision": 0
        }))
        .expect("test agent")
    }

    #[test]
    fn prompt_success_has_no_outcome_fields() {
        let response = agent_prompt_success("req".into(), test_agent()).unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["result"]["type"], "agent_prompted");
        assert!(response["result"].get("outcome").is_none());
        assert!(response["result"].get("merged_draft").is_none());
    }

    #[test]
    fn not_submitted_evidence_is_the_current_composer_text_only() {
        let needle = "hello";
        assert_eq!(
            prompt_not_submitted_evidence(
                "Unrecognized command 'hello'\n",
                needle,
                1,
                Some(crate::detect::Agent::Codex)
            ),
            None,
            "a stale rejection line must not count against the baseline"
        );
        assert_eq!(
            prompt_not_submitted_evidence("Unrecognized command 'hello'\n", needle, 0, None),
            None,
            "the generic path must not classify a rejection without composer evidence"
        );
        assert_eq!(
            prompt_not_submitted_evidence(
                "\u{203a} hello\n",
                needle,
                0,
                Some(crate::detect::Agent::Codex)
            ),
            Some("the text is still in the agent's input box")
        );
        assert_eq!(
            prompt_not_submitted_evidence(
                "\u{203a} /moXYZ-not-a-command\n",
                "/moXYZ-not-a-command",
                0,
                Some(crate::detect::Agent::Codex)
            ),
            Some("the text is still in the agent's input box")
        );
        assert_eq!(
            prompt_not_submitted_evidence(
                "idle screen",
                needle,
                0,
                Some(crate::detect::Agent::Codex)
            ),
            None
        );
    }

    #[test]
    fn prompt_effect_timeout_respects_caller_budget() {
        assert!(matches!(
            prompt_effect_timeout(Some(500), true),
            (500, AgentWaitTimeoutKind::Status)
        ));
        assert!(matches!(
            prompt_effect_timeout(None, true),
            (
                5000,
                AgentWaitTimeoutKind::PromptAcceptance { timeout_ms: 5000 }
            )
        ));
        assert!(matches!(
            prompt_effect_timeout(None, false),
            (
                5000,
                AgentWaitTimeoutKind::PromptStalled { timeout_ms: 5000 }
            )
        ));
    }

    #[test]
    fn acceptance_evidence_requires_queued_message_or_cleared_composer() {
        let queued = PromptAcceptanceProbe {
            mode: PromptAcceptanceMode::Working {
                baseline_screen: Some(String::new()),
            },
            needle: "hello".into(),
            baseline: 0,
            earliest: std::time::Instant::now(),
            expected_agent: Some(crate::detect::Agent::Codex),
        };
        assert!(prompt_acceptance_evidence(
            "• Messages to be submitted after next tool call\n  hello\n",
            &queued
        ));
        assert!(!prompt_acceptance_evidence("\u{203a} hello\n", &queued));

        let cleared = PromptAcceptanceProbe {
            mode: PromptAcceptanceMode::ComposerCleared,
            needle: "hello".into(),
            baseline: 0,
            earliest: std::time::Instant::now(),
            expected_agent: Some(crate::detect::Agent::Codex),
        };
        assert!(prompt_acceptance_evidence(
            "hello\n\n\u{203a} Ask Codex to do anything\n",
            &cleared
        ));
        assert!(!prompt_acceptance_evidence("\u{203a} hello\n", &cleared));
    }

    /// A private-config manifest override, so these tests exercise the evidence
    /// contract with synthetic rules rather than the bundled agent grammar.
    struct SyntheticManifests {
        _lock: std::sync::MutexGuard<'static, ()>,
        previous: Option<std::ffi::OsString>,
        dir: std::path::PathBuf,
    }

    impl SyntheticManifests {
        fn install(manifests: &[(&str, &str)]) -> Self {
            let lock = crate::config::test_config_env_lock()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous = std::env::var_os("XDG_CONFIG_HOME");
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "herdr-wait-manifest-{}-{stamp}",
                std::process::id()
            ));
            let overrides = dir
                .join(crate::config::app_dir_name())
                .join("agent-detection");
            std::fs::create_dir_all(&overrides).unwrap();
            for (agent, manifest) in manifests {
                std::fs::write(overrides.join(format!("{agent}.toml")), manifest).unwrap();
            }
            std::env::set_var("XDG_CONFIG_HOME", &dir);
            crate::detect::manifest::reload_manifests();
            Self {
                _lock: lock,
                previous,
                dir,
            }
        }
    }

    impl Drop for SyntheticManifests {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
            crate::detect::manifest::reload_manifests();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    // Codex is only the registry key; every gate below is synthetic.
    const SYNTHETIC_CODEX: &str = r#"
id = "codex"
version = "9999.01.01.1"
min_engine_version = 5

[[rules]]
id = "base"
state = "idle"
contains = ["TEST-IDLE"]

[[composer]]
id = "selection"
state = "selection_list"
priority = 1000
region = "whole_recent"
contains = ["TEST-SELECTION"]

[[composer]]
id = "partial"
state = "partial_command"
priority = 900
region = "current_prompt_line"
regex = ['(?m)^›[ \t]+/']

[[composer]]
id = "draft"
state = "draft"
priority = 600
region = "current_prompt_line"
regex = ['(?m)^›[ \t]+[^/\r\n]']
not = [{ contains = ["TEST-EMPTY"] }]

[[composer]]
id = "queued"
state = "queued_evidence"
priority = 100
region = "whole_recent"
requires_submitted_text = true
contains = ["TEST-QUEUED"]

[[composer]]
id = "submitted"
state = "submitted_evidence"
priority = 100
region = "before_current_prompt_marker"
requires_submitted_text = true
regex = ['(?m)^›(?: |$)']
"#;

    // Another agent kind with input-box rules but no submitted-evidence rule.
    const SYNTHETIC_CLAUDE: &str = r#"
id = "claude"
version = "9999.01.01.1"
min_engine_version = 5

[[rules]]
id = "base"
state = "idle"
contains = ["TEST-IDLE"]

[[composer]]
id = "draft"
state = "draft"
region = "current_prompt_line"
regex = ['(?m)^›[ \t]+[^/\r\n]']
not = [{ contains = ["TEST-EMPTY"] }]
"#;

    const STEER_NEEDLE: &str = "review the diff";
    const SCREEN_HEAD: &str = "TEST-HEADER\n\u{203a} first request\n\u{2022} reply\n";
    const STEER_ENTRY: &str = "\u{203a} review the diff\n";
    const SCREEN_TAIL: &str = "\u{2022} TEST-WORKING\n\n";
    const EMPTY_INPUT: &str = "\u{203a} TEST-EMPTY\n  TEST-FOOTER\n";
    const DRAFT_INPUT: &str = "\u{203a} review the diff\n  TEST-FOOTER\n";

    fn working_screen(head: &str, entries: &[&str], input: &str) -> String {
        let mut screen = head.to_string();
        for entry in entries {
            screen.push_str(entry);
        }
        screen.push_str(SCREEN_TAIL);
        screen.push_str(input);
        screen
    }

    fn working_probe(
        before_screen: Option<&str>,
        expected_agent: Option<crate::detect::Agent>,
    ) -> PromptAcceptanceProbe {
        PromptAcceptanceProbe {
            mode: PromptAcceptanceMode::Working {
                baseline_screen: before_screen.map(str::to_string),
            },
            needle: STEER_NEEDLE.into(),
            baseline: before_screen
                .map(|screen| {
                    crate::detect::manifest::composer_text_occurrences(screen, STEER_NEEDLE)
                })
                .unwrap_or(0),
            earliest: std::time::Instant::now(),
            expected_agent,
        }
    }

    #[test]
    fn a_prompt_steered_into_a_working_turn_is_accepted() {
        let _manifests = SyntheticManifests::install(&[("codex", SYNTHETIC_CODEX)]);
        let codex = Some(crate::detect::Agent::Codex);
        assert!(crate::detect::submitted_evidence_supported(codex));
        let before = working_screen(SCREEN_HEAD, &[], EMPTY_INPUT);
        let steered = working_screen(SCREEN_HEAD, &[STEER_ENTRY], EMPTY_INPUT);
        assert!(prompt_acceptance_evidence(
            &steered,
            &working_probe(Some(&before), codex)
        ));

        // A steer whose first line wrapped onto an indented continuation line
        // is still the same entry.
        let wrapped = working_screen(SCREEN_HEAD, &["\u{203a} review the\n  diff\n"], EMPTY_INPUT);
        assert!(prompt_acceptance_evidence(
            &wrapped,
            &working_probe(Some(&before), codex)
        ));
    }

    #[test]
    fn a_steer_still_in_the_input_line_is_not_accepted() {
        let _manifests = SyntheticManifests::install(&[("codex", SYNTHETIC_CODEX)]);
        let codex = Some(crate::detect::Agent::Codex);
        let before = working_screen(SCREEN_HEAD, &[], EMPTY_INPUT);
        let still_typed = working_screen(SCREEN_HEAD, &[], DRAFT_INPUT);
        assert!(!prompt_acceptance_evidence(
            &still_typed,
            &working_probe(Some(&before), codex)
        ));

        // A matching history entry does not count while the input box holds a
        // draft, a slash command or a selection list.
        for input in [
            DRAFT_INPUT,
            "\u{203a} /model\n  TEST-FOOTER\n",
            "\u{203a} TEST-EMPTY\nTEST-SELECTION\n",
        ] {
            let steered_with_input = working_screen(SCREEN_HEAD, &[STEER_ENTRY], input);
            assert!(
                !prompt_acceptance_evidence(
                    &steered_with_input,
                    &working_probe(Some(&before), codex)
                ),
                "{input:?}"
            );
        }
    }

    #[test]
    fn a_steered_entry_already_in_the_baseline_is_not_accepted() {
        let _manifests = SyntheticManifests::install(&[("codex", SYNTHETIC_CODEX)]);
        let codex = Some(crate::detect::Agent::Codex);
        let before = working_screen(SCREEN_HEAD, &[STEER_ENTRY], EMPTY_INPUT);
        assert!(!prompt_acceptance_evidence(
            &before,
            &working_probe(Some(&before), codex)
        ));

        // Without a pre-submission screen there is no baseline to rise above.
        assert!(!prompt_acceptance_evidence(
            &before,
            &working_probe(None, codex)
        ));
    }

    #[test]
    fn revealing_the_marker_of_an_older_entry_is_not_a_new_steer() {
        let _manifests = SyntheticManifests::install(&[("codex", SYNTHETIC_CODEX)]);
        let codex = Some(crate::detect::Agent::Codex);
        // The detection window starts inside an older multiline message, so its
        // continuation with the prompt text is visible but its marker is not.
        let old_tail = "  review the diff\n\u{2022} reply\n";
        let before = working_screen(old_tail, &[], EMPTY_INPUT);
        // The window grows by one row and reveals that message's marker. No
        // text was submitted: the prompt text count stays 1 while the entry
        // count rises from 0 to 1. The revealed entry lies above the history
        // both screens share, so it is not new.
        let revealed_head = format!("\u{203a} Old multiline request\n{old_tail}");
        let revealed = working_screen(&revealed_head, &[], EMPTY_INPUT);
        assert_eq!(
            crate::detect::composer_submitted_evidence_since(
                codex,
                &before,
                &revealed,
                STEER_NEEDLE
            ),
            Some(0)
        );
        assert!(!prompt_acceptance_evidence(
            &revealed,
            &working_probe(Some(&before), codex)
        ));

        // A reveal together with a real steer: the steer follows the shared
        // history, so the screens order it as new while the reveal stays old.
        let revealed_and_steered = working_screen(&revealed_head, &[STEER_ENTRY], EMPTY_INPUT);
        assert_eq!(
            crate::detect::composer_submitted_evidence_since(
                codex,
                &before,
                &revealed_and_steered,
                STEER_NEEDLE
            ),
            Some(1)
        );
        assert!(prompt_acceptance_evidence(
            &revealed_and_steered,
            &working_probe(Some(&before), codex)
        ));
    }

    #[test]
    fn revealing_an_old_entry_with_the_same_text_is_not_a_new_steer() {
        let _manifests = SyntheticManifests::install(&[("codex", SYNTHETIC_CODEX)]);
        let codex = Some(crate::detect::Agent::Codex);
        // An old message repeats the prompt text on its continuation line. The
        // window first shows only that continuation, then grows by one row and
        // reveals the message's own first line: the text count goes 1 -> 2 and
        // the entry count 0 -> 1 with nothing submitted.
        let continuation = "  review the diff\n\u{2022} reply\n";
        let before = working_screen(continuation, &[], EMPTY_INPUT);
        let revealed = working_screen(
            &format!("\u{203a} review the diff\n{continuation}"),
            &[],
            EMPTY_INPUT,
        );
        assert!(!prompt_acceptance_evidence(
            &revealed,
            &working_probe(Some(&before), codex)
        ));
    }

    #[test]
    fn revealing_a_whole_old_entry_above_existing_history_is_not_a_new_steer() {
        let _manifests = SyntheticManifests::install(&[("codex", SYNTHETIC_CODEX)]);
        let codex = Some(crate::detect::Agent::Codex);
        let anchor = "\u{2022} reply\n";
        let before = working_screen(anchor, &[], EMPTY_INPUT);
        let revealed = working_screen(&format!("{STEER_ENTRY}{anchor}"), &[], EMPTY_INPUT);
        assert!(!prompt_acceptance_evidence(
            &revealed,
            &working_probe(Some(&before), codex)
        ));
    }

    #[test]
    fn screens_without_shared_history_are_not_steer_evidence() {
        let _manifests = SyntheticManifests::install(&[("codex", SYNTHETIC_CODEX)]);
        let codex = Some(crate::detect::Agent::Codex);
        let before = working_screen("\u{2022} older output\n", &[], EMPTY_INPUT);
        let unrelated = working_screen("\u{2022} other output\n", &[STEER_ENTRY], EMPTY_INPUT);
        assert_eq!(
            crate::detect::composer_submitted_evidence_since(
                codex,
                &before,
                &unrelated,
                STEER_NEEDLE
            ),
            None,
            "screens that share no history cannot be ordered"
        );
        assert!(!prompt_acceptance_evidence(
            &unrelated,
            &working_probe(Some(&before), codex)
        ));
    }

    #[test]
    fn a_steer_repeating_an_earlier_prompt_is_accepted() {
        let _manifests = SyntheticManifests::install(&[("codex", SYNTHETIC_CODEX)]);
        let codex = Some(crate::detect::Agent::Codex);
        // The same text was submitted before and is still on screen; the new
        // steer follows the history both screens share.
        let before = working_screen(SCREEN_HEAD, &[STEER_ENTRY, "\u{2022} done\n"], EMPTY_INPUT);
        let repeated = working_screen(
            SCREEN_HEAD,
            &[STEER_ENTRY, "\u{2022} done\n", STEER_ENTRY],
            EMPTY_INPUT,
        );
        assert!(prompt_acceptance_evidence(
            &repeated,
            &working_probe(Some(&before), codex)
        ));
    }

    #[test]
    fn queued_evidence_still_accepts_a_working_prompt() {
        let _manifests = SyntheticManifests::install(&[("codex", SYNTHETIC_CODEX)]);
        let codex = Some(crate::detect::Agent::Codex);
        let before = working_screen(SCREEN_HEAD, &[], EMPTY_INPUT);
        let queued = working_screen(
            SCREEN_HEAD,
            &["TEST-QUEUED\n  review the diff\n"],
            EMPTY_INPUT,
        );
        assert!(prompt_acceptance_evidence(
            &queued,
            &working_probe(Some(&before), codex)
        ));
    }

    #[test]
    fn a_steered_entry_is_not_evidence_for_other_agent_kinds() {
        let _manifests = SyntheticManifests::install(&[
            ("codex", SYNTHETIC_CODEX),
            ("claude", SYNTHETIC_CLAUDE),
        ]);
        let before = working_screen(SCREEN_HEAD, &[], EMPTY_INPUT);
        let steered = working_screen(SCREEN_HEAD, &[STEER_ENTRY], EMPTY_INPUT);
        for agent in [Some(crate::detect::Agent::Claude), None] {
            assert!(!crate::detect::submitted_evidence_supported(agent));
            assert!(!prompt_acceptance_evidence(
                &steered,
                &working_probe(Some(&before), agent)
            ));
        }
    }

    #[test]
    fn stalled_timeout_says_input_was_written() {
        let response = agent_wait_timeout(
            "req".into(),
            AgentWaitTimeoutKind::PromptStalled { timeout_ms: 5000 },
            &test_agent(),
        )
        .unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["error"]["code"], "agent_prompt_stalled");
        let message = response["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("input was written"),
            "stalled message should say the input was written: {message}"
        );
    }
}
