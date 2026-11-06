//! Direct terminal admission guarded by an expected launch.
//!
//! `herdr terminal attach|session control|session observe --expect-launch`
//! sends its mode, target and expected launch in one named `EndpointControl`
//! instead of `AttachTerminal`, `ControlTerminal` or `ObserveTerminal`. No
//! existing message variant, field or codec changes meaning, and the server
//! checks the launch in the same step that admits the connection. An older
//! server ignores the unknown control and attaches nothing, so the guard can
//! never be dropped on the way to an attach.
//!
//! The server answers with [`ANSWER_KIND`] before it sends any frame. A
//! refused connection is then closed.

use serde::{Deserialize, Serialize};

use super::{ClientMessage, ServerMessage};

pub(crate) const REQUEST_KIND: &str = "terminal.launch_guard.request.v1";
pub(crate) const ANSWER_KIND: &str = "terminal.launch_guard.answer.v1";

/// How a stream's close reason starts when the guarded launch stopped
/// occupying the terminal during the attach.
pub(crate) const LAUNCH_ENDED_CODE: &str = "terminal_launch_ended";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AdmissionMode {
    /// `terminal attach`: the target is a terminal id.
    Attach,
    /// `terminal session control`: the target is a pane, terminal or agent.
    Control,
    /// `terminal session observe`: the target is a pane, terminal or agent.
    Observe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AdmissionRequest {
    pub mode: AdmissionMode,
    pub target: String,
    #[serde(default)]
    pub takeover: bool,
    pub expected_launch_incarnation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AdmissionAnswer {
    pub admitted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub(crate) fn request_message(request: &AdmissionRequest) -> serde_json::Result<ClientMessage> {
    Ok(ClientMessage::EndpointControl {
        kind: REQUEST_KIND.into(),
        data: serde_json::to_string(request)?,
    })
}

pub(crate) fn answer_message(answer: &AdmissionAnswer) -> serde_json::Result<ServerMessage> {
    Ok(ServerMessage::EndpointControl {
        kind: ANSWER_KIND.into(),
        data: serde_json::to_string(answer)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_names_its_mode_and_launch_on_the_wire() {
        let request = AdmissionRequest {
            mode: AdmissionMode::Observe,
            target: "w1:p1".into(),
            takeover: false,
            expected_launch_incarnation: "launch_0".into(),
        };
        let ClientMessage::EndpointControl { kind, data } = request_message(&request).unwrap()
        else {
            panic!("the request is a named control");
        };
        assert_eq!(kind, REQUEST_KIND);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&data).unwrap(),
            serde_json::json!({
                "mode": "observe",
                "target": "w1:p1",
                "takeover": false,
                "expected_launch_incarnation": "launch_0",
            })
        );
    }

    #[test]
    fn a_request_without_an_expected_launch_does_not_parse() {
        assert!(serde_json::from_str::<AdmissionRequest>(
            r#"{"mode":"attach","target":"term","takeover":false}"#
        )
        .is_err());
        assert!(serde_json::from_str::<AdmissionRequest>(
            r#"{"mode":"other","target":"term","expected_launch_incarnation":"x"}"#
        )
        .is_err());
    }
}
